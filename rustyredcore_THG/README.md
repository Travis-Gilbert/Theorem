# RustyRed GraphDB — Technical Documentation

## Technical reference for RustyRed GraphDB 0.6.0, derived from the source tree rather than
marketing copy. Two audiences are covered:

Integration developers — building against the HTTP, gRPC, or MCP surfaces.
Operators — deploying, configuring, and running the release.

RustyRed is an in-memory graph and vector database with append-only-file/snapshot
persistence, multi-tenant isolation, confidence-weighted epistemic edges, HNSW vector search,
BM25 full-text, H3 spatial indexing, a bounded Cypher surface, a graph-state-aware cache, and a
first-class MCP agent port. It is written in Rust and ships as a single service with no Redis
sidecar required.
Map
DocumentForCoversArchitectureBothCrates, request flow, the RedCore storage engine, persistence & recoveryData modelDevsNodes, edges, epistemic types, properties, versioning, content addressing, tenancyHTTP APIDevsComplete REST reference with curl examples and the error modelQuery surfaceDevsStructured /v1/query, the Cypher subset, and the graph-aware cachegRPC APIDevsrustyred.v1.GraphDatabase service and RPC catalogMCP agent portDevsJSON-RPC methods, tool/resource catalog, read-only & admin gatingDeploymentOperatorsDocker, Railway, ports, volumes, format upgrades, build featuresConfigurationOperatorsEvery environment variable, auth model, scopesObservabilityOperatorsPrometheus metrics, diagnostics, health/readiness
At a glance

Latest version: 0.6.0 (workspace Cargo.toml), Rust edition 2021, MSRV 1.85.
Default storage mode: embedded (RedCore native engine; in-memory graph + AOF + snapshots).
Default HTTP/gRPC port: 8380 (HTTP and gRPC share one listener).
Default RESP port: 6380 (experimental scaffold — see Architecture).
Auth: Bearer tokens with scopes; RUSTY_RED_REQUIRE_AUTH defaults to true.
On-disk format version: 1 (CURRENT_FORMAT_VERSION); migrate with rustyred-upgrade-format.


Source of truth for configuration is crates/rustyred-server/src/config.rs; for the data model,
crates/rustyred-core/src/graph_store.rs. Where this documentation and the README disagree, the
code wins

# Architecture

## Workspace layout

RustyRed is a Cargo workspace. One core engine crate is shared by every network surface, so the
HTTP server, the MCP adapter, and direct Rust callers all execute the same command logic.

| Crate | Path | Role |
|-------|------|------|
| `rusty_red_native` | `src/lib.rs` | Root facade. Re-exports the core graph algorithms and adds `push_ppr`, an integer-id ACL local-push Personalized PageRank helper. No Python or native-extension dependency. |
| `rustyred-core` | `crates/rustyred-core` | The engine: command vocabulary, executor, graph store (in-memory / RedCore / Redis), vector / spatial / full-text indexes, versioning, and the Instant-KG merged views. |
| `rustyred-server` | `crates/rustyred-server` | The product server: axum HTTP API + tonic gRPC on one port, auth, config, the Cypher surface, the graph cache, metrics, and the `rustyred-upgrade-format` tool. |
| `rustyred-search` | `crates/rustyred-search` | Standalone RustyWeb crawl/search kernel: guarded fetching, robots handling, crawl graph emission, SERP payloads, and Web Commons fragment types. |
| `rustyred-mcp` | `crates/rustyred-mcp` | Model Context Protocol adapter — turns the core into a JSON-RPC tool/resource server. |
| `rustyred-compat-server` | `crates/rustyred-compat-server` | Minimal standalone HTTP command server over the core executor (compatibility / embedding). |
| `rustyred-resp-server` | `crates/rustyred-resp-server` | Experimental Redis-RESP listener scaffold (see below). |

The release binary is `rustyred-server`. The Dockerfile builds only that crate.

## Request flow

```
HTTP / gRPC client ─┐
MCP client ─────────┤→ rustyred-server ─→ AppState ─→ rustyred-core
RESP client (exp.) ─┘    (axum + tonic)    (per-tenant   (GraphStore: InMemory │ RedCore │ Redis)
                                            stores)
```

`rustyred-server`'s `main` reads `Config::from_env()`, validates it, builds the axum router and the
tonic gRPC routes, and **merges them onto a single TCP listener**. Content-type sniffing routes
`application/grpc*` traffic to gRPC and everything else to the HTTP handlers, so both protocols
answer on the same port (default `8380`). Handlers resolve the target tenant, acquire that tenant's
graph store from `AppState`, and dispatch into `rustyred-core`.

## Storage modes

`RUSTY_RED_MODE` selects the backend (`crates/rustyred-server/src/config.rs`):

- **`embedded`** *(default)* — the **RedCore** native engine. The working graph lives in memory;
  durability comes from an append-only file (AOF) plus periodic snapshots on the data volume.
- **`memory`** — in-memory only, no persistence. For ephemeral test deployments.
- **`redis`** — legacy compatibility backend (requires the `redis-store` build feature). Not used
  by the Railway template and not recommended for new deployments.

All three implement the same `GraphStore` trait, so the API surface is identical regardless of mode.

## The RedCore storage engine

RedCore (`crates/rustyred-core/src/graph_store.rs`) is a Redis-style durability layer implemented
natively in Rust around an `InMemoryGraphStore`.

**Durability modes** (`RUSTY_RED_DURABILITY`):

| Mode | Behaviour |
|------|-----------|
| `aof_everysec` *(default)* | Append each mutation to the AOF; fsync batched roughly once per second. |
| `aof_always` | Synchronous fsync on every write. Required for strict-ACID mode. |
| `snapshot_only` | No AOF; rely on periodic snapshots. |
| `none` | No persistence (memory). |

**Snapshots.** A full snapshot is written every `RUSTY_RED_SNAPSHOT_INTERVAL_WRITES` mutations
(default `1000`), bounding AOF replay time on restart.

**On-disk artifacts.** A `manifest` records the format version, graph version, last/snapshot
transaction ids, durability mode, and the snapshot/AOF filenames. A **directory lock** prevents two
processes from opening the same data directory. The current on-disk format version is
`1` (`CURRENT_FORMAT_VERSION`); a build refuses to load a snapshot whose manifest version is newer
than it understands.

**Recovery.** On startup RedCore loads the latest snapshot, then replays AOF frames recorded after
the snapshot transaction id. Each AOF frame carries a payload checksum. Recovery tolerates orphan
edges (edges whose endpoints were not yet present) rather than aborting.

**Transactions.** Mutations apply as transactions with a monotonic `txn_id` and a `graph_version`.
The HTTP transaction API (`/v1/transactions/*`) exposes begin/commit/rollback with snapshot
isolation; strict-ACID mode (`RUSTY_RED_STRICT_ACID=true`) upgrades this to serializable,
single-writer, `aof_always` and is only valid in `embedded` mode.

## Index structures

`InMemoryGraphStore` maintains, alongside the node and edge maps:

- **Out/in adjacency** keyed by `(node_id, edge_type)` for directional neighbor lookups.
- **Label index**, **edge-type index**, and a **property index** keyed by `(property, value)`.
- **Vector designations and HNSW indexes** keyed by `(label, property)`.

Full-text (BM25) and spatial (H3) indexes are separate subsystems layered on the same store.

## Higher-level subsystems

- **Versioned graph** (`versioned_graph.rs`) — content-addressed node/edge objects compile into
  Prolly-style trees with Git-like commits, refs/branches, diff, checkout, and merge. See
  [Data model](data-model.md) and the version endpoints in [HTTP API](http-api.md).
- **Instant-KG** (`instant_kg.rs`) — "Harness" merged views that overlay session-fresh code deltas
  on a durable tenant graph for code PageRank, impact analysis, related-object lookup, search, and
  edge explanations. Protocol id `harness-instant-kg-v1`.
- **Graph cache** (`graph_cache.rs`) — a graph-state-aware result cache with ten entry kinds that
  invalidate when the underlying graph mutates. See [Query surface](query-surface.md).
- **RustyWeb / Web Commons federation** (`rustyred-search` + `router.rs`) — bounded crawls write
  Page/Domain/ContentSnapshot/LINKS_TO graph fragments locally. When federation is enabled, a
  deployment signs a bounded Web Commons fragment with Ed25519 and submits it to a hub. The hub
  verifies the signature, checks peer trust, stores accepted pages as probationary/canonical, and
  uses the versioned-graph merge primitive before committing the fragment batch.

## A note on the RESP server

`rustyred-resp-server` is a **scaffold**, not a finished feature. Its `main` binds a TCP listener
(default `127.0.0.1:6380`, override with `RUSTYRED_RESP_ADDR`) and currently accepts and drops
connections without serving them. A command-mapping helper exists but is not yet wired to the
executor and maps only the `RUSTYRED.RUN.*` / `RUSTYRED.STATE.HASH` commands. Treat RESP as
experimental and do not depend on it in production.

# Data model

The canonical types live in `crates/rustyred-core/src/graph_store.rs`. The graph is a directed
property graph with first-class *epistemic* semantics on edges.

## Nodes

`NodeRecord`:

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | Caller-supplied unique id. |
| `labels` | string[] | Zero or more labels; normalized on write. Backs the label index. |
| `properties` | JSON object | Arbitrary JSON. Scalar properties back the property index. |
| `version` | uint | Monotonic per-record version, bumped on each upsert. |
| `tombstone` | bool | Soft-delete flag. Tombstoned nodes are retained but excluded from queries. |
| `content_hash` | string? | Content address (see below). |
| `parent_hashes` | string[] | Prior content hashes, for version lineage. |

## Edges

`EdgeRecord`:

| Field | Type | Notes |
|-------|------|-------|
| `id` | string | Caller-supplied unique id. |
| `from_id` / `to_id` | string | Endpoint node ids. Both must exist and be live at write time. |
| `type` | string | Edge type (relationship name). Backs the edge-type index and adjacency. |
| `properties` | JSON object | Arbitrary JSON. |
| `version` | uint | Monotonic per-record version. |
| `tombstone` | bool | Soft-delete flag. |
| `confidence` | float? | Optional, clamped to `[0,1]`. Defaults to `1.0` when absent. |
| `epistemic_type` | enum? | One of the epistemic types below. |
| `provenance` | object? | `{ source_id?, timestamp?, method? }`. |
| `content_hash` | string? | Content address. |
| `parent_hashes` | string[] | Version lineage. |

### Epistemic edge types

`EpistemicType` (`supports`, `contradicts`, `tension`, `derives`, `cites`) makes the graph aware of
how claims relate. It drives two behaviours:

- **Epistemic neighbor traversal** — filter and walk neighbors by epistemic type, minimum
  confidence, and hop depth (`/v1/tenants/{t}/graph/epistemic-neighbors`).
- **Hybrid scoring** — `contradicts` and `tension` edges carry negative default weights
  (`-1.0` and `-0.5`), so contradictory paths *reduce* graph proximity rather than increasing it.

## Multi-tenancy

Every persistent surface is namespaced by `tenant_id`. Stored data is segregated under
`<RUSTY_RED_KEY_PREFIX>:<tenant_id>:…` (default prefix `rusty-red:tenant`). Tenant ids are sanitized
into safe key/path segments. HTTP routes carry the tenant explicitly
(`/v1/tenants/{tenant_id}/…`); root convenience routes and MCP calls fall back to a default tenant
(`RUSTY_RED_MCP_DEFAULT_TENANT`, literal `default` if unset). Per-tenant runtime overrides
(durability, snapshot interval, strict-ACID, memory quota, hybrid scoring) can be supplied at
startup via `RUSTY_RED_TENANT_CONFIG_JSON` / `RUSTY_RED_TENANT_CONFIG_PATH`.

## Content addressing & versioning

Each record can compute a stable **content address**: a `sha256:` hash over its identifying fields
(id, labels/type, properties, tombstone, and — for edges — confidence, epistemic type, and
provenance). Content addressing underpins:

- **Integrity** — `checksum()` is returned on every write (`GraphWriteResult { id, version, checksum }`).
- **Versioned graph packs** — `snapshot_content_objects`, `build_prolly_tree`, and
  `compile_graph_pack` turn a snapshot into a content-addressed Prolly-style tree plus a Git-like
  commit (`GraphCommit`) with author/message/parents.
- **Refs, diff, merge** — branches default to `main` (`DEFAULT_GRAPH_BRANCH`); `diff_graph_snapshots`
  produces `GraphDiffEntry` lists; `merge_graph_snapshots` performs three-way merges with conflict
  reporting and selectable strategies.

The whole graph also has a single monotonic `version` (in `GraphStats.version`) that increments with
each mutation; the cache and verify subsystems use it to detect staleness.

## Indexes derived from the data

| Index | Keyed by | Used for |
|-------|----------|----------|
| Label | label → node ids | `NodeQuery` by label |
| Edge type | type → edge ids | edge-type filters |
| Property | `(property, value)` → node ids | exact property match |
| Adjacency (out/in) | `(node_id, edge_type)` → edge ids | neighbor expansion |
| Vector | `(label, property)` → HNSW index | vector / hybrid search |
| Full-text | `(label, property)` → BM25 index | full-text search |
| Spatial | `(label, lat_prop, lon_prop, resolution)` → H3 cells | radius / bbox queries |

## Stats & integrity

- `GraphStats` reports `version`, totals for nodes/edges/labels/edge-types/property-keys/property
  indexes, and estimated `memory_bytes` against `memory_quota_bytes`.
- `verify()` returns a `VerifyReport { ok, stats, problems[] }`; `rebuild_indexes()` repairs derived
  indexes and returns before/after verify reports. Exposed at `…/graph/verify` and
  `…/graph/rebuild-indexes`.
