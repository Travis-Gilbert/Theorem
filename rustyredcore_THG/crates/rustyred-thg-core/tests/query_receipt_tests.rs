use rustyred_thg_core::{
    AccessPathTrace, IndexAdvisor, IndexAdvisorConfig, IndexBackend, IndexCreatedBy, IndexKind,
    IndexManifest, IndexScope, PlanTrace, QueryKind, QueryReceipt, ReceiptScope,
};

fn scope() -> ReceiptScope {
    ReceiptScope::from([
        ("tenant".to_string(), "Travis-Gilbert".to_string()),
        ("project".to_string(), "Theorem".to_string()),
    ])
}

fn trace_with_method(method: &str, full_relation_scans: usize) -> PlanTrace {
    let mut trace = PlanTrace {
        fusion: "rrf".to_string(),
        candidate_set_size: 3,
        full_relation_scans,
        ..PlanTrace::default()
    };
    trace.access_paths.push(AccessPathTrace {
        relation: "artifacts".to_string(),
        alias: "artifact".to_string(),
        predicate: "scope + artifact_type + updated_at".to_string(),
        method: method.to_string(),
        est_rows: 3.0,
        est_work: 1.0,
        returned_rows: 3,
        visited_rows: 3,
    });
    trace
}

#[test]
fn planner_trace_becomes_query_receipt_and_explain() {
    let trace = trace_with_method("ordered", 0);
    let receipt = QueryReceipt::from_plan_trace(
        "qr:artifact-list:1",
        QueryKind::ArtifactList,
        scope(),
        &trace,
        2,
        123,
    );

    assert_eq!(receipt.full_scan_count, 0);
    assert_eq!(receipt.hydrated_object_count, 2);
    assert_eq!(
        receipt.indexes_used,
        vec!["ordered:artifacts:scope + artifact_type + updated_at"]
    );
    assert_eq!(
        receipt
            .candidate_counts_by_stage
            .get("artifact:scope + artifact_type + updated_at:ordered"),
        Some(&3)
    );

    let explain = receipt.explain();
    assert_eq!(explain.full_scans, 0);
    assert_eq!(explain.access_paths_selected.len(), 1);
    assert_eq!(explain.candidate_counts.get("candidate_set"), Some(&3));
}

#[test]
fn advisor_detects_repeated_full_scan_pain_and_builds_proposal() {
    let trace = trace_with_method("full_scan", 1);
    let first = QueryReceipt::from_plan_trace(
        "qr:scan:1",
        QueryKind::ArtifactList,
        scope(),
        &trace,
        3,
        123,
    );
    let second = QueryReceipt::from_plan_trace(
        "qr:scan:2",
        QueryKind::ArtifactList,
        scope(),
        &trace,
        4,
        124,
    );
    assert_eq!(first.query_signature, second.query_signature);

    let advisor = IndexAdvisor::new(IndexAdvisorConfig::default());
    let signals = advisor.detect_full_scan_pain([&first, &second]);
    assert_eq!(signals.len(), 1);
    assert_eq!(
        signals[0].supporting_receipts,
        vec!["qr:scan:1", "qr:scan:2"]
    );
    assert_eq!(signals[0].total_full_scans, 2);

    let draft = IndexManifest::new(
        "idx:artifact-list-shadow",
        "Artifact list shadow",
        IndexKind::Composite,
        IndexBackend::AdvisorShadow,
        IndexScope::Project,
        "ContextArtifact",
        IndexCreatedBy::Advisor,
    )
    .with_target_properties(["scope", "artifact_type", "updated_at"]);
    let proposal = advisor.proposal_for_pain("proposal:artifact-list", draft, &signals[0]);

    assert_eq!(proposal.supporting_receipts.len(), 2);
    assert!(proposal
        .shadow_validation_plan
        .contains("replay supporting receipts"));
}
