# Composed Agent Binding

Status: first implementation slice, 2026-06-02.

Source spec: `/Users/travisgilbert/Downloads/theorem-composed-agent-spec (1).md`.

## Goal

Give Theorem a first-class `AgentBinding` runtime object in the Rust harness core. The binding is the enforceable object that makes a composed agent real: identity, roster, private working memory, published scope, capability scope, budget, and trace all travel together instead of being implied by a prompt.

This slice implements the foundation only. It does not call models, persist bindings to GraphStore, expose a UI, or wrap the remaining Theseus apps. Those are later slices.

## Dependency Cut

The spec's build order starts with:

1. Extend the parity-green harness state machine with the binding lifecycle.
2. Add `AgentBinding` as a runtime object.
3. Add the `AgentHead` registry shape.
4. Add the private working-memory scratchpad scope.

This slice covers those four as pure Rust contracts in `theorem-harness-core`:

- `agent_binding::AgentBinding`
- `agent_binding::AgentHead`
- `agent_binding::BindingMemoryScope`
- `agent_binding::apply_binding_transition`

The existing `state_machine.rs` remains unchanged in behavior so its Python parity corpus stays stable. The binding lifecycle is a sibling pure machine first; a later parity slice can generate Python reference fixtures for the binding events once the canonical Theseus surface has the same contract.

## Runtime Invariants

- A binding must have a non-empty `agent_id`, `owner_id`, `agent_name`, `trust_tier`, and head roster.
- Every `active_head_set` entry must exist in the roster.
- `composition_hash` is computed from the roster plus active head set. Swapping heads changes the hash.
- Reasoning cores and specialized coders can append to the private scratchpad when active. Skill plugins cannot.
- Budget allocation is a hard guard. A run cannot allocate more than the binding's shared budget.
- Publication cannot occur until policy has passed.
- Memory patches must remain review-gated before promotion.
- Terminal binding runs reject further lifecycle transitions.

## Claude Coordination

Room id: `theorem-composed-agent-binding`.

Codex claimed the first Rust implementation slice:

- `docs/plans/composed-agent-binding/README.md`
- `rustyredcore_THG/crates/theorem-harness-core/src/agent_binding.rs`
- `rustyredcore_THG/crates/theorem-harness-core/src/lib.rs`

Claude Code was mentioned on the native coordination substrate and asked to take a non-overlapping review/planning lane: critique the contract against the spec and propose the next slices, without editing the claimed files until Codex closes the intent.

## Next Slices

1. Add Python reference fixtures for the binding lifecycle once Theseus has the matching canonical shape.
2. Persist `AgentBinding`, `AgentHead`, scratchpad revisions, and binding lifecycle events through `theorem-harness-runtime`.
3. Expose binding read/write tools through `rustyred-thg-mcp`.
4. Compile a binding charter from stance plus capability enumeration.
5. Add the intra-agent loop: primary proposal, critic contributions, synthesis, policy check, publication receipt.
6. Wire the remaining Theseus app abilities as native affordances or theorem gRPC tools.
