use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::{
    aggregate_messages_fixed_point, choose_scatter_aggregation_path,
    quarantine_densification_candidates, rank_densification_candidates,
    rank_pairformer_densification_candidates, representation_sidecar_node_id,
    upsert_representation_sidecar, DensificationRequest, PairformerConfig,
    RepresentationSidecarInput, RepresentationTargetKind, ScatterAggregationPath,
    ScatterAggregationRequest, DEFAULT_FIXED_POINT_SCALE, REFLEXIVE_EDGE_CANDIDATE_LABEL,
};

#[test]
fn scatter_policy_uses_portable_paths_before_float_atomics() {
    let small = choose_scatter_aggregation_path(ScatterAggregationRequest {
        num_edges: 128,
        feature_dim: 64,
        deterministic_required: false,
        browser_webgpu_target: false,
        float_atomic_add_available: true,
        burn_native_max_elements: 20_000,
    });
    assert_eq!(small, ScatterAggregationPath::BurnScatterAdd);

    let portable = choose_scatter_aggregation_path(ScatterAggregationRequest {
        num_edges: 10_000,
        feature_dim: 64,
        deterministic_required: true,
        browser_webgpu_target: false,
        float_atomic_add_available: true,
        burn_native_max_elements: 20_000,
    });
    assert_eq!(portable, ScatterAggregationPath::FixedPointAtomicCompatible);

    let browser = choose_scatter_aggregation_path(ScatterAggregationRequest {
        num_edges: 10_000,
        feature_dim: 64,
        deterministic_required: false,
        browser_webgpu_target: true,
        float_atomic_add_available: true,
        burn_native_max_elements: 20_000,
    });
    assert_eq!(browser, ScatterAggregationPath::FixedPointAtomicCompatible);

    let native = choose_scatter_aggregation_path(ScatterAggregationRequest {
        num_edges: 10_000,
        feature_dim: 64,
        deterministic_required: false,
        browser_webgpu_target: false,
        float_atomic_add_available: true,
        burn_native_max_elements: 20_000,
    });
    assert_eq!(native, ScatterAggregationPath::NativeFloatAtomicFastPath);
}

#[test]
fn fixed_point_aggregation_counts_degrees_once_per_edge() {
    let messages = vec![vec![1.25, 2.0], vec![2.75, -1.0], vec![10.0, 4.0]];
    let edge_dst = vec![1, 1, 0];

    let sum =
        aggregate_messages_fixed_point(&messages, &edge_dst, 3, DEFAULT_FIXED_POINT_SCALE, false)
            .unwrap();
    assert_eq!(sum[0], vec![10.0, 4.0]);
    assert_eq!(sum[1], vec![4.0, 1.0]);
    assert_eq!(sum[2], vec![0.0, 0.0]);

    let mean =
        aggregate_messages_fixed_point(&messages, &edge_dst, 3, DEFAULT_FIXED_POINT_SCALE, true)
            .unwrap();
    assert_eq!(mean[0], vec![10.0, 4.0]);
    assert_eq!(mean[1], vec![2.0, 0.5]);
    assert_eq!(mean[2], vec![0.0, 0.0]);
}

#[test]
fn representation_sidecar_keeps_embeddings_out_of_topology_nodes() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new(
            "node:a",
            ["Object"],
            json!({ "tenant_id": "theorem", "title": "A" }),
        ))
        .unwrap();

    let missing = upsert_representation_sidecar(
        &mut store,
        RepresentationSidecarInput {
            tenant_id: "theorem".to_string(),
            representation_id: "repr-missing".to_string(),
            target_kind: RepresentationTargetKind::Node,
            target_id: "node:missing".to_string(),
            model_id: "graphlora-sidecar/test".to_string(),
            embedding: vec![0.1, 0.2],
            adapter_ids: vec![],
            graph_version: 1,
            metadata: json!({}),
            manifest_version: 1,
        },
        Some("test"),
    )
    .unwrap_err();
    assert_eq!(missing.code, "missing_graph_endpoint");

    let writeback = upsert_representation_sidecar(
        &mut store,
        RepresentationSidecarInput {
            tenant_id: "theorem".to_string(),
            representation_id: "repr-a-v1".to_string(),
            target_kind: RepresentationTargetKind::Node,
            target_id: "node:a".to_string(),
            model_id: "graphlora-sidecar/test".to_string(),
            embedding: vec![0.1, 0.2],
            adapter_ids: vec!["adapter:a".to_string()],
            graph_version: 1,
            metadata: json!({ "pairformer_block": "heuristic" }),
            manifest_version: 1,
        },
        Some("test"),
    )
    .unwrap();

    assert_eq!(
        writeback.representation_node_id,
        representation_sidecar_node_id("theorem", "repr-a-v1")
    );
    let topology_node = store.get_node("node:a").unwrap();
    assert!(topology_node.properties.get("embedding").is_none());
    let representation = store.get_node(&writeback.representation_node_id).unwrap();
    assert_eq!(representation.labels, vec!["RepresentationSidecar"]);
}

#[test]
fn densification_quarantines_candidates_without_inserting_edges() {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["node:a", "node:b", "node:c"] {
        store
            .upsert_node(NodeRecord::new(node_id, ["Object"], json!({})))
            .unwrap();
    }
    store
        .upsert_edge(EdgeRecord::new(
            "edge:a-b",
            "node:a",
            "RELATES_TO",
            "node:b",
            json!({}),
        ))
        .unwrap();
    store
        .upsert_edge(EdgeRecord::new(
            "edge:b-c",
            "node:b",
            "RELATES_TO",
            "node:c",
            json!({}),
        ))
        .unwrap();

    let snapshot = store.snapshot();
    let result = rank_densification_candidates(
        &snapshot,
        DensificationRequest {
            tenant_id: "theorem".to_string(),
            seed_node_ids: vec!["node:a".to_string()],
            max_nodes: 16,
            max_depth: 2,
            min_path_confidence: 0.0,
            confidence_threshold: 0.5,
            confidence_ceiling: 0.72,
            max_candidates: 8,
            admission_tier: "advisory_inferred".to_string(),
            model_id: "pairformer-link-predict/test".to_string(),
            allowed_edge_types: vec!["RELATES_TO".to_string()],
        },
    )
    .unwrap();

    assert_eq!(result.candidates.len(), 1);
    let candidate = &result.candidates[0];
    assert_eq!(candidate.source_id, "node:a");
    assert_eq!(candidate.target_id, "node:c");
    assert_eq!(candidate.confidence, 0.72);
    assert_eq!(candidate.admission_tier, "advisory_inferred");

    let quarantine = quarantine_densification_candidates(
        &mut store,
        "theorem",
        "run-1",
        &result.candidates,
        Some("test"),
    )
    .unwrap();
    assert_eq!(quarantine.candidate_node_ids.len(), 1);
    let candidate_node = store.get_node(&quarantine.candidate_node_ids[0]).unwrap();
    assert!(candidate_node
        .labels
        .contains(&REFLEXIVE_EDGE_CANDIDATE_LABEL.to_string()));
    assert_eq!(candidate_node.properties["quarantined"], json!(true));
    assert!(store
        .get_edge("node:a-INFERRED_RELATES_TO-node:c")
        .is_none());

    store
        .upsert_edge(EdgeRecord::new(
            "edge:a-c-direct",
            "node:a",
            "RELATES_TO",
            "node:c",
            json!({}),
        ))
        .unwrap();
    let snapshot = store.snapshot();
    let result = rank_densification_candidates(
        &snapshot,
        DensificationRequest {
            tenant_id: "theorem".to_string(),
            seed_node_ids: vec!["node:a".to_string()],
            max_nodes: 16,
            max_depth: 2,
            min_path_confidence: 0.0,
            confidence_threshold: 0.5,
            confidence_ceiling: 0.72,
            max_candidates: 8,
            admission_tier: "advisory_inferred".to_string(),
            model_id: "pairformer-link-predict/test".to_string(),
            allowed_edge_types: vec!["RELATES_TO".to_string()],
        },
    )
    .unwrap();
    assert!(result.candidates.is_empty());
}

#[test]
fn pairformer_densification_uses_bounded_support_and_direct_edge_suppression() {
    let mut store = InMemoryGraphStore::new();
    for (node_id, embedding) in [
        ("node:a", vec![1.0, 0.0, 0.1]),
        ("node:b", vec![0.4, 0.6, 0.2]),
        ("node:c", vec![0.0, 1.0, 0.3]),
    ] {
        store
            .upsert_node(NodeRecord::new(
                node_id,
                ["Object"],
                json!({ "embedding": embedding }),
            ))
            .unwrap();
    }
    store
        .upsert_edge(
            EdgeRecord::new(
                "edge:a-b",
                "node:a",
                "RELATES_TO",
                "node:b",
                json!({ "features": [0.7, 0.1] }),
            )
            .with_confidence(0.92),
        )
        .unwrap();
    store
        .upsert_edge(
            EdgeRecord::new(
                "edge:b-c",
                "node:b",
                "RELATES_TO",
                "node:c",
                json!({ "features": [0.8, 0.2] }),
            )
            .with_confidence(0.9),
        )
        .unwrap();

    let request = DensificationRequest {
        tenant_id: "theorem".to_string(),
        seed_node_ids: vec!["node:a".to_string()],
        max_nodes: 16,
        max_depth: 2,
        min_path_confidence: 0.0,
        confidence_threshold: 0.30,
        confidence_ceiling: 0.72,
        max_candidates: 8,
        admission_tier: "advisory_inferred".to_string(),
        model_id: "pairformer-link-predict/test".to_string(),
        allowed_edge_types: vec!["RELATES_TO".to_string()],
    };
    let result = rank_pairformer_densification_candidates(
        &store.snapshot(),
        request.clone(),
        PairformerConfig {
            pair_dim: 8,
            single_dim: 8,
            blocks: 2,
            transition_hidden_dim: 16,
            max_nodes: 16,
            ..PairformerConfig::default()
        },
    )
    .unwrap();

    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.source_id == "node:a" && candidate.target_id == "node:c")
        .unwrap();
    assert_eq!(candidate.proposed_edge_type, "INFERRED_RELATES_TO");
    assert_eq!(candidate.confidence_ceiling, 0.72);
    assert_eq!(
        candidate.support_path_edge_ids,
        vec!["edge:a-b".to_string(), "edge:b-c".to_string()]
    );

    store
        .upsert_edge(EdgeRecord::new(
            "edge:a-c-direct",
            "node:a",
            "RELATES_TO",
            "node:c",
            json!({}),
        ))
        .unwrap();
    let result = rank_pairformer_densification_candidates(
        &store.snapshot(),
        request,
        PairformerConfig {
            pair_dim: 8,
            single_dim: 8,
            blocks: 2,
            transition_hidden_dim: 16,
            max_nodes: 16,
            ..PairformerConfig::default()
        },
    )
    .unwrap();

    assert!(result
        .candidates
        .iter()
        .all(|candidate| candidate.source_id != "node:a" || candidate.target_id != "node:c"));
}
