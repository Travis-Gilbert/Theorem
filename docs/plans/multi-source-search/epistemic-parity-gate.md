# Epistemic-filter Python parity gate

**Status:** Rust half landed; Python half is Codex's (needs the Theseus runtime).
**Goal:** prove `rustyred-web::apply_epistemic_filter` is byte-faithful to Theseus
`apps/notebook/search/retrieval.py::_fuse_score_and_threshold` over a shared
fixture set. This closes the parity step flagged in
`docs/plans/multi-source-search/HANDOFF.md` ("the eleven-stage filter port should
be done stage by stage against the Theseus reference").

## The shared contract

`rustyredcore_THG/crates/rustyred-web/tests/epistemic_parity_fixtures.json` is the
single source of inputs. Both halves run every case and emit the same output
shape; the gate asserts the two outputs are equal.

### Per-field defaults (apply identically on both sides)

Candidate: `epistemic_weight=1.0`, `acceptance_status="accepted"`,
`justification_prior=0.5`, `slug=null`, `title=""`, `source_system=null`,
`code_entity_type=null`, `signals={}`.
Config: `top_n=50`, `min_score=0.1`, `include_provisional=false`,
`include_contested=false`, `exclude_acquaintance=true`, `include_code=false`,
`query_type=null`.

### Output shape (both goldens)

A JSON array of `{ "name": <case>, "results": [ row, ... ] }`, where each `row`
is, in this field order:

```json
{
  "object_pk": "<pk>",
  "rrf_score": <round(rrf, 6)>,
  "learned_score": <round(learned, 4) after all boosts>,
  "scoring_method": "rrf_fallback",
  "epistemic_weight": <float>,
  "acceptance_status": "<status>",
  "title_overlap_bonus": <float or null>
}
```

`results` is in final ranked+filtered+sliced order (the function's return value).
The Rust golden is committed at
`rustyredcore_THG/crates/rustyred-web/tests/epistemic_parity_rust_golden.json` for
reference; the gate compares **parsed values**, not JSON text, so float
formatting differences between the two encoders never cause a false mismatch.

## Codex's half: the Python harness

Write a script (suggested `apps/notebook/search/parity_harness.py` in Theseus, or
a standalone test) that imports the REAL `_fuse_score_and_threshold` and runs each
fixture case through it, then writes
`rustyredcore_THG/crates/rustyred-web/tests/epistemic_parity_python_golden.json`
(path it via the Theorem checkout, or hand the file to Claude to drop in).

The function computes Stage 2 (RRF) internally from `pass_results`, but the
fixtures supply `rrf_score` directly (Stage 2 is Codex's `search.rs` RRF, tested
separately). So bypass Stage 2 and inject the fixture scores:

1. **Inject the fused list.** Monkeypatch the RRF entry the function uses
   (`weighted_reciprocal_rank_fusion` when `USE_WEIGHTED_RRF`, else
   `reciprocal_rank_fusion`) to return, in fixture order,
   `[(candidate.object_pk, candidate.rrf_score, {}) for candidate in case]`. Pass
   any non-empty `pass_results` so the function reaches the fusion call. After the
   patch, `candidate_pks` and `rrf_score_map` come straight from the fixture.
   (Or set `USE_WEIGHTED_RRF=false` and patch only `reciprocal_rank_fusion`.)

2. **Mock Object hydration (Stage 4).** Patch the `Object.objects.filter(...)`
   queryset so it returns, for each fixture candidate, a lightweight object whose
   attributes are the fixture fields:
   `pk=object_pk`, `epistemic_weight`, `acceptance_status`,
   `justification_confidence_prior=justification_prior`, `slug`, `title`,
   `source_system`, `properties={"code_entity_type": code_entity_type}`, plus the
   feature-vector attrs `body=""`, `edge_count=0`, `notebook=None`,
   `graph_uncertainty=0.0` so `_build_query_feature_vector` does not crash.
   Preserve `candidate_pks` order. Skip the `_NON_PROPOSITIONAL_TYPES` /
   `FACTUAL_OBJECT_FILTER` exclusions for the mock (or set the mock's
   `object_type.slug` to something other than `script` and `subtype="factual"`).

3. **Stub the seams to their identity/empty behavior:**
   - `score_connection(fv)`: raise, so Stage 5 takes the `except` branch
     (`learned = rrf_score`, `method = "rrf_fallback"`). This matches the Rust
     `RrfFallbackScorer` exactly.
   - `scope_query_to_world(query)` -> `{"mode": "empirical"}`;
     `filter_queryset_for_world_scope(qs, scope)` -> `qs` (identity).
   - `_filter_out_code_pks(pks)`: replace the DB lookup with the pure check over
     the fixture fields -- drop a pk when its `source_system == "codebase"` OR its
     `code_entity_type` is in the 7-element set
     `{code_file, code_structure, code_member, code_process, specification,
     fix_pattern, commit}`. (This is what the Rust Stage 3b does.)
   - `_retrieve_code_debug_candidates`: not exercised (no fixture injects extra
     code-debug pks); stub to `[]`.
   - temporal `filter_retrieval_results_by_scope`: not called (no `as_of`/
     `between`); leave as is.

4. **Call** `_fuse_score_and_threshold(...)` per case with the fixture's `query`,
   `top_n`, `min_score`, `include_provisional`, `include_contested`,
   `exclude_acquaintance`, `include_code`, `query_type`, and `as_of=None`,
   `between=None`, `enable_nli=False`.

5. **Project each returned result dict** to the row shape above:
   `object_pk` = `result["object_pk"]`, `rrf_score` = `result["rrf_score"]`
   (already `round(_,6)`), `learned_score` = `result["learned_score"]` (already
   `round(_,4)`), `scoring_method` = `result["scoring_method"]`,
   `epistemic_weight` = `result["epistemic_weight"]`, `acceptance_status` =
   `result["acceptance_status"]`, `title_overlap_bonus` =
   `result.get("title_overlap_bonus")` (None when the stage did not fire).

## Closing the gate

Drop `epistemic_parity_python_golden.json` next to the fixtures. Then
`cargo test -p rustyred-web --test epistemic_parity` runs
`python_golden_matches_rust_when_present`, which asserts the Rust output equals
the Python golden value-for-value. A failure is a real parity divergence -- the
most likely culprit is banker's-rounding at an interleave point (Rust uses
`round_ties_even` on the scaled value; Python `round()` is correctly-rounded
decimal half-to-even). The `rounding_interleave` fixture (`r` -> `0.2482`, via
Stage-5 round `0.123457 -> 0.1235`, Stage-6 `*1.2 -> 0.1482`, Stage-8 `+0.1 ->
0.2482`) is the canary; if it diverges, reconcile the rounding method, not the
fixture.

## Why this is a faithful gate, not a re-port

The Python side runs the ACTUAL `_fuse_score_and_threshold`, not a
re-implementation, so the gate compares the Rust port against ground truth. Only
the seams that are genuinely DB/ML (object hydration, the learned scorer, world
scope, the code-pk DB lookup) are stubbed -- and each is stubbed to the exact
identity/empty behavior the Rust port assumes for those seams. The pure ranking
stages (3a, 3b, 5-11) run unmodified on both sides.
