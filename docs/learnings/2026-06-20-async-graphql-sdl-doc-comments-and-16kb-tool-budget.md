# In the THG MCP GraphQL surface, `///` doc comments ARE the SDL (descriptions), and the SDL is fetched through the 16KB MCP tool-result budget -- both bite when you grow the typed schema

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (SPEC-GRAPHQL-MCP-FINISH A6 typed cluster domains)`
**Domain tags:** async-graphql, graphql, sdl, mcp, tool-result-budget, rustyred-thg-mcp, introspection

## Trigger

Adding ~11 typed `SimpleObject`s (`SkillPack`, `Job`, `EnsembleSelection`, `HarnessRun`, ...) to `graphql/clusters.rs` to satisfy the spec's "typed objects, not raw JSON blobs" broke 6 tests two different ways, neither obvious from the code:

1. **A doc comment failed a schema invariant.** `graphql_introspect_exposes_cluster_domains` asserts `!sdl.to_lowercase().contains("tenant")` (the schema must be tenant-free; tenant is connection-scoped). My `SkillPack` `///` doc said "No `tenant` field: the connection tenant is implicit..." -- and async-graphql renders `///` doc comments as SDL **descriptions**, so the word "tenant" landed IN the schema and tripped the assertion. (`//` line comments and `//!` module docs are NOT rendered; only `///` on types/fields/resolvers/args.)
2. **The SDL outgrew the tool-result budget.** The richer typed schema pushed the SDL from under 16KB to **16718 bytes**, past `DEFAULT_TOOL_RESULT_BUDGET_BYTES = 16 * 1024`. Five `graphql_introspect_exposes_*_domain` tests fetch the SDL THROUGH the tool (`call_tool_json(..., "graphql_introspect", ...)`), which truncated it into a `fetch_handle` envelope, so `sdl.as_str()` returned `None` and they panicked `introspect should return an SDL string`. The sibling `graphql_introspect_returns_sdl_and_flat_tools_still_answer` already passed because it set `tool_result_budget_bytes = 0`.

## Rule

When adding to the typed GraphQL surface in `rustyred-thg-mcp`: (1) keep any schema-banned word (e.g. "tenant") OUT of `///` doc comments on types/fields/resolvers -- they become SDL descriptions; use `//` for notes that must stay out of the schema. (2) Any test that fetches the full SDL through the `graphql_introspect` TOOL must set `config.tool_result_budget_bytes = 0`, because the SDL only grows and the default 16KB MCP boundary budget truncates it into a fetch-handle envelope. A test using `introspect_sdl()` directly (in-crate) is not subject to the budget.

## Evidence

- Measured `SDL_LEN=16718 (budget=16384)` via a temporary `eprintln!` in an in-crate introspect test.
- Fix: reworded the `SkillPack` `///` doc to drop "tenant"; set `tool_result_budget_bytes = 0` on the 5 per-domain introspect-via-tool tests (matching the existing convention).
- After the fixes: `102 passed` in `rustyred-thg-mcp` (the one red, `native_coordination`, was a separate co-mingled Codex change). Landed in `a4e7110`.
