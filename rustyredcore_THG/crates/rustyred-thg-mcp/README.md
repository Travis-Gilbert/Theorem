# rustyred-thg-mcp

The native Rust MCP request handler over the RustyRed graph store. It exposes harness capabilities as MCP tools (memory, coordination, jobs, code intelligence, graph queries and algorithms, versioning, search, symbolic, browsing) with no Python in the loop.

This crate is a handler, not a transport. The same surface is served over stdio (the bundled `theorems-harness` plugin) and over HTTP (`POST /mcp` on `rustyred-thg-server`); this crate owns neither listener. The entry point is `handle_mcp_request(provider, config, payload) -> Value`.

## The SharedStore seam

- `McpGraphBackend`: the per-tenant store interface, auto-implemented so any `GraphStore` works while a durable store's inherent overrides (native code search, real upserts) are preserved.
- `McpGraphProvider { backend_for_tenant(tenant) }`: yields a backend per tenant.
- `SharedStore<S>(Rc<RefCell<S>>)`: the embedded, in-process seam (North Star E0). One owned durable store behind `Rc<RefCell>`; `backend_for_tenant` clones the handle (no reopen). It is its own provider, so `SharedStore::new(store)` passes straight to `handle_mcp_request`. Single-threaded by construction, matching the synchronous GraphQL dispatch.
- Config: `McpServerConfig` (`read_only`, `allow_admin`, `tool_result_budget_bytes`, `graphql_default_surface`, ...). `McpRequestContext { scopes }` (scope-gated; `"*"` wildcard). Protocol version `2025-06-18`, JSON-RPC 2.0.

## Tool surface

108 flat tools (`tool(...)` / `tool_write(...)`) plus a GraphQL transport (`graphql_query`, `graphql_mutate`, `graphql_introspect`). In read-only mode, write tools return a structured `mcp_read_only` error. In `graphql_default_surface` mode, the GraphQL-covered flat tools are hidden from `tools/list` (`GRAPHQL_COVERED_FLAT_TOOLS`).

Tool families include: memory/self (`recall`, `remember`, `encode`, `self_note`), coordination (`coordinate`, `presence`, room reads, `stream_*`), jobs (`job_submit`/`list`/`note`/`archive`), harness/multihead (`harness_run`, `multihead_*`), code/KG (`compute_code`, `code_ingest`, `harness_kg_*`), graph reads (`rustyred_thg_graph_*`, `rustyred_thg_relational_query`), vector/full-text/spatial (`rustyred_thg_vector_*`, `..._fulltext_*`, `..._spatial_*`), algorithms (`rustyred_thg_algorithm_pagerank/ppr/components/communities`), versioning (`rustyred_thg_graph_version_*`), symbolic (`rustyred_thg_symbolic_*`), browsing (`web_consume`, `browse_for_me`, `fractal_expansion`), and the connector gateway meta-tools (`tool_search`, `describe`, `invoke`). Plus 6 static resources, 3 resource templates, and 4 prompts.

The GraphQL transport (`graphql/`) wires typed domains (graph, memory, coordination, items, code, kg, epistemic) and executes synchronously via `futures_executor::block_on`.

Path deps: `rustyred-thg-core`, `rustyred-thg-code`, `rustyred-thg-ml`, `rustyred-thg-affordances`, `rustyred-thg-connectors`, `theorem-harness-{core,runtime}`, `ensemble`. Feature `redis-store` forwards to core.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-mcp
```

`tests/skill_corpus_acceptance.rs` plus extensive inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
