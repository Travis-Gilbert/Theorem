//! Storage spine (handoff 6) acceptance criteria, as observable tests.
//!
//! Each test maps to one numbered criterion in
//! `theorem-harness-handoff-6-storage-spine.md`:
//!   1. evict-yet-survives-rehydration
//!   2. eviction is a frontier pop, not an O(n) walk (operation counter)
//!   3. cold tier durable across an operating-store restart
//!   4. parked warm scope absent until first access, then rehydrates whole
//!   5. eviction leaves the structural store version (the PPR cache key) intact

use rustyred_thg_core::{
    DiskColdIndex, DiskObjectStore, EdgeRecord, InMemoryGraphStore, InMemoryObjectStore,
    NodeRecord, OrderedColdIndex,
};
use rustyred_thg_memory::{
    evict_decayed, park_scope, recall_with_cold_tier, rehydrate, seed_frontier, ColdTier,
    DecayInput, MemoryRecallInput, HARNESS_MEMORY_LABEL, MEMORY_DOCUMENT_LABEL, SUPPORTS,
};
use serde_json::json;

fn mem_node(
    id: &str,
    tenant: &str,
    title: &str,
    content: &str,
    activation: f64,
    last_accessed_ms: i64,
) -> NodeRecord {
    NodeRecord::new(
        id,
        [HARNESS_MEMORY_LABEL, MEMORY_DOCUMENT_LABEL],
        json!({
            "tenant_id": tenant,
            "doc_id": id,
            "title": title,
            "content": content,
            "summary": content,
            "status": "active",
            "activation": activation,
            "fitness": { "score": 1.0 },
            "updated_at_ms": last_accessed_ms,
            "last_accessed_ms": last_accessed_ms,
        }),
    )
}

fn stale_decay(tenant: &str) -> DecayInput {
    // cutoff = now (10_000) - inactive (1_000) = 9_000: last_accessed <= 9_000 is stale.
    DecayInput {
        tenant_id: tenant.to_string(),
        now_ms: Some(10_000),
        inactive_after_ms: 1_000,
        activation_threshold: 0.1,
        ..DecayInput::default()
    }
}

// 1. A decayed node is removed from the operating store (count + RAM fall), yet
//    survives: a later recall that reaches its id rehydrates and returns it.
#[test]
fn decayed_node_is_evicted_yet_survives_rehydration() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(mem_node(
            "mem:cold",
            "theorem",
            "Stale",
            "old cold content",
            0.0,
            10,
        ))
        .unwrap();
    store
        .upsert_node(mem_node(
            "mem:hot",
            "theorem",
            "Fresh",
            "recent hot content",
            0.0,
            9_500,
        ))
        .unwrap();
    let mut cold = ColdTier::in_memory();
    let decay = stale_decay("theorem");
    seed_frontier(&store, &mut cold, &decay).unwrap();

    let nodes_before = store.stats().nodes_total;
    let bytes_before = store.stats().memory_bytes;
    let report = evict_decayed(&mut store, &mut cold, decay).unwrap();

    assert_eq!(report.evicted, 1);
    assert_eq!(report.evicted_nodes, vec!["mem:cold".to_string()]);
    // Removed from the operating store; count and RAM footprint fall.
    assert!(store.get_node("mem:cold").is_none());
    assert!(store.get_node("mem:hot").is_some());
    assert_eq!(store.stats().nodes_total, nodes_before - 1);
    assert!(
        store.stats().memory_bytes < bytes_before,
        "evicting a node must drop the operating store's RAM footprint"
    );
    // Durably retained in the cold tier.
    assert!(cold.index().lookup("mem:cold").unwrap().is_some());

    // A later recall that reaches its id rehydrates it and returns it.
    let recalled = recall_with_cold_tier(
        &mut store,
        &mut cold,
        MemoryRecallInput {
            tenant_id: "theorem".to_string(),
            seeds: vec!["mem:cold".to_string()],
            query: "cold".to_string(),
            top_k: 5,
            budget_tokens: 500,
            ..MemoryRecallInput::default()
        },
    )
    .unwrap();
    assert!(
        store.get_node("mem:cold").is_some(),
        "rehydrated back into the operating store"
    );
    assert!(
        recalled.memories.iter().any(|m| m.graph_id == "mem:cold"),
        "rehydrated node is returned by recall"
    );
    // No longer cold.
    assert!(cold.index().lookup("mem:cold").unwrap().is_none());
}

// SPEC-RUSTYRED-RELATIONAL-CORE sec6 / acceptance #1: the native ordered.rs-
// backed cold index (`OrderedColdIndex`) is a drop-in for the eviction path.
// Evict-yet-rehydrate works identically to `InMemoryColdIndex` with no sqlx
// call and no network hop, and scope->ids is served from `ordered.rs`
// (`OrderedIndex`) rather than a directory or table scan.
#[test]
fn ordered_cold_index_is_a_drop_in_for_evict_and_rehydrate() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(mem_node(
            "mem:cold",
            "theorem",
            "Stale",
            "old cold content",
            0.0,
            10,
        ))
        .unwrap();
    store
        .upsert_node(mem_node(
            "mem:hot",
            "theorem",
            "Fresh",
            "recent hot content",
            0.0,
            9_500,
        ))
        .unwrap();

    // The ordered.rs-backed cold index. Keep a shared handle (it is `Arc`-backed,
    // so the clone shares state) to read scope->ids after eviction.
    let index = OrderedColdIndex::new();
    let mut cold = ColdTier::new(
        Box::new(InMemoryObjectStore::new()),
        Box::new(index.clone()),
    );
    let decay = stale_decay("theorem");
    seed_frontier(&store, &mut cold, &decay).unwrap();

    let report = evict_decayed(&mut store, &mut cold, decay).unwrap();
    assert_eq!(report.evicted, 1);
    assert_eq!(report.evicted_nodes, vec!["mem:cold".to_string()]);
    assert!(store.get_node("mem:cold").is_none());
    assert!(store.get_node("mem:hot").is_some());

    // Durable in the ordered.rs-backed cold tier: id->residency is a keyed
    // lookup, and scope->ids comes off the `OrderedIndex`.
    assert!(cold.index().lookup("mem:cold").unwrap().is_some());
    assert_eq!(
        index.ids_for_scope("theorem", 0).unwrap(),
        vec!["mem:cold".to_string()]
    );

    // A recall reaching the id rehydrates it back into the operating store.
    let recalled = recall_with_cold_tier(
        &mut store,
        &mut cold,
        MemoryRecallInput {
            tenant_id: "theorem".to_string(),
            seeds: vec!["mem:cold".to_string()],
            query: "cold".to_string(),
            top_k: 5,
            budget_tokens: 500,
            ..MemoryRecallInput::default()
        },
    )
    .unwrap();
    assert!(
        store.get_node("mem:cold").is_some(),
        "rehydrated back into the operating store"
    );
    assert!(recalled.memories.iter().any(|m| m.graph_id == "mem:cold"));
    // Rehydration clears both the keyed map and the scope ordered index.
    assert!(cold.index().lookup("mem:cold").unwrap().is_none());
    assert!(index.ids_for_scope("theorem", 0).unwrap().is_empty());
}

// 2. Eviction is a pop, not a scan: evicting k cold nodes out of n performs
//    O(k log n) ordered-index work, verified by the frontier operation counter
//    and by candidates_examined == k (not n) -- the fresh nodes above the cutoff
//    are never visited.
#[test]
fn eviction_is_a_frontier_pop_not_an_on_walk() {
    let mut store = InMemoryGraphStore::new();
    let n: usize = 200;
    let k: usize = 5;
    for i in 0..k {
        store
            .upsert_node(mem_node(
                &format!("mem:cold:{i}"),
                "theorem",
                "stale",
                "cold",
                0.0,
                10,
            ))
            .unwrap();
    }
    for i in 0..(n - k) {
        // last_accessed 100_000 is far above the cutoff -> never a candidate.
        store
            .upsert_node(mem_node(
                &format!("mem:hot:{i}"),
                "theorem",
                "fresh",
                "hot",
                0.0,
                100_000,
            ))
            .unwrap();
    }
    let mut cold = ColdTier::in_memory();
    let decay = DecayInput {
        tenant_id: "theorem".to_string(),
        now_ms: Some(60_000),
        inactive_after_ms: 1_000, // cutoff = 59_000
        activation_threshold: 0.1,
        ..DecayInput::default()
    };
    seed_frontier(&store, &mut cold, &decay).unwrap();

    let report = evict_decayed(&mut store, &mut cold, decay).unwrap();

    assert_eq!(report.evicted, k);
    // The frontier returned exactly the k cold nodes; the n-k fresh nodes above
    // the cutoff were never examined (early-stop range, not a scan).
    assert_eq!(report.candidates_examined, k);
    assert_eq!(report.scanned_nodes, 0);
    // O(k log n) ordered-index ops -- a small multiple of k, far below n.
    assert!(
        report.frontier_ops <= 3 * k,
        "frontier ops {} should be O(k), not O(n={n})",
        report.frontier_ops
    );
    assert!(report.frontier_ops < n);
    // The fresh working set is untouched.
    assert_eq!(store.stats().nodes_total, n - k);
}

// 3. The cold tier is durable across an operating-store restart: clear the
//    operating store, and a cold id rehydrates from the disk-backed object store
//    with its payload intact.
#[test]
fn cold_tier_is_durable_across_operating_store_restart() {
    let dir = std::env::temp_dir().join(format!("spine-durable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // Phase 1: evict to a disk-backed cold tier, then drop the operating store.
    {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(mem_node(
                "mem:durable",
                "theorem",
                "Durable",
                "survives restart",
                0.0,
                10,
            ))
            .unwrap();
        let mut cold = ColdTier::new(
            Box::new(DiskObjectStore::open(&dir).unwrap()),
            Box::new(DiskColdIndex::open(&dir).unwrap()),
        );
        let decay = stale_decay("theorem");
        seed_frontier(&store, &mut cold, &decay).unwrap();
        assert_eq!(
            evict_decayed(&mut store, &mut cold, decay).unwrap().evicted,
            1
        );
    } // store + cold dropped: the operating store is gone.

    // Phase 2: a fresh empty operating store + a cold tier reopened from disk.
    let mut store2 = InMemoryGraphStore::new();
    assert!(store2.get_node("mem:durable").is_none());
    let mut cold2 = ColdTier::new(
        Box::new(DiskObjectStore::open(&dir).unwrap()),
        Box::new(DiskColdIndex::open(&dir).unwrap()),
    );
    assert!(rehydrate(&mut store2, &mut cold2, "mem:durable").unwrap());
    let node = store2.get_node("mem:durable").unwrap();
    assert_eq!(
        node.properties.get("title").and_then(|v| v.as_str()),
        Some("Durable")
    );
    assert_eq!(
        node.properties.get("content").and_then(|v| v.as_str()),
        Some("survives restart")
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// 4. A parked warm scope is absent from the operating store until first access,
//    then rehydrates whole; the round-trip preserves the subgraph (nodes+edges).
#[test]
fn parked_warm_scope_is_absent_until_first_access_then_rehydrates_whole() {
    let mut store = InMemoryGraphStore::new();
    for id in ["scope:a", "scope:b", "scope:c"] {
        store
            .upsert_node(mem_node(id, "theorem", id, "scope content", 0.5, 1_000))
            .unwrap();
    }
    store
        .upsert_edge(EdgeRecord::new(
            "edge:ab",
            "scope:a",
            SUPPORTS,
            "scope:b",
            json!({ "tenant_id": "theorem" }),
        ))
        .unwrap();
    store
        .upsert_edge(EdgeRecord::new(
            "edge:bc",
            "scope:b",
            SUPPORTS,
            "scope:c",
            json!({ "tenant_id": "theorem" }),
        ))
        .unwrap();
    let mut cold = ColdTier::in_memory();

    let node_ids = vec![
        "scope:a".to_string(),
        "scope:b".to_string(),
        "scope:c".to_string(),
    ];
    park_scope(&mut store, &mut cold, "scope:test", &node_ids).unwrap();

    // Absent from the operating store until first access.
    for id in &node_ids {
        assert!(store.get_node(id).is_none(), "{id} should be parked");
    }
    assert!(store.get_edge("edge:ab").is_none());
    assert_eq!(store.stats().nodes_total, 0);
    assert_eq!(store.stats().edges_total, 0);

    // First access to any member rehydrates the whole scope.
    assert!(rehydrate(&mut store, &mut cold, "scope:a").unwrap());
    for id in &node_ids {
        assert!(store.get_node(id).is_some(), "{id} should be rehydrated");
    }
    // The subgraph is preserved: edges are restored.
    assert!(store.get_edge("edge:ab").is_some());
    assert!(store.get_edge("edge:bc").is_some());
}

// 5. Eviction does not bust the PPR cache for live nodes: it leaves the
//    structural store version (the cache key) unchanged. Rehydration likewise.
#[test]
fn eviction_leaves_the_structural_store_version_unchanged() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(mem_node("mem:cold", "theorem", "stale", "cold", 0.0, 10))
        .unwrap();
    store
        .upsert_node(mem_node("mem:live", "theorem", "live", "hot", 0.0, 9_500))
        .unwrap();
    let mut cold = ColdTier::in_memory();
    let decay = stale_decay("theorem");
    seed_frontier(&store, &mut cold, &decay).unwrap();

    let version_before = store.stats().version;
    let report = evict_decayed(&mut store, &mut cold, decay).unwrap();
    assert_eq!(report.evicted, 1);
    // A residency change, not a logical mutation: the PPR-cache key is unchanged.
    assert_eq!(
        store.stats().version,
        version_before,
        "eviction must not bump the structural store version"
    );

    // Rehydration is version-neutral too.
    assert!(rehydrate(&mut store, &mut cold, "mem:cold").unwrap());
    assert_eq!(
        store.stats().version,
        version_before,
        "rehydration must not bump the structural store version"
    );
}
