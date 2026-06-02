# Theorem as a Composed Agent: Implementation Plan

Source spec: `~/Downloads/theorem-composed-agent-spec.md` ("Theorem as a Composed Agent: The Build Plan", 2026-05-31).
Grounded against: `rustyredcore_THG/crates/theorem-harness-core` (parity-green harness kernel) and `theorem-harness-runtime` (GraphStore persistence seam), read at the start of this build.

This plan is checklist-first. Every spec Part (1-7) and every build-order step (1-11) has at least one checklist item backreferencing it. Spec sections the spec itself defers are listed under "Spec-authored deferrals" with the spec's own deferral language, not buried.

## Thesis (spec, lines 12-19)

An agent is a composition of execution heads bound to a shared working-memory scope over the substrate. Theorem is the first-party composition. Within one agent the heads share the uncommitted scratchpad directly; between agents they share only committed graph state. Alignment lives in the binding (a governance layer Travis controls), not in the heads (training he does not control).

## Lane split (along the spec Part 7 Rust/Python boundary)

- LANE A (claude-code, this plan's primary scope): the Rust binding kernel. Pure logic, correctness-critical, hot-path, concurrent. Lives in `theorem-harness-core` + `theorem-harness-runtime`. Build-order steps 1, 2, 4, 6, 9.
- LANE B (codex, builds on Lane A types): model-bearing + grpc seam. AgentHead live registry/transports, the intra-agent scratchpad reasoning loop, the charter compiler, exposing Theseus apps as affordances/tools over `theorem_grpc`. Build-order steps 3, 5, 7, 8.
- SEAM: Lane A's public types in `theorem-harness-core` (`AgentBinding`, `BindingScope`, `Scratchpad`, `BudgetScope`, the binding transition events + guards). Lane B compiles against those. See `lane-split.md`.

## Build status (2026-06-02 session: claude-code + codex)

Lane A (the correctness-critical Rust binding kernel, everything spec Part 7 assigns to Rust) is COMPLETE and fully tested. Built jointly: Codex produced the kernel and handed it off on the coordination substrate; claude-code reviewed it, then built persistence and completed the partial guards.

| Spec build step | Status | Owner | Commit |
|---|---|---|---|
| 1 binding state machine (16 events, guards, hashing) | done | codex | f0b1c19 |
| 2 AgentBinding + seven planes + composition_hash | done | codex | f0b1c19 |
| 3 AgentHead live registry (endpoints/credentials/transport) | NOT done (Lane B) | codex (proposed) | - |
| 4 BindingScope + versioned scratchpad + 4 memory zones | done | codex (kernel) + claude-code (revision persistence) | f0b1c19, a92276c |
| 5 intra-agent scratchpad loop (propose/critique/synthesize/publish, router) | NOT done (Lane B) | codex (proposed) | - |
| 6 budget governor as a hard guard | done | claude-code | 1b99c85 |
| 7 charter compiler (stance + capability enumeration) | NOT done (Lane B) | codex (proposed) | - |
| 8 expose Theseus ability (wrap engines as affordances) | NOT done (Lane B) | codex (proposed) | - |
| 9 alignment guards (consensus + grounding + action tiers) | done, grounding enforce-if-present | claude-code | 1b99c85 |
| Part 6 persistence (binding + events + scratchpad to GraphStore) | done | claude-code | a92276c |
| 10 compose-your-own-agent UI | deferred by spec | - | - |
| 11 self-optimizing / dynamic compositions | deferred by spec | - | - |

Named gap (per the no-lie-by-omission rule): step 9 grounding is enforced only when the publication payload carries `claims`; strict-always-grounding waits until every publication path supplies its claims (a Lane B concern, since Lane B produces the publications). Consensus and action-tier guards are unconditional. Verification: `cargo test -p theorem-harness-core -p theorem-harness-runtime` green (33 + 24 tests), `cargo clippy -- -D warnings` clean, single-agent `apply_transition` parity suites unchanged.

Remaining real-build work = Lane B (steps 3, 5, 7, 8): the model-bearing layer (head registry/transports, the intra-agent reasoning loop, the charter compiler, and exposing the Theseus apps over theorem_grpc). It builds on the Lane A types now committed. Owned by codex per the lane split; needs provider credentials and the grpc seam.

## Architecture decisions (the mini-design for the Rust translation)

These are the implementation-architecture choices made translating the spec to the existing crate. Recorded here because the spec is the design but the Rust shape is a decision.

1. The binding state machine is a SIBLING function `apply_binding_transition` in a new `binding_state_machine.rs`, NOT new match arms inside the existing `apply_transition`. Rationale: the single-agent run flow (`created -> observed -> ... -> closed`) and the composed-agent binding flow (`binding_resolved -> heads_probed -> ... -> closed`) are different lifecycles. A sibling keeps the parity-green `apply_transition` byte-for-byte untouched (CLAUDE.md byte-parity discipline) while reusing every shared primitive.
2. ONE run type. `RunState` stays the single run type; binding-specific fields are added as `#[serde(default)]` optionals (`binding`, `scratchpad`, `budget_state`, `draft_reviews`). Existing single-agent runs serialize identically (defaults absent), so the single-agent state hash is preserved. Persistence, replay, fork, and compare then work uniformly across both run kinds.
3. Shared guard helpers (`guard_violation`, `require_payload_fields`) move to a small `guards.rs` exposed `pub(crate)`, so `binding_state_machine.rs`, `budget.rs`, and `alignment.rs` reuse them instead of duplicating. `apply_transition`'s behavior is unchanged.
4. Alignment constraints are GUARDS, not charter requests (spec Part 5 point 1). The `POLICY.CHECKED` transition runs grounding, consensus, and action-tier checks and returns a `GuardViolation` that makes `PUBLISHED_TO_SUBSTRATE` unreachable on violation.
5. The budget governor is a hard runtime guard (spec Part 3 budget plane): `HEADS.CONTRIBUTE` calls `can_wake` + `charge`; a breach returns a `GuardViolation`, so no head can act past the binding's allocation.
6. GraphStore persistence (spec Part 6 storage mapping) mirrors the existing `coordination.rs` / `memory.rs` / `event_log.rs` patterns in `theorem-harness-runtime`: binding + scratchpad + binding-events become graph nodes and append-chain edges. The Memgraph/Redis/Postgres split in the spec is the deployment target; in Theorem the in-process `GraphStore` is the canonical durable layer.

As-built note (decisions 1-3 diverged from this pre-build design; 4-6 held). The kernel Codex shipped co-locates `apply_binding_transition` in `agent_binding.rs` rather than a separate `binding_state_machine.rs` (decision 1's sibling-function principle still held: `apply_transition` is untouched and parity-stable; only the file placement differs). It uses a dedicated `BindingLifecycleState` plus a standalone `AgentBinding` rather than extending `RunState` with optionals (decision 2): cleaner full separation, persisted as its own `AgentBinding` node in `binding_store.rs`. The shared `guards.rs` refactor (decision 3) did not happen; `agent_binding.rs` kept local guard helpers and the new `budget.rs` / `alignment.rs` construct `GuardViolation` directly. Decisions 4 (alignment as `POLICY.CHECKED` guards), 5 (budget hard guard at `HEADS.CONTRIBUTE`), and 6 (persistence mirrors `event_log`) shipped as planned.

---

## LANE A checklist (claude-code)

### CA-A0: shared guard helpers refactor
- [ ] CA-A0.1 Extract `guard_violation` + `require_payload_fields` + payload-field helpers into `theorem-harness-core/src/guards.rs` as `pub(crate)`; keep `state_machine.rs` using them with identical behavior. (enables steps 1,6,9) Spec: Part 3, Part 5.
- [ ] CA-A0.2 Confirm `apply_transition` parity unaffected: existing `tests/parity.rs`, `tests/toolgraph_parity.rs`, `tests/context_web_parity.rs` still green. Spec: Part 7 (Rust enforcement).

### CA-A1: binding state machine (build-order step 1; spec Part 3)
- [ ] CA-A1.1 Add `binding_state_machine.rs` with `apply_binding_transition(run, transition) -> Result<TransitionResult, HarnessError>` reusing `RunState`/`EventState`/`TransitionResult`, `state_hash`, and `guards`. Spec: Part 3 (per-run binding state machine), build-order 1.
- [ ] CA-A1.2 Implement the 16 binding events and their status flow: `BINDING.RESOLVED -> binding_resolved`, `HEADS.PROBED -> heads_probed`, `MEMORY_SCOPE.MOUNTED -> memory_scope_mounted`, `CHARTER.COMPILED -> charter_compiled`, `CAPABILITIES.SELECTED -> capabilities_selected`, `BUDGET.ALLOCATED -> budget_allocated`, `RUN.STARTED -> run_started`, `PRIVATE_WORK.OPENED -> private_work_opened`, `HEADS.CONTRIBUTE -> heads_contributing`, `DRAFTS.SYNTHESIZED -> drafts_synthesized`, `PUBLICATION.PROPOSED -> publication_proposed`, `POLICY.CHECKED -> policy_checked`, `PUBLISHED_TO_SUBSTRATE -> published_to_substrate`, `OUTCOME.RECORDED -> outcome_recorded`, `MEMORY_PATCHES.PROPOSED -> memory_patches_proposed`, `RUN.CLOSED -> closed`. Spec: Part 3 (the explicit ladder).
- [ ] CA-A1.3 `allowed_previous_statuses` table enforcing the ordering; out-of-order transitions return `GuardViolation`. Reuse `TERMINAL_STATUSES` rejection. Spec: Part 3 ("A head cannot act as part of the agent because a prompt says so; it enters through this state machine").
- [ ] CA-A1.4 Support `RUN.FAILED` / `RUN.CANCELLED` from any non-terminal binding status (robustness parity with single-agent machine).
- [ ] CA-A1.5 Unit tests: full happy-path ladder, each guard rejection, terminal-state rejection, state-hash stability. Spec: build-order 1 ("parity-tested"); see "Parity note".

### CA-A2: AgentBinding runtime object + planes (build-order step 2; spec Part 3, Part 6)
- [ ] CA-A2.1 `binding.rs`: `AgentBinding` with the seven planes. Identity: `agent_id, owner_id, agent_name, composition_hash, version, trust_tier, active_head_set`. Spec: Part 3 identity plane.
- [ ] CA-A2.2 Composition plane: `Vec<HeadRef>` where each carries `head_id, provider, model, credential_ref, transport, capabilities, cost_profile, reliability_profile, allowed_tools, trace_tier, role`. `HeadRole { ReasoningCore, SkillPlugin, SpecializedCoder }`, `HeadTransport { Api, Mcp, Local, Hosted }`. Spec: Part 1 (three kinds) + Part 3 composition plane.
- [ ] CA-A2.3 `composition_hash` = `stable_value_hash` over the sorted roster (provider/model/role/transport). Swapping/adding/removing a head changes the hash = new agent version, same named lineage. Spec: Part 3 ("composition_hash matters").
- [ ] CA-A2.4 `PublishedScope`, `CapabilityScope`, `TraceScope` as plane structs. Capability scope carries visible/callable/confirmation-gated/shared-vs-private tool ids (the plane where the deferred MCP-learning-layer plugs in). Spec: Part 3 published/capability/trace planes.
- [ ] CA-A2.5 The within/between invariant: `can_access(binding_id, zone) -> bool` enforcing HeadLocal/BindingPrivate are binding-scoped, AgentPublished/Commons are cross-binding-readable. Spec: Part 3 invariant (line 69).
- [ ] CA-A2.6 Unit tests: composition_hash changes on roster change; access invariant holds across two bindings.

### CA-A3: BindingScope + versioned scratchpad + 4 memory zones (build-order step 4; spec Part 2, Part 3)
- [ ] CA-A3.1 `scratchpad.rs`: `Scratchpad { version, entries, head_hash }`; append-only with git-like snapshots (each append bumps version + records parent hash, no destructive collision). Spec: Part 2 ("structured, versioned document ... so no collision is destructive").
- [ ] CA-A3.2 `ScratchpadEntry { entry_id, author_head, zone, kind, content, created_at, parent_version }`; `EntryKind { Task, Context, Charter, Proposal, Critique, Synthesis, Note }`. Spec: Part 2 loop (proposal/critique/synthesis appended).
- [ ] CA-A3.3 `MemoryZone { HeadLocal, BindingPrivate, AgentPublished, Commons }` (the four zones). Spec: build-order 4, Part 3.
- [ ] CA-A3.4 `BindingScope` ties a binding_id to its private scratchpad; reads/appends are zone-checked via CA-A2.5. Spec: Part 3 working-memory scope plane.
- [ ] CA-A3.5 Unit tests: append bumps version + chains parent hash; concurrent appends both retained (no lost write); zone access enforced.

### CA-A4: budget governor as a hard guard (build-order step 6; spec Part 3)
- [ ] CA-A4.1 `budget.rs`: `BudgetScope { per_run_tokens, per_head_tokens, escalation_threshold, background_allowance, max_parallel_heads }` and `BudgetState { spent_total, spent_per_head, active_heads }` (state lives on `RunState`). Spec: Part 3 budget plane.
- [ ] CA-A4.2 `BudgetGovernor`: `can_wake(state, scope, head) -> Result<(), GuardViolation>` (max_parallel + per-head cap + total cap) and `charge(state, head, tokens) -> Result<BudgetState, GuardViolation>`. Spec: Part 3 ("No head wakes unless the binding allows ... a token bonfire" otherwise).
- [ ] CA-A4.3 Wire into `apply_binding_transition`: `HEADS.CONTRIBUTE` calls `can_wake` + `charge`; breach is a transition-blocking `GuardViolation`. Spec: build-order 6 ("hard guard").
- [ ] CA-A4.4 Unit tests: contribution within budget passes; over-cap and over-parallel return GuardViolation; escalation_threshold surfaced in the violation details.

### CA-A5: alignment guards (build-order step 9; spec Part 5)
- [ ] CA-A5.1 `alignment.rs`: `ActionTier { Tier1, Tier2, Tier3 }` (reversible/audited; gated-at-commit; irreversible/external). `tier_gate(tier, payload) -> Result<(), GuardViolation>` requiring `human_authorization` for Tier3. Spec: Part 5 point 4 (autonomy inverse to irreversibility).
- [ ] CA-A5.2 `AlignmentConstraint` set + grounding gate: every published claim must carry provenance (a cited source) else `GROUNDING_MISSING`. Spec: Part 5 point 2 (grounding-and-provenance constrains to verifiable reality).
- [ ] CA-A5.3 Critic-as-alignment: `DRAFTS.SYNTHESIZED` must record that heterogeneous critics reviewed and consensus >= threshold; `check_publication` rejects with `CONSENSUS_BELOW_THRESHOLD` otherwise. Spec: Part 5 point 3 (heterogeneity catches errors; the critic step is an alignment mechanism).
- [ ] CA-A5.4 Wire all three into the `POLICY.CHECKED` transition so `PUBLISHED_TO_SUBSTRATE` is unreachable on any violation. Spec: Part 5 point 1 (constraints are GUARDS, not requests).
- [ ] CA-A5.5 Unit tests: ungrounded claim blocks publication; below-threshold consensus blocks; tier-3 without human auth blocks; all-clear publishes.

### CA-A6: runtime persistence (spec Part 6 storage mapping; Lane A tail)
- [ ] CA-A6.1 `theorem-harness-runtime`: persist `AgentBinding` + `Scratchpad` + binding-events as GraphStore nodes and append-chain edges, mirroring `event_log.rs`/`coordination.rs`/`memory.rs`. Spec: Part 6 (Memgraph canonical; THG hot state; the in-process GraphStore is Theorem's durable layer).
- [ ] CA-A6.2 Reopen-persistence test (RedCore reopen) mirroring the existing runtime tests. Spec: Part 6.
- [ ] CA-A7 (rollup) Re-export the new public types from `lib.rs`; `cargo test -p theorem-harness-core` and `-p theorem-harness-runtime` green; post the stable seam to the room for Lane B.

---

## LANE B checklist (codex; builds on Lane A types)

### CA-B1: AgentHead live registry (build-order step 3; spec Part 1, Part 6)
- [ ] CA-B1.1 Live registry resolving `HeadRef` to a callable endpoint per transport (api/mcp/local/hosted), credential reference resolution (keys are credentials for heads, never the agent), provider configs (note: some providers bake config into the key, e.g. Mistral Studio). Spec: Part 1, Part 6 line 116.
- [ ] CA-B1.2 Reasoning-core vs skill-plugin vs specialized-coder roster wiring (3-5 active cores; plugins called not joined; coders invoked for coding work). Spec: Part 1.

### CA-B2: intra-agent scratchpad loop (build-order step 5; spec Part 2)
- [ ] CA-B2.1 The loop: task drops -> scratchpad init (task + context + charter + maps) -> router selects primary -> proposal appended -> heterogeneous critics review -> if consensus < threshold, synthesis step -> publish with UseReceipts (which cores contributed, how weighted). Spec: Part 2 loop (lines 40-49).
- [ ] CA-B2.2 Router moderation (blackboard volunteering internally; turn-taking + selection; the Pairformer for intra-agent orchestration). Spec: Part 2 (lines 49-52). Reuse the pairformer A/B machinery already in `theorem-harness-core`.

### CA-B3: charter compiler (build-order step 7; spec Part 3, Part 4)
- [x] CA-B3.1 Per-binding charter compiler: stance/preferences + zero-silent-capability enumeration of every affordance/tool the binding can reach. Implemented as the pure `rustyred-thg-affordances::compile_binding_charter(_from_store)` compiler, producing deterministic payloads for `CHARTER.COMPILED` and `CAPABILITIES.SELECTED` without resolving credentials or invoking models. Spec: Part 4 ("enumerated in the charter, zero silent capability"). Feeds `CHARTER.COMPILED`.

### CA-B4: expose Theseus ability (build-order step 8; spec Part 4)
- [ ] CA-B4.1 Wrap the unwrapped reasoning engines as affordances (currently only datalog + probabilistic are wrapped; add causal, egraph, evolution, expression, optimizer, proof, simulation, solver + the top-level `apps/causal`). Spec: Part 4 (ten engines).
- [ ] CA-B4.2 Enumerate the remaining Theseus apps as affordances/tools reached natively or over `theorem_grpc`: anti_misinfo_algo, corpus_surface, federation + epistemic_federation, paper_trail, public_verbs, publisher, research, user_model, memory_tensions, observability. Spec: Part 4 (the app surface).
- [ ] CA-B4.3 Reuse the existing `rustyred-thg-affordances` registry (connector-as-substrate) so engine/app affordances are first-class graph nodes with invocation receipts. Spec: Part 4 + the deferred MCP-learning-layer hook.

---

## Spec-authored deferrals (the spec defers these itself; not silent cuts)

- Build-order step 10 (compose-your-own-agent UI; spec Part 6): the spec says "Ship the static binding first." Deferred until Lane A + B land. When built it triggers the project design-gate (it is a visual surface).
- Build-order step 11 (self-optimizing + dynamic compositions; spec Part 6 line 128): the spec says these "make the agent self-improving at the composition level. Ship the static binding first."
- MCP-as-substrate-learning-layer (the connector moat; spec lines 8, 65, 160): the spec marks it "noted as mattering and deferred to a later turn ... real; they are next." Plan stub already exists at `docs/plans/mcp-learning-layer/`; the capability-scope plane (CA-A2.4) is built to receive it.
- OpenClaw-style action layer (the runway; spec lines 8, 106, 160): spec defers ("real and next"). The action tiers (CA-A5.1) are the governance hook it lands behind.
- Latent memory / xLSTM / native hidden-state capture (spec line 158): "Do NOT start with latent memory or xLSTM (open research) ... Upgrade the memory plane ... later." The scratchpad is the buildable-now working-memory channel.

## Parity note

The binding states are net-new; there is no Theseus Python reference binding machine yet, so CA-A1.5/CA-A3.5/etc. are spec-driven Rust unit tests (the existing `tests/parity.rs` corpus pattern). If/when the binding machine is also mirrored in Theseus, a byte-parity differential gate applies (the rule from CLAUDE.md). Recorded so a future session does not mistake "unit tested" for "byte-parity gated."

## Validation

- `cd rustyredcore_THG && cargo test -p theorem-harness-core` (Lane A core).
- `cd rustyredcore_THG && cargo test -p theorem-harness-runtime` (Lane A persistence).
- `cd rustyredcore_THG && cargo test --no-run -p theorem-harness-core` for fast compile-coherence checks mid-build.
- No regression in existing parity suites (CA-A0.2).

## Reconciliation

At session end, reconcile each CA-* item to done / partial / blocked / skipped with reason, and report against the spec build order (1-9 real, 10-11 deferred by spec).
