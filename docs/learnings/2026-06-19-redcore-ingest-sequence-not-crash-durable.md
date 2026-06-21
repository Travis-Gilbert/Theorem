# RedCore: a multi-write ingest sequence is not crash-durable across an abrupt restart, even with AofAlways (single writes are)

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (CommonPlace durable backing)`
**Domain tags:** redcore, durability, aof, fsync, crash-recovery, rustyred-thg-core

## Trigger

Wiring durable backing for `apps/commonplace-api` (RedCore + `DiskObjectStore` behind the GraphQL schema and the `commonplace-mcp` bin), the durable acceptance tests showed a sharp, reproducible split across a **process restart** (spawn the binary with `COMMONPLACE_DATA_DIR`, write over HTTP/MCP, `child.kill()` = SIGKILL, respawn over the same dir, read):

- `putNote` (one node upsert) -> **survives** the restart. Item present.
- Full `ingest` (designate vector + create collection + double-upsert collection + put item with embedding) -> **lost**. `items` is empty after restart.

Three isolating probes (RedCore opened directly, in-process drop + reopen) ALL passed: (A) `designate_vector_property` then `upsert_node`; (B) a node with an f32 embedding array property; (C) the FULL `IngestPipeline::ingest` over `redcore_store` -> reopen -> item present. So designation, embeddings, and even the whole ingest are durable across an **in-process** reopen. Switching the durable store from the default `AofEverysec` to `AofAlways` (fsync per commit) did NOT change the restart outcome -- ingest was still lost across SIGKILL. So it is not a "lost the last second" fsync-cadence issue; it is specific to the multi-write ingest pattern surviving a clean drop but not an abrupt process kill.

## Rule

Do not assume RedCore is crash-durable for multi-write transactions just because single writes persist and in-process reopen works. Until the core gap is fixed: (1) `put`/`edit` (single-node) writes are durable across a restart and can be relied on; (2) the auto-structuring `ingest` sequence is durable across a GRACEFUL stop / in-process reopen but NOT across an abrupt kill -- treat ingest-then-crash as potentially lossy. When testing RedCore durability, test a real PROCESS restart (in-process drop + reopen hides this, because RedCore has no `Drop`-flush and an in-process reopen reads what a clean drop left). The fix belongs in `rustyred-thg-core` (AOF append/flush of a commit's full frame set must be fsync-complete before the commit returns, so a SIGKILL immediately after a returned commit cannot lose it). Surface this to the core lane; do not paper over it by only testing graceful shutdown.

## Evidence

- `apps/commonplace-api/tests/durable_http_acceptance.rs`: `http_binary_persists_items_across_restart` (putNote) PASSES; `ingest_survives_restart_blocked_on_redcore` is `#[ignore]`d (asserts the target; un-ignore when core fixes it). `durable_mcp_acceptance.rs`: `mcp_persists_items_across_restart` (putNote) PASSES.
- Probes (since removed) confirmed in-process durability of designate+upsert, f32-embedding nodes, and full ingest -- so the data path and recover() are correct for a clean reopen.
- `RedCoreOptions { durability: AofAlways, .. }` did not change the SIGKILL outcome for ingest.
- `RedCoreGraphStore` has no `impl Drop` (only `RedCoreDirectoryLock` does), so durability relies entirely on commit-time AOF persistence; the gap implies a commit's frames are not fully on disk when the commit returns for the multi-write path. Core file: `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` (`append_aof`, `persist_before_publish`, `commit_batch`, `recover`). Related: [[redcore-graphstore-is-send-sync]].
