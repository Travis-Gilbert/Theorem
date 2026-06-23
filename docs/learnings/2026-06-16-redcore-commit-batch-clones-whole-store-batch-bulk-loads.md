# `RedCoreGraphStore::commit_batch` clones the entire in-memory store on every call — bulk loads must accumulate into one `GraphMutationBatch` or they go O(N²)

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (graph-hook-primitive)`
**Domain tags:** rust, rustyred-thg-core, RedCoreGraphStore, commit_batch, performance, bulk-load

## Trigger

A centrality benchmark built a 3,000-symbol call graph by looping ~15,000
individual `store.upsert_node(...)` / `store.upsert_edge(...)` calls. It never
finished inside a 120s+ window and had to be killed (exit 144).

Root cause is in `commit_batch` (which `upsert_node`/`upsert_edge` each funnel
through, one mutation at a time):

```rust
pub fn commit_batch(&mut self, batch: GraphMutationBatch) -> ... {
    let mut staged = self.store.clone();   // <-- clones ALL current nodes+edges
    for mutation in batch.mutations { staged.apply(...) }
    self.persist_before_publish(...)?;     // stage-then-publish for crash-atomicity
    self.store = staged;
}
```

So the staging clone is **per `commit_batch` call**, not per mutation. 15k calls
against a store that grows to N elements = ~O(N²) ≈ 9M node-clones. Rewriting the
build to collect every `GraphMutation` into ONE `GraphMutationBatch` and calling
`commit_batch` once (one clone, 15k staged applies) made it finish in ~10s.

## Rule

For any bulk write to RedCore — ingest, fixtures, backfill, benchmarks, a hook
that rewrites many nodes — accumulate `GraphMutation`s into a single
`GraphMutationBatch` and call `commit_batch` once. Never loop
`upsert_node`/`upsert_edge` over more than a handful of records. The
stage-then-publish clone is deliberate (rollback on a failed AOF append), so the
cost is paid per `commit_batch`, and the only lever you have is batch size. This
is also why the incremental-centrality hook writes back only the nodes whose
value actually changed.

## Evidence

- `rustyredcore_THG/crates/rustyred-thg-core/src/graph_store.rs` `commit_batch`:
  `let mut staged = self.store.clone();` near the top.
- `crates/rustyred-thg-code/tests/centrality_latency.rs`: `build_call_graph`
  builds one `GraphMutationBatch` and commits once (the comment records why).
