# Harness Memory Tenancy (V2 / Theorems-Harness)

Date: 2026-06-02
Status: Historical convention plus live-drift warning. This records the intended
partitioning model for Travis's harness memory on the deployed V2 server
(`rustyredcore-theorem-production`, self-id `rusty-red-graph-database`), but the
2026-06-16 production check below supersedes the old lowercase assumption.
2026-06-18 implementation note: runtime memory receipts now preserve the caller's
tenant casing for new writes, and memory reads inside an already-opened tenant
partition also check legacy lowercase row metadata for compatibility. Do not use
this compatibility read as permission to address the lowercased split partition.

NOTE: the memory itself is LIVE DATA in the RustyRed graph store, not in git.
This file is the durable record of the convention + the operations performed; it
is not the data. As of the 2026-06-16 check, the reachable production memory
surface is the `Travis-Gilbert` tenant partition, while `travis-gilbert` is a
near-empty split partition.

## Live drift check (2026-06-16)

- `rustyred_thg_graph_schema tenant=Travis-Gilbert` returned 54 nodes, including
  `MemoryDocument`, `HarnessMemory`, and coordination labels.
- `rustyred_thg_graph_schema tenant=travis-gilbert` returned 2 coordination
  nodes and no `MemoryDocument` nodes.
- `rustyred_thg_graph_schema tenant=default` returned 4253 coordination-heavy
  nodes and no `MemoryDocument` nodes.
- `rustyred_thg_graph_query tenant=Travis-Gilbert label=MemoryDocument`
  returned 4 memory documents. This is lower than the 47 documents recorded in
  the 2026-06-02 verified end state, so the missing memory corpus still needs a
  recovery or migration pass from the older store/backups.
- Local Codex plugin hooks now resolve `THEOREM_TENANT_ID=Travis-Gilbert` by
  default in the installed cache and source plugin copy, so new hook traffic
  stops falling into `default` when no tenant env var is set.

## Consolidation pass (2026-06-16)

The missing corpus was found on the older RedCore service
`https://thg-product-production.up.railway.app`, not in `default` on the current
`rustyredcore-theorem-production` service. The older service held V1-style
`MemoryAtom` / `TheoremsHarness` data under lowercase `travis-gilbert` and
`default`; the current service held only the small V2 `MemoryDocument` set under
capitalized `Travis-Gilbert`.

Codex migrated the deduped old corpus into current canonical tenant
`Travis-Gilbert` on `rustyredcore-theorem-production` using deterministic
`legacy-thg-<hash>` `doc_id`s and `upsert_note`, so reruns update the same
documents instead of creating duplicates.

Migration result:

- Old unique sources inspected: 1042.
- Migrated V2 `MemoryDocument`s: 372.
- Source split: 264 from old `travis-gilbert`, 108 from old `default`.
- Status split: 228 active knowledge-bearing documents, 144 archived
  coordination-telemetry documents.
- Deduped or empty sources skipped: 670. Three non-empty source nodes were
  skipped because an equivalent content/title/kind source was migrated:
  `custom-graphql-scalars...-2`, `harness:encode:7f373135...`, and
  `harness:encode:bd7331...`.
- Verification after migration: current `Travis-Gilbert` held 376
  `MemoryDocument`s total, with 232 active and 144 archived. The migrated batch
  is tagged `migration_batch=2026-06-16-legacy-thg-consolidation`,
  `legacy_source_service=thg-product-production`, `legacy_source_tenant`, and
  `legacy_source_node_id`.

Status policy for this pass: semantic memory kinds (`orchestrate`, `solution`,
`feedback`, `postmortem`, `encode`, `self_note`) were kept active; old
coordination/presence/subscribe/room telemetry was preserved but archived so it
is recoverable without dominating normal recall.

## The tenant model (grounded in `rustyredcore_THG`)

- A "tenant" is a **keyspace partition** keyed by a sanitized slug, not a row
  with a display name. `GraphStore::tenant(...)` + `tenant_prefix(base, id)`
  namespace every key; `sanitize_tenant_segment` percent-encodes unsafe bytes
  and preserves ASCII casing. In practice, `Travis-Gilbert` and
  `travis-gilbert` are different partitions on the deployed store.
- `default` is the **catch-all** the server uses when no tenant is named. The
  default is env-configurable: `RUSTY_RED_MCP_DEFAULT_TENANT` /
  `RUSTYRED_THG_MCP_DEFAULT_TENANT` (falls back to `"default"`,
  `rustyred-thg-server/src/config.rs:243`, `rustyred-thg-mcp/src/lib.rs:298`).
- Tenants **auto-create on first write** (no pre-declaration). Writing a memory
  with `tenant=travis-gilbert` creates the partition.

## The convention (binding)

- **Travis's harness memory belongs under `Travis-Gilbert`, never the
  lowercased split partition and never the catch-all `default`.** Memory landed
  in `default` only because un-tenanted writes fall back there; lowercase
  receipts caused a separate split-read footgun.
- **ALWAYS pass `tenant=Travis-Gilbert`** on his memory ops (`remember`,
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
  binding, so it depends on each session passing `tenant=Travis-Gilbert`.
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
must pass `tenant=Travis-Gilbert`, and a periodic sweep (same mechanism above)
catches any that slipped. Recorded as cross-session memory in the agent's
`harness-mcp-v2-split` note.
