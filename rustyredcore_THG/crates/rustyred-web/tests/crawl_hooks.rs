//! Acceptance 6 for the self-organizing crawl frontier: flipping a `url` node
//! to `state=fetched` fires the fetch-completion hook, the page's entities
//! appear as graph nodes, and the frontier priorities change. Toggling the hook
//! off leaves the frontier static (the control case). Discovering a `links_to`
//! edge sets the target's initial priority.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustyred_thg_core::{
    EdgeRecord, HookDispatcher, HookDispatcherConfig, NodeQuery, NodeRecord, RedCoreGraphStore,
};
use rustyred_web::crawl_hooks;
use rustyred_web::frontier::model::{
    fingerprint, EDGE_LINKS_TO, LABEL_URL, STATE_FETCHED, STATE_FRONTIER,
};
use rustyred_web::WEB_ENTITY_LABEL;
use serde_json::json;

fn url_node(fp_hex: &str, url: &str, state: &str, content_hash: &str) -> NodeRecord {
    NodeRecord::new(
        fp_hex,
        [LABEL_URL],
        json!({
            "url": url,
            "state": state,
            "priority": 0.0,
            "depth": 0,
            "content_hash": content_hash,
        }),
    )
}

fn priority(store: &Arc<Mutex<RedCoreGraphStore>>, id: &str) -> f64 {
    store
        .lock()
        .unwrap()
        .get_node(id)
        .unwrap()
        .unwrap()
        .properties
        .get("priority")
        .and_then(|v| v.as_f64())
        .unwrap_or(f64::NAN)
}

fn set_state(store: &Arc<Mutex<RedCoreGraphStore>>, id: &str, state: &str) {
    let mut g = store.lock().unwrap();
    let mut node = g.get_node(id).unwrap().unwrap();
    node.properties
        .as_object_mut()
        .unwrap()
        .insert("state".to_string(), json!(state));
    g.upsert_node(node).unwrap();
}

/// Seed A -> B, C -> B with B central. Returns the (a, b, c) node ids.
fn seed_link_graph(store: &Arc<Mutex<RedCoreGraphStore>>) -> (String, String, String) {
    let a = fingerprint("GET", "https://example.com/a", b"").to_hex();
    let b = fingerprint("GET", "https://example.com/b", b"").to_hex();
    let c = fingerprint("GET", "https://example.com/c", b"").to_hex();
    let mut g = store.lock().unwrap();
    g.upsert_node(NodeRecord::new(
        "content_snapshot:hashA",
        ["ContentSnapshot"],
        json!({ "text": "Theorem Theorem Theorem powers Theseus and Memgraph across Rust." }),
    ))
    .unwrap();
    g.upsert_node(url_node(
        &a,
        "https://example.com/a",
        STATE_FRONTIER,
        "hashA",
    ))
    .unwrap();
    g.upsert_node(url_node(&b, "https://example.com/b", STATE_FRONTIER, ""))
        .unwrap();
    g.upsert_node(url_node(&c, "https://example.com/c", STATE_FRONTIER, ""))
        .unwrap();
    g.upsert_edge(EdgeRecord::new("l:a-b", &a, EDGE_LINKS_TO, &b, json!({})))
        .unwrap();
    g.upsert_edge(EdgeRecord::new("l:c-b", &c, EDGE_LINKS_TO, &b, json!({})))
        .unwrap();
    (a, b, c)
}

#[test]
fn fetched_page_reprioritizes_frontier_and_extracts_entities() {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    let (a, b, _c) = seed_link_graph(&store);

    // Wire hooks AFTER seeding so the seed writes do not trigger anything.
    let config = HookDispatcherConfig {
        debounce: Duration::from_millis(40),
        idle_poll: Duration::from_millis(10),
        ..Default::default()
    };
    let dispatcher = HookDispatcher::start(Arc::clone(&store), crawl_hooks(), config);
    {
        let mut g = store.lock().unwrap();
        g.attach_hook_emitter(dispatcher.emitter());
        g.set_hook_tenant("Travis-Gilbert");
    }

    let b_before = priority(&store, &b);
    assert_eq!(b_before, 0.0, "frontier starts at the static priority");

    set_state(&store, &a, STATE_FETCHED);
    assert!(dispatcher.quiesce(Duration::from_secs(10)), "hooks drained");

    // The fetched page was source-classified.
    let a_node = store.lock().unwrap().get_node(&a).unwrap().unwrap();
    assert!(
        a_node.properties.get("source_class").is_some(),
        "fetched page classified"
    );

    // The page's entities appear as graph nodes.
    let entities = store
        .lock()
        .unwrap()
        .query_nodes(NodeQuery::label(WEB_ENTITY_LABEL))
        .unwrap();
    assert!(!entities.is_empty(), "entities materialized into the graph");

    // The frontier pop order changed: central B now carries PPR priority.
    let b_after = priority(&store, &b);
    assert!(
        b_after > b_before,
        "frontier reprioritized: {b_before} -> {b_after}"
    );
}

/// Control case: with the hook off, the same fetch leaves the frontier static.
#[test]
fn without_hook_frontier_priority_is_unchanged() {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    let (a, b, _c) = seed_link_graph(&store);
    // No dispatcher attached.

    let b_before = priority(&store, &b);
    set_state(&store, &a, STATE_FETCHED);
    let b_after = priority(&store, &b);

    assert_eq!(b_before, 0.0);
    assert_eq!(
        b_after, 0.0,
        "no hook -> static priority, unchanged crawl order"
    );
}

#[test]
fn discovered_link_sets_initial_priority() {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    let parent = fingerprint("GET", "https://example.com/parent", b"").to_hex();
    let child = fingerprint("GET", "https://example.com/child", b"").to_hex();
    {
        let mut g = store.lock().unwrap();
        g.upsert_node(url_node(
            &parent,
            "https://example.com/parent",
            STATE_FRONTIER,
            "",
        ))
        .unwrap();
        g.upsert_node(url_node(
            &child,
            "https://example.com/child",
            STATE_FRONTIER,
            "",
        ))
        .unwrap();
    }

    let config = HookDispatcherConfig {
        debounce: Duration::from_millis(40),
        idle_poll: Duration::from_millis(10),
        ..Default::default()
    };
    let dispatcher = HookDispatcher::start(Arc::clone(&store), crawl_hooks(), config);
    {
        let mut g = store.lock().unwrap();
        g.attach_hook_emitter(dispatcher.emitter());
        g.set_hook_tenant("Travis-Gilbert");
    }

    assert_eq!(priority(&store, &child), 0.0);

    // Discover parent -> child.
    store
        .lock()
        .unwrap()
        .upsert_edge(EdgeRecord::new(
            "l:parent-child",
            &parent,
            EDGE_LINKS_TO,
            &child,
            json!({}),
        ))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(10)));

    assert!(
        priority(&store, &child) > 0.0,
        "discovered link seeded an initial frontier priority"
    );
}

/// End-to-end wiring over the real (tokio-async) frontier store: `attach_crawl_hooks`
/// bridges the async-mutex store to the sync hook worker via `blocking_lock`, and
/// a fetched page reprioritizes the frontier. Proves the CrawlRunner-facing path.
#[tokio::test(flavor = "multi_thread")]
async fn attach_crawl_hooks_bridges_async_store() {
    use rustyred_web::attach_crawl_hooks;
    use std::sync::Arc as StdArc;

    let store: rustyred_web::frontier::SharedFrontierStore =
        StdArc::new(tokio::sync::Mutex::new(RedCoreGraphStore::memory()));

    let a = fingerprint("GET", "https://example.com/a", b"").to_hex();
    let b = fingerprint("GET", "https://example.com/b", b"").to_hex();
    let c = fingerprint("GET", "https://example.com/c", b"").to_hex();
    {
        let mut g = store.lock().await;
        g.upsert_node(NodeRecord::new(
            "content_snapshot:hA",
            ["ContentSnapshot"],
            json!({ "text": "Theorem Theorem Theseus Memgraph Rust" }),
        ))
        .unwrap();
        g.upsert_node(url_node(&a, "https://example.com/a", STATE_FRONTIER, "hA"))
            .unwrap();
        g.upsert_node(url_node(&b, "https://example.com/b", STATE_FRONTIER, ""))
            .unwrap();
        g.upsert_node(url_node(&c, "https://example.com/c", STATE_FRONTIER, ""))
            .unwrap();
        g.upsert_edge(EdgeRecord::new("l:a-b", &a, EDGE_LINKS_TO, &b, json!({})))
            .unwrap();
        g.upsert_edge(EdgeRecord::new("l:c-b", &c, EDGE_LINKS_TO, &b, json!({})))
            .unwrap();
    }

    let dispatcher = StdArc::new(attach_crawl_hooks(StdArc::clone(&store), "Travis-Gilbert").await);

    // Fetch A.
    {
        let mut g = store.lock().await;
        let mut node = g.get_node(&a).unwrap().unwrap();
        node.properties
            .as_object_mut()
            .unwrap()
            .insert("state".to_string(), json!(STATE_FETCHED));
        g.upsert_node(node).unwrap();
    }

    // quiesce blocks, so run it off the runtime.
    {
        let d = StdArc::clone(&dispatcher);
        tokio::task::spawn_blocking(move || d.quiesce(Duration::from_secs(10)))
            .await
            .unwrap();
    }

    let b_priority = store
        .lock()
        .await
        .get_node(&b)
        .unwrap()
        .unwrap()
        .properties
        .get("priority")
        .and_then(|v| v.as_f64())
        .unwrap();
    assert!(
        b_priority > 0.0,
        "async-store fetch reprioritized the frontier"
    );
}
