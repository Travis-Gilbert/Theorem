//! Burn tensor lane for EdgeMPNN aggregation (`pairformer-burn-cubecl`).
//!
//! This is the actual Burn/CubeCL aggregation the plan calls for: `select`
//! gathers source rows, `scatter` with `IndexingUpdateOp::Add` accumulates
//! messages per destination, and the CubeCL kernels in `pairformer_cubecl`
//! are launched (not just compiled) through the wgpu runtime with a
//! capability probe deciding float-atomic vs fixed-point. The deterministic
//! fixed-point function remains the parity oracle for every path.

use burn::tensor::backend::Backend;
use burn::tensor::{Int, Tensor, TensorData};

use rustyred_thg_core::{ThgError, ThgResult};

use crate::edge_mpnn::MessageAggregator;
use crate::reflexive::ScatterAggregationPath;

fn validate_messages(
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
    let message_tensor = Tensor::<B, 2>::from_data(
        TensorData::new(flat, [num_edges, feature_dim]),
        device,
    );
    let dst_indices = Tensor::<B, 1, Int>::from_data(
        TensorData::new(
            edge_dst.iter().map(|dst| *dst as i64).collect::<Vec<_>>(),
            [num_edges],
        ),
        device,
    );

    // scatter(dim=0, indices [E, D], values [E, D], Add) onto zeros [N, D].
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

/// [`MessageAggregator`] implementation over a Burn backend, so the sparse
/// EdgeMPNN completion scorer can run its aggregation on tensors.
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
        aggregate_messages_burn::<B>(&self.device, messages, edge_dst, num_nodes, mean_aggregate)
    }

    fn aggregator_id(&self) -> &'static str {
        "burn_scatter_add"
    }
}

/// One relation-aware EdgeMPNN layer over Burn tensors. `select` gathers the
/// source rows; messages compose source state with per-edge relation
/// embeddings and confidence; scatter-add aggregates; gated tanh updates.
#[derive(Clone, Debug)]
pub struct BurnEdgeMpnnLayer<B: Backend> {
    pub self_gate: Tensor<B, 1>,
    pub agg_gate: Tensor<B, 1>,
}

impl<B: Backend> BurnEdgeMpnnLayer<B> {
    pub fn from_gates(device: &B::Device, self_gate: &[f32], agg_gate: &[f32]) -> Self {
        Self {
            self_gate: Tensor::from_data(TensorData::new(self_gate.to_vec(), [self_gate.len()]), device),
            agg_gate: Tensor::from_data(TensorData::new(agg_gate.to_vec(), [agg_gate.len()]), device),
        }
    }

    pub fn forward(
        &self,
        node_states: Tensor<B, 2>,
        edge_src: &[usize],
        edge_dst: &[usize],
        relation_embeddings: Tensor<B, 2>,
        edge_confidence: &[f32],
    ) -> ThgResult<Tensor<B, 2>> {
        if edge_src.len() != edge_dst.len() || edge_src.len() != edge_confidence.len() {
            return Err(ThgError::new(
                "edge_mpnn_shape_mismatch",
                "edge_src, edge_dst, and edge_confidence must align",
            ));
        }
        let [num_nodes, feature_dim] = node_states.dims();
        if edge_src.iter().chain(edge_dst).any(|idx| *idx >= num_nodes) {
            return Err(ThgError::new(
                "edge_mpnn_index_out_of_bounds",
                "edge endpoint outside node_states rows",
            ));
        }
        let num_edges = edge_src.len();
        let device = node_states.device();

        let src_indices = Tensor::<B, 1, Int>::from_data(
            TensorData::new(
                edge_src.iter().map(|idx| *idx as i64).collect::<Vec<_>>(),
                [num_edges],
            ),
            &device,
        );
        // Row gather via select: messages start as source node states.
        let source_states = node_states.clone().select(0, src_indices);
        let confidence = Tensor::<B, 2>::from_data(
            TensorData::new(edge_confidence.to_vec(), [num_edges, 1]),
            &device,
        );
        let messages = source_states * relation_embeddings * confidence;

        let dst_indices = Tensor::<B, 1, Int>::from_data(
            TensorData::new(
                edge_dst.iter().map(|idx| *idx as i64).collect::<Vec<_>>(),
                [num_edges],
            ),
            &device,
        );
        let dst_expanded = dst_indices.unsqueeze_dim::<2>(1).repeat_dim(1, feature_dim);
        let aggregated = Tensor::<B, 2>::zeros([num_nodes, feature_dim], &device).scatter(
            0,
            dst_expanded,
            messages,
            burn::tensor::IndexingUpdateOp::Add,
        );

        let self_gate = self.self_gate.clone().unsqueeze_dim::<2>(0);
        let agg_gate = self.agg_gate.clone().unsqueeze_dim::<2>(0);
        Ok((node_states * self_gate + aggregated * agg_gate).tanh())
    }
}

/// Result of a CubeCL kernel launch, read back to host floats.
#[derive(Clone, Debug, PartialEq)]
pub struct CubeclAggregateOutput {
    pub aggregated: Vec<Vec<f32>>,
    pub degrees: Vec<f32>,
    pub path: ScatterAggregationPath,
}

#[cfg(feature = "pairformer-burn-cubecl")]
pub mod wgpu_launch {
    //! wgpu-runtime launches for the `pairformer_cubecl` kernels. The float
    //! atomic kernel runs only when the device advertises atomic f32 add;
    //! otherwise the fixed-point kernel carries the load. Both are validated
    //! against the deterministic fixed-point oracle in tests.

    use cubecl::features::AtomicUsage;
    use cubecl::ir::{StorageType, Type};
    use cubecl::prelude::*;
    use cubecl::wgpu::{WgpuDevice, WgpuRuntime};
    use cubecl::zspace::{Shape, Strides};

    use rustyred_thg_core::ThgResult;

    use super::{validate_messages, CubeclAggregateOutput};
    use crate::pairformer_cubecl::{
        fused_pair_aggregate_fixed_point_kernel, fused_pair_aggregate_float_atomic_kernel,
    };
    use crate::reflexive::{
        choose_scatter_aggregation_path, ScatterAggregationPath, ScatterAggregationRequest,
        DEFAULT_FIXED_POINT_SCALE,
    };

    fn wgpu_client() -> ComputeClient<WgpuRuntime> {
        WgpuRuntime::client(&WgpuDevice::default())
    }

    /// Probe whether the active wgpu device supports atomic f32 add.
    pub fn float_atomic_add_supported() -> bool {
        let client = wgpu_client();
        let ty = StorageType::Atomic(f32::as_type_native_unchecked().elem_type());
        let vector = Type::new(ty).with_vector_size(1);
        client
            .properties()
            .atomic_type_usage(vector)
            .contains(AtomicUsage::Add)
    }

    fn cube_grid(num_edges: usize, feature_dim: usize) -> (CubeCount, CubeDim) {
        (
            CubeCount::Static(
                num_edges.div_ceil(16) as u32,
                feature_dim.div_ceil(16) as u32,
                1,
            ),
            CubeDim { x: 16, y: 16, z: 1 },
        )
    }

    /// Launch the fixed-point CubeCL kernel on wgpu and read back floats.
    pub fn launch_fixed_point_aggregate(
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        scale: i64,
        mean_aggregate: bool,
    ) -> ThgResult<CubeclAggregateOutput> {
        let feature_dim = validate_messages(messages, edge_dst, num_nodes)?;
        if messages.is_empty() || feature_dim == 0 {
            return Ok(CubeclAggregateOutput {
                aggregated: vec![vec![0.0; feature_dim]; num_nodes],
                degrees: vec![0.0; num_nodes],
                path: ScatterAggregationPath::FixedPointAtomicCompatible,
            });
        }
        let scale = scale.max(1);
        let num_edges = messages.len();
        let client = wgpu_client();

        let scaled = messages
            .iter()
            .flat_map(|row| {
                row.iter()
                    .map(|value| (*value as f64 * scale as f64).round() as i32)
            })
            .collect::<Vec<i32>>();
        let dst = edge_dst.iter().map(|dst| *dst as u32).collect::<Vec<u32>>();
        let agg_zero = vec![0_i32; num_nodes * feature_dim];
        let deg_zero = vec![0_i32; num_nodes];

        let pairs_handle = client.create_from_slice(i32::as_bytes(&scaled));
        let dst_handle = client.create_from_slice(u32::as_bytes(&dst));
        let agg_handle = client.create_from_slice(i32::as_bytes(&agg_zero));
        let deg_handle = client.create_from_slice(i32::as_bytes(&deg_zero));

        let (cube_count, cube_dim) = cube_grid(num_edges, feature_dim);
        fused_pair_aggregate_fixed_point_kernel::launch::<WgpuRuntime>(
            &client,
            cube_count,
            cube_dim,
            unsafe {
                TensorArg::from_raw_parts(
                    pairs_handle.clone(),
                    Strides::new(&[feature_dim, 1]),
                    Shape::new([num_edges, feature_dim]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    dst_handle.clone(),
                    Strides::new(&[1]),
                    Shape::new([num_edges]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    agg_handle.clone(),
                    Strides::new(&[feature_dim, 1]),
                    Shape::new([num_nodes, feature_dim]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    deg_handle.clone(),
                    Strides::new(&[1, 1]),
                    Shape::new([num_nodes, 1]),
                )
            },
            true,
        );

        let agg_bytes = client.read_one_unchecked(agg_handle);
        let deg_bytes = client.read_one_unchecked(deg_handle);
        let agg_scaled = i32::from_bytes(&agg_bytes);
        let degrees_raw = i32::from_bytes(&deg_bytes);

        let degrees = degrees_raw
            .iter()
            .take(num_nodes)
            .map(|value| *value as f32)
            .collect::<Vec<_>>();
        let mut aggregated = Vec::with_capacity(num_nodes);
        for node_idx in 0..num_nodes {
            let divisor = if mean_aggregate {
                degrees[node_idx].max(1.0)
            } else {
                1.0
            };
            let row = (0..feature_dim)
                .map(|dim_idx| {
                    agg_scaled[node_idx * feature_dim + dim_idx] as f32 / scale as f32 / divisor
                })
                .collect::<Vec<_>>();
            aggregated.push(row);
        }
        Ok(CubeclAggregateOutput {
            aggregated,
            degrees,
            path: ScatterAggregationPath::FixedPointAtomicCompatible,
        })
    }

    /// Launch the float-atomic CubeCL kernel when the runtime supports it.
    /// Returns `None` on devices without atomic f32 add; callers fall back to
    /// the fixed-point launch.
    pub fn launch_float_atomic_aggregate(
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        mean_aggregate: bool,
    ) -> ThgResult<Option<CubeclAggregateOutput>> {
        if !float_atomic_add_supported() {
            return Ok(None);
        }
        let feature_dim = validate_messages(messages, edge_dst, num_nodes)?;
        if messages.is_empty() || feature_dim == 0 {
            return Ok(Some(CubeclAggregateOutput {
                aggregated: vec![vec![0.0; feature_dim]; num_nodes],
                degrees: vec![0.0; num_nodes],
                path: ScatterAggregationPath::NativeFloatAtomicFastPath,
            }));
        }
        let num_edges = messages.len();
        let client = wgpu_client();

        let flat = messages
            .iter()
            .flat_map(|row| row.iter().copied())
            .collect::<Vec<f32>>();
        let dst = edge_dst.iter().map(|dst| *dst as u32).collect::<Vec<u32>>();
        let agg_zero = vec![0.0_f32; num_nodes * feature_dim];
        let deg_zero = vec![0.0_f32; num_nodes];

        let pairs_handle = client.create_from_slice(f32::as_bytes(&flat));
        let dst_handle = client.create_from_slice(u32::as_bytes(&dst));
        let agg_handle = client.create_from_slice(f32::as_bytes(&agg_zero));
        let deg_handle = client.create_from_slice(f32::as_bytes(&deg_zero));

        let (cube_count, cube_dim) = cube_grid(num_edges, feature_dim);
        fused_pair_aggregate_float_atomic_kernel::launch::<f32, WgpuRuntime>(
            &client,
            cube_count,
            cube_dim,
            unsafe {
                TensorArg::from_raw_parts(
                    pairs_handle.clone(),
                    Strides::new(&[feature_dim, 1]),
                    Shape::new([num_edges, feature_dim]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    dst_handle.clone(),
                    Strides::new(&[1]),
                    Shape::new([num_edges]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    agg_handle.clone(),
                    Strides::new(&[feature_dim, 1]),
                    Shape::new([num_nodes, feature_dim]),
                )
            },
            unsafe {
                TensorArg::from_raw_parts(
                    deg_handle.clone(),
                    Strides::new(&[1, 1]),
                    Shape::new([num_nodes, 1]),
                )
            },
            true,
        );

        let agg_bytes = client.read_one_unchecked(agg_handle);
        let deg_bytes = client.read_one_unchecked(deg_handle);
        let agg_values = f32::from_bytes(&agg_bytes);
        let degrees_raw = f32::from_bytes(&deg_bytes);

        let degrees = degrees_raw.iter().take(num_nodes).copied().collect::<Vec<_>>();
        let mut aggregated = Vec::with_capacity(num_nodes);
        for node_idx in 0..num_nodes {
            let divisor = if mean_aggregate {
                degrees[node_idx].max(1.0)
            } else {
                1.0
            };
            let row = (0..feature_dim)
                .map(|dim_idx| agg_values[node_idx * feature_dim + dim_idx] / divisor)
                .collect::<Vec<_>>();
            aggregated.push(row);
        }
        Ok(Some(CubeclAggregateOutput {
            aggregated,
            degrees,
            path: ScatterAggregationPath::NativeFloatAtomicFastPath,
        }))
    }

    /// Policy-honoring entry: consult `choose_scatter_aggregation_path` with
    /// the probed float-atomic capability, then launch the selected kernel.
    pub fn launch_selected_aggregate(
        messages: &[Vec<f32>],
        edge_dst: &[usize],
        num_nodes: usize,
        deterministic_required: bool,
        mean_aggregate: bool,
    ) -> ThgResult<CubeclAggregateOutput> {
        let feature_dim = messages.first().map(Vec::len).unwrap_or(0);
        let path = choose_scatter_aggregation_path(ScatterAggregationRequest {
            num_edges: messages.len(),
            feature_dim,
            deterministic_required,
            browser_webgpu_target: false,
            float_atomic_add_available: float_atomic_add_supported(),
            burn_native_max_elements: 1,
        });
        match path {
            ScatterAggregationPath::NativeFloatAtomicFastPath => {
                if let Some(output) =
                    launch_float_atomic_aggregate(messages, edge_dst, num_nodes, mean_aggregate)?
                {
                    return Ok(output);
                }
                launch_fixed_point_aggregate(
                    messages,
                    edge_dst,
                    num_nodes,
                    DEFAULT_FIXED_POINT_SCALE,
                    mean_aggregate,
                )
            }
            _ => launch_fixed_point_aggregate(
                messages,
                edge_dst,
                num_nodes,
                DEFAULT_FIXED_POINT_SCALE,
                mean_aggregate,
            ),
        }
    }
}
