"""Tests for the CO-1 ledger reporting aggregator."""

from __future__ import annotations

from apps.notebook.inference_engines.benchmark import BenchmarkLedger, BenchmarkRecord
from apps.notebook.inference_engines.benchmark.report import (
    summarize_ledger,
    summarize_records,
)


def _op(routing_mode: str, **overrides) -> BenchmarkRecord:
    base = dict(
        query_id='q1',
        operation_type='synthesis',
        routing_mode=routing_mode,
        chosen_executor='llm',
    )
    base.update(overrides)
    return BenchmarkRecord(**base)


def test_per_arm_aggregates():
    records = [
        _op('A', executor_cost=1.0, gpu_seconds=2.0, wall_ms=100.0),
        _op('A', executor_cost=1.0, gpu_seconds=2.0, wall_ms=300.0),
        _op('B1', executor_cost=0.4, cpu_seconds=0.01, wall_ms=50.0, missed_affordance=True),
    ]
    report = summarize_records(records)

    assert report.arms['A'].operation_count == 2
    assert report.arms['A'].total_executor_cost == 2.0
    assert report.arms['A'].total_gpu_seconds == 4.0
    assert report.arms['A'].mean_wall_ms == 200.0
    assert report.arms['B1'].routing_miss_rate == 1.0


def test_cost_reduction_vs_baseline():
    records = [
        _op('A', executor_cost=1.0),
        _op('B1', executor_cost=0.5),
        _op('B0', executor_cost=0.2),
    ]
    report = summarize_records(records)

    assert report.cost_reduction_vs_baseline['B1'] == 0.5
    assert report.cost_reduction_vs_baseline['B0'] == 0.8
    assert 'A' not in report.cost_reduction_vs_baseline


def test_band_check_uses_pre_registration():
    records = [
        _op('A', executor_cost=1.0),
        _op('B1', executor_cost=0.5),   # 50% reduction -> within [35,60]
        _op('B2', executor_cost=0.9),   # 10% reduction -> below reprice 30
    ]
    pre = {'converged_band_pct': [35, 60], 'reprice_threshold_pct': 30}
    report = summarize_records(records, pre_registration=pre)

    assert report.band_check['B1']['within_band'] is True
    assert report.band_check['B1']['below_reprice'] is False
    assert report.band_check['B2']['within_band'] is False
    assert report.band_check['B2']['below_reprice'] is True


def test_gate0_pass_rate_and_failures():
    records = [
        _op('B1', operation_type='datalog.derive', correctness_label='correct', operation_id='op-ok'),
        _op('B1', operation_type='datalog.derive', correctness_label='incorrect', operation_id='op-bad'),
        _op('A', correctness_label='unknown'),
    ]
    report = summarize_records(records)

    assert report.gate0_pass_rate == 0.5
    assert report.gate0_failures == ('op-bad',)


def test_summarize_ledger_reads_header_band(tmp_path):
    ledger = BenchmarkLedger(tmp_path / 'run.jsonl')
    ledger.record(_op('A', executor_cost=1.0))
    ledger.record(_op('B1', executor_cost=0.5))

    report = summarize_ledger(ledger)
    assert report.pre_registration['converged_band_pct'] == [35, 60]
    assert report.band_check['B1']['within_band'] is True
