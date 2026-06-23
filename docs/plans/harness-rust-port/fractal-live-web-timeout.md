# Fractal live-web route times out at the MCP client (async handoff needed)

Date: 2026-06-06
Reporter: Claude Code
Owner: Codex (async harness-server route)
Coordination: the native room was down (coordinate writes timing out through the same
RustyRedCore MCP server), so this is the git-fallback handoff note.

## Symptom

On the live `RustyRedCore - Theorem` server (rustyredcore-theorem-production), both
`fractal_expansion` and `rustyweb_search_acquisition` return HTTP 499 on `POST /mcp`
after ~25-60s. The MCP client times out and closes the connection before the synchronous
live-web operation returns, so no receipt is ever delivered.

HTTP log evidence (production, 2026-06-06):

```
06:46-06:47  POST /mcp 200  69-750ms     (normal MCP calls: tool list, handshake, etc.)
06:50:43     POST /mcp 499  29937ms      (fractal_expansion)
06:51:38     POST /mcp 499  29910ms      (retry)
06:52:41     POST /mcp 499  24936ms      (rustyweb_search_acquisition, search-only)
06:53:22     POST /mcp 499  60000ms      (waited a full 60s, still aborted)
```

## Isolation (what is NOT the cause)

- The Perplexity provider is deployed and correct: `PerplexitySearchProvider` landed in
  `rustyred-web/src/providers.rs` at `9567e7d4`, unit test green, env set
  (`RUSTYWEB_SEARCH_PROVIDERS=perplexity` + `RUSTYWEB_PERPLEXITY_API_KEY`) on the service.
- Perplexity + the key are fast and valid: the same key/query against a standalone Perplexity
  client returned 5 ranked results with extracted content in a few seconds.
- The request reaches the server and routes correctly (the 499s are `POST /mcp`, not 404s).

So this is a server-route latency/concurrency problem, not the provider, key, or config.

## Likely root cause

Two coupled issues in the async harness-server live-web route:

1. Synchronous request lifetime. The route holds ONE MCP request open through the entire
   search -> crawl pipeline (`fanout_search_providers` + per-seed `FetchCascade` with
   escalation tiers). That exceeds the MCP client's ~25-30s timeout. Even the search-only
   acquisition path exceeds it, so the search step from the server is already slow, or the
   route has heavy first-call init overhead.

2. Executor starvation (stronger signal). During the long requests, concurrent lightweight
   probes also degraded:

   ```
   06:52:11  GET /.well-known/oauth-authorization-server/mcp  499  4894ms
   06:52:16  GET /mcp/.well-known/oauth-authorization-server  499  4939ms
   06:52:21  GET /.well-known/oauth-authorization-server      499  4937ms
   ```

   ~5s latencies on trivial discovery GETs (and the room `coordinate` write timing out
   through the same server) suggest the live-web work is blocking the async executor: a
   blocking call (sync IO / blocking sleep) somewhere in the fetch cascade running on a tokio
   worker, starving other tasks.

## Proposed fix (Codex's route)

- Async run handoff: `fractal_expansion` / `rustyweb_search_acquisition` should kick off the
  work, persist a `HarnessRun`, return a `run_id` immediately, and let the client poll for the
  receipt (reuse the harness run lifecycle / `harness_run` readback). Do not hold the MCP
  request open through live web IO.
- Non-blocking fetch: ensure `FetchCascade` does not block the tokio worker (`spawn_blocking`
  for any sync IO, or fully async fetch with bounded per-tier timeouts), so one expansion
  cannot starve concurrent requests.
- Bounded per-provider + per-seed timeouts so a slow upstream fails fast with a receipt
  (`web_seed_failures` / `provider_receipts` error) instead of hanging.

## Status / impact

The Perplexity provider integration is complete and validated; the only thing blocking the
live `/research` web demo (Perplexity -> crawl -> corpus) is this route. Once the route
returns a `run_id` and the client polls, the `perplexity` `provider_receipt` + admitted pages
will be observable end to end.
