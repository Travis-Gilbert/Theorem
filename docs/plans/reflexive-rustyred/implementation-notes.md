# Reflexive RustyRed Implementation Notes

Status: implementation slice on `Travis-Gilbert/Reflexive-red`, 2026-06-09.

Source inputs:

- `/Users/travisgilbert/Downloads/reflexive-rustyred-plan.md`
- `/Users/travisgilbert/Downloads/cubecl_scatter_aggregate.rs`
- Claude.ai correction: float atomic aggregation is a native fast path only; fixed-point or Burn scatter is the portable fallback.

## Landed Shape

The implementation keeps the shared invariant from the plan:

> Learned code ranks or steers within a bounded, enumerated space. It does not author free-form output.

### `rustyred-thg-adapters::reflexive`

- `choose_scatter_aggregation_path` selects Burn scatter for small tensors, deterministic fixed-point aggregation for portable or browser-WebGPU paths, and the native float-atomic fast path only when the runtime advertises support.
- `aggregate_messages_fixed_point` provides the deterministic fallback contract for the CubeCL scatter-add kernel shape.
- `upsert_representation_sidecar` writes learned representations as sidecar nodes keyed by node or edge id. It does not mutate topology nodes with tensor fields.
- `rank_densification_candidates` extracts bounded two-hop candidates from a capped neighborhood and returns ranked advisory inferred-edge candidates.
- `rank_pairformer_densification_candidates` runs the bounded Pairformer scorer, converts supported link scores into capped advisory inferred-edge candidates, and suppresses existing direct edges.
- `quarantine_densification_candidates` writes candidate nodes plus support-path metadata. It does not insert inferred edges into live topology.

### `rustyred-thg-adapters::pairformer`

- `run_pairformer` implements the bounded Pairformer core: dense pair representations, single representations, outgoing and incoming triangle multiplicative updates, starting-node and ending-node triangle attention, SwiGLU transitions, and single attention with pair bias.
- Link scoring keeps bounded support explicit. Unsupported links stay below the advisory candidate threshold, while supported paths carry support edge ids and node ids forward into quarantine.

### `rustyred-thg-adapters::pairformer_cubecl`

- `pairformer-burn-cubecl` installs Burn `0.21.0` and CubeCL `0.10.0` as optional dependencies.
- The feature compiles a float-atomic CubeCL scatter kernel and a fixed-point CubeCL scatter kernel.
- The Burn-to-CubeCL tensor bridge remains separate from graph storage and requires runtime capability checks before selecting float atomics.

### `rustyred-thg-adapters::situation_search`

- `score_context_atoms` implements the memory/context scorer from the plan. It ranks already-known atoms under a hard token budget, with explicit features for similarity, use-receipt success/failure, recency, graph degree, pins, and required atoms.
- `context_candidates_from_similar_situation` bridges semantic memory retrieval into context scoring so vector hits can become budgeted context atoms without adding a second retrieval path.
- `record_context_scoring_result` writes a `ContextPack` receipt plus `CONTEXT_ATOM_SELECTED` edges. This creates the future label surface for a learned memory GNN while keeping the current scorer deterministic and inspectable.

### `rustyred-thg-server::cypher::planner`

- `steer_plan_candidates` implements the Bao-style contract: the native rule planner enumerates candidate plans; learned metrics can pick among those candidates only after a cold-start observation floor is met.
- The native candidate remains the fallback when observations are sparse or learned candidates exceed the tail-risk ceiling.

## Dependency Decision

Burn and CubeCL are optional integration dependencies, not storage dependencies. The RustyRed graph crates compile without a tensor backend by default. Enabling `pairformer-burn-cubecl` brings in the tensor/GPU lane and validates the CubeCL kernels without making `NodeRecord`, `EdgeRecord`, or `GraphStore` backend-parametric.

## Deferred With Reason

- The actual Burn-to-CubeCL kernel launch bridge is not wired in this slice. It depends on extracting backend primitives as `TensorArg` handles and checking runtime atomic support. The portable fixed-point path and policy gate are in place first so the native fast path cannot become the only path by accident.
- Live Cypher execution does not call the steered optimizer yet. The current server planner module owns post-MATCH row-shape operators, not a full candidate-plan enumerator. The steering function is tested and ready for the later candidate enumeration seam.
