# Spec 2: Harness-node-to-Item projection and the live changefeed

**Register**: execution | **Home**: Theorem, `rustyredcore_THG/crates/rustyred-thg-mcp/src/graphql` and the async product server | **Date**: 2026-06-20

Makes the symmetry real: an agent writing through the harness MCP, and a person watching CommonPlace, see the same objects, and the agent's write appears in the Auto-Organizer live. Reading the code reshaped the naive version, and the design now respects two findings.

## Two findings that shape this spec

1. **The GraphQL surface has no Item domain.** `QueryRoot` is memory, graph, coordination, epistemic, code, kg, clusters. CommonPlace Items, which the UI renders, are not in the shared schema. So "the UI GraphQL matches the MCP GraphQL" is not yet true for Items, because there is no Item field on either side. This spec adds the Item domain to the one shared schema, and a projection populates it with harness and Theseus writes as well as real Items.

2. **The GraphQL transport is synchronous and the backend is not `Send` or `Sync`.** Every query and mutation runs through `futures_executor::block_on` on the dispatch thread, with a thread-local invoker installed only for the duration of that one synchronous execution (`mod.rs` documents this; the fixture backend is `Rc<RefCell<..>>`). A long-lived GraphQL subscription cannot use that model: the invoker is gone the moment `block_on` returns, and the non-`Send` backend cannot move into an async task. So the live changefeed is a separate streaming endpoint on the async product server, fed by a hook, while the Item query that hydrates it rides the existing synchronous surface.

## What is graph-resident, and what is not

The hook changefeed (`hooks.rs`) fires on `RedCoreGraphStore` mutations. The graph-resident harness writes are on it: the work-graph task node (`multihead_task_payload`), the coordination record (`write_record_payload`), and memory (`remember` and `encode`), each of which takes `&mut` backend and writes the graph store. Theseus's organizing writes, when they land as Items and edges through the contract, are on it too. The Dispatch v2 Job's canonical state is a Postgres row (`theorem-dispatch/src/model.rs`: hot execution state in Postgres), so a job submission is not guaranteed to be on the graph changefeed. This spec covers the graph-resident writes, the path behind "lay out the next spec's tasks and watch them appear," and reconciles the job path explicitly rather than pretending one mechanism covers both.

## The invariant

The projection never mutates the harness or Theseus nodes it reads. It presents them as Items on read and as Item deltas on the changefeed, and the single source of truth stays the native node. One projection function serves both the read resolver and the changefeed publisher, so the live shape and the queried shape cannot diverge.

## Deliverables

### 1. The projection function

- New module `rustyred-thg-mcp/src/graphql/projection.rs`. One function maps a graph node, by label, to the Item GraphQL type: the work-graph task label to `ItemKind::Other("task")`, the coordination record label to `Other("coordination")`, the memory doc label to `Other("memory")`, and the `Item` label to its own kind. Title comes from the node's title or goal field, timestamps from its created and updated fields, `source` is set to the origin, and the node's native properties are preserved under the Item's `extra`. A second form maps a `MutationEvent` (kind, labels, id, changed_props) to the same projection for the changefeed.
- Bind the exact label strings by reading `multihead_task_payload` and `write_record_payload` in `rustyred-thg-mcp/src/lib.rs`; they are the writers, and their node labels are the projection's match keys.

### 2. The Item domain on the shared GraphQL schema

- New `rustyred-thg-mcp/src/graphql/items.rs` with an `ItemQuery` object: `items`, `itemsByKind`, and `item(id)`. It returns real Items, read through the commonplace store facade over the connection-tenant backend, unioned with projected harness and Theseus nodes via the projection function. Resolvers wrap reads, matching the house pattern where the existing domains call crate-private `*_payload` fns; this calls the commonplace store reads plus the projection.
- Add `ItemQuery` to `QueryRoot` in `mod.rs`. This is the change that makes the UI GraphQL and the MCP GraphQL carry the same Item field, so a query issued by the UI and a query issued by an MCP client return the same Items.

### 3. The changefeed hook

- Register a `HookRegistration` (`hooks.rs`) whose matcher is the `Item` label plus the projected harness labels, and whose handler maps each `MutationEvent` through the projection and publishes the resulting Item delta to a broadcast channel. The handler captures the channel sender; `HookHandler` is `Arc<dyn Fn + Send + Sync>`, so it holds the sender cleanly, and the substrate stays tokio-free because the send is non-blocking and called from the std::thread dispatcher.
- Install the registration on the dispatcher at server init, alongside or through `PluginRegistry::hooks`. The hook obeys the existing contract unchanged: post-commit, coalesced, fail-open, loop-guarded by depth.

### 4. The async changefeed endpoint

- On the async product server, the one that already serves the server-only tools named in `mod.rs` (`composed_agent_run`, the browse and fractal tools), expose the broadcast channel as a streaming endpoint at a stable path: server-sent events or a WebSocket, emitting projected Item deltas as JSON. The endpoint is the async consumer of the channel the sync hook feeds; this is the one sync-to-async seam, and it is crossed by the channel, not by sharing the non-`Send` backend.
- The CommonPlace Auto-Organizer subscribes to this endpoint and applies each delta, and hydrates initial state through the Item query in deliverable 2. Literal GraphQL subscription syntax is a later upgrade: mount an async-graphql `Subscription` on this same async server reading the same channel, once a `Send` and `Sync` read handle for hydration exists. The changefeed substance does not wait on that.

### 5. Reconcile the Dispatch job path

- Confirm whether `job_submit_payload` writes a graph node in addition to the Postgres row. If it does, that node projects like any other and needs nothing further. If it does not, the job lifecycle is off the graph changefeed, and the next slice is a parallel publisher: the dispatch layer publishes job state changes to the same broadcast channel, projected to the Item shape with `Other("job")`, so jobs reach the Auto-Organizer by the same endpoint without forcing every job into the graph.

## Acceptance criteria

1. Creating a work-graph task node, a coordination record, or a memory doc through the harness MCP yields a projected Item visible through the `items` query, carrying kind, title, timestamps, `source`, and the native fields under `extra`.
2. The same write delivers a projected Item delta on the changefeed endpoint within the dispatcher debounce window, identical in shape to the query projection.
3. A real Item written through the commonplace store appears on the query and the changefeed in the same shape as a projected harness Item, so harness-origin and user-origin Items are indistinguishable to the UI.
4. The projection is one function, called by both the Item resolver and the changefeed publisher; a shape change in one is a shape change in both.
5. The changefeed hook holds the hook contract: writer throughput is unchanged with it installed, a panicking publish is logged and skipped, and the loop guard bounds any hook-induced writes.
6. `ItemQuery` is a field on the one `QueryRoot`, so the SDL returned by `graphql_introspect` shows the Item field that the UI and MCP clients both query.
7. The existing GraphQL query and mutate transport, and their tests, are unchanged; the Item domain and the changefeed are additive.
8. The job path is reconciled: the spec records whether `job_submit` writes a graph node, and either projects that node or records the parallel-publisher path as the next slice.

## Assumptions

- The work-graph task node label and the coordination record node label are confirmed by reading `multihead_task_payload` and `write_record_payload` in `rustyred-thg-mcp/src/lib.rs` before the projection binds them.
- The async product server's structure and the mount point for the streaming endpoint are confirmed on the branch; the endpoint lives there because the MCP GraphQL transport is synchronous and cannot host a live stream.
- The commonplace store facade can be constructed over the connection-tenant backend inside the Item resolver; if the backend handle and the facade's `GraphStore` bound need an adapter, that adapter is part of deliverable 2.
- The broadcast channel type, a tokio broadcast or a std channel drained by an async task, is the implementer's choice, constrained only by a non-blocking send callable from the std::thread hook dispatcher.
- The hook emitter is attached to the same `RedCoreGraphStore` the harness writes route through, so harness mutations actually emit `MutationEvent`s; confirm `attach_hook_emitter` is wired at server init.
