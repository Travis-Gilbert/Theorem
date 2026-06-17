# "Is the reranker connected?" is NOT answered by the reranker service being deployed — `web_cross_encoder_from_env()` silently falls back to `LexicalCrossEncoder` when the URL env var is missing on the CONSUMER service

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (hipporag+search-rerank verify / railway restore)`
**Domain tags:** railway, reranker, cross-encoder, env-var, silent-fallback, search-rerank-gate, rustyred-thg-server

## Trigger

Asked to confirm the rerankers were "hooked up" on Railway. The trap: the Theorem project has three reranker model services (`theorem-reranker-gte`, `-bge`, `-jina`), all deployed and SUCCESS — which *looks* like "connected." But the model servers being healthy says nothing about whether anything calls them. The actual connection is an **env var on the consumer**, and its absence is **silent**:

`rustyred-thg-server/src/router.rs::web_cross_encoder_from_env()`:
```rust
match env_nonempty(&["THEOREM_RERANKER_URL","RUSTYRED_RERANKER_URL","RUSTYWEB_RERANKER_URL"]) {
    Some(url) => Box::new(HttpCrossEncoder::new(reranker_endpoint(&url,"score"), model_id)),
    None      => Box::new(LexicalCrossEncoder::new("lexical-cross-encoder")), // <- silent degrade
}
```

If the URL env is unset on `RustyRedCore - Theorem`, search keeps working but reranks **lexically** — no error, no warning, and the three reranker services sit idle and unused. You cannot detect this by looking at the reranker services.

## Rule

To confirm a reranker is actually wired, check the env var on the **consumer** service, not the health of the reranker service:

- cross-encoder: `THEOREM_RERANKER_URL` (aliases `RUSTYRED_RERANKER_URL`, `RUSTYWEB_RERANKER_URL`) → `http://<reranker>.railway.internal:8080`; the code appends `/score`.
- listwise: `THEOREM_LISTWISE_RERANKER_URL` → appends `/rerank`.
- models: `THEOREM_RERANKER_MODEL` / `THEOREM_LISTWISE_RERANKER_MODEL`.

End-to-end proof is the **membrane receipt**: run a search and assert `reranker_version` is the model id (e.g. `Alibaba-NLP/gte-reranker-modernbert-base:membrane-v1`), NOT `lexical-cross-encoder:membrane-v1`. A lexical version string is the tell that the URL env is missing and the gate silently degraded.

## Evidence

- `RustyRedCore - Theorem` (prod) had the vars correctly set: `THEOREM_RERANKER_URL=http://theorem-reranker-gte.railway.internal:8080`, `THEOREM_LISTWISE_RERANKER_URL=http://theorem-reranker-jina.railway.internal:8080`, models `gte-reranker-modernbert-base` / `jina-reranker-v3` — so the real `HttpCrossEncoder` path is live.
- Fallback branch: `router.rs:1853-1868`. Endpoint shaping (`{base}/score` vs `{base}/rerank`): `reranker_endpoint()` at `router.rs:1888`.
- `reranker_version` string is produced by `RerankScorer::version()` = `"{model_id}:membrane-v1"` in `rustyred-rerank/src/lib.rs`.
