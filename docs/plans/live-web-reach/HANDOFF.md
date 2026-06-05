# Handoff: Live Web Reach for the Harness

The web-reaching logic already exists in `rustyredcore_THG/crates/rustyred-thg-fractal/src/lib.rs`, and a real live fetcher already exists in `rustyredcore_THG/crates/rustyred-web/src/fetch_cascade.rs`. Nothing connects them, and the fractal crate is not exposed as an MCP tool. The fractal verb that is currently exposed is the Theseus/Python `theorem_fractal_expansion`, which stops at the graph. So the harness cannot reach the live web today even though every piece to do it is in the repo. This handoff is two parts. Part A makes web search exist as a callable harness capability. Part B makes it good. They are separable, and Part A benefits automatically when Part B lands.

## Part A: expose and wire the live fractal loop

The current fractal runner is fixture-fed, not fixture-logic. `run_fixture_fractal_expansion(store, request, fetched_pages)` takes the already-fetched pages as an argument and runs the real pipeline on them: `graph_frontier` via `search_substrate`, then `web_seeds_from_frontier`, then `rank_and_sanitize_pages`, then `build_v2_fixture_crawl` to turn admitted pages into graph mutations, then `annotate_open_web_batch` to stamp `trust_tier = open_web_unverified`, `quarantine = true`, `confidence_ceiling = 0.35`, then `apply_to_store`. The only thing fixtured is the source of the pages. The page-to-graph build and the quarantined ingest are production logic.

Add a live runner beside the fixture one. Factor the post-fetch pipeline (rank, sanitize, build, annotate, apply) out of `run_fixture_fractal_expansion` into a shared private function, `ingest_admitted_pages(store, &request, &web_seed_urls, &admitted_pages, &frontier)`, and have both runners call it. Then add:

```rust
pub async fn run_fractal_expansion<S: GraphStore>(
    store: &mut S,
    request: FractalExpansionRequest,
    cascade: &FetchCascade,
    max_bytes: usize,
) -> ThgResult<FractalExpansionReceipt> {
    let request = request.normalized();
    validate_request(&request)?;

    let frontier = graph_frontier(store, &request);
    let web_seed_urls = web_seeds_from_frontier(&request, &frontier);
    if web_seed_urls.is_empty() {
        return Err(ThgError::new(
            "fractal_web_seed_required",
            "fractal expansion cannot terminate at the graph frontier",
        ));
    }

    let mut fetched = Vec::new();
    for url in &web_seed_urls {
        match cascade.fetch_with_promotion(url, max_bytes).await {
            Ok(result) => fetched.push(fetched_page_from_tier_result(url, result)),
            Err(_) => continue, // a dead seed should not fail the run
        }
    }

    ingest_admitted_pages(store, &request, &web_seed_urls, &fetched, &frontier)
}
```

The adapter from `FetchTierResult` to `FetchedPage`. `rank_and_sanitize_pages` reads `page.url`, `page.body`, `page.status`. `FetchTierResult` carries `final_url`, `html_bytes: Vec<u8>`, `http_status: u16`. The adapter decodes the body lossily and carries the status:

```rust
fn fetched_page_from_tier_result(seed_url: &str, r: FetchTierResult) -> FetchedPage {
    let body = String::from_utf8_lossy(&r.html_bytes).into_owned();
    // FetchedPage::html currently hard-sets status 200; carry the real status
    // through whatever constructor exists, extending FetchedPage if needed.
    FetchedPage::with_status(
        if r.final_url.is_empty() { seed_url } else { &r.final_url },
        &body,
        r.http_status,
    )
}
```

If `FetchedPage` only exposes `::html(url, body)` today, add a constructor that carries status, since the rank step uses `page.status == 200` as a keep condition. That is the one place the existing struct likely needs a small extension.

Own the `FetchCascade` in the MCP server, not per-call. `DomainTierState` learns the working tier per domain and only promotes upward, so it must persist across runs. Construct one `FetchCascade` in the `rustyred-thg-mcp` server state from `FetchCascadeOptions { user_agent, timeout_seconds }` and borrow it into the handler.

Register the MCP tool in `rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs`. That file already registers every other verb (`graph_query`, `neighbors`, `schema`), so mirror one of those handlers. The tool is `fractal_expansion` with an input schema of `query` (required), `tenant_id` (from session), and optional `web_seed_urls`, `top_k`, `frontier_limit`, `web_seed_limit`, `embedder_model`, `actor_id`. The handler builds a `FractalExpansionRequest` from the args plus server context (generate `run_id`, take `tenant_id` from the session, take `actor_id` from caller identity), calls `run_fractal_expansion` with the server's `FetchCascade` and `GraphStore`, and returns the `FractalExpansionReceipt`. The receipt already reports `web_reached`, `frontier`, `admitted_pages`, `applied_writes`, and `crawl_receipt_id`, which is the right "what did it reach and ingest" surface. The quarantined nodes remain queryable through the existing `open_web_pages_for_tenant`.

When Part A lands, the harness has live web reach over ordinary HTTP-fetchable pages, with quarantined lower-trust ingest, callable from claude.ai, Claude Code, and Codex. The Theseus graph-only fractal verb can stay for the research corpus, but the product path now points at this one.

## Part B: Tier 2 (Impersonate) and Tier 3 (Rendered, Servo)

`fetch_cascade.rs` already has the right shape: a `FetchTier` enum of `Http2`, `Impersonate`, `Rendered`, per-domain promotion state, and a `should_promote` that already detects when escalation is needed (403, 429, 503, sub-512-byte bodies, or Cloudflare interstitial markers). The cap is `FetchTier::max_supported()` returning `Http2`, and `fetch_with_promotion` records promotion intent but never acts above Tier 1. Part B implements the two higher tiers and lifts the cap.

Dispatch on tier. Change `fetch_with_promotion` to route by the desired tier rather than capping it:

```rust
let result = match supported_tier {
    FetchTier::Http2 => self.fetch_http2(url, max_bytes, supported_tier).await?,
    FetchTier::Impersonate => self.fetch_impersonate(url, max_bytes).await?,
    FetchTier::Rendered => self.fetch_rendered(url, max_bytes).await?,
};
```

and raise `max_supported` to `Rendered` once both are implemented.

Tier 2, Impersonate. This is a fetch that mimics a real browser's TLS/JA3 plus HTTP/2 fingerprint and header ordering, so anti-bot layers that fingerprint and block default `reqwest` let it through. Add a second client inside `FetchCascade` for this tier and keep the plain `reqwest` client for Tier 1. The leading Rust option is `rquest` (the maintained successor to `reqwest-impersonate`, BoringSSL-backed, JA3/JA4 and HTTP2 fingerprint impersonation with Chrome/Firefox/Safari profiles). The fallback is shelling out to a `curl-impersonate` binary, which is heavier because it is process-per-fetch. Use `rquest`, and confirm its current API and maintenance before pinning, because this corner of the ecosystem churns. The promotion path is already there: `should_promote` fires, the domain is promoted to `Impersonate`, and the next fetch for that domain uses the impersonating client.

Tier 3, Rendered, is Servo. This is locked. Two things make it more than a fetch tier. First, Tier 3 Rendered is the Servo-in-RustyRed crawl enhancement scoped earlier; the `FetchTier::Rendered` variant is precisely that escalation slot, so this build and "embed Servo as an optional crawl enhancement" are one thing, not two. Second, it is the shared engine for the planned Servo browser-use agent: a Servo you can drive to render is a Servo you can drive to navigate, click, and extract. So structure the Servo integration as a drivable engine with a `navigate`, `render`, and `extract` surface from the start, not a one-shot renderer, so the browser-use agent reuses it. Keep it pooled and on-demand, not browser-per-fetch, and offload heavy compute to RunPod rather than the local or Railway CPU. Servo recently shipped a V2 with increasing coverage; confirm the current embedding API when wiring it.

The gate that protects the evidence bar. Tier 2 impersonation is, precisely, defeating anti-bot defenses. That is fine on open sites that merely run aggressive bot protection, and it is exactly the thing never to do on a source whose terms forbid automated access. So promotion to `Impersonate` or `Rendered` must pass the `robots.rs` and `trigger_gate.rs` checks first. Impersonation is gated to open sources, never fired at a source whose terms prohibit automation. Wire that check before the promotion in `should_promote`'s consumer, not after.

Resource discipline falls out for free. Both higher tiers are escalation-only, so cost is bounded to the sites that demand them, and `DomainTierState` persists the learned tier so repeat fetches skip straight to what works. Tier 3's renderer is pooled and offloaded.

When Part B lands, web search is good: ordinary pages via Tier 1, bot-defended open pages via Tier 2, JS-heavy pages via Tier 3, each escalated only when needed and each gated by the terms check.

## Open items to confirm at build time

The `rquest` pin (current API and maintenance). The Servo V2 embedding API. Both were flagged because they were not verifiable at authoring time without a live check.
