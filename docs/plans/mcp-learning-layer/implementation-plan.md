# MCP Learning Layer: The Connector Layer as a Substrate Learning Registry

**Status:** implementation plan, 2026-06-02. Source spec: [`spec.md`](spec.md)
(captured from `~/Downloads/mcp-learning-layer-spec.md`). Built in the Theorem
repo (Rust-native substrate spine).

**One line:** do not build a passthrough MCP-of-MCPs. Make every connector tool a
first-class `Affordance` node in the RustyRed graph, learn which to reach for from
accumulated outcomes, and scope the relevant subset per agent. This is the
capability-scope plane of the AgentBinding.

## Architecture decision

The spec asserts "no new infrastructure; it composes what 0.6.0 already has." That
holds, and the codebase makes it concrete through **two existing precedents**:

1. `rustyred-thg-adapters` (the LoRA adapter catalog) is the **structural template**:
   a typed struct (`LoraAdapter`) that serializes to a `NodeRecord` with a label, a
   vector designation, validate/normalize, a tenant-scoped node-id scheme, edge
   constants (`TRAINED_ON`/`DERIVED_FROM`/`FITNESS_SIGNAL`), PPR+embedding selection
   (`routing.rs`), an outcome->fitness EWMA loop (`fitness.rs`), and a durable
   training export + writeback (`training_substrate.rs`). The affordance learning
   registry is a structural sibling of this catalog.

2. `theorem-harness-core` already owns the **affordance vocabulary and the pairformer
   validation gate**:
   - `affordances.rs`: `AffordanceContract` + content-addressed `AffordanceReceipt` +
     `default_affordance_registry()` (11 built-in symbolic-engine contracts). This is a
     *static Vec*, not a graph or a dynamic connector registry.
   - `session_metrics.rs`: `PAIRFORMER_MODES = ["off","gate","full"]` + `compare_modes`
     (Welch z-test, 90%-confidence promotion bar at n>=50, z>=1.645, reduction>0). This
     is exactly the spec's "validate on held-out, do not assume more invocations help".
   - `toolgraph.rs`: `select_tools` / `compile_task_toolkit` (permission-keyed toolkit
     selection) is the *static ancestor* of capability-scope.

**Decision: a new crate `rustyredcore_THG/crates/rustyred-thg-affordances`**, depending
on `rustyred-thg-core` (graph store, PPR, `NodeRecord`/`EdgeRecord`, `VectorDesignation`,
`stable_hash`, `now_ms`) and `theorem-harness-core` (reuse `AffordanceContract` +
`AffordanceReceipt` so the affordance vocabulary and receipt hashing are not forked).
This is the exact dependency shape of `theorem-harness-runtime`, which is itself the
precedent for "persist harness-core receipts as graph nodes/edges."

Why a new crate, not a module in `rustyred-thg-adapters`:
- `rustyred-thg-adapters` has uncommitted, actively-edited work (`training_substrate.rs`,
  `fitness.rs`, `upsert.rs`, `lib.rs`) from Codex as of 2026-06-02. The harness
  coordination substrate is down (HTTP 500), so structural isolation is the only
  collision-avoidance available. A new crate touches exactly one shared line (the
  workspace `members` list), committed with an explicit pathspec.
- Affordances (connector tools) are conceptually adjacent to but distinct from LoRA
  adapters; a sibling crate keeps each catalog coherent.

**No new storage primitive.** Everything rides the existing `GraphStore` trait (a local
`AffordanceGraphStore` mirrors `AdapterGraphStore`, impl'd for `InMemoryGraphStore` +
`RedCoreGraphStore`). The affordance registry is a labeled region of the existing tenant
graph. Mirror-rule note: `AffordanceGraphStore` duplicates `AdapterGraphStore`; a future
shared `CatalogGraphStore` trait is a promotion candidate, deferred to avoid coupling to
the hot adapters crate.

## Spec -> checklist traceability

Every spec section maps to at least one requirement. No section is unaddressed.

| Spec section | Requirement | Where |
|---|---|---|
| "Affordances are nodes" (+ build step 1) | **R1** Affordance node shape + `SERVED_TASK`/`PRODUCED_OUTCOME`/`SEQUENCED_WITH` edges, tenant-scoped, validate/normalize, `from_contract` bridge | `types.rs` |
| "Pairformer learns to select" + "Selection is proactive" | **R2** Proactive learned selection: PPR over outcome edges + embedding + fitness prior, ranked candidates, works with zero training (warm graph prior) | `selection.rs` |
| "How this rides existing machinery" (registry is inverse of MCP adapter) + build step 2 | **R3** Connector registration path: walk tool catalog, upsert one `Affordance` per tool, idempotent on re-registration, caller-supplied embedding | `registry.rs` |
| Receipt pattern extends to affordance calls + build step 3 | **R3b** Invocation receipts: candidate set, selection, graph version, outcome -> `InvocationReceipt` node + edges + fitness update (reuse `AffordanceReceipt` hashing) | `outcomes.rs` |
| "The charter enumerates the relevant subset" + build step 5 | **R4** `CapabilityScope` (charter): enumerate allowed affordance subset; selection filters to scope then ranks within | `types.rs` + `selection.rs` |
| "Pairformer learns to select" (training) + build step 4 + "lab-graph/reasoning-trace corpus" | **R5** Affordance-outcome training export (`ranking_pairs`) + Pairformer model writeback + connect to `compare_modes` held-out validation gate | `training.rs` |
| "Why passthrough is wrong" / "Why it matters for the runway" (compounding) | **C3** Tests prove compounding: a recorded positive outcome changes selection order | `tests/selection_test.rs` |
| "What this is not": forwarding is the fallback for an unprimed affordance | **C5** Selection returns scoped candidates even with no outcome edges (freshly connected tools reachable) | `selection.rs` + test |
| "What this is not": never 1000 tools in front of the model | **C4-scope** Scope is always applied before ranking | `selection.rs` + test |
| receipt = audit trail + training data, content-addressed, records graph version | **C6** Receipt content-addressed (reuse harness-core), records `graph_version` | `outcomes.rs` + test |
| "It is not new infrastructure" | **C1** New crate over existing `GraphStore`; durable on `RedCoreGraphStore` aof_always | crate + `tests/training_test.rs` |
| idempotent on re-registration | **C2** Re-registration preserves fitness/embedding/outcomes, same node ids | `registry.rs` + test |

## Module layout (mirrors `rustyred-thg-adapters`)

```
crates/rustyred-thg-affordances/
  Cargo.toml                      deps: rustyred-thg-core, theorem-harness-core, serde, serde_json
  src/lib.rs                      module decls + re-exports + #[cfg(test)] test decls
  src/types.rs                    Affordance struct, edge/label constants, AffordanceGraphStore
                                  trait (InMemory + RedCore impls), CapabilityScope,
                                  request/result structs, node-id + provenance helpers,
                                  Affordance::from_contract bridge
  src/registry.rs                 register_connector / upsert_affordance /
                                  register_builtin_affordances (project the 11 contracts)
  src/selection.rs                select_affordances (PPR + embedding + fitness, scope-filtered),
                                  select_affordances_by_embedding, rank_candidate_scores
  src/outcomes.rs                 record_invocation (InvocationReceipt node + SERVED_TASK +
                                  PRODUCED_OUTCOME + SEQUENCED_WITH + fitness EWMA),
                                  effective_affordance_fitness, affordance_nodes
  src/training.rs                 export_affordance_training_view (ranking_pairs, deterministic,
                                  graph_version + snapshot_hash), register_pairformer_artifact
                                  (ModelArtifact + EvaluationReceipt writeback),
                                  pairformer_validation_gate (wraps compare_modes)
  src/tests/registry_test.rs
  src/tests/selection_test.rs
  src/tests/outcomes_test.rs
  src/tests/training_test.rs
```

### Node + edge taxonomy

Labels: `Affordance`, `Connector` (owning MCP server), `TaskType`, `InvocationReceipt`.
Edges:
- `OFFERS` (Connector -> Affordance): ownership.
- `SERVED_TASK` (Affordance -> TaskType): this affordance served this task shape.
- `PRODUCED_OUTCOME` (Affordance -> InvocationReceipt): outcome-weighted result edge.
- `SEQUENCED_WITH` (Affordance -> Affordance): commonly sequenced together in a session.
- `FITNESS_SIGNAL` reused conceptually via the fitness property + outcome edges.

Node-id scheme (tenant-scoped, mirrors adapters):
- `affordance:{tenant}:{server}:{tool}`
- `connector:{tenant}:{server}`
- `task_type:{tenant}:{task}`
- `invocation_receipt:{tenant}:{receipt_hash}`

### Affordance struct (the node)

`affordance_id, tenant_id, server_id, tool_name, label, description, input_schema (Value),
permissions (Vec<String>), cost (Value), writeback_policy (String), tags (Vec<String>),
embedding (Option<Vec<f32>>), fitness (f32), version (u32), created_at_ms (i64),
manifest_version (u32)`.

`Affordance::from_contract(&AffordanceContract, tenant)` projects each built-in
symbolic-engine contract into a node so the existing 11 affordances are first-class graph
nodes too (satisfies "Affordances are nodes" for the existing registry, not only for new
connectors).

### Selection (cold-start Pairformer)

`select_affordances(store, req)`:
1. Resolve `CapabilityScope` -> candidate affordance set (charter enforcement).
2. Score each candidate:
   - structural: PPR seeded from the `TaskType` node over `SERVED_TASK` + `PRODUCED_OUTCOME`
     edges (the learned prior: "this affordance has worked for this task shape").
   - semantic (optional): cosine of the task-description embedding against affordance
     embeddings.
   - fitness multiplier (time-decayed EWMA), mirroring `effective_fitness_from_node`.
3. Combine (`score * fitness`), sort, truncate to `k`.
4. Forwarding fallback: an affordance with no outcome edges still appears (scoped) with a
   base score, so freshly connected tools are reachable. This is the spec's "forwarding is
   the fallback for an affordance that has no learned prior yet."

The full learned Pairformer head is an external training job (see R5); this PPR+fitness
selector is the warm-start that works on day one with zero training. Same pattern as the
SceneDirector GNN-with-rule-fallback elsewhere in the project.

## Build phases (dependency order, matches the spec's "build, in dependency order")

### Phase 1 — R1: node shape + edges (`types.rs`)
Acceptance:
- `Affordance` round-trips `to_node_record` <-> `from_node_record`.
- `validate()` rejects empty id/server/tool and bad manifest_version; `normalized()`
  trims, clamps fitness, defaults timestamps.
- `AffordanceGraphStore` impl'd for `InMemoryGraphStore` and `RedCoreGraphStore`.
- `Affordance::from_contract` produces a valid node from each of the 11 built-in contracts.
- `CapabilityScope` filters an affordance set by ids/servers/families/tags.

### Phase 2 — R3 + C2: connector registration (`registry.rs`)
Acceptance:
- `register_connector(manifest)` with N tools -> 1 `Connector` node + N `Affordance` nodes
  + N `OFFERS` edges, one transaction.
- Re-registration is idempotent: same node ids, updated metadata, preserved fitness +
  embedding + outcome history.
- `register_builtin_affordances` writes 11 affordance nodes from the harness-core registry.
- Tenant scoping enforced on every node id + property.

### Phase 3 — R3b + C6: invocation receipts (`outcomes.rs`)
Acceptance:
- `record_invocation` writes an `InvocationReceipt` node (carrying candidates considered,
  selected affordance, `graph_version`, outcome label + score, task_type), a `SERVED_TASK`
  edge, a `PRODUCED_OUTCOME` edge (outcome-weighted), and `SEQUENCED_WITH` between the
  selected affordance and the prior selection in the session.
- The receipt reuses `theorem_harness_core::AffordanceReceipt` content-addressing (same
  inputs -> same `receipt_hash`).
- Fitness updates by EWMA (`alpha = weight/(weight+4)`), time-decayed on read.

### Phase 4 — R2 + C3 + C5 + scope: selection (`selection.rs`)
Acceptance:
- With no outcomes, `select_affordances` returns scoped candidates (forwarding fallback).
- After a positive outcome for affordance A on task T, A ranks above B for T (compounding —
  the core proof that this is not a passthrough).
- Out-of-scope affordances never appear in results.
- `select_affordances_by_embedding` ranks by cosine within scope.

### Phase 5 — R5: training export + writeback (`training.rs`)
Acceptance:
- `export_affordance_training_view(snapshot)` emits deterministic `ranking_pairs` (task,
  candidates, selected, outcome) with `graph_version` + `snapshot_hash`; rejects empty.
- `register_pairformer_artifact` writes back a `ModelArtifact` (model_type
  `pairformer`/`tool_router`) + `EvaluationReceipt` + `TRAINED_ON`/`EVALUATED_BY`/
  `PROMOTED_TO_ACTIVE` edges; refuses `PROMOTED_TO_ACTIVE` without an evaluation receipt.
- `pairformer_validation_gate` wraps `theorem_harness_core::compare_modes` and applies the
  90%-confidence bar before promotion.

### Phase 6 — C1: durability + wiring
Acceptance:
- `tests/training_test.rs`: open `RedCoreGraphStore` with `aof_always`, register a connector,
  record invocations, `snapshot_now()`, reopen from disk -> affordances + outcome edges
  survive; export is deterministic across reopen.
- Append `crates/rustyred-thg-affordances` to `rustyredcore_THG/Cargo.toml` `[workspace].members`.
- `cargo test -p rustyred-thg-affordances` green.
- Update `CLAUDE.md` + `AGENTS.md` crate table with the new crate.

## Named gaps / seams (surfaced, not buried)

- **External Pairformer trainer.** The trained selection head is a Python/Modal job, not
  Rust. This crate ships the affordance-outcome export contract, the writeback contract,
  and the validation-gate wiring (R5). The trainer that consumes `ranking_pairs` and emits
  a model artifact lives outside the graph engine, exactly as the durable-training-substrate
  plan draws the Rust/Python boundary (`Trainer orchestration outside RustyRed, on
  Modal/RunPod`). The cold-start PPR+fitness selector (R2) is fully functional now; the
  learned head is the enrichment. **This is not a silent scope cut**; it is the established
  Rust/Python boundary.
- **Text embedder.** RustyRed core has vector storage + HNSW (`VectorDesignation`,
  `VectorIndex`) but no text->vector embedder. Affordance description embeddings are
  caller-supplied (the Python MCP layer or a future Rust embedder provides them). With no
  embedding, selection degrades gracefully to structural PPR + fitness.
- **Live MCP `tools/list` ingestion.** `register_connector` takes a `ConnectorManifest`
  (the normalized `{name, description, input_schema}` shape the MCP crate already emits via
  `tool_definitions`). Wiring a live MCP server's `tools/list` into a manifest is a thin
  integration in `rustyred-thg-mcp` or the Python MCP layer; this crate is the registry
  core with the manifest as the contract boundary.
- **Unification candidates.** `AffordanceGraphStore` vs `AdapterGraphStore` (shared
  `CatalogGraphStore`); `register_pairformer_artifact` vs training_substrate
  `register_model_artifact` (shared model-artifact catalog); `select_affordances` vs
  `toolgraph::select_tools` (capability-scope replacing the static permission filter). All
  three are promotion candidates, deferred to keep this slice decoupled from hot files.

## Coordination note (harness substrate down)

Harness `coordinate`/`presence`/`mentions` returned HTTP 500 this session. Coordination is
via git: a new crate (zero collision with Codex's hot `rustyred-thg-adapters` work), the
one shared-file edit (workspace `members`) committed with an explicit pathspec, and this
plan + the commit message as the claim. Do not `git add -A`; stage only
`crates/rustyred-thg-affordances/`, `rustyredcore_THG/Cargo.toml`, the plan, and the doc
updates.
