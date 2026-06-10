# CubeCL Scatter Aggregate Review

Status: reviewed against Burn `0.21.0` and CubeCL `0.10.0`, 2026-06-09.

## Verdict

The execution shape is correct:

- X axis maps to edges.
- Y axis maps to feature dimensions.
- Each `(edge, dim)` work item contributes one value.
- The `dim == 0` work item counts degree once per edge.

The first safe landing is now in `rustyred-thg-adapters`:

- `pairformer-burn-cubecl` installs Burn and CubeCL as optional dependencies.
- `pairformer_cubecl::fused_pair_aggregate_float_atomic_kernel` compiles the native float-atomic fast path.
- `pairformer_cubecl::fused_pair_aggregate_fixed_point_kernel` compiles the portable deterministic fixed-point path.
- Default builds keep the RustyRed graph crates free of a tensor backend.

## Corrections

1. `Tensor<F>` inside a CubeCL kernel must resolve to `cubecl::prelude::Tensor`, not `burn::tensor::Tensor`.

   Burn tensors are the caller-side graph API. CubeCL tensors are kernel arguments with `shape()` and `stride()` metadata.

2. Atomic outputs need atomic element types.

   Use `&mut Tensor<Atomic<F>>` for float atomic accumulation, or `&mut Tensor<Atomic<i32>>` for fixed-point accumulation. Then call `fetch_add` on the indexed atomic:

   ```rust
   aggregated[agg_mem_idx].fetch_add(val);
   degrees[deg_mem_idx].fetch_add(F::new(1.0));
   ```

3. CubeCL kernels do not support plain `return`.

   Put the body inside the bounds guard, or use `terminate!()`.

4. Launch is generic over the CubeCL runtime, not the Burn backend.

   Burn integration must unwrap or bridge backend primitives into CubeCL `TensorArg<R>` handles, then launch with `::<F, R>`.

5. Float atomics are a capability-gated fast path.

   CubeCL exposes feature probing through `client.properties().atomic_type_usage(...)`. CUDA and some native WGPU paths can support float add. Browser WebGPU should use fixed-point atomics or Burn `scatter(..., IndexingUpdateOp::Add)`.

6. Burn native scatter remains the portable high-level path for small graphs.

   The draft EdgeMPNN scatter-add shape is right: expand `edge_dst` to `[E, D]`, call `scatter(0, dst_idx, updated_pairs, IndexingUpdateOp::Add)`, and divide by a clamped degree tensor for mean aggregation.

## Open Bridge

The remaining integration work is the Burn-to-CubeCL bridge:

- Detect that the active Burn backend is CubeCL-backed.
- Extract the CubeCL client and primitive handles.
- Build `TensorArg<R>` with the Burn tensor shape and strides.
- Select float atomic only when `Atomic<f32>` supports `AtomicUsage::Add`.
- Otherwise select fixed-point or Burn native scatter.
