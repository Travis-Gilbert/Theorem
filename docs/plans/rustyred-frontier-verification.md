# RustyRed Crawl Frontier — Verification & Review (Claude Code lane)

Companion to the handoff `rustyred-frontier-handoff.md`. This is the **Claude Code**
seam: review + verification, written while **Codex** live-builds the frontier itself.

## Coordination state (2026-06-14)

- **Codex owns** `rustyredcore_THG/crates/rustyred-web/src/frontier/*`, plus that
  crate's `lib.rs` (`pub mod frontier;`) and `Cargo.toml` frontier deps. Landed so
  far: `model.rs`, `queue.rs` (~2/7 files; `prioritizer`, `politeness`, `fetcher`,
  `runner`, `mod.rs` pending; crate does not compile until `mod.rs` exists).
- **Claude Code owns** this doc: static review of landed files, the
  acceptance-criteria verification matrix (runnable once Codex commits), the reuse
  map, and the OPTIONAL graph_store CAS hardening spec.
- Claim declared in harness room `repo:theorem:branch:001-local-loop`
  (tenant `rustyredcore-theorem-production`). Codex sprints git-only and does not
  read the room; treat the human as the relay for the findings below.

## Review of landed files (`model.rs`, `queue.rs`)

Faithful to the handoff and correct on the substance. `fingerprint()` matches the
spec (blake3 of `canonical \n METHOD \n hex(body)`); `canonicalize_url(raw, base)`
is *richer* than the crate's existing `canonicalize_url` (lowercases host, strips
`:80`/`:443`, sorts query, resolves dot segments); `MemoryFrontierQueue` pop is
atomic under a `Mutex` and `RedisFrontierQueue` uses `ZADD -priority` + an atomic
`ZREM` Lua pop. Three findings worth acting on before the build finishes:

### Finding 1 — anchor_text / rel will be empty (extractor gap)

`DiscoveredLink { url_raw, anchor_text, rel }` (model.rs) cannot be populated by the
existing extractor. `extract_links_for_url` → `extract_links_with_profile`
(`lib.rs:1400`, `lib.rs:1408`) runs a `lol_html` `element!("a[href]")` handler that
captures **only the joined href** — it discards element text and never reads `rel`.
To honor `DiscoveredLink` fully, the `a[href]` handler must also accumulate the
element's text (via a `text!` handler keyed to the same element) and
`el.get_attribute("rel")`. The handoff's "reuse the extractor, don't write a second
one" instruction has this gap: the current extractor is href-only.

### Finding 2 — vocabulary fork → two disjoint subgraphs in one store

`model.rs` defines `LABEL_URL = "url"`, `EDGE_LINKS_TO = "links_to"`,
`EDGE_ON_DOMAIN = "on_domain"` (lowercase). The crate already emits a crawl graph
with `LABEL_PAGE = "Page"`, `EDGE_LINKS_TO = "LINKS_TO"`, `EDGE_ON_DOMAIN =
"ON_DOMAIN"` (`lib.rs:20-33`) from `build_fixture_crawl_graph` and
`web_consume_to_graph`. With the literal-spec labels, a URL the frontier discovers
(`url` node, fingerprint id) and the same page the existing crawler fetches (`Page`
node, `page_id()` id) become **two unrelated nodes** — the crawl graph and the
frontier graph never reconcile in one store.

This is a real architectural choice, not a bug. Options:
- **Keep the fork** (literal handoff): frontier is its own `url`/`links_to`
  subgraph. Acceptance #6 (PPR/neighbors over `links_to`) still holds within it.
- **One graph home** (recommended if reconciliation matters): reuse `page_id()` +
  `LABEL_PAGE` + `EDGE_LINKS_TO`/`EDGE_ON_DOMAIN`, and store the scheduler lifecycle
  as a new `frontier_state` property (no collision with the existing `page_state`).
  A discovered URL and its later-fetched page then share one node id; resume
  (acceptance #1) and "queryable as graph" (#6) fall out for free.

### Finding 3 — graph_store CAS is absent, and OPTIONAL here

Confirmed: `rustyred-thg-core` has **no** guarded/compare-and-set mutation
(`graph_store.rs` `GraphStore` trait at line 287; upserts are unconditional
full-replace). The handoff says "verify or add." Resolution: the queue's atomic pop
(`MemoryFrontierQueue` Mutex pop / `RedisFrontierQueue` Lua `ZREM`) is the real
single-claim primitive, and a fingerprint is its own node id *and* its own ZSET
member, so enqueue is idempotent and a claim is single-popper. **Acceptance #2 (no
URL reaches `in_flight` twice) holds without an in-core CAS.** The CAS is
belt-and-suspenders hardening only — see the optional spec at the end. Leave
`graph_store.rs` alone unless that hardening is explicitly wanted (and note it is a
Codex-dirty file from the CRDT work).

## Acceptance-criteria verification matrix

Run once Codex commits a compiling frontier. Each row is observable.

| # | Criterion | Concrete check |
|---|-----------|----------------|
| 1 | Fixture crawl visits each unique URL exactly once; `links_to` connected; re-run resumes | Seed N roots over a `FixtureFetcher`; assert `visited.len() == unique_urls.len()` and no fingerprint fetched twice; assert `links_to` edge set forms one connected component; reopen the store and assert nodes still in `frontier_state = "frontier"` are popped on the next run |
| 2 | Two concurrent workers never fetch the same URL | Share one `Frontier` across 2 worker loops on a `rt-multi-thread` runtime; record every `frontier -> in_flight` transition; assert each fingerprint transitions at most once |
| 3 | Per-domain crawl delay respected, enforced by `pop_eligible` | Set `crawl_delay_ms`; record fetch timestamps per host; assert consecutive same-host fetches are spaced `>= crawl_delay_ms`; assert no sleeping in the runner (delay is a pop-eligibility gate) |
| 4 | `PprPrioritizer` reorders vs `DepthPrioritizer` | Fixture where a deep node has high centrality; assert the visit sequence differs between the two prioritizers (the reorder is observable) |
| 5 | Fetcher-agnostic: swap `CascadeFetcher` ↔ another `Fetcher` | Run the same fixture under two `Fetcher` impls; assert identical visited set AND identical node/edge sets. `FixtureFetcher` vs `CascadeFetcher` proves this hermetically (no `spider`/`servo` dep needed) |
| 6 | Frontier queryable as a graph mid-crawl | Mid-run, build the `links_to` adjacency and call `rustyred_thg_core::personalized_pagerank` + `GraphStore::neighbors`; assert non-empty, sensible structure |

Verification commands (post-commit):

```bash
cd rustyredcore_THG
cargo test -p rustyred-web                 # frontier unit + integration tests
cargo test -p rustyred-web --features redis-frontier   # compile the Redis queue
# Redis integration (needs a server) - keep #[ignore] unless THEOREM_FRONTIER_REDIS_URL is set:
THEOREM_FRONTIER_REDIS_URL=redis://127.0.0.1:6379 cargo test -p rustyred-web --features redis-frontier -- --ignored
cargo build -p rustyred-web --features spider_fetch    # compile-gate the optional Spider backend
```

Note acceptance #2/#3 need `tokio` `rt-multi-thread` + `macros` (already in
rustyred-web `[dev-dependencies]`).

## Reuse map (existing helpers → frontier seams)

| Frontier need | Reuse (do not rebuild) |
|---------------|------------------------|
| Link extraction | `extract_links_for_url` (`lib.rs:1400`) — but extend the `a[href]` handler for anchor text + `rel` (Finding 1) |
| robots allow/deny + crawl-delay | `global_robots_cache().check(client, url, ua)` async (`robots.rs:72`); `crawl_delay_duration(&decision)` (`robots.rs:147`) |
| Multi-tier fetch (CascadeFetcher) | `FetchCascade::new(FetchCascadeOptions{..})` + `fetch_with_promotion(url, max_bytes)` → `FetchTierResult { http_status, final_url, etag, html_bytes, truncated, .. }` (`fetch_cascade.rs:170`) |
| SSRF guard before enqueue | `guarded_canonicalize_url(raw, &UrlGuardPolicy)` / `guard_canonical_url` (`lib.rs:1353`/`1359`) |
| Commit a node/edge batch | `apply_batch_to_store(store, &batch)` (`lib.rs:1291`) |
| PPR for PprPrioritizer | `rustyred_thg_core::personalized_pagerank(adjacency: &HashMap<String, Vec<(String,f64)>>, seeds: &HashMap<String,f64>, alpha, epsilon, max_pushes)` (`graph.rs:261`, re-exported at crate root) |
| Read node state / dedup | `GraphStore::get_node(id) -> Option<&NodeRecord>`; `query_nodes(NodeQuery::label(..).with_property(..))`; `neighbors(NeighborQuery::out(id).with_edge_type(..))` |
| Durable store | `RedCoreGraphStore::open(dir, RedCoreOptions::default())` (`graph_store.rs:2096`) |

## OPTIONAL — graph_store guarded-CAS hardening spec (Lane B)

Only if belt-and-suspenders is wanted (acceptance #2 already passes without it).
Two default-method additions to the `GraphStore` trait (`graph_store.rs:287`), so
existing impls keep compiling:

```rust
/// Atomic create-only-if-absent. Returns Some(write) if the node was created,
/// None if a node with this id already existed (no write performed).
/// This is the dedupe primitive: the URL fingerprint node exists at most once.
fn upsert_node_if_absent(
    &mut self,
    node: NodeRecord,
) -> GraphStoreResult<Option<GraphWriteResult>> {
    if self.get_node(&node.id).is_some() {
        return Ok(None);
    }
    self.upsert_node(node).map(Some) // default: NOT atomic — override in real impls
}

/// Atomic compare-and-set of a single node property. Sets
/// properties[key] = new IFF it currently equals expected; returns whether it set.
/// This is the claim primitive: frontier_state "frontier" -> "in_flight".
fn compare_and_set_property(
    &mut self,
    id: &str,
    key: &str,
    expected: &serde_json::Value,
    new: serde_json::Value,
) -> GraphStoreResult<bool>;
```

**The audited hazard to close:** a read-then-write claim (read state, then upsert
`in_flight`) races under concurrency — two workers can both read `frontier` and both
write `in_flight`. The CAS must be a **single committed compare-and-set inside the
store's commit path**, not a trait-default read + separate upsert. `InMemoryGraphStore`
does this under its write lock; `RedCoreGraphStore` must fold the compare into the
same AOF-appended commit (not a get + later upsert). The default impl above is
deliberately non-atomic and exists only so non-frontier stores compile — real
correctness needs the per-impl override.

Because the queue already serializes claims, this is hardening, not a launch gate.
```
