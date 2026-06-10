# Reflexive RustyRed Implementation Notes

Status: implementation slices on `Travis-Gilbert/Reflexive-red`, 2026-06-09 (Codex)
plus 2026-06-10 completion pass (Claude Code) closing the deferred seams below.

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

## Completion Pass (2026-06-10)

Both deferrals from the first slice are closed, plus the remaining spec seams:

### `rustyred-thg-adapters::edge_mpnn` (new)

- `rank_global_completion_candidates` is the sparse NBFNet-style global
  completion scorer: per-seed generalized Bellman-Ford message passing over
  COO edges with layer and frontier caps, DistMult-style relation
  composition, parent-pointer provenance chains, and advisory
  `InferredEdgeCandidate` output feeding the existing quarantine pipeline.
  Candidates without a provenance chain back to the seed are dropped.
- The `MessageAggregator` trait is the aggregation seam: the default
  `FixedPointAggregator` IS `aggregate_messages_fixed_point` (the formerly
  dangling fallback is now the consumed primitive), and the policy decision
  from `choose_scatter_aggregation_path` is consulted per layer and reported
  in the result.

### `rustyred-thg-adapters::burn_mpnn` (new, behind `pairformer-burn-cubecl`)

- `aggregate_messages_burn` is the actual Burn tensor path: `select` row
  gather, `scatter(0, indices, values, IndexingUpdateOp::Add)` accumulation,
  clamped-degree mean. Parity-tested against the fixed-point oracle on the
  NdArray backend (`ndarray` added to the burn feature set).
- `BurnAggregator<B>` implements `MessageAggregator`, so the sparse
  completion scorer runs its aggregation on Burn tensors and produces the
  same candidates as the deterministic path (verified by test).
- `BurnEdgeMpnnLayer<B>` is the relation-aware EdgeMPNN layer over tensors.
- `wgpu_launch` LAUNCHES the CubeCL kernels (they were previously compiled
  but never invoked): `launch_fixed_point_aggregate` and
  `launch_float_atomic_aggregate` build `TensorArg` handles, dispatch over
  the wgpu runtime, and read results back; `float_atomic_add_supported`
  probes `atomic_type_usage(...).contains(AtomicUsage::Add)` so the float
  path runs only where the device advertises it; `launch_selected_aggregate`
  honors the scatter policy. Verified green on Metal via wgpu on 2026-06-10.

### `rustyred-thg-adapters::reflexive_executor` (new)

- The sidecar READ path: `load_node_representation` joins a topology node
  with its `RepresentationSidecar` via incoming `REPRESENTS_NODE` edges
  (highest `graph_version` wins). The sidecar write existed; nothing read it.
- GraphLoRA-pattern adapter application: `LowRankAdapterFactors` (rank,
  alpha, down/up factor matrices) live in their own `AdapterFactorSidecar`
  node keyed by adapter id, never in the node struct;
  `apply_low_rank_adapter` adds `up @ (down @ x) * alpha/rank` to the frozen
  base representation. Dimension mismatches are recorded skips, not silent
  identity.
- `score_match_neighborhood` is the pure executor scorer (topology + sidecar
  + adapters -> bounded Pairformer -> advisory candidates);
  `reflexive_match_inference` is the store-generic wrapper over the new
  read-only `ReflexiveReadStore` trait (explicit impls; a blanket impl over
  `AdapterGraphStore` would block downstream store wrappers).

### `rustyred-thg-server` in-DB inference during MATCH

- `PublicCypherBody.reflexive_inference` (default false) opts a read query
  into reflexive inference. The `EdgeVarLength` and `EdgeChain` arms collect
  the walked neighborhood (capped at 64 nodes), join it with the
  representation sidecar through `TenantGraphStore`'s `ReflexiveReadStore`
  impl, run the Pairformer, and attach a `reflexive` advisory block to the
  response. Rows are never modified; no edge is ever written; advisory
  failures degrade to an error note inside the block.

### `rustyred-thg-server` live steered optimizer

- `planner::enumerate_edge_pattern_candidates` enumerates the real candidate
  set for single-hop relationship MATCH: native anchor-left expand-out, plus
  anchor-right expand-in when the right node is independently anchorable.
- `planner::PlanSteeringState` (on `AppState.plan_steering`) records measured
  execution cost units per `(query shape, candidate)` and snapshots
  `PlanObservationMetrics` (uncertainty = standard error of mean cost).
- `execute_cypher_query_with_steering` runs the loop live: enumerate ->
  `steer_plan_candidates` behind the cold-start floor -> execute the selected
  enumerated plan (`execute_edge_anchor_left` / `execute_edge_anchor_right`)
  -> record the observation -> expose `plan_candidate` + `plan_steering` in
  response stats. The HTTP router feeds it; the ranker cannot produce a plan
  shape that was not enumerated.

### Context scorer receipt read-back (`situation_search`)

- `record_context_use_outcome` writes `ContextUseReceipt` nodes linked from
  context packs (`CONTEXT_PACK_OUTCOME`): the use-receipt label source the
  plan names.
- `enrich_context_candidates_from_store` closes the loop that previously
  zero-filled: selection edges -> `use_count`, pack outcomes ->
  `success_count`/`failure_count`, graph degree, age, and pin state now feed
  the scorer from graph truth, and receipts measurably move rankings.

## Trainable Pairformer (2026-06-10, second pass)

`rustyred-thg-adapters::burn_pairformer` (behind `pairformer-burn-cubecl`,
which now enables burn `autodiff`) is the learned counterpart to the
deterministic scorer:

- Real `Param` weights in the AF3 block structure: sigmoid-gated triangle
  multiplicative updates (outgoing/incoming as separate module instances),
  multi-head triangle attention with logits biased by the third edge
  (ending-node orientation runs its own weights on the transposed grid),
  SwiGLU transitions, single attention with pair bias, and no pair
  flow-back from the single stream. Pair grid math is per-channel batched
  matmul over `[C, N, N]`.
- Self-supervised masked-edge training: per epoch, a fraction of observed
  directed edges is hidden from the input grid; the model scores hidden
  pairs against sampled non-edges under BCE-with-logits (stable
  `log_sigmoid` form), AdamW steps, deterministic xorshift sampling, and
  `B::seed` for reproducible init. The graph is its own label source.
- Proof of learning in tests: on a planted compositional rule
  (`a -r1-> b -r2-> c` implies `a -r3-> c`) with two triples' closures
  held out entirely, training reduces the smoothed loss and the trained
  model ranks held-out closures above structural negatives, beating an
  untrained baseline.
- Persistence and registry: `save_pairformer_file` / `load_pairformer_file`
  (BinFileRecorder, exact score round-trip verified) and
  `register_trained_pairformer_artifact` writing a `ModelArtifact` node
  (weights pointer; metrics live on the linked `EvaluationReceipt`). Nodes
  never carry tensors.
- Inference: `rank_trained_pairformer_densification_candidates` mirrors the
  deterministic path (same bounded neighborhood, direct-edge suppression,
  provenance-required candidates, quarantine pipeline); only the scorer is
  learned.

Burn gotchas encoded in the tests: `Param` initialization is lazy (random
draws happen at first forward, so seed-then-materialize per model), and the
backend RNG is process-global (seeded tests serialize behind a lock).

## Still Deferred (named, not cut)

- Default inference still runs the deterministic floors; the trained
  Pairformer becomes the scorer once a promoted artifact exists for the
  tenant (load by config + artifact pointer). The EdgeMPNN scorer and
  context scorer remain deterministic; the same training pattern (Burn
  modules + masked-recovery / use-receipt labels) is the intended path.
- Pairformer candidates remain limited to two-hop-supported pairs by
  construction (`score_links` caps unsupported pairs below threshold). The
  N-hop provenance lane is the EdgeMPNN scorer.
- Candidate promotion: quarantined `ReflexiveEdgeCandidate` nodes have no
  corroboration/promotion path into live topology yet (insertion stays
  gated off entirely).
- Steering covers the single-hop edge-pattern anchor decision; chain and
  var-length arms have one enumerated plan today.
- Known pre-existing failure unrelated to this work:
  `router::tests::mcp_search_acquisition_async_handoff_persists_pollable_harness_run`
  fails on the base commit (6b21ea60) as well; tracked separately.
