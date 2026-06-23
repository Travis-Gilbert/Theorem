//! CubeCL kernels for HOT feature preparation.
//!
//! The first GPU boundary for HOT is patching/alignment: it is embarrassingly
//! parallel, reused by both training and inference, and easy to parity-check
//! against `hot.rs`.  The higher-level Burn module consumes the resulting pair
//! representations while BRT tensorization grows behind this feature gate.

use cubecl::prelude::*;

#[cube(launch)]
pub fn hot_patch_sum_kernel<F: Float>(
    rows: &Tensor<F>,
    patched: &mut Tensor<F>,
    #[comptime] patch_size: u32,
) {
    let patch_idx = ABSOLUTE_POS_X as usize;
    let dim_idx = ABSOLUTE_POS_Y as usize;
    let patch_size = patch_size as usize;

    if patch_idx < patched.shape(0) && dim_idx < patched.shape(1) {
        let mut sum = F::new(0.0);
        let mut count = F::new(0.0);
        let start = patch_idx * patch_size;
        let mut offset = 0usize;
        while offset < patch_size {
            let row_idx = start + offset;
            if row_idx < rows.shape(0) && dim_idx < rows.shape(1) {
                let row_mem_idx = row_idx * rows.stride(0) + dim_idx * rows.stride(1);
                sum += rows[row_mem_idx];
                count += F::new(1.0);
            }
            offset += 1;
        }
        let out_mem_idx = patch_idx * patched.stride(0) + dim_idx * patched.stride(1);
        if count > F::new(0.0) {
            patched[out_mem_idx] = sum / count;
        } else {
            patched[out_mem_idx] = F::new(0.0);
        }
    }
}
