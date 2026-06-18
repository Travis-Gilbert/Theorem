//! Acceptance suite for the shadow epistemic graph.
//!
//! Proves the observable acceptance criteria of SPEC-EPISTEMICRAG and
//! SPEC-EPISTEMICRAG-INSTANT end-to-end against the shipped core API
//! (`rustyred_thg_core::epistemic`). Criteria that live in sibling crates are
//! covered there and noted inline:
//!   - INSTANT #4 (code drift) -> `rustyred-thg-code::code_epistemic_hook` tests.
//!   - EPISTEMICRAG #2 (recall include_epistemic) -> `rustyred-thg-memory` tests.
//!
//! These are integration tests over public APIs only; they touch no private
//! state and add zero source surface, so they are safe to co-author beside the
//! crate they verify.

use std::collections::HashMap;

use rustyred_thg_core::epistemic::{
    epistemic_shadow_ppr, read_epistemic_shadow, run_epistemic_cron_pass,
    structural_epistemic_pass, EpistemicAnnotation, EpistemicAnnotations, EpistemicCandidatePair,
    EpistemicCronInput, EpistemicEnricher, EpistemicEnrichmentError, EpistemicEnrichmentMode,
    EpistemicSourceKind, GroundedExtensionStatus, PredictedEdgePointer, SourceReliability,
    StructuralEpistemicConfig, StructuralEpistemicInput, EPISTEMIC_SHADOW_LABEL,
};
use rustyred_thg_core::{EdgeRecord, GraphStore, InMemoryGraphStore, NodeQuery, NodeRecord};
use serde_json::json;

// --------------------------------------------------------------------------- //
// Test fixtures
// --------------------------------------------------------------------------- //

fn claim(id: &str, text: &str) -> NodeRecord {
    NodeRecord::new(id, ["Claim"], json!({ "content": text }))
}

fn shadow_count(store: &InMemoryGraphStore) -> usize {
    store
        .query_nodes(NodeQuery::label(EPISTEMIC_SHADOW_LABEL).with_limit(100_000))
        .len()
}

/// An enricher that returns canned annotations (the success path / a stand-in
/// for Theseus over gRPC).
struct StaticEnricher(EpistemicAnnotations);
impl EpistemicEnricher for StaticEnricher {
    fn enrich(
        &self,
        _subgraph: rustyred_thg_core::epistemic::UserSubgraph,
        _mode: EpistemicEnrichmentMode,
    ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError> {
        Ok(self.0.clone())
    }
}

/// An enricher that always fails, simulating a dropped gRPC connection.
struct DroppingEnricher;
impl EpistemicEnricher for DroppingEnricher {
    fn enrich(
        &self,
        _subgraph: rustyred_thg_core::epistemic::UserSubgraph,
        _mode: EpistemicEnrichmentMode,
    ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError> {
        Err(EpistemicEnrichmentError::new(
            "grpc_drop",
            "deadline exceeded",
        ))
    }
}

fn run_structural(store: &mut InMemoryGraphStore, batch: &[&str], pairs: Vec<(&str, &str)>) {
    let candidate_pairs = pairs
        .into_iter()
        .map(|(l, r)| EpistemicCandidatePair {
            left_content_id: l.to_string(),
            right_content_id: r.to_string(),
        })
        .collect();
    structural_epistemic_pass(
        store,
        StructuralEpistemicInput {
            batch_node_ids: batch.iter().map(|s| s.to_string()).collect(),
            candidate_pairs,
            explicit_relations: Vec::new(),
            config: StructuralEpistemicConfig::default(),
        },
    )
    .expect("structural pass");
}

// --------------------------------------------------------------------------- //
// SPEC-EPISTEMICRAG-INSTANT
// --------------------------------------------------------------------------- //

#[test]
fn instant_tier1_populates_with_no_model() {
    // INSTANT #2: Tier 1 fields populate on any ingested subgraph, no model.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("a", "claim a")).unwrap();
    store.upsert_node(claim("b", "claim b")).unwrap();
    store.upsert_node(claim("orphan", "lonely")).unwrap();
    store
        .upsert_edge(EdgeRecord::new("e1", "a", "CITES", "b", json!({})))
        .unwrap();

    run_structural(&mut store, &["a", "b", "orphan"], vec![]);

    let sb = read_epistemic_shadow(&store, "b").expect("shadow b");
    assert_eq!(sb.support_in_degree, 1, "b is supported by a (CITES)");
    assert!(!sb.unsupported_leaf);
    let sa = read_epistemic_shadow(&store, "a").expect("shadow a");
    assert!(sa.unsupported_leaf, "a has no supporting in-edge");
    let so = read_epistemic_shadow(&store, "orphan").expect("shadow orphan");
    assert!(so.orphan, "disconnected node is an orphan");
}

#[test]
fn instant_planted_contradiction_undercuts_and_grounded_marks_loser_out() {
    // INSTANT #3 / EPISTEMICRAG (grounded extension): a contradiction becomes an
    // UNDERCUTS edge and the grounded extension marks the loser OUT.
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(claim("A", "the cache is safe under concurrency"))
        .unwrap();
    store
        .upsert_node(claim("B", "the cache is not safe under concurrency"))
        .unwrap();

    run_structural(&mut store, &["A", "B"], vec![("A", "B")]);

    let sa = read_epistemic_shadow(&store, "A").expect("shadow A");
    let sb = read_epistemic_shadow(&store, "B").expect("shadow B");
    let statuses = [sa.grounded_extension_status, sb.grounded_extension_status];
    assert!(
        statuses.contains(&GroundedExtensionStatus::Out),
        "the loser is marked out (got {statuses:?})"
    );
    assert!(
        statuses.contains(&GroundedExtensionStatus::In),
        "the winner stays in"
    );
}

#[test]
fn instant_checked_pairs_are_hnsw_bounded_not_quadratic() {
    // INSTANT #5: the checked-pair count scales with k*claims, not claims^2.
    let mut store = InMemoryGraphStore::new();
    let n = 20usize;
    let ids: Vec<String> = (0..n).map(|i| format!("c{i}")).collect();
    for id in &ids {
        store.upsert_node(claim(id, "neutral statement")).unwrap();
    }
    let k = 4usize;
    // A k-bounded candidate set: k neighbors per claim.
    let mut pairs = Vec::new();
    for i in 0..n {
        for j in 1..=k {
            pairs.push(EpistemicCandidatePair {
                left_content_id: format!("c{i}"),
                right_content_id: format!("c{}", (i + j) % n),
            });
        }
    }
    let readout = structural_epistemic_pass(
        &mut store,
        StructuralEpistemicInput {
            batch_node_ids: ids.clone(),
            candidate_pairs: pairs,
            explicit_relations: Vec::new(),
            config: StructuralEpistemicConfig {
                candidate_top_k: k,
                ..StructuralEpistemicConfig::default()
            },
        },
    )
    .unwrap();
    assert!(
        readout.checked_pair_count <= k * n,
        "checked {} <= k*n {}",
        readout.checked_pair_count,
        k * n
    );
    assert!(
        readout.checked_pair_count < n * (n - 1) / 2,
        "bounded well below all-pairs {}",
        n * (n - 1) / 2
    );
}

#[test]
fn instant_shadow_carries_source_kind_structural() {
    // INSTANT #6 / EPISTEMICRAG #7: structural fields are tagged source_kind.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("x", "a claim")).unwrap();
    run_structural(&mut store, &["x"], vec![]);
    let sx = read_epistemic_shadow(&store, "x").expect("shadow x");
    assert_eq!(sx.source_kind, EpistemicSourceKind::Structural);
    assert!(
        sx.predicted_edges.is_empty(),
        "no learned predicted edges yet"
    );
}

#[test]
fn instant_structural_pass_is_idempotent() {
    // INSTANT (idempotency) / EPISTEMICRAG #6: a re-run creates no duplicates.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("a", "alpha")).unwrap();
    store.upsert_node(claim("b", "beta")).unwrap();
    store
        .upsert_edge(EdgeRecord::new("e", "a", "CITES", "b", json!({})))
        .unwrap();
    run_structural(&mut store, &["a", "b"], vec![]);
    let first = shadow_count(&store);
    run_structural(&mut store, &["a", "b"], vec![]);
    let second = shadow_count(&store);
    assert_eq!(first, second, "no duplicate shadow nodes on re-run");
}

// --------------------------------------------------------------------------- //
// SPEC-EPISTEMICRAG (learned layer + cron)
// --------------------------------------------------------------------------- //

fn learned_annotation(content_id: &str, predicted_target: &str) -> EpistemicAnnotation {
    EpistemicAnnotation {
        content_node_id: content_id.to_string(),
        predicted_edges: vec![PredictedEdgePointer {
            target_content_id: predicted_target.to_string(),
            relation: "RELATES".to_string(),
            confidence: 0.9,
            quarantine: true,
        }],
        completion_confidence: Some(0.9),
        structural_role_vector: vec![0.1, 0.2, 0.3],
        source_reliability: Some(SourceReliability {
            alpha: 4.0,
            beta: 1.0,
            mean: 0.8,
        }),
        community_id: Some("c1".to_string()),
        grounded_extension_status: None,
    }
}

#[test]
fn epistemicrag_cron_adds_learned_fields_preserving_structural() {
    // EPISTEMICRAG #1 + #7 + INSTANT #7: after the structural pass then a cron
    // pass, the shadow has non-null structural fields AND learned fields, with
    // both source kinds distinguishable; structural is untouched.
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(claim("doc", "a documented claim"))
        .unwrap();
    store.upsert_node(claim("dep", "a dependency")).unwrap();
    store
        .upsert_edge(EdgeRecord::new("e", "dep", "SUPPORTS", "doc", json!({})))
        .unwrap();
    run_structural(&mut store, &["doc", "dep"], vec![]);

    let before = read_epistemic_shadow(&store, "doc").unwrap();
    assert_eq!(before.source_kind, EpistemicSourceKind::Structural);
    let structural_support = before.support_in_degree;

    let enricher = StaticEnricher(EpistemicAnnotations {
        annotations: vec![learned_annotation("doc", "ghost")],
        support_relations: Vec::new(),
        attack_relations: Vec::new(),
    });
    let report = run_epistemic_cron_pass(
        &mut store,
        EpistemicCronInput {
            content_node_ids: vec!["doc".into(), "dep".into()],
            ..EpistemicCronInput::default()
        },
        &enricher,
    )
    .unwrap();
    assert!(report.grpc_ok);
    assert_eq!(report.annotations_received, 1);

    let after = read_epistemic_shadow(&store, "doc").unwrap();
    // Structural preserved.
    assert_eq!(
        after.support_in_degree, structural_support,
        "structural in-degree preserved"
    );
    assert!(after.grounded_extension_status == before.grounded_extension_status);
    // Learned added + distinguishable.
    assert_eq!(after.source_kind, EpistemicSourceKind::Mixed, "now mixed");
    assert_eq!(after.completion_confidence, Some(0.9));
    assert!(after.source_reliability.is_some());
    assert!(
        after
            .field_provenance
            .values()
            .any(|p| p.source_kind == EpistemicSourceKind::Learned),
        "learned field provenance present"
    );
    assert!(
        after
            .field_provenance
            .values()
            .any(|p| p.source_kind == EpistemicSourceKind::Structural),
        "structural field provenance preserved"
    );
}

#[test]
fn epistemicrag_predicted_edges_are_quarantined_pointers_not_injected_nodes() {
    // EPISTEMICRAG #5: predicted-edge targets appear as pointers, never injected
    // as full content nodes.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("doc", "a claim")).unwrap();
    run_structural(&mut store, &["doc"], vec![]);

    let enricher = StaticEnricher(EpistemicAnnotations {
        annotations: vec![learned_annotation("doc", "ghost_target")],
        support_relations: Vec::new(),
        attack_relations: Vec::new(),
    });
    run_epistemic_cron_pass(
        &mut store,
        EpistemicCronInput {
            content_node_ids: vec!["doc".into()],
            ..EpistemicCronInput::default()
        },
        &enricher,
    )
    .unwrap();

    let shadow = read_epistemic_shadow(&store, "doc").unwrap();
    assert_eq!(shadow.predicted_edges.len(), 1);
    assert!(
        shadow.predicted_edges[0].quarantine,
        "predicted edge quarantined"
    );
    assert_eq!(shadow.predicted_edges[0].target_content_id, "ghost_target");
    // The predicted target was never created as a node.
    assert!(
        GraphStore::get_node(&store, "ghost_target").is_none(),
        "predicted target must not be injected as a content node"
    );
}

#[test]
fn epistemicrag_grpc_drop_completes_cleanly_without_partial_writes() {
    // EPISTEMICRAG #4: a simulated gRPC drop leaves the cron completing cleanly,
    // no partial learned-field writes, and no deleted shadow nodes.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("doc", "a claim")).unwrap();
    run_structural(&mut store, &["doc"], vec![]);
    let before = shadow_count(&store);
    let before_shadow = read_epistemic_shadow(&store, "doc").unwrap();
    assert_eq!(before_shadow.source_kind, EpistemicSourceKind::Structural);

    let report = run_epistemic_cron_pass(
        &mut store,
        EpistemicCronInput {
            content_node_ids: vec!["doc".into()],
            ..EpistemicCronInput::default()
        },
        &DroppingEnricher,
    )
    .unwrap();
    assert!(report.attempted);
    assert!(report.no_op, "drop is a clean no-op");
    assert!(!report.grpc_ok);

    // Shadow node still present, still purely structural (no partial learned write).
    assert_eq!(shadow_count(&store), before, "no shadow nodes deleted");
    let after = read_epistemic_shadow(&store, "doc").unwrap();
    assert_eq!(
        after.source_kind,
        EpistemicSourceKind::Structural,
        "no learned fields written"
    );
    assert!(after.completion_confidence.is_none());
}

#[test]
fn epistemicrag_cron_is_idempotent_by_engine_version() {
    // EPISTEMICRAG #6: re-running the cron with the same engine_version creates
    // no duplicate shadow nodes or edges.
    let mut store = InMemoryGraphStore::new();
    store.upsert_node(claim("doc", "a claim")).unwrap();
    run_structural(&mut store, &["doc"], vec![]);
    let enricher = StaticEnricher(EpistemicAnnotations {
        annotations: vec![learned_annotation("doc", "ghost")],
        support_relations: Vec::new(),
        attack_relations: Vec::new(),
    });
    let input = || EpistemicCronInput {
        content_node_ids: vec!["doc".into()],
        ..EpistemicCronInput::default()
    };
    run_epistemic_cron_pass(&mut store, input(), &enricher).unwrap();
    let first = shadow_count(&store);
    run_epistemic_cron_pass(&mut store, input(), &enricher).unwrap();
    let second = shadow_count(&store);
    assert_eq!(first, second, "no duplicate shadow nodes on cron re-run");
}

#[test]
fn epistemicrag_density_floor_gates_the_learned_pass() {
    // Gate before first run: completion runs only above an edge-density floor.
    let mut store = InMemoryGraphStore::new();
    // Three nodes, no edges -> density 0, below any positive floor.
    for id in ["a", "b", "c"] {
        store.upsert_node(claim(id, "x")).unwrap();
    }
    let enricher = StaticEnricher(EpistemicAnnotations::default());
    let report = run_epistemic_cron_pass(
        &mut store,
        EpistemicCronInput {
            content_node_ids: vec!["a".into(), "b".into(), "c".into()],
            density_floor: 5.0,
            ..EpistemicCronInput::default()
        },
        &enricher,
    )
    .unwrap();
    assert!(
        report.no_op,
        "sparse subgraph gated out of the learned pass"
    );
    assert!(
        report.skipped_reason.contains("edge_density_below_floor"),
        "skipped for density, got {}",
        report.skipped_reason
    );
}

#[test]
fn epistemicrag_ppr_over_shadow_layer_returns_ranking() {
    // EPISTEMICRAG #3 / recall deliverable 6: PPR runs over shadow-to-shadow
    // edges and returns a ranking.
    let mut store = InMemoryGraphStore::new();
    // A chain of equivalent claims: infer_pair_relation emits SUPPORTS for
    // normalized-equal text, so the shadow layer gains SUPPORTS edges to walk.
    store.upsert_node(claim("a", "the cache is safe")).unwrap();
    store.upsert_node(claim("b", "the cache is safe")).unwrap();
    store.upsert_node(claim("c", "the cache is safe")).unwrap();
    run_structural(&mut store, &["a", "b", "c"], vec![("a", "b"), ("b", "c")]);

    let mut seeds = HashMap::new();
    seeds.insert("a".to_string(), 1.0);
    let ranking = epistemic_shadow_ppr(&store, &seeds, 10, 0.85, 1e-6, 1000);
    assert!(!ranking.is_empty(), "shadow-layer PPR returns a ranking");
}
