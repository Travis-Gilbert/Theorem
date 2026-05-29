"""Tests for the runnable Gate 0 harness."""

from __future__ import annotations

import pytest

from apps.notebook.inference_engines.benchmark.gate0 import (
    Gate0Check,
    Gate0DatalogCase,
    Gate0ExpectedValueCase,
    Gate0SourceReliabilityCase,
    run_gate0,
    run_gate0_cases,
)
from apps.notebook.inference_engines.benchmark.ledger import BenchmarkLedger
from apps.notebook.inference_engines.benchmark.report import summarize_ledger
from apps.notebook.inference_engines.benchmark.differential import run_datalog_differential
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records


CLAIM = {'id': 10, 'source_object_id': 1, 'text': 'A claim', 'status': 'proposed'}
RULES = ('unsupported_claim',)


def test_gate0_harness_writes_records_and_reports_pass(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'gate0.jsonl')
    datalog_pack = build_fact_pack_from_records(claims=[CLAIM])
    evidence = (
        {'id': 'e1', 'status': 'accepted'},
        {'id': 'e2', 'status': 'refuted'},
    )
    validators = (
        {'id': 'cheap', 'status': 'passed', 'cost': 1.0},
        {'id': 'slow', 'status': 'failed', 'cost': 5.0},
    )

    result = run_gate0_cases(
        ledger=ledger,
        datalog_cases=(
            Gate0DatalogCase(
                query_id='gate0-datalog',
                django_fact_pack=datalog_pack,
                substrate_input={'nodes': [{'node_type': 'claim', **CLAIM}]},
                rule_ids=RULES,
                input_refs=('claim:10',),
            ),
        ),
        source_reliability_cases=(
            Gate0SourceReliabilityCase(
                query_id='gate0-source-reliability',
                evidence_records=evidence,
                substrate_input={'evidence_records': evidence},
                source_id='source-a',
                input_refs=('e1', 'e2'),
            ),
        ),
        expected_value_cases=(
            Gate0ExpectedValueCase(
                query_id='gate0-expected-value',
                validator_records=validators,
                substrate_input={'validator_records': validators},
                decision_value=8.0,
                input_refs=('cheap', 'slow'),
            ),
        ),
    )

    assert result.passed
    assert result.report.gate0_pass_rate == 1.0
    assert result.report.gate0_failures == ()
    assert [record.operation_type for record in result.records] == [
        'datalog.derive',
        'probabilistic.source_reliability',
        'probabilistic.expected_value_of_information',
    ]
    assert len(ledger.records()) == 3


def test_gate0_harness_reports_failure_and_keeps_recording(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'gate0-fail.jsonl')
    datalog_pack = build_fact_pack_from_records(claims=[CLAIM])
    evidence = ({'id': 'e1', 'status': 'accepted'},)

    result = run_gate0_cases(
        ledger=ledger,
        datalog_cases=(
            Gate0DatalogCase(
                query_id='gate0-datalog-bad',
                django_fact_pack=datalog_pack,
                substrate_input={
                    'nodes': [{
                        'node_type': 'claim',
                        'id': 10,
                        'source_object_id': 1,
                        'text': 'A DIFFERENT claim',
                        'status': 'proposed',
                    }],
                },
                rule_ids=RULES,
            ),
        ),
        source_reliability_cases=(
            Gate0SourceReliabilityCase(
                query_id='gate0-source-ok',
                evidence_records=evidence,
                substrate_input={'evidence_records': evidence},
                source_id='source-a',
            ),
        ),
    )

    assert not result.passed
    assert result.report.gate0_pass_rate == 0.5
    assert len(result.report.gate0_failures) == 1
    assert [record.correctness_label for record in ledger.records()] == ['incorrect', 'correct']


def test_gate0_harness_rejects_empty_runs(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'empty.jsonl')

    with pytest.raises(ValueError, match='at least one parity case'):
        run_gate0_cases(ledger=ledger)


def test_gate0_accepts_precomputed_differential_checks(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'precomputed.jsonl')
    datalog_pack = build_fact_pack_from_records(claims=[CLAIM])
    result = run_datalog_differential(
        django_fact_pack=datalog_pack,
        substrate_input={'nodes': [{'node_type': 'claim', **CLAIM}]},
        rule_ids=RULES,
    )

    gate_report = run_gate0(
        (Gate0Check(query_id='precomputed-datalog', result=result, input_refs=('claim:10',)),),
        ledger=ledger,
    )

    assert gate_report.gate_passed
    assert gate_report.total == 1
    assert gate_report.failures == ()
    assert summarize_ledger(ledger).gate0_pass_rate == 1.0
