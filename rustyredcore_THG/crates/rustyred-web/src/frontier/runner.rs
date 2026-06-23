use std::sync::Arc;

use futures_util::future::join_all;
use rustyred_thg_core::HookDispatcher;
use serde::{Deserialize, Serialize};

use super::fetcher::Fetcher;
use super::model::FetchOutcome;
use super::{Frontier, FrontierResult};
use crate::crawl_hooks::attach_crawl_hooks;

#[derive(Clone)]
pub struct CrawlRunner {
    frontier: Frontier,
    fetcher: Arc<dyn Fetcher>,
    concurrency: usize,
    // Owns the self-organizing crawl hook dispatcher when enabled; kept alive
    // for the runner's lifetime (its worker stops on drop). `None` = the legacy
    // periodic-prioritizer behavior, unchanged.
    hook_dispatcher: Option<Arc<HookDispatcher>>,
}

impl CrawlRunner {
    pub fn new(frontier: Frontier, fetcher: impl Fetcher + 'static, concurrency: usize) -> Self {
        Self {
            frontier,
            fetcher: Arc::new(fetcher),
            concurrency: concurrency.max(1),
            hook_dispatcher: None,
        }
    }

    pub fn from_shared(frontier: Frontier, fetcher: Arc<dyn Fetcher>, concurrency: usize) -> Self {
        Self {
            frontier,
            fetcher,
            concurrency: concurrency.max(1),
            hook_dispatcher: None,
        }
    }

    /// Opt into the event-driven self-organizing frontier: attach the crawl
    /// hooks so each fetched page and discovered link reprioritizes the frontier
    /// inside the store, instead of relying on the periodic prioritizer. The
    /// dispatcher is owned by the runner and torn down with it.
    pub async fn enable_crawl_hooks(&mut self) {
        let dispatcher = attach_crawl_hooks(self.frontier.store(), self.frontier.tenant()).await;
        self.hook_dispatcher = Some(Arc::new(dispatcher));
    }

    /// The crawl hook dispatcher, when enabled (for tests / quiesce).
    pub fn hook_dispatcher(&self) -> Option<&HookDispatcher> {
        self.hook_dispatcher.as_deref()
    }

    pub async fn run(&self) -> FrontierResult<CrawlReport> {
        let mut report = CrawlReport::default();
        while self.frontier.has_pending().await? {
            report.iterations += 1;
            let tasks = self.frontier.next_batch(self.concurrency).await?;
            if tasks.is_empty() {
                break;
            }
            let futures = tasks.into_iter().map(|task| {
                let fetcher = Arc::clone(&self.fetcher);
                async move {
                    let outcome = fetcher.fetch(task.clone()).await;
                    (task, outcome)
                }
            });
            for (task, outcome) in join_all(futures).await {
                match &outcome {
                    FetchOutcome::Ok { links, .. } => {
                        self.frontier
                            .enqueue_discovered(&task.fp, links.clone(), task.depth)
                            .await?;
                        report.fetched += 1;
                    }
                    FetchOutcome::Error { .. } => {
                        report.errors += 1;
                    }
                    FetchOutcome::Skipped { .. } => {
                        report.skipped += 1;
                    }
                }
                self.frontier.complete(&task.fp, outcome).await?;
            }
        }
        report.pending = self.frontier.has_pending().await?;
        Ok(report)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct CrawlReport {
    pub fetched: usize,
    pub errors: usize,
    pub skipped: usize,
    pub iterations: usize,
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use rustyred_thg_core::{GraphStore, NodeQuery, RedCoreGraphStore};

    use super::*;
    use crate::frontier::model::{DiscoveredLink, FetchTask};
    use crate::frontier::{DepthPrioritizer, MemoryFrontierQueue};

    #[derive(Clone, Debug)]
    struct FixtureFetcher;

    #[async_trait]
    impl Fetcher for FixtureFetcher {
        async fn fetch(&self, task: FetchTask) -> FetchOutcome {
            let links = if task.depth == 0 {
                vec![DiscoveredLink {
                    url_raw: "/child".to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                }]
            } else {
                Vec::new()
            };
            FetchOutcome::Ok {
                final_url: task.url,
                status: 200,
                content_hash: [0u8; 32],
                etag: None,
                links,
            }
        }
    }

    #[tokio::test]
    async fn runner_fetches_discovered_links_once() {
        let frontier = Frontier::new(
            RedCoreGraphStore::memory(),
            MemoryFrontierQueue::new(),
            DepthPrioritizer::default(),
            "tenant",
        );
        frontier
            .seed(vec!["https://example.com/root".to_string()])
            .await
            .unwrap();
        let runner = CrawlRunner::new(frontier.clone(), FixtureFetcher, 2);
        let report = runner.run().await.unwrap();
        assert_eq!(report.fetched, 2);
        assert!(!report.pending);

        let store = frontier.store();
        let store = store.lock().await;
        let fetched = GraphStore::query_nodes(&*store, NodeQuery::label("url"))
            .into_iter()
            .filter(|node| node.properties.get("state") == Some(&serde_json::json!("fetched")))
            .count();
        assert_eq!(fetched, 2);
    }
}
