use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::{
    quarantine_densification_candidates, rank_global_completion_candidates,
    rank_global_completion_candidates_default, FixedPointAggregator, GlobalCompletionConfig,
    GlobalCompletionRequest, MessageAggregator, ScatterAggregationPath,
};

fn completion_request(seeds: Vec<&str>) -> GlobalCompletionRequest {
    GlobalCompletionRequest {
        tenant_id: "theorem".to_string(),
        seed_node_ids: seeds.into_iter().map(str::to_string).collect(),
        min_path_confidence: 0.0,
        confidence_threshold: 0.05,
        confidence_ceiling: 0.72,
        max_candidates: 16,
        admission_tier: "advisory_inferred".to_string(),
        model_id: "edge-mpnn-completion/test".to_string(),
        allowed_edge_types: vec![],
    }
}

fn line_graph_store() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["node:a", "node:b", "node:c", "node:d"] {
        store
            .upsert_node(NodeRecord::new(node_id, ["Object"], json!({})))
            .unwrap();
    }
    for (edge_id, from, to) in [
        ("edge:a-b", "node:a", "node:b"),
        ("edge:b-c", "node:b", "node:c"),
        ("edge:c-d", "node:c", "node:d"),
    ] {
        store
            .upsert_edge(
                EdgeRecord::new(edge_id, from, "RELATES_TO", to, json!({})).with_confidence(0.9),
            )
            .unwrap();
    }
    store
}

#[test]
fn global_completion_scores_multi_hop_targets_with_support_chains() {
    let store = line_graph_store();
    let result = rank_global_completion_candidates_default(
        &store.snapshot(),
        completion_request(vec!["node:a"]),
        GlobalCompletionConfig::default(),
    )
    .unwrap();

    assert_eq!(result.seeds_run, 1);
    // Direct neighbor b is suppressed; c (2-hop) and d (3-hop) are reachable.
    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.target_id != "node:b"));
    let two_hop = result
        .candidates
        .iter()
        .find(|candidate| candidate.target_id == "node:c")
        .expect("two-hop candidate");
    assert_eq!(two_hop.support_path_edge_ids, vec!["edge:a-b", "edge:b-c"]);
    assert_eq!(
        two_hop.support_path_node_ids,
        vec!["node:a", "node:b", "node:c"]
    );
    assert_eq!(two_hop.proposed_edge_type, "INFERRED_RELATES_TO");
    assert!(two_hop.confidence <= 0.72);

    let three_hop = result
        .candidates
        .iter()
        .find(|candidate| candidate.target_id == "node:d")
        .expect("three-hop candidate");
    assert_eq!(three_hop.support_path_edge_ids.len(), 3);
    assert!(two_hop.confidence >= three_hop.confidence);
}

#[test]
fn global_completion_is_deterministic_across_runs() {
    let store = line_graph_store();
    let first = rank_global_completion_candidates_default(
        &store.snapshot(),
        completion_request(vec!["node:a"]),
        GlobalCompletionConfig::default(),
    )
    .unwrap();
    let second = rank_global_completion_candidates_default(
        &store.snapshot(),
        completion_request(vec!["node:a"]),
        GlobalCompletionConfig::default(),
    )
    .unwrap();
    assert_eq!(first, second);
}

#[test]
fn frontier_cap_bounds_the_active_set_and_reports_it() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new("node:hub", ["Object"], json!({})))
        .unwrap();
    for idx in 0..12 {
        let spoke = format!("node:spoke-{idx}");
        let leaf = format!("node:leaf-{idx}");
        store
            .upsert_node(NodeRecord::new(&spoke, ["Object"], json!({})))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(&leaf, ["Object"], json!({})))
            .unwrap();
        store
            .upsert_edge(
                EdgeRecord::new(
                    format!("edge:hub-{idx}"),
                    "node:hub",
                    "RELATES_TO",
                    &spoke,
                    json!({}),
                )
                .with_confidence(0.9),
            )
            .unwrap();
        store
            .upsert_edge(
                EdgeRecord::new(
                    format!("edge:spoke-leaf-{idx}"),
                    &spoke,
                    "RELATES_TO",
                    &leaf,
                    json!({}),
                )
                .with_confidence(0.9),
            )
            .unwrap();
    }

    let result = rank_global_completion_candidates_default(
        &store.snapshot(),
        completion_request(vec!["node:hub"]),
        GlobalCompletionConfig {
            max_frontier_nodes: 4,
            ..GlobalCompletionConfig::default()
        },
    )
    .unwrap();

    assert!(result.frontier_capped);
    assert!(!result.candidates.is_empty());
    assert!(result.candidates.len() <= 16);
}

#[test]
fn aggregation_policy_and_aggregator_are_reported() {
    struct CountingAggregator {
        inner: FixedPointAggregator,
    }
    impl MessageAggregator for CountingAggregator {
        fn aggregate(
            &self,
            messages: &[Vec<f32>],
            edge_dst: &[usize],
            num_nodes: usize,
            mean_aggregate: bool,
        ) -> rustyred_thg_core::ThgResult<Vec<Vec<f32>>> {
            self.inner
                .aggregate(messages, edge_dst, num_nodes, mean_aggregate)
        }
        fn aggregator_id(&self) -> &'static str {
            "counting_test_aggregator"
        }
    }

    let store = line_graph_store();
    let result = rank_global_completion_candidates(
        &store.snapshot(),
        completion_request(vec!["node:a"]),
        GlobalCompletionConfig::default(),
        &CountingAggregator {
            inner: FixedPointAggregator::default(),
        },
    )
    .unwrap();

    assert_eq!(result.aggregator_id, "counting_test_aggregator");
    // Deterministic-required keeps the policy off the float-atomic fast path.
    assert_ne!(
        result.aggregation_path,
        ScatterAggregationPath::NativeFloatAtomicFastPath
    );
}

#[test]
fn completion_candidates_flow_into_the_quarantine_pipeline() {
    let mut store = line_graph_store();
    let result = rank_global_completion_candidates_default(
        &store.snapshot(),
        completion_request(vec!["node:a"]),
        GlobalCompletionConfig::default(),
    )
    .unwrap();
    assert!(!result.candidates.is_empty());

    let quarantine = quarantine_densification_candidates(
        &mut store,
        "theorem",
        "completion-run-1",
        &result.candidates,
        Some("test"),
    )
    .unwrap();
    assert_eq!(quarantine.candidate_node_ids.len(), result.candidates.len());
    // No live topology edge was inserted for any candidate.
    for candidate in &result.candidates {
        assert!(store
            .get_edge(&format!(
                "edge:{}:{}:{}",
                candidate.source_id, candidate.proposed_edge_type, candidate.target_id
            ))
            .is_none());
    }
}
