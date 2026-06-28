# RustyRedCore-THG

RustyRedCore-THG is the multimodal substrate engine behind Theorem and Theseus: an in-process, durable, graph-native database with vector, relational/SQL, full-text, spatial, temporal, document-tree (filesystem), and cold-storage tiers, plus a PyO3 bridge that exports native accelerators to Python as `theseus_native`.

It is the Theseus-customized fork of RustyRed Core with THG (Theseus Hot Graph) protocol support. It is distinct from the standalone RustyRed-GraphDB OSS repo: this is the merger of the original Theseus Hot Graph atop the RustyRed substrate. One engine crate, `rustyred-thg-core`, is shared by every surface. The PyO3 module, the HTTP/gRPC/MCP product server, the Postgres-wire server, and direct Rust callers all execute the same store and command logic.

This README is the front door for this Cargo workspace. For the wider Theorem repo (the browser, RustyWeb apps, the harness console, deploy targets) read `../CLAUDE.md`, the navigation truth for the parent tree.

## At a glance

- Workspace version 0.5.0; Rust edition 2021; MSRV 1.85.
- One Cargo workspace: 37 member crates plus the root PyO3 cdylib. `cargo -p <crate>` works from this directory.
- Default product server: HTTP and gRPC merged on one port, 8380.
- Default storage: embedded RedCore (in-memory working graph plus append-only file and snapshots). In-memory and Redis-backed modes implement the same `GraphStore` trait.
- PyO3 wheel: `theseus_native` 0.4.0, built with maturin.
- Where this and the code disagree, the code wins. Source of truth for the data model is `crates/rustyred-thg-core/src/graph_store.rs`; for product-server config, `crates/rustyred-thg-server/src/config.rs`.

## One substrate, many modalities

Every modality lives on, or is reachable through, the core `GraphStore`. The store hosts the graph plus vector and ordered-index designations directly; full-text and spatial are standalone index primitives reached through the planner's `ModalityResolver` seam.

| Modality | What you get | Where it lives |
|----------|--------------|----------------|
| Property graph | `NodeRecord`/`EdgeRecord` with confidence-weighted epistemic edges; PageRank, personalized PageRank (cached), connected components, communities, bounded/weighted paths | `rustyred-thg-core` (`graph_store.rs`, `graph.rs`, `ppr_cache.rs`) |
| Vector | First-class designations: `designate_vector_property`, `vector_search`, `hybrid_search`. Exact normalized cosine by default; optional TurboVec acceleration. Not HNSW. | `rustyred-thg-core` (`graph_store.rs`; `vector-accelerated` feature) |
| Relational / SQL | Native planner (`QueryIr`, `execute_query`, roaring-bitmap intersections, `PlanTrace`), pluggable access methods, `NativeCatalog`, rank fusion (`FusionPolicy`: RRF default, weighted, cascade). A Postgres wire server lowers SQL to this planner. | `rustyred-thg-core` (`relational.rs`, `planner.rs`, `access_method.rs`, `ranking.rs`); `rustyred-thg-pg-server` |
| Full-text | Hand-rolled BM25 inverted index (`FullTextIndex`); optional Tantivy backend | `rustyred-thg-core` (`fulltext.rs`; `tantivy` feature) |
| Spatial | H3-cell index (`SpatialIndex`, via h3o); optional S2 backend; radius and bbox search | `rustyred-thg-core` (`spatial.rs`; `s2` feature); composed with time in `rustyred-thg-geotemporal` |
| Temporal | Bitemporal facts, node TTL, `time_series` access method, cursor-resumable working log | `rustyred-thg-core` (`working_log.rs`, TTL in `graph_store.rs`) |
| Filesystem | Copy-on-write document/folder tree (`DocTree`) over a content-addressed object store (`DiskObjectStore`); inline-or-overflow bodies | `rustyred-thg-core` (`doc_tree.rs`, `object_store.rs`); the durable backing for the FUSE and code-workspace apps under `../apps/` |
| Cold storage | Eviction tiers: keyed rehydration catalog (`ColdIndex`), coldest-first `EvictionFrontier`, columnar `cold_fragments` with zone maps; version-neutral `evict`/`readmit` so the PPR cache stays warm | `rustyred-thg-core` (`cold_index.rs`, `ordered.rs`, `cold_fragments.rs`); policy in `rustyred-thg-memory` |
| Images / binary | Arbitrary byte bodies content-addressed (`sha256:<hex>`) in the object store and doc tree | `rustyred-thg-core` (`object_store.rs`, `doc_tree.rs`) |
| Versioned graph | Git for graphs: Prolly-tree content objects, commits, refs/branches, diff, three-way merge | `rustyred-thg-core` (`versioned_graph.rs`) |
| CRDT | HLC-stamped structural convergence (`join_delta`/`diff_since`) for co-presence | `rustyred-thg-core` (`crdt/`); surfaced by `theorem-copresence` |
| Symbolic | Datalog derivation, e-graph dedup (egg), probabilistic source reliability, evolution archive | `rustyred-thg-core` (`symbolic.rs`, `epistemic.rs`) |
| Reactive hooks | Post-commit, off-critical-path, idempotent, depth-bounded compute (centrality, embeddings, epistemic passes) | `rustyred-thg-core` (`hooks.rs`) |

Backends swap behind env switches (`RUSTY_RED_FULLTEXT_BACKEND`, `RUSTY_RED_SPATIAL_BACKEND`) and Cargo features.

## Architecture

`rustyred-thg-core` defines the `GraphStore` trait and three implementations:

- `InMemoryGraphStore` is ephemeral and in-process. `InMemoryGraphStore::new()`. Used for tests and scratch.
- `RedCoreGraphStore` is durable, file-backed, and in-process (AOF plus snapshots). `RedCoreGraphStore::open(data_dir, RedCoreOptions)`; `RedCoreOptions::default()` is `AofEverysec`, use `AofAlways` for fsync-per-commit. This is the in-process substrate the browser and embedded apps persist to.
- `RedisGraphStore` connects to a Redis/RustyRed server (an out-of-process boundary). Feature-gated behind `redis-store`.

Gotcha: `RedCoreGraphStore` and `RedisGraphStore` expose inherent methods that shadow the trait methods with different (owned, fallible) signatures. To call the trait method on those types, use UFCS: `GraphStore::get_node(&store, id)`.

Every network surface calls the same core. The product server (`rustyred-thg-server`) merges axum HTTP and tonic gRPC onto one TCP listener: it sniffs `application/grpc*` to gRPC and routes everything else to HTTP, so both answer on port 8380. The Postgres-wire server lowers SQL to the planner; the MCP crate is a request handler served over stdio (the bundled harness plugin) and over HTTP (`POST /mcp` on the product server).

## The PyO3 bridge: theseus_native

The root crate (`src/`) is a `#[pymodule]` exported to Python as `theseus_native`. The name override in `src/lib.rs` is load-bearing: without it Python's `import theseus_native` falls silently into the slow Python fallback path. The module exports native accelerators that match exact Python signatures in Theseus, with the Python implementations kept as graceful fallbacks. Byte-parity is enforced by the `tests/*_parity.py` suite and the gates under `../apps/notebook/benchmarks/`.

Exports:

- `push_ppr`, `push_ppr_filtered`: ACL local-push personalized PageRank with lazy neighbor extraction (`src/push_ppr.rs`).
- `cmh_*`: Continuous Agent Memory Harness hash parity helpers (atom id, handoff state hash); the pure native owner is `theorem-harness-core::cmh`, while `src/cmh.rs` is only the Python ABI wrapper. Not the live memory store/read path, which lives in `theorem-harness-runtime`/`rustyred-thg-mcp` over `GraphStore`.
- `bgi_*`: stable hashing, receipt summaries, fact-pack hashing, probabilistic source reliability and expected value, evolution archive, receipt compaction. Deterministic JSON/hash contracts live in `theorem-harness-core::bgi`; engine-backed calls stay behind the Python ABI in `src/bgi.rs`.
- `search_*`: URL normalization, frontier scoring, score fusion, cosine top-k (`src/search_kernel.rs`).
- `graph_*`: id remap and edge packing (`src/graph_export.rs`).
- `rustyred_thg_expand_bounded`, `rustyred_thg_paths_shortest`, and the `RustyredThgCoreExecutor` class (`src/thg.rs`).
- LoRA adapter catalog/routing/fitness (`src/adapters.rs`).

## Crate map

Each crate has its own README with the public API surface. Group by role:

**Engine and bridge**

| Crate | Role |
|-------|------|
| root (`src/`) | The PyO3 cdylib exported as `theseus_native`. Native accelerators over the core. |
| [`rustyred-thg-core`](crates/rustyred-thg-core) | The substrate engine: `GraphStore` trait and three impls, RedCore durability, relational/SQL planner, vector/full-text/spatial/temporal, doc tree and object store, cold tiers, versioned graph, CRDT, symbolic engines, reactive hooks. |

**Storage, memory, embeddings**

| Crate | Role |
|-------|------|
| [`rustyred-thg-memory`](crates/rustyred-thg-memory) | Graph-native memory: PPR recall, consolidation, decay, validity/contradiction, and the cold-tier eviction/rehydration policy. |
| [`rustyred-thg-catalog`](crates/rustyred-thg-catalog) | sqlx/Postgres catalog for tenants, projects, billing, auth, and the cold index/scope rows. |
| [`rustyred-code-embedding`](crates/rustyred-code-embedding) | Shared code-embedding seam (`CodeEmbedder`): deterministic hash default, optional HTTP and local BGE backends. |

**Query surfaces and servers**

| Crate | Role |
|-------|------|
| [`rustyred-thg-server`](crates/rustyred-thg-server) | The product HTTP + gRPC + MCP surface (port 8380): tenant graph routes, query/Cypher, vector/full-text/spatial, harness coordination, browser actions, fractal expansion, TTL sweep. Ships the `rustyred-thg-upgrade-format` tool. |
| [`rustyred-thg-pg-server`](crates/rustyred-thg-pg-server) | Postgres wire-protocol server (default 6543): simple and extended protocol, SQL lowered to the planner. |
| [`rustyred-thg-mcp`](crates/rustyred-thg-mcp) | Native Rust MCP request handler: 108 flat tools plus a GraphQL transport, resources, and prompts over the `SharedStore` seam. Serves no transport itself. |
| [`rustyred-thg-resp-server`](crates/rustyred-thg-resp-server) | RESP/Redis-protocol server (default 6380) exposing scoped sorted-set commands over `OrderedIndexRegistry`. |
| [`rustyred-thg-compat-server`](crates/rustyred-thg-compat-server) | Minimal legacy HTTP control server (default 7379) over the core executor. |

**Geo and temporal**

| Crate | Role |
|-------|------|
| [`rustyred-thg-geotemporal`](crates/rustyred-thg-geotemporal) | Composes the core H3 spatial index with temporal filtering; registers the planner-facing `time_series` access method. |

**Retrieval and web**

| Crate | Role |
|-------|------|
| [`rustyred-web`](crates/rustyred-web) | RustyWeb: graph-native crawler and search kernel; page-to-graph emission, the crawl frontier, search providers, the eleven-stage epistemic fusion filter, and a Servo-free browser-automation contract. |
| [`rustyred-hipporag`](crates/rustyred-hipporag) | HippoRAG 2 candidate generation: phrase/hub schema, RAPTOR summary hubs, query-specific PPR retrieval. |
| [`rustyred-membrane`](crates/rustyred-membrane) | Context admission and eviction membrane: one gate fills a token budget and turns overflow into recoverable handles. |
| [`rustyred-rerank`](crates/rustyred-rerank) | Reranker `Scorer` impls for membrane admission: lexical and HTTP cross-encoders, listwise rerank. |
| [`rustyred-thg-fractal`](crates/rustyred-thg-fractal) | Native fractal expansion over the substrate and RustyWeb; admits web state into a quarantined low-trust tier. |

**Code intelligence and learned organs**

| Crate | Role |
|-------|------|
| [`rustyred-thg-code`](crates/rustyred-thg-code) | Code parsing runtime: parse repos into a code graph; CodeCrawler search/explain/explore, on-write centrality/embedding/epistemic hooks, context-pack membrane. |
| [`rustyred-thg-adapters`](crates/rustyred-thg-adapters) | LoRA adapter catalog and routing, plus the inference organs (Pairformer, HOT temporal, EdgeMPNN), the training substrate/runner, and grounded-skill output. Ships the `theorem_training_run` CLI. |
| [`rustyred-thg-ml`](crates/rustyred-thg-ml) | Shared graph-tensor and message-passing primitives; ColPali-style multi-vector retrieval. |

**CommonPlace (the personal-database lane)**

| Crate | Role |
|-------|------|
| [`commonplace`](crates/commonplace) | Consumer object model (Item/Collection/Tag/Task) and auto-structuring ingest, graph-native over the core. |
| [`rustyred-thg-intake`](crates/rustyred-thg-intake) | Source intake: the ingestion-spoke framework, a scoped incremental sync driver, curated and universal spokes (Gmail/GSuite/Outlook/Notion/Linear), and the NeedsYou act seam. |
| [`rustyred-thg-connectors`](crates/rustyred-thg-connectors) | Live MCP connector transport (stdio and HTTP/SSE): list a server's tools and register them as learnable affordances; gated invoke. |
| [`rustyred-thg-affordances`](crates/rustyred-thg-affordances) | Connector-as-substrate learning registry: MCP tools as `Affordance` nodes selected by PPR over outcome edges and time-decayed fitness. |

**Harness and agents**

| Crate | Role |
|-------|------|
| [`theorem-harness-core`](crates/theorem-harness-core) | Pure-logic harness kernel: guarded run state machine, state hashing, replay/fork, toolgraph selection, the composed-agent binding plane, the Job domain. No storage or network. |
| [`theorem-harness-runtime`](crates/theorem-harness-runtime) | GraphStore-backed runtime seam: persists transition receipts, the Dispatch v2 job board, durable coordination, graph-native memory, skill/engineering packs, live provider head invocation. |
| [`theorem-harness`](crates/theorem-harness) | SDK v2 Rust surface (run handles, sessions, idempotency, cancellation, resumable events, trace export); the source for generated Python/Node/Swift/WASM bindings. |
| [`theorem-dispatch`](crates/theorem-dispatch) | Postgres hot-execution queue: `FOR UPDATE SKIP LOCKED` claims, leases, retries, dead-letter. |
| [`theorem-receiver`](crates/theorem-receiver) | Dispatch v2 receiver: an outbound-only loop that spawns the local `claude`/`codex` CLI in a mapped worktree; `HeadAdapter` and `SandboxRuntime` seams. |
| [`theorem-agentd`](crates/theorem-agentd) | Local assistant daemon: OpenAI-compatible model loop, schema-guarded MCP tool host, receiver sidecar, capture/relay, compute-offload ledger. |
| [`theorem-browser-agent`](crates/theorem-browser-agent) | Servo-free browser-use perceive/govern/afford kernel: context command, perception bundle, gated action rail, browsing-run receipt. |
| [`theorem-copresence`](crates/theorem-copresence) | Headless co-presence peer and surface-adapter seam: structure converges on the graph CRDT, free text on yrs text regions, awareness on the working log. |
| [`pilot-core`](crates/pilot-core) | Servo-free, Playwright-class browser-automation core: locators, actionability/auto-wait, geometry snapshots, web-first assertions, behind a `BrowserDriver` trait. |

**Capability layer, scene output, checkers**

| Crate | Role |
|-------|------|
| [`ensemble`](crates/ensemble) | Pack-level capability registry, budgeted deterministic selector, trust ladder, and invocation-outcome fitness. |
| [`scene-os-core`](crates/scene-os-core) | SceneOS director: serde-only scene contracts, projection/chrome catalogs, `compile_scene_package` to a `ScenePackageV2`. |
| [`scene-os-web`](crates/scene-os-web) | SceneOS renderer: a `ScenePackageV2` to one self-contained HTML page, with script-safe payload escaping. |
| [`prose-check`](crates/prose-check) | Deterministic writing-engineering style checker and skill-pack payload. |
| [`design-check`](crates/design-check) | Static design-engineering checker (CSS, tokens, WCAG contrast, grid/type/motion) and skill-pack payload, now with the browserless Design Scout callable validator cut for normalized facts, audit/drift, token output, and HTML reports. |

**Standalone (not a workspace member)**

| Crate | Role |
|-------|------|
| [`reconstruction-engine`](crates/reconstruction-engine) | Generative reconstruction engine (the generative SceneOS projection class). Its own single-crate workspace; needs the sibling `our-civic-atlas-backend` checkout. Build from inside the crate dir. |

## Build and test

This directory is a Cargo workspace, so `-p <crate>` works here:

```bash
# Type-check the whole workspace
cargo check --workspace

# Test a single crate
cargo test -p rustyred-thg-core
cargo test -p theorem-harness-core

# Compile-only coherence check
cargo test --no-run -p rustyred-thg-core

# Build the PyO3 wheel (theseus_native) into the active venv
maturin develop

# PyO3 byte-parity gates (mirror of Theseus inference)
python -m pytest tests/            # test_*_parity.py, test_smoke.py
```

Notable Cargo features:

- `rustyred-thg-core`: `redis-store` (the Redis backend), `vector-accelerated` (TurboVec), `tantivy` (alternative full-text backend), `s2` (alternative spatial backend). All default-off; the default build is exact cosine, hand-rolled BM25, and H3.
- `rustyred-thg-adapters`: `pairformer-burn-cubecl` pulls Burn 0.21 / CubeCL 0.10, which need rustc 1.92 or newer (above the crate's declared MSRV). The default build is the deterministic reference path and builds on 1.85.

The release binary is `rustyred-thg-server`; the `Dockerfile` builds only that crate and copy-renames it to `rusty-red-graph-server` for the Railway service.

## Servers and ports

| Server | Default bind | Selector |
|--------|--------------|----------|
| `rustyred-thg-server` (HTTP + gRPC + MCP) | 8380 | `PORT` / `RUSTY_RED_PORT`; `RUSTY_RED_MODE` selects embedded/memory/redis |
| `rustyred-thg-pg-server` (Postgres wire) | 6543 | `RUSTYRED_THG_PG_ADDR`; `RUSTYRED_THG_PG_MODE` (relational default / executor) |
| `rustyred-thg-resp-server` (RESP) | 6380 | `RUSTYRED_THG_RESP_ADDR` |
| `rustyred-thg-compat-server` (legacy HTTP) | 7379 | `RUSTYRED_THG_PORT` / `PORT` |

Each prints a `RUSTYRED_THG_*_READY <addr>` line on bind.

## Conventions

- Mirror discipline. This workspace is the Rust projection of the canonical Theseus application. Keep parity receipts and the PyO3 surface reconciled with Theseus; surface drift, do not bury it. See `../Theseus/Theorem.md`.
- No emojis. No em or en dashes (use colons, parens, semicolons).
- Per-crate READMEs carry the public API surface and test counts; this file is the map.
- Per-strand status lives in `../docs/reference/status-current-direction.md`.
