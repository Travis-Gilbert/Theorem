# theorem-harness SDK v2: Destination Architecture + Switchover Reconciliation

Date: 2026-06-02
Author: Claude Code (co-plan with Codex)
Sibling of: `implementation-plan.md` (Codex, the migration-mechanics spine)
Grounds: `/Users/travisgilbert/Downloads/theorem-harness-sdk-v2-spec.md` (Travis, 2026-06-02)
Coordination room: `theorem-rustyred-plugin-switchover` (native Theorem RustyRed)

## Thesis (read this first)

Two artifacts now describe one project at two altitudes:

- Codex's `implementation-plan.md` (THPS-001..010) is the **migration mechanics**:
  route the plugin, SDK clients, and host hooks off the Theseus/Index-API RustyRed
  path and onto the native Theorem RustyRed substrate, preserving the public verbs.
- The SDK v2 spec is the **destination architecture**: one Rust core
  (`theorem-harness-core` + `theorem-harness-runtime`) as the single source of
  truth, wrapped by **generated** bindings (Python/Swift via UniFFI, Node via
  NAPI-RS, browser via wasm-bindgen), exposing runs-as-event-streams,
  sessions-as-scopes, and affordances/skills/runs as first-class objects, with
  idempotency, cancellation, resumable streaming, receipts, and trace export
  designed in.

The switchover is not a separate project that precedes SDK v2. **The switchover is
the migration-mechanics layer of SDK v2.** Both need the exact same thing first:
native Theorem RustyRed as the runtime substrate the plugin talks to. Codex's plan
ships that. This document folds the SDK v2 destination on top so the switchover
**converges toward** the generated-bindings architecture instead of building a
parallel surface that v2 would later have to delete.

## Grounded finding: the destination is mostly substrate-complete already

The SDK v2 spec says "the binding state machine, the session/scope surface, the
affordance and skill objects, and the cancellation and resumable-streaming
primitives are not all in the core yet." That is true for the **SDK surface
shape**, but understates how much **substrate** already exists. Verified against
source this session:

| SDK v2 primitive | Status in core/runtime today | Evidence |
|---|---|---|
| Run state machine + canonical events | Built, parity-green | `theorem-harness-core/src/state_machine.rs` (54KB); `RUN.CREATED/CLOSED/FAILED/CANCELLED/REPLAYED/FORKED` |
| Sessions-as-scopes (within vs across) | Built | `agent_binding.rs`: `BindingMemoryScope` (`bindingscope:{agent}`), `ScratchpadDocument`+`ScratchpadRevision` (versioned working memory), `PublishedScope` (`published:{agent}`, committed-across-sessions), `BindingCapabilityScope` (`capability:{agent}`, charter scope) |
| Session persistence | Built | `theorem-harness-runtime/src/binding_store.rs`: `persist_binding`, `load_binding`, `load_scratchpad_revisions`, idempotent upsert + contiguous-append |
| Monotonic event seq (resumable-streaming substrate) | Built | `theorem-harness-runtime/src/event_log.rs`: per-run `seq: u64`, `expected_previous_seq`, gap/conflict detection, `event-next` append-chain edges |
| Cancellation transition | Built | `state_machine.rs` `RUN.CANCELLED` requires `reason`, `cancelled_by` |
| Replay / fork | Built | `state_machine.rs` `RUN.REPLAYED`/`RUN.FORKED`; `replay.rs` |
| Idempotency **key field** | Built (field only) | `theorem-harness-core/src/types.rs:205` `pub idempotency_key: String`; `binding_store.rs` "idempotent upsert" discipline |
| Receipts | Built | 9 core/runtime files carry `Receipt`/`receipt` |
| Affordances + charter scope | Built | `affordances.rs`; `rustyred-thg-affordances/src/charter.rs` (Codex, this session) |
| Toolkit selection | Built | `toolgraph.rs` |
| Agent heads (composition) | Built | `agent_head_registry.rs`, `intra_agent_loop.rs` |

So the destination is roughly 70% substrate-complete. What is genuinely missing is
**surface shape and boundary enforcement**, plus the **generated bindings**:

| Genuinely new work | Why it is new | Spec section |
|---|---|---|
| Runs-as-event-streams SDK surface | `Stream` appears in 1 file; MCP is RPC-shaped. The streamed-event iterator is a binding-layer surface over the existing transition log. | "Runs that stream typed events" |
| Two stream views (raw typed + text-only) | Neither exists as a consumer surface. | same |
| Sessions-as-handles ergonomic surface | `BindingMemoryScope` exists; an `open_session(scope)` handle that runs share does not. | "Sessions for continuity" |
| Idempotency **enforcement** at the boundary | The key field exists; nothing short-circuits a retry with a seen key to the prior result. The plugin does not thread a token at all. | "Idempotency tokens" |
| Cancel **handle / polled flag** channel | The `RUN.CANCELLED` transition exists; a live cancel signal a run loop polls at transition boundaries does not. | "Cancellation as an explicit channel" |
| Resume-**from-seq** stream surface | The `seq` substrate exists; "reconnect and pass last seq" does not. | "Resumable streaming" |
| Receipts surfaced **on events** | Receipts exist on operations; they are not attached to streamed events for the consumer. | "Receipts and provenance on every event" |
| `harness_export` (trace export) | `Export` appears in 0 files. New verb: run/session events to SFT or preference pairs. | "Trace export as a headline" |
| Skills as a first-class SDK object | Affordances exist; the encoding-pipeline "skill" object is not yet an SDK surface. | "The three units" / "Skills as the encoding-pipeline's SDK surface" |
| Generated bindings (NAPI / UniFFI / wasm) | The whole "cannot drift" bet. None generated yet. | "The layered structure" / "Sequencing" |

This reframing matters because it tells us where to spend effort: not rebuilding
substrate, but exposing the surface the spec converged on and generating the
bindings once the surface is stable.

## The one architectural fork to resolve

Codex's **THPS-002** ("SDK client boundary") adds hand-written JS clients in the
plugin: `TheoremHarnessMcpClient`, `TheoremHarnessHttpClient`, `TheseusEngineClient`,
under `codex-plugins/theorems-harness/sdk/`. That is the right move for shipping the
switchover **now**. But the SDK v2 spec is explicit that the Node surface should be
**NAPI-RS-generated from the Rust core** so it cannot drift. A hand-written JS SDK is
exactly the parity surface v2 exists to delete: every method becomes a thing that can
disagree with the core.

This is not "Codex is wrong." It is a **sequencing** decision, and the spec already
states the rule: "land the core API first, stabilize it, then generate the bindings
once," precisely so generation is done against a stable surface rather than churned
against a moving one. The current `server.mjs` (2218 lines, defaults
`THEOREM_CONTEXT_BASE_URL` to `index-api-production.../api/v2/theseus`, `THG_BASE_URL`
to `thg-product-production`) cannot be replaced by a generated binding today because
the SDK surface (streams, session handles, export) is not in the core yet.

### Reconciliation (proposed joint decision)

1. **Keep the THPS-002 JS route policy, but frame it as an explicitly-sunset
   compatibility shim.** It ships the switchover, it carries the route receipts, it
   is the host adapter. It is labeled in code and docs as transitional, with the
   named successor being the generated NAPI binding.
2. **Add a parallel track (Phase 3 below) that stabilizes the core SDK surface and
   generates the Node binding,** which then *replaces* the hand-written JS client.
   The plugin `server.mjs` becomes a thin idiomatic shell over the generated
   `theorem-harness` Node module (the spec's "thin hand-written idiomatic shell over
   the generated layer").
3. **The JS client's method shape mirrors the intended core API now,** so the later
   swap to the generated binding is a substitution, not a redesign. Same method
   names, same argument shapes, same receipt fields.

Net: the switchover routes to native RustyRed today; the SDK boundary converges to a
single generated source of truth instead of forking into a permanent JS surface.

## Deltas to Codex's THPS steps (the co-authoring)

These are additive edits to `implementation-plan.md`, not a replacement. Codex owns
that file; this section is the proposed diff for Codex to fold in or for us to apply
jointly.

- **THPS-002 (SDK client boundary):** reframe as transitional. The JS clients are a
  sunset shim. Add: the client method table is recorded as the **draft core SDK API**
  so Phase 3 generation targets the same shape. Add a `route` receipt field already
  planned; also add an `idempotency_key` field on the client call signature for every
  state-changing verb (threaded to the runtime even before enforcement lands, so the
  wire shape is stable).
- **THPS-003 (native memory switchover):** add idempotency **enforcement**. The
  plugin generates a token per logical write (`remember`, `encode`, `relate`,
  `forget`, `handoff`), passes it to native MCP, and the runtime short-circuits a
  repeat token to the prior result. The field already exists at `types.rs:205`; this
  step wires plugin to runtime through it. This is load-bearing because the room
  itself returned HTTP 500 this session, so retries are the normal case, and an
  unguarded retry double-writes memory.
- **THPS-004 (native coordination switchover):** unchanged in mechanics; add that
  coordination writes (`coordinate`, `coordination_record`, intent) also carry an
  idempotency token, for the same retry-safety reason. The `event-next` chain in
  `event_log.rs` already enforces contiguous append, so coordination records get
  conflict detection for free; surface that as a receipt.
- **THPS-005 (run lifecycle and context ergonomics):** the biggest delta. The native
  run wrappers (`harness_begin`, `harness_step`, `harness_search`, `harness_context`,
  `harness_patch`, `harness_replay`, `harness_fork`, `harness_compare`,
  `harness_toolkit`) must be the **RPC projection of an event-stream core**, not a
  parallel model. Concretely:
  - Each wrapper appends a transition through the existing `event_log` path so the run
    is a single ordered event sequence with state hashes (already present).
  - Add a **cancel handle**: the run carries a cancel flag the loop polls at each
    transition boundary (the boundaries already exist), driving a clean `RUN.CANCELLED`.
  - Add **resume-from-seq**: a reader can request events after `seq = N` (bounded
    replay over the existing `event-next` chain).
  - Attach the existing **receipt** to each event in the read surface.
  - These are core/runtime additions Claude can own (THPS-005 already assigns native
    wrappers to Claude); they do not overlap Codex's plugin route files.
- **THPS-007 (product HTTP and connector alignment):** extend to **name skills as a
  first-class object** alongside affordances. Affordances are learned tools
  (`affordances.rs`, charter-scoped); skills are encoding-pipeline outputs. The SDK
  surface should reserve `skills` now (list/invoke), even if invocation initially
  delegates, so the three-units model (affordances/skills/runs) is not foreclosed.
- **THPS-009 (docs/skill truth pass):** add the SDK v2 layering to the docs:
  core/runtime/SDK/bindings, and the "async that does not fight the host runtime"
  design rule (core stays runtime-free and awaits only neutral primitives; I/O is a
  trait the runtime and bindings supply). This rule must be a stated invariant so a
  future contributor does not pull `reqwest` or a tokio-spinning crate into the core
  and silently break the binding model.
- **THPS-010 (migration and rollback controls):** unchanged; the four route modes
  (`native`/`compat`/`shadow`/`legacy`) remain. Add that the route receipt also
  reports `idempotency_key` and `event_seq` so a consumer can correlate retries and
  resume points.

### New steps the SDK v2 spec requires (no silent drops)

- **THPS-011: SDK core surface stabilization.** Define and freeze the core SDK API:
  the run handle (start/stream/cancel), the session handle (`open_session` over
  `BindingMemoryScope`, runs sharing the scratchpad), the affordance/skill/run
  objects, idempotency, resumable-from-seq, and the event+receipt shape. Output is a
  versioned `theorem-harness` Rust crate API plus a parity corpus. This is the spec's
  "land the core API first, stabilize it" gate. Owner: Claude designs the surface,
  Codex reviews the runtime contract.
- **THPS-012: Binding generation.** Once THPS-011 freezes the surface, generate the
  Node binding (NAPI-RS), then Python/Swift (UniFFI), then browser (wasm-bindgen).
  The Node binding replaces the THPS-002 JS shim. Each binding gets a thin idiomatic
  shell (Python context manager, TS async iterator, Swift actor). Owner: shared; the
  Node binding is the first because it retires the plugin shim.
- **THPS-013: Trace export.** `harness_export(run_or_session) -> training rows`
  (instruction/response SFT pairs and preference pairs from outcomes), in the format
  the training pipeline consumes. The receipts are the corpus. Owner: Claude.

## Revised phase graph (sequencing)

The phases are the spec's own discipline (substrate, then surface, then generation),
mapped onto Codex's steps. Phase numbers are the spec's structure, not invented
scope reduction.

- **Phase 0: Inventory + schema fix.** THPS-001. (Codex already fixed the top-level
  `anyOf` schema bug in `rustyred-thg-mcp` this session; the tool-contract matrix and
  the regression test that tool schemas expose no top-level `anyOf`/`oneOf`/`allOf`
  remain.)
- **Phase 1: Migration mechanics (native today).** THPS-002 (as sunset shim) + 003
  (memory, + idempotency enforcement) + 004 (coordination) + 005 (run lifecycle, +
  cancel/resume/receipt-on-event) + 006 (manifests) + 007 (connectors + skills name).
  At the end of Phase 1 the plugin's base harness state lives on native Theorem
  RustyRed, behind the four route modes.
- **Phase 2: SDK surface on the core.** THPS-011. Freeze the core SDK API (run
  handle, session handle, streams, idempotency, resume, export shape). No bindings
  yet. This is where runs-as-event-streams and sessions-as-handles become first-class
  in the Rust SDK.
- **Phase 3: Binding generation (gated on Phase 2).** THPS-012. Node first (retires
  the JS shim), then Python/Swift/wasm. THPS-013 trace export ships as a headline
  binding call.
- **Phase 4: Deploy, auth, truth gates.** THPS-008 + 009 (docs incl. SDK layering) +
  010 (rollback). Separate read-health and write-health gates; deploy proof and
  install proof are separate acceptance gates.

The honest constraint (spec's "one honest constraint"): Phase 3 must not start until
Phase 2 freezes the surface, or the bindings churn. Phase 1 ships value the whole
time it runs, independent of Phase 2/3.

## Ownership (reconciled with Codex's matrix)

Codex owns: plugin MCP route policy and handler changes (`server.mjs`,
`scripts/lib.sh`), native memory/coordination integration tests, deploy/write-mode/
auth smoke, runtime/MCP code when new native verbs are needed.

Claude owns or reviews: the tool-contract matrix and parity fixtures, the core SDK
surface design (THPS-011), the ergonomic native run wrappers + cancel/resume/event
receipts (THPS-005 core side), trace export (THPS-013), manifest + docs truth pass,
the SDK-layering documentation, and peer review before plugin release.

Shared seam: the **event-stream contract** (the shape of a streamed harness event +
its receipt) and the **draft core SDK API** (THPS-002 client method table == THPS-011
frozen surface). Both agents edit these only with an intent claim in the room.

## Spec coverage map (the floor, not the ceiling)

Every SDK v2 spec section has at least one step. No silent drops.

| SDK v2 spec section | Covered by |
|---|---|
| Layered structure (core/runtime/SDK/bindings) | THPS-011 + THPS-012 |
| Runs that stream typed events (raw + text views) | THPS-005 (core event surface) + THPS-011 (handle) |
| Sessions for continuity (sessions-as-scopes) | THPS-011 over existing `agent_binding.rs` |
| Three units: affordances, skills, runs | THPS-007 (skills name) + existing affordances + THPS-011 |
| Skills as encoding-pipeline SDK surface | THPS-007 extension + THPS-011 object |
| Idempotency tokens | THPS-002 (wire shape) + THPS-003/004 (enforcement) |
| Resumable streaming | THPS-005 (resume-from-seq) over existing `event_log.rs` |
| Cancellation as explicit channel | THPS-005 (cancel handle) over existing `RUN.CANCELLED` |
| Async that does not fight host runtime | THPS-009 (stated invariant) + THPS-011 (core stays runtime-free) |
| Receipts/provenance on every event | THPS-005 (receipt-on-event) over existing receipts |
| Trace export as headline | THPS-013 |
| Sequencing (stabilize core, generate once) | Phase 2 -> Phase 3 boundary |

## Open decisions for Travis (only the genuine ones)

These cannot be inferred from source and are product calls, not implementation
details:

1. **`recall` semantics.** Codex flagged this in THPS-003's risk: plugin `recall`
   currently points at saved-context preview recall, not native memory recall. SDK v2
   treats memory recall as the base verb. Proposal: `recall` means native memory
   recall; product preview moves behind `saved_context_preview_recall`. Confirm or
   override.
2. **Default stream view.** SDK v2 offers a raw typed-event stream and a text-only
   convenience stream. Proposal: text stream is the default for the plugin verbs (most
   callers want the answer), typed stream is opt-in for provenance. Confirm.
3. **First binding target.** Proposal: Node first (it retires the plugin JS shim and
   is the highest-leverage), then Swift (the iOS app), then Python (data/ML), then
   wasm. Confirm or reorder.

Everything else in this document is a proposal we (Claude + Codex) own under the
creative-freedom grant and will record as joint decisions in the room.
