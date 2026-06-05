#!/usr/bin/env python3
"""Theseus-side parity oracle for the eleven-stage epistemic filter.

Reproduces the PURE stages of Theseus
`apps/notebook/search/retrieval.py::_fuse_score_and_threshold` VERBATIM (stages
3a, 3b, 5-11), executed by CPython so Python's actual `round()` (banker's,
round-half-to-even), `list.sort()` (stable), and float arithmetic are the oracle
the Rust port (`rustyred-web::apply_epistemic_filter`) is checked against.

The DB/ML/world/temporal seams are stubbed exactly as the Rust `RrfFallbackScorer`
path assumes, identically on both sides:
  - rrf is injected from the fixture (Stage 2 RRF is upstream / Codex's search.rs),
  - `score_connection` raises -> the `except` branch (learned = rrf,
    method = 'rrf_fallback'),
  - objects are the fixture candidates (no Object hydration),
  - world scope / temporal / code_debug-DB re-admission are no-ops.

This is the verbatim Python stage logic run in CPython, not the imported Django
function, so it is a faithful arithmetic/ordering oracle. Cross-confirm against
the live `_fuse_score_and_threshold` import in the Theseus runtime when available
(the stages are unchanged; only the stubbed seams differ).

Usage:
  python3 docs/plans/multi-source-search/parity_harness.py
Writes epistemic_parity_python_golden.json next to the shared fixtures.
"""
import json
import os

# Stage 3b code-entity set (the 7-element, second/winning definition in source).
_CODE_ENTITY_TYPES = {
    "code_file",
    "code_structure",
    "code_member",
    "code_process",
    "specification",
    "fix_pattern",
    "commit",
}


def _filter_out_code_pks(candidate_pks, objs):
    drop = set()
    for pk in candidate_pks:
        obj = objs[pk]
        if obj["source_system"] == "codebase" or (
            obj.get("code_entity_type") in _CODE_ENTITY_TYPES
        ):
            drop.add(pk)
    return [pk for pk in candidate_pks if pk not in drop]


def fuse(case):
    cfg = case.get("config", {})
    top_n = cfg.get("top_n", 50)
    min_score = cfg.get("min_score", 0.1)
    include_provisional = cfg.get("include_provisional", False)
    include_contested = cfg.get("include_contested", False)
    exclude_acquaintance = cfg.get("exclude_acquaintance", True)
    include_code = cfg.get("include_code", False)
    query_type = cfg.get("query_type")
    query_text = case["query"]

    # Build the fixture's view of objects, fused list, and per-pk signals (the
    # stubbed Stage 2 + Stage 4: rrf injected directly, objects from the fixture).
    objs = {}
    fused = []
    signal_scores_map = {}
    for cand in case["candidates"]:
        pk = cand["object_pk"]
        objs[pk] = {
            "epistemic_weight": cand.get("epistemic_weight", 1.0),
            "acceptance_status": cand.get("acceptance_status", "accepted"),
            "justification_confidence_prior": cand.get("justification_prior", 0.5),
            "slug": cand.get("slug"),
            "title": cand.get("title", ""),
            "source_system": cand.get("source_system"),
            "code_entity_type": cand.get("code_entity_type"),
        }
        fused.append((pk, cand["rrf_score"], {}))
        signal_scores_map[pk] = cand.get("signals", {})

    # ---- Stage 3a: candidate truncation to 2 * top_n
    top_candidates = fused[: 2 * top_n]
    if not top_candidates:
        return []
    candidate_pks = [pk for pk, _, _ in top_candidates]
    rrf_score_map = {pk: score for pk, score, _ in top_candidates}

    # ---- Stage 3b: code-object exclusion gate
    if not include_code and query_type != "code_debug" and candidate_pks:
        candidate_pks = _filter_out_code_pks(candidate_pks, objs)

    # ---- Stage 5: learned scoring (score_connection stubbed -> rrf_fallback)
    scored_results = []
    for pk in candidate_pks:
        obj = objs.get(pk)
        if not obj:
            continue
        rrf_score = rrf_score_map.get(pk, 0.0)
        signals = signal_scores_map.get(pk, {})
        # score_connection(fv) raises -> the except branch in the source:
        learned_score = rrf_score
        method = "rrf_fallback"
        scored_results.append(
            {
                "object_pk": pk,
                "rrf_score": round(rrf_score, 6),
                "learned_score": round(learned_score, 4),
                "scoring_method": method,
                "signals": signals,
            }
        )

    # ---- Stage 6: epistemic-weight boosting
    for result in scored_results:
        obj = objs.get(result["object_pk"])
        if obj is None:
            continue
        ew = obj["epistemic_weight"]
        result["epistemic_weight"] = ew
        result["learned_score"] = round(result["learned_score"] * ew, 4)
        result["acceptance_status"] = obj.get("acceptance_status", "accepted")
        if query_type == "code_debug" and result["signals"].get("code_boost"):
            result["learned_score"] = round(result["learned_score"] + 0.25, 4)
        result["justification_prior"] = obj.get("justification_confidence_prior", 0.5)

    # ---- Stage 7: slug deduplication (strict >, index semantics)
    seen_slugs = {}
    deduped_indices = set()
    for i, result in enumerate(scored_results):
        obj = objs.get(result["object_pk"])
        slug = obj.get("slug") if obj else None
        if slug and slug in seen_slugs:
            prev_idx = seen_slugs[slug]
            if result["learned_score"] > scored_results[prev_idx]["learned_score"]:
                deduped_indices.add(prev_idx)
                seen_slugs[slug] = i
            else:
                deduped_indices.add(i)
        elif slug:
            seen_slugs[slug] = i
    if deduped_indices:
        scored_results = [
            r for i, r in enumerate(scored_results) if i not in deduped_indices
        ]

    # ---- Stage 8: title-query overlap bonus
    query_words = set(query_text.lower().split())
    for result in scored_results:
        obj = objs.get(result["object_pk"])
        title_words = set(((obj.get("title") if obj else "") or "").lower().split())
        overlap = len(query_words & title_words)
        if overlap >= 1:
            bonus = min(0.3, overlap * 0.10)
            result["learned_score"] = round(result["learned_score"] + bonus, 4)
            result["title_overlap_bonus"] = bonus

    # ---- Stage 9: sort (stable) + min-score + acquaintance
    scored_results.sort(key=lambda x: x["learned_score"], reverse=True)
    filtered = [r for r in scored_results if r["learned_score"] >= min_score]
    if exclude_acquaintance:
        filtered = [r for r in filtered if r.get("epistemic_weight", 1.0) > 0.0]

    # ---- Stage 10: acceptance-status filtering
    filtered = [
        r for r in filtered if r.get("acceptance_status", "accepted") != "retracted"
    ]
    if not include_provisional:
        filtered = [
            r
            for r in filtered
            if r.get("acceptance_status", "accepted") != "provisional"
        ]
    if not include_contested:
        filtered = [
            r for r in filtered if r.get("acceptance_status", "accepted") != "contested"
        ]

    # ---- Stage 11: temporal scope (no-op here) + final top_n slice
    return filtered[:top_n]


def project(result):
    """Project to the shared parity row shape (matches the Rust ResultRow)."""
    return {
        "object_pk": result["object_pk"],
        "rrf_score": result["rrf_score"],
        "learned_score": result["learned_score"],
        "scoring_method": result["scoring_method"],
        "epistemic_weight": result["epistemic_weight"],
        "acceptance_status": result["acceptance_status"],
        "title_overlap_bonus": result.get("title_overlap_bonus"),
    }


def main():
    tests_dir = os.path.join(
        os.path.dirname(os.path.abspath(__file__)),
        "..",
        "..",
        "..",
        "rustyredcore_THG",
        "crates",
        "rustyred-web",
        "tests",
    )
    tests_dir = os.path.normpath(tests_dir)
    fixtures_path = os.path.join(tests_dir, "epistemic_parity_fixtures.json")
    out_path = os.path.join(tests_dir, "epistemic_parity_python_golden.json")

    with open(fixtures_path) as handle:
        fixtures = json.load(handle)

    out = []
    for case in fixtures["cases"]:
        results = fuse(case)
        out.append({"name": case["name"], "results": [project(r) for r in results]})

    with open(out_path, "w") as handle:
        json.dump(out, handle, indent=2)
        handle.write("\n")
    print(f"wrote {out_path} ({len(out)} cases)")


if __name__ == "__main__":
    main()
