# rustyred-thg-fractal

Native fractal expansion over the RustyRed substrate and RustyWeb. A run is corpus growth, not graph-only retrieval: the runner always builds a web crawl request and ingests admitted web graph state into a lower-trust, quarantined tier.

## Key API

- Sync fixture runners: `run_fixture_fractal_expansion<S>(&mut S, FractalExpansionRequest, &[FetchedPage]) -> ThgResult<FractalExpansionReceipt>`, `run_fixture_fractal_expansion_with_scorer`.
- Async live runners: `run_fractal_expansion`, `run_fractal_expansion_with_scorer`, `run_fractal_expansion_with_search_providers`, and `..._and_scorer` variants.
- `open_web_pages_for_tenant(&S, tenant_id) -> Vec<String>`.
- Types: `FractalExpansionRequest`, `FractalExpansionReceipt`, `FractalFrontierHit`, `FractalProviderCandidate`, `FractalProviderReceipt`, `FractalPageExcerpt`.
- Quarantine: `OPEN_WEB_UNVERIFIED_TRUST_TIER = "open_web_unverified"`, `DEFAULT_OPEN_WEB_CONFIDENCE_CEILING = 0.35`. Default embedder model is Qwen3-Embedding-4B; default budget 2000 tokens.

Path deps: `rustyred-hipporag`, `rustyred-membrane` (feature `graph-store`), `rustyred-rerank`, `rustyred-thg-core`, `rustyred-web`.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p rustyred-thg-fractal
```

Tests are inline in `lib.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
