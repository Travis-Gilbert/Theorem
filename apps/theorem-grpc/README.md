# theorem-grpc

Theorem's first gRPC server. Serves `theseus_search.v1.SearchService` in pure
Rust over the RustyRed substrate (`rustyred-web` search kernel +
`rustyred-thg-core` store/PPR). No PyO3, no Django: this binary never touches
the `rustyredcore_THG/src/*.rs` `#[pyfunction]` wrappers.

It is the URL-swap target for the civic-atlas-server `SearchServiceClient`: same
proto package + message shapes (copied byte-identical from
`RustyRed-Graph-Database/proto/theseus_search/v1/search.proto`), so the civic
backend dials it by setting `THEOREM_SEARCH_URL` with no code change.

It also serves `theorem_code.v1.CodeCrawlerService`, a native internal code
index lane. `IngestCodebase` and `ReindexCodebase` crawl local repo files into a
durable RedCore code graph; `SearchCode` ranks indexed symbols; `RecognizeCode`
extracts symbols from indexed files or inline source; `ExploreCode` expands
Rust AST-backed call/dependency edges; `CodeContext` expands a file or symbol
hit into surrounding source context; `ExplainCode` returns a compact symbol
summary with context, trust tier, and graph evidence; `RecordUseReceipt` records
agent use/outcome metadata for the learning loop. Every operation returns a
content-addressed receipt, and read/use receipt writes are separate from the
code graph mutations.

The app surface serves `theorem_grpc.AppAffordanceService`, the live transport
boundary for metadata-registered `theorem_grpc.*` Theseus app affordances. The
service validates affordance ids, confirmation gates, timeout policy, and
content-addressed receipt shape, then dispatches local receipt-first handlers.
Code affordances (`code_search.ingest`, `code_search.reindex`,
`code_search.search`, `code_search.recognize`, `code_search.explore`,
`code_search.context`, `code_search.explain`,
`code_search.record_use_receipt`) call the native CodeCrawler runtime directly,
so app/harness callers and direct gRPC callers share the same code index. Each
non-dry-run invocation records an affordance outcome into the
service RedCore store and returns charter-scoped capability recommendations from
the same selection path. Confirmed non-code side-effecting actions call a
configured Theseus app adapter endpoint; without one they fail safely and still
record the failed attempt as a learning receipt.

Durability and live adapter knobs:

- `THEOREM_GRPC_REDCORE_DIR`: RedCore data directory for app affordance receipts
  (default: `data/theorem-grpc/redcore`).
- `THEOREM_GRPC_REDCORE_DURABILITY`: `aof_everysec` default; also supports
  `aof_always`, `snapshot_only`, or `none`.
- `THEOREM_GRPC_REDCORE_STRICT_ACID`: set to `true` with `aof_always` when every
  receipt write must fsync before success.
- `THEOREM_CODE_INDEX_DIR` or `THEOREM_GRPC_CODE_INDEX_DIR`: RedCore data
  directory for the native code graph (default:
  `data/theorem-grpc/code-index`; when `THEOREM_GRPC_DATA_DIR` is set, defaults
  to `$THEOREM_GRPC_DATA_DIR/code-index`).
- `THEOREM_CODE_INDEX_DURABILITY`: code-index RedCore durability; falls back to
  `THEOREM_GRPC_REDCORE_DURABILITY`.
- `THESEUS_APP_ADAPTER_ENDPOINT`: full HTTP endpoint for confirmed
  side-effecting app affordance calls.
- `THESEUS_APP_BASE_URL`: alternate base URL; the service appends
  `/api/v2/theorem/app-affordances/invoke/`.
- `THESEUS_APP_ADAPTER_TOKEN`: optional bearer token for that adapter endpoint.
- RustyRed MCP `code_search` callers dial this service's
  `AppAffordanceService` by setting `THEOREM_APP_AFFORDANCE_GRPC_URL`
  (fallbacks: `THEOREM_GRPC_URL`, `THEOREM_SEARCH_URL`, `THESEUS_BRIDGE_URL`).
  The RustyRed product Docker image defaults this to
  `http://theorem-grpc.railway.internal:8080`; bare `host:port` values are
  normalized to `http://host:port`.
- `VALKEY_URL`: optional private-network Valkey URL for cache-aside reads. When
  unset, the service computes directly from RustyRed and stores nothing outside
  RedCore.
- `VALKEY_CACHE_TTL_SECONDS`: TTL for Valkey cached responses (default: `60`).
- `VALKEY_KEY_PREFIX`: cache key prefix (default: `theorem-grpc`).

Build: `cargo build -p theorem-grpc` (run from this dir; standalone Cargo root,
not a member of `rustyredcore_THG`).

Deploy: its own Railway service (`railway.toml` + `Dockerfile`, build context =
Theorem repo root). Binds `0.0.0.0:$PORT` (Railway injects `PORT`; default
`50071` locally).

Search RPCs: `Search` is real graph rank (`prior_knowledge` populated from the
substrate). `GapWalk` is a real single-round PPR over the existing substrate.
`SourcePair` returns the honest empty state (no source/web anchoring layer
ingested yet). `Provenance` returns the real node or honest-empty. None
fabricate: graph-grounded-or-empty, never invented. An empty substrate yields
zero hits, which is truthful, not a bug.

## Valkey cache/offload

Valkey is an accelerator only. It holds recomputable `Search`, `GapWalk`, and
`code_search.context_pack` cache values with TTL. It never holds dispatch
state, coordination state, provenance, execution receipts, or any
must-not-lose graph data.

Railway service shape:

- Image: `valkey/valkey:8` or a later pinned `valkey/valkey` tag.
- Network: private Railway networking only; do not attach a public domain.
- Persistence: disabled. Run with `--save "" --appendonly no`.
- Memory policy: set `--maxmemory <instance-size>` and
  `--maxmemory-policy allkeys-lru`.
- Consumers: set `VALKEY_URL` on `theorem-grpc`, the harness service, and any
  future API service that uses cache-aside reads.

Example Valkey start command:

```bash
valkey-server --bind :: --protected-mode yes --save "" --appendonly no \
  --maxmemory 256mb --maxmemory-policy allkeys-lru
```

Private-network validation:

```bash
redis-cli -u "$VALKEY_URL" PING
```

Expected result: `PONG` from services inside the Railway private network, and no
publicly reachable Valkey endpoint.
