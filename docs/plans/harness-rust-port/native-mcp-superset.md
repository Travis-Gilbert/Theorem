# Native MCP Harness Superset (route all harness-base tools through Theorems-Harness V2)

Date: 2026-06-01
Author: Claude Code (planning solo; Codex executes)
Status: PLAN. Not started. Sits as a new slice on top of the live
`harness-rust-port` tree. Does not modify `implementation-plan.md` or
`CLAIMS.md` (Codex-claimed files); Codex adds the claim rows when it picks
this up.

Coordination note: the Python coordination MCP (`theseus-mcp-production`) was
not responding when this plan was written (`mentions` / `presence` returned
"connector's server isn't responding"). That is the exact failure this plan
removes. Durable coordination channel for this slice is git history +
path-scoped commit messages + the `CLAIMS.md` table, per the port's hybrid
protocol.

## Goal (the user's words, made precise)

> "Route all harness base MCP tools through the Theorem rustyred MCP. It should
> be a superset of the harness skills when they were present on Theseus. No
> Python or Django deps. Capture the write tools; many of the read tools are
> already there."

Target server: `https://rustyredcore-theorem-production.up.railway.app/mcp`
(the deployed `rustyred-thg-mcp` crate, surfaced to the agent as
**Theorems-Harness V2**).

Definition of done (the floor, not the ceiling): every verb the Theorem's
Harness skill teaches as a *base* tool (coordination + memory + run-lifecycle +
research/expansion) is served natively from `rustyred-thg-mcp`, over the
`theorem-harness-runtime` + `GraphStore` substrate, **with no Python/Django in
the request path**, and the **write** verbs are live (not just the reads).
Every base verb below gets a checklist item or an explicitly-surfaced,
consent-required deferral. No silent MVP cut.

### Confirmed target state (Travis, 2026-06-01)

V2 (`rustyredcore-theorem-production`) must serve, all native over the RedCore
`GraphStore`, no Python/Django: **graph + coordination reads + coordination
writes + memory.** Those four pillars are the definition of done for the base:

- graph: already live.
- coordination reads: already live (`coordination_context`, `mentions`,
  `read_*`).
- coordination writes: coded in `225f3e2`; blocked only on deploy + write-mode
  (Lane O), with completeness gaps in Lane C (`mentions_wait`,
  `resolve_tension`, typed `read_*_since`).
- memory: the build (Lane M) — memory atoms become RedCore `GraphStore` nodes
  on this same server.

Travis also accepted the broader `theorem_*` product surface (Lane T) on top of
that base. Verified live on 2026-06-01: V2 exposes 40 tools, all reads (graph +
coordination reads); zero memory verbs and zero coordination writes are live
yet. That probe is the before-state this plan moves off.

## Why this is mostly an extension, not a greenfield build

Grounded in the live tree on 2026-06-01 (`git show 225f3e2`, the MCP
`tools/list`, and the runtime sources), the port is already deep:

- `theorem-harness-core` ports the pure spine: `state_machine`, `replay`,
  `toolgraph`, `context_web`, `affordances`, `session_metrics`,
  `federated_signals`, `map_artifacts`, and **`memory_contracts`** (the
  recall-*preview* preparation contract only: hydration handles, banks, recall
  policy, evidence/policy typing).
- `theorem-harness-runtime` has `coordination` (room / intent / presence /
  message / mention / record) and `event_log` (run / transition / replay),
  both `GraphStore`-backed with `RedCoreGraphStore` reopen tests.
- `rustyred-thg-mcp` already registers, gated behind write mode:
  `coordinate`, `coordination_room`, `coordination_intent`,
  `coordination_record`, `coordination_contribution`, `coordination_context`,
  `presence`, `mentions`, `subscribe`, `read_intents_for_room`,
  `read_messages_for_room`, `read_records_for_room`, `harness_append_transition`,
  `harness_run`. Plus the read graph surface: `rustyred_thg_*`, `harness_kg_*`.

So the **coordination write layer is done in code** (commit `225f3e2`,
2026-06-01 12:14). What is missing splits into one real code gap and one ops
gap:

1. **Memory substrate (the gap the user named).** There is no native
   `recall` / `remember` / `relate` / `self_note` / `self_revise` /
   `self_archive` / `self_recall_archive` / `encode` / `forget` / `observe` /
   `handoff`. The runtime has no `memory` module. The typed *preparation*
   contract exists in core; the durable atom store + cross-surface recall do
   not.
2. **Run-lifecycle ergonomic verbs.** The core logic exists (`state_machine`,
   `replay`, `toolgraph`, `context_web`), and the MCP exposes the generic
   `harness_append_transition` + `harness_run`, but the discrete verbs the skill
   teaches (`harness_begin` / `harness_step` / `harness_search` /
   `harness_context` / `harness_patch` / `harness_replay` / `harness_fork` /
   `harness_compare` / `harness_toolkit` / `harness_fractal_expansion`) are not
   individually surfaced.
3. **A few coordination verbs** the skill teaches are absent: `mentions_wait`
   (the blocking long-poll), `resolve_tension`, `next`, `orchestrate_map`,
   `ppr_neighborhood`, and the typed `read_*_since` reads.
4. **Ops.** The deployed binary predates `225f3e2` (live `tools/list` shows only
   `rustyred_thg_*` reads), and the server defaults to `read_only: true`
   (`rustyred-thg-mcp/src/lib.rs:304`, env `THG_MCP_READ_ONLY` /
   `RUSTY_RED_MCP_READ_ONLY`). Even after deploy, writes stay hidden until write
   mode is enabled with auth.

## Canonical contract source

The Python verbs this supersets live in
`Index-API/mcp_server/tools/verbs.py` (kitchen-sink theseus-mcp) and are
re-registered in `Index-API/mcp_server_theorem/tools/workspace.py`
(theorem-mcp). Line references below are into `verbs.py` at the state read on
2026-06-01. Storage helpers (`create_document_entry`, `serialize_document`,
`revise_document_entry`, `archive_document_entry`, `recall_archived_entries`,
`create_memory_node`, `serialize_memory_node`, `recall_items`,
`related_items`, `encode_memory_entry`) are the behavior to reproduce in Rust.

The Python helpers reach Django ORM + Memgraph. The native port reproduces
their *contract*, not their implementation: memory atoms become `GraphStore`
nodes/edges; recall composes the existing native `rustyred_thg_fulltext_search`
+ `harness_kg_*` reads. This is the parity discipline already used for the
state-machine and toolgraph ports.

## Full base-verb superset, classified

| Verb | Kind | Native status | Lane / item |
|---|---|---|---|
| `coordinate` | write | DONE (`225f3e2`) | ops only (O*) |
| `coordination_room` | read | DONE | ops only |
| `coordination_intent` | write | DONE | ops only |
| `coordination_record` | write | DONE | ops only |
| `coordination_contribution` | write | DONE | ops only |
| `coordination_context` | read | DONE | ops only |
| `presence` | write | DONE | ops only |
| `mentions` | read/consume | DONE | ops only |
| `subscribe` | write | DONE | ops only |
| `read_intents_for_room` | read | DONE | ops only |
| `read_messages_for_room` | read | DONE | ops only |
| `read_records_for_room` | read | DONE | ops only |
| `harness_append_transition` | write | DONE | ops only |
| `harness_run` | read | DONE | ops only |
| `ppr_neighborhood` | read | native primitive exists (`rustyred_thg_algorithm_ppr`, `harness_kg_ppr`); needs the named alias | C11 |
| `remember` | write | GAP | M1 |
| `recall` | read | GAP | M2 |
| `relate` | read | GAP | M3 |
| `self_note` | write | GAP | M4 |
| `self_revise` | write | GAP | M5 |
| `self_archive` | write | GAP | M6 |
| `self_recall_archive` | read | GAP | M7 |
| `encode` | write | GAP | M8 |
| `forget` | write | GAP | M9 |
| `observe` | read (composite) | GAP | M10 |
| `handoff` | write | GAP | M11 |
| `harness_begin` | write | core exists; not surfaced | R1 |
| `harness_step` | write | covered generically by `harness_append_transition`; needs ergonomic verb | R2 |
| `harness_search` | write | GAP (records search step + observation) | R3 |
| `harness_context` | read/compile | core (`context_web`) exists; not surfaced | R4 |
| `harness_patch` | write (review-gated) | GAP | R5 |
| `harness_replay` | read | covered by `harness_run`; needs named verb | R6 |
| `harness_fork` | write | core (`replay::fork_*`) exists; not surfaced | R7 |
| `harness_compare` | read | core (`replay::compare_*`) exists; not surfaced | R8 |
| `harness_toolkit` | read/compile | core (`toolgraph`) exists; not surfaced | R9 |
| `harness_fractal_expansion` | read/write | composes native PPR; not surfaced | R10 |
| `code_search` | read | native MCP routes to `theorem_grpc.code_search.*` app affordances through the product backend gRPC hook | R11 |
| `mentions_wait` | read (long-poll) | GAP | C1 |
| `resolve_tension` | write | GAP (records have `tension` type; no resolve verb) | C4 |
| `next` | read (action rail) | GAP | C3 |
| `orchestrate_map` | read/compile | core (`map_artifacts`) exists; not surfaced | C10 |
| `read_decisions_since` | read | typed read over records | C5 |
| `read_events_since` | read | typed read over records | C6 |
| `read_open_tensions` | read | typed read over records | C7 |
| `read_reflections_for_room` | read | typed read over records | C8 |
| `read_validation_receipts_for_room` | read | typed read over records | C9 |

## Lane M: memory substrate (the centerpiece)

New module `theorem-harness-runtime::memory` (GraphStore-backed) + exposure in
`rustyred-thg-mcp`. Pure Rust; no Django. Reuses the existing native fulltext +
graph reads for the recall index.

### Storage design (GraphStore nodes + edges)

- `MemoryDocument` node, id `mem:doc:{tenant}:{doc_id}`. Fields: `doc_id`
  (stable slug), `kind`, `title`, `content`, `summary`, `tags[]`, `links[]`,
  `actor_id`, `session_id`, `origin_surface`, `status`
  (`active|superseded|archived|deleted`), `created_at`, `updated_at`,
  `memory_node_type` (for `self_note` typed memory), `target_actor_id` +
  `expires_at` (for `handoff`), `fitness` (`{outcome, signal, score}` for
  `encode`), `metadata{}`.
- `MemoryNode` node, id `mem:node:{tenant}:{node_id}` for `NODE_MEMORY_KINDS`
  (`claim`, `finding`) per `verbs.py:825` `create_memory_node`.
- Edges: `MEMORY_SUPERSEDES` (revise chain), `MEMORY_CITES`,
  `MEMORY_DERIVED_FROM`, `MEMORY_ARCHIVED_AS`, `MEMORY_HANDOFF_TO`,
  `MEMORY_RELATES`.
- Recall index: at write time `rustyred_thg_fulltext_designate` the memory
  node's content/title/tags; `recall` queries via the existing native fulltext
  search + graph neighbors, then post-filters by actor/surface/kind/since/status
  and attaches provenance (`actor`, `surface`, `session`). `relate` walks
  `harness_kg_related_objects` / graph neighbors from the seed id.

This is the elegant reuse: recall is "designate memory atoms as fulltext, then
compose the native search the server already ships." No new index engine.

### Checklist (each item backreferences the canonical contract)

- [x] **M0** Create `theorem-harness-runtime/src/memory.rs`; declare node-id /
  edge-id helpers, `MemoryError`, and the typed input/result structs. Wire into
  `lib.rs` exports. (Mirrors the `coordination.rs` shape.)
- [x] **M1 `remember`** — kind-routed write. `claim`/`finding` -> `MemoryNode`;
  document kinds -> `MemoryDocument`. Inputs: `kind, content, title, tags,
  links, project_slug`. Output: `{saved_type, node|document}`.
  Ref `verbs.py:820`.
- [x] **M2 `recall`** — native retrieval over memory documents + memory nodes;
  run/context-artifact recall stays in Lane T/context IO. Inputs: `query, actor, since, kind, surface,
  limit, include_low_fitness, include_consolidation_sources`; `kind=="handoff"`
  consumes handoffs. Output: `{results[], count}`, each result carries
  `actor/surface/session` provenance. Ref `verbs.py:665`.
- [x] **M3 `relate`** — connected things for a seed doc/node. Inputs: `seed_id,
  edge_types, max_hops`. Output: `{seed_id, results[], count}`.
  Ref `verbs.py:861`. Implemented over native memory graph neighbor edges.
- [x] **M4 `self_note`** — typed agent-memory document with
  `metadata.memory_node_type` + `source="self_note"`. Inputs: `content, kind,
  title, tags, links, memory_node_type, summary`. Ref `verbs.py:925`.
- [x] **M5 `self_revise`** — revision-tracked replacement; new active atom,
  prior atom -> `superseded`, `MEMORY_SUPERSEDES` edge, plus `MEMORY_CITES` /
  `MEMORY_DERIVED_FROM` edges. Inputs: `doc_id, content, title, summary, reason,
  memory_node_type, cites_doc_ids, derived_from_doc_ids`. Output:
  `{revised, superseded}`. Ref `verbs.py:954`.
- [x] **M6 `self_archive`** — move atom to cold tier; status -> `archived`,
  `MEMORY_ARCHIVED_AS` edge to an archive record. Inputs: `doc_id, reason,
  title`. Output `{archived, archive}`. Ref `verbs.py:983`.
- [x] **M7 `self_recall_archive`** — query the archived tier only; archived
  atoms must NOT appear in M2 default recall. Inputs: `query, actor, limit`.
  Ref `verbs.py:1007`.
- [x] **M8 `encode`** — feedback/solution/postmortem with `outcome` + fitness
  `signal` + optional `event_id` linkage. Inputs: `content, title, kind,
  outcome, signal, reason, event_id, tags, links, summary, metadata, context,
  auto_triggered`. Writes `fitness` onto the atom and stores `event_id` in that
  fitness envelope; a direct event edge is a follow-up. Ref `verbs.py:1034` /
  `encode_memory_entry`.
- [x] **M9 `forget`** — soft-delete a document OR memory node by `id`, with an
  audit `reason`. Inputs: `id, reason`. Output `{forgotten_type:
  document|node, document|node}`. Set status -> `deleted` (+ `deleted_reason` /
  `deleted_at` provenance), remove from recall by status filtering, preserve
  audit history. Fulltext de-designation is a follow-up once the deployed memory
  index is designated. Ref `verbs.py:1727`.
- [x] **M10 `observe`** — composite read: actor/tenant identity + coordination
  room status + pending mentions + latest continuity pack + active orchestrate
  notes + optional `recall` for a query. No writes, no consume. Inputs: `actor,
  room_id, query, limit, include_low_fitness, include_consolidation_sources`.
  Ref `verbs.py:699`. Composes existing native `room_status` + `mentions` (read)
  + M2.
- [x] **M11 `handoff`** — cross-surface handoff as a pending `handoff`-kind
  document targeted at `to_actor` with `expires_at`; consumed by M2 with
  `kind="handoff"`. Inputs: `to_actor, payload, title, expires_in`. Output
  `{handoff}`. Ref `verbs.py:895`.
- [ ] **M12 (in-base extension) document CRUD verbs** over the same
  `MemoryDocument` store: `theorem_document_read` / `_write` / `_search` /
  `_history` / `_link`. The skill's output discipline references document kinds
  (`scratch`, `markdown`, `insight`, `handoff`, `solution`, `postmortem`); the
  store lands them in M1/M4, so exposing read/history/link is incremental.
  Ref `verbs.py:646` (`theorem_document_history`) + the document helpers.

Implementation note (Codex, 2026-06-01): M0-M11 are now implemented in code
over `theorem-harness-runtime::memory` and exposed through `rustyred-thg-mcp`
with read-only/write-mode gating. Runtime tests cover document/node recall,
revision, archive, handoff consume, relate, encode, forget, and RedCore reopen
persistence. MCP tests cover tool listing plus a full JSON-RPC memory
round-trip. Remaining before Lane M can be called deployment-complete: P1
parity-memory fixtures, the clippy/acceptance gates, and Lane O live deploy
with bearer-auth write mode.

## Lane R: run-lifecycle ergonomic verbs

Surface the discrete verbs the skill teaches over the existing core +
`event_log`. These are thin MCP wrappers around logic that already exists; the
work is the tool schema, the run-id threading, and the read/write gating.

- [ ] **R1 `harness_begin`** — open a run (task, actor, scope); returns
  `run_id`. Persists the `RUN.CREATED` transition through `event_log`.
- [ ] **R2 `harness_step`** — record a `tool_call` / `observation` / `decision`
  step. Ergonomic wrapper over `harness_append_transition`.
- [ ] **R3 `harness_search`** — run-scoped native search that records a
  `tool_call` + `observation` pair into the run (composes
  `rustyred_thg_fulltext_search` / `harness_kg_search`).
- [ ] **R4 `harness_context`** — compile the context artifact for a run via
  `theorem_harness_core::context_web` (`ContextWebPack::bounded`,
  `ContextWebBudget::capped_for_mode`).
- [ ] **R5 `harness_patch`** — review-gated belief-state patch proposal. Records
  the proposal; does not auto-apply (the human is the reviewer).
- [ ] **R6 `harness_replay`** — full event timeline. Named verb over the same
  contract `harness_run` returns (`{run, events}`).
- [ ] **R7 `harness_fork`** — branch a run through a step id via
  `theorem_harness_core::replay::fork_*`; opens a new run id.
- [ ] **R8 `harness_compare`** — state-hash diff + evidence overlap + divergence
  point via `replay::compare_*`.
- [ ] **R9 `harness_toolkit`** — compile/inspect the task toolkit from
  `task_type` + `permissions` + `scope` via `toolgraph::compile_task_toolkit`.
- [ ] **R10 `harness_fractal_expansion`** — query-driven gap-frontier search;
  optional `run_id` records it into the run. Composes the native PPR
  (`rustyred_thg_algorithm_ppr` / `harness_kg_ppr`); no Python `push_ppr`
  fallback (the deploy log `FALLBACK: Python push_ppr running` becomes
  structurally impossible because there is no Python in the path).
- [ ] **R11 `code_search`** — search ingested code symbols. First native lane
  now exists in `apps/theorem-grpc`: `CodeCrawlerService` can ingest/reindex a
  repo into a RedCore code graph, search symbols, recognize symbols from indexed
  files or inline source, expand Rust AST-backed call/dependency edges, read
  context, explain symbols with trust tiers, and record search/use receipts; the
  same runtime is exposed as `theorem_grpc.code_search.*` affordances. The named
  MCP `code_search` harness verb now routes through the product backend's live
  app-affordance gRPC hook to those ids. Remaining R11 work: configure the
  deployment endpoint env and add parser-grade parity for non-Rust languages
  where needed.

## Lane C: coordination completeness

- [ ] **C1 `mentions_wait`** — short blocking long-poll (default 30s, cap 120s)
  over the native mention inbox; returns when a pending mention arrives or the
  timeout expires. Single-request long-poll, not a permanent listener.
- [ ] **C3 `next`** — action-rail "what next" read for an actor/run.
- [ ] **C4 `resolve_tension`** — write that resolves a `tension` coordination
  record (status -> resolved, resolution note). Pairs with existing
  `coordination_record` writes and C7 reads.
- [ ] **C5 `read_decisions_since`** — typed read over `decision` records since a
  cursor.
- [ ] **C6 `read_events_since`** — typed read over `event` records since a
  cursor.
- [ ] **C7 `read_open_tensions`** — typed read over open `tension` records.
- [ ] **C8 `read_reflections_for_room`** — typed read over `reflection`
  records.
- [ ] **C9 `read_validation_receipts_for_room`** — typed read over validation
  receipts (the policy-receipt / contribution validation surface).
- [ ] **C10 `orchestrate_map`** — read/compile MapArtifact via
  `theorem_harness_core::map_artifacts` (`compile_map_artifact`,
  `describe_map_artifact`).
- [ ] **C11 `ppr_neighborhood`** — named alias for seed-PK PPR over the native
  algorithm (`rustyred_thg_algorithm_ppr`), matching the skill's documented
  `ppr_neighborhood` verb.

C5-C9 are typed projections over the existing `read_records_for_room` +
record-type filter; the work is the named verbs and `since`/cursor semantics,
not a new store.

## Lane T: theorem_* product surface (accepted scope, Travis 2026-06-01)

The broader Theorem product verbs, ported native over the same RedCore
`GraphStore` substrate as Lane M. **Naming: register Form-B (drop the
`theorem_` prefix)** to match the existing native convention
(`harness_begin`, not `theorem_harness_begin`); the MCP framework already
namespaces as `mcp__<server>__<tool>`. Most of these sit directly on the Lane M
atom store + the existing `context_web` / `event_log` / `map_artifacts` /
`replay` core, so they are incremental once Lane M lands.

Clean GraphStore ports (no Python):

- [ ] **T1-T5 document CRUD** — `document_read` / `document_write` /
  `document_search` / `document_history` / `document_link` over the Lane M
  `MemoryDocument` store. Supersedes M12 (fold M12 here).
- [ ] **T6-T13 context IO** — `context_compile` / `context_recall` /
  `context_remember` / `context_audit` / `search_context` / `hydrate_context` /
  `enrich_context` / `explain_context` over `theorem_harness_core::context_web`
  + the Lane M store. This is the full context IO retrieval the base-plan body
  listed as later runtime work; it lands here.
- [ ] **T14-T17 memory lifecycle** — `memory` (read a memory item),
  `memory_promote`, `memory_signal` (fitness signal; ref `verbs.py:1774`),
  `review_memory` over the Lane M atoms + `fitness` field.
- [ ] **T18-T19** — `consolidate`, `reflect` as durable records over the
  coordination record store + Lane M.
- [ ] **T20-T22 trajectory** — `record_step`, `record_outcome`,
  `record_trajectory` over `event_log`.
- [ ] **T23-T25 expansion** — `expand`, `explore_neighborhood`,
  `fractal_expansion` over native PPR (share impl with R10; do not duplicate).
- [ ] **T26-T27 artifacts** — `export_artifact`, `compiler_artifact_search`
  over the graph + content-addressed packs (`rustyred_thg_graph_version_compile`
  already exists).
- [ ] **T28** — `tension_resolve`: alias/shared impl with C4 `resolve_tension`.
- [ ] **T29-T30 graph passthrough** — `thg_command`, `thg_cypher` over the
  native graph ops.
- [ ] **T31** — `check`: health/consistency read.
- [ ] **T32** — `prepare_agent`: agent context prep over
  `theorem_harness_core::memory_contracts` (the prep contract already exists).
- [ ] **T33-T34 session** — `session_offload`, `session_recall` over the store.

Open sub-question (the only non-GraphStore part):

- [ ] **T35 `runner_launch_claude_code` / T36 `runner_fetch_session`** — these
  spawn and fetch a Claude Code agent **process**, not graph state. "Native"
  here means a Rust runner service (extend `apps/theorem-harness-server`) that
  owns process spawn, NOT the MCP graph layer. **Sub-decision D4:** keep
  T35/T36 on the existing Python runner, or build a Rust runner. Default
  proposal: keep on Python runner short-term (process orchestration is the one
  place Python is not a reliability liability), expose T35/T36 as thin proxies
  until a Rust runner exists. Confirm.

## Lane O: ops (deploy + write mode + auth)

This lane is what makes the user's observation ("the deployed server doesn't
have coordinate/presence") go away. It is sequenced AFTER M/R/C land so one
deploy carries the full superset.

- [ ] **O1** Build + deploy the current `rustyred-thg-mcp` (>= `225f3e2` + the
  new lanes) to `rustyredcore-theorem-production` (Railway). Confirm the
  binary is newer than the live one (live `tools/list` currently shows only
  `rustyred_thg_*` reads).
- [ ] **O2** Enable write mode: set `THG_MCP_READ_ONLY=false` (alias
  `RUSTY_RED_MCP_READ_ONLY=false`) on the Railway service so the write verbs are
  listed and callable.
- [ ] **O3** Gate write mode behind bearer auth + tenant scoping on the public
  URL. Reuse the existing `RUSTY_RED_API_TOKENS` / `RUSTYRED_THG_API_TOKENS`
  bearer convention (NOT the dead `THG_API_TOKENS` name) and
  `RUSTY_RED_ALLOWED_ORIGINS`. A public, unauthenticated write endpoint is not
  acceptable; writes must attach a tenant.
- [ ] **O4** Verify: `tools/list` on the deployed URL shows the full superset;
  curl-smoke one write per lane (`remember`, `coordinate`, `harness_begin`) and
  one read (`recall`) end-to-end against a scratch tenant.
- [ ] **O5** Point the MCP server and `apps/theorem-harness-server` at the same
  RedCore tenant dir (`THEOREM_HARNESS_DATA_DIR=$RUSTY_RED_DATA_DIR/tenants/<tenant>`)
  so iOS/web read surfaces see MCP-written memory + coordination.

## Lane P: parity + acceptance + truth-in-docs

- [ ] **P1** Generate a memory fixture corpus from the Python verbs
  (`Index-API/mcp_server/tools/verbs.py` + the storage helpers) under
  `docs/plans/harness-rust-port/parity-memory/`: for `remember`/`recall`/
  `self_revise`/`self_archive`/`encode`/`forget`, record inputs and the
  resulting atom shape + recall ordering + status transitions. Mirror the
  existing `parity/`, `parity-toolgraph/`, `parity-context/` discipline.
- [ ] **P2** `cargo test -p theorem-harness-runtime` green (memory module:
  write/recall/revise/archive/forget/relate, fulltext designation, status
  filtering, `RedCoreGraphStore` reopen).
- [ ] **P3** `cargo test -p rustyred-thg-mcp` green (memory + run-lifecycle +
  coordination-completeness MCP round trips, read-only/write-mode gating per
  verb).
- [ ] **P4** `cargo clippy -p theorem-harness-runtime --all-targets --no-deps
  -- -D warnings` and `cargo clippy -p rustyred-thg-mcp --all-targets --no-deps
  -- -D warnings` clean.
- [ ] **P5** Make the harness skill docs true, do NOT repoint them at Python.
  The `theorem-harness`/`theorems-harness` skill already claims memory lives on
  the native server ("recall/remember against it"). That claim is *premature*
  today (memory actually executes on Python `theseus-mcp`), but it is the goal:
  memory lives in rustyred. Sequencing P5 AFTER Lane M + Lane O is what makes
  the existing claim true. Then sharpen the doc to: Theorems-Harness V2
  (`rustyredcore-theorem-production`) is the single native home for the harness
  base + product surface incl. **memory + coordination writes** over the RedCore
  GraphStore; Python `theseus-mcp` remains only for the heavy `theseus_*` engine
  (code agent, frontend check, epistemic/scorer/Modal). Do not write "memory is
  on Python" into the doc; land the port so "memory is native" stops being a
  lie.

## Proposed deferrals (consent required, surfaced individually)

These are NOT silent cuts. Each needs an explicit yes/no before it is treated
as out of scope:

- **D1 — `theseus_*` engine verbs** (`theseus_code_agent`,
  `theseus_frontend_check`, `theseus_ask_and_visualize`, the epistemic engine,
  scorer/IQ/training, Modal dispatch). Justification: these are the heavy
  Theseus *engine*, not the harness base; they legitimately stay on the Python
  `theseus-mcp` (they invoke PyTorch / spaCy / Modal). Proposed: keep on Python.
- **D2 — ACCEPTED INTO SCOPE (Travis, 2026-06-01).** The broader `theorem_*`
  product surface (document CRUD, context IO, memory lifecycle/signals,
  consolidate/reflect, trajectory recording, expansion, artifacts, session) is
  now in scope as **Lane T** below. Only the `theorem_runner_*` process-spawn
  verbs carry an open sub-question (T35/T36); everything else is a clean
  GraphStore port. No longer deferred.
- **D3 — closed for first richer lane.** Native code-symbol
  ingestion/search/recognition/exploration/context/explanation/use-receipting is
  no longer blocked on the Python CodeCrawler path. The remaining deferral is
  production rollout depth, not basic availability: the MCP verb has a live
  app-affordance gRPC backend path, Rust has AST-backed call/dependency
  semantics, and the remaining parser-grade dependency/call parity is beyond
  Rust.

If the answer is "no, do all of it," D1-D3 fold into a Phase 2 of this same
plan rather than disappearing.

## Sequencing

1. Lane M (M0 -> M1..M11) — the centerpiece atom store + recall; unblocks the
   user's ask and is the substrate Lane T sits on.
2. Lane T (T1-T34) after M, since document/context/memory-lifecycle verbs ride
   the Lane M store. T35/T36 wait on sub-decision D4.
3. Lane R + Lane C in parallel with M/T (independent surfaces; share only the
   read/write gating pattern). M12 is folded into T1-T5.
4. Lane P parity gates land per-lane as each verb is implemented.
5. Lane O last, as one deploy carrying the whole superset, with O3 auth as a
   hard gate before O2 write-mode on the public URL.

## Acceptance (the floor)

- `cargo test -p theorem-harness-core`, `-p theorem-harness-runtime`,
  `-p rustyred-thg-mcp` all green from `rustyredcore_THG/`.
- Both clippy gates clean.
- `apps/theorem-harness-server` tests green; live smoke returns memory +
  coordination reads from a shared `RedCoreGraphStore`.
- Memory parity corpus regenerates byte-identical
  (`parity-memory/generate_*.py --check`).
- Deployed `tools/list` shows every base verb in the superset table above
  (minus consented deferrals), and a write-per-lane + read smoke passes against
  the live URL with bearer auth.
- The harness skill docs (P5) describe the real two-server split: native V2 for
  the base surface, Python only for the engine.

---

## Codex Execution Handoff

Claude Code planned this; Codex executes. Lane split honors the live `CLAIMS.md`
ownership (Codex owns the runtime + MCP exposure lanes; Claude Code owns
fixtures + the HTTP transport).

### Ownership proposal

- **Codex:** Lane M (`theorem-harness-runtime/src/memory.rs` + MCP exposure in
  `rustyred-thg-mcp/src/lib.rs`), Lane T (the `theorem_*` product surface,
  Form-B names, on the Lane M store), Lane R, Lane C, Lane O. These are the
  runtime + MCP lanes Codex already owns per `CLAIMS.md`.
- **Claude Code (offer):** Lane P parity-memory fixtures
  (`docs/plans/harness-rust-port/parity-memory/`), matching the
  `parity`/`parity-toolgraph`/`parity-context` corpora it already generated. P5
  skill-doc correction. Claude Code can take these in parallel without touching
  the runtime/MCP files Codex is editing.

### Claim protocol (from CLAIMS.md, binding)

- Path-scoped commits only (`git commit -- <paths>`); never a bare `git
  commit` (shared index).
- Do not create a second harness crate; extend `theorem-harness-runtime` with a
  `memory` module, do not fork it.
- Add a `CLAIMS.md` row before editing a claimed file, in the same path-scoped
  commit.
- Keep `CLAUDE.md`, `Product.MD`, and
  `rustyredcore_THG/crates/reconstruction-engine/Cargo.lock` out of these
  commits unless Travis asks.

### Per-verb acceptance Codex must hit

Each memory verb is done only when: (a) it round-trips through the MCP test
suite with read-only/write-mode gating asserted, (b) its parity-memory fixture
(P1) matches, and (c) clippy is clean. A verb that writes must also be readable
back by `recall` (M2) with provenance, and an archived/forgotten atom must NOT
surface in default `recall`.

### Start command for Codex

`cd rustyredcore_THG && cargo test -p theorem-harness-runtime` is the green
baseline. Add `memory.rs`, re-run, then wire MCP exposure and re-run
`cargo test -p rustyred-thg-mcp`. The canonical contract to mirror is
`Index-API/mcp_server/tools/verbs.py` (memory verbs) + the storage helpers it
calls; reproduce the contract over `GraphStore`, not the Django implementation.
