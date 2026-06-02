# Composed Agent (Theorem's voice)

The plan tree for building Theorem into a composed agent: a curated roster of heads coordinating through an intra-agent loop, bound by a first-class `AgentBinding` runtime object the substrate enforces, aligned by the binding rather than the heads.

Source spec: `~/Downloads/theorem-composed-agent-spec.md` (2026-05-31).

## Files

- `implementation-plan.md`: the checklist-first plan. Every spec Part (1-7) and build-order step (1-11) is backreferenced. Architecture decisions for the Rust translation are recorded there.
- `lane-split.md`: the Lane A / Lane B seam contract. Lane A (claude-code) = the Rust binding kernel; Lane B (codex) = the model-bearing + grpc layer on top.

## One-paragraph orientation

An agent is a composition of heads bound to a shared working-memory scope over the substrate. Within an agent the heads share the uncommitted scratchpad directly; between agents they share only committed graph state. Theorem is the first-party composition. The binding (identity, composition, working-memory scope, published scope, capability scope, budget, trace) is enforced by the substrate, not explained in a prompt. Alignment lives in the binding (guards Travis controls), not in the heads (training he does not). The build extends the parity-green Rust harness state machine in `theorem-harness-core` with a binding state machine, adds the binding planes, the versioned scratchpad with four memory zones, a hard budget governor, and alignment guards; then (Lane B) the head registry, the intra-agent loop, the charter compiler, and the exposure of Theseus's full ability over `theorem_grpc`.

## Status

Live build. Lane A in progress (claude-code). Lane B claimed for codex on the coordination substrate (room `repo:theorem:branch:main`). Steps 10-11 deferred by the spec.
