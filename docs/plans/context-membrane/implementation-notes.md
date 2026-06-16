# SPEC-CONTEXT-MEMBRANE-1.0 implementation notes

Source spec: `~/Downloads/SPEC-CONTEXT-MEMBRANE-1.0.md`. Built 2026-06-16 as a
two-head co-build (claude-code + codex) over the Theorems-Harness V2 substrate.

## What it is

The context window is a cache; the graph is the backing store. Every context
decision is one of two operations on that cache, and both reduce to one
mechanism: score candidates, greedily fill a token budget, send the overflow to
the graph as recoverable handles rather than discarding it. Admission control
(this spec) and compaction eviction share one `fill_to_budget` and one graph
tier; they differ only in their `Scorer`. Web search and code-KG injection are
the two sources that feed admission.

## Crate map

| Crate | Owner | What |
|-------|-------|------|
| `rustyred-membrane` | codex core + claude-code additions | `Scorer`/`Candidate`/`Handle`/`Admission`/`fill_to_budget` (with the redundancy term), `MembraneReceipt`. Added: `recover.rs` (graph-backed lossless `context_fetch` + `admit_to_budget` + `emit_receipt`, behind the `graph-store` feature) and `compaction.rs` (`CompactionFitnessScorer`, `page_back`, the eviction scorer/path that feeds the same fill). |
| `rustyred-rerank` | codex | `RerankScorer` (impl `Scorer`), per-arm `ArmWeights::web`/`code`, `CrossEncoder` trait + `LexicalCrossEncoder` offline default + `HttpCrossEncoder`, `SequenceClassificationModel` (gte-reranker-modernbert-base / bge-reranker-v2-m3), `BenchmarkLedger` small-CPU winner selection, `ListwiseReranker` + jina-v3 HTTP seam, and `ListwiseRankScorer` for feeding listwise order into the gate. Added: `tests/ordering_quality.rs` (nDCG: RerankScorer beats RRF-only). |
| `rustyred-web` | claude-code (workflow agent) | `SearXngSearchProvider` (registered in the fan-out), `search_graph.rs`: `web_search_graph` (async fan-out + warm-substrate read + gate) and `gate_search_graph` (the sync store half), `warm_pages_task` fire-and-forget warming. |
| `rustyred-thg-code` | claude-code (workflow agent) | `ensure.rs`: `ensure_repo_kg` (SHA-keyed: snapshot load / incremental reindex / full ingest). `context_pack.rs`: `context_pack` / `code_context_pack_in_store` building `CodeSymbol` candidates with warm-centrality + task-seeded-PPR proximity, gated through the membrane. Plugin operations `context_pack` + `code_ingest_ensure`. |
| `rustyredcore_THG/hooks/session_start.sh` | claude-code | Automatic code-arm reflex: detect repo + SHA, `code_ingest_ensure`, `context_pack`, inject the gated code map. Fail-open. |
| `rustyred-thg-server` | wired by claude-code | `web_search_graph` MCP tool (async fan-out outside the tenant store lock, sync `gate_search_graph` inside a scoped lock, fire-and-forget warming). |

## Acceptance criteria (all addressed)

1. **One fill.** `fill_to_budget` is the only fill; `CompactionFitnessScorer`
   (eviction) and `RerankScorer` (admission) both feed it.
   SPEC-CONTEXT-COMPACTION `page_back` is now a caller of the same fill.
2. **Budget + lossless overflow.** `fill_to_budget` admits within budget,
   descending score; `admit_to_budget` persists deferred candidates byte-exact;
   `context_fetch` recovers them (verified by re-hash). Tests:
   `recover::tests::deferred_overflow_is_recoverable_byte_exact`,
   `context_fetch_rejects_a_tampered_digest`, plus per-arm recovery tests.
3. **Rerank > RRF.** `rustyred-rerank/tests/ordering_quality.rs`
   (`reranker_beats_rrf_only_ordering_on_fixed_set`): mean nDCG of `RerankScorer`
   beats RRF-only by > 0.10 on a fixed query set.
4. **Web fast path + warm second query.** `web_search_graph` returns from
   snippets + the warm subgraph without awaiting extraction; `warm_pages_task`
   writes `state=fetched` pages (firing `FetchCompletionHook`) fire-and-forget.
   Test: `search_graph::tests::second_query_admits_warm_subgraph_context`,
   `write_fetched_pages_lands_state_fetched_pages_for_the_hook`.
5. **SearXNG is one provider.** `SearXngSearchProvider` joins the existing
   fan-out; a provider error yields a per-provider receipt and does not fail the
   search. Test: `fanout_continues_when_one_provider_errors`.
6. **SHA-keyed code entry.** `ensure_repo_kg` checks the snapshot before any
   clone: current-SHA re-entry returns `LoadedFromSnapshot` (no re-ingest),
   changed SHA reindexes the diff. Tests:
   `ensure::tests::entering_at_current_sha_loads_snapshot_without_reingest`,
   `entering_unknown_then_new_sha_full_then_incremental`.
7. **context_pack reranks warm centrality within budget.**
   `context_pack_uses_warm_centrality_and_budgeted_membrane_admission`,
   `task_seeded_ppr_lifts_a_called_neighbor_above_an_unrelated_symbol`.
8. **Receipts.** `MembraneReceipt` reports `tokens_admitted`/`tokens_deferred`
   per gate, emitted as a content-addressed node via `emit_receipt`.

## Travis's corrections (where each landed)

- Keep the `Scorer` seam: `RerankScorer impl Scorer`; the cross-encoder is behind
  `CrossEncoder`, swappable without touching the gate.
- Fast single-forward-pass SequenceClassification reranker over causal-LM:
  `SequenceClassificationModel::{gte_modernbert_base, bge_v2_m3}`;
  `BenchmarkLedger` + `select_small_cpu_sequence_classifier` pick the small-CPU
  leader (test proves a 149M gte beats a 1.2B Qwen3 on the latency/size-penalized
  score).
- Benchmark on our own candidates: `ModelBenchmark` is measured on this repo's
  candidate set; `ordering_quality.rs` is a fixed-set nDCG harness, not a vendor
  table.
- jina-v3 listwise for the web final rerank: `ListwiseReranker` trait + jina-v3
  seam, applied to the candidate set before the gate. The ranked order is
  stamped into candidate metadata and fed through `ListwiseRankScorer`, so
  diversity can affect admission rather than merely reordering admitted context.
- Per-arm weighting: `ArmWeights::web` (relevance-dominant) vs
  `ArmWeights::code` (centrality/PPR-dominant).
- Redundancy term in `fill_to_budget`: MMR-style greedy with an exact
  `redundancy_key` match plus lexical Jaccard, gated by `ScoreContext.redundancy_penalty`.
- Spend at least as much on candidate generation as on the reranker: the web arm
  candidate pool is provider fan-out + RRF *plus* the warm substrate subgraph;
  the code arm pool is lexical search *plus* centrality-seed hits *plus*
  task-seeded PPR neighborhood.

## Code-grounded divergences

- **Recovery lives in the membrane, not a compaction crate.** The spec routes
  recovery through SPEC-CONTEXT-COMPACTION's `context_fetch`, which is not in
  tree. The single shared recovery is `rustyred_membrane::recover` behind the
  `graph-store` feature; the default membrane stays a pure cache-mechanics crate.
- **`task_token_delta_vs_baseline` is `None` on the web arm at call time** (no
  un-gated baseline token count exists at the gate); it is populated only by a
  run measured against a no-gate baseline.
- **Reranker default is the offline `LexicalCrossEncoder`.** gte-modernbert /
  bge-v2-m3 are served behind `THEOREM_RERANKER_URL` via `HttpCrossEncoder` (a
  149M reranker is a single forward pass; serve it like Theseus SBERT), and the
  learned variant is a mechanical env swap. jina-v3 listwise is served behind
  `THEOREM_LISTWISE_RERANKER_URL`. No model weights ship in the Rust crates; the
  Railway service template lives under `infra/railway/reranker-service`.
- **`code_ingest_ensure` requires a tenant** (multi-tenant substrate), unlike the
  spec's no-tenant signature. Same pattern as the obsidian-sync path-scoped
  tenant divergence.
- **SearXNG lives in the flat `providers.rs`**, not `providers/searxng.rs`,
  matching the crate's existing provider idiom.
- **The code-arm proximity blend is a damped prior-lift**, not a convex average:
  `proximity = centrality + 0.5 * seeded * (1 - centrality)`. A convex average
  pulled a task-neighborhood node below an equal-centrality peer; the lift keeps
  the warm prior a floor while task-seeded PPR raises a node above it, and at the
  0.5 gain a high-mass seed equals the prior convex value (so centrality still
  dominates a pure lexical match).

## Co-build notes

Codex built `rustyred-membrane` + `rustyred-rerank` git-only and unannounced;
claude-code read its work and built on it (recovery seam, compaction scorer,
ordering benchmark, both arms, MCP surface, reflex) on disjoint files, with the
only edits to codex's files being two append-only changes to the membrane
`Cargo.toml` and `lib.rs`. Bugs fixed in the workflow agents' arm code: a
moved-value borrow, an `EdgeRecord::new` argument-order error, the proximity
blend, and a dead duplicate `git_head_sha`.

## Adversarial verification pass (2026-06-16)

A four-lens read-only adversarial review (membrane/recovery, web arm, code arm,
spec-completeness) ran against all 8 acceptance criteria and Travis's
corrections. Findings folded in:

- **Web dedupe normalization (fixed).** The warm pool keyed on `canonicalize_url`
  (no host-lowercase / no default-port strip) while the fresh pool keyed on
  `normalize_candidate_url`; a mixed-case host or explicit `:443` leaked the same
  page in twice. Both pools now key through `normalize_candidate_url`, so a page
  present in both collapses (warm wins). `search_graph.rs::warm_url_key`.
- **Code budget vs realized payload (fixed).** `token_count` counted only
  signature + snippet, but the realized `text` also carries file_path + name, so
  the gate budget and the receipt under-counted the admitted bytes (a leaky cost
  lever). `token_count` now counts the full composed `text`.
- **MCP `context_pack` was pack-only (fixed).** The MCP op ignored `repo_url`/
  `sha` and never ran `ensure`, so a one-call pack on an un-ingested repo
  returned empty. `handle_context_pack_code_operation` now composes ensure-then-
  pack (SHA-keyed) when a fetchable `repo_url` is present and attaches
  `ingest_status` to the result.
- **Code redundancy key (changed).** Code candidates no longer use `file_path` as
  the redundancy key (same-file co-location is signal for code, not duplication);
  redundancy falls back to content-based lexical overlap, which still collapses
  genuinely near-identical boilerplate.
- **`task_token_delta_vs_baseline` (made consistent).** Both arms now populate it
  with the single-gate saving (= `tokens_deferred`: what an ungated admit-all run
  would have placed in the window minus what the gate admitted). A true per-task
  no-gate baseline is a deployed cross-run measurement.

Consciously NOT changed:

- **`combine_proximity` cold-fallback discontinuity.** The review suggested
  treating the damped lexical score as an additive floor in all branches. That
  would let a low-centrality pure-lexical match's proximity rise to ~0.25+lift
  and beat a strong centrality prior, flipping the code arm's centrality-dominant
  guarantee (worked the arithmetic: the warm-centrality test would invert). The
  lexical fallback is deliberately a cold-start crutch for fully-cold, unseeded
  nodes only; once a node has any warm centrality, the prior drives proximity.
  The boundary discontinuity is an accepted edge case, not a fix target.
- **RerankScorer is a 3-weight per-arm blend**, not the spec's literal
  `relevance + alpha*ppr + beta*epistemic`. This is Travis's per-arm weighting
  correction; the spec explicitly allows the weighting to vary since the reranker
  is a swappable `Scorer`.
- **Acceptance #3 is proven with the offline `LexicalCrossEncoder`**, not the
  served gte/bge model: the deliverable is the swappable seam; the model is a
  deploy-time HTTP swap. The benchmark proves the architecture (rerank beats RRF)
  on a fixed set, not the model.

The `DeferredContext` overflow nodes are content-addressed by the text digest, so
two identical-text candidates intentionally collapse to one node;
`source_node_id` on a collapsed node is therefore non-authoritative. Byte-exact
recovery is unaffected (`context_fetch` re-hashes the stored text).

## Build + test

```
cd rustyredcore_THG
cargo test -p rustyred-membrane --features graph-store   # core + recovery + compaction
cargo test -p rustyred-rerank                             # scorer + benchmark
cargo test -p rustyred-web                                # web arm
cargo test -p rustyred-thg-code                           # code arm
cargo test -p rustyred-thg-server                         # web_search_graph MCP tool
```

## Open / handoff

- SearXNG container deploy (self-hosted, no API key) on Railway: codex lane.
- jina-v3 listwise + gte/bge cross-encoder HTTP services: deploy + point
  `THEOREM_RERANKER_URL` / listwise URL at them; the seams are in place.
- SPEC-CONTEXT-COMPACTION `page_back` refactors onto `fill_to_budget` when that
  crate lands (the `CompactionFitnessScorer` is ready).
