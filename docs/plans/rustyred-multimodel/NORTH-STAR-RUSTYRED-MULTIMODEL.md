# RustyRed Multi-Model Database: North Star

A map, not a handoff. It states the whole target, the landed foundation, and the
work areas as units an agent loop can pick up, each with observable acceptance
and the spec that holds its detail. The detailed execution specs are the
enumerated register; this is the looser one that orders them.

## The target
RustyRed as the embedded, local-first, multi-model database the agent inhabits.
One engine that is natively graph, relational, document, vector, image, and
time, queried through a single typed GraphQL surface, running embedded in the
agent's environment the way SQLite does, with its own models (GL-Fusion) reading
the graph in their forward pass. The agent's files, memory, and knowledge live
in it and gain versioning, semantic search, graph structure, and memory as
properties of the filesystem.

## What "done" looks like for the whole
- An agent links the crate or drops a single binary, runs locally, and has
  graph, relational, document, vector, image, and time in one process over local
  files, no server required.
- All capability reaches agents through one typed GraphQL surface
  (`graphql_query` / `graphql_mutate` / `graphql_introspect`); the flat tools are
  retired behind it.
- One pg-wire endpoint serves the native views and federates auth and billing to
  Neon.
- The folder tree is the agent's working filesystem; a Claude Code or Codex
  session runs inside a project stored in the harness and that project gains
  versioning, embeddings, and graph structure for free.
- GL-Fusion models condition on the local graph directly.

## The landed foundation (the loop's starting point, from the live tree)
- `ThgExecutor` in-process core plus `InMemoryThgExecutor`; the compat, resp, and
  pg servers are wire adapters over it.
- `rustyred-thg-catalog` (the relational catalog) is present.
- `DiskObjectStore` (content-addressed local files); `ordered.rs` (the COW B-tree
  and `EvictionFrontier`); Turbovec (vector); `fulltext_tantivy` (FTS).
- `rustyred-thg-mcp/src/lib.rs` holds the 100-plus flat tools, the surface to
  collapse.
- `theorem-gateway` is the browser GraphQL-over-gRPC server, the async-graphql
  pattern to copy.
- `rustyred-thg-geotemporal`, `rustyred-thg-code`, `rustyred-thg-pg-server`
  exist as scaffolds.

## How to work this doc (the loop)
- A unit is ready when its dependency units are landed. Pick a ready unit,
  complete it to its acceptance criteria, verify against the running engine,
  record the result through the harness, take the next ready unit.
- Each unit names the spec that holds its detail. Read that spec before working
  the unit. Units marked `[spec to write]` need their execution handoff written
  first; writing it is itself a planning unit.
- Units inside an area can usually proceed in parallel across heads. Announce
  footprints through the harness room and reconcile concrete edits there.
- The execution specs and this map live in the repo (`docs/plans/`) so the loop
  reads them in-tree. Commit them there.

## Area A: the GraphQL MCP surface
Collapse the flat tools into one typed surface, slice by slice, resolvers
lowering to the existing handlers, coexisting with the flat tools until each
domain is covered. Spec: `SPEC-GRAPHQL-MCP.md`.

- **A0 scaffold.** The three transport tools; the schema module and resolver
  seam in `rustyred-thg-mcp` mirroring `theorem-gateway/src/schema/`; tenant
  resolved from connection context through the one consolidated normalizer.
  Acceptance: an empty schema serves introspection; a session opened with an
  empty tenant is rejected.
- **A1 memory** (specced). `MemoryDoc` with `related` and `links`; the three
  queries and four mutations; lowering to the memory handlers. Acceptance: the
  criteria in `SPEC-GRAPHQL-MCP.md`.
- **A2 coordination.** `Room` over messages, intents, records, presence,
  mentions; the multihead `WorkGraph` and `TaskNode`; the seam where the stream
  transport attaches. Spec: `[spec to write]` plus
  `SPEC-RUSTYRED-STREAM-COORDINATION.md` for the transport. Acceptance: a room
  query returns messages, intents, and records in one response; a mutation
  publishes an event another head reads; the same event rides the stream seam.
- **A3 graph.** `graphAlgorithm(kind: PAGERANK | PPR | COMMUNITIES | COMPONENTS,
  ...)` collapsing the eight algorithm tools; `graphNode`, `neighbors`,
  `schema`; `vectorSearch`, `spatialSearch`, `fulltextSearch` with a designate
  mutation each; bulk upserts; symbolic (`deriveFacts`, `sourceReliability`,
  `expectedValue`). Acceptance: `graphAlgorithm(kind: PPR)` matches the
  `rustyred_thg_algorithm_ppr` tool; switching `kind` switches algorithm from one
  field.
- **A4 epistemic.** `epistemicNeighbors`, frontier, compile, enrich,
  `hippoRetrieve`, over the shadow graph. Acceptance: an epistemic-neighbor query
  returns the shadow nodes a `rustyred_thg_epistemic_neighbors` call returns.
- **A5 code.** `codeSearch`, `codeContext`, `codeExplore`, `codeExplain`,
  `ingestCodebase`, the harness KG fields. Acceptance: a code query returns what
  the matching `compute_code` operation returns.
- **A6 remaining clusters.** Web and browse, affordance gateway, ensemble,
  skills, dispatch, harness run, each as typed fields. Acceptance: each cluster's
  representative operation answers through GraphQL.
- **A7 cutover.** Hide covered flat tools from the advertised surface; GraphQL is
  the default agent path. Acceptance: an agent completes a full task through
  GraphQL alone.

## Area B: the relational and storage engine
The unified object model, the access-method seam, and the planner, so graph,
relational, document, vector, spatial, and time are access methods over one
store. Specs: `SPEC-RUSTYRED-RELATIONAL-CORE.md` (with the sections 3 and 6
full-physical-unification rework), `SPEC-RUSTYRED-DOCUMENT-TIER.md`.

- **B0 full physical unification.** A node is a row, an edge is a row plus a
  `graph_csr` adjacency access method, native rows live in the same store;
  `graph_store` responsibilities move into the unified store. `[rework sections 3
  and 6]`. Acceptance: a node, an edge, and a catalog row read from one store; a
  traversal runs as an access method over edge-rows.
- **B1 access-method seam plus planner.** The `AccessMethod` trait over
  `plugin.rs` and the cost-based planner as the core build (`planner.rs`).
  Acceptance: a query routes through the planner to the chosen access method.
- **B2 native ColdIndex** over `ordered.rs`, retiring the Postgres `cold_index`.
  Acceptance: cold lookups resolve natively with no Postgres call.
- **B3 document and folder tier.** `doc_tree.rs`: a COW B-tree keyed by path over
  the object store; `ls`, `tree`, `read`, `write`, `move`, `mkdir`. Acceptance:
  the folder tree navigates by prefix range scan and point lookup; a written file
  gains a content hash, an embedding, and a graph node.
- **B4 cold columnar and N-d array tier** (TileDB-format) with a zstd filter and
  first-class 1D array types in the core and SQL. Acceptance: a large array
  stores and slices from the cold tier; embeddings store as float arrays.
- **B5 time-series access method** plus bi-temporal decay-versus-invalidate.
  Acceptance: a time-range query and a point-in-time read both resolve.

## Area C: the wire and query-language surface
Spec: `SPEC-RUSTYRED-PG-WIRE.md`.

- **C0 pg-wire adapter** (`rustyred-thg-pg-server`): handshake plus the simple
  and extended query loops; SQL frontend (sqlparser-rs) lowering to the planner
  IR and the access-method seam. Acceptance: a psql client connects and runs a
  scoped query.
- **C1 federation.** FDW-style passthrough to Neon for auth, billing, and the
  escape hatch; one endpoint serves native and forwards the rest. Acceptance: the
  app and ORM see one database; a native view and a Neon-backed table both answer
  through it.
- **C2 SQL/PGQ** (graph MATCH inside SQL) over the same surface; GQL optional and
  standalone later. Acceptance: a MATCH clause traverses the graph through the
  same endpoint.
- **C3 SeaORM** as the application data layer over both stores via pg-wire.
  Acceptance: one ORM addresses Neon auth tables and RustyRed views.

## Area D: the modality subsystems
- **D0 image and visual subsystem.** Cold-tier image storage; SigLIP 2 embed and
  zero-shot classify on ingest, auto-filing into the folder tree; the permissive
  OmniParser replacement (an RF-DETR, RT-DETR, or YOLOX detector plus a permissive
  captioner, trained on RunPod) as a parse-and-verify capability whose output
  stores as structured nodes on the image. Spec: `[spec to write]`. Acceptance: a
  dropped image is embedded, classified, filed, and similarity-searchable; a UI
  screenshot parses into structured element nodes.
- **D1 geo and spatiotemporal.** The S2 geotemporal plugin as a joinable access
  method so geo, time, and relational compose in one planner query. Acceptance: a
  single query joins a geo-within predicate, a time range, and a relational
  filter.
- **D2 LiDAR.** Sparse 3D arrays in the cold tier, an octree index, neural
  compression. Spec: `[spec to write, its own]`. Acceptance: a point cloud
  ingests, indexes by octree, and answers a spatial range query.
- **D3 spatiotemporal GNN.** Train Theseus's existing ST-GNN over Theorem's
  graph; port to Rust (ONNX or candle forward pass) only if native inference
  latency becomes load-bearing. Acceptance: the trained model scores the graph;
  the port is deferred until measured need.

## Area E: local-embedded and the model tier
- **E0 embedded mode.** The core as a linkable library and a single local binary;
  capabilities as local calls; the folder tree as the agent's working filesystem;
  a local persistence path with restart rehydration; single-tenant local config.
  Spec: `[spec to write, the embedded-mode north-star]`. Acceptance: an agent
  runs the engine in-process over a local directory, restarts, and rehydrates.
- **E1 stream transport.** Passive subscribe, intentional ping, selective
  attention; a local live-tail for warm heads and the console. Spec:
  `SPEC-RUSTYRED-STREAM-COORDINATION.md`. Acceptance: the criteria in that spec.
- **E2 GL-Fusion native path.** The model conditions on the local graph in its
  forward pass. Acceptance: a GL-Fusion generation reads graph structure with no
  tokenization step.
- **E3 learned indexes** over the cold and static read path, slotting into the
  access-method seam, fit per deployment. Acceptance: a learned index answers
  cold-tier lookups and falls back cleanly on miss. (See the learned-index thread
  for the design.)
- **E4 (exploration) neural working memory.** A Titans-style memory module wired
  into a GL-Fusion model, trained on RunPod. Held next to the GNN work as model-
  tier research, not a database layer.

## Area F: memory and compound engineering
Specs: `SPEC-MEMORY-FOUR-LAYER.md`, `compound-engineering-corpora-backlog.md`,
`SPEC-SKILLOPT-BORROWS.md`.

- **F0 the four memory types:** episodic always-write, epistemic shadow,
  procedural (skills plus compound engineering), user-model. Acceptance: a raw
  episode is written every session and compiles into the other layers.
- **F1 the compounding compiler** (episodic into procedural) plus the SkillOpt
  borrows. Acceptance: an outcome signal promotes or demotes a procedural
  artifact.

## What unblocks what (relationships, not a timed order)
- A0 precedes A1 through A7.
- B0 and B1 underlie B2 through B5, C0, D1, and E3, since each is an access method
  over the unified store through the seam.
- C0 precedes C1, C2, C3.
- B3 precedes D0 (image storage reuses the cold and folder tier) and E0 (the
  folder tree is the filesystem).
- E0 precedes E1 and E2 (local streams and native graph conditioning land most
  cleanly embedded).
- A2 and E1 share the stream seam; build them aware of each other.
- The specs marked `[spec to write]` (A2 coordination slice, D0 visual, D2 LiDAR,
  E0 embedded mode) gate their units; writing each is a planning unit that comes
  before its execution.
