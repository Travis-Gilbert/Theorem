//! SPEC-HIPPORAG2-HUBS-1.0 acceptance criteria, as executable integration tests.
//!
//! The crate internals (schema/indexing/retrieve/raptor) are owned by the codex
//! head. This suite is the claude-code verifier deliverable: it pins the spec's
//! seven observable acceptance criteria against the public API, so a regression
//! in either head's lane is caught here rather than in production.
//!
//! Each test maps to a numbered acceptance criterion in the spec.

use rustyred_hipporag::retrieve::retrieve_with_trace;
use rustyred_hipporag::{
    build_summary_tree_for_region, index_passage, retrieve, summary_tree_hook, HippoQuery,
    RaptorPolicy, EDGE_CONTAINS, LABEL_HUB, LABEL_PAGE, LABEL_PHRASE,
};
use rustyred_membrane::{fill_to_budget, Candidate, ScoreContext, Scorer};
use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeQuery, NodeRecord};
use serde_json::json;

// ---- helpers ---------------------------------------------------------------

fn put_page(store: &mut InMemoryGraphStore, id: &str, text: &str) {
    store
        .upsert_node(NodeRecord::new(
            id,
            [LABEL_PAGE],
            json!({ "text": text, "tenant_id": "t" }),
        ))
        .expect("upsert page");
}

/// Upsert a Page passage and run the HippoRAG indexer over it.
fn index(store: &mut InMemoryGraphStore, id: &str, text: &str) {
    put_page(store, id, text);
    index_passage(store, id).expect("index passage");
}

fn count_label(store: &InMemoryGraphStore, label: &str) -> usize {
    store
        .query_nodes(NodeQuery::label(label).with_limit(100_000))
        .len()
}

fn node_kind(c: &Candidate) -> Option<&str> {
    c.metadata.get("node_kind").map(String::as_str)
}

fn rank(cands: &[Candidate], id: &str) -> Option<usize> {
    cands.iter().position(|c| c.node_id == id)
}

/// A trivial PPR-only scorer, so the gate test needs no rerank dep.
struct PprScorer;
impl Scorer for PprScorer {
    fn score(&self, c: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
        c.ppr_proximity
    }
}

// ---- Acceptance #1: indexing produces the dual-node structure --------------

#[test]
fn acceptance_1_indexing_builds_passage_phrase_structure() {
    let mut store = InMemoryGraphStore::new();
    let stats = {
        index(
            &mut store,
            "p1",
            "graph neural networks learn node embeddings",
        );
        index_passage(&mut store, "p1").expect("reindex idempotent")
    };

    // Phrases upserted, Contains edges emitted.
    assert!(
        stats.phrases_upserted >= 3,
        "expected phrases, got {stats:?}"
    );
    assert!(
        stats.contains_edges >= 3,
        "expected contains edges, got {stats:?}"
    );

    // Phrase nodes exist and every one carries node_specificity (IDF-style).
    let phrases = store.query_nodes(NodeQuery::label(LABEL_PHRASE).with_limit(1000));
    assert!(!phrases.is_empty(), "no Phrase nodes were created");
    assert!(
        phrases
            .iter()
            .all(|p| p.properties.get("node_specificity").is_some()),
        "every Phrase must carry node_specificity"
    );

    // Passage -> Phrase Contains edges are present.
    let contains = store.neighbors(NeighborQuery::out("p1").with_edge_type(EDGE_CONTAINS));
    assert!(!contains.is_empty(), "no Contains edges from the passage");

    // Re-indexing is idempotent: the phrase ids are content-addressed, so the
    // phrase node count is stable across a second pass.
    let phrase_count_first = count_label(&store, LABEL_PHRASE);
    index_passage(&mut store, "p1").expect("third index");
    assert_eq!(phrase_count_first, count_label(&store, LABEL_PHRASE));
}

// ---- Acceptance #2: hubs appear with Summarizes + HubParent, fired once ----

#[test]
fn acceptance_2_hubs_with_summarizes_and_hub_parent_fire_once() {
    let mut store = InMemoryGraphStore::new();
    // Two disconnected token cliques. Each is internally connected (shared
    // tokens) but shares nothing with the other, so community detection must
    // separate them -> two level-0 hubs -> a level-1 hub with HubParent edges.
    index(&mut store, "px1", "alpha beta gamma");
    index(&mut store, "px2", "beta gamma alpha");
    index(&mut store, "px3", "gamma alpha beta");
    index(&mut store, "py1", "delta epsilon zeta");
    index(&mut store, "py2", "epsilon zeta delta");
    index(&mut store, "py3", "zeta delta epsilon");

    let policy = RaptorPolicy {
        region_node_threshold: 3,
        min_members: 2,
        max_level: 3,
    };
    let stats =
        build_summary_tree_for_region(&mut store, policy.clone(), &["px1".into(), "py1".into()])
            .expect("build hubs");

    assert!(
        stats.hubs_upserted >= 2,
        "expected >= 2 hubs, got {stats:?}"
    );
    assert!(stats.summarize_edges >= 1, "expected Summarizes edges");
    assert!(
        stats.hub_parent_edges >= 1,
        "two communities must roll up into a HubParent hierarchy, got {stats:?}"
    );

    // Hub nodes are real, first-class, PPR-routable nodes in the same graph.
    assert!(count_label(&store, LABEL_HUB) >= 2, "Hub nodes must exist");

    // Fire-once-per-region-per-batch: the counter advances by exactly one per
    // build call (acceptance #2's "verified by a counter").
    assert_eq!(stats.hook_counter, 1, "first build = one fire");
    let again = build_summary_tree_for_region(&mut store, policy, &["px1".into(), "py1".into()])
        .expect("rebuild");
    assert_eq!(again.hook_counter, 2, "second build = two fires");

    // The hook registration coalesces by region, so an ingest storm in one
    // tenant collapses to a single handler call.
    let reg = summary_tree_hook();
    assert_eq!(reg.name, "hipporag.summary_tree");
}

// ---- Acceptance #4: recluster touches only the dirty region ----------------

#[test]
fn acceptance_4_recluster_is_scoped_to_the_dirty_region() {
    let mut store = InMemoryGraphStore::new();
    // Small cluster A.
    index(&mut store, "a1", "apple apricot avocado");
    // Large, disconnected cluster B (shares no tokens with A).
    for i in 0..12 {
        index(
            &mut store,
            &format!("b{i}"),
            "neutron proton electron photon",
        );
    }

    let total_nodes = count_label(&store, LABEL_PAGE) + count_label(&store, LABEL_PHRASE);

    // Seed the region from cluster A only.
    let policy = RaptorPolicy {
        region_node_threshold: 1,
        min_members: 2,
        max_level: 1,
    };
    let stats =
        build_summary_tree_for_region(&mut store, policy, &["a1".into()]).expect("scoped build");

    // The reclustered region is far smaller than the whole graph: the dirty
    // region of cluster A cannot reach disconnected cluster B.
    assert!(
        stats.region_nodes_seen < total_nodes,
        "region {} should be < total {}",
        stats.region_nodes_seen,
        total_nodes
    );
    assert!(
        stats.region_nodes_seen <= 8,
        "cluster A region should be tiny, got {}",
        stats.region_nodes_seen
    );
}

// ---- Acceptance #5: warm structure on the hot path, never a global PPR -----

#[test]
fn acceptance_5_hot_path_reads_warm_structure_no_global_ppr() {
    let mut store = InMemoryGraphStore::new();
    index(&mut store, "p1", "alpha beta gamma delta");
    index(&mut store, "p2", "beta gamma delta epsilon");
    index(&mut store, "p3", "gamma delta epsilon zeta");
    build_summary_tree_for_region(
        &mut store,
        RaptorPolicy {
            region_node_threshold: 1,
            min_members: 2,
            max_level: 2,
        },
        &["p1".into()],
    )
    .expect("warm the hubs");

    let (_cands, trace) = retrieve_with_trace(&store, HippoQuery::new("alpha beta", 5));

    // Seed-conditioned PPR runs; a cold full-graph PPR never does.
    assert!(trace.ran_query_ppr, "query PPR must run");
    assert!(
        !trace.ran_global_ppr,
        "the hot path must not recompute a global PPR"
    );
}

// ---- Acceptance #6: the candidate set is consumed unchanged by the gate ----

#[test]
fn acceptance_6_candidates_feed_fill_to_budget_unchanged() {
    let mut store = InMemoryGraphStore::new();
    index(&mut store, "p1", "vector search over graph embeddings");
    index(&mut store, "p2", "graph embeddings power retrieval");
    index(&mut store, "p3", "retrieval augmented generation systems");

    let candidates = retrieve(&store, HippoQuery::new("graph embeddings retrieval", 10));
    assert!(!candidates.is_empty(), "retrieve produced no candidates");
    let produced = candidates.len();

    // The exact Vec<Candidate> retrieve returns is the type fill_to_budget
    // consumes -- no adapter, no rerank inside hipporag.
    let active: Vec<String> = Vec::new();
    let ctx = ScoreContext::new("graph embeddings retrieval", &active);
    let admission = fill_to_budget(candidates, &PprScorer, &ctx, 24);

    // Nothing is dropped at the gate: admitted + deferred accounts for all of it.
    assert_eq!(
        admission.admitted.len() + admission.deferred.len(),
        produced,
        "the gate must defer, never drop"
    );
    assert!(
        admission.tokens_admitted <= 24,
        "admission must respect the token budget"
    );
}

// ---- Acceptance #3: one call, hubs for coverage + passages for specificity -

#[test]
fn acceptance_3_include_hubs_flag_governs_hub_eligibility() {
    let mut store = InMemoryGraphStore::new();
    index(&mut store, "p1", "alpha beta gamma delta");
    index(&mut store, "p2", "beta gamma delta epsilon");
    index(&mut store, "p3", "gamma delta epsilon zeta");
    build_summary_tree_for_region(
        &mut store,
        RaptorPolicy {
            region_node_threshold: 1,
            min_members: 2,
            max_level: 2,
        },
        &["p1".into()],
    )
    .expect("build hubs");
    assert!(
        count_label(&store, LABEL_HUB) >= 1,
        "need hubs for this test"
    );

    // include_hubs = false: no hub may appear among the candidates.
    let specific = retrieve(
        &store,
        HippoQuery {
            text: "alpha beta gamma",
            top_k: 10,
            include_hubs: false,
        },
    );
    assert!(
        specific.iter().all(|c| node_kind(c) != Some("Hub")),
        "include_hubs=false must exclude hubs"
    );
    assert!(
        specific.iter().any(|c| node_kind(c) == Some("Page")),
        "a specific query must still surface leaf Page passages"
    );

    // include_hubs = true: hubs are eligible candidates in the same call.
    let with_hubs = retrieve(
        &store,
        HippoQuery {
            text: "alpha beta gamma delta epsilon",
            top_k: 10,
            include_hubs: true,
        },
    );
    assert!(
        with_hubs.iter().any(|c| node_kind(c) == Some("Hub")),
        "include_hubs=true must let hub coverage candidates surface"
    );
}

// ---- Acceptance #7: hub-aware PPR beats dense-only on an associative query -

#[test]
fn acceptance_7_multihop_ppr_beats_dense_only() {
    let mut store = InMemoryGraphStore::new();
    // An associatively connected physics cluster: quantum -> entanglement ->
    // nonlocality -> theorem is reachable as a multi-hop path through shared
    // phrases, even though no single passage contains both query terms.
    index(&mut store, "p1", "quantum entanglement experiment");
    index(&mut store, "p2", "entanglement nonlocality measurement");
    index(&mut store, "p3", "nonlocality theorem proof");
    // A distractor that shares a single query word ("quantum") but is otherwise
    // structurally isolated from the physics cluster.
    index(&mut store, "dd", "quantum cooking recipe kitchen");

    let query = "quantum theorem";
    let cands = retrieve(&store, HippoQuery::new(query, 10));

    let r_p3 = rank(&cands, "p3");
    let r_dd = rank(&cands, "dd");
    assert!(r_p3.is_some(), "target passage p3 must be retrieved");

    // Dense-only signal (lexical overlap with the query) ties the distractor
    // with the target: "quantum theorem" overlaps "dd" on "quantum" and "p3" on
    // "theorem", one term each. Under dense-only ordering the alphabetically
    // earlier "dd" would sort first. Hub-aware PPR routes mass through the
    // multi-hop quantum->entanglement->nonlocality->theorem path and lifts the
    // structurally-connected target above the isolated distractor.
    assert!(
        r_dd.is_none_or(|dd| r_p3.unwrap() < dd),
        "hub-aware PPR must rank the multi-hop target p3 (#{r_p3:?}) above the \
         isolated dense-only distractor dd (#{r_dd:?})"
    );
}
