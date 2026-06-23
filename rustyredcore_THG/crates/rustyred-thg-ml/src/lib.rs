//! Shared graph tensor and message-passing primitives for THG learned organs.
//!
//! This crate owns reusable graph-to-tensor shapes and aggregation oracles.
//! Model-specific policy, admission, quarantine, and graph mutation stay in
//! the consuming crates.

pub mod multivector;

use serde::{Deserialize, Serialize};

use rustyred_thg_core::{ThgError, ThgResult};

pub use multivector::{
    binary_hamming_maxsim_score, binary_projection_bytes, exact_f16_bytes, exact_f32_bytes,
    exact_maxsim_score, quantize_sign_bits, rank_binary_hamming_maxsim, rank_exact_maxsim,
    recall_against_exact_top_k, rerank_exact_maxsim_bounded, storage_costs, BinaryMultiVectorSet,
    MaxSimAggregation, MaxSimScorer, MultiVectorEmbeddingSet, MultiVectorManifest,
    MultiVectorRecallReport, MultiVectorScore, MultiVectorStorageCost,
};

pub const DEFAULT_SCATTER_BURN_NATIVE_MAX_ELEMENTS: usize = 262_144;
pub const DEFAULT_FIXED_POINT_SCALE: i64 = 1_000_000;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct GraphTensorBatch {
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub edge_src: Vec<usize>,
    pub edge_dst: Vec<usize>,
    pub edge_type: Vec<usize>,
    pub edge_confidence: Vec<f32>,
    pub node_features: Vec<Vec<f32>>,
    pub edge_features: Vec<Vec<f32>>,
    pub relation_types: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScatterAggregationPath {
    /// Burn `select` for row gather plus tensor `scatter` sum update.
    BurnScatterAdd,
    /// Integer atomic-compatible accumulation, then rescale to floats.
    FixedPointAtomicCompatible,
    /// Native-only fast path. Valid only when the runtime advertises float atomics.
    NativeFloatAtomicFastPath,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScatterAggregationRequest {
    pub num_edges: usize,
    pub feature_dim: usize,
    pub deterministic_required: bool,
    pub browser_webgpu_target: bool,
    pub float_atomic_add_available: bool,
    pub burn_native_max_elements: usize,
}

impl ScatterAggregationRequest {
    pub fn normalized(mut self) -> Self {
        if self.burn_native_max_elements == 0 {
            self.burn_native_max_elements = DEFAULT_SCATTER_BURN_NATIVE_MAX_ELEMENTS;
        }
        self
    }
}

pub fn choose_scatter_aggregation_path(
    request: ScatterAggregationRequest,
) -> ScatterAggregationPath {
    let request = request.normalized();
    if request.deterministic_required || request.browser_webgpu_target {
        return ScatterAggregationPath::FixedPointAtomicCompatible;
    }

    let elements = request.num_edges.saturating_mul(request.feature_dim);
    if elements <= request.burn_native_max_elements {
        return ScatterAggregationPath::BurnScatterAdd;
    }
    if !request.float_atomic_add_available {
        return ScatterAggregationPath::FixedPointAtomicCompatible;
    }
    ScatterAggregationPath::NativeFloatAtomicFastPath
}

pub trait MessageAggregator {
    fn aggregate(
        &self,
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        mean_aggregate: bool,
    ) -> ThgResult<Vec<Vec<f32>>>;

    fn aggregator_id(&self) -> &'static str;
}

#[derive(Clone, Copy, Debug)]
pub struct FixedPointAggregator {
    pub scale: i64,
}

impl Default for FixedPointAggregator {
    fn default() -> Self {
        Self {
            scale: DEFAULT_FIXED_POINT_SCALE,
        }
    }
}

impl MessageAggregator for FixedPointAggregator {
    fn aggregate(
        &self,
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        mean_aggregate: bool,
    ) -> ThgResult<Vec<Vec<f32>>> {
        aggregate_messages_fixed_point(messages, edge_dst, num_nodes, self.scale, mean_aggregate)
    }

    fn aggregator_id(&self) -> &'static str {
        "fixed_point_deterministic"
    }
}

/// Deterministic fixed-point scatter aggregation for parity fixtures and for
/// backends where float atomics are unavailable. It mirrors the CubeCL kernel's
/// "one edge contributes once to degree" contract.
pub fn aggregate_messages_fixed_point(
    messages: &[Vec<f32>],
    edge_dst: &[usize],
    num_nodes: usize,
    scale: i64,
    mean_aggregate: bool,
) -> ThgResult<Vec<Vec<f32>>> {
    let scale = scale.max(1);
    validate_messages(messages, edge_dst, num_nodes)?;
    let feature_dim = messages.first().map(Vec::len).unwrap_or(0);

    let mut sums = vec![vec![0_i64; feature_dim]; num_nodes];
    let mut degrees = vec![0_i64; num_nodes];
    for (message, dst) in messages.iter().zip(edge_dst) {
        degrees[*dst] += 1;
        for (slot, value) in sums[*dst].iter_mut().zip(message) {
            *slot += (*value as f64 * scale as f64).round() as i64;
        }
    }

    let mut out = vec![vec![0.0_f32; feature_dim]; num_nodes];
    for node_idx in 0..num_nodes {
        let divisor = if mean_aggregate {
            degrees[node_idx].max(1) as f32
        } else {
            1.0
        };
        for dim_idx in 0..feature_dim {
            out[node_idx][dim_idx] = sums[node_idx][dim_idx] as f32 / scale as f32 / divisor;
        }
    }
    Ok(out)
}

pub fn validate_messages(
    messages: &[Vec<f32>],
    edge_dst: &[usize],
    num_nodes: usize,
) -> ThgResult<usize> {
    if messages.len() != edge_dst.len() {
        return Err(ThgError::new(
            "scatter_shape_mismatch",
            "messages and edge_dst must have the same length",
        ));
    }
    let feature_dim = messages.first().map(Vec::len).unwrap_or(0);
    if messages.iter().any(|message| message.len() != feature_dim) {
        return Err(ThgError::new(
            "scatter_shape_mismatch",
            "all message rows must have the same feature dimension",
        ));
    }
    if edge_dst.iter().any(|dst| *dst >= num_nodes) {
        return Err(ThgError::new(
            "scatter_index_out_of_bounds",
            "edge_dst contains a destination outside num_nodes",
        ));
    }
    Ok(feature_dim)
}

#[cfg(feature = "burn")]
pub mod burn {
    use burn::tensor::backend::Backend;
    use burn::tensor::{Int, Tensor, TensorData};

    use rustyred_thg_core::{ThgError, ThgResult};

    use crate::{validate_messages, MessageAggregator};

    /// Burn-native scatter-add aggregation: the portable high-level tensor path.
    pub fn aggregate_messages_burn<B: Backend>(
        device: &B::Device,
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        mean_aggregate: bool,
    ) -> ThgResult<Vec<Vec<f32>>> {
        let feature_dim = validate_messages(messages, edge_dst, num_nodes)?;
        if messages.is_empty() || feature_dim == 0 {
            return Ok(vec![vec![0.0; feature_dim]; num_nodes]);
        }
        let num_edges = messages.len();

        let flat = messages
            .iter()
            .flat_map(|row| row.iter().copied())
            .collect::<Vec<f32>>();
        let message_tensor =
            Tensor::<B, 2>::from_data(TensorData::new(flat, [num_edges, feature_dim]), device);
        let dst_indices = Tensor::<B, 1, Int>::from_data(
            TensorData::new(
                edge_dst.iter().map(|dst| *dst as i64).collect::<Vec<_>>(),
                [num_edges],
            ),
            device,
        );

        let dst_expanded = dst_indices
            .clone()
            .unsqueeze_dim::<2>(1)
            .repeat_dim(1, feature_dim);
        let aggregated = Tensor::<B, 2>::zeros([num_nodes, feature_dim], device).scatter(
            0,
            dst_expanded,
            message_tensor,
            burn::tensor::IndexingUpdateOp::Add,
        );

        let aggregated = if mean_aggregate {
            let ones = Tensor::<B, 2>::ones([num_edges, 1], device);
            let degrees = Tensor::<B, 2>::zeros([num_nodes, 1], device).scatter(
                0,
                dst_indices.unsqueeze_dim::<2>(1),
                ones,
                burn::tensor::IndexingUpdateOp::Add,
            );
            aggregated / degrees.clamp_min(1.0)
        } else {
            aggregated
        };

        let values = aggregated
            .into_data()
            .to_vec::<f32>()
            .map_err(|error| ThgError::new("burn_tensor_readback", format!("{error:?}")))?;
        Ok(values
            .chunks(feature_dim)
            .map(|chunk| chunk.to_vec())
            .collect())
    }

    #[derive(Clone, Debug)]
    pub struct BurnAggregator<B: Backend> {
        pub device: B::Device,
    }

    impl<B: Backend> MessageAggregator for BurnAggregator<B> {
        fn aggregate(
            &self,
            messages: &[Vec<f32>],
            edge_dst: &[usize],
            num_nodes: usize,
            mean_aggregate: bool,
        ) -> ThgResult<Vec<Vec<f32>>> {
            aggregate_messages_burn::<B>(
                &self.device,
                messages,
                edge_dst,
                num_nodes,
                mean_aggregate,
            )
        }

        fn aggregator_id(&self) -> &'static str {
            "burn_scatter_add"
        }
    }
}

#[cfg(feature = "burn")]
pub use self::burn::{aggregate_messages_burn, BurnAggregator};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_point_sum_aggregation_matches_expected_rows() {
        let messages = vec![vec![1.25, 2.0], vec![0.75, -1.0], vec![3.0, 4.0]];
        let edge_dst = vec![0, 0, 2];

        let aggregated = aggregate_messages_fixed_point(
            &messages,
            &edge_dst,
            3,
            DEFAULT_FIXED_POINT_SCALE,
            false,
        )
        .unwrap();

        assert_eq!(
            aggregated,
            vec![vec![2.0, 1.0], vec![0.0, 0.0], vec![3.0, 4.0]]
        );
    }

    #[test]
    fn fixed_point_mean_aggregation_divides_by_destination_degree() {
        let messages = vec![vec![1.0, 3.0], vec![3.0, 5.0], vec![10.0, 20.0]];
        let edge_dst = vec![1, 1, 2];

        let aggregated = aggregate_messages_fixed_point(
            &messages,
            &edge_dst,
            3,
            DEFAULT_FIXED_POINT_SCALE,
            true,
        )
        .unwrap();

        assert_eq!(
            aggregated,
            vec![vec![0.0, 0.0], vec![2.0, 4.0], vec![10.0, 20.0]]
        );
    }

    #[test]
    fn fixed_point_rejects_out_of_bounds_destination() {
        let err =
            aggregate_messages_fixed_point(&[vec![1.0]], &[2], 2, DEFAULT_FIXED_POINT_SCALE, false)
                .expect_err("dst 2 is outside num_nodes 2");

        assert_eq!(err.code, "scatter_index_out_of_bounds");
    }

    #[test]
    fn scatter_policy_selects_expected_paths() {
        let small = choose_scatter_aggregation_path(ScatterAggregationRequest {
            num_edges: 4,
            feature_dim: 16,
            deterministic_required: false,
            browser_webgpu_target: false,
            float_atomic_add_available: false,
            burn_native_max_elements: 1024,
        });
        assert_eq!(small, ScatterAggregationPath::BurnScatterAdd);

        let portable = choose_scatter_aggregation_path(ScatterAggregationRequest {
            num_edges: 4096,
            feature_dim: 1024,
            deterministic_required: true,
            browser_webgpu_target: false,
            float_atomic_add_available: true,
            burn_native_max_elements: 1024,
        });
        assert_eq!(portable, ScatterAggregationPath::FixedPointAtomicCompatible);

        let browser = choose_scatter_aggregation_path(ScatterAggregationRequest {
            num_edges: 4,
            feature_dim: 16,
            deterministic_required: false,
            browser_webgpu_target: true,
            float_atomic_add_available: true,
            burn_native_max_elements: 1024,
        });
        assert_eq!(browser, ScatterAggregationPath::FixedPointAtomicCompatible);

        let native = choose_scatter_aggregation_path(ScatterAggregationRequest {
            num_edges: 4096,
            feature_dim: 1024,
            deterministic_required: false,
            browser_webgpu_target: false,
            float_atomic_add_available: true,
            burn_native_max_elements: 1024,
        });
        assert_eq!(native, ScatterAggregationPath::NativeFloatAtomicFastPath);
    }
}
