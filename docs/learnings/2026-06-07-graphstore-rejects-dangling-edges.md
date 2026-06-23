# GraphStore rejects edges to non-existent endpoints (referential integrity)

**Kind:** gotcha
**Captured:** 2026-06-07
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** rust, thg, graphstore, rustyredcore_thg

## Trigger

A `theorem-harness-runtime` job_queue test failed with:

```
Store(GraphStoreError { code: "missing_graph_endpoint",
  message: "edge harness:edge:dispatched-as:job-... to endpoint
  harness:run:harnessrun:abc does not exist" })
```

I had written a `DISPATCHED_AS` edge from a `Job` node to a run node
(`harness:run:{run_id}`) that did not exist in the store. `InMemoryGraphStore`
(and the `GraphStore` contract) enforce referential integrity on `upsert_edge`:
an edge whose `from`/`to` endpoint is missing is a hard error, not a silent
insert. This is easy to hit when a node in one subsystem (a Job) references a
node owned by another subsystem (a run from the event log, or a memory doc).

## Rule

When writing a cross-subsystem edge, link opportunistically: guard with
`store.get_node(target_id).is_some()` and skip the edge if the endpoint is
absent (let the property carry the reference until it materializes), OR create
the endpoint node first in the same `&mut` borrow. Never `upsert_edge` to an id
you have not confirmed exists.

`event_log.rs` did not need this guard only because its edges point at nodes it
upserts in the same call (the run node and the previous event). New code that
references foreign nodes must add the existence check.

## Evidence

- `rustyredcore_THG/crates/theorem-harness-runtime/src/job_queue.rs`:
  `link_dispatched_as` and `maybe_link_spec` both gate on
  `store.get_node(...).is_none() { return Ok(()) }`.
- The `PRODUCED` edge is safe because `link_produced` upserts the artifact node
  immediately before the edge, in the same borrow.

## Encoded in

- `docs/learnings/2026-06-07-graphstore-rejects-dangling-edges.md` (this file)
