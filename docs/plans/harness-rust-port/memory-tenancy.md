# Harness Memory Tenancy (V2 / Theorems-Harness)

Date: 2026-06-02
Status: Convention in force. Operational runbook for how Travis's harness memory
is partitioned on the deployed V2 server (`rustyredcore-theorem-production`,
self-id `rusty-red-graph-database`).

NOTE: the memory itself is LIVE DATA in the RustyRed graph store, not in git.
This file is the durable record of the convention + the operations performed; it
is not the data. The data lives under the `travis-gilbert` tenant partition on
the deployed server.

## The tenant model (grounded in `rustyredcore_THG`)

- A "tenant" is a **keyspace partition** keyed by a sanitized slug, not a row
  with a display name. `GraphStore::tenant(...)` + `tenant_prefix(base, id)`
  namespace every key; `sanitize_tenant_segment` keeps `[A-Za-z0-9-_]` and drops
  everything else (so "Travis Gilbert" -> "TravisGilbert"; the clean slug is
  `travis-gilbert`). There is no separate display-name field: the slug IS the
  name everywhere (`tenant_slug` in results).
- `default` is the **catch-all** the server uses when no tenant is named. The
  default is env-configurable: `RUSTY_RED_MCP_DEFAULT_TENANT` /
  `RUSTYRED_THG_MCP_DEFAULT_TENANT` (falls back to `"default"`,
  `rustyred-thg-server/src/config.rs:243`, `rustyred-thg-mcp/src/lib.rs:298`).
- Tenants **auto-create on first write** (no pre-declaration). Writing a memory
  with `tenant=travis-gilbert` creates the partition.

## The convention (binding)

- **Travis's harness memory belongs under `travis-gilbert`, never the catch-all
  `default`.** Memory landed in `default` only because un-tenanted writes fall
  back there.
- **ALWAYS pass `tenant=travis-gilbert`** on his memory ops (`remember`,
  `recall`, `encode`, `self_note`, `self_revise`, `relate`, `forget`) and on
  `graph_query` reads of his memory.
- **Do NOT change the global default tenant** (`RUSTY_RED_MCP_DEFAULT_TENANT`
  stays `default`) and **do NOT touch coordination.** Travis explicitly rejected
  the default-flip: it would re-home the active coordination room
  (`repo:theorem:branch:main`) and all un-tenanted activity, which is broader
  than memory and would disrupt cross-agent coordination.

## Now vs eventually

- **Now** (single real user; the `/mcp` endpoint has no per-user auth): the
  tenant is carried per-call by convention. There is no per-connection tenant
  binding, so it depends on each session passing `tenant=travis-gilbert`.
- **Eventually** (multi-user): tenant is **derived from auth** (bearer token ->
  tenant) so un-tenanted writes can't happen and each user's memory auto-scopes.
  This is the same work as locking the public write endpoint (superset plan
  Lane O3). It supersedes the convention.

## Operations performed (2026-06-02)

All via the native MCP verbs over the deployed server; idempotent + reversible.

1. **Migrate (36).** The 36 accumulated `MemoryAtom` learnings from the OLD
   harness store (`mcp__rustyred-thg__*` / `mcp__01cc4e24__*`, the
   `MemoryAtom`/`MemoryRevision`/`TheoremsHarness` model) were re-encoded into
   `travis-gilbert` via `encode` (old `MemoryAtom` -> V2 `MemoryDocument`),
   tagged `metadata.migrated_from='rustyred-thg'` + `source_node_id`. The
   `default` duplicates from the first pass were soft-deleted via `forget`.
   `AgentMemory`/`MemoryRevision` proved to be overlapping labels on the same 36
   nodes, so 36 is the full memory set from that store.
2. **Sweep (11).** 11 active non-migration `MemoryDocument`s that recent
   un-tenanted sessions had written straight to `default` (all `solution` kind:
   encoding-pipeline, RustyRed-trainability, composition-roster,
   multi-agent-collision, durable-substrate, personality-as-self-memory, SDK v2
   spec, connector-layer, lab-graph-superset, mobile-Servo, SDK
   Rust-core-first) were re-encoded into `travis-gilbert`, tagged
   `metadata.swept_from='default'` + `source_doc_id`, then soft-deleted from
   `default`.

### Mechanism (reproducible without the throwaway scripts)

- **Copy-then-forget:** a `default` doc is `forget`-ed only after its
  `travis-gilbert` copy is confirmed written. Nothing is deleted before its
  replacement exists.
- **Idempotent dedup:** before copying, read the target tenant's existing
  `MemoryDocument` nodes (`graph_query` node_match, `tenant=travis-gilbert`),
  collect `metadata.source_node_id` (migration) / `metadata.source_doc_id`
  (sweep), and skip any source already present. Re-running never duplicates.
- **Fitness preserved:** `solution`/`feedback`/`postmortem`/`encode` kinds go
  through `encode` (carrying `outcome`/`signal`/`reason`), not the lossy
  `remember` path.

### Verified end state

- `recall` (no tenant -> `default`) returns **0** of Travis's migrated/swept
  memories.
- `recall` (`tenant=travis-gilbert`) returns them.
- `travis-gilbert` holds **47** memory documents (36 migrated + 11 swept) +
  coordination; `default` is clean of his memory.

## Known live gap

The leak recurs until auth-derived tenancy lands: any future session that writes
memory un-tenanted will land in `default` again. Until Lane O3, future sessions
must pass `tenant=travis-gilbert`, and a periodic sweep (same mechanism above)
catches any that slipped. Recorded as cross-session memory in the agent's
`harness-mcp-v2-split` note.
