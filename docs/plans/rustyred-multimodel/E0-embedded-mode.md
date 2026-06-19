# E0: Embedded Mode -- Execution Handoff

A `[spec to write]` unit from `NORTH-STAR-RUSTYRED-MULTIMODEL.md` (Area E). The map
says: "writing it is itself a planning unit that comes before its execution." This
is that handoff. It turns the North Star's headline "done" sentence --

> An agent links the crate or drops a single binary, runs locally, and has graph,
> relational, document, vector, image, and time in one process over local files,
> no server required.

-- into buildable units with acceptance criteria, grounded in the live tree.

## The E0 target (from the map)

> **E0 embedded mode.** The core as a linkable library and a single local binary;
> capabilities as local calls; the folder tree as the agent's working filesystem;
> a local persistence path with restart rehydration; single-tenant local config.
> Acceptance: an agent runs the engine in-process over a local directory,
> restarts, and rehydrates.

Dependency edges (from the map's "what unblocks what"): **B3 (document and folder
tier) precedes E0** (the folder tree is the filesystem); **E0 precedes E1 (stream
transport) and E2 (GL-Fusion native path)** (local streams and native graph
conditioning land most cleanly embedded).

## The landed foundation (verified against the live tree, 2026-06-19)

E0 is mostly an *assembly + entrypoint + config* unit, not a from-scratch build:
the pieces exist and are already in-process.

- **The durable in-process store is real.** `rustyred-thg-core/src/graph_store.rs`:
  `RedCoreGraphStore::open(data_dir, RedCoreOptions)` is the durable, file-backed,
  in-process substrate (AOF + snapshots); `recover()` rebuilds the in-memory
  mirror from the AOF on open; `RedCoreGraphStore::memory()` is the ephemeral
  variant. `RedCoreOptions::default()` is `AofEverysec` (use `AofAlways` for
  fsync-per-commit determinism). This IS "the in-process substrate with no API
  boundary" the embedded agent persists to. **Restart rehydration already works at
  the store layer** (`open` -> `recover`); E0's job is to prove it end-to-end
  through the agent's surface and wire the folder tree on top.
- **Capabilities are already in-process, no server required.** The
  `GraphStore` trait + `RedCoreGraphStore` give graph/relational/vector/spatial/
  full-text/time as in-process calls. The relational core (`access_method.rs`,
  `planner.rs`, `relational.rs`), the cold tier (`object_store.rs`,
  `cold_index.rs`, `ordered.rs`), Turbovec (vector), `fulltext_tantivy` (FTS), and
  the geotemporal `time_series` access method are all linkable.
- **The typed agent surface is in-process.** Area A (A0-A7, COMPLETE) made the
  `rustyred-thg-mcp` GraphQL surface a synchronous, in-process dispatch over any
  `McpGraphBackend` (the document executes via `futures_executor::block_on` on the
  calling thread -- no async runtime, no socket). `graphql_default_surface` makes
  GraphQL the single advertised agent path. This is the "all capability reaches
  agents through one typed surface" half of "done", already embeddable.
- **A local-host candidate exists.** `theorem-agentd` (the local assistant daemon:
  OpenAI-compatible model loop + schema-guarded MCP tool host) is the closest
  thing to the single local binary; E0 should evaluate hosting the embedded engine
  there vs. a dedicated thin binary.
- **The PyO3 bridge** (`rustyredcore_THG/src/lib.rs`, exported as
  `theseus_native`) already links the core into a host process; the embedded story
  is the native analogue.

## The gap (what E0 must actually build)

1. **A single local binary / linkable entrypoint.** A `rustyred` (or
   `theorem-embedded`) crate exposing: (a) `lib` -- `Engine::open(dir, config) ->
   Engine` that owns a `RedCoreGraphStore` + the registered access methods + the
   in-process GraphQL invoker, with typed `query`/`mutate`/`introspect` methods
   that are just local calls into the existing `execute_graphql`; and (b) `bin` --
   a single binary that opens an `Engine` over a local directory and exposes it
   over a **local stdio MCP transport** (no TCP, no HTTP), so a Claude Code/Codex
   session can attach to a project with `no server required`.
2. **Single-tenant local config.** A local config (TOML at the project root, e.g.
   `theorem.toml`) carrying the data dir, the tenant slug (single-tenant local
   default), durability mode, and `graphql_default_surface`. Reuse / extend
   `McpServerConfig`; do not invent a parallel config type.
3. **The folder tree as the working filesystem (depends on B3).** Wire
   `doc_tree.rs` (the CoW B-tree path-keyed document namespace over the object
   store, B3) so the project's files ARE the agent's filesystem: a written file
   gains a content hash, an embedding, and a graph node (B3's acceptance), and the
   embedded engine navigates it by prefix range scan + point lookup. E0 consumes
   B3; if B3's `DocTree`-as-`ColdIndex` wiring is still open (flagged in CLAUDE.md),
   E0 surfaces that as its blocking dependency rather than papering over it.
4. **End-to-end restart rehydration.** The store-level `open`->`recover` is proven;
   E0 must prove the *agent-visible* round-trip: write through the embedded
   GraphQL surface, drop the `Engine`, re-`open` over the same directory, and read
   the same data back through the surface.

## Work units (each with acceptance)

- **E0.1 -- `Engine` linkable core.** New `apps/rustyred-embedded` (or a core
  `embedded` module): `Engine::open(dir, EmbeddedConfig)` owning the
  `RedCoreGraphStore` + access-method registry + GraphQL invoker; `engine.query(gql)`
  / `engine.mutate(gql)` / `engine.introspect()` as local calls lowering to the
  existing `execute_graphql`. Acceptance: a Rust test links the crate, opens an
  engine over a temp dir, runs `bulkNodes` (mutate) then `neighbors`/`graphAlgorithm`
  (query) entirely in-process, no socket.
- **E0.2 -- restart rehydration round-trip.** Acceptance: write through the engine,
  drop it, re-open over the same dir, and a query returns the written data (the
  map's headline acceptance: "runs in-process over a local directory, restarts, and
  rehydrates"). Use `AofAlways` in the test for determinism.
- **E0.3 -- single binary + local stdio MCP transport.** A `bin` that opens an
  Engine and serves the existing MCP tool surface over stdio (newline-delimited
  JSON-RPC; no TCP/HTTP). Acceptance: a smoke test pipes `initialize` +
  `tools/call graphql_query` over stdio and gets the in-process result; the binary
  links no server crate (`cargo tree` shows no axum/tonic).
- **E0.4 -- single-tenant local config.** `theorem.toml` at the project root ->
  `EmbeddedConfig` (data dir, tenant, durability, `graphql_default_surface`).
  Acceptance: the binary reads the config, applies the tenant as the connection
  tenant (the GraphQL surface rejects an empty tenant -- already enforced), and an
  absent config falls back to a documented single-tenant local default.
- **E0.5 -- folder tree wiring (consumes B3).** Expose `ls`/`tree`/`read`/`write`/
  `move`/`mkdir` over `doc_tree.rs` through the engine, and on `write` emit the
  content hash + embedding + graph node. Acceptance: a written file is found by
  prefix range scan AND re-found as a graph node by label (mirrors B3's AC7).
  Blocked-on note: if B3's `DocTree: ColdIndex` wiring is open, E0.5 is gated on it.

## Risks / coordination

- **Coordination.** At planning time, the core crates (`rustyred-thg-core`, the
  relational/cold/doc tiers) were Codex-active and carried large uncommitted
  work; E0.1-E0.4 should live in a NEW crate (`apps/rustyred-embedded`) to stay
  collision-free, reaching into the core only through its public `GraphStore` /
  `execute_graphql` seams. E0.5 touches `doc_tree.rs` (B3, Codex lane) --
  coordinate before editing; prefer consuming B3's public API over modifying it.
- **No new config type.** Extend/wrap `McpServerConfig`; a parallel config is the
  anti-pattern.
- **Single binary discipline.** The acceptance bar "no server required" is a real
  gate: the embedded binary must not link axum/tonic. Verify with `cargo tree`.
- **B3 dependency is real, not nominal.** E0.5 cannot fake the
  filesystem-as-substrate; if B3's folder API isn't wired, surface it as the
  blocker and build E0.1-E0.4 (which only need the store + GraphQL surface, both
  landed) first.

## Why this unit now

C2 (SQL/PGQ) is blocked on B0 (graph-as-access-method); the planner today registers
only `Ordered` + `TimeSeries` access methods, no graph traversal. B0/B4/D1 are
heavy reworks in Codex-active core crates. E0 is the North Star's headline "done"
criterion, its execution units (E0.1-E0.4) only need the already-landed store +
GraphQL surface, and writing this handoff is itself the on-map E0 planning unit
that unblocks E0 execution + E1 + E2.
