# A RustyRed plugin handler receives a concrete `&mut RedCoreGraphStore`, not `&mut dyn GraphStore`, and RedCore's inherent methods shadow the `GraphStore` trait with different return types; keep durable logic in generic `fn<S: GraphStore>` and let the handler hold the concrete-store specifics

**Kind:** gotcha
**Captured:** 2026-06-27
**Session signature:** `claude-code:travisgilbert (DATAWAVE ingest+edge intake; Codex built the reconstruction half)`
**Domain tags:** rustyred, graphstore, plugin, redcore, trait-shadowing

## Trigger

Building the `theorem.ingest.datawave` harness handlers (mirroring `rustyred-thg-reconstruct-harness`), I expected the plugin `PluginOperationContext.store` to be `&mut dyn GraphStore`. It is not: it is a concrete `&'a mut RedCoreGraphStore` (`crates/rustyred-thg-core/src/plugin.rs`). That matters because `RedCoreGraphStore` defines *inherent* `query_nodes`/`upsert_node`/etc. that shadow the `GraphStore` trait methods with different signatures: the inherent `query_nodes` returns `GraphStoreResult<Vec<NodeRecord>>` while the trait `query_nodes` returns a bare `Vec<NodeRecord>`. On a concrete receiver the inherent (Result) version wins, so a handler that calls `context.store.query_nodes(q)` must `?` it -- a generic `fn<S: GraphStore>` calling the same name would instead get the non-Result trait method. The deeper structural fact (found earlier the same session): `designate_vector_property` is inherent-only on `InMemoryGraphStore`/`RedCoreGraphStore` and is NOT on the `GraphStore` trait, so a data layer written generic over `S: GraphStore` cannot call it at all. That is why the intake data layer stays trait-generic (it gets exactly the trait surface and works for both InMemory tests and RedCore), and the harness handlers -- which hold the concrete store -- own the `?`-on-inherent reads. Designation/encoder concerns belong to whoever holds the concrete store, not the generic layer.

## Rule

In a RustyRed plugin handler, the store is the concrete `RedCoreGraphStore`, not a trait object. Expect inherent-vs-trait method shadowing (notably Result-returning inherent `query_nodes`/`upsert_*` vs the bare-return trait methods) and `?` the inherent reads. Architecturally: write durable writes/reads as generic `fn<S: GraphStore>` helpers so they compile against `InMemoryGraphStore` in unit tests and `RedCoreGraphStore` in the handler; keep concrete-store-only capabilities (`designate_vector_property`, `designate_ordered_property`) out of the generic layer and in the concrete caller. When you must call a trait method on a concrete RedCore/Redis store, use UFCS (`GraphStore::get_node(&store, id)`) to dodge the shadow.

## Evidence

- `crates/rustyred-thg-core/src/plugin.rs`: `PluginOperationContext { tenant_id, operation, command, store: &'a mut RedCoreGraphStore }`; `PluginOperationHandler = fn(PluginOperationContext, Value) -> GraphStoreResult<Value>`.
- `crates/rustyred-thg-core/src/graph_store.rs`: trait `query_nodes -> Vec<NodeRecord>` vs RedCore inherent `query_nodes -> GraphStoreResult<Vec<NodeRecord>>`; `designate_vector_property` inherent on InMemory/RedCore only.
- `crates/rustyred-thg-datawave-harness` lookup/intersect handlers `?` the inherent `query_nodes`; the data layer `crates/rustyred-thg-datawave` stays generic over `S: GraphStore` and never designates.

## Encoded in

- `docs/learnings/2026-06-27-plugin-handler-store-is-concrete-redcore-with-trait-shadowing.md` (this file)
