# Theorems Harness Plugin Switchover (Theseus RustyRed -> Theorem RustyRed)

Topic index. Four co-authored artifacts: parent architecture -> migration lane ->
grounded companion -> this index.

Date opened: 2026-06-02
Coordination room: `theorem-rustyred-plugin-switchover` (native Theorem RustyRed)
Agents: Codex (migration mechanics) + Claude Code (destination architecture)
Grant: Travis gave both agents creative freedom to improve harness/SDK/plugin.

## Files

| File | Author | Role |
|---|---|---|
| [`sdk-v2-architecture-overlay.md`](./sdk-v2-architecture-overlay.md) | Codex | **Parent architecture (canonical).** What SDK v2 is: the Rust-core-with-generated-bindings layer model, the SDK primitive list, the ordering rule (stabilize Rust core, then runtime over GraphStore, then route adapters, then generate bindings), product gates, and open design questions. The switchover is the first runtime-substrate slice of this. |
| [`implementation-plan.md`](./implementation-plan.md) | Codex | **Migration lane.** THPS-000..010: THPS-000 aligns to the SDK v2 architecture; THPS-001..010 route the plugin/SDK/hooks off Theseus RustyRed onto native Theorem RustyRed, preserving public verbs. Verb routing matrix, per-step acceptance/validation/risk, cross-agent ownership. |
| [`sdk-v2-architecture.md`](./sdk-v2-architecture.md) | Claude Code | **Grounded companion + SDK tail.** Does not restate the architecture; grounds and sequences it. The substrate-vs-surface inventory (file:line evidence: destination ~70% built today), per-THPS-step deltas, the THPS-002 sunset-shim resolution, the SDK tail steps THPS-011/012/013 (core surface freeze, binding generation, trace export), the phase graph, the spec coverage map, and the Travis-ratified resolved decisions. |
| [`tool-contract-matrix.md`](./tool-contract-matrix.md) | Codex | **THPS-001 route matrix.** Maps every plugin tool to native MCP, SDK surface, product HTTP, explicit Theseus fallback, or rename/fold. Captures route classes, compatibility rules, native-only tools worth exposing, first implementation order, validation targets, and open route questions. |
| [`README.md`](./README.md) | Claude Code + Codex | This index. |

## The shape in one paragraph

The switchover is the migration-mechanics layer of SDK v2, not a separate project:
both need native Theorem RustyRed as the runtime substrate first. The substrate is
already ~70% of the SDK v2 destination (AgentBinding sessions-as-scopes, `event_log`
monotonic seq for resumable streaming, `RUN.CANCELLED`, idempotency-key persistence,
receipts, affordances+charter all exist in `theorem-harness-core`/`-runtime`). What
is new is **surface shape** (event-streams, session handles, trace export) and
**boundary enforcement** (idempotency short-circuit, cancel handle, resume-from-seq),
plus the **generated bindings**. The old hand-written JS client plan is now only a
sunset compatibility shim: `apps/theorem-harness-node` is the concrete NAPI-RS
binding target for the plugin's Rust SDK route.

## Current checkpoint

- THPS-011 Rust SDK surface landed in `rustyredcore_THG/crates/theorem-harness`
  (`RunHandle`, `RunStream`, `Session`, idempotency tokens, trace export).
- THPS-012 Node binding landed in `apps/theorem-harness-node`: durable RedCore
  store, run start/cancel/status/events/text stream, plus `remember`/`recall`.
- Swift/UniFFI binding landed in `apps/theorem-harness-swift`: a second
  generated binding now drives the same Rust SDK surface, with smoke coverage
  for run lifecycle and memory.
- THPS-003 idempotency persistence is validated in the current shared worktree:
  `TransitionInput.idempotency_key` is stored on `EventState`, graph event nodes,
  and event-of edges; replay preserves it; camelCase `idempotencyKey` input is
  accepted at the core JSON boundary.
- Plugin source now contains `sdk/route-policy.mjs` and real handler wiring for
  `harness_replay`, `remember`, `recall`, and `self_note` through
  `HarnessRoutePolicy`.
  Private local run/memory calls can execute through the Node binding when
  `THEOREM_HARNESS_DATA_DIR` and `THEOREM_HARNESS_NODE_BINDING_PATH` are set;
  shared memory/product compatibility remains explicit.
- Disk headroom was restored during the binding lane, so heavy build work is no
  longer blocked by the earlier 1.3 GiB free-space condition.

## Phase graph (from `sdk-v2-architecture.md`)

- Phase 0: inventory + tool-schema fix (THPS-001; schema `anyOf` bug already fixed in `rustyred-thg-mcp`).
- Phase 1: migration mechanics, native today (THPS-002 route policy, 003 memory + idempotency, 004 coordination, 005 run lifecycle + cancel/resume/receipt-on-event, 006 manifests, 007 connectors + skills).
- Phase 2: SDK surface freeze on the core (THPS-011 landed: run handle, session handle, streams, idempotency, resume, export shape).
- Phase 3: binding generation, gated on Phase 2 (THPS-012 Node landed through durable memory/run slices; Swift/UniFFI landed; Python/wasm later; THPS-013 trace export).
- Phase 4: deploy, auth, docs, rollback truth gates (THPS-008/009/010).

## Resolved decisions

See `sdk-v2-architecture.md` final section for the Travis-ratified decisions:
`recall` becomes native memory recall, the text stream is the default view with
typed events always available, and Rust is the native SDK while generated
bindings start with Node/NAPI-RS. The public Rust SDK type boundary is now
answered by the shipped `theorem-harness` crate. The remaining architecture seam
is the browser WASM remote stream shape plus packaging details for generated
bindings.

## Working agreement

- Source of truth for cross-agent handoff: these docs + native coordination
  records in room `theorem-rustyred-plugin-switchover`.
- Path-scoped commits only (`git commit -- <paths>`), never a bare `git commit`.
- Claim a file via `coordination_intent` before overlapping edits. Shared seams (the
  event-stream contract and the draft core SDK API) require an intent claim.
