"""Tests for the CO-1 benchmark ledger."""

from __future__ import annotations

import json

import pytest

from apps.notebook.inference_engines.benchmark import (
    PRE_REGISTRATION,
    BenchmarkLedger,
    BenchmarkRecord,
    receipt_hash_for,
)
from apps.notebook.inference_engines.datalog.contracts import DatalogReceipt


def _record(**overrides) -> BenchmarkRecord:
    base = dict(
        query_id='q1',
        operation_type='datalog.derive',
        routing_mode='B1',
        chosen_executor='datalog-cpu',
    )
    base.update(overrides)
    return BenchmarkRecord(**base)


def test_header_written_once_and_first(tmp_path):
    path = tmp_path / 'run.jsonl'
    ledger = BenchmarkLedger(path)
    ledger.record(_record())
    BenchmarkLedger(path)  # re-open must not duplicate the header

    lines = [json.loads(line) for line in path.read_text().splitlines() if line.strip()]
    headers = [line for line in lines if line['record_type'] == 'pre_registration']
    assert len(headers) == 1
    assert lines[0]['record_type'] == 'pre_registration'
    assert lines[0]['reprice_threshold_pct'] == 30


def test_record_round_trip(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'run.jsonl')
    ledger.record(_record(
        candidate_executors=['datalog-cpu', 'llm'],
        input_refs=['node:1', 'node:2'],
        cascade_escalated=True,
        quality_score=0.91,
        synthesis_used_result=True,
        gpu_seconds=0.0,
        cpu_seconds=0.012,
    ))

    [loaded] = ledger.records()
    assert loaded.query_id == 'q1'
    assert loaded.candidate_executors == ('datalog-cpu', 'llm')
    assert loaded.input_refs == ('node:1', 'node:2')
    assert loaded.cascade_escalated is True
    assert loaded.quality_score == 0.91
    assert loaded.synthesis_used_result is True
    assert loaded.cpu_seconds == 0.012


def test_pre_registration_readable(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'run.jsonl')
    pre = ledger.pre_registration()
    assert pre['converged_band_pct'] == [35, 60]
    assert pre['axes']['combined_vs_baseline_a']['claude_pct'] == [40, 60]
    assert pre['axes']['combined_vs_baseline_a']['codex_pct'] == [35, 55]
    assert pre['cascade_literature_value_rejected_pct'] == 95


def test_invalid_routing_mode_rejected():
    with pytest.raises(ValueError):
        _record(routing_mode='Z')


def test_invalid_correctness_label_rejected():
    with pytest.raises(ValueError):
        _record(correctness_label='maybe')


def test_quality_score_out_of_range_rejected():
    with pytest.raises(ValueError):
        _record(quality_score=1.5)


def test_operation_id_autofilled():
    assert _record().operation_id


def test_operation_id_preserved_on_round_trip(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'run.jsonl')
    rec = _record()
    ledger.record(rec)
    [loaded] = ledger.records()
    assert loaded.operation_id == rec.operation_id


def test_receipt_hash_for_binds_real_receipt():
    receipt = DatalogReceipt(
        engine='python-reference-datalog',
        fact_pack_hash='abc',
        rule_ids=('r1',),
        derived_facts=(),
    )
    first = receipt_hash_for(receipt)
    assert first and first == receipt_hash_for(receipt)
    assert receipt_hash_for(receipt.to_dict()) == first


def test_receipt_hash_for_none_is_empty():
    assert receipt_hash_for(None) == ''


def test_all_19_spec_fields_present():
    spec_fields = {
        'operation_id', 'query_id', 'operation_type', 'candidate_executors',
        'chosen_executor', 'routing_mode', 'input_refs', 'receipt_hash',
        'correctness_label', 'synthesis_used_result', 'decision_tokens',
        'executor_cost', 'wall_ms', 'retry_count', 'missed_affordance',
        'quality_score', 'cache_hit', 'stale_receipt', 'cascade_escalated',
    }
    payload = _record().to_dict()
    assert spec_fields <= set(payload)
    assert {'gpu_seconds', 'cpu_seconds'} <= set(payload)
