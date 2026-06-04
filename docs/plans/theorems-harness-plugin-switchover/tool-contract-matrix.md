# THPS-001 Tool Contract Matrix

Date: 2026-06-02
Status: active contract snapshot with first handler wiring
Owner: Codex
Coordination room: `theorem-rustyred-plugin-switchover`

## Purpose

This matrix keeps the plugin switchover honest. It maps the current
Theorems-Harness plugin tools to the intended Theorem RustyRed route, while
separating native-ready verbs from product HTTP surfaces and explicit Theseus
fallbacks.

The goal is not to move every tool to native MCP. The goal is to make every
route deliberate, observable, and testable.

## Sources

- Plugin source of truth:
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- Plugin manifest source:
  `/Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/plugin.manifest.json`
- Native Theorem RustyRed MCP:
  `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`
- SDK v2 plan:
  `docs/plans/theorems-harness-plugin-switchover/implementation-plan.md`
- SDK v2 surface companion:
  `docs/plans/theorems-harness-plugin-switchover/sdk-v2-architecture.md`
- THPS-011 Rust SDK surface:
  `docs/plans/theorems-harness-plugin-switchover/thps-011-sdk-surface.md`

## Route Classes

| Class | Meaning |
|---|---|
| `native-now` | Native Theorem RustyRed MCP already has the substrate verb. Plugin should route there after schema adapter and auth/write-mode gating. |
| `native-adapter` | Native substrate exists, but the plugin verb needs a semantic or schema adapter. |
| `sdk-surface` | Routes through the `theorem-harness` Rust SDK surface. Node/NAPI-RS now exists in `apps/theorem-harness-node`; MCP remains the shared/remote host adapter. |
| `product-http` | Keep on `apps/theorem-harness-server` or product HTTP for now. Not a Theseus fallback. |
| `theseus-engine` | Heavy Python/Django/code-ingest/model path. Keep explicit and labelled. |
| `rename-or-fold` | Current plugin verb is duplicate, stale, or too ambiguous to keep as-is. |

## Compatibility Rules

- Every state-changing native call must carry an idempotency token once the
  route policy lands.
- The runtime now persists that token as canonical `idempotency_key` on events
  and accepts camelCase `idempotencyKey` at the Rust JSON boundary.
- Every plugin response for a stateful verb must include a route receipt:
  `route`, `server`, `tenant`, `read_only`, `fallback_used`, and
  `native_write_mode`. For the Node binding route, include `data_dir` as a
  redacted or basename-only value, never a secret-bearing path dump.
- Native MCP `tools/list` must never expose top-level `anyOf`, `oneOf`, or
  `allOf`. Runtime aliases belong in handler code.
- Read health and write health are separate. A read-only native tool list can be
  green while write-mode tools are intentionally hidden.

## Plugin Tool Matrix

| Plugin tool | Current route in `server.mjs` | Target class | Target route | Adapter or follow-up |
|---|---|---|---|---|
| `harness_route` | Local heuristic only | product-http | local plugin router | Keep local. Add route receipt once route policy exists. |
| `orchestrate_refresh` | Theseus `/orchestrate/prepare/` | product-http | product/context HTTP until native context lane lands | Do not silently route to graph MCP. This compiles prompt context, not graph recall. |
| `harness_replay` | Theseus `/harness/runs/{id}/events/` and `/state-hash/` | sdk-surface | Node binding `eventsJson` / `pollText` / `runStatus`, or legacy HTTP compat fallback | First handler wiring landed: native binding path executes when `THEOREM_HARNESS_DATA_DIR` and `THEOREM_HARNESS_NODE_BINDING_PATH` are configured; compat fallback stays explicit. Native MCP remote/shared replay remains a later wrapper. |
| `harness_describe_current` | Local disk state | product-http | local plugin state | Keep local. It describes injected artifact state, not substrate state. |
| `context_compile` | Theseus `/context/compile/` | product-http | product/context HTTP | Preserve under a precise context name. Add native `harness_context` later for graph-backed packs. |
| `code_search` | Theseus `/code/symbols/` | native-adapter | native MCP `code_search` -> product backend gRPC hook -> `theorem_grpc.code_search.*` app affordance route | Native ingestion/search/recognize/explore/context/explain/use receipts now exist; deployment must set `THEOREM_APP_AFFORDANCE_GRPC_URL` or `THEOREM_GRPC_URL` before removing legacy fallback. |
| `code_crawl` | Theseus `/code/ingest/` | theseus-engine | explicit Theseus/code ingest fallback | Heavy ingest path. Do not hide under base harness state. |
| `harness_fractal_expansion` | Theseus `/harness/runs/{id}/fractal-expansion/` | native-adapter | native graph tools: `harness_kg_search`, `harness_kg_ppr`, `harness_kg_related_objects`, plus SDK run wrapper | Needs a native wrapper that records run events through the SDK surface. |
| `fractal_expand` | Alias for `harness_fractal_expansion` | native-adapter | same as `harness_fractal_expansion` | Keep as alias if launch-facing naming is still useful. |
| `instant_kg_status` | THG product `/instant-kg/status` | native-now | `harness_kg_status` | Schema adapter from product manifest/delta body to native arguments. |
| `instant_kg_reingest` | Theseus `/capture/instant-kg/` | product-http | product ingest until native ingest route exists | This is ingest/capture, not simple graph read. |
| `self_note` | Theseus `/harness/memory/self-note/` plus mirror | native-now | native MCP `self_note` or Node binding `remember` for private local session notes | First wiring landed: shared default preserves explicit legacy compat; `scope: private` executes through the Node binding as a `self_note` memory document and does not silently fall back. |
| `self_revise` | Theseus `/harness/memory/self-revise/` plus mirror | native-now | `self_revise` | Local schema fix removed top-level `anyOf`; runtime keeps `doc_id` and `docId` aliases. |
| `self_archive` | Theseus `/harness/memory/self-archive/` plus mirror | native-now | `self_archive` | Local schema fix removed top-level `anyOf`; runtime keeps aliases. |
| `self_recall_archive` | Theseus `/harness/memory/self-recall-archive/` | native-now | `self_recall_archive` | Route to native archive recall. |
| `recall` | Product saved-context preview recall | rename-or-fold | native MCP `recall` or Node binding `recall` | First wiring landed: `query` now means native memory recall and defaults to private Node binding when configured; old `tenant_slug` plus `task` calls remain saved-context preview compatibility with a product route receipt. |
| `remember` | Theseus `/harness/memory/self-note/` plus mirror | native-now | native MCP `remember` or Node binding `remember` | First wiring landed: shared default preserves explicit legacy compat; `scope: private` executes through the Node binding and does not silently fall back. Add idempotency token generation next. |
| `relate` | Theseus `/harness/thg/command/` edge upsert | native-now | `relate` | Native schema now uses canonical `seed_id` required field while keeping runtime aliases. Confirm plugin argument mapping from `from_id`/`to_id`. |
| `encode` | Theseus `/harness/encode/` plus mirror | native-now | `encode` | Next memory-family route. Must use native `encode_memory` / MCP adapter rather than generic Node `remember`, because outcome, signal, fitness, and training metadata must survive. |
| `coordination_intent` | Theseus `/harness/coordination/intent/` plus mirror | native-now | native MCP `coordination_intent` | Native tool exists in write mode. Keep coordination on shared native MCP, not a per-plugin local binding, so heads see the same room. |
| `coordination_reflection` | Theseus `/harness/coordination/reflection/` plus mirror | native-adapter | `coordination_record(record_type=reflection)` | Plugin verb should become a typed adapter over native record writes. |
| `coordination_decision` | Theseus `/harness/coordination/decision/` plus mirror | native-adapter | `coordination_record(record_type=decision)` | Preserve title/choice/rationale in summary/body metadata. |
| `coordination_tension` | Theseus `/harness/coordination/tension/` plus mirror | native-adapter | `coordination_record(record_type=tension)` now; `resolve_tension` later | Native durable record exists. Dedicated resolve verb remains a follow-up. |
| `coordinate` | Theseus `/harness/coordinate/` plus mirror | native-now | native MCP `coordinate` | Native direct message and mention queue work. Keep this shared/remote by default. |
| `mentions` | Theseus `/harness/mentions/` | native-now | `mentions` | Native read/consume works, but consume requires write mode. |
| `mentions_wait` | Theseus `/harness/mentions/wait/` | native-adapter | add native `mentions_wait` | Not native yet. Keep short polling caps and avoid host-thread lockups. |
| `presence` | Theseus `/harness/presence/` plus mirror | native-now | `presence` | Native write mode supports heartbeat/get/end. |
| `coordination_room` | Theseus `/harness/coordination/room/` plus mirror | native-now | `coordination_room` | Native start/join/status works. |
| `subscribe` | Theseus `/harness/subscribe/` plus mirror | rename-or-fold | likely `mentions`/room context or product subscription | The gossip protocol no longer needs subscription as the main path. Decide whether any product UI still uses it. |
| `continuity_pack` | Theseus `/harness/session/continuity-pack/` plus mirror | native-adapter | native record/reflection plus product continuity pack | Split coordination reflection from larger product compaction artifact. |
| `provenance_trace` | Product trace HTTP | product-http | product trace HTTP | Keep product trace reads until THPS-013 trace export lands. |
| `product_bootstrap` | Product HTTP `/product/bootstrap/` | product-http | product HTTP | Keep. This is product bootstrap, not base harness memory. |
| `saved_contexts_list` | Product saved-context HTTP | product-http | product HTTP | Keep as product context surface. |
| `saved_context_create` | Product saved-context HTTP | product-http | product HTTP | Keep. Do not mix with native memory recall. |
| `saved_context_update` | Product saved-context HTTP | product-http | product HTTP | Keep. |
| `saved_context_mute` | Product saved-context HTTP | product-http | product HTTP | Keep. |
| `saved_context_activate` | Product saved-context HTTP | product-http | product HTTP | Keep. |
| `saved_context_delete` | Product saved-context HTTP | product-http | product HTTP | Keep. |
| `saved_context_preview_recall` | Product saved-context preview recall | product-http | product HTTP | Keep or rename to `context_preview`; this is not memory `recall`. |
| `memory_patch_review_queue` | Product memory-patch review HTTP | product-http | product HTTP | Keep as review workflow. |
| `memory_patch_review_update` | Product memory-patch review HTTP | product-http | product HTTP | Keep as review workflow. |
| `domain_list` | Product pack API | product-http | product HTTP | Keep. |
| `domain_install` | Product pack API | product-http | product HTTP | Keep. |

## Native-Only Tools Worth Exposing Through The Plugin Later

| Native tool | Why it matters | Proposed plugin surface |
|---|---|---|
| `coordination_context` | One-call room packet for turn-start injection | Add plugin `coordination_context` or use internally in hooks. |
| `read_intents_for_room` | Cold-start lane and claim inspection | Use internally in `harness-coordinate`; optional expert tool. |
| `read_messages_for_room` | Room history without consuming mentions | Use in coordination diagnostics. |
| `read_records_for_room` | Decisions, events, tensions, reflections | Use for session catchup and compact handoff. |
| `coordination_record` | Unified event/decision/tension/reflection write surface | Back existing typed plugin verbs. |
| `coordination_contribution` | Compact contribution receipt | Add to end-of-slice reporting. |
| `harness_run` | Native run detail and state | Back remote/shared `harness_replay` and future `harness_state`. |
| `harness_append_transition` | Low-level run event append | Keep internal; SDK wrappers and Node binding should hide event names for ordinary users. |
| `observe` | Native memory/tenant observation | Useful for diagnostics and smoke tests. |
| `forget` | Native memory deletion/forget path | Add plugin tool or fold into memory maintenance commands. |
| `handoff` | Native cross-actor memory handoff | Add plugin tool if not already represented through coordination. |
| `rustyred_thg_graph_schema` | Cheap native graph context | Use in context/graph diagnostics. |
| `rustyred_thg_graph_query` | Native graph query | Expert/debug tool, not default user path. |
| `rustyred_thg_algorithm_ppr` | Native PPR over tenant graph | Back graph discovery and fractal expansion. |
| `rustyred_thg_algorithm_*_inline` | Inline algorithms for small graphs | Useful for tests and local reasoning without tenant writes. |
| `harness_kg_*` | Instant KG read/search/PPR/impact tools | Back research/fractal-expansion routes. |

## First Implementation Order

1. Add route policy scaffolding with fake transports. Do not change production
   defaults yet.
2. Add a `NativeBindingClient` adapter over `apps/theorem-harness-node` for
   local durable runs and private session memory.
3. Route coordination verbs through native MCP: room, intent, record adapters,
   coordinate, mentions, presence. This must stay shared substrate state.
4. Route memory verbs through policy: native MCP for shared room/tenant memory,
   Node binding for local private session memory, never silent fallback. First
   slice landed for `remember` / `recall` / `self_note`; shared memory still
   has explicit legacy compat until the native MCP adapter is wired.
5. Keep saved-context preview under `saved_context_preview_recall`; maintain
   old `recall(tenant_slug, task)` as a compatibility branch during migration.
6. Route graph/status reads: `instant_kg_status` to `harness_kg_status`, then
   `harness_fractal_expansion` through native graph tools plus SDK run events.
7. Keep product HTTP and Theseus-engine routes explicit with receipts.

## Validation Targets

- `node --check /Users/travisgilbert/Tech Dev Local/codex-plugins/theorems-harness/mcp/server.mjs`
- Native MCP `tools/list` in read-only mode has no top-level schema combinators.
- Native MCP `tools/list` in write mode has no top-level schema combinators.
- `cargo test -p rustyred-thg-mcp`
- `node sdk/route-policy.test.mjs` from the plugin source repo
- Real native-binding replay smoke through `HarnessRoutePolicy.execute` against
  `apps/theorem-harness-node/theorem_harness_node.node`
- Real native-binding memory smoke through `HarnessRoutePolicy.execute`:
  `remember(scope=private)` / `self_note(scope=private)` then
  `recall(scope=private)`
- `node apps/theorem-harness-node/smoke.mjs`
- Plugin fake transport tests for memory, coordination, product HTTP, and
  Theseus-engine fallback route classes.

## Open Questions

- Should the saved-context preview verb keep the existing
  `saved_context_preview_recall` name or become shorter `context_preview`?
- Should `subscribe` survive as a product UI helper, or should agent-facing docs
  remove it in favor of room context plus mentions?
- Does `relate` keep plugin `from_id`/`to_id` arguments, or should the plugin
  adopt native `seed_id` plus target aliases for the public surface?
- Which shared-memory adapter should land first: direct native MCP client inside
  the plugin router, or a product HTTP bridge backed by the same runtime?
