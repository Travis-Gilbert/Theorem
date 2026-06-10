use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::{
    apply_low_rank_adapter, load_adapter_factors, load_node_representation,
    reflexive_match_inference, upsert_adapter_factors_sidecar, upsert_representation_sidecar,
    DensificationRequest, LowRankAdapterFactors, PairformerConfig, RepresentationSidecarInput,
    RepresentationTargetKind,
};

fn match_request() -> DensificationRequest {
    DensificationRequest {
        tenant_id: "theorem".to_string(),
        seed_node_ids: vec!["node:a".to_string()],
        max_nodes: 16,
        max_depth: 2,
        min_path_confidence: 0.0,
        confidence_threshold: 0.30,
        confidence_ceiling: 0.72,
        max_candidates: 8,
        admission_tier: "advisory_inferred".to_string(),
        model_id: "pairformer-match-inference/test".to_string(),
        allowed_edge_types: vec![],
    }
}

fn pairformer_config() -> PairformerConfig {
    PairformerConfig {
        pair_dim: 8,
        single_dim: 8,
        blocks: 2,
        transition_hidden_dim: 16,
        max_nodes: 16,
        ..PairformerConfig::default()
    }
}

fn chain_store_with_sidecars() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["node:a", "node:b", "node:c"] {
        store
            .upsert_node(NodeRecord::new(node_id, ["Object"], json!({})))
            .unwrap();
    }
    store
        .upsert_edge(
            EdgeRecord::new("edge:a-b", "node:a", "RELATES_TO", "node:b", json!({}))
                .with_confidence(0.92),
        )
        .unwrap();
    store
        .upsert_edge(
            EdgeRecord::new("edge:b-c", "node:b", "RELATES_TO", "node:c", json!({}))
                .with_confidence(0.9),
        )
        .unwrap();

    for (node_id, repr_id, embedding) in [
        ("node:a", "repr-a", vec![1.0_f32, 0.0, 0.2, 0.1]),
        ("node:b", "repr-b", vec![0.4, 0.6, 0.1, 0.3]),
        ("node:c", "repr-c", vec![0.1, 0.9, 0.4, 0.2]),
    ] {
        upsert_representation_sidecar(
            &mut store,
            RepresentationSidecarInput {
                tenant_id: "theorem".to_string(),
                representation_id: repr_id.to_string(),
                target_kind: RepresentationTargetKind::Node,
                target_id: node_id.to_string(),
                model_id: "graphlora-base/test".to_string(),
                embedding,
                adapter_ids: vec!["adapter:alpha".to_string()],
                graph_version: 3,
                metadata: json!({}),
                manifest_version: 1,
            },
            Some("test"),
        )
        .unwrap();
    }
    store
}

#[test]
fn low_rank_adapter_application_is_exact_and_dimension_checked() {
    // Identity-free zero factors: no delta.
    let zero = LowRankAdapterFactors {
        adapter_id: "adapter:zero".to_string(),
        rank: 2,
        input_dim: 3,
        alpha: 4.0,
        down: vec![0.0; 6],
        up: vec![0.0; 6],
    }
    .validated()
    .unwrap();
    let base = vec![0.5_f32, -1.0, 2.0];
    assert_eq!(apply_low_rank_adapter(&base, &zero).unwrap(), base);

    // Rank-1 crafted factors: delta = up * (down . x) * alpha/rank.
    let crafted = LowRankAdapterFactors {
        adapter_id: "adapter:crafted".to_string(),
        rank: 1,
        input_dim: 3,
        alpha: 2.0,
        down: vec![1.0, 0.0, 0.0],
        up: vec![0.0, 1.0, 0.0],
    }
    .validated()
    .unwrap();
    // down . x = 0.5; scale = alpha/rank = 2.0; delta on dim 1 = 0.5 * 2.0.
    let adapted = apply_low_rank_adapter(&base, &crafted).unwrap();
    assert_eq!(adapted, vec![0.5, 0.0, 2.0]);

    let mismatch = apply_low_rank_adapter(&[1.0, 2.0], &crafted).unwrap_err();
    assert_eq!(mismatch.code, "adapter_dimension_mismatch");
}

#[test]
fn factor_sidecar_round_trips_and_validates() {
    let mut store = InMemoryGraphStore::new();
    let factors = LowRankAdapterFactors {
        adapter_id: "adapter:alpha".to_string(),
        rank: 2,
        input_dim: 4,
        alpha: 1.5,
        down: (0..8).map(|idx| idx as f32 * 0.1).collect(),
        up: (0..8).map(|idx| idx as f32 * -0.05).collect(),
    };
    upsert_adapter_factors_sidecar(&mut store, "theorem", factors.clone(), Some("test")).unwrap();

    let loaded = load_adapter_factors(&store, "theorem", "adapter:alpha")
        .unwrap()
        .expect("factors loaded");
    assert_eq!(loaded, factors);

    let invalid = LowRankAdapterFactors {
        adapter_id: "adapter:bad".to_string(),
        rank: 2,
        input_dim: 4,
        alpha: 1.0,
        down: vec![0.0; 3],
        up: vec![0.0; 8],
    };
    let error =
        upsert_adapter_factors_sidecar(&mut store, "theorem", invalid, Some("test")).unwrap_err();
    assert_eq!(error.code, "invalid_adapter_factors");
}

#[test]
fn representation_join_prefers_highest_graph_version() {
    let mut store = chain_store_with_sidecars();
    // A newer representation for node:a supersedes the older one.
    upsert_representation_sidecar(
        &mut store,
        RepresentationSidecarInput {
            tenant_id: "theorem".to_string(),
            representation_id: "repr-a-v2".to_string(),
            target_kind: RepresentationTargetKind::Node,
            target_id: "node:a".to_string(),
            model_id: "graphlora-base/test".to_string(),
            embedding: vec![0.9, 0.1, 0.0, 0.0],
            adapter_ids: vec![],
            graph_version: 7,
            metadata: json!({}),
            manifest_version: 1,
        },
        Some("test"),
    )
    .unwrap();

    let representation = load_node_representation(&store, "theorem", "node:a")
        .unwrap()
        .expect("representation");
    assert_eq!(representation.graph_version, 7);
    assert_eq!(representation.embedding, vec![0.9, 0.1, 0.0, 0.0]);
}

#[test]
fn match_inference_joins_sidecar_applies_adapters_and_stays_advisory() {
    let mut store = chain_store_with_sidecars();
    upsert_adapter_factors_sidecar(
        &mut store,
        "theorem",
        LowRankAdapterFactors {
            adapter_id: "adapter:alpha".to_string(),
            rank: 1,
            input_dim: 4,
            alpha: 1.0,
            down: vec![0.5, 0.5, 0.0, 0.0],
            up: vec![0.1, 0.1, 0.1, 0.1],
        },
        Some("test"),
    )
    .unwrap();

    let node_ids = vec![
        "node:a".to_string(),
        "node:b".to_string(),
        "node:c".to_string(),
    ];
    let result =
        reflexive_match_inference(&store, &node_ids, match_request(), pairformer_config()).unwrap();

    assert_eq!(result.representations_joined, 3);
    assert_eq!(result.adapters_applied, 3);
    assert!(result.adapter_skips.is_empty());
    let candidate = result
        .candidates
        .iter()
        .find(|candidate| candidate.source_id == "node:a" && candidate.target_id == "node:c")
        .expect("a->c advisory candidate");
    assert_eq!(candidate.proposed_edge_type, "INFERRED_RELATES_TO");
    assert!(candidate.confidence <= 0.72);
    // Advisory only: no topology edge was written.
    assert!(store.get_edge("edge:a-c").is_none());

    // Sidecar nodes never enter the scored neighborhood as topology.
    assert!(result
        .considered_node_ids
        .iter()
        .all(|node_id| !node_id.starts_with("representation_sidecar:")));
}

#[test]
fn match_inference_records_skips_for_missing_or_mismatched_factors() {
    let mut store = chain_store_with_sidecars();
    // Factors whose input_dim does not match the 4-dim representations.
    upsert_adapter_factors_sidecar(
        &mut store,
        "theorem",
        LowRankAdapterFactors {
            adapter_id: "adapter:alpha".to_string(),
            rank: 1,
            input_dim: 3,
            alpha: 1.0,
            down: vec![0.5, 0.5, 0.0],
            up: vec![0.1, 0.1, 0.1],
        },
        Some("test"),
    )
    .unwrap();

    let node_ids = vec![
        "node:a".to_string(),
        "node:b".to_string(),
        "node:c".to_string(),
    ];
    let result =
        reflexive_match_inference(&store, &node_ids, match_request(), pairformer_config()).unwrap();

    assert_eq!(result.adapters_applied, 0);
    assert_eq!(result.adapter_skips.len(), 3);
    assert!(result.adapter_skips[0].contains("adapter:alpha"));
    // Inference still ran on the frozen base representations.
    assert_eq!(result.representations_joined, 3);
}
