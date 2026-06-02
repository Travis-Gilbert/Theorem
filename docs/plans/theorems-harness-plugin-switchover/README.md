# Theorems Harness Plugin Switchover (Theseus RustyRed -> Theorem RustyRed)

Topic index. Two co-authored artifacts, one project at two altitudes.

Date opened: 2026-06-02
Coordination room: `theorem-rustyred-plugin-switchover` (native Theorem RustyRed)
Agents: Codex (migration mechanics) + Claude Code (destination architecture)
Grant: Travis gave both agents creative freedom to improve harness/SDK/plugin.

## Files

| File | Author | Role |
|---|---|---|
| [`implementation-plan.md`](./implementation-plan.md) | Codex | Migration mechanics. THPS-001..010: route the plugin/SDK/hooks off the Theseus/Index-API RustyRed path onto native Theorem RustyRed, preserving public verbs. Verb routing matrix, target architecture, per-step acceptance/validation/risk, cross-agent ownership. |
| [`sdk-v2-architecture.md`](./sdk-v2-architecture.md) | Claude Code | Destination architecture. Folds the `theorem-harness SDK v2` spec onto Codex's steps: Rust core + generated bindings, runs-as-event-streams, sessions-as-scopes, affordances/skills/runs, idempotency/cancellation/resumable/receipts/export. Grounded substrate-vs-surface inventory, the THPS-002 sunset-shim fork resolution, deltas to each THPS step, new steps THPS-011/012/013, the revised phase graph, and the spec coverage map. |

## The shape in one paragraph

The switchover is the migration-mechanics layer of SDK v2, not a separate project:
both need native Theorem RustyRed as the runtime substrate first. The substrate is
already ~70% of the SDK v2 destination (AgentBinding sessions-as-scopes, `event_log`
monotonic seq for resumable streaming, `RUN.CANCELLED`, idempotency-key field,
receipts, affordances+charter all exist in `theorem-harness-core`/`-runtime`). What
is new is **surface shape** (event-streams, session handles, trace export) and
**boundary enforcement** (idempotency short-circuit, cancel handle, resume-from-seq),
plus the **generated bindings**. Codex's hand-written JS SDK clients (THPS-002) ship
the switchover now but are framed as a sunset compatibility shim, replaced by the
NAPI-RS-generated Node binding once the core SDK surface freezes (Phase 2 -> Phase 3).

## Phase graph (from `sdk-v2-architecture.md`)

- Phase 0: inventory + tool-schema fix (THPS-001; schema `anyOf` bug already fixed in `rustyred-thg-mcp`).
- Phase 1: migration mechanics, native today (THPS-002 as shim, 003 memory + idempotency, 004 coordination, 005 run lifecycle + cancel/resume/receipt-on-event, 006 manifests, 007 connectors + skills).
- Phase 2: SDK surface freeze on the core (THPS-011: run handle, session handle, streams, idempotency, resume, export shape).
- Phase 3: binding generation, gated on Phase 2 (THPS-012 Node first then Swift/Python/wasm; THPS-013 trace export).
- Phase 4: deploy, auth, docs, rollback truth gates (THPS-008/009/010).

## Open decisions for Travis

See `sdk-v2-architecture.md` final section: (1) `recall` = native memory recall vs
saved-context preview; (2) default stream view (text vs typed); (3) first binding
target (proposed Node first). Everything else is agent-owned under the creative-freedom
grant and recorded as joint decisions in the room.

## Working agreement

- Source of truth for cross-agent handoff: these two docs + native coordination
  records in room `theorem-rustyred-plugin-switchover`.
- Path-scoped commits only (`git commit -- <paths>`), never a bare `git commit`.
- Claim a file via `coordination_intent` before overlapping edits. Shared seams (the
  event-stream contract and the draft core SDK API) require an intent claim.
