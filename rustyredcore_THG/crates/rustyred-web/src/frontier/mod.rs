use std::fmt;
use std::sync::Arc;

use rustyred_thg_core::{
    now_ms, EdgeRecord, GraphStore, GraphStoreError, GraphWriteResult, NodeQuery, NodeRecord,
    RedCoreGraphStore,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;
use url::Url;

use crate::RustyWebError;

pub mod fetcher;
pub mod model;
pub mod politeness;
pub mod prioritizer;
pub mod queue;
pub mod runner;

pub use fetcher::{CascadeFetcher, Fetcher};
pub use model::{DiscoveredLink, FetchOutcome, FetchTask, UrlFingerprint, UrlNodeView};
pub use politeness::Politeness;
pub use prioritizer::{DepthPrioritizer, FrontierCtx, PprPrioritizer, Prioritizer};
pub use queue::{FrontierQueue, MemoryFrontierQueue};
pub use runner::{CrawlReport, CrawlRunner};

use model::{
    canonicalize_url, domain_for_url, fingerprint, EDGE_LINKS_TO, EDGE_ON_DOMAIN, LABEL_DOMAIN,
    LABEL_URL, STATE_ERROR, STATE_FETCHED, STATE_FRONTIER, STATE_IN_FLIGHT, STATE_SKIPPED,
};

pub type SharedFrontierStore = Arc<Mutex<RedCoreGraphStore>>;
pub type FrontierResult<T> = Result<T, FrontierError>;

#[derive(Debug)]
pub enum FrontierError {
    Graph(GraphStoreError),
    Web(RustyWebError),
    Queue(String),
    Invalid(String),
}

impl fmt::Display for FrontierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => write!(f, "{error:?}"),
            Self::Web(error) => write!(f, "{error}"),
            Self::Queue(error) => write!(f, "frontier queue error: {error}"),
            Self::Invalid(error) => write!(f, "invalid frontier state: {error}"),
        }
    }
}

impl std::error::Error for FrontierError {}

impl From<GraphStoreError> for FrontierError {
    fn from(error: GraphStoreError) -> Self {
        Self::Graph(error)
    }
}

impl From<RustyWebError> for FrontierError {
    fn from(error: RustyWebError) -> Self {
        Self::Web(error)
    }
}

#[cfg(feature = "redis-frontier")]
impl From<redis::RedisError> for FrontierError {
    fn from(error: redis::RedisError) -> Self {
        Self::Queue(error.to_string())
    }
}

#[derive(Clone)]
pub struct Frontier {
    store: SharedFrontierStore,
    queue: Arc<dyn FrontierQueue>,
    prioritizer: Arc<dyn Prioritizer>,
    politeness: Politeness,
    tenant: String,
}

impl Frontier {
    pub fn new(
        store: RedCoreGraphStore,
        queue: impl FrontierQueue + 'static,
        prioritizer: impl Prioritizer + 'static,
        tenant: impl Into<String>,
    ) -> Self {
        Self::from_shared(
            Arc::new(Mutex::new(store)),
            Arc::new(queue),
            Arc::new(prioritizer),
            Politeness::default(),
            tenant,
        )
    }

    pub fn from_shared(
        store: SharedFrontierStore,
        queue: Arc<dyn FrontierQueue>,
        prioritizer: Arc<dyn Prioritizer>,
        politeness: Politeness,
        tenant: impl Into<String>,
    ) -> Self {
        Self {
            store,
            queue,
            prioritizer,
            politeness,
            tenant: tenant.into(),
        }
    }

    pub fn store(&self) -> SharedFrontierStore {
        Arc::clone(&self.store)
    }

    pub fn tenant(&self) -> &str {
        &self.tenant
    }

    pub async fn resume_pending(&self) -> FrontierResult<usize> {
        let pending = {
            let store = self.store.lock().await;
            GraphStore::query_nodes(&*store, NodeQuery::label(LABEL_URL))
                .into_iter()
                .filter(|node| node.properties.get("state") == Some(&json!(STATE_FRONTIER)))
                .filter_map(|node| {
                    let fp = UrlFingerprint::from_hex(&node.id)?;
                    let domain = node
                        .properties
                        .get("domain")
                        .and_then(Value::as_str)?
                        .to_string();
                    let priority = node
                        .properties
                        .get("priority")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    Some((fp, domain, priority))
                })
                .collect::<Vec<_>>()
        };
        for (fp, domain, priority) in &pending {
            self.queue.remember_domain(fp, domain).await?;
            self.queue.push(fp, *priority).await?;
        }
        Ok(pending.len())
    }

    pub async fn seed(&self, roots: Vec<String>) -> FrontierResult<()> {
        let mut pushes = Vec::new();
        {
            let mut store = self.store.lock().await;
            for root in roots {
                let Some(canonical) = canonicalize_url(&root, None) else {
                    continue;
                };
                let Some(domain) = domain_for_url(&canonical) else {
                    continue;
                };
                let fp = fingerprint("GET", &canonical, b"");
                let node = url_node(fp, &canonical, &root, &domain, 0, STATE_FRONTIER, 0.0, true);
                let created = store.insert_node_if_absent(node)?.is_some();
                ensure_domain(&mut store, &self.politeness, &domain)?;
                ensure_on_domain_edge(&mut store, fp, &domain)?;
                if created {
                    let view = UrlNodeView {
                        fp,
                        url: canonical,
                        domain: domain.clone(),
                        depth: 0,
                        state: STATE_FRONTIER.to_string(),
                        priority: 0.0,
                        retry_count: 0,
                    };
                    let priority = self.prioritizer.score(
                        &FrontierCtx {
                            store: &store,
                            tenant: &self.tenant,
                        },
                        &view,
                    );
                    write_node_priority(&mut store, fp, priority)?;
                    pushes.push((fp, domain, priority));
                }
            }
        }
        for (fp, domain, priority) in pushes {
            self.queue.remember_domain(&fp, &domain).await?;
            self.queue.push(&fp, priority).await?;
        }
        Ok(())
    }

    pub async fn enqueue_discovered(
        &self,
        parent: &UrlFingerprint,
        links: Vec<DiscoveredLink>,
        parent_depth: u32,
    ) -> FrontierResult<()> {
        let mut pushes = Vec::new();
        {
            let mut store = self.store.lock().await;
            let parent_node = store.get_node(&parent.to_hex())?.ok_or_else(|| {
                FrontierError::Invalid(format!("missing parent URL node {parent}"))
            })?;
            let parent_url = parent_node
                .properties
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| FrontierError::Invalid("parent URL node missing url".to_string()))?
                .to_string();
            let base = Url::parse(&parent_url).map_err(|error| {
                FrontierError::Invalid(format!("stored parent URL is invalid: {error}"))
            })?;
            for link in links {
                let Some(canonical) = canonicalize_url(&link.url_raw, Some(&base)) else {
                    continue;
                };
                let Some(domain) = domain_for_url(&canonical) else {
                    continue;
                };
                let fp = fingerprint("GET", &canonical, b"");
                let depth = parent_depth.saturating_add(1);
                let node = url_node(
                    fp,
                    &canonical,
                    &link.url_raw,
                    &domain,
                    depth,
                    STATE_FRONTIER,
                    0.0,
                    true,
                );
                let created = store.insert_node_if_absent(node)?.is_some();
                ensure_domain(&mut store, &self.politeness, &domain)?;
                ensure_on_domain_edge(&mut store, fp, &domain)?;
                ensure_links_to_edge(&mut store, parent, fp, &link)?;
                if created {
                    let view = UrlNodeView {
                        fp,
                        url: canonical,
                        domain: domain.clone(),
                        depth,
                        state: STATE_FRONTIER.to_string(),
                        priority: 0.0,
                        retry_count: 0,
                    };
                    let priority = self.prioritizer.score(
                        &FrontierCtx {
                            store: &store,
                            tenant: &self.tenant,
                        },
                        &view,
                    );
                    write_node_priority(&mut store, fp, priority)?;
                    pushes.push((fp, domain, priority));
                }
            }
        }
        for (fp, domain, priority) in pushes {
            self.queue.remember_domain(&fp, &domain).await?;
            self.queue.push(&fp, priority).await?;
        }
        Ok(())
    }

    pub async fn next_batch(&self, n: usize) -> FrontierResult<Vec<FetchTask>> {
        let mut tasks = Vec::new();
        for _ in 0..n {
            let ready_domains = {
                let store = self.store.lock().await;
                self.politeness.ready_domains(&store, now_ms())
            };
            let Some(fp) = self
                .queue
                .pop_eligible(&|domain| ready_domains.get(domain).copied().unwrap_or(true))
                .await?
            else {
                break;
            };

            let mut store = self.store.lock().await;
            let claimed = store
                .compare_and_set_node_property(
                    &fp.to_hex(),
                    "state",
                    &json!(STATE_FRONTIER),
                    json!(STATE_IN_FLIGHT),
                )?
                .is_some();
            if !claimed {
                continue;
            }
            let Some(view) = read_url_view(&store, &fp)? else {
                continue;
            };
            update_domain_inflight(&mut store, &view.domain, 1)?;
            tasks.push(FetchTask {
                fp,
                url: view.url,
                domain: view.domain,
                depth: view.depth,
            });
        }
        Ok(tasks)
    }

    pub async fn complete(&self, fp: &UrlFingerprint, outcome: FetchOutcome) -> FrontierResult<()> {
        let mut requeue = None;
        {
            let mut store = self.store.lock().await;
            let Some(mut node) = store.get_node(&fp.to_hex())? else {
                return Ok(());
            };
            let domain = node
                .properties
                .get("domain")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            update_domain_inflight(&mut store, &domain, -1)?;
            match outcome {
                FetchOutcome::Ok {
                    final_url,
                    status,
                    content_hash,
                    etag,
                    links: _,
                } => {
                    merge_properties(
                        &mut node,
                        json!({
                            "url": final_url,
                            "state": STATE_FETCHED,
                            "fetched_at": now_ms(),
                            "status_code": status,
                            "content_hash": bytes_to_hex(&content_hash),
                            "etag": etag,
                        }),
                    );
                    store.upsert_node(node)?;
                    mark_domain_fetched(&mut store, &domain)?;
                }
                FetchOutcome::Skipped { reason } => {
                    merge_properties(
                        &mut node,
                        json!({
                            "state": STATE_SKIPPED,
                            "fetched_at": now_ms(),
                            "skip_reason": reason,
                        }),
                    );
                    store.upsert_node(node)?;
                    mark_domain_fetched(&mut store, &domain)?;
                }
                FetchOutcome::Error { status, retryable } => {
                    let retry_count = node
                        .properties
                        .get("retry_count")
                        .and_then(Value::as_u64)
                        .unwrap_or(0) as u32
                        + 1;
                    let current_priority = node
                        .properties
                        .get("priority")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0);
                    if retryable && retry_count <= self.politeness.max_retries {
                        let priority = self
                            .politeness
                            .retry_priority(current_priority, retry_count);
                        merge_properties(
                            &mut node,
                            json!({
                                "state": STATE_FRONTIER,
                                "retry_count": retry_count,
                                "status_code": status,
                                "priority": priority,
                            }),
                        );
                        store.upsert_node(node)?;
                        requeue = Some((*fp, domain.clone(), priority));
                    } else {
                        merge_properties(
                            &mut node,
                            json!({
                                "state": STATE_ERROR,
                                "fetched_at": now_ms(),
                                "retry_count": retry_count,
                                "status_code": status,
                            }),
                        );
                        store.upsert_node(node)?;
                        mark_domain_fetched(&mut store, &domain)?;
                    }
                }
            }
        }
        if let Some((fp, domain, priority)) = requeue {
            self.queue.remember_domain(&fp, &domain).await?;
            self.queue.requeue(&fp, priority).await?;
        }
        Ok(())
    }

    pub async fn has_pending(&self) -> FrontierResult<bool> {
        Ok(self.queue.len().await? > 0)
    }
}

fn url_node(
    fp: UrlFingerprint,
    url: &str,
    url_raw: &str,
    domain: &str,
    depth: u32,
    state: &str,
    priority: f64,
    robots_allowed: bool,
) -> NodeRecord {
    NodeRecord::new(
        fp.to_hex(),
        [LABEL_URL],
        json!({
            "url": url,
            "url_raw": url_raw,
            "domain": domain,
            "depth": depth,
            "state": state,
            "priority": priority,
            "discovered_at": now_ms(),
            "fetched_at": Value::Null,
            "status_code": Value::Null,
            "content_hash": Value::Null,
            "etag": Value::Null,
            "retry_count": 0_u32,
            "robots_allowed": robots_allowed,
        }),
    )
}

fn ensure_domain(
    store: &mut RedCoreGraphStore,
    politeness: &Politeness,
    domain: &str,
) -> FrontierResult<Option<GraphWriteResult>> {
    Ok(store.insert_node_if_absent(NodeRecord::new(
        domain,
        [LABEL_DOMAIN],
        politeness.domain_defaults(domain),
    ))?)
}

fn ensure_on_domain_edge(
    store: &mut RedCoreGraphStore,
    fp: UrlFingerprint,
    domain: &str,
) -> FrontierResult<GraphWriteResult> {
    Ok(store.upsert_edge(EdgeRecord::new(
        format!("{}:{}:{}", EDGE_ON_DOMAIN, fp.to_hex(), domain),
        fp.to_hex(),
        EDGE_ON_DOMAIN,
        domain,
        json!({}),
    ))?)
}

fn ensure_links_to_edge(
    store: &mut RedCoreGraphStore,
    parent: &UrlFingerprint,
    child: UrlFingerprint,
    link: &DiscoveredLink,
) -> FrontierResult<GraphWriteResult> {
    let edge_hash = blake3::hash(
        format!(
            "{}\n{}\n{}\n{}",
            parent.to_hex(),
            child.to_hex(),
            link.anchor_text,
            link.rel
        )
        .as_bytes(),
    )
    .to_hex()
    .to_string();
    Ok(store.upsert_edge(EdgeRecord::new(
        format!("{}:{edge_hash}", EDGE_LINKS_TO),
        parent.to_hex(),
        EDGE_LINKS_TO,
        child.to_hex(),
        json!({
            "anchor_text": link.anchor_text,
            "rel": link.rel,
            "discovered_at": now_ms(),
        }),
    ))?)
}

fn read_url_view(
    store: &RedCoreGraphStore,
    fp: &UrlFingerprint,
) -> FrontierResult<Option<UrlNodeView>> {
    Ok(store.get_node(&fp.to_hex())?.map(|node| UrlNodeView {
        fp: *fp,
        url: node
            .properties
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        domain: node
            .properties
            .get("domain")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        depth: node
            .properties
            .get("depth")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
        state: node
            .properties
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        priority: node
            .properties
            .get("priority")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        retry_count: node
            .properties
            .get("retry_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32,
    }))
}

fn write_node_priority(
    store: &mut RedCoreGraphStore,
    fp: UrlFingerprint,
    priority: f64,
) -> FrontierResult<()> {
    let Some(mut node) = store.get_node(&fp.to_hex())? else {
        return Ok(());
    };
    merge_properties(&mut node, json!({ "priority": priority }));
    store.upsert_node(node)?;
    Ok(())
}

fn update_domain_inflight(
    store: &mut RedCoreGraphStore,
    domain: &str,
    delta: i64,
) -> FrontierResult<()> {
    if domain.is_empty() {
        return Ok(());
    }
    let Some(mut node) = store.get_node(domain)? else {
        return Ok(());
    };
    let current = node
        .properties
        .get("in_flight_count")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    merge_properties(
        &mut node,
        json!({ "in_flight_count": current.saturating_add(delta).max(0) }),
    );
    store.upsert_node(node)?;
    Ok(())
}

fn mark_domain_fetched(store: &mut RedCoreGraphStore, domain: &str) -> FrontierResult<()> {
    if domain.is_empty() {
        return Ok(());
    }
    let Some(mut node) = store.get_node(domain)? else {
        return Ok(());
    };
    merge_properties(&mut node, json!({ "last_fetched_at": now_ms() }));
    store.upsert_node(node)?;
    Ok(())
}

fn merge_properties(node: &mut NodeRecord, patch: Value) {
    let mut properties = node
        .properties
        .as_object()
        .cloned()
        .unwrap_or_else(serde_json::Map::new);
    if let Some(patch) = patch.as_object() {
        for (key, value) in patch {
            properties.insert(key.clone(), value.clone());
        }
    }
    node.properties = Value::Object(properties);
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(hex_char(byte >> 4));
        out.push(hex_char(byte & 0x0f));
    }
    out
}

fn hex_char(nibble: u8) -> char {
    match nibble {
        0..=9 => (b'0' + nibble) as char,
        10..=15 => (b'a' + (nibble - 10)) as char,
        _ => '0',
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{GraphStore, NeighborQuery};

    #[tokio::test]
    async fn seed_writes_url_domain_and_queue_entry() {
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
        assert!(frontier.has_pending().await.unwrap());
        let store = frontier.store();
        let store = store.lock().await;
        assert_eq!(
            GraphStore::query_nodes(&*store, rustyred_thg_core::NodeQuery::label(LABEL_URL)).len(),
            1
        );
        assert_eq!(
            GraphStore::query_nodes(&*store, rustyred_thg_core::NodeQuery::label(LABEL_DOMAIN))
                .len(),
            1
        );
    }

    #[tokio::test]
    async fn enqueue_discovered_dedups_nodes_but_keeps_provenance_edges() {
        let frontier = Frontier::new(
            RedCoreGraphStore::memory(),
            MemoryFrontierQueue::new(),
            DepthPrioritizer::default(),
            "tenant",
        );
        frontier
            .seed(vec![
                "https://example.com/a".to_string(),
                "https://example.com/b".to_string(),
            ])
            .await
            .unwrap();
        let first = frontier.next_batch(1).await.unwrap().remove(0);
        frontier
            .enqueue_discovered(
                &first.fp,
                vec![DiscoveredLink {
                    url_raw: "/shared".to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                }],
                first.depth,
            )
            .await
            .unwrap();
        frontier
            .complete(
                &first.fp,
                FetchOutcome::Ok {
                    final_url: first.url,
                    status: 200,
                    content_hash: [0u8; 32],
                    etag: None,
                    links: Vec::new(),
                },
            )
            .await
            .unwrap();

        let second = frontier.next_batch(1).await.unwrap().remove(0);
        frontier
            .enqueue_discovered(
                &second.fp,
                vec![DiscoveredLink {
                    url_raw: "/shared".to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                }],
                second.depth,
            )
            .await
            .unwrap();
        let store = frontier.store();
        let store = store.lock().await;
        let urls = GraphStore::query_nodes(&*store, rustyred_thg_core::NodeQuery::label(LABEL_URL));
        assert_eq!(urls.len(), 3);
        let shared = urls
            .iter()
            .find(|node| node.properties.get("url") == Some(&json!("https://example.com/shared")))
            .unwrap();
        let incoming = GraphStore::neighbors(
            &*store,
            NeighborQuery::in_(shared.id.clone()).with_edge_type(EDGE_LINKS_TO),
        );
        assert_eq!(incoming.len(), 2);
    }

    #[tokio::test]
    async fn next_batch_claims_each_url_once() {
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
        let first = frontier.next_batch(1).await.unwrap();
        let second = frontier.next_batch(1).await.unwrap();
        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn resume_pending_rehydrates_queue_from_graph_state() {
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

        let resumed = Frontier::from_shared(
            frontier.store(),
            Arc::new(MemoryFrontierQueue::new()),
            Arc::new(DepthPrioritizer::default()),
            Politeness::default(),
            "tenant",
        );
        assert_eq!(resumed.resume_pending().await.unwrap(), 1);
        let tasks = resumed.next_batch(1).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].url, "https://example.com/root");
    }
}
