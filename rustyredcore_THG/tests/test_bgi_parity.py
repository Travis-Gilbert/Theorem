from __future__ import annotations

import pytest

theseus_native = pytest.importorskip("theseus_native")

pytestmark = pytest.mark.skipif(
    not all(
        hasattr(theseus_native, name)
        for name in (
            "bgi_compact_receipts_json",
            "bgi_datalog_verified_rule_ids_json",
            "bgi_datalog_derive_core_json",
            "bgi_probabilistic_source_reliability_json",
            "bgi_probabilistic_expected_value_json",
            "bgi_evolution_archive_json",
        )
    ),
    reason="installed theseus_native wheel does not include BGI symbolic parity exports",
)


def test_bgi_native_parity_benchmarks_match_python_reference() -> None:
    benchmark = pytest.importorskip(
        "apps.notebook.benchmarks.bgi_native_parity",
        reason="BGI Python benchmark reference fixtures are not present in this checkout",
    )

    report = benchmark.run_all_parity_benchmarks(
        iterations=2,
        native_module=theseus_native,
    )

    assert report["native_available"] is True
    assert report["all_parity_passed"] is True
    assert {item["name"] for item in report["benchmarks"]} == {
        "stable_hash_golden_vectors",
        "egraph_receipt_summary",
        "datalog_receipt_summary",
        "datalog_derive_core",
        "fact_pack_hash",
        "receipt_compaction",
        "probabilistic_source_reliability",
        "probabilistic_expected_value",
        "evolution_archive",
    }


def test_bgi_native_symbolic_exports_are_executable() -> None:
    import json

    egraph = json.loads(theseus_native.bgi_egraph_extract_context_pack_json(json.dumps({
        "expression_id": "native-test",
        "items": [
            {"id": "a", "text": "keep", "tokens": 2, "obligation_id": "o1"},
            {"id": "b", "text": "keep", "tokens": 2, "obligation_id": "o1"},
            {"id": "empty", "text": "", "tokens": 1},
        ],
    })))
    assert egraph["native_backend"] == "rust-egg-context-pack"
    assert egraph["equivalent"] is True
    assert len(egraph["extraction"]["items"]) == 1

    datalog = json.loads(theseus_native.bgi_datalog_derive_core_json(json.dumps([
        {"relation": "claim", "entity_id": "claim-1", "attributes": {"status": "proposed"}, "fact_id": "f1"},
        {"relation": "object", "entity_id": "obj-1", "attributes": {"title": "Same"}, "fact_id": "f2"},
        {"relation": "object", "entity_id": "obj-2", "attributes": {"title": "same"}, "fact_id": "f3"},
    ])))
    assert datalog["engine"] == "python-reference-datalog"
    assert datalog["derived_count"] == 3
    assert {fact["relation"] for fact in datalog["derived_facts"]} == {
        "unsupported_claim",
        "likely_duplicate_entity",
        "claim_has_no_independent_support",
    }

    source_reliability = json.loads(theseus_native.bgi_probabilistic_source_reliability_json(json.dumps({
        "source_id": "source-a",
        "prior_alpha": 2.0,
        "prior_beta": 2.0,
        "corroborated": 6,
        "contradicted": 2,
    })))
    assert source_reliability["posterior"]["alpha"] == 8.0
    assert source_reliability["posterior"]["beta"] == 4.0

    expected_value = json.loads(theseus_native.bgi_probabilistic_expected_value_json(json.dumps({
        "current_uncertainty": 0.6,
        "expected_uncertainty_after": 0.2,
        "decision_value": 10.0,
        "validator_cost": 1.0,
    })))
    assert expected_value["posterior"]["expected_value"] == pytest.approx(3.0)

    archive = json.loads(theseus_native.bgi_evolution_archive_json(json.dumps({
        "candidates": [
            {"candidate_id": "a", "niche": "n1", "score": 0.5, "novelty": 0.1, "payload": {"k": 1}},
            {"candidate_id": "b", "niche": "n1", "score": 0.6, "novelty": 0.1, "payload": {"k": 2}},
            {"candidate_id": "c", "niche": "n2", "score": 0.4, "novelty": 0.8, "payload": {}},
        ],
        "elites_per_niche": 1,
    })))
    assert archive["engine"] == "quality-diversity-python-fallback"
    assert archive["elites_by_niche"]["n1"][0]["candidate_id"] == "b"
    assert archive["rejected_count"] == 1
