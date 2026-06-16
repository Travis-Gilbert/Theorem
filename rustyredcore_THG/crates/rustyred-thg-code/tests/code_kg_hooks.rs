//! Acceptance for the code-KG hooks: after an ingest burst, the centrality and
//! embedding hooks have warmed derived structure onto the `CodeSymbol` nodes
//! inside the store, off the writer's path.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustyred_thg_code::{
    builtin_code_plugin_registry, code_kg_hooks, start_code_kg_dispatcher, CENTRALITY_PROPERTY,
    EMBEDDING_DIM, EMBEDDING_PROPERTY,
};
use rustyred_thg_core::{
    EdgeRecord, HookDispatcher, HookDispatcherConfig, NodeRecord, RedCoreGraphStore,
};
use serde_json::json;

fn symbol(id: &str, repo: &str, name: &str) -> NodeRecord {
    NodeRecord::new(
        id,
        ["CodeSymbol"],
        json!({
            "repo_id": repo,
            "name": name,
            "signature": format!("fn {name}(input: i32) -> i32"),
            "snippet": format!("fn {name}() {{ /* {name} body */ }}"),
        }),
    )
}

fn wired_store() -> (Arc<Mutex<RedCoreGraphStore>>, HookDispatcher) {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    let config = HookDispatcherConfig {
        debounce: Duration::from_millis(60),
        idle_poll: Duration::from_millis(10),
        ..Default::default()
    };
    let dispatcher = HookDispatcher::start(Arc::clone(&store), code_kg_hooks(), config);
    {
        let mut guard = store.lock().unwrap();
        guard.attach_hook_emitter(dispatcher.emitter());
        guard.set_hook_tenant("Travis-Gilbert");
    }
    (store, dispatcher)
}

fn centrality(store: &Arc<Mutex<RedCoreGraphStore>>, id: &str) -> f64 {
    store
        .lock()
        .unwrap()
        .get_node(id)
        .unwrap()
        .unwrap()
        .properties
        .get(CENTRALITY_PROPERTY)
        .and_then(|v| v.as_f64())
        .unwrap_or_else(|| panic!("no centrality on {id}"))
}

/// Acceptance 1: centrality is populated after an ingest completes, and the
/// embedding is warmed onto the same nodes. The most-depended-upon symbol ranks
/// highest.
#[test]
fn centrality_and_embeddings_warm_after_ingest() {
    let (store, dispatcher) = wired_store();

    // s1 -> s2, s1 -> s3, s2 -> s3 (s3 is the most depended-upon).
    {
        let mut g = store.lock().unwrap();
        for (id, name) in [
            ("code:symbol:s1", "alpha"),
            ("code:symbol:s2", "beta"),
            ("code:symbol:s3", "gamma"),
        ] {
            g.upsert_node(symbol(id, "repo:test", name)).unwrap();
        }
        for (eid, from, to) in [
            ("code:edge:e1", "code:symbol:s1", "code:symbol:s2"),
            ("code:edge:e2", "code:symbol:s1", "code:symbol:s3"),
            ("code:edge:e3", "code:symbol:s2", "code:symbol:s3"),
        ] {
            g.upsert_edge(EdgeRecord::new(eid, from, "CALLS_SYMBOL", to, json!({})))
                .unwrap();
        }
    }

    assert!(dispatcher.quiesce(Duration::from_secs(10)), "hooks drained");

    let g = store.lock().unwrap();
    for id in ["code:symbol:s1", "code:symbol:s2", "code:symbol:s3"] {
        let node = g.get_node(id).unwrap().expect("symbol present");
        assert!(
            node.properties
                .get(CENTRALITY_PROPERTY)
                .and_then(|v| v.as_f64())
                .is_some(),
            "centrality warmed for {id}"
        );
        let dims = node
            .properties
            .get(EMBEDDING_PROPERTY)
            .and_then(|v| v.as_array())
            .map(|a| a.len());
        assert_eq!(dims, Some(EMBEDDING_DIM), "embedding warmed for {id}");
    }
    drop(g);

    // s3 is called by both s1 and s2, so PPR mass concentrates there.
    assert!(
        centrality(&store, "code:symbol:s3") >= centrality(&store, "code:symbol:s2"),
        "most depended-upon symbol ranks highest"
    );

    // The embedding designation is live, so semantic search is queryable.
    let designated = store
        .lock()
        .unwrap()
        .vector_designations()
        .into_iter()
        .any(|d| d.label == "CodeSymbol" && d.property == EMBEDDING_PROPERTY);
    assert!(designated, "embedding vector designation registered");
}

/// The embedding refreshes when a symbol's signature changes, with no batch
/// backfill job — and a re-ingest with identical text does not loop.
#[test]
fn embedding_refreshes_on_signature_change() {
    let (store, dispatcher) = wired_store();

    store
        .lock()
        .unwrap()
        .upsert_node(symbol("code:symbol:x", "repo:test", "widget"))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(10)));

    let first = store
        .lock()
        .unwrap()
        .get_node("code:symbol:x")
        .unwrap()
        .unwrap()
        .properties
        .get(EMBEDDING_PROPERTY)
        .and_then(|v| v.as_array())
        .cloned()
        .expect("embedding warmed");

    // Change the signature -> embedding should move.
    {
        let mut g = store.lock().unwrap();
        let mut node = g.get_node("code:symbol:x").unwrap().unwrap();
        node.properties.as_object_mut().unwrap().insert(
            "signature".to_string(),
            json!("fn widget(a: i32, b: i32, c: i32) -> String"),
        );
        g.upsert_node(node).unwrap();
    }
    assert!(dispatcher.quiesce(Duration::from_secs(10)));

    let second = store
        .lock()
        .unwrap()
        .get_node("code:symbol:x")
        .unwrap()
        .unwrap()
        .properties
        .get(EMBEDDING_PROPERTY)
        .and_then(|v| v.as_array())
        .cloned()
        .expect("embedding present");

    assert_eq!(second.len(), EMBEDDING_DIM);
    assert_ne!(first, second, "embedding refreshed on signature change");
}

/// The plugin registry collects the code-KG hooks, so an embedder wires them
/// through the same `PluginRegistry::hooks()` path as operations.
#[test]
fn registry_collects_code_kg_hooks() {
    let names: Vec<String> = builtin_code_plugin_registry()
        .hooks()
        .into_iter()
        .map(|hook| hook.name)
        .collect();
    assert!(
        names.iter().any(|n| n == "code.incremental_centrality"),
        "centrality hook registered, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "code.incremental_embed"),
        "embed hook registered, got {names:?}"
    );
    assert_eq!(code_kg_hooks().len(), names.len());
}

/// The public one-call embedder wiring (`start_code_kg_dispatcher`) attaches the
/// emitter and warms centrality on commit.
#[test]
fn start_code_kg_dispatcher_wires_and_warms() {
    let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
    let dispatcher = start_code_kg_dispatcher(Arc::clone(&store));

    store
        .lock()
        .unwrap()
        .upsert_node(symbol("code:symbol:solo", "repo:test", "solo"))
        .unwrap();
    assert!(dispatcher.quiesce(Duration::from_secs(10)));

    let node = store
        .lock()
        .unwrap()
        .get_node("code:symbol:solo")
        .unwrap()
        .unwrap();
    assert!(
        node.properties.get(CENTRALITY_PROPERTY).is_some(),
        "centrality warmed via start_code_kg_dispatcher"
    );
}
