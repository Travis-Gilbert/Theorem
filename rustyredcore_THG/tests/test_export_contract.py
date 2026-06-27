"""ABI contract for the ``theseus_native`` PyO3 export surface."""

from __future__ import annotations

import importlib

import pytest

try:
    theseus_native = importlib.import_module("theseus_native")
except ModuleNotFoundError as exc:
    if exc.name != "theseus_native":
        raise
    pytest.fail("theseus_native is not importable")

TOP_LEVEL_EXPORTS = [
    "push_ppr",
    "push_ppr_filtered",
    "adapters",
    "theseus_native",
    "ThgError",
    "cmh_body_hash",
    "cmh_atom_id_v1",
    "cmh_handoff_state_hash_v1",
    "bgi_stable_hash_json",
    "bgi_fact_pack_hash_rows_json",
    "bgi_egraph_receipt_summary_json",
    "bgi_egraph_extract_context_pack_json",
    "bgi_datalog_receipt_summary_json",
    "bgi_datalog_verified_rule_ids_json",
    "bgi_datalog_derive_core_json",
    "bgi_probabilistic_source_reliability_json",
    "bgi_probabilistic_expected_value_json",
    "bgi_evolution_archive_json",
    "bgi_compact_receipts_json",
    "search_normalize_urls_batch",
    "search_score_frontier_batch",
    "search_fuse_scores_batch",
    "search_cosine_topk",
    "graph_remap_ids_batch",
    "graph_pack_edges_batch",
    "rustyred_thg_expand_bounded",
    "rustyred_thg_paths_shortest",
    "RustyredThgCoreExecutor",
]

ADAPTER_EXPORTS = [
    "LoraAdapter",
    "AdapterRef",
    "find_adapters",
    "upsert_adapter",
    "get_adapter",
    "list_adapters",
    "record_fitness",
    "supersede_adapter",
    "ThgError",
]


def _assert_exact_exports(module, expected: list[str]) -> None:
    actual = {name for name in dir(module) if not name.startswith("_")}
    expected_set = set(expected)
    missing = sorted(expected_set - actual)
    unexpected = sorted(actual - expected_set)
    if missing or unexpected:
        details = []
        if missing:
            details.append(f"missing exports: {', '.join(missing)}")
        if unexpected:
            details.append(f"unexpected exports: {', '.join(unexpected)}")
        pytest.fail("; ".join(details))


def test_top_level_exports_are_present() -> None:
    _assert_exact_exports(theseus_native, TOP_LEVEL_EXPORTS)


def test_adapters_submodule_exports_are_present() -> None:
    try:
        adapters = importlib.import_module("theseus_native.adapters")
    except ModuleNotFoundError as exc:
        if exc.name != "theseus_native.adapters":
            raise
        pytest.fail("theseus_native.adapters is not importable")
    _assert_exact_exports(adapters, ADAPTER_EXPORTS)
