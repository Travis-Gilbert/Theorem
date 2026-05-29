"""Tests for the CO-1 routing-arm harness."""

from __future__ import annotations

from apps.notebook.inference_engines.benchmark.arms import (
    BenchmarkCase,
    ExecutorObservation,
    record_for_routing_mode,
    run_arm_records,
)
from apps.notebook.inference_engines.benchmark.ledger import BenchmarkLedger


def _case() -> BenchmarkCase:
    return BenchmarkCase(
        query_id='q1',
        operation_type='datalog.derive',
        input_refs=('claim:10',),
        baseline_executor='llm',
        deterministic_executor='substrate',
        tool_call_executor='tool-call',
        pairformer_executor='pairformer',
        observations={
            'llm': ExecutorObservation(
                executor_id='llm',
                receipt_hash='llm-receipt',
                correctness_label='correct',
                executor_cost=0.12,
                gpu_seconds=1.5,
                wall_ms=900,
                decision_tokens=0,
            ),
            'substrate': ExecutorObservation(
                executor_id='substrate',
                receipt_hash='substrate-receipt',
                correctness_label='correct',
                executor_cost=0.01,
                cpu_seconds=0.03,
                wall_ms=40,
            ),
            'tool-call': ExecutorObservation(
                executor_id='tool-call',
                receipt_hash='tool-receipt',
                correctness_label='incorrect',
                executor_cost=0.08,
                gpu_seconds=0.8,
                wall_ms=500,
                decision_tokens=180,
                missed_affordance=True,
            ),
            'pairformer': ExecutorObservation(
                executor_id='pairformer',
                receipt_hash='pairformer-receipt',
                correctness_label='correct',
                executor_cost=0.03,
                gpu_seconds=0.2,
                cpu_seconds=0.02,
                wall_ms=120,
                decision_tokens=24,
            ),
        },
    )


def test_arm_records_distinguish_a_b1_b2_b3_and_append_to_ledger(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'cost-test.jsonl')

    records = run_arm_records([_case()], routing_modes=('A', 'B1', 'B2', 'B3'), ledger=ledger)

    assert [record.routing_mode for record in records] == ['A', 'B1', 'B2', 'B3']
    assert [record.chosen_executor for record in records] == [
        'llm',
        'substrate',
        'tool-call',
        'pairformer',
    ]
    assert records[2].decision_tokens == 180
    assert records[2].missed_affordance
    assert [record.routing_mode for record in ledger.records()] == ['A', 'B1', 'B2', 'B3']


def test_b0_oracle_chooses_cheapest_correct_candidate():
    record = record_for_routing_mode(_case(), 'B0')

    assert record.routing_mode == 'B0'
    assert record.chosen_executor == 'substrate'
    assert record.correctness_label == 'correct'
    assert record.receipt_hash == 'substrate-receipt'


def test_b0_oracle_can_be_explicit_when_needed():
    case = BenchmarkCase(
        query_id='q2',
        operation_type='probabilistic.source_reliability',
        oracle_executor='pairformer',
        observations={
            'substrate': ExecutorObservation(
                executor_id='substrate',
                receipt_hash='substrate-receipt',
                correctness_label='correct',
                executor_cost=0.01,
            ),
            'pairformer': ExecutorObservation(
                executor_id='pairformer',
                receipt_hash='pairformer-receipt',
                correctness_label='correct',
                executor_cost=0.03,
            ),
        },
    )

    record = record_for_routing_mode(case, 'B0')

    assert record.chosen_executor == 'pairformer'
    assert record.receipt_hash == 'pairformer-receipt'
