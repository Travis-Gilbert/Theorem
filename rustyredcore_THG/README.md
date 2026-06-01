# Rusty Red Graph Database

Embedded property graph database with a first-class MCP agent port,
multi-tenancy, HNSW vector search, confidence-weighted epistemic edges,
and a graph-version-aware AI cache. Written in Rust.

Designed for AI agents and GraphRAG workloads, not for replacing Neo4j.

[![Deploy on Railway](https://railway.com/button.svg)](https://railway.com/new/template/RUSTY_RED_GRAPH_DATABASE_TEMPLATE_ID?utm_medium=integration&utm_source=button&utm_campaign=rusty-red-graph-database)

Template ID pending: after creating or publishing the Railway template, replace `RUSTY_RED_GRAPH_DATABASE_TEMPLATE_ID` in the badge URL with the Railway template code.

## What Rusty Red does

- **Graph storage** with AOF/snapshot persistence, per-tenant isolation, single-writer serializable commits, and committed read snapshots
- **Stable, versioned on-disk format** with `rustyred-thg-upgrade-format` migrations between releases (no export/re-import on upgrade)
- **HNSW vector search** on node properties via `instant-distance`, with hybrid scoring that blends vector similarity and graph proximity
- **Inverted-index BM25 full-text search** with automatic indexing on node upserts
- **H3 spatial index** on node lat/lon properties with radius and bounding-box queries
- **Epistemic edge types** (Supports, Contradicts, Tension, Derives, Cites) with confidence-weighted traversal across configurable hop depth
- **Graph algorithms over HTTP/MCP**: PPR, connected components, PageRank, and label-propagation community detection
- **Harness Instant KG merged views**: session-fresh code deltas overlay durable tenant graph artifacts for code PPR, impact analysis, related-object lookup, search, and edge explanations
- **MCP agent port** with scoped auth tokens, read-only and read-write modes, tool annotations, and structured tool/resource/prompt surfaces
- **Graph-version-aware cache** (10 kinds) that detects stale entries when the underlying graph mutates
- **Bounded Cypher surface**: single-hop and outgoing multi-hop MATCH, bounded variable-length expand, path aliases, property projections, `COUNT(*) / COUNT(binding)`, and transaction-scoped `CREATE`/`MERGE`/`SET`/`DELETE`
- **JSONL bulk loader** for nodes and edges
- **Observability**: Prometheus `/metrics` (17 counters), slow-query ring buffer at `/v1/diagnostics/slow_queries`
- **HTTP transaction API**: `/v1/transactions/begin|commit|rollback` with snapshot isolation
- **50x to 400x** faster Personalized PageRank than Python (ACL local-push algorithm, exposed via PyO3)

## What you can't do yet

These are on the roadmap, in roughly this priority order:

1. Incoming and undirected Cypher relationship patterns, plus the rest of full OpenCypher/GQL coverage
2. `OPTIONAL MATCH`, `WITH`, `UNION`, `CALL`, `ORDER BY`, `SKIP`, `DISTINCT`
3. `SUM` / `AVG` / `MIN` / `MAX` aggregations
4. `REMOVE` clauses
5. CSV/JSONL `LOAD CSV` syntactic form (JSONL bulk endpoints exist already)
6. Per-query spatial backend selection; H3 is the default and S2 is available behind the `s2` feature plus `RUSTY_RED_SPATIAL_BACKEND=s2`
7. Distributed snapshot replication

## Crate structure

| Crate | Purpose |
|-------|---------|
| `rustyred-thg-core` | Graph store engine, command executor, HNSW vector index, epistemic edges |
| `rustyred-web` | Graph-native crawler kernel and V2 crawl receipt contract |
| `rustyred-thg-mcp` | MCP agent port: tool dispatch, resource reads, prompt surface |
| `rustyred-rustyred-thg-compat-server` | HTTP server, query surface, graph cache, auth, OpenAPI |
| `rustyred-thg-compat-server` | Standalone THG command server (non-product) |
| `rustyred-thg-resp-server` | RESP protocol shim (limited, not a Redis replacement) |
| root crate | PyO3 bindings for `push_ppr` and `ThgCoreExecutor` |

## Source and release model

The standalone RustyRed repository at
`https://github.com/Travis-Gilbert/RustyRed-Graph-Database` is the upstream
home for generic database engine work, public modules, clients, MCP/HTTP server
behavior, release tags, and the one-image Railway deployment.

This `theseus_native` tree is the Theseus downstream import of that upstream
engine. Theseus-specific adapters, Django bridge code, harness policy, and
private deployment wiring should live outside the public RustyRed upstream.
Import public RustyRed releases into Theseus by pinned tag or commit using:

```bash
scripts/rustyred_upstream_sync.sh status
scripts/rustyred_upstream_sync.sh import --ref v0.4.0 --execute
```

See `docs/adr/0002-rustyred-public-upstream.md` for the accepted sync model.

## Build (local development)

Requires Rust 1.85+ and `maturin >= 1.7`.

```bash
python3 -m pip install --user maturin
cd theseus_native
maturin develop --release
```

This builds an `abi3-py312` wheel and installs it into the active Python environment. After this, `from theseus_native import push_ppr` works in any Python 3.12+ interpreter that shares the venv.

## Product server

The product server runs in `RUSTY_RED_MODE=embedded` with RedCore
RAM-first storage and local AOF/snapshot persistence. It exposes graph
operations, vector search, epistemic traversal, Cypher queries, and the
graph-version cache over HTTP and MCP.

`RUSTY_RED_MODE=redis` is available for legacy THG state commands only.
The base Railway deployment is one RustyRed application image plus a mounted
volume. A separate Redis-compatible service is optional compatibility
infrastructure for the standalone template, not part of the default public
database deployment. In Theseus, Redis remains shared operational
infrastructure around RustyRedCore-THG.

Run the product server locally:

```bash
cd theseus_native
RUSTY_RED_MODE=embedded RUSTY_RED_DATA_DIR=data/rusty-red cargo run -p rustyred-rustyred-thg-compat-server
```

Strict local durability mode is explicit:

```bash
RUSTY_RED_MODE=embedded \
RUSTY_RED_CONCURRENCY=single_writer \
RUSTY_RED_TXN_ISOLATION=serializable \
RUSTY_RED_STRICT_ACID=true \
RUSTY_RED_DURABILITY=aof_always \
RUSTY_RED_DATA_DIR=data/rusty-red \
cargo run -p rustyred-rustyred-thg-compat-server
```

Core routes:

```text
GET  /health
GET  /ready
GET  /openapi.json
GET  /.well-known/mcp/rustyred_thg.json
GET  /.well-known/agent.json
POST /mcp
GET  /metrics
POST /v1/command
POST /v1/batch
POST /v1/query
POST /v1/cypher
POST /v1/cypher/explain
POST /v1/transactions/begin
POST /v1/transactions/commit
POST /v1/transactions/rollback
POST /v1/cache/put
POST /v1/cache/get
POST /v1/cache/check
POST /v1/cache/explain
POST /v1/cache/invalidate
POST /v1/cache/stats
POST /v1/tenants/{tenant_id}/command
POST /v1/tenants/{tenant_id}/batch
GET  /v1/tenants/{tenant_id}/runs/{run_id}
POST /v1/tenants/{tenant_id}/graph/query
POST /v1/tenants/{tenant_id}/graph/nodes
POST /v1/tenants/{tenant_id}/graph/nodes/query
GET  /v1/tenants/{tenant_id}/graph/nodes/{node_id}
POST /v1/tenants/{tenant_id}/graph/edges
GET  /v1/tenants/{tenant_id}/graph/edges/{edge_id}
POST /v1/tenants/{tenant_id}/graph/neighbors
GET  /v1/tenants/{tenant_id}/graph/stats
GET  /v1/tenants/{tenant_id}/graph/verify
POST /v1/tenants/{tenant_id}/graph/rebuild-indexes
POST /v1/tenants/{tenant_id}/graph/vector/search
POST /v1/tenants/{tenant_id}/graph/vector/hybrid
POST /v1/tenants/{tenant_id}/graph/vector/designate
POST /v1/tenants/{tenant_id}/graph/epistemic-neighbors
POST /v1/tenants/{tenant_id}/graph/algorithms/ppr
POST /v1/tenants/{tenant_id}/graph/algorithms/components
POST /v1/tenants/{tenant_id}/graph/algorithms/pagerank
POST /v1/tenants/{tenant_id}/graph/algorithms/communities
POST /v1/tenants/{tenant_id}/graph/spatial/designate
POST /v1/tenants/{tenant_id}/graph/spatial/radius
POST /v1/tenants/{tenant_id}/graph/spatial/bbox
POST /v1/tenants/{tenant_id}/graph/fulltext/designate
POST /v1/tenants/{tenant_id}/graph/fulltext/search
POST /v1/tenants/{tenant_id}/graph/bulk/nodes
POST /v1/tenants/{tenant_id}/graph/bulk/edges
POST /v1/tenants/{tenant_id}/graph/version/compile
POST /v1/tenants/{tenant_id}/graph/version/diff
POST /v1/tenants/{tenant_id}/graph/version/ref
POST /v1/tenants/{tenant_id}/graph/version/log
POST /v1/tenants/{tenant_id}/graph/version/checkout
POST /v1/tenants/{tenant_id}/graph/version/merge
POST /v1/tenants/{tenant_id}/instant-kg/status
POST /v1/tenants/{tenant_id}/instant-kg/ppr
POST /v1/tenants/{tenant_id}/instant-kg/impact
POST /v1/tenants/{tenant_id}/instant-kg/related-objects
POST /v1/tenants/{tenant_id}/instant-kg/search
POST /v1/tenants/{tenant_id}/instant-kg/explain-edge
GET  /v1/diagnostics/slow_queries
GET  /v1/diagnostics/config
POST /v1/tenants/{tenant_id}/context/pack
```

### MCP tools

The `/mcp` endpoint exposes these tools (via JSON-RPC `tools/list` and `tools/call`):

| Tool | Description |
|------|-------------|
| `thg.graph.query` / `thg.graph.explain` / `thg.graph.neighbors` | Bounded native graph reads and plan inspection |
| `thg.graph.schema` / `thg.graph.index_status` | Graph schema and index-health reads |
| `rustyred_thg_graph_version_compile` (`rustyred_thg_git_compile`) / `rustyred_thg_graph_version_diff` (`rustyred_thg_git_diff`) / `rustyred_thg_graph_version_ref` (`rustyred_thg_git_ref`) / `rustyred_thg_graph_version_log` (`rustyred_thg_git_log`) / `rustyred_thg_graph_version_checkout` (`rustyred_thg_git_checkout`) / `rustyred_thg_graph_version_merge` (`rustyred_thg_git_merge`) | Content-addressed graph pack, refs/log/checkout, and three-way merge tools |
| `thg.algorithm.ppr` (alias: `thg.algo.ppr`) / `thg.algorithm.components` (`thg.algo.components`) / `thg.algorithm.pagerank` (`thg.algo.pagerank`) / `thg.algorithm.communities` (`thg.algo.communities`) | Graph algorithms (PPR, connected components, PageRank, label-propagation communities). §P6-B SPEC names accepted as aliases. |
| `harness_kg_status` / `harness_kg_ppr` / `harness_kg_impact` / `harness_kg_related_objects` / `harness_kg_search` / `harness_kg_explain_edge` | Harness Instant KG tools over a RedCore tenant base graph plus an optional session delta. Legacy `RUSTY_RED_MODE=redis` returns a diagnostic because Instant KG is a native THG/RedCore capability. |
| `harness_run` / `harness_append_transition` | Native Theorem harness runtime event-log reads and appends over the tenant `GraphStore`. `harness_run` is read-only; `harness_append_transition` appears only in read/write MCP mode and writes `HarnessRun` / `HarnessEvent` nodes for the HTTP/iOS transport to read from the same RedCore store. |
| `thg.fulltext.search` (alias: `thg.graph.fulltext.search`) / `thg.spatial.radius` (`thg.graph.spatial.radius`) / `thg.spatial.bbox` (`thg.graph.spatial.bbox`) | Full-text and spatial read surfaces. §P6-B SPEC names accepted as aliases. |
| `thg.vector.search` | HNSW nearest-neighbor search on vector properties |
| `thg.vector.hybrid` | Hybrid search blending vector similarity with graph proximity |
| `thg.vector.designate` | Register a vector property for HNSW indexing (write) |
| `thg.epistemic.neighbors` | Confidence-weighted epistemic traversal by edge type |
| `thg.fulltext.designate` (alias: `thg.graph.fulltext.designate`) / `thg.spatial.designate` (`thg.graph.spatial.designate`) / `thg.bulk.nodes` (`thg.graph.bulk.nodes`) / `thg.bulk.edges` (`thg.graph.bulk.edges`) | Write-mode-only designation and bulk ingest tools. §P6-B SPEC names accepted as aliases. |
| `thg.admin.verify` | Admin-only index-integrity verification; rebuild remains on the HTTP graph route |

The public query surface is now split cleanly:

- `/v1/query` is the product-facing native subset for `node_match` and `neighbors`.
- `/v1/cypher` and `/v1/cypher/explain` are the bounded OpenCypher-compatible surface for read queries plus transaction-scoped `CREATE`/`MERGE`/`SET`/`DELETE` writes.
- `/v1/tenants/{tenant_id}/graph/query` remains the older debug bridge and should not be treated as the product route.

The graph version routes are the THG Git/provenance substrate. `/graph/version/compile` reads the current tenant graph and returns a `rustyred-versioned-graph-v1` pack with content hashes, a Prolly-style tree root, Git-like commit metadata, and declarative compiler capabilities. `/graph/version/diff` compares a supplied base snapshot against the current graph or an explicit target snapshot. `/graph/version/ref`, `/log`, and `/checkout` operate on caller-supplied graph repositories so downstream products can choose their own persistence layer; checkout returns a snapshot and does not mutate the tenant graph. `/graph/version/merge` performs a read-only three-way merge with content-hash conflict detection and confidence-weighted edge conflict resolution. Full Skill Encoder promotion, code-corpus lowering, and model-adapter policy stay in Theseus/Theorem encode surfaces rather than in the public RedCore release.

`GET /v1/diagnostics/config` returns the static runtime config snapshot, including
startup-only tenant override details. Runtime mutation of tenant config is not
supported in this slice.

The OpenAPI document is served at `/openapi.json`. It exists because Rusty Red
is exposed through HTTP and MCP even though the underlying storage engine is a
database-style service. Its `info.version` is the `rustyred-rustyred-thg-compat-server` package
version, and the v0.4 contract covers the router's graph, vector, epistemic,
algorithm, spatial, full-text, bulk ingest, cache, transaction, diagnostic,
and MCP HTTP routes. MCP tool, resource, and prompt metadata are discovered
through the MCP endpoint and well-known manifests.

Railway template readiness follows the public template guidance: use a GitHub
source repo, keep the service root minimal, set `/ready` as the health check,
wire Redis only for explicit `RUSTY_RED_MODE=redis` deployments through private
networking/reference variables, attach persistent storage to stateful dependencies,
set `RUSTY_RED_REQUIRE_VOLUME=true` for embedded Railway deployments so `/ready`
fails when the mounted volume is absent, generate any public-ingress tokens with
Railway template variable functions, and replace the badge placeholder above once
Railway assigns the final template URL.

Railway can deploy this directory directly:

```bash
cd theseus_native
railway up
```

The included `Dockerfile`, `railway.toml`, and `.railwayignore` are for the
standalone Rusty Red subtree repository. The monorepo Railway template under
`railway-templates/rusty-red-graph-database/` remains useful when deploying
from the full Theseus repository.

## Build (release wheels)

CI builds Linux x86_64 manylinux2014 wheels via `.github/workflows/build_native_wheels.yml`. macOS arm64 is built locally for now (Travis's M1); CI build for Darwin is out of scope for the first cut.

Use `scripts/verify_thg_release.sh` from the repository root for the THG
runtime/product release check. The verifier intentionally uses package-targeted
Cargo release builds for the THG server binaries and uses `maturin` for the root
PyO3 extension:

```bash
scripts/verify_thg_release.sh
scripts/verify_thg_release.sh --develop  # install into the active Python env
```

Do not use `cargo build --manifest-path theseus_native/Cargo.toml --workspace
--release` as the native release check on macOS. That command attempts to link
the root PyO3 `cdylib` as a plain Cargo artifact and can fail with undefined
Python symbols even when the THG binaries and `maturin` wheel path are healthy.

## Public API

```python
def push_ppr(
    adjacency: dict[int, list[tuple[int, float]]],
    seeds: dict[int, float],
    *,
    alpha: float = 0.15,
    epsilon: float = 1e-4,
    max_pushes: int = 200_000,
) -> dict[int, float]: ...
```

THG exports:

```python
from theseus_native import ThgCoreExecutor

executor = ThgCoreExecutor()
executor.execute_json('{"command":"RUSTYRED_THG.RUN.BEGIN","args":{"task":"demo"}}')
executor.state_hash()
```

Matches `apps/notebook/sparse_ppr.py:push_ppr` exactly. ACL local-push personalized PageRank: alpha is the restart probability (Theseus convention), epsilon is the per-node convergence threshold, max_pushes caps total iterations to prevent pathological walks.

## Fallback semantics

`apps/notebook/sparse_ppr.py` is the dispatcher. It tries `from theseus_native import push_ppr` first; on ImportError, or when `THESEUS_DISABLE_NATIVE=1` is set in the environment at call time, it routes to the pure-Python `_python_push_ppr` defined in the same file. The fallback exists indefinitely (per ADR 0001 follow-up) so dev environments without the wheel still function.

The wrapper logs once at WARNING level on the first import that finds the wheel missing: `theseus_native unavailable, using Python push_ppr`. Subsequent imports do not re-log.

## THG standalone HTTP server

Phase 1 standalone mode lives in `crates/rustyred-thg-compat-server`:

```bash
cd theseus_native
cargo run -p rustyred-thg-compat-server -- --host 127.0.0.1 --port 7379
```

Endpoints:

```text
GET  /health
GET  /ready
GET  /v1/state/hash
GET  /v1/runs/{id}
POST /v1/command
POST /v1/batch
```

Django selects embedded or remote THG with:

```bash
THG_MODE=in_process
THG_MODE=remote_http THG_HTTP_URL=http://localhost:7379
```

## Algorithm reference

Andersen, R., Chung, F., and Lang, K. (2006). Local Graph Partitioning using PageRank Vectors. FOCS 2006.

## Benchmarks

Single-threaded, single-seed PPR queries on random sparse graphs (avg degree 4, alpha 0.15, epsilon 1e-4), captured on the developer's M1 Max via the harness in `tests/test_benchmarks.py`:

| Nodes | Native | Python | Speedup |
|-------|--------|--------|---------|
| 50K   | 0.0024s | 0.0318s | 13.2x |
| 200K  | 0.0034s | 0.1753s | 51.3x |
| 1M    | 0.0023s | 0.9573s | 413.9x (acceptance gate: must be >= 20x) |

To re-run:

```bash
cd theseus_native
python3 -m pytest tests/test_benchmarks.py -v -s
```

The fixture is generated with seed 42 for reproducibility. Numbers vary across hardware; the 20x floor is enforced on whatever runner executes the test.

The native impl uses lazy on-demand neighbor extraction: ACL Push typically touches ~1/(epsilon*alpha) ~ 67k nodes for production params, so converting only those (not the full adjacency dict) eliminates the dominant FFI cost.

## License

MIT.
