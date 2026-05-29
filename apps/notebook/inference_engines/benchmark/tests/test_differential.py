"""Tests for the CO-0.3 differential-receipt runner."""

from __future__ import annotations

from apps.notebook.inference_engines.affordances import AffordanceReceipt
from apps.notebook.inference_engines.benchmark.differential import (
    compare_datalog_receipts,
    run_datalog_differential,
    run_probabilistic_expected_value_differential,
    run_probabilistic_source_reliability_differential,
)
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records

CLAIM = {'id': 10, 'source_object_id': 1, 'text': 'A claim', 'status': 'proposed'}
RULES = ['unsupported_claim']


def test_matched_when_same_entities_via_django_and_substrate_paths():
    django_pack = build_fact_pack_from_records(claims=[CLAIM])
    substrate_input = {'nodes': [{'node_type': 'claim', **CLAIM}]}

    result = run_datalog_differential(
        django_fact_pack=django_pack,
        substrate_input=substrate_input,
        rule_ids=RULES,
    )

    assert result.matched
    assert result.mismatches == ()
    assert result.fact_pack_hash_django == result.fact_pack_hash_substrate
    assert result.derived_count_django == result.derived_count_substrate


def test_mismatch_detected_when_substrate_entities_differ():
    django_pack = build_fact_pack_from_records(claims=[CLAIM])
    substrate_input = {
        'nodes': [{
            'node_type': 'claim',
            'id': 10,
            'source_object_id': 1,
            'text': 'A DIFFERENT claim',
            'status': 'proposed',
        }],
    }

    result = run_datalog_differential(
        django_fact_pack=django_pack,
        substrate_input=substrate_input,
        rule_ids=RULES,
    )

    assert not result.matched
    assert any('fact_pack_hash differs' in m for m in result.mismatches)


def test_to_record_labels_correctness_and_carries_receipt_hash():
    django_pack = build_fact_pack_from_records(claims=[CLAIM])
    substrate_input = {'nodes': [{'node_type': 'claim', **CLAIM}]}
    result = run_datalog_differential(
        django_fact_pack=django_pack,
        substrate_input=substrate_input,
        rule_ids=RULES,
    )

    record = result.to_record(query_id='gate0-q1', input_refs=['claim:10'])
    assert record.correctness_label == 'correct'
    assert record.operation_type == 'datalog.derive'
    assert record.routing_mode == 'B1'
    assert record.receipt_hash == result.substrate_receipt_hash


def test_comparator_matches_consistent_pair():
    pack = build_fact_pack_from_records(claims=[CLAIM])
    receipt = DatalogEngine().derive(pack, rule_ids=RULES)
    affordance = AffordanceReceipt(
        engine_id=receipt.engine,
        affordance_id='datalog.derive',
        input_hash=pack.pack_hash,
        payload=receipt.to_dict(),
    )

    result = compare_datalog_receipts(receipt, affordance)
    assert result.matched
    assert result.substrate_receipt_hash == affordance.receipt_hash


def test_comparator_flags_fact_pack_hash_mismatch():
    pack = build_fact_pack_from_records(claims=[CLAIM])
    receipt = DatalogEngine().derive(pack, rule_ids=RULES)
    affordance = AffordanceReceipt(
        engine_id=receipt.engine,
        affordance_id='datalog.derive',
        input_hash='tampered-hash',
        payload=receipt.to_dict(),
    )

    result = compare_datalog_receipts(receipt, affordance)
    assert not result.matched
    assert any('fact_pack_hash differs' in m for m in result.mismatches)


def test_probabilistic_source_reliability_matches_runtime_and_substrate_paths():
    evidence = [
        {'id': 'e1', 'status': 'accepted'},
        {'id': 'e2', 'status': 'refuted'},
        {'id': 'e3', 'status': 'corroborated'},
    ]

    result = run_probabilistic_source_reliability_differential(
        evidence_records=evidence,
        substrate_input={'evidence_records': evidence},
        source_id='source-a',
        prior_alpha=2.0,
        prior_beta=1.0,
    )

    assert result.matched
    assert result.mismatches == ()
    assert result.model_id_django == result.model_id_substrate
    assert result.django_payload_hash == result.substrate_payload_hash


def test_probabilistic_source_reliability_mismatch_when_substrate_records_differ():
    django_evidence = [{'id': 'e1', 'status': 'accepted'}]
    substrate_evidence = [{'id': 'e1', 'status': 'refuted'}]

    result = run_probabilistic_source_reliability_differential(
        evidence_records=django_evidence,
        substrate_input={'evidence_records': substrate_evidence},
        source_id='source-a',
    )

    assert not result.matched
    assert any('observations differs' in m or 'posterior differs' in m for m in result.mismatches)


def test_probabilistic_expected_value_matches_runtime_and_substrate_paths():
    validators = [
        {'id': 'cheap', 'status': 'passed', 'cost': 1.0},
        {'id': 'slow', 'status': 'failed', 'cost': 5.0},
    ]

    result = run_probabilistic_expected_value_differential(
        validator_records=validators,
        substrate_input={'validator_records': validators},
        decision_value=8.0,
    )

    assert result.matched
    assert result.django_payload_hash == result.substrate_payload_hash
    record = result.to_record(query_id='gate0-probabilistic-q1', input_refs=['cheap', 'slow'])
    assert record.correctness_label == 'correct'
    assert record.operation_type == 'probabilistic.expected_value_of_information'
    assert record.chosen_executor == 'probabilistic-cpu'
