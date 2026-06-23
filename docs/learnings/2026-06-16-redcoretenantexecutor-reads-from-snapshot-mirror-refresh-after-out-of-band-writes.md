# `RedCoreTenantExecutor` serves reads from a `committed_snapshot` mirror refreshed only on its own commit path — writes made out-of-band through the `writer` are invisible to reads until you refresh it

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (graph-hook-primitive)`
**Domain tags:** rust, rustyred-thg-server, RedCoreTenantExecutor, committed_snapshot, reads, hooks

## Trigger

Wiring graph-level hooks into `RedCoreTenantExecutor`, the hook worker writes
derived nodes by locking the executor's `writer: Mutex<RedCoreGraphStore>`
directly and calling `commit_batch` on it. The write succeeded and was durable —
but `executor.get_node("derived:1")` kept returning `None`.

The executor splits read and write state:

```rust
pub struct RedCoreTenantExecutor {
    writer: Mutex<RedCoreGraphStore>,            // writes go here
    committed_snapshot: RwLock<InMemoryGraphStore>, // reads come from here
    cached_edges: RwLock<Option<(u64, Arc<Vec<EdgeRecord>>)>>,
}
```

`get_node`/`query_nodes`/`neighbors`/`stats` all read `committed_snapshot` (via
`with_snapshot`). That mirror is rebuilt ONLY inside the executor's own
`commit_batch`/`rebuild_indexes`:
`*committed_snapshot.write() = InMemoryGraphStore::from_snapshot(writer.graph_snapshot())`.
A write that bypasses those methods (locking `writer` directly) never refreshes
the mirror, so reads stay stale. The fix was a `run_hook_batch` that rebuilds
`committed_snapshot` from the writer immediately after the out-of-band write.

## Rule

Any code that mutates a tenant store by locking `RedCoreTenantExecutor::writer`
*outside* `commit_batch`/`rebuild_indexes` (hooks, TTL sweeps, background jobs)
MUST rebuild `committed_snapshot` from `writer.graph_snapshot()` after the write,
or the change is invisible to every read path. Reads never touch the writer. If a
durable write "doesn't show up" in `get_node`/`query_nodes`, suspect a missing
snapshot refresh before suspecting the write.

## Evidence

- `rustyredcore_THG/crates/rustyred-thg-server/src/state.rs`: `commit_batch`
  refreshes `committed_snapshot`; `get_node` etc. call `with_snapshot`;
  `run_hook_batch` replicates the refresh for the hook write path.
- Test `state::tests::graph_hooks_refresh_executor_read_snapshot` asserts the
  hook-derived node is visible through the read snapshot.
