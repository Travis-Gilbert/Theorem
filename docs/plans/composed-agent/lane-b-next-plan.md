# Composed Agent Lane B: Next Execution Plan

Status: LB-1 pure registry, LB-2 deterministic fake-head loop, and LB-3 fake invocation seam implemented locally; runtime/provider execution, learned/router policy, and remaining app affordances remain open.

Source plan: `implementation-plan.md`, Lane B checklist CA-B1 through CA-B4.

## Current Truth

Lane A is complete: the binding lifecycle, binding runtime object, scratchpad/memory planes, runtime persistence, budget guard, and strict publication alignment guards are in `theorem-harness-core` / `theorem-harness-runtime`.

Lane B has local slices implemented and validated:

- `rustyred-thg-affordances::compile_binding_charter(_from_store)` compiles per-binding charters for `CHARTER.COMPILED` and `CAPABILITIES.SELECTED`.
- Builtin reasoning-engine affordances are registered as graph nodes through `register_builtin_affordances`.
- `theorem-harness-core::AgentHeadRegistry` resolves active heads to fake api/mcp/local/hosted endpoints, emits `HEADS.PROBED` payloads, preserves credential references only, and rejects inactive/unknown heads before invocation.
- `theorem-harness-core::run_fake_intra_agent_loop` runs a deterministic fake-head loop over the existing binding state machine, appends proposal/critique/synthesis scratchpad revisions, records separate `HEADS.CONTRIBUTE` receipts, charges budget, and relies on strict grounding guards.
- `theorem-harness-core::head_invocation` adds the fake-first invocation seam: `HeadInvoker`, `HeadInvocationRequest`, `HeadInvocationReceipt`, and `FakeHeadInvoker`. The loop now consumes invocation receipts rather than inline fake payloads.
- Connector registration/invocation surfaces have started landing separately; do not collapse that work into the binding loop.

The remaining Lane B work is the model-bearing layer: connect resolved heads to runtime/provider adapters, add learned/router policy, and expose the remaining Theseus app abilities through native affordances or `theorem_grpc`.

## Execution Order

### LB-1: AgentHead Live Registry

Goal: turn `AgentHead` records from declarative roster entries into callable, policy-scoped endpoints without resolving secrets inside the binding object.

Local state: the pure fake-endpoint registry is implemented in `theorem-harness-core/src/agent_head_registry.rs`. It does not call providers, does not persist credentials, and is the contract LB-2 can consume. The remaining LB-1 work is the runtime/provider adapter that exchanges `credential_ref` for actual credentials outside GraphStore.

Likely files:

- `rustyredcore_THG/crates/theorem-harness-core/src/agent_binding.rs` for any minimal shared type additions.
- A new crate/module if needed for runtime-only resolution, preferably outside the pure kernel.
- `rustyredcore_THG/crates/theorem-harness-runtime/src/` if registry state must persist as graph nodes.

Acceptance criteria:

- Done locally: resolve a head by `head_id` and `HeadTransport` (`api`, `mcp`, `local`, `hosted`) to fake targets.
- Done locally: preserve `credential_ref` as a reference only in the resolved view; no credential values exist in the registry API.
- Done locally: distinguish reasoning cores, skill plugins, and specialized coders in the resolved registry view.
- Done locally: reject inactive or unknown heads before any model call.
- Done locally: unit tests cover each transport kind with fake targets and no network.
- Open: runtime/provider adapter and GraphStore node mapping for the live registry must keep credential values out of persisted nodes.

Validation:

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-core
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
cd rustyredcore_THG && cargo clippy -p theorem-harness-core --all-targets --no-deps -- -D warnings
cd rustyredcore_THG && cargo clippy -p theorem-harness-runtime --all-targets --no-deps -- -D warnings
```

### LB-2: Pure Intra-Agent Loop Scaffold

Goal: build the loop shape without provider calls first, so the binding state machine, budget guard, scratchpad revisions, critic consensus, and strict grounding contract can be tested deterministically.

Local state: implemented in `theorem-harness-core/src/intra_agent_loop.rs` as `run_fake_intra_agent_loop`. The scaffold is fake-head-only: it does not call providers, connectors, or tools. It consumes the LB-1 registry, appends scratchpad revisions, records proposal/critique/synthesis contributions, and drives the lifecycle through publication and close.

Loop events:

1. `BINDING.RESOLVED`
2. `HEADS.PROBED`
3. `MEMORY_SCOPE.MOUNTED`
4. `CHARTER.COMPILED`
5. `CAPABILITIES.SELECTED`
6. `BUDGET.ALLOCATED`
7. `RUN.STARTED`
8. `PRIVATE_WORK.OPENED`
9. `HEADS.CONTRIBUTE`
10. `DRAFTS.SYNTHESIZED`
11. `PUBLICATION.PROPOSED`
12. `POLICY.CHECKED`
13. `PUBLISHED_TO_SUBSTRATE`
14. `OUTCOME.RECORDED`
15. `RUN.CLOSED`

Acceptance criteria:

- Done locally: a fake-head loop can append proposal, critique, and synthesis revisions to the binding scratchpad.
- Done locally: `HEADS.CONTRIBUTE` charges budget through the existing guard.
- Done locally: `DRAFTS.SYNTHESIZED` records at least two distinct contributing heads for publication.
- Done locally: `POLICY.CHECKED` payload includes grounded `claims: [{ text, provenance }]`.
- Done locally: claimless or ungrounded publications fail loudly through the existing strict grounding guard.
- Open: replace fake-head receipts with runtime model invocation receipts.
- Open: add learned/router moderation on top of the deterministic scaffold.

Validation:

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-core
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
```

Coordination split:

- Claude Code can safely take this pure scaffold if it stays on fake heads and does not introduce provider credentials or transport execution.
- Codex should hold the registry/model-call seam so credential and transport assumptions stay in one lane.

### LB-3: Model Invocation Seam

Goal: add a narrow invocation trait or adapter layer that turns resolved heads into proposal/critique/synthesis receipts.

Local state: implemented in `theorem-harness-core/src/head_invocation.rs` as a fake-first core contract. `HeadInvoker` is synchronous and pure so runtime/provider adapters can implement it later without bringing network concerns into `theorem-harness-core`. `FakeHeadInvoker` rejects skill plugins, produces deterministic request ids, content-addressed receipts, and structured payloads. `run_fake_intra_agent_loop` now appends invocation receipts into the scratchpad and uses receipt hashes for contribution events.

Acceptance criteria:

- Done locally: the loop can run against fake invokers in tests.
- Open: real invokers behind an explicit runtime adapter.
- Done locally: fake outputs are converted into structured scratchpad revisions and contribution receipts.
- Done locally: publication proposals carry grounded claims; ungrounded fake output fails through the existing strict grounding guard.
- Done locally: skill-plugin heads are rejected by the invocation seam rather than joined as reasoning heads.
- Open: tool/plugin calls through registered affordances for non-reasoning-head capability use.

Validation:

```bash
cd rustyredcore_THG && cargo test -p theorem-harness-core
cd rustyredcore_THG && cargo test -p theorem-harness-runtime
cd rustyredcore_THG && cargo test -p rustyred-thg-affordances
```

### LB-4: Remaining Theseus App Affordances

Goal: expose the remaining Theseus app surface as graph-visible affordances reachable by charter and invocation receipts.

Open app/tool families:

- `anti_misinfo_algo`
- `corpus_surface`
- `federation` / `epistemic_federation`
- `paper_trail`
- `public_verbs`
- `publisher`
- `research`
- `user_model`
- `memory_tensions`
- `observability`

Acceptance criteria:

- Each app ability has an `Affordance` node with tenant, server/tool id, family, permissions, writeback policy, and cost metadata.
- Invocation results can be recorded through `record_invocation`.
- The charter compiler can include or exclude these affordances by scope.
- `theorem_grpc` wrappers are explicit about transport, timeout, and failure receipt shape.

Validation:

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-affordances
cd rustyredcore_THG && cargo clippy -p rustyred-thg-affordances --all-targets --no-deps -- -D warnings
```

## Non-Goals For This Lane

- Compose-your-own-agent UI: deferred by the spec until the static binding works.
- Dynamic/self-optimizing compositions: deferred by the spec.
- Full harness/plugin transfer: later operational lane, after the local Rust surfaces are truthful.
- Latent memory/xLSTM/native hidden-state capture: research lane, not the static binding.

## First Implementation Slice

Current implementation slice landed the runtime/provider adapter behind the `HeadInvoker` contract: `ProviderHeadInvoker` keeps real provider calls explicit and outside `theorem-harness-core`, resolves credential references only at call time, and keeps credential values out of GraphStore-backed binding state. Next slices are learned/router policy, charter-surface reintegration, or LB-4 app affordance wrapping.
