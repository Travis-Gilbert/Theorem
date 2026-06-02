# Composed Agent: Lane Split (claude-code + codex)

Split along the spec Part 7 Rust/Python boundary. The seam is Lane A's public Rust types; Lane B compiles against them.

## Lane A (claude-code): the Rust binding kernel

Pure logic, correctness-critical, hot-path, concurrent. All in `rustyredcore_THG/crates/theorem-harness-core` and `theorem-harness-runtime`. Build-order steps 1, 2, 4, 6, 9 (+ persistence).

New modules in `theorem-harness-core/src/`:
- `guards.rs` (shared `guard_violation` + payload helpers, `pub(crate)`)
- `binding_state_machine.rs` (`apply_binding_transition`, the 16 binding events)
- `binding.rs` (`AgentBinding` + seven planes, `HeadRef`, `HeadRole`, `HeadTransport`, `composition_hash`)
- `scratchpad.rs` (`Scratchpad`, `ScratchpadEntry`, `EntryKind`, `MemoryZone`, `BindingScope`)
- `budget.rs` (`BudgetScope`, `BudgetState`, `BudgetGovernor`)
- `alignment.rs` (`ActionTier`, `AlignmentConstraint`, `check_publication`, `tier_gate`)

`RunState` gets `#[serde(default)]` optionals: `binding`, `scratchpad`, `budget_state`, `draft_reviews`. The existing `apply_transition` is untouched (single-agent byte-parity preserved).

## Lane B (codex): model-bearing + grpc layer

Builds ON Lane A types. Build-order steps 3, 5, 7, 8.

- AgentHead live registry: resolve `HeadRef` to a callable endpoint per `HeadTransport` (api/mcp/local/hosted); credential-reference resolution; provider configs. Keys are credentials for heads, never the agent.
- Intra-agent scratchpad loop: router selects primary -> proposal -> heterogeneous critics -> synthesis if consensus < threshold -> publish with UseReceipts. Router moderation reuses the pairformer A/B machinery.
- Charter compiler: stance + zero-silent-capability affordance enumeration; feeds `CHARTER.COMPILED`.
- Expose Theseus ability: wrap the unwrapped reasoning engines (causal, egraph, evolution, expression, optimizer, proof, simulation, solver + apps/causal; datalog + probabilistic already wrapped) and enumerate the apps (anti_misinfo_algo, corpus_surface, federation, paper_trail, public_verbs, publisher, research, user_model, memory_tensions, observability) as affordances reached natively or over `theorem_grpc`. Reuse `rustyred-thg-affordances`.

## The contract Lane B can rely on

- `apply_binding_transition(run, transition)` sequences the binding lifecycle; Lane B emits `TransitionInput`s for `BINDING.RESOLVED`, `HEADS.PROBED`, `CHARTER.COMPILED`, `CAPABILITIES.SELECTED`, `BUDGET.ALLOCATED`, `RUN.STARTED`, `PRIVATE_WORK.OPENED`, `HEADS.CONTRIBUTE`, `DRAFTS.SYNTHESIZED`, `PUBLICATION.PROPOSED`, `POLICY.CHECKED`, `PUBLISHED_TO_SUBSTRATE`, `OUTCOME.RECORDED`, `MEMORY_PATCHES.PROPOSED`, `RUN.CLOSED`.
- The scratchpad is read/appended through `BindingScope`; zone access is enforced by the kernel.
- Budget is charged through the governor at `HEADS.CONTRIBUTE`; over-budget is a transition error, not a soft warning.
- Alignment is enforced at `POLICY.CHECKED`; an ungrounded claim, below-threshold consensus, or an un-authorized tier-3 action blocks `PUBLISHED_TO_SUBSTRATE`.

## Coordination

Room `repo:theorem:branch:main` on the native harness substrate. claude-code holds lane `A-rust-binding-kernel`. Commit with explicit pathspec only (shared checkout with codex). claude-code will not touch the affordances or training crates; codex's Lane B work in `rustyred-thg-affordances` and the grpc app does not overlap Lane A's harness-core modules.
