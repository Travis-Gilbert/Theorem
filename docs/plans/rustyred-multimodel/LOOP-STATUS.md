# RustyRed Multi-Model North Star -- Loop Status

A consolidation of the `/harness build NORTH-STAR-RUSTYRED-MULTIMODEL on loop`
run (Claude Code, 2026-06-19, 13 iterations). It records what landed, what is
genuinely gated, and the next phase that needs Travis's direction. The running
detail is in the memory file `north-star-loop-a3-graph-graphql.md`.

At loop handoff, all work below was **verified green + clippy-clean** but left
uncommitted because the loop never force-commits a shared tree. This consolidated
ship branch is the commit path for that reviewed slice.

## North Star "done" scorecard

The spec's "What done looks like for the whole" has five criteria:

1. **Link the crate / drop a single binary, run locally, graph + relational +
   document + vector + image + time in one process over local files, no server.**
   → **MET** for graph/relational/document/vector/time (image is D0, gated). The
   `apps/rustyred-embedded` `Engine` + single stdio binary do exactly this.
2. **All capability through one typed GraphQL surface; flat tools retired behind
   it.** → **MET.** Area A (A0-A7): 8 domains over one async-graphql schema +
   the `graphql_default_surface` cutover flag.
3. **One pg-wire endpoint serves native views + federates auth/billing to Neon.**
   → **PARTIAL.** C0 (pg-wire over native views) was already shipped; C1 (Neon
   FDW federation) is **gated on a live Neon** (deploy).
4. **The folder tree is the agent's working filesystem; a project gains
   versioning, embeddings, and graph structure for free.** → **MET.** E0.5: a
   written file gains a content hash + a (deterministic offline) embedding + a
   graph node, navigable by point lookup, prefix scan, and vector search,
   durable across restart.
5. **GL-Fusion models condition on the local graph directly.** → **GATED.** E2 is
   model-tier (the model reads the graph in its forward pass, no tokenization) --
   not a Rust-crate build. The `GLFUSION_URL` HTTP client in `theorem-gateway`
   is a tokenized request path, not the native one.

**3 of 5 met by CC's solo, low-collision lane; 2 genuinely gated.**

## What landed (this loop)

### Area A -- the GraphQL MCP surface (COMPLETE, A0-A7)
`rustyredcore_THG/crates/rustyred-thg-mcp/src/graphql/` -- one typed schema
collapsing ~100 flat MCP tools, slice by slice, resolvers lowering to the
existing `*_payload` handlers:
- A1 memory, **A3 graph** (graphAlgorithm + graphNode/neighbors/graphSchema/
  vector*/fulltext/spatial*/symbolic + designate*/bulk*), **A4 epistemic**,
  **A5 code** (CodeCrawler) + **A5-kg** (harness instant-KG), **A6 clusters**
  (ensemble/skills/jobs/harness-run) -- all CC. A2 coordination was built by
  Codex in parallel on the same module.
- **A7 cutover**: a default-off `McpServerConfig.graphql_default_surface` +
  `GRAPHQL_COVERED_FLAT_TOOLS` const + a `tool_definitions` filter; an agent can
  complete a full task through GraphQL alone.
- 91 mcp tests green. Honest divergences logged: web/browse/fractal +
  hippoRetrieve are server-only (not wrapped); jobs/code are RedCore-backed
  (routing-proven on the in-memory fixture).

### Area E -- embedded mode (COMPLETE, E0.1-E0.5 + E1)
`apps/rustyred-embedded/` -- a NEW standalone crate (zero collision):
- **E0.1** `Engine::open(dir, EmbeddedConfig)` over the in-process `SharedStore<S>`
  seam (added to `rustyred-thg-mcp`: a cheap `Rc<RefCell<S>>` handle that
  forwards all 42 `McpGraphBackend` methods, so a non-Clone durable
  `RedCoreGraphStore` drives the full GraphQL surface in-process, no socket).
- **E0.2** restart rehydration (write → drop → re-open → read back).
- **E0.3** single stdio binary (`src/main.rs`, newline-delimited JSON-RPC over
  stdin/stdout, no axum/tonic/socket -- "no server required").
- **E0.4** `theorem.toml` config loader.
- **E0.5** folder tree (`fs_write`/`fs_read`/`fs_ls` over a serde-persisted
  `DocTree` + `DiskObjectStore`; a written file gains content hash + embedding +
  graph node, vector-searchable, durable across restart).
- **E1** stream transport (stream_publish→stream_read live-tail through the
  embedded engine).
- 11 tests green, clippy-clean.

### Planning unit
`docs/plans/rustyred-multimodel/E0-embedded-mode.md` (the E0 [spec to write]).

## What is gated (needs Travis's direction)

| Unit | Status | Gate |
|---|---|---|
| **C1** Neon federation | unbuilt | a live Neon instance (deploy) |
| **E2** GL-Fusion native path | unbuilt | model-tier (model reads graph in forward pass); not a Rust-crate build |
| **B0** full physical unification | unbuilt | heavy core rework in `rustyred-thg-core` -- Codex-active lane (collision) |
| **B4** cold columnar / N-d array tier | unbuilt | core (TileDB-format, array types) -- Codex lane |
| **D1** geo joinable access method | unbuilt | core + geotemporal -- Codex lane |
| **C2** SQL/PGQ (graph MATCH in SQL) | unbuilt | blocked on B0 (the planner has no graph-traversal access method) |
| **C3** SeaORM app layer | unbuilt | over C1 |
| **D0** visual / **D2** LiDAR | spec-to-write | zero-collision, but speculative (little code to ground a spec yet) |
| **E3/E4** learned indexes / neural memory | exploration | model-tier |
| **F0/F1** memory four-layer / compounding | partial | episodic + epistemic-shadow landed; rest in Codex core/harness lanes |

## Recommended next phase (operator choice)

1. **Coordinate with Codex** on the heavy core units (B0 unification → unblocks
   C2; B4 array tier; D1 geo-join). These are in the co-mingled `rustyred-thg-core`
   lane and want a claimed split, not a solo CC edit.
2. **Provision a Neon instance** to build + test C1 FDW federation (the pg-wire
   endpoint federating auth/billing).
3. **Model work** for E2 GL-Fusion native conditioning (RunPod / the GL-Fusion
   model itself reading the local graph).
4. **Re-invoke `/loop`** to have CC write the D0 (visual/SigLIP) and D2 (LiDAR)
   `[spec to write]` planning handoffs (zero-collision), or to take any newly-
   unblocked unit.

## Commit posture

The loop originally left everything uncommitted by design: the Area A/E MCP edits
live in `rustyred-thg-mcp/src/lib.rs`, co-mingled with another head's work, so no
pathspec commit could cleanly isolate the loop's changes. This consolidated ship
branch lands the reviewed slice together. `apps/rustyred-embedded` remains the
clean standalone crate; its only cross-file dependency is the additive
`SharedStore<S>` in `rustyred-thg-mcp/src/lib.rs`.
