//! Acceptance: the memory graph computes semantic edges over memory docs.
//!
//! Two planted topic clusters (Postgres/database vs Servo/browser) with disjoint
//! vocabulary. The builder must link within a cluster and never across it, respect
//! k and the score threshold, stay tenant-scoped, and be idempotent on re-run.

use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeRecord};
use rustyred_thg_memory::{
    compute_memory_similarity_edges, HashEmbedder, SimilarityOptions, MEMORY_DOCUMENT_LABEL,
    MEMORY_SIMILAR,
};
use serde_json::json;

fn add_doc(store: &mut InMemoryGraphStore, id: &str, tenant: &str, title: &str, content: &str) {
    add_doc_with_kind(store, id, tenant, "note", title, content);
}

fn add_doc_with_kind(
    store: &mut InMemoryGraphStore,
    id: &str,
    tenant: &str,
    kind: &str,
    title: &str,
    content: &str,
) {
    store
        .upsert_node(NodeRecord::new(
            id,
            [MEMORY_DOCUMENT_LABEL],
            json!({
                "tenant_id": tenant,
                "tenant_slug": tenant,
                "kind": kind,
                "title": title,
                "summary": "",
                "content": content,
                "status": "active",
            }),
        ))
        .unwrap();
}

fn similar_ids(store: &InMemoryGraphStore, id: &str) -> Vec<String> {
    let mut ids: Vec<String> = store
        .neighbors(NeighborQuery::out(id))
        .into_iter()
        .filter(|hit| hit.edge_type == MEMORY_SIMILAR)
        .map(|hit| hit.node_id)
        .collect();
    ids.sort();
    ids
}

fn seeded_store() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::new();
    // Cluster A: Postgres / database (disjoint vocabulary from cluster B).
    add_doc(&mut store, "doc_pg1", "t", "Postgres tuning", "postgres database index tuning speeds queries");
    add_doc(&mut store, "doc_pg2", "t", "Database speed", "database queries faster postgres index tuning");
    // Cluster B: Servo / browser.
    add_doc(&mut store, "doc_sv1", "t", "Servo render", "servo browser renders pages graph");
    add_doc(&mut store, "doc_sv2", "t", "Browser ingest", "browser servo ingest pages render graph");
    // A doc in a different tenant, vocabulary matching cluster A.
    add_doc(&mut store, "doc_other", "other", "Postgres elsewhere", "postgres database index tuning queries");
    store
}

fn options() -> SimilarityOptions {
    SimilarityOptions {
        k: 3,
        min_score: 0.1,
        ..Default::default()
    }
}

#[test]
fn computes_intra_cluster_edges_and_none_across_clusters() {
    let mut store = seeded_store();
    let stats =
        compute_memory_similarity_edges(&mut store, "t", &HashEmbedder::new(256), &options()).unwrap();

    assert_eq!(stats.docs, 4, "only the four tenant-t docs are enumerated (not the 'other' tenant)");

    // Postgres doc links to the other Postgres doc, and to neither browser doc.
    assert_eq!(similar_ids(&store, "doc_pg1"), vec!["doc_pg2".to_string()]);
    assert_eq!(similar_ids(&store, "doc_pg2"), vec!["doc_pg1".to_string()]);
    // Servo doc links to the other Servo doc only.
    assert_eq!(similar_ids(&store, "doc_sv1"), vec!["doc_sv2".to_string()]);
    assert_eq!(similar_ids(&store, "doc_sv2"), vec!["doc_sv1".to_string()]);

    // 4 directed edges total: pg1<->pg2, sv1<->sv2.
    assert_eq!(stats.edges_written, 4);
}

#[test]
fn is_tenant_scoped() {
    let mut store = seeded_store();
    compute_memory_similarity_edges(&mut store, "t", &HashEmbedder::new(256), &options()).unwrap();

    // A Postgres doc in tenant t never links to the same-topic doc in tenant 'other'.
    assert!(
        !similar_ids(&store, "doc_pg1").contains(&"doc_other".to_string()),
        "edges must not cross the tenant boundary"
    );
    // The other tenant's doc got no edges from this run.
    assert!(similar_ids(&store, "doc_other").is_empty());
}

#[test]
fn re_run_is_idempotent() {
    let mut store = seeded_store();
    let opts = options();
    let first =
        compute_memory_similarity_edges(&mut store, "t", &HashEmbedder::new(256), &opts).unwrap();
    let before = similar_ids(&store, "doc_pg1");
    let second =
        compute_memory_similarity_edges(&mut store, "t", &HashEmbedder::new(256), &opts).unwrap();
    let after = similar_ids(&store, "doc_pg1");

    assert_eq!(first, second, "stats stable across runs");
    assert_eq!(before, after, "neighbor set unchanged; deterministic edge ids overwrite, no duplicates");
    assert_eq!(after, vec!["doc_pg2".to_string()]);
}

#[test]
fn the_score_threshold_prunes_weak_links() {
    // With an impossibly high threshold, even the strong intra-cluster pair is dropped.
    let mut store = seeded_store();
    let stats = compute_memory_similarity_edges(
        &mut store,
        "t",
        &HashEmbedder::new(256),
        &SimilarityOptions {
            k: 8,
            min_score: 0.999,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(stats.edges_written, 0, "nothing clears a near-1.0 cosine threshold");
    assert!(similar_ids(&store, "doc_pg1").is_empty());
}

#[test]
fn kind_filter_excludes_internal_docs_and_can_allowlist_real_kinds() {
    let mut store = seeded_store();
    add_doc_with_kind(
        &mut store,
        "doc_orchestrate",
        "t",
        "orchestrate",
        "Agent exhaust",
        "postgres database index tuning queries",
    );
    add_doc_with_kind(
        &mut store,
        "doc_decision",
        "t",
        "decision",
        "Database decision",
        "postgres database index tuning queries",
    );

    let stats =
        compute_memory_similarity_edges(&mut store, "t", &HashEmbedder::new(256), &options()).unwrap();

    assert_eq!(
        stats.docs, 5,
        "default options exclude graph-internal orchestrate docs before embedding"
    );
    assert!(
        similar_ids(&store, "doc_orchestrate").is_empty(),
        "excluded docs must not receive MEMORY_SIMILAR edges"
    );

    let mut allowlisted = seeded_store();
    add_doc_with_kind(
        &mut allowlisted,
        "doc_decision",
        "t",
        "decision",
        "Database decision",
        "postgres database index tuning queries",
    );
    let opts = SimilarityOptions {
        include_kinds: vec!["decision".to_string()],
        exclude_kinds: Vec::new(),
        ..options()
    };
    let stats =
        compute_memory_similarity_edges(&mut allowlisted, "t", &HashEmbedder::new(256), &opts).unwrap();
    assert_eq!(stats.docs, 1, "include_kinds limits the source set");
    assert_eq!(stats.edges_written, 0, "one allowed doc has no neighbor to link");
}
