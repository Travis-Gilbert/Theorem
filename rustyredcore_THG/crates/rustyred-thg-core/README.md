# rustyred-thg-core

The substrate engine: a multimodal, in-process, graph-native database with no Django, Python, network-server, or tokio dependency. Every Theorem surface (the PyO3 module, the product server, the Postgres-wire server, direct Rust callers) executes this crate.

It is a property-graph core plus durable file-backed storage, a content-addressed cold-tier storage spine, a copy-on-write document/folder tree, a native relational/SQL planner with pluggable access methods and rank fusion, TurboVec quantized vector search for accelerated builds, BM25 full-text and H3 spatial primitives, temporal/TTL facilities, git-style versioned-graph commits, post-commit reactive hooks, a CRDT layer, stream logs, and a family of symbolic engines.

## The three GraphStore impls

`GraphStore` (`graph_store.rs`) is the shared trait. Required methods include `upsert_node`, `upsert_edge`, `get_node`, `get_edge`, `query_nodes`, `neighbors`, `stats`, `verify`, `rebuild_indexes`; provided methods cover TTL, version-neutral residency (`evict_node`/`readmit_node`/`evict_edge`/`readmit_edge`), and `graph_snapshot`.

- `InMemoryGraphStore::new()` is ephemeral and in-process; it carries the inherent vector/ordered designation API.
- `RedCoreGraphStore::open(data_dir, RedCoreOptions)` is durable (AOF plus snapshots). `RedCoreGraphStore::memory()` is a non-durable variant. `RedCoreDurability`: `None`, `AofEverysec` (default), `AofAlways`, `SnapshotOnly`. On open it locks the dir and runs `recover()` (snapshot then AOF replay; orphan edges tolerated). A full snapshot is written every `snapshot_interval_writes` mutations (default 1000). On-disk format is `CURRENT_FORMAT_VERSION` = 1.
- `RedisGraphStore::new(url, prefix)` / `::tenant(...)` is out-of-process. Feature-gated behind `redis-store`.

UFCS gotcha: `RedCoreGraphStore` and `RedisGraphStore` define inherent `get_node`/`upsert_node`/etc. that shadow the trait methods with owned, fallible signatures. To call the trait method, write `GraphStore::get_node(&store, id)`.

## Modalities and key API

- Graph: `NodeRecord`, `EdgeRecord` (`confidence`, `epistemic_type`, `provenance`, `content_hash`, `parent_hashes`), `NeighborQuery`, `NodeQuery`, `GraphStats`. Algorithms (`graph.rs`): `pagerank`, `personalized_pagerank`, `connected_components`, `label_propagation_communities`, `expand_bounded[_weighted]`, `paths_shortest[_weighted]`. Cached PPR in `ppr_cache.rs` (`cached_personalized_pagerank`, keyed on `stats().version`). CSR adjacency in `graph_csr.rs`.
- Vector: `designate_vector_property(label, property, dimension)`, `vector_search(label, property, query, k)`, `hybrid_search` / `hybrid_search_with_config` (`HybridScoringConfig`). The default build keeps the legacy exact normalized-cosine path for local/dev use. The `vector-accelerated` feature is the resident production path: every vector designation builds a TurboVec quantized index at a configured 2- or 4-bit width, records that width in the index manifest, rejects unsupported dimensions at designation time, and does not retain a resident full-precision corpus or exact rerank fallback. This is not HNSW.
- Relational / SQL: `relational.rs` (`RelationalStore`, `RelationSchema`, `NativeCatalog`, `from_graph_snapshot`), `access_method.rs` (`AccessMethod`, `AccessMethodRegistry::with_native_defaults`, `Predicate`, `ModalityResolver`), `planner.rs` (`QueryIr`, `execute_query`, `execute_query_with_resolver`, `compile_graphql_selection`, `FusionPolicy::{Rrf, Weighted, Cascade}`), `ranking.rs` (`RankingRule`, `apply_cascade`, `EpistemicGate`).
- Full-text: `fulltext.rs` (`FullTextIndex`, BM25 k1=1.2 b=0.75; `make_fulltext_backend`). A standalone primitive reached via `ModalityResolver`, not a store method. Optional `tantivy` backend.
- Spatial: `spatial.rs` (`SpatialIndex` over h3o, `radius_search`/`bbox_search`). Standalone primitive via `ModalityResolver`. Optional `s2` backend.
- Temporal: `working_log.rs` (`TemporalFact` bitemporal, `WorkingLog` cursor-resumable). Node TTL (`TTL_PROPERTY`, `purge_expired_nodes`). `TimeSeriesAccessMethod` for range predicates.
- Filesystem: `doc_tree.rs` (`DocTree` copy-on-write B-tree, inline below 4096 bytes zstd then overflow, `put_body`/`resolve_body`), `object_store.rs` (`ColdObjectStore`, `DiskObjectStore` git-object-shaped, restart-durable). sha256 content addressing.
- Cold storage: `cold_index.rs` (`ColdIndex`, `InMemory`/`Disk`/`Ordered`), `ordered.rs` (`OrderedIndex` zset-shaped, `EvictionFrontier` coldest-first O(k log n)), `cold_fragments.rs` (columnar fragments with zone maps, `Zstandard`/`DoubleDelta`/`RunLength`). Residency changes do not bump `stats().version`.
- Images / binary: arbitrary byte bodies content-addressed in the object store and doc tree (`sha256:<hex>`); inline-or-overflow.
- Versioned graph: `versioned_graph.rs` (`build_prolly_tree`, `compile_graph_pack`, `checkout_graph_version`, `diff_graph_snapshots`, `merge_graph_snapshots`, `GraphCommit`, refs/branches default `main`).
- CRDT: `crdt/` (`join_delta`, `diff_since`, `Hlc`, `VersionVector`, `StampedBatch`, `JoinReport`).
- Streams: `stream.rs` (`StreamLog`, `StreamRegistry`, live-tail events).
- Plugins: `plugin.rs` (`RustyRedPlugin`, `PluginRegistry`, on-demand compute, the twin of hooks).
- Instant KG: `instant_kg.rs` (`HarnessInstantKg`, code KG ingest plus impact).

## Symbolic engines and hooks

- `symbolic.rs`: Datalog (`derive_datalog_receipt`, 14 built-in rules), probabilistic (`probabilistic_source_reliability`, `probabilistic_expected_value`), evolution archive (`evolution_archive`), determinism helpers (`stable_hash_json`, `canonical_json`). `epistemic.rs`: e-graph dedup (egg) plus the epistemic enrichment suite (`structural_epistemic_pass`, `run_epistemic_cron_pass`).
- `hooks.rs`: post-commit reactive compute. `HookDispatcher`, `HookHandler`, `HookRegistration`, `MutationEvent`/`MutationKind`, `coalesce_per_id`. Off the writer's critical path (bounded queue), idempotent and coalesced, fail-open (handler errors caught and skipped), depth-bounded. Plain `std::thread` worker (tokio-free). `RedCoreGraphStore` wires it via `attach_hook_emitter`.

## Command/executor layer

`executor.rs` (`ThgExecutor`, `InMemoryThgExecutor`, `execute_request_json`), `commands.rs` (`ThgCommand`/`ThgRequest`/`ThgResponse`), `state.rs` (`ThgState`, `stable_hash`), `errors.rs` (`ThgError`/`ThgResult`).

## Features

`default = []`. `redis-store` (Redis backend; `redis 0.27`), `vector-accelerated` (TurboVec), `tantivy` (alternative full-text), `s2` (alternative spatial). Always-on deps include `egg`, `h3o`, `imbl`, `roaring`, `sha2`, `zstd`, `fs2`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-core
cargo test --no-run -p rustyred-thg-core   # compile-only coherence
```

Large offline test suite across `src/` unit modules and `tests/` (`document_tier_acceptance`, `epistemic_acceptance`, `hook_dispatch`, `planner_multimodal_acceptance`, `prolly_incremental_commit`, `redcore_snapshot_replay_parity`, `relational_core_acceptance`, and others). No `#[ignore]` or live/network tests.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
