# Theorem Harness SDK v2 Substrate Switchover

Date: 2026-06-02
Status: active implementation artifact; nested under SDK v2 architecture
Coordination room: `theorem-rustyred-plugin-switchover`
Parent architecture: `docs/plans/theorems-harness-plugin-switchover/sdk-v2-architecture-overlay.md`
Grounded companion and SDK tail: `docs/plans/theorems-harness-plugin-switchover/sdk-v2-architecture.md`
Source spec: `/Users/travisgilbert/Downloads/theorem-harness-sdk-v2-spec.md`

## Goal

Make the Theorem RustyRed switchover the first runtime-substrate slice of
`theorem-harness` SDK v2.

The parent architecture is no longer "swap the plugin from Theseus RustyRed to
Theorem RustyRed." SDK v2 is a Rust-core-with-generated-bindings product
surface: `theorem-harness-core` defines the stable logic contract,
`theorem-harness-runtime` persists that logic over a `GraphStore`, and generated
bindings expose the same semantics to Python, TypeScript/Node, browser WASM,
and Swift. This plan covers the migration mechanics that make the plugin, SDK,
and host hooks use the native Theorem RustyRed substrate while preserving the
user-facing harness verbs.

The plugin should stop treating Theseus as the default home for harness memory,
coordination, run lifecycle, and graph learning state. Theorem RustyRed becomes
the primary substrate for those base capabilities. Theseus remains available for
heavy engine work that is still legitimately Python/Django/Modal-backed.

## Current evidence

- The plugin source of truth is
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness`, not the
  installed cache.
- The slim plugin MCP currently defaults to
  `THEOREM_CONTEXT_BASE_URL=https://index-api-production-a5f7.up.railway.app/api/v2/theseus`
  and calls `theoremPost(...)` for memory, coordination, context, and harness
  run verbs.
- The slim plugin MCP has optional THG mirror writes through
  `THEOREMS_HARNESS_THG_WRITES`, but the default target is
  `https://thg-product-production.up.railway.app`, not the native Theorem
  RustyRed MCP target.
- `rustyred-thg-mcp` now exposes native coordination reads/writes, native
  memory reads/writes, observe, handoff, graph reads, graph algorithms, and
  `harness_run` / `harness_append_transition` over `theorem-harness-runtime`.
- `theorem-harness-runtime::memory` exists and is GraphStore-backed. Runtime and
  MCP tests cover memory round trips, archive, handoff, relate, encode, forget,
  RedCore reopen, and read-only/write-mode gating.
- `apps/theorem-harness-server` exposes HTTP run, coordination read, and
  connector surfaces over the same runtime family. It is already the iOS
  product read transport.
- `theorem-harness-runtime::skill_pack` now stores content-addressed
  `SkillPack` nodes keyed by `pack_content_hash`, source/artifact hash edges,
  and durable `SkillPackUseReceipt` nodes. `rustyred-thg-mcp` exposes
  `skill_list` and `skill_get` in read-only mode plus write-gated
  `skill_publish` and `skill_apply`.
- The source plugin route policy now has a native MCP client and treats
  `skill_list`, `skill_get`, `skill_publish`, and `skill_apply` as native
  Theorem/RustyRed capability verbs. The slim plugin MCP advertises those tools
  and forwards them through the native MCP route with route receipts.
- The source plugin slim MCP now routes the core room/agent-space tools
  native-first through route policy: `coordination_room`, `presence`,
  `coordination_intent`, `coordination_reflection`, `coordination_decision`,
  `coordination_tension`, `coordinate`, and `mentions`. The old Theseus
  endpoints remain fallback when the deployed native write surface is missing
  or unauthenticated.
- `rustyredcore_THG/crates/theorem-harness` now exists as the native Rust SDK
  surface over core/runtime. It exposes run handles, stream cursors, session
  memory, idempotency tokens, and trace export without leaking the raw transition
  machinery as the public binding contract.
- `apps/theorem-harness-node` now exists as a standalone NAPI-RS binding over
  the Rust SDK. The binding opens a durable `RedCoreGraphStore` from a data dir
  and exposes `startRun`, `cancel`, `eventsJson`, `pollText`, `runStatus`,
  `remember`, and `recall`.
- Current local worktree checkpoint: `TransitionInput.idempotency_key` persists
  through `EventState`, `HarnessEvent` nodes, `HARNESS_EVENT_OF` edges, and
  replay. Old empty-key events remain compatible, and the core JSON boundary
  accepts `idempotencyKey` aliases.
- Live coordination check during planning: native Theorem room/context worked;
  the current marketplace plugin room and mention endpoints returned HTTP 500.
  Read health and write/admission health must therefore be separate gates.
- Claude-side incident during planning: `rustyredcore-theorem-production` was
  advertising six native MCP tools with top-level `anyOf` schemas
  (`harness_run`, `relate`, `self_revise`, `self_archive`, `handoff`,
  `harness_append_transition`). Claude rejected the whole request with
  `tools.24.custom.input_schema` before `/theorems-harness:coordinate` could
  execute. Local fix: advertise canonical `required` fields and keep alias
  handling in Rust runtime code; add a regression test that tool schemas do not
  expose top-level `anyOf` / `oneOf` / `allOf`.

## Design decision

The best path is not a one-line URL swap. It is a contract-preserving SDK v2
substrate migration:

1. Stabilize the SDK v2 Rust core contract before binding generation: streamed
   runs, sessions as binding scopes, affordances, skills, idempotency,
   cancellation, resumable events, receipts, and trace export.
2. Keep the public plugin verbs stable: `/harness`, `/coordinate`, `/encode`,
   `remember`, `recall`, `harness_replay`, `harness_fractal_expansion`, and the
   skill behavior should feel continuous to agents.
3. Make Theorem RustyRed the primary substrate client for harness base state:
   memory, coordination, run lifecycle, graph search, graph algorithms,
   affordance registration, and connector learning.
4. Treat native MCP and HTTP as adapters over the SDK/runtime contract, not as
   the contract itself.
5. Split the plugin-facing route layer into explicit clients instead of one
   ambiguous context client: `NativeBindingClient` for local Rust SDK/RedCore
   calls, `TheoremHarnessMcpClient` for shared native MCP verbs,
   `TheoremHarnessHttpClient` for product/read HTTP routes, and
   `TheseusEngineClient` for explicit heavy Python engine fallback.
6. Remove silent mirror semantics. A call is either primary-native, dual-run
   shadow-verified, or explicitly delegated to Theseus. The result receipt must
   say which path executed.
7. Generate or validate the plugin tool table against native `tools/list` so
   docs and host manifests cannot drift quietly.

## SDK Tail Topology

THPS-011, THPS-012, and THPS-013 stay in
`sdk-v2-architecture.md` as the SDK tail rather than being duplicated here. This
plan is the migration lane: plugin routing, native MCP/HTTP adapters, route
receipts, manifest truth, deployment gates, and fallback controls.

The companion doc owns the higher-level SDK tail:

- THPS-011: freeze the native Rust SDK surface over `theorem-harness-core` and
  `theorem-harness-runtime`. Current state: landed in
  `rustyredcore_THG/crates/theorem-harness`.
- THPS-012: generate bindings from that frozen Rust surface, starting with
  Node/NAPI-RS. Current state: Node binding landed in `apps/theorem-harness-node`
  with durable RedCore run lifecycle, stream polling, status, and memory.
- THPS-013: expose trace export as a headline SDK capability.

This file can reference those steps, but the full definitions live in the
companion so the parent architecture and migration mechanics do not drift into
two competing SDK specs.

## Coordination room readiness snapshot

This is the near-term shape needed before the larger Codex + Claude planning
session starts.

| Gate | Current state | Evidence | Next action |
|---|---|---|---|
| Native room can hold membership/intent | Working locally for Codex | `coordination_intent` wrote a `working` intent to `theorem-rustyred-plugin-switchover`; `read_intents_for_room` read it back | Claude should join the same room on its next clean turn |
| Native durable records work | Working locally for Codex | `coordination_record` wrote event records for the switchover and schema incident | Use records for decisions, tensions, and handoff notes in the larger session |
| Native direct mentions work | Working locally for Codex | `coordinate` queued messages to `claude-code` in the native room | Claude should drain mentions after its schema load succeeds |
| Claude can load native MCP schemas | Fixed in local source and usable in current room flow | Earlier live `rustyredcore-theorem-production` advertised top-level `anyOf`; local `rustyred-thg-mcp` source now advertises canonical `required` fields and regression coverage | Keep schema regression in CI; verify installed plugin cache after any reinstall |
| Old Theseus/plugin coordination path | Not reliable | Current marketplace plugin room/mentions returned HTTP 500; `theseus-mcp-production` returned HTTP 500 during live check | Do not use old Theseus coordination as the planning substrate |
| Product THG endpoint schema | Clean but not the target room | `thg-product-production` live `tools/list` has no top-level schema combinators | Keep separate from Theorem-native room unless intentionally bridging product graph reads |

For the larger planning session and implementation work, the intended substrate
is the native Theorem room. The earlier Claude blocker was schema advertisement,
not the underlying room model; keep verifying installed cache state after source
or deployment changes so that failure mode does not return silently.

## Non-goals

- Do not port the heavy `theseus_*` engine verbs in this slice unless the user
  explicitly reopens that boundary. They invoke Python/Modal/scorer/model
  surfaces and are a different migration.
- Do not remove the local slim plugin MCP. It becomes the compatibility router
  and host adapter.
- Do not create a second harness runtime crate. Extend the existing
  `theorem-harness-core`, `theorem-harness-runtime`, `rustyred-thg-mcp`,
  `apps/theorem-harness-server`, and plugin source.
- Do not deploy as part of this shaping slice. Deployment, write-mode auth, and
  installed-plugin cache verification remain explicit later gates.

## Target architecture

| Layer | Target role | Primary home |
|---|---|---|
| SDK v2 Rust contract | Streamed runs, sessions, affordances, skills, idempotency, cancellation, resumability, receipts, trace export | `theorem-harness-core`, Rust SDK crate |
| Generated bindings | Python, TypeScript/Node, browser WASM, Swift surfaces generated from stable Rust contract | UniFFI, NAPI-RS v2, wasm-bindgen |
| Plugin skills and commands | User-facing behavior, routing instructions, host affordances | `codex-plugins/theorems-harness/skills`, `commands`, `agents` |
| Slim plugin MCP | Local compatibility router, host identity, environment policy, receipt formatting | `codex-plugins/theorems-harness/mcp/server.mjs` |
| Harness route policy | Chooses native binding, native MCP, product HTTP, or explicit Theseus fallback and emits receipts | New SDK module in the plugin source, then extractable package |
| Native Node binding | Local durable run/session surface over the Rust SDK | `apps/theorem-harness-node` |
| Native harness MCP | Base memory, coordination, run lifecycle, graph search/algorithms | `rustyredcore_THG/crates/rustyred-thg-mcp` |
| Runtime substrate | Durable RedCore GraphStore state | `rustyredcore_THG/crates/theorem-harness-runtime` |
| Product HTTP | iOS/web product reads, connector management, operational smoke routes | `apps/theorem-harness-server` |
| Theseus engine fallback | Heavy Python/Modal/model work only | Existing Theseus MCP/API, explicitly labeled |

## Verb routing matrix

| Verb family | Examples | Target route |
|---|---|---|
| Memory write/read | `remember`, `recall`, `self_note`, `self_revise`, `self_archive`, `self_recall_archive`, `encode`, `forget`, `handoff`, `observe` | Native `rustyred-thg-mcp` first |
| Coordination | `coordination_room`, `coordination_intent`, `coordination_record`, `coordinate`, `mentions`, `presence`, `coordination_context` | Native `rustyred-thg-mcp` first |
| Coordination completeness | `mentions_wait`, `resolve_tension`, typed `read_*_since` | Add to native MCP, then route plugin there |
| Run lifecycle | `harness_begin`, `harness_step`, `harness_search`, `harness_context`, `harness_patch`, `harness_replay`, `harness_fork`, `harness_compare`, `harness_toolkit` | Node binding first for local durable runs; native MCP wrappers for shared/remote runs |
| Graph and research | `harness_fractal_expansion`, `fractal_expand`, `ppr_neighborhood`, `code_search` | Native graph/PPR/fulltext where data exists; Theseus only for missing code-symbol ingestion |
| Connectors and affordances | `connectors/register`, connector listing, invoke bridge | `apps/theorem-harness-server` plus `rustyred-thg-connectors` and `rustyred-thg-affordances` |
| Product context/saved contexts | `saved_context_*`, `memory_patch_review_*`, product bootstrap | Keep on product HTTP until native document/context lanes land |
| Heavy Theseus engines | `theseus_code_agent`, scorer, epistemic engine, Modal dispatch | Explicit Theseus fallback, never hidden under harness base verbs |

## Build plan

### THPS-000: SDK v2 architecture alignment

Owner: Codex and Claude jointly.

Files:
- `docs/plans/theorems-harness-plugin-switchover/sdk-v2-architecture-overlay.md`
- `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`
- `rustyredcore_THG/crates/theorem-harness-core/src/`
- `rustyredcore_THG/crates/theorem-harness-runtime/src/`

Work:
- Treat the user-provided SDK v2 spec as the parent architecture for this plan.
- Reclassify THPS as the runtime-substrate migration lane under SDK v2.
- Define the minimum stable Rust core API before any generated bindings are
  attempted: streamed runs, sessions, affordances, skills, idempotency,
  cancellation, resumable events, receipts, and trace export.
- Mark native MCP, product HTTP, and plugin proxy surfaces as transports or
  host adapters over the SDK contract.
- Add binding-generation gates to the plan so Python, TS/Node, Swift, and WASM
  are generated only after the core surface is stable.

Acceptance:
- The plan names SDK v2 as the parent architecture.
- The existing Theorem RustyRed switchover is preserved as migration mechanics,
  not discarded.
- No generated-binding work starts until the Rust contract is stable.

Validation:
- `cargo test -p theorem-harness-core`
- `cargo test -p theorem-harness-runtime`
- Native MCP `tools/list` schema compatibility check
- Cross-host coordination dogfood in `theorem-rustyred-plugin-switchover`

### THPS-001: Live surface inventory and contract snapshot

Owner: Codex primary, Claude review.

Files:
- `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/README.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`

Work:
- Capture the plugin tool list from `server.mjs`.
- Capture native Theorem MCP `tools/list` in read-only and write mode.
- Produce a verb diff: plugin-only, native-only, common but schema-different,
  common and safe to route.
- Check every advertised MCP `inputSchema` for Claude-compatible top-level
  shape: no `anyOf`, `oneOf`, or `allOf` at the root. Runtime aliases belong in
  handler code, not top-level schema composition.
- Record the exact current failure modes: plugin Theseus 500s, native read
  success, native write-mode availability, deployed binary version.

Acceptance:
- A checked-in `tool-contract-matrix.md` lists every plugin verb and its target.
- No verb is classified as "unknown".
- Read health and write health are tracked separately.
- Tool-list schema compatibility is tested for both read-only and write-enabled
  native MCP modes.

Validation:
- `node /Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
  can still list tools locally.
- Native MCP `tools/list` is captured for read-only and write-enabled config.

Risk:
- The installed plugin may differ from source. Always inspect the source repo
  first, then installed cache only as a deploy verification step.

### THPS-002: Route policy and host client boundary

Owner: Codex primary, Claude API review.

Files:
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/scripts/lib.sh`
- New plugin SDK files under `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/`

Work:
- Add a small route-policy layer used by `server.mjs` and shell hooks:
  `NativeBindingClient` (Node/NAPI binding), `TheoremHarnessMcpClient`
  (shared native MCP), `TheoremHarnessHttpClient` (product/read HTTP),
  `TheseusEngineClient` (explicit heavy-engine fallback), and a shared
  `HarnessRoutePolicy`.
- Do not rebuild a permanent hand-written JS harness SDK. Any hand-written JS
  code is a transport adapter or sunset shim; the Rust SDK and generated binding
  are the source of truth for run/session semantics.
- Normalize env names:
  - `THEOREM_HARNESS_DATA_DIR` for the local RedCore directory used by the
    Node binding.
  - `THEOREM_HARNESS_MCP_URL` for native Theorem MCP.
  - `THEOREM_HARNESS_HTTP_URL` for `apps/theorem-harness-server`.
  - `THESEUS_ENGINE_MCP_URL` or existing `THEOREM_CONTEXT_BASE_URL` for engine
    fallback.
  - `THEOREM_HARNESS_API_TOKEN` for native/authenticated writes.
- Preserve compatibility aliases for one release: existing
  `THEOREM_CONTEXT_*`, `THEOREMS_HARNESS_THG_*`, and `RUSTYRED_THG_*` env vars
  still work but emit structured deprecation receipts in debug output.
- Add receipt metadata to every call: `route`, `server`, `tenant`, `read_only`,
  `fallback_used`, and `native_write_mode`.

Acceptance:
- `server.mjs` no longer embeds route policy in each verb handler.
- Local run lifecycle can use the Node binding without calling Theseus.
- Shell hooks use the same route policy for run/session/coordination writes.
- No secret values are printed in receipts.

Validation:
- Plugin MCP unit/smoke tests cover route policy, env fallback, and redaction.
- Existing commands still work against a fake local server.

Risk:
- Shell hooks are easy to leave behind. Treat `scripts/lib.sh` as part of the
  SDK boundary, not a legacy afterthought.

### THPS-003: Native memory switchover

Owner: Codex primary, Claude parity fixture review.

Files:
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`
- `rustyredcore_THG/crates/theorem-harness-runtime/src/memory.rs`
- `docs/plans/harness-rust-port/parity-memory/`

Work:
- Route `remember`, `recall`, `relate`, `self_note`, `self_revise`,
  `self_archive`, `self_recall_archive`, `encode`, `forget`, `handoff`, and
  `observe` to native MCP by default.
- Allow local private session memory to use `apps/theorem-harness-node` when the
  route policy selects an in-process `THEOREM_HARNESS_DATA_DIR`; coordination
  and shared room memory still default to native MCP so multiple heads see the
  same substrate.
- Keep Theseus memory fallback disabled by default. If enabled for migration,
  it must be explicit: `THEOREM_HARNESS_FALLBACK=theseus-read` or
  `theseus-shadow`.
- Add schema adapters where plugin argument names differ from native MCP
  names, for example `tenant_slug` vs `tenant`, `doc_id` vs `docId`.
- Add dual-read verification for a temporary migration window: native recall
  result plus optional Theseus recall comparison, reported as a receipt rather
  than blended into one answer.

Acceptance:
- Default plugin memory writes persist to native Theorem GraphStore.
- Every state-changing memory write emits the idempotency token route receipt
  needed to detect duplicate retries.
- Archived and forgotten memory do not appear in default native recall.
- Handoffs can be targeted and consumed natively.
- Fallback path is never silent.

Validation:
- `cd rustyredcore_THG && cargo test -p theorem-harness-runtime`.
- `cd rustyredcore_THG && cargo test -p rustyred-thg-mcp native_memory_tools_round_trip_through_mcp`.
- Plugin fake-server tests for every memory verb.
- One local or deployed smoke writes `remember`, reads `recall`, writes
  `encode`, and checks `observe`.

Risk:
- Existing plugin `recall` currently points at saved-context preview recall,
  not native memory recall. Decide whether to keep a separate saved-context
  command or make `recall` mean memory recall and move product preview behind
  `saved_context_preview_recall`.

Progress:
- The Rust SDK `Session::remember` / `Session::recall` surface is landed.
- The Node binding exposes durable `Harness.remember` / `Harness.recall`.
- Runtime idempotency persistence is validated in the current worktree.
- First plugin memory wiring is landed in source: `remember`, `recall`, and
  `self_note` now route through `HarnessRoutePolicy`. `remember` and
  `self_note` preserve shared compat behavior by default and use the Node
  binding only for `scope: private`; `recall` now treats `query` as native
  memory recall and keeps `tenant_slug` plus `task` as saved-context preview
  compatibility.
- Fake-transport route-policy tests and a real native-binding memory smoke pass
  against `apps/theorem-harness-node/theorem_harness_node.node`.

Next implementation steps:
- Wire the rest of the memory family through the same policy: `self_revise`,
  `self_archive`, `self_recall_archive`, `encode`, `relate`, `forget`,
  `handoff`, and `observe`.
- Add plugin-level idempotency-token generation and duplicate-write
  short-circuit once all state-changing memory writes use the route boundary.

### THPS-004: Native coordination switchover

Owner: Codex primary, Claude coordination reviewer.

Files:
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/skills/harness-coordinate/SKILL.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/hooks/hooks.json`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/hooks/codex-hooks.json`
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`

Work:
- Route room, presence, intent, record, message, mentions, and context reads to
  native MCP.
- Add or finish native `mentions_wait`, `resolve_tension`, typed
  `read_decisions_since`, `read_events_since`, `read_open_tensions`, and
  `read_reflections_for_room`.
- Update hook flow to read native coordination context at session start and
  prompt submit.
- Keep the gossip protocol: no handshake loop, no lane-first behavior, compact
  durable records.

Acceptance:
- Codex and Claude can join the same native room, write records, mention each
  other, and read the result from a cold next turn.
- Current plugin 500 failures are not on the default path.
- `harness-coordinate` docs name the native Theorem MCP as the primary home.

Validation:
- Native MCP coordination tests cover read-only and write-enabled behavior.
- Plugin fake-server tests cover route selection and mention parsing.
- Live smoke in room `theorem-rustyred-plugin-switchover`: write event,
  decision, mention, read context.

Risk:
- `mentions_wait` can tie up host requests. Keep short polling caps and prefer
  checkpoint reads.

Progress:
- 2026-06-05 source plugin wiring landed for the core room/agent-space tools:
  `coordination_room`, `presence`, `coordination_intent`,
  `coordination_reflection`, `coordination_decision`, `coordination_tension`,
  `coordinate`, and `mentions` now go native-first through the route policy and
  fall back to the old Theseus endpoint when native write/read is unavailable.
- `coordination_reflection`, `coordination_decision`, and
  `coordination_tension` map to native `coordination_record` with
  `record_type` set to `reflection`, `decision`, or `tension`.

Remaining:
- `mentions_wait`, `subscribe`, and `continuity_pack` still use compatibility
  routes because native blocking wait and continuity-pack contracts are not yet
  one-to-one.
- Hook flow still needs a native SessionStart/UserPromptSubmit context pass.

### THPS-005: Run lifecycle and context ergonomics

Owner: Claude owns the core/runtime event surface. Codex owns plugin route
integration.

Files:
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`
- `rustyredcore_THG/crates/theorem-harness-core/src/*`
- `rustyredcore_THG/crates/theorem-harness-runtime/src/*`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/skills/replay-last-run/SKILL.md`

Work:
- Treat native run wrappers as the RPC projection of the SDK event-stream core,
  not a parallel run model.
- Use `apps/theorem-harness-node` as the first plugin run-lifecycle transport
  where a local durable RedCore data dir is acceptable.
- Add ergonomic native MCP wrappers over existing core/runtime:
  `harness_begin`, `harness_step`, `harness_search`, `harness_context`,
  `harness_patch`, `harness_replay`, `harness_fork`, `harness_compare`,
  `harness_toolkit`, `harness_fractal_expansion`, and `ppr_neighborhood`.
- Keep `harness_append_transition` as the low-level primitive, but stop making
  plugin users know event names for ordinary run operations.
- Add resume-from-seq over the existing per-run event sequence.
- Attach receipts to every event surfaced by a stream or replay call.
- Add a cancel handle and polled cancel channel that drives the existing
  `RUN.CANCELLED` transition.
- Route plugin `harness_replay`, `harness_fractal_expansion`, and context
  operations to native wrappers when present.

Acceptance:
- A plugin-started run can be replayed natively with state hashes and event
  sequence intact.
- A run can be cancelled through the native surface and replayed through the
  `RUN.CANCELLED` event path.
- A reader can resume from a known event sequence and receive event receipts.
- Context packs are compiled by the native `context_web` contract where
  possible.
- Graph/PPR research never falls back to Python `push_ppr` silently.

Validation:
- `cd rustyredcore_THG && cargo test -p theorem-harness-core -p theorem-harness-runtime -p rustyred-thg-mcp`.
- `node apps/theorem-harness-node/smoke.mjs`.
- `node apps/theorem-harness-node/recover.mjs` after a two-process write smoke.
- Plugin integration test starts a run, appends a step, replays it, and forks
  it against a fake native MCP.

Risk:
- `context_compile` in the plugin currently calls Theseus `/context/compile/`.
  Preserve product-context behavior under a clear name while adding native
  `harness_context`.

### THPS-006: Plugin manifest and host install switchover

Owner: Claude manifest pass, Codex verification.

Files:
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/plugin.manifest.json`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/.claude-plugin/plugin.json`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/.codex-plugin/plugin.json`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/.mcp.json`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/README.md`

Work:
- Register native Theorem RustyRed MCP as the primary remote server for the
  plugin.
- Keep the local slim MCP as `theorems-harness` compatibility router.
- Move old Theseus server registration to `theseus-engine` with description
  that it is for heavy engine fallback, not harness base state.
- Remove or rename `rustyred-thg` pointing at `thg-product-production` unless it
  is still needed as a separate product graph target.
- Bump plugin version and regenerate host manifests from source.

Acceptance:
- Claude and Codex plugin manifests both name the same primary native server.
- README environment table matches actual env names.
- No manifest implies memory/coordination live on Theseus.

Validation:
- Manifest sync script or equivalent check passes.
- Fresh install/cached install comparison shows the intended MCP server list.

Risk:
- Installed plugin cache drift can make the user think the source change failed.
  Verification must include source, generated host manifests, and installed
  cache after reinstall.

### THPS-007: Product HTTP and connector alignment

Owner: Codex primary, Claude iOS/product reviewer.

Files:
- `apps/theorem-harness-server/src/lib.rs`
- `apps/theorem-harness-server/src/main.rs`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- `docs/plans/mcp-learning-layer/`

Work:
- Keep connector registration/listing on `apps/theorem-harness-server`.
- Add plugin SDK methods for connector listing/register/invoke where safe.
- Ensure connector affordance registrations and native MCP memory/coordination
  write to the same tenant RedCore directory or a clearly bridged tenant.
- Make the plugin expose connector-learning state as harness capability scope,
  not as a second disconnected product panel.

Acceptance:
- Registered connectors appear in the HTTP product surface and can be used by
  the harness capability router.
- Connector affordances have writeback-policy badges and invocation receipts.
- iOS/web read surfaces can see MCP-written substrate state for the same tenant.

Validation:
- `cd apps/theorem-harness-server && cargo test`.
- Connector fake transport test plus one operator-runnable register smoke.

Risk:
- Slow MCP server handshakes must not hold the GraphStore lock. Keep existing
  spawn/register discipline.

### THPS-007A: Native skill-pack serving

Owner: Codex primary, Claude validation-gate reviewer.

Files:
- `rustyredcore_THG/crates/theorem-harness-runtime/src/skill_pack.rs`
- `rustyredcore_THG/crates/theorem-harness-runtime/src/lib.rs`
- `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`
- `docs/plans/skill-encoder-theorem-port/implementation-plan.md`

Work:
- Store finished `CapabilityPackSpec` artifacts from the Theseus offline
  encoder as content-addressed native `SkillPack` graph nodes.
- Expose `skill_list` and `skill_get` in read-only native MCP mode so agents can
  discover and pull packs without write credentials.
- Expose `skill_publish` and `skill_apply` only in write mode. `skill_publish`
  accepts the pack JSON and provenance hashes. `skill_apply` writes a durable
  use receipt that can feed the later promotion loop.
- Keep the runtime path Python-free. The encoder remains an out-of-band
  producer; it does not become a harness runtime dependency.

Current state:
- Shipped 2026-06-05: `skill_pack` runtime module, MCP `skill_*` verbs,
  read-only/write-mode tool-list gating, source/artifact graph edges, and
  `SkillPackUseReceipt` persistence.
- Shipped 2026-06-05: source plugin route-policy wiring for `skill_*` through
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/route-policy.mjs`
  and
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`.
- Shipped validator mode: safe deterministic declaration checks
  (`required_field`, `context_required_field`, `artifact_hash_present`,
  `always_pass`) with `validator_execution_mode: safe_declaration`.
- Shipped validator mode: bounded native artifact-descriptor checks for
  `native_validator_candidate` / Rust validator artifacts emitted by the
  Theseus encoder, currently covering the code-member signature predicate with
  `validator_execution_mode: native_artifact_sandbox`.
- Shipped 2026-06-05 in Index-API: the 20-task held-out Rust refactoring
  corpus plus the pure baseline-vs-treatment receipt-scoring gate in
  `apps/notebook/encode/benchmarks.py`.
- Not shipped yet: a live E1 encoded-pack run feeding real baseline/treatment
  receipts through that gate, live authenticated deployment smoke,
  installed-cache verification, and any future compiled crate/WASM validator
  runner.

Acceptance:
- Native MCP can publish a pack, list it, read it by id or hash, apply it, and
  persist a use receipt without calling Theseus.
- Read-only mode advertises `skill_list` and `skill_get` but not
  `skill_publish` or `skill_apply`.
- Write mode advertises all four `skill_*` verbs with top-level object schemas
  and no root schema combinators.
- The response receipt makes validator execution mode explicit.
- The source plugin can call the native `skill_*` tools through route policy,
  with `route_receipt.route = native-mcp`.
- Applying a pack with native validator artifact descriptors executes the
  bounded descriptor sandbox and persists those validator receipts as
  `SkillPackUseReceipt` data.

Validation:
- `cd rustyredcore_THG && cargo test -p theorem-harness-runtime`
- `cd rustyredcore_THG && cargo test -p rustyred-thg-mcp`
- `cd rustyredcore_THG && cargo clippy -p theorem-harness-runtime -p rustyred-thg-mcp --all-targets --no-deps -- -D warnings`
- `cd /Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness && node sdk/route-policy.test.mjs`

Risk:
- Do not run arbitrary uploaded Rust validator code in the MCP request path.
  The shipped S3b runner interprets bounded descriptors only; compiled
  crate/WASM execution must be a separate isolated runner.
- Cargo resolution currently depends on the dirty RustyWeb optional
  impersonation dependency being resolvable. This slice unblocked default builds
  by aliasing that optional dependency to `wreq`, but did not validate the
  `impersonate-fetch` feature.

### THPS-008: Deployment, auth, and live truth gates

Owner: Codex primary, Claude smoke reviewer.

Files:
- `rustyredcore_THG/Dockerfile`
- `apps/theorem-harness-server/Dockerfile`
- Railway config for the native MCP and harness server
- Plugin README and release notes

Work:
- Deploy `rustyred-thg-mcp` with a binary that contains native memory and
  coordination writes.
- Enable write mode only behind bearer auth and tenant scoping.
- Point the plugin defaults at the native Theorem MCP and HTTP surfaces.
- Keep write mode disabled for public unauthenticated URLs.
- Add health endpoints or smoke scripts that prove read-only, authenticated
  write, and tenant-isolated behavior separately.

Acceptance:
- Live `tools/list` shows read tools when unauthenticated/read-only.
- Authenticated write-mode `tools/list` shows write tools.
- Live smoke writes memory, reads memory, writes coordination, reads
  coordination context, starts or appends a run, and verifies tenant isolation.
- If any smoke fails, plugin release remains on compatibility mode.

Validation:
- `cargo test -p rustyred-thg-mcp`.
- `cargo test -p theorem-harness-runtime`.
- `cargo test` in `apps/theorem-harness-server`.
- Live MCP JSON-RPC smoke with bearer auth against a scratch tenant.

Risk:
- A deployed service can be green while the installed client still points at old
  URLs. Treat deploy proof and plugin install proof as separate acceptance gates.

### THPS-009: Documentation and skill truth pass

Owner: Claude primary, Codex technical review.

Files:
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/README.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/skills/**/*.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/commands/**/*.md`
- `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/references/*.md`

Work:
- Update all skill docs to say base harness state is native Theorem RustyRed.
- Replace stale Redis/Theseus wording where it implies the base harness lives
  on Python.
- Keep explicit fallback docs for Theseus heavy engines.
- Add a short operator section: how to verify which backend a plugin call used.
- Add "do not claim native" examples for failed write-mode, missing auth, or
  installed-cache drift.

Acceptance:
- No docs claim memory/coordination are Python-backed base paths.
- No docs hide the heavy-engine fallback.
- Skill instructions continue to teach the gossip coordination protocol.

Validation:
- `rg` over plugin docs for old URLs and stale terms:
  `theseus-mcp-production`, `index-api-production`, `thg-product-production`,
  `Redis harness`, `mirror writes`.
- Manual doc review against the contract matrix.

Risk:
- Over-correcting docs could imply Theseus is gone. Theseus remains canonical
  for Python engine surfaces and cross-repo mirror discipline.

### THPS-010: Migration and rollback controls

Owner: Codex primary, Claude release reviewer.

Files:
- Plugin SDK route policy
- Plugin README
- Release notes

Work:
- Add route modes:
  - `native`: Theorem native only.
  - `compat`: native reads, Theseus fallback only for explicitly allowed
    product/engine commands.
  - `shadow`: execute native primary and compare selected Theseus reads.
  - `legacy`: old path for emergency rollback.
- Default to `compat` during first release, then `native` once live smoke and
  install smoke pass.
- Add a route receipt to every plugin tool response.
- Add a rollback checklist that changes env only, without code edits.

Acceptance:
- User can tell exactly which backend handled a tool call.
- Rollback does not require editing source files.
- Shadow mode never promotes mismatched results silently.

Validation:
- Route-policy tests for all four modes.
- Manual smoke with route mode env override.

Risk:
- Too much fallback can conceal bugs. `legacy` should be a deliberate release
  escape hatch, not the normal mode.

## Cross-agent ownership

Codex should own:
- SDK route policy and plugin MCP handler changes.
- Native memory and coordination integration tests.
- Deployment/write-mode/auth smoke scripts.
- Runtime/MCP code changes when new native verbs are needed.

Claude Code should own or review:
- Tool-contract matrix and parity fixture review.
- Manifest and docs truth pass.
- THPS-005 core/runtime event surface: event stream, cancel handle,
  resume-from-seq, and receipt-on-event.
- THPS-011/012/013 SDK tail: Rust SDK surface freeze, generated bindings, and
  trace export.
- Peer review before plugin release.

Both agents should use the native room `theorem-rustyred-plugin-switchover`.
The source-of-truth handoff is this plan plus native coordination records. Use
path-scoped commits only.

## Acceptance summary

The switchover is complete only when:

- The plugin source, generated host manifests, and installed cache all point
  harness base state at Theorem RustyRed.
- Native memory, coordination, graph, and run lifecycle calls pass local and
  live authenticated smoke tests.
- The plugin response receipt names the route used for every stateful verb.
- Theseus fallback is explicit and limited to heavy engine/product surfaces.
- Docs and skills describe the live truth.
- Claude and Codex have reviewed the contract matrix and release smoke output.

## Next execution slice

THPS-001, THPS-011, THPS-012 Node, and the runtime idempotency persistence slice
are now live enough to stop shaping and wire the first plugin adapter.

Current code checkpoint:
- Plugin source now has a standalone route-policy skeleton in
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/sdk/route-policy.mjs`.
- Fake-transport coverage lives beside it in `sdk/route-policy.test.mjs`.
- `harness_replay` is the first real handler wired through the route policy in
  `mcp/server.mjs`: when `THEOREM_HARNESS_DATA_DIR` and
  `THEOREM_HARNESS_NODE_BINDING_PATH` are configured, replay executes through
  `apps/theorem-harness-node`; otherwise compat mode falls back to the existing
  legacy HTTP path with an explicit fallback receipt.
- Real native replay smoke passed against the built Node binding and a temp
  RedCore data dir.
- `remember` / `recall` / `self_note` are wired through the same route policy.
  Private memory uses the Node binding over a configured
  `THEOREM_HARNESS_DATA_DIR`; shared memory keeps an explicit legacy fallback
  until the native MCP shared-memory adapter is wired. Saved-context preview
  remains available through `saved_context_preview_recall` and through old
  `recall` calls that pass `tenant_slug` plus `task`.
- Real native memory smoke passed through `HarnessRoutePolicy.execute` against
  the built Node binding and a temp RedCore data dir for `remember`, `recall`,
  and `self_note`.

Next wiring slice:

1. Wire run-write verbs (`harness_begin`, `harness_step`, cancel/status where
   exposed) through the Node binding with existing legacy behavior as compat
   fallback.
2. Wire the remaining memory-family verbs through native MCP or the Node binding
   according to the matrix, starting with `encode` on the native MCP adapter so
   outcome and fitness metadata are preserved.
3. Emit a route receipt for every stateful tool call.
4. Only after route receipts and fake tests pass, flip the default from legacy
   to compat.
