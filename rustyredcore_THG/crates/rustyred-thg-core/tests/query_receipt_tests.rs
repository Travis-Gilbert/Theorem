use rustyred_thg_core::{
    AccessPathTrace, IndexAdvisor, IndexAdvisorConfig, IndexBackend, IndexCreatedBy, IndexKind,
    IndexManifest, IndexPainKind, IndexProposalStatus, IndexScope, PlanTrace, QueryKind,
    QueryReceipt, ReceiptScope, ShadowValidationReport,
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

#[test]
fn advisor_clusters_receipts_and_detects_candidate_token_recall_and_cold_pain() {
    let trace = trace_with_method("full_scan", 0);
    let mut first = QueryReceipt::from_plan_trace(
        "qr:pain:1",
        QueryKind::ContextCompile,
        scope(),
        &trace,
        2,
        123,
    );
    first
        .latency_by_stage_ms
        .insert("vector".to_string(), 750.0);
    first
        .candidate_counts_by_stage
        .insert("candidate_set".to_string(), 50);
    first
        .candidate_counts_by_stage
        .insert("cold_reads".to_string(), 2);
    first
        .candidate_counts_by_stage
        .insert("recall_missing".to_string(), 1);
    first.token_cost = Some(1_500);

    let mut second = first.clone();
    second.id = "qr:pain:2".to_string();

    let advisor = IndexAdvisor::new(IndexAdvisorConfig {
        min_repeated_receipts: 1,
        min_total_full_scans: 1,
        min_total_latency_ms: 500.0,
        min_candidate_waste_ratio: 4.0,
        min_token_cost: 1_000,
        min_recall_missing: 1,
        min_cold_reads: 1,
    });
    let clusters = advisor.cluster_receipts([&first, &second]);
    assert_eq!(clusters.len(), 1);
    assert_eq!(clusters[0].receipt_ids, vec!["qr:pain:1", "qr:pain:2"]);

    let signals = advisor.detect_pain([&first, &second]);
    let kinds = signals
        .iter()
        .map(|signal| signal.pain_kind.clone())
        .collect::<Vec<_>>();
    assert!(kinds.contains(&IndexPainKind::Latency));
    assert!(kinds.contains(&IndexPainKind::CandidateWaste));
    assert!(kinds.contains(&IndexPainKind::TokenCost));
    assert!(kinds.contains(&IndexPainKind::PoorRecall));
    assert!(kinds.contains(&IndexPainKind::ColdRead));
}

#[test]
fn advisor_promotes_only_after_shadow_validation_and_rejects_bad_proposals() {
    let advisor = IndexAdvisor::new(IndexAdvisorConfig::default());
    let draft = IndexManifest::new(
        "idx:context-shadow",
        "Context shadow",
        IndexKind::Vector,
        IndexBackend::AdvisorShadow,
        IndexScope::Project,
        "ContextAtom",
        IndexCreatedBy::Advisor,
    );
    let pain = rustyred_thg_core::IndexPainSignal {
        pain_kind: IndexPainKind::Latency,
        query_signature: "sig:context".to_string(),
        supporting_receipts: vec!["qr:1".to_string()],
        total_full_scans: 0,
        total_results: 2,
        total_candidate_count: 50,
        total_latency_ms: 900.0,
        total_token_cost: 0,
        total_cold_reads: 0,
        recall_missing_count: 0,
        reason: "slow context compile".to_string(),
    };
    let mut bad = advisor.proposal_for_pain("proposal:bad", draft.clone(), &pain);
    let rejected = advisor.apply_shadow_validation(
        &mut bad,
        &ShadowValidationReport {
            replayed_receipts: vec!["qr:1".to_string()],
            latency_saved_ms: 0.0,
            scan_reduction: 0.0,
            recall_drop: 1.0,
            write_amplification: 3.0,
            scope_policy_ttl_tombstone_filters_enforced: false,
            explain_manifest_id: None,
        },
    );
    assert!(!rejected);
    assert_eq!(bad.status, IndexProposalStatus::Rejected);

    let mut good = advisor.proposal_for_pain("proposal:good", draft, &pain);
    let promoted = advisor.apply_shadow_validation(
        &mut good,
        &ShadowValidationReport {
            replayed_receipts: vec!["qr:1".to_string()],
            latency_saved_ms: 10.0,
            scan_reduction: 2.0,
            recall_drop: 0.0,
            write_amplification: 1.0,
            scope_policy_ttl_tombstone_filters_enforced: true,
            explain_manifest_id: Some("idx:context-shadow".to_string()),
        },
    );
    assert!(promoted);
    assert_eq!(good.status, IndexProposalStatus::Promoted);
}

#[test]
fn advisor_retires_unused_or_harmful_indexes() {
    let advisor = IndexAdvisor::new(IndexAdvisorConfig::default());
    let mut manifest = IndexManifest::new(
        "idx:harmful",
        "Harmful index",
        IndexKind::Composite,
        IndexBackend::RustyredCore,
        IndexScope::Project,
        "ContextArtifact",
        IndexCreatedBy::Advisor,
    );
    manifest.record_miss(20.0);
    manifest.record_miss(22.0);

    assert!(advisor.retire_unused_or_harmful(&mut manifest, 2));
    assert_eq!(
        manifest.retirement_reason.as_deref(),
        Some("advisor_retired_unused_or_harmful_index")
    );
}
