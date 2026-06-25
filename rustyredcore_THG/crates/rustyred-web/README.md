# rustyred-web

RustyWeb: the graph-native crawler and search kernel. It turns fetched or fixture HTML pages into graph state (a `CrawlGraph` of node/edge mutations) and provides the read side (substrate search, SERP rendering, the epistemic fusion filter) plus a Servo-free browser-automation contract.

## Key API

- Page-to-graph: `build_fixture_crawl_graph(CrawlConfig, &[FixturePage]) -> CrawlGraph`, `build_v2_fixture_crawl(CrawlRequest, &[FetchedPage]) -> CrawlRunOutput`, async `run_live_crawl[_with_options]`, `fetch_seed_pages`. Apply with `CrawlGraph::apply_to_store(&mut impl GraphStore)`.
- URLs/links: `canonicalize_url`, `guarded_canonicalize_url`, `extract_links[_with_profile]`, `graph_delta_hash`. Content hashing is BLAKE3.
- Types: `CrawlConfig`, `FixturePage` (alias `FetchedPage`), `CrawlRequest`, `CrawlBudget`, `CrawlScope`, `CrawlReceipt`, `CrawlRunOutput`, `CrawlGraph`, `UrlGuardPolicy`, `RustyWebError`.
- Web Commons federation: `build_web_commons_fragment`, `build_web_commons_ingest_plan`, `WebCommonsFragment`/`Receipt`/`IngestPlan`.

## Module map

- `lib.rs`: crawl graph build, URL guard/canonicalization, link extraction, Web Commons.
- `fetch_cascade.rs`: tiered fetch (`FetchCascade`, reqwest then impersonate then rendered endpoint).
- `cache.rs`: Valkey/Redis cache-aside accelerator (never source of truth).
- `frontier/`: self-organizing crawl frontier (`Frontier`, queues, prioritizers incl. `PprPrioritizer`, politeness, fetchers, runner).
- `crawl_hooks.rs`: graph-store hooks for PPR frontier recompute and entity extraction.
- `browser_engine.rs`: `FetchCascadeBrowserEngine`, `web_consume_to_graph`, `WebConsumeRequest`/`Receipt`. Re-exports `BrowserAction`, `PageState`, `WaitCondition`, `InteractiveElement`, `ElementBox`, `BrowserActionPolicy` from `pilot-core` (those types are defined there).
- `browser_automation.rs`, `browser_driver.rs`, `browser_perception.rs`, `browser_run.rs`: Playwright-shaped, Servo-free automation, accessibility perception/governance, and record/replay.
- `providers.rs`: search providers (Brave, Mojeek, Exa, SerpApi, Perplexity, Firecrawl, SearXng, Offline).
- `robots.rs`, `source_class.rs`, `trigger_gate.rs`: robots handling, source classification, web-reach gating.
- `embedding.rs`: page-vector annotation (`QwenEmbeddingClient`, `Qwen/Qwen3-Embedding-4B`, D=2560, `semantic_vec` property).
- `search.rs`, `search_graph.rs`, `serp.rs`: substrate search seam, the WEB arm of the context membrane, and self-contained SERP HTML.
- `epistemic_filter.rs`: the eleven-stage epistemic fusion filter (Rust port of Theseus `retrieval.py`).
- `relevance.rs`: query-aligned passage extraction.

## Features

`default = []`. `redis-frontier`, `valkey-cache`, `servo` (marker), `spider_fetch`, `impersonate-fetch`, `accesskit`, `vector-accelerated`. Path deps: `rustyred-membrane` (feature `graph-store`), `rustyred-hipporag`, `rustyred-rerank`, `rustyred-thg-core`, `pilot-core`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-web
cargo run -p rustyred-web --example serp_preview
```

Tests: `tests/` (`fixture_crawl`, `crawl_hooks`, `frontier_acceptance`, `epistemic_parity`) plus extensive inline tests. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
