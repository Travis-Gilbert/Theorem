# theorem-grpc

Theorem's first gRPC server. Serves `theseus_search.v1.SearchService` in pure
Rust over the RustyRed substrate (`rustyred-web` search kernel +
`rustyred-thg-core` store/PPR). No PyO3, no Django: this binary never touches
the `rustyredcore_THG/src/*.rs` `#[pyfunction]` wrappers.

It is the URL-swap target for the civic-atlas-server `SearchServiceClient`: same
proto package + message shapes (copied byte-identical from
`RustyRed-Graph-Database/proto/theseus_search/v1/search.proto`), so the civic
backend dials it by setting `THEOREM_SEARCH_URL` with no code change.

It also serves `theorem_grpc.AppAffordanceService`, the first live transport
boundary for metadata-registered `theorem_grpc.*` Theseus app affordances. This
initial path validates affordance ids, confirmation gates, timeout policy, and
content-addressed receipt shape. Concrete app-family handlers are not live yet:
confirmed non-dry-run calls return `HANDLER_NOT_IMPLEMENTED` receipts instead of
fabricating success.

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
