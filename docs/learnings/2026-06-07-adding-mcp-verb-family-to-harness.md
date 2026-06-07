# Adding an MCP verb family to the harness: the four sites that must all change

**Kind:** method
**Captured:** 2026-06-07
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** rust, thg, mcp, rustyred-thg-mcp, rustyred-thg-server

## Trigger

Adding the six dispatch verbs (`job_submit` … `job_complete`) as MCP tools meant
touching several non-adjacent sites. The dangerous one is the product server
backend: the `McpGraphBackend` trait gives every verb a default "not supported"
body, so a verb that compiles and passes stdio/in-memory tests can still be
silently dead on the real Railway server path if you forget to override it on
`ProductMcpBackend`.

## Rule

A new harness MCP verb requires all four sites, in this order:

1. **Trait** (`rustyred-thg-mcp/src/lib.rs`): add the method to `McpGraphBackend`
   with a default body returning `McpError::internal("... not supported by this
   MCP backend")`.
2. **In-process backends** (same file): override on `RedCoreGraphStore` and
   `InMemoryGraphStore`, each delegating to a shared free function.
3. **Product server backend** (`rustyred-thg-server/src/state.rs`): override on
   `ProductMcpBackend` — WRITE verbs wrap `RuntimeTenantMirrorGraphStore::new(&mut
   self.store)` then call the runtime; READ verbs build
   `InMemoryGraphStore::from_snapshot(self.store.graph_snapshot()?)` and read from
   the mirror. This is the site that makes the verb work in production; it is the
   easy one to forget.
4. **Dispatch + schema** (`rustyred-thg-mcp/src/lib.rs`): add the `call_tool`
   match arm (gate writes behind `config.read_only`) and a `tool` / `tool_write`
   entry in `tool_definitions`.

Centralize payload shaping in pub `*_to_store` helpers in the MCP crate so the
in-process backends and the server's mirror backend return byte-identical JSON;
the server reuses the same helpers rather than re-deriving the envelope.

## Evidence

- Commit `b6be2e42` (dispatch-queue): the `job_*_to_store` helpers are `pub` and
  called by both the mcp-crate impls and `ProductMcpBackend`.
- The pattern mirrors the pre-existing `append_harness_transition` /
  `harness_run_detail`, which already used the mirror-on-write and
  snapshot-on-read shape on `ProductMcpBackend`.

## Encoded in

- `docs/learnings/2026-06-07-adding-mcp-verb-family-to-harness.md` (this file)
