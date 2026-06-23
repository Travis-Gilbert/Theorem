//! CubeCL kernels for Pairformer and EdgeMPNN aggregation fast paths.
//!
//! These kernels are compiled only with `pairformer-burn-cubecl`. Launch code
//! must still check backend atomic support before selecting the float path.

use cubecl::prelude::*;

#[cube(launch)]
pub fn fused_pair_aggregate_float_atomic_kernel<F: Float>(
    updated_pairs: &Tensor<F>,
    edge_dst: &Tensor<u32>,
    aggregated: &mut Tensor<Atomic<F>>,
    degrees: &mut Tensor<Atomic<F>>,
    #[comptime] track_degree: bool,
) {
    let edge_idx = ABSOLUTE_POS_X as usize;
    let dim_idx = ABSOLUTE_POS_Y as usize;

    if edge_idx < updated_pairs.shape(0) && dim_idx < updated_pairs.shape(1) {
        let dst_node = edge_dst[edge_idx] as usize;
        let pair_mem_idx = edge_idx * updated_pairs.stride(0) + dim_idx * updated_pairs.stride(1);
        let agg_mem_idx = dst_node * aggregated.stride(0) + dim_idx * aggregated.stride(1);
        let val = updated_pairs[pair_mem_idx];

        aggregated[agg_mem_idx].fetch_add(val);

        if track_degree && dim_idx == 0 {
            let deg_mem_idx = dst_node * degrees.stride(0);
            degrees[deg_mem_idx].fetch_add(F::new(1.0));
        }
    }
}

#[cube(launch)]
pub fn fused_pair_aggregate_fixed_point_kernel(
    updated_pairs_scaled: &Tensor<i32>,
    edge_dst: &Tensor<u32>,
    aggregated_scaled: &mut Tensor<Atomic<i32>>,
    degrees: &mut Tensor<Atomic<i32>>,
    #[comptime] track_degree: bool,
) {
    let edge_idx = ABSOLUTE_POS_X as usize;
    let dim_idx = ABSOLUTE_POS_Y as usize;

    if edge_idx < updated_pairs_scaled.shape(0) && dim_idx < updated_pairs_scaled.shape(1) {
        let dst_node = edge_dst[edge_idx] as usize;
        let pair_mem_idx =
            edge_idx * updated_pairs_scaled.stride(0) + dim_idx * updated_pairs_scaled.stride(1);
        let agg_mem_idx =
            dst_node * aggregated_scaled.stride(0) + dim_idx * aggregated_scaled.stride(1);
        let val = updated_pairs_scaled[pair_mem_idx];

        aggregated_scaled[agg_mem_idx].fetch_add(val);

        if track_degree && dim_idx == 0 {
            let deg_mem_idx = dst_node * degrees.stride(0);
            degrees[deg_mem_idx].fetch_add(1);
        }
    }
}
