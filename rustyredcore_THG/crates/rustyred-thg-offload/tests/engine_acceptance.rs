//! Acceptance tests for the compute-offload engine.
//!
//! Proves: the classifier marks the four symbolic kinds eligible and
//! `NeuralSynthesis` not; the wired graph-centrality affordance computes a REAL
//! PageRank over real edges (exact, not faked); the ledger accumulates
//! `gpu_seconds_saved`; and an eligible-but-unrouted kind records honestly.

use rustyred_thg_core::EdgeRecord;
use rustyred_thg_offload::{
    GraphCentralityAffordance, OffloadAffordance, OffloadClassifier, OffloadEngine, OffloadOperation,
    OffloadOperationKind, OffloadOutcome, GRAPH_PAGERANK_AFFORDANCE,
};
use serde_json::json;

/// A directed star: three leaves all point at one hub. PageRank must rank the
/// hub strictly highest - this is a ground-truth structural fact the test
/// asserts, so the affordance is proven to compute a real result.
fn star_edges() -> Vec<EdgeRecord> {
    vec![
        EdgeRecord::new("e1", "leaf_a", "SIMILAR_TO", "hub", json!({})),
        EdgeRecord::new("e2", "leaf_b", "SIMILAR_TO", "hub", json!({})),
        EdgeRecord::new("e3", "leaf_c", "SIMILAR_TO", "hub", json!({})),
    ]
}

#[test]
fn classifier_marks_four_symbolic_kinds_eligible_and_neural_not() {
    let classifier = OffloadClassifier::new();

    for kind in [
        OffloadOperationKind::LogicalDerivation,
        OffloadOperationKind::ProbabilisticInference,
        OffloadOperationKind::ConstraintSolving,
        OffloadOperationKind::GraphAlgorithm,
    ] {
        let op = OffloadOperation::new("symbolic", kind).with_units(10);
        let decision = classifier.classify(&op);
        assert!(decision.eligible, "{kind:?} must be offload-eligible");
        assert!(
            decision.estimated_gpu_seconds_saved > 0.0,
            "{kind:?} eligible op must estimate a positive GPU-second saving"
        );
        assert!(kind.is_offload_eligible());
    }

    let neural = OffloadOperation::new("synthesis", OffloadOperationKind::NeuralSynthesis);
    let decision = classifier.classify(&neural);
    assert!(
        !decision.eligible,
        "NeuralSynthesis genuinely needs the GPU; must NOT be eligible"
    );
    assert_eq!(decision.estimated_gpu_seconds_saved, 0.0);
    assert!(!OffloadOperationKind::NeuralSynthesis.is_offload_eligible());
}

#[test]
fn wired_affordance_computes_real_pagerank() {
    // Call the affordance directly: it must compute an EXACT PageRank, ranking
    // the hub of a directed star strictly highest.
    let affordance = GraphCentralityAffordance::default();
    assert_eq!(affordance.id(), GRAPH_PAGERANK_AFFORDANCE);
    assert_eq!(affordance.kind(), OffloadOperationKind::GraphAlgorithm);

    let op = OffloadOperation::new("similar_item_centrality", OffloadOperationKind::GraphAlgorithm)
        .with_units(4)
        .with_payload(json!({ "edges": star_edges() }));

    let result = affordance.execute(&op).expect("real PageRank must compute");
    assert_eq!(result.affordance, GRAPH_PAGERANK_AFFORDANCE);

    // The hub is the most central node - a ground-truth structural fact.
    assert_eq!(
        result.summary["top_node"], json!("hub"),
        "PageRank must rank the star hub highest"
    );

    // Real scores: the hub's score must strictly exceed every leaf's.
    let scores = result.result["scores"].as_object().expect("scores object");
    let hub = scores["hub"].as_f64().unwrap();
    for leaf in ["leaf_a", "leaf_b", "leaf_c"] {
        let leaf_score = scores[leaf].as_f64().unwrap();
        assert!(
            hub > leaf_score,
            "hub ({hub}) must outrank leaf {leaf} ({leaf_score})"
        );
    }
    // PageRank scores sum to ~1.0 (exact, dangling-mass-redistributed).
    let total: f64 = scores.values().map(|v| v.as_f64().unwrap()).sum();
    assert!((total - 1.0).abs() < 1e-6, "PageRank scores must sum to 1.0, got {total}");
}

#[test]
fn engine_routes_graph_op_executes_and_ledger_accumulates() {
    let mut engine = OffloadEngine::default();
    assert!(engine.has_affordance(OffloadOperationKind::GraphAlgorithm));
    assert_eq!(engine.gpu_seconds_saved(), 0.0);

    let op = OffloadOperation::new("similar_item_centrality", OffloadOperationKind::GraphAlgorithm)
        .with_units(4)
        .with_payload(json!({ "edges": star_edges() }));

    let report = engine.route(&op);
    assert!(report.executed(), "graph op must execute on the wired affordance");
    assert_eq!(report.decision.affordance.as_deref(), Some(GRAPH_PAGERANK_AFFORDANCE));
    let result = report.result().expect("executed op carries a result");
    assert_eq!(result.summary["top_node"], json!("hub"));

    // The ledger banked a positive realized saving and recorded one entry.
    assert!(report.gpu_seconds_saved > 0.0);
    assert_eq!(engine.ledger().len(), 1);
    assert!(engine.gpu_seconds_saved() > 0.0);
    assert!(engine.ledger().realized_gpu_seconds_saved() > 0.0);

    // Route a second op: the cumulative total strictly grows (accumulation).
    let before = engine.gpu_seconds_saved();
    let report2 = engine.route(&op);
    assert!(report2.executed());
    assert!(
        engine.gpu_seconds_saved() > before,
        "cumulative gpu_seconds_saved must accumulate across routed ops"
    );
    assert_eq!(engine.ledger().len(), 2);
}

#[test]
fn eligible_but_unrouted_kind_records_honestly() {
    // A bare engine with NO affordances: an eligible op has nothing to run.
    let mut engine = OffloadEngine::new(OffloadClassifier::new());
    assert!(!engine.has_affordance(OffloadOperationKind::LogicalDerivation));

    let op = OffloadOperation::new("entailment", OffloadOperationKind::LogicalDerivation)
        .with_units(20);
    let report = engine.route(&op);

    assert!(report.decision.eligible, "logical derivation is eligible");
    assert!(
        !report.executed(),
        "with no affordance wired it must NOT execute (no faked result)"
    );
    assert!(
        matches!(
            report.outcome,
            OffloadOutcome::EligibleUnrouted {
                kind: OffloadOperationKind::LogicalDerivation
            }
        ),
        "honest eligible-but-unrouted outcome"
    );
    assert!(report.result().is_none(), "no result is fabricated");

    // The estimate is banked (the saving available once an affordance lands),
    // but it counts as ESTIMATED, not realized.
    assert!(report.gpu_seconds_saved > 0.0);
    assert!(engine.gpu_seconds_saved() > 0.0);
    assert_eq!(
        engine.ledger().realized_gpu_seconds_saved(),
        0.0,
        "nothing was realized: no affordance actually ran"
    );
}

#[test]
fn neural_synthesis_routes_to_not_eligible_no_saving() {
    let mut engine = OffloadEngine::default();
    let op = OffloadOperation::new("answer_synthesis", OffloadOperationKind::NeuralSynthesis)
        .with_units(50);
    let report = engine.route(&op);

    assert!(!report.decision.eligible);
    assert!(matches!(report.outcome, OffloadOutcome::NotEligible));
    assert_eq!(report.gpu_seconds_saved, 0.0);
    assert_eq!(engine.gpu_seconds_saved(), 0.0, "neural synthesis stays on GPU; no saving");
    assert_eq!(engine.ledger().len(), 1, "the not-eligible decision is still recorded");
}

#[test]
fn ledger_is_replayable() {
    // Routing produces a ledger whose entries re-sum to the running total - the
    // replayability property the thesis's cost accounting needs.
    let mut engine = OffloadEngine::default();
    let graph_op =
        OffloadOperation::new("centrality", OffloadOperationKind::GraphAlgorithm)
            .with_units(4)
            .with_payload(json!({ "edges": star_edges() }));
    let logical_op =
        OffloadOperation::new("entailment", OffloadOperationKind::LogicalDerivation).with_units(8);
    let neural_op =
        OffloadOperation::new("synth", OffloadOperationKind::NeuralSynthesis).with_units(3);

    engine.route(&graph_op);
    engine.route(&logical_op);
    engine.route(&neural_op);

    let replayed: f64 = engine
        .ledger()
        .entries()
        .iter()
        .map(|entry| entry.gpu_seconds_saved)
        .sum();
    assert!(
        (replayed - engine.gpu_seconds_saved()).abs() < 1e-12,
        "re-summing ledger entries must reproduce the cumulative total"
    );
    assert_eq!(engine.ledger().len(), 3);
}
