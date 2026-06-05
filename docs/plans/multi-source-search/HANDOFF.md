# Multi-source search acquisition pipeline

## Purpose

Two acquisition paths that compound into one index. Path 1 is borrowed breadth: front existing independent web indexes through a provider abstraction so the system can answer broad queries on day one. Path 2 is owned depth: a focused crawl over the verticals where Theseus already has a corpus. The two are wired so that every query using borrowed breadth also feeds owned depth, because the external results become crawl seeds, the crawl expands the graph, and the graph is the index. Perplexity's Bing fallback does not compound this way; ours does.

This plan sits on top of two companion handoffs in this directory: `live-web-reach` (factors the fractal post-fetch pipeline into a shared `ingest_admitted_pages`, adds `run_fractal_expansion` calling `FetchCascade::fetch_with_promotion`, registers the fractal MCP tool, and lifts the fetch tiers to Impersonate and Servo) and `coordination-room-push`. It also assumes the embedding and index decisions locked on 2026-06-04, described next.

## Substrate the pipeline writes into

One embedding space. The model is Qwen3-Embedding-4B, theorem-owned, served behind an HTTP endpoint (vLLM, or a Rust TEI process) on a GPU host (RunPod). Native output is 2560-dim, MRL-truncatable to smaller dims if storage pressure warrants. `rustyred-web` already speaks HTTP through `reqwest` and holds no ML framework, so adopting this is a matter of pointing the embedding client at the theorem endpoint. Run exactly one instance; both Theseus and theorem call it. This is the switch down from Qwen3-Embedding-8B, taken to cut generation cost, with the small precision drop recovered by the reranker.

One vector index. Turbovec (TurboQuant, 2-4 bit compression, SIMD approximate nearest neighbor, data-oblivious so new vectors add at any time with no retraining) is the single vector index across graph nodes and crawled pages. The HNSW index in `rustyred-thg-core/src/graph_store.rs` is removed; graph node-vector search routes through Turbovec like everything else. Turbovec's design assumes compressed approximate recall followed by exact rerank, which matches the cascade: Turbovec recall, then Qwen3-Reranker for precision.

Lexical stays Tantivy (`rustyred-thg-core/src/fulltext_tantivy.rs`).

Fusion is entirely Rust. Port the logic of Theseus `apps/notebook/search/retrieval.py` (`unified_retrieve`) into `rustyred-web/src/search.rs`: the reciprocal-rank-fusion merge and the eleven-stage epistemic filter. References are `docs/SPEC-C2-epistemic-retrieval.md` and the eleven-stage filter doc under the Theseus codebase-ingestion plans. `unified_retrieve.py` becomes the algorithm reference, not a service called per query. No Python hop in the search hot path.

One migration covers all of this. Re-embed the corpus with the 4B model at 2560 dims, build the single Turbovec index, drop HNSW. The 8B-to-4B re-embed and the HNSW-to-Turbovec consolidation are the same rebuild, not two. The dimension changes from 4096 to 2560, so existing vectors are not reusable and the re-embed is mandatory regardless; doing it inside the index consolidation makes it free.

## Provider layer (borrowed breadth)

A `SearchProvider` trait in `search.rs`:

```rust
#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn search(&self, query: &str, opts: &SearchOpts) -> Result<Vec<SearchCandidate>>;
}

pub struct SearchCandidate {
    pub url: String,
    pub title: Option<String>,
    pub snippet: Option<String>,
    pub source: String,   // provider name
    pub rank: usize,      // provider-local rank, for RRF
}
```

Providers are a config list, one impl per source. At query time, fan out across the enabled providers concurrently, dedup by normalized URL, and RRF-merge into one ranked candidate set.

Live API lane: Brave is the primary independent index. Mojeek adds independent diversity. Exa is neural find-similar, used for semantic seed discovery rather than as the keyword workhorse. SerpAPI is not an independent index; it is a Google and Bing SERP proxy, and it is the deliberate incumbent-fallback lane for long-tail coverage the independent indexes miss. Each needs a key and has a quota; put both in config and secrets.

Bulk corpus lane (offline): Common Crawl WARC on AWS, and OWI from OpenWebSearch.eu. OWI ships as open data, not an API: a CIFF inverted index, Parquet metadata and plaintext, and WARC, accessed through the owilix CLI and the MOSAIC Docker stack. There is no CIFF-to-Tantivy path, so the Rust-native route is to ingest OWI Parquet plaintext or WARC into Tantivy directly. Stand up MOSAIC as a sidecar now and treat it as another live provider, or defer to offline ingest; ingesting into the own index later is what compounds.

## Compounding crawl-feed loop (owned depth)

This is the moat. The candidates returned by the provider fan-out serve two roles at once: they are answer candidates, and they are crawl seeds.

Top-K candidates go to `FetchCascade::fetch_with_promotion` (Tier 1 `reqwest`, Tier 2 `rquest` Impersonate, Tier 3 Servo rendered, per the `live-web-reach` handoff), get extracted with `lol_html`, embedded at the 4B endpoint, and ingested into the graph as quarantined open-web pages (`open_web_pages_for_tenant`, fractal quarantine). The Turbovec index grows with each fetch. Borrowed breadth thereby compounds into owned depth on every query.

Before live traffic, bootstrap one vertical by ingesting an OWI or Common Crawl slice for it, so owned depth is not starting from zero.

## Always-web fractal

The fractal crate (`rustyred-thg-fractal`) already refuses graph-only termination and errors `fractal_web_seed_required`, so it always reaches the web; the `live-web-reach` handoff is what wires it to the live fetcher. Source its web seeds from the same `SearchProvider` fan-out rather than hardcoded seeds, so fractal expansion and query search share one acquisition layer.

## Vectorized verticals as semantic bridges

Vectors at a vertical are a bridge for seed discovery: follow meaning, not only hyperlinks. When expanding, use Turbovec nearest neighbors over the 4B space to pull semantically near seeds across domains, not just outlinks. This is the crawl's sense of smell.

Pick the vertical to vectorize first by where there is both an existing corpus and a first user, not by topic taste. The three real corpora are code, civic and local-government records, and research papers. Vectorize the one with a user first; keep the layer pluggable across verticals.

## Wiring summary

`search.rs` in `rustyred-web` holds the `SearchProvider` trait and impls, the fan-out, dedup, and RRF merge; `epistemic_filter.rs` holds the pure eleven-stage filter port, but hydration/scorer integration and the crawl-feed entry stay as follow-up work. One `FetchCascade` is owned in the MCP and server state for `DomainTierState` persistence and is shared with the fractal loop. The embedding client points at the theorem-owned 4B endpoint. Turbovec is the single vector index in core, replacing the HNSW in `graph_store.rs`, and `rustyred-web` writes crawl vectors to it. A search MCP tool is registered in `rustyred-thg-mcp/src/lib.rs` mirroring the fractal tool, running the fan-out and returning ranked candidates, optionally triggering the crawl-feed.

## Open items

The rquest pin and Servo V2 embedding API carry over from the `live-web-reach` handoff. The eleven-stage filter port should be done stage by stage against the Theseus reference. Provider keys and quotas (Brave, Exa, SerpAPI) go in config and secrets. Decide whether to stand up the OWI MOSAIC sidecar now or defer to offline ingest. Confirm the Qwen3-Reranker (4B or 8B) is served alongside the embedder for the exact rerank over Turbovec's approximate candidates.

## Implementation status (2026-06-05 Codex)

First slice landed in the Rust library layer:

- `rustyred-web/src/search.rs` now owns the borrowed-breadth acquisition contract: `SearchProvider`, `SearchOpts`, `SearchCandidate`, provider receipts, `fanout_search_providers`, normalized URL dedupe, and RRF merge.
- `rustyred-web/src/epistemic_filter.rs` is exported as the pure ranking/filter port. `SearchAcquisition::fused_candidates_for_epistemic_filter` now calibrates provider RRF scores into the filter input shape so default `min_score` does not drop every borrowed-breadth candidate. Full graph hydration and learned scorer wiring are still open.
- `StaticSearchProvider` gives tests and future config smoke checks a deterministic provider.
- `rustyred-web/src/providers.rs` now has live provider adapters for Brave, Mojeek, Exa, and SerpAPI, plus a file-backed offline JSON/JSONL seed-manifest provider for OWI/Common Crawl corpus jobs. Providers are enabled only by `RUSTYWEB_SEARCH_PROVIDERS` / `RUSTY_RED_SEARCH_PROVIDERS` plus provider-specific config; stray keys alone do not trigger outbound API calls.
- `rustyred-thg-fractal` now exposes `run_fractal_expansion_with_search_providers` and `run_fixture_fractal_expansion_with_search_providers`; provider candidates are merged into `web_seed_urls`, and receipts report provider candidates plus provider receipts.
- `rustyweb_search_acquisition` is advertised by `rustyred-thg-mcp` and intercepted by `rustyred-thg-server`; tests cover both the stable empty-provider shape and configured-provider seed URLs.
- `fractal_expansion` now calls the provider-backed wrapper when the server has configured providers; otherwise it preserves the existing explicit-seed/frontier behavior.
- `rustyred-web/src/embedding.rs` owns the Qwen3-Embedding-4B contract: model id `Qwen/Qwen3-Embedding-4B`, `Page.semantic_vec`, 2560 dimensions, cosine normalization, OpenAI/vLLM and TEI response parsing, env-configured HTTP client, crawl Page annotation, and an embedding receipt.
- Live `run_fractal_expansion` embeds admitted crawl pages when a Qwen4B endpoint is configured; fixture fractal runs stay deterministic and skip outbound embedding.
- `rustyred-thg-server` designates `Page.semantic_vec` at 2560 dimensions before live fractal writes and returns a `vector_designation` readiness payload so Redis or unsupported stores fail transparently instead of silently missing the vector index.
- `rustyred-thg-core` moved the graph vector index off `instant-distance` HNSW and onto Turbovec `IdMapIndex` with exact cosine rerank over recalled candidates. Unsupported toy dimensions fall back to exact search so small tests keep their old semantics; supported embedding dimensions exercise Turbovec.
- Focused validation is green: `cargo test -p rustyred-thg-core`, `cargo test -p rustyred-web`, `cargo test -p rustyred-thg-fractal`, `cargo test -p rustyred-thg-mcp`, and `cargo test -p rustyred-thg-server`.

Still open for the next slice:

- Add the heavier OWI/Common Crawl ingest path into Tantivy/Turbovec once the corpus target is chosen; the JSON/JSONL seed-manifest provider is only the first offline bridge.
- Wire hydrated graph fields and a learned scorer into the existing pure eleven-stage filter.
- Run the full corpus re-embed with Qwen3-Embedding-4B and rebuild the production vector indexes. The live crawl path is ready, but the historical corpus migration has not been run here.
- Confirm and serve the Qwen3-Reranker model alongside the embedder for exact rerank over Turbovec recall.

Provider env:

- `RUSTYWEB_SEARCH_PROVIDERS=brave,mojeek,exa,serpapi,offline` (or `RUSTY_RED_SEARCH_PROVIDERS`).
- Brave key: `RUSTYWEB_BRAVE_SEARCH_API_KEY`, `RUSTY_RED_BRAVE_SEARCH_API_KEY`, or `BRAVE_SEARCH_API_KEY`.
- Mojeek key: `RUSTYWEB_MOJEEK_SEARCH_API_KEY`, `RUSTY_RED_MOJEEK_SEARCH_API_KEY`, or `MOJEEK_SEARCH_API_KEY`.
- Exa key: `RUSTYWEB_EXA_API_KEY`, `RUSTY_RED_EXA_API_KEY`, or `EXA_API_KEY`.
- SerpAPI key: `RUSTYWEB_SERPAPI_API_KEY`, `RUSTY_RED_SERPAPI_API_KEY`, or `SERPAPI_API_KEY`.
- Offline seed manifest: `RUSTYWEB_OFFLINE_SEARCH_MANIFEST` or `RUSTY_RED_OFFLINE_SEARCH_MANIFEST`. The file can be a JSON array or JSONL rows with `url`, optional `title`, `snippet`, `source`, `rank`, `query`, and `terms`.
- Qwen4B embed endpoint: `RUSTYWEB_QWEN4B_EMBED_URL`, `RUSTYWEB_QWEN3_EMBEDDING_4B_URL`, `RUSTY_RED_QWEN4B_EMBED_URL`, `RUNPOD_QWEN3_EMBED_URL`, or `QWEN3_EMBEDDING_4B_URL`.
- Optional Qwen4B overrides: `RUSTYWEB_QWEN4B_MODEL_ID`, `RUSTYWEB_QWEN4B_DIMENSION`, `RUSTYWEB_QWEN4B_BATCH_SIZE`, `RUSTYWEB_QWEN4B_TIMEOUT_SECONDS`, and `RUSTYWEB_QWEN4B_REQUEST_FORMAT` (`auto`, `openai`, or `tei`).

Docs checked before implementation: Brave Web Search uses `https://api.search.brave.com/res/v1/web/search` with `X-Subscription-Token` and returns `web.results`; Exa uses `POST https://api.exa.ai/search` with `x-api-key` and `results`; SerpAPI Google Search uses `https://serpapi.com/search.json?engine=google` and `organic_results`; Mojeek uses `https://api.mojeek.com/search` with `api_key`, `q`, `fmt=json`, and `response.results`.
