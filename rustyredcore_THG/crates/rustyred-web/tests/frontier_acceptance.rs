//! Acceptance-criteria verification for the RustyRed crawl frontier.
//!
//! Claude Code verification lane (companion to
//! `docs/plans/rustyred-frontier-verification.md`). Codex owns `src/frontier/*`;
//! these external integration tests close the coverage gaps its in-module unit
//! tests leave: the in-module `next_batch_claims_each_url_once` is sequential
//! (not concurrent) and `ppr_prioritizer_ranks_central_node` only asserts a
//! score is `> 0.0` (not that PPR reorders vs depth). These four tests prove the
//! handoff's acceptance criteria AC2 / AC4 / AC5 / AC6 as observable behavior.

use std::collections::{BTreeSet, HashMap, HashSet};

use async_trait::async_trait;
use serde_json::json;

use rustyred_thg_core::{
    EdgeRecord, GraphStore, NeighborQuery, NodeQuery, NodeRecord, RedCoreGraphStore,
};
use rustyred_web::frontier::model::{fingerprint, EDGE_LINKS_TO, LABEL_URL};
use rustyred_web::frontier::{
    CrawlRunner, DepthPrioritizer, DiscoveredLink, FetchOutcome, FetchTask, Fetcher, Frontier,
    FrontierCtx, MemoryFrontierQueue, PprPrioritizer, Prioritizer, UrlFingerprint, UrlNodeView,
};

// ---------------------------------------------------------------------------
// AC2 - two concurrent workers never claim the same URL twice (under load).
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn ac2_concurrent_workers_claim_each_url_exactly_once() {
    let frontier = Frontier::new(
        RedCoreGraphStore::memory(),
        MemoryFrontierQueue::new(),
        DepthPrioritizer::default(),
        "tenant",
    );
    // Distinct hosts so per-domain in-flight politeness (cap 1) does not
    // serialize the workers - we want them genuinely racing the claim path.
    let seeds: Vec<String> = (0..200).map(|i| format!("https://h{i}.example.com/")).collect();
    frontier.seed(seeds).await.unwrap();

    let mut handles = Vec::new();
    for _ in 0..8 {
        let worker = frontier.clone();
        handles.push(tokio::spawn(async move {
            let mut claimed: Vec<UrlFingerprint> = Vec::new();
            loop {
                let batch = worker.next_batch(4).await.unwrap();
                if batch.is_empty() {
                    if worker.has_pending().await.unwrap() {
                        tokio::task::yield_now().await;
                        continue;
                    }
                    break;
                }
                claimed.extend(batch.into_iter().map(|task| task.fp));
            }
            claimed
        }));
    }

    let mut all = Vec::new();
    for handle in handles {
        all.extend(handle.await.unwrap());
    }

    // No fingerprint reaches a worker twice...
    let mut seen = HashSet::new();
    for fp in &all {
        assert!(seen.insert(*fp), "fingerprint claimed by two workers: {fp}");
    }
    // ...and every seeded URL was claimed exactly once across all workers.
    assert_eq!(seen.len(), 200, "expected all 200 URLs claimed exactly once");
    assert_eq!(all.len(), 200, "no double-claims and none dropped");
}

// ---------------------------------------------------------------------------
// AC4 - PprPrioritizer produces a different order than DepthPrioritizer when
// centrality and depth disagree. Graph: root -> {x1,x2,x3} -> hub. The hub is
// the DEEPEST node (depth 2) yet the most CENTRAL (3 in-links).
// ---------------------------------------------------------------------------

#[test]
fn ac4_ppr_reorders_versus_depth() {
    let root = fingerprint("GET", "https://example.com/root", b"");
    let x1 = fingerprint("GET", "https://example.com/x1", b"");
    let x2 = fingerprint("GET", "https://example.com/x2", b"");
    let x3 = fingerprint("GET", "https://example.com/x3", b"");
    let hub = fingerprint("GET", "https://example.com/hub", b"");

    let mut store = RedCoreGraphStore::memory();
    let node = |fp: UrlFingerprint, depth: u64| {
        NodeRecord::new(fp.to_hex(), [LABEL_URL], json!({ "depth": depth }))
    };
    store.upsert_node(node(root, 0)).unwrap();
    for fp in [x1, x2, x3] {
        store.upsert_node(node(fp, 1)).unwrap();
    }
    store.upsert_node(node(hub, 2)).unwrap();
    for x in [x1, x2, x3] {
        store
            .upsert_edge(EdgeRecord::new(
                format!("{}->{}", root.to_hex(), x.to_hex()),
                root.to_hex(),
                EDGE_LINKS_TO,
                x.to_hex(),
                json!({}),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                format!("{}->{}", x.to_hex(), hub.to_hex()),
                x.to_hex(),
                EDGE_LINKS_TO,
                hub.to_hex(),
                json!({}),
            ))
            .unwrap();
    }

    let ctx = FrontierCtx {
        store: &store,
        tenant: "t",
    };
    let view = |fp: UrlFingerprint, depth: u32| UrlNodeView {
        fp,
        url: String::new(),
        domain: String::new(),
        depth,
        state: String::new(),
        priority: 0.0,
        retry_count: 0,
    };

    // Depth ranks the hub LAST (deepest -> lowest score).
    let depth = DepthPrioritizer::default();
    let depth_hub = depth.score(&ctx, &view(hub, 2));
    let depth_x1 = depth.score(&ctx, &view(x1, 1));
    assert!(
        depth_hub < depth_x1,
        "DepthPrioritizer must rank the deep hub below the shallow x nodes (hub={depth_hub}, x1={depth_x1})"
    );

    // PPR ranks the hub ABOVE the x nodes (centrality wins). Same nodes, flipped order.
    let ppr = PprPrioritizer {
        seeds: vec![root],
        ..Default::default()
    };
    let scores: HashMap<UrlFingerprint, f64> =
        ppr.recompute(&ctx).unwrap().into_iter().collect();
    let ppr_hub = *scores.get(&hub).unwrap_or(&0.0);
    let ppr_x1 = *scores.get(&x1).unwrap_or(&0.0);
    assert!(
        ppr_hub > ppr_x1,
        "PprPrioritizer must rank the central hub above the x nodes (hub={ppr_hub}, x1={ppr_x1})"
    );
}

// ---------------------------------------------------------------------------
// AC5 - swapping the Fetcher changes nothing about the frontier graph. Two
// fetchers emit the same site's links in a DIFFERENT order; the resulting
// visited set and links_to edge set must be identical. Proves fetcher-agnosticism
// without pulling in the heavy spider/servo backends.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct OrderedFetcher {
    links: Vec<&'static str>,
}

#[async_trait]
impl Fetcher for OrderedFetcher {
    async fn fetch(&self, task: FetchTask) -> FetchOutcome {
        let links = if task.depth == 0 {
            self.links
                .iter()
                .map(|url| DiscoveredLink {
                    url_raw: (*url).to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                })
                .collect()
        } else {
            Vec::new()
        };
        FetchOutcome::Ok {
            final_url: task.url,
            status: 200,
            content_hash: [7u8; 32],
            etag: None,
            links,
        }
    }
}

async fn crawl_graph_with(fetcher: OrderedFetcher) -> (BTreeSet<String>, BTreeSet<(String, String)>) {
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
    CrawlRunner::new(frontier.clone(), fetcher, 2)
        .run()
        .await
        .unwrap();

    let store = frontier.store();
    let store = store.lock().await;
    let nodes = GraphStore::query_nodes(&*store, NodeQuery::label(LABEL_URL));
    let visited: BTreeSet<String> = nodes
        .iter()
        .filter_map(|node| {
            node.properties
                .get("url")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect();
    let mut edges = BTreeSet::new();
    for node in &nodes {
        for hit in GraphStore::neighbors(
            &*store,
            NeighborQuery::out(node.id.clone()).with_edge_type(EDGE_LINKS_TO),
        ) {
            edges.insert((node.id.clone(), hit.node_id));
        }
    }
    (visited, edges)
}

#[tokio::test]
async fn ac5_fetcher_swap_yields_identical_graph() {
    let (visited_a, edges_a) =
        crawl_graph_with(OrderedFetcher { links: vec!["/a", "/b", "/c"] }).await;
    let (visited_b, edges_b) =
        crawl_graph_with(OrderedFetcher { links: vec!["/c", "/b", "/a"] }).await;

    assert_eq!(visited_a, visited_b, "fetcher swap changed the visited set");
    assert_eq!(edges_a, edges_b, "fetcher swap changed the crawl graph");
    assert_eq!(visited_a.len(), 4, "expected root + /a + /b + /c");
    assert_eq!(edges_a.len(), 3, "expected root->a, root->b, root->c");
}

// ---------------------------------------------------------------------------
// AC6 - an in-progress crawl is queryable as a graph: with root in_flight and
// its children still in the frontier, neighbors() and PPR over links_to return
// sensible structure.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ac6_in_progress_crawl_is_queryable_as_graph() {
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

    // Claim root (-> in_flight) and discover two children (-> frontier): mid-crawl.
    let root_task = frontier.next_batch(1).await.unwrap().remove(0);
    frontier
        .enqueue_discovered(
            &root_task.fp,
            vec![
                DiscoveredLink {
                    url_raw: "/a".to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                },
                DiscoveredLink {
                    url_raw: "/b".to_string(),
                    anchor_text: String::new(),
                    rel: String::new(),
                },
            ],
            root_task.depth,
        )
        .await
        .unwrap();

    let store = frontier.store();
    let store = store.lock().await;

    // neighbors over links_to exposes the in-progress structure.
    let out = GraphStore::neighbors(
        &*store,
        NeighborQuery::out(root_task.fp.to_hex()).with_edge_type(EDGE_LINKS_TO),
    );
    assert_eq!(out.len(), 2, "root should link to its two discovered children");

    // PPR over the in-progress graph returns positive mass.
    let ppr = PprPrioritizer {
        seeds: vec![root_task.fp],
        ..Default::default()
    };
    let scores = ppr
        .recompute(&FrontierCtx {
            store: &store,
            tenant: "t",
        })
        .unwrap();
    assert!(!scores.is_empty(), "PPR should yield scores mid-crawl");
    assert!(
        scores.iter().any(|(_, score)| *score > 0.0),
        "PPR should assign positive mass mid-crawl"
    );

    // The mixed lifecycle is inspectable as graph state.
    let states: BTreeSet<String> = GraphStore::query_nodes(&*store, NodeQuery::label(LABEL_URL))
        .iter()
        .filter_map(|node| {
            node.properties
                .get("state")
                .and_then(|value| value.as_str())
                .map(str::to_string)
        })
        .collect();
    assert!(states.contains("in_flight"), "root should be in_flight");
    assert!(states.contains("frontier"), "children should be in frontier");
}
