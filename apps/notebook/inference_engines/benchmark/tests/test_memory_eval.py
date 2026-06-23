"""Tests for memory-specific benchmark ledger adapters."""

from __future__ import annotations

from apps.notebook.inference_engines.benchmark.memory_eval import (
    MemoryEvalCase,
    MemoryObservation,
    record_memory_eval,
    token_reduction_vs_baseline,
)


def test_record_memory_eval_emits_baseline_and_harness_rows():
    case = MemoryEvalCase(
        query_id='mem-001',
        ability='knowledge_update',
        observations={
            'long-context': MemoryObservation(
                system_id='long-context',
                correctness_label='incorrect',
                input_tokens=10_000,
                executor_cost=1.5,
            ),
            'theorems-harness': MemoryObservation(
                system_id='theorems-harness',
                correctness_label='correct',
                input_tokens=500,
                answer_grounded=True,
                receipt_hash='sha256:abc',
            ),
        },
        input_refs=('session:a', 'session:b'),
    )

    records = record_memory_eval([case])

    assert [record.routing_mode for record in records] == ['A', 'B1']
    assert records[0].operation_type == 'memory.knowledge_update'
    assert records[1].chosen_executor == 'theorems-harness'
    assert records[1].correctness_label == 'correct'
    assert records[1].synthesis_used_result is True
    assert records[1].input_refs == ('session:a', 'session:b')


def test_token_reduction_vs_baseline_uses_decision_tokens():
    case = MemoryEvalCase(
        query_id='mem-002',
        ability='temporal_reasoning',
        observations={
            'long-context': MemoryObservation(
                system_id='long-context',
                correctness_label='correct',
                input_tokens=1000,
            ),
            'theorems-harness': MemoryObservation(
                system_id='theorems-harness',
                correctness_label='correct',
                input_tokens=100,
            ),
        },
    )

    records = record_memory_eval([case])

    assert token_reduction_vs_baseline(records)['B1'] == 0.9
