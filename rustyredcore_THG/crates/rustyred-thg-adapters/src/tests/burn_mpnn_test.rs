use burn::backend::ndarray::NdArrayDevice;
use burn::backend::NdArray;
use burn::tensor::{Tensor, TensorData};
use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::burn_mpnn::{aggregate_messages_burn, BurnAggregator, BurnEdgeMpnnLayer};
use crate::{
    aggregate_messages_fixed_point, rank_global_completion_candidates, FixedPointAggregator,
    GlobalCompletionConfig, GlobalCompletionRequest, DEFAULT_FIXED_POINT_SCALE,
};

type TestBackend = NdArray<f32>;

fn fixture_messages() -> (Vec<Vec<f32>>, Vec<usize>, usize) {
    let messages = vec![
        vec![1.25, 2.0, -0.5],
        vec![2.75, -1.0, 0.25],
        vec![10.0, 4.0, 1.5],
        vec![-3.0, 0.5, 2.0],
    ];
    let edge_dst = vec![1, 1, 0, 2];
    (messages, edge_dst, 4)
}

fn assert_rows_close(left: &[Vec<f32>], right: &[Vec<f32>], tolerance: f32) {
    assert_eq!(left.len(), right.len());
    for (left_row, right_row) in left.iter().zip(right) {
        assert_eq!(left_row.len(), right_row.len());
        for (a, b) in left_row.iter().zip(right_row) {
            assert!(
                (a - b).abs() <= tolerance,
                "value mismatch: {a} vs {b} (tolerance {tolerance})"
            );
        }
    }
}

#[test]
fn burn_scatter_add_matches_fixed_point_oracle() {
    let (messages, edge_dst, num_nodes) = fixture_messages();
    let device = NdArrayDevice::default();

    for mean_aggregate in [false, true] {
        let oracle = aggregate_messages_fixed_point(
            &messages,
            &edge_dst,
            num_nodes,
            DEFAULT_FIXED_POINT_SCALE,
            mean_aggregate,
        )
        .unwrap();
        let burn = aggregate_messages_burn::<TestBackend>(
            &device,
            &messages,
            &edge_dst,
            num_nodes,
            mean_aggregate,
        )
        .unwrap();
        assert_rows_close(&oracle, &burn, 1e-4);
    }
}

#[test]
fn burn_aggregator_drives_global_completion_to_the_same_candidates() {
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
    let request = GlobalCompletionRequest {
        tenant_id: "theorem".to_string(),
        seed_node_ids: vec!["node:a".to_string()],
        min_path_confidence: 0.0,
        confidence_threshold: 0.05,
        confidence_ceiling: 0.72,
        max_candidates: 16,
        admission_tier: "advisory_inferred".to_string(),
        model_id: "edge-mpnn-completion/test".to_string(),
        allowed_edge_types: vec![],
    };

    let fixed = rank_global_completion_candidates(
        &store.snapshot(),
        request.clone(),
        GlobalCompletionConfig::default(),
        &FixedPointAggregator::default(),
    )
    .unwrap();
    let burn = rank_global_completion_candidates(
        &store.snapshot(),
        request,
        GlobalCompletionConfig::default(),
        &BurnAggregator::<TestBackend> {
            device: NdArrayDevice::default(),
        },
    )
    .unwrap();

    assert_eq!(burn.aggregator_id, "burn_scatter_add");
    let fixed_pairs = fixed
        .candidates
        .iter()
        .map(|candidate| (candidate.source_id.clone(), candidate.target_id.clone()))
        .collect::<Vec<_>>();
    let burn_pairs = burn
        .candidates
        .iter()
        .map(|candidate| (candidate.source_id.clone(), candidate.target_id.clone()))
        .collect::<Vec<_>>();
    assert_eq!(fixed_pairs, burn_pairs);
    for (fixed_candidate, burn_candidate) in fixed.candidates.iter().zip(&burn.candidates) {
        assert!((fixed_candidate.confidence - burn_candidate.confidence).abs() <= 1e-3);
        assert_eq!(
            fixed_candidate.support_path_edge_ids,
            burn_candidate.support_path_edge_ids
        );
    }
}

#[test]
fn burn_edge_mpnn_layer_forward_matches_manual_update() {
    let device = NdArrayDevice::default();
    let node_states_rows = vec![vec![0.5_f32, -0.25], vec![0.1, 0.9], vec![0.0, 0.0]];
    let edge_src = vec![0_usize, 1];
    let edge_dst = vec![1_usize, 2];
    let relation_rows = vec![vec![1.0_f32, 0.5], vec![-0.5, 1.0]];
    let edge_confidence = vec![0.8_f32, 1.0];
    let self_gate = [0.7_f32, 0.7];
    let agg_gate = [0.9_f32, 0.9];

    // Manual expectation through the fixed-point oracle.
    let messages = edge_src
        .iter()
        .zip(&relation_rows)
        .zip(&edge_confidence)
        .map(|((src, relation), confidence)| {
            node_states_rows[*src]
                .iter()
                .zip(relation)
                .map(|(state, rel)| state * rel * confidence)
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    let aggregated =
        aggregate_messages_fixed_point(&messages, &edge_dst, 3, DEFAULT_FIXED_POINT_SCALE, false)
            .unwrap();
    let expected = node_states_rows
        .iter()
        .zip(&aggregated)
        .map(|(state_row, agg_row)| {
            state_row
                .iter()
                .zip(agg_row)
                .zip(self_gate.iter().zip(agg_gate.iter()))
                .map(|((state, agg), (sg, ag))| (state * sg + agg * ag).tanh())
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();

    let layer = BurnEdgeMpnnLayer::<TestBackend>::from_gates(&device, &self_gate, &agg_gate);
    let node_states = Tensor::<TestBackend, 2>::from_data(
        TensorData::new(node_states_rows.concat(), [3, 2]),
        &device,
    );
    let relations = Tensor::<TestBackend, 2>::from_data(
        TensorData::new(relation_rows.concat(), [2, 2]),
        &device,
    );
    let updated = layer
        .forward(
            node_states,
            &edge_src,
            &edge_dst,
            relations,
            &edge_confidence,
        )
        .unwrap();
    let updated_rows = updated
        .into_data()
        .to_vec::<f32>()
        .unwrap()
        .chunks(2)
        .map(|chunk| chunk.to_vec())
        .collect::<Vec<_>>();
    assert_rows_close(&expected, &updated_rows, 1e-4);
}

#[test]
fn cubecl_kernels_launch_on_wgpu_and_match_the_oracle() {
    use crate::burn_mpnn::wgpu_launch;

    let (messages, edge_dst, num_nodes) = fixture_messages();
    let oracle_sum = aggregate_messages_fixed_point(
        &messages,
        &edge_dst,
        num_nodes,
        DEFAULT_FIXED_POINT_SCALE,
        false,
    )
    .unwrap();
    let oracle_mean = aggregate_messages_fixed_point(
        &messages,
        &edge_dst,
        num_nodes,
        DEFAULT_FIXED_POINT_SCALE,
        true,
    )
    .unwrap();

    let fixed = wgpu_launch::launch_fixed_point_aggregate(
        &messages,
        &edge_dst,
        num_nodes,
        DEFAULT_FIXED_POINT_SCALE,
        false,
    )
    .unwrap();
    assert_rows_close(&oracle_sum, &fixed.aggregated, 1e-4);
    // One degree increment per edge, regardless of feature dimension.
    assert_eq!(fixed.degrees, vec![1.0, 2.0, 1.0, 0.0]);

    let fixed_mean = wgpu_launch::launch_fixed_point_aggregate(
        &messages,
        &edge_dst,
        num_nodes,
        DEFAULT_FIXED_POINT_SCALE,
        true,
    )
    .unwrap();
    assert_rows_close(&oracle_mean, &fixed_mean.aggregated, 1e-4);

    // The float-atomic fast path runs only where the device advertises it.
    match wgpu_launch::launch_float_atomic_aggregate(&messages, &edge_dst, num_nodes, false)
        .unwrap()
    {
        Some(float_out) => {
            assert_rows_close(&oracle_sum, &float_out.aggregated, 1e-3);
            assert_eq!(float_out.degrees, vec![1.0, 2.0, 1.0, 0.0]);
        }
        None => {
            assert!(!wgpu_launch::float_atomic_add_supported());
        }
    }

    let selected =
        wgpu_launch::launch_selected_aggregate(&messages, &edge_dst, num_nodes, false, false)
            .unwrap();
    assert_rows_close(&oracle_sum, &selected.aggregated, 1e-3);
}
