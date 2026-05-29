# Rusty Red Web Implementation Plan

**Status:** Active plan after 2026-05-29 reprioritization.
**Owner lane:** Codex-heavy Rust implementation, with Claude/Codex contract review on graph schemas.
**Scope:** RustyWeb / Rusty Red Web as a RustyRed-backed crawler and substrate-native search engine. This supersedes the older "later separate track" wording in the reconciliation notes.

## Decision

RustyWeb is now front-of-queue work. The kernel-object gaps (`Artifact`, `ValidationReceipt`, `Subscription.delivery_policy`, and the general object model cleanup) can move to parallel Codex/Claude sessions. Keep `AnswerDraft` in close coordination because it is both a kernel-object subtype and a RustyWeb V1 graph contract.

The current `feat/theseus-burst-crawler-scaffold` branch is useful seed code, but it is not the final RustyWeb shape. It is a fast frontier fetcher. RustyWeb V0 must output queryable graph state and use RustyRed's graph/index/algorithm primitives.

## Current Context

### Built or partially built

- `docs/plans/commonplace/implementation-plan.md` already defines RustyWeb as the third Commonplace layer and gives V0's target: `link` tenant, URL frontier, HTTP/1.1+2, robots/politeness, streaming parse, BLAKE3 content-hash dedup, gRPC query/control, and local graph search.
- `docs/plans/rustyweb-v1-design/` defines V1: graph-yielding discovery broker, AnswerDraft, and license-tiering.
- `rustyredcore_THG/crates/rustyred-web` contains the RustyWeb graph kernel and
  V2 fixture contract. It emits V0 crawl graph mutations, accepts
  `CrawlRequest`/`CrawlBudget`/`CrawlScope`, guards seed URLs, annotates hot
  graph mutations with TTL/license/federation metadata, and emits
  content-addressed `CrawlReceipt` nodes.
- `feat/theseus-burst-crawler-scaffold` contains an isolated Rust+Tokio scaffold:
  - `crates/theseus-burst-crawler/src/main.rs`
  - `crates/theseus-burst-crawler/Dockerfile`
  - `crates/theseus-burst-crawler/railway.toml`
  - `apps/notebook/search/kernel/providers/burst_crawler.py`
  - Follow-up commits added bearer auth, private-IP/metadata-service SSRF guards, redirect blocking, body caps, and max-page passthrough.
- The active Index-API tree already has Python crawl/search seams:
  - `apps/notebook/search/kernel/crawl_runner.py`
  - `apps/notebook/discovery_runs/crawl_executor.py`
  - `apps/notebook/web/spider/`
  - `apps/notebook/web/{fetch,robots,ingest}.py`
- The sibling RustyRed repo has the graph substrate RustyWeb should stand on: `rustyred-core`, HNSW vector search, BM25/Tantivy fulltext, PPR/PageRank/components/communities, HTTP/gRPC/MCP surfaces, bulk node/edge writes, and Railway packaging.

### Not built or drifted

- The active Index-API checkout has only `crates/theseus-burst-crawler/target/`; the source is on the feature branch, not in the working tree.
- The live `rustyred-web` crate is still a deterministic fixture/contract layer,
  not the impure network fetch loop. It has no `reqwest`, Tokio crawler service,
  real robots fetch/cache, frontier scheduler, or MCP crawl tool yet.
- `/api/v2/theseus/search/crawl/` currently returns a deliberate `501` in `apps/notebook/api/search.py`, even though docs/tests claim the native Scrapy dispatch exists. This is code/docs/test drift and must be reconciled before claiming the crawl surface is shipped.
- The feature-branch burst crawler still lacks robots enforcement at the source, streaming HTML parsing, graph writes, local graph search, gRPC/control, and RustyRed integration.

## Architecture

### Canonical code home

Build the real product in the RustyRed workspace as a new Rust crate:

- Preferred: `/Users/travisgilbert/Tech Dev Local/RustyRed-Graph-Database/crates/rustyred-web`
- Index-API role: Python client/adapter, bridge into existing Search Kernel and ask/self-expansion flows.
- Existing `crates/theseus-burst-crawler` branch role: seed implementation to extract from, then retire or keep only as a historical scaffold.

Reason: RustyWeb is a RustyRed product surface, not an Index-API-only helper. It should embed/use `rustyred-core` directly and expose graph-shaped crawler output to any client.

### V0 graph model

Store crawler output as RustyRed graph state from the beginning:

- Nodes:
  - `Domain`
  - `Page`
  - `ContentSnapshot`
  - `CrawlRun`
  - `FetchAttempt`
  - `RobotsPolicy`
- Edges:
  - `FETCHED`: `CrawlRun -> FetchAttempt`
  - `RESULTED_IN`: `FetchAttempt -> Page`
  - `HAS_SNAPSHOT`: `Page -> ContentSnapshot`
  - `LINKS_TO`: `Page -> Page`
  - `ON_DOMAIN`: `Page -> Domain`
  - `ROBOTS_APPLIED`: `FetchAttempt -> RobotsPolicy`
  - `CANONICAL_OF`: `Page -> Page`

Use a single V0 tenant with `namespace` or label properties for `link` and future `episteme`; avoid a premature multi-tenant bridge. If V1 needs hosted tenant separation, add an explicit projection/sync API then.

### V0 service surface

Expose both operator and client surfaces:

- `GET /healthz`
- `POST /v1/crawl/runs` - create/run a bounded crawl from frontier URLs
- `GET /v1/crawl/runs/{id}` - status and counters
- `GET /v1/crawl/runs/{id}/results` - graph node/edge summaries
- `POST /v1/graph/query` or direct reuse of RustyRed query endpoints - local graph search
- gRPC equivalents once the HTTP path is stable, matching the Commonplace plan's gRPC convention.

## Implementation Slices

### RW-0: Recover the scaffold safely

1. Rebase `feat/theseus-burst-crawler-scaffold` onto current `HEAD`.
2. Preserve the security fixes from `24412cad` and `007cd610`.
3. Resolve current provider mesh drift. Current `providers/__init__.py` and `discovery.py` are ahead of the feature branch; do not lose ArcGIS, OSM, Library of Congress, or lazy `instantiate_all()` wiring.
4. Run focused checks:
   - `cargo test` in the crawler crate worktree.
   - Python import/unit tests for the adapter if the adapter is carried forward.
5. Do not deploy this scaffold as "RustyWeb complete." It only proves the fetcher seed.

Acceptance: the scaffold builds/tests on a current worktree and its remaining gaps are documented.

### RW-1: Create the RustyRed Web crate

**Started:** `rustyredcore_THG/crates/rustyred-web` now contains the first
fixture/local RustyWeb kernel in the Theorem repo. It emits the V0 graph
contract as `GraphMutationBatch` and applies it to a RustyRed `GraphStore`.
The service/fetcher layers below remain next work.

1. Add `crates/rustyred-web` to the RustyRed workspace.
2. Move/adapt the scaffold's safe pieces:
   - Axum service shell
   - bounded Tokio concurrency
   - bearer auth
   - private-IP and metadata-service guards
   - redirect policy
   - response body cap
   - structured tracing
3. Replace weak pieces:
   - Regex link extraction -> streaming `lol-html`.
   - SHA-256 content hash -> BLAKE3 content hash per V0 plan.
   - "Robots later" -> source-level robots fetch/cache/enforcement.
   - Flat fetch result -> RustyRed node/edge writes.
4. Add per-domain politeness:
   - token bucket by origin
   - robots `crawl-delay`
   - operator caps for total pages, per-domain pages, in-flight requests, and bytes.

Acceptance: local fixture crawl produces `Domain`, `Page`, `ContentSnapshot`, `CrawlRun`, `FetchAttempt`, and `LINKS_TO` edges in a RustyRed store.

**V2 contract status:** Started in `rustyredcore_THG/crates/rustyred-web`.
`build_v2_fixture_crawl` wraps the V0 fixture kernel with agent-invokable
request shape, budget enforcement, seed URL SSRF guards, scope metadata
(`source_graph`, `source_license`, `federable`, advisory tier, TTL), and
content-addressed `CrawlReceipt` / `DiscoverySeed` graph nodes. This is still
fixture-fed; replacing `FixturePage` with a live fetch loop remains RW-1/RW-2
work.

### RW-2: Graph search and frontier loop

1. Designate fulltext fields for `Page.title`, `Page.body_text`, and `ContentSnapshot.text`.
2. Add local query endpoints that return graph-shaped results, not just URLs.
3. Reuse RustyRed PPR/PageRank to prioritize outbound frontiers:
   - seed from query-matched pages and existing high-confidence pages
   - bias toward unexplored domains with healthy fetch history
   - downweight duplicate content hashes and low-success domains
4. Persist frontier decisions as graph state so future crawls learn from prior attempts.

Acceptance: a query over the crawled fixture returns pages via fulltext and suggests the next frontier via graph/PPR state.

### RW-3: Bridge Index-API to RustyWeb

1. Add an Index-API client behind `RUSTYWEB_URL` and `RUSTYWEB_API_TOKEN`.
2. Teach `apps/notebook/discovery_runs/crawl_executor.py::_select_engine` a `rustyweb` engine while preserving the Python fallback.
3. Add a Search Kernel provider/client that turns gap-frontier URLs into RustyWeb crawl runs.
4. Reconcile `/api/v2/theseus/search/crawl/`:
   - either dispatch to RustyWeb and return a job handle, or explicitly mark the endpoint disabled behind a feature flag
   - remove the current contradiction where docs/tests expect dispatch but the handler returns `501`
5. Decide the ingestion bridge:
   - V0: RustyWeb returns fetched page text/metadata and Python admits WebDocs through the existing `web.ingest`/`webdoc_writer` path.
   - V0.5: RustyWeb exposes graph deltas and Theseus subscribes/imports into THG as `WebDoc`/Object equivalents.

Acceptance: an ask/self-expansion gap can create a RustyWeb crawl run, fetch bounded pages, and make those pages visible to the existing search/ask path.

### RW-4: Deploy and smoke

1. Add Railway deploy config for RustyWeb with mandatory bearer token in any public deploy.
2. Add `/healthz`, metrics, slow-run logs, and crawl budget counters.
3. Provision the service and set `RUSTYWEB_URL`/`RUSTYWEB_API_TOKEN` on Index-API web and worker services.
4. Run a fixture smoke and a tiny public smoke:
   - 5 seeds
   - max 10 pages
   - robots obeyed
   - private-IP guard verified
   - graph query returns `Page` and `LINKS_TO`
   - Index-API bridge sees the result.

Acceptance: the service is live, token-protected, crawl-bounded, and visible from Index-API without replacing the Python fallback.

### RW-5: V1 contracts after V0 is boring

Run these in parallel with implementation only as contracts, not blockers for V0:

1. AnswerDraft schema finalization:
   - `AnswerDraft` is the first concrete `Artifact` subtype.
   - Tier A is compose/ask output persisted as graph state.
   - `GROUNDED_IN` targets must match live labels (`Claim`, `Page`/`WebDoc`).
2. Graph-yielding broker contract:
   - adapters emit typed edges, not URL lists.
   - start with CC0/permissive sources: Wikidata, OpenAlex, Common Crawl.
3. License-tiering:
   - every imported edge has `source_graph`, `source_license`, and `federable`.
   - share-alike sources default to `federable=false`.
   - prefer property-gated federation unless the RustyRed federation design forces a separate namespace.

Acceptance: V1 schemas are stable before implementation begins, and V0 does not depend on GPU/LLM synthesis.

## Parallel Handoffs

Recommended split if other Codex/Claude sessions take kernel-object gaps:

- `Artifact` + `ValidationReceipt`: own generic kernel object model and validation lineage. Avoid editing RustyWeb implementation files.
- `Subscription.delivery_policy`: own coordination kernel delivery semantics. Avoid RustyWeb crawler/client files.
- `AnswerDraft`: own schema proposal, but coordinate labels and edge semantics with RustyWeb before merging.
- RustyWeb implementation: owns Rust crate/service, Index-API RustyWeb client, crawl endpoint reconciliation, and deploy smoke.

## Stop Conditions

Stop and re-plan if any of these happen:

- The feature branch cannot be rebased without losing newer Search Kernel provider wiring.
- RustyRed core lacks an index/write primitive needed for Page/ContentSnapshot storage.
- Robots/politeness cannot be enforced at source without rewriting the fetch loop.
- The Index-API crawl endpoint has divergent product requirements from RustyWeb's job model.
- A live deploy would expose an unauthenticated public fetch proxy.

## Validation Checklist

- `cargo test` for `rustyred-core` and `rustyred-web`.
- Fixture crawl with robots allow/deny cases.
- SSRF tests for private IPv4, loopback, link-local, metadata IP, IPv6 loopback, and IPv6 unique-local.
- Body-cap test.
- Duplicate content-hash test.
- Graph node/edge count test after crawl.
- Fulltext query test over crawled content.
- PPR/frontier suggestion test.
- Django test for RustyWeb client unavailable fallback.
- Django test for RustyWeb client happy path.
- `/api/v2/theseus/search/crawl/` no longer contradicts its tests/docs.
