# theorem-grpc

Theorem's first gRPC server. Serves `theseus_search.v1.SearchService` in pure
Rust over the RustyRed substrate (`rustyred-web` search kernel +
`rustyred-thg-core` store/PPR). No PyO3, no Django: this binary never touches
the `rustyredcore_THG/src/*.rs` `#[pyfunction]` wrappers.

It is the URL-swap target for the civic-atlas-server `SearchServiceClient`: same
proto package + message shapes (copied byte-identical from
`RustyRed-Graph-Database/proto/theseus_search/v1/search.proto`), so the civic
backend dials it by setting `THEOREM_SEARCH_URL` with no code change.

It also serves `theorem_grpc.AppAffordanceService`, the live transport boundary
for metadata-registered `theorem_grpc.*` Theseus app affordances. The service
validates affordance ids, confirmation gates, timeout policy, and
content-addressed receipt shape, then dispatches local receipt-first handlers for
read-only Theseus app families. Each non-dry-run invocation records an
affordance outcome into the service RedCore store and returns charter-scoped
capability recommendations from the same selection path. Confirmed side-effecting
actions call a configured Theseus app adapter endpoint; without one they fail
safely and still record the failed attempt as a learning receipt.

Durability and live adapter knobs:

- `THEOREM_GRPC_REDCORE_DIR`: RedCore data directory for app affordance receipts
  (default: `data/theorem-grpc/redcore`).
- `THEOREM_GRPC_REDCORE_DURABILITY`: `aof_everysec` default; also supports
  `aof_always`, `snapshot_only`, or `none`.
- `THEOREM_GRPC_REDCORE_STRICT_ACID`: set to `true` with `aof_always` when every
  receipt write must fsync before success.
- `THESEUS_APP_ADAPTER_ENDPOINT`: full HTTP endpoint for confirmed
  side-effecting app affordance calls.
- `THESEUS_APP_BASE_URL`: alternate base URL; the service appends
  `/api/v2/theorem/app-affordances/invoke/`.
- `THESEUS_APP_ADAPTER_TOKEN`: optional bearer token for that adapter endpoint.

Build: `cargo build -p theorem-grpc` (run from this dir; standalone Cargo root,
not a member of `rustyredcore_THG`).

Deploy: its own Railway service (`railway.toml` + `Dockerfile`, build context =
Theorem repo root). Binds `0.0.0.0:$PORT` (Railway injects `PORT`; default
`50071` locally).

Honest-minimal RPCs: `Search` is real graph rank (`prior_knowledge` populated
from the substrate). `GapWalk` is a real single-round PPR over the existing
substrate. `SourcePair` returns the honest empty state (no source/web anchoring
layer ingested yet). `Provenance` returns the real node or honest-empty. None
fabricate: graph-grounded-or-empty, never invented. An empty substrate yields
zero hits, which is truthful, not a bug.
