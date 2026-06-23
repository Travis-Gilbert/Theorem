"""CO-1 ledger reporting: aggregate a benchmark ledger into per-arm summaries.

Reads an append-only benchmark ledger and computes the numbers the cost test
reports (spec sections 3 CO-1 and 5): per-routing-arm cost, resource,
correctness, and routing-quality aggregates; cost reduction vs the baseline arm
A; a Gate 0 correctness pass-rate; and a check against the locked
pre-registration band.

Pure aggregation over committed BenchmarkRecord rows. No DB, no external deps.
Cost reduction assumes the arms cover the same query set (true by construction in
a Gate 1 A/B run); it compares total executor_cost per arm.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Sequence

from .ledger import BenchmarkLedger
from .records import BenchmarkRecord


def _rate(count: int, total: int) -> float:
    return count / total if total else 0.0


@dataclass(frozen=True, slots=True)
class ArmSummary:
    """Aggregates for one routing arm (A / B0 / B1 / B2 / B3)."""

    routing_mode: str
    operation_count: int
    total_executor_cost: float
    total_gpu_seconds: float
    total_cpu_seconds: float
    total_decision_tokens: int
    mean_wall_ms: float
    correctness_pass_rate: float | None
    routing_miss_rate: float
    cache_hit_rate: float
    stale_receipt_rate: float
    cascade_escalation_rate: float

    def to_dict(self) -> dict[str, Any]:
        return {
            'routing_mode': self.routing_mode,
            'operation_count': self.operation_count,
            'total_executor_cost': self.total_executor_cost,
            'total_gpu_seconds': self.total_gpu_seconds,
            'total_cpu_seconds': self.total_cpu_seconds,
            'total_decision_tokens': self.total_decision_tokens,
            'mean_wall_ms': self.mean_wall_ms,
            'correctness_pass_rate': self.correctness_pass_rate,
            'routing_miss_rate': self.routing_miss_rate,
            'cache_hit_rate': self.cache_hit_rate,
            'stale_receipt_rate': self.stale_receipt_rate,
            'cascade_escalation_rate': self.cascade_escalation_rate,
        }


@dataclass(frozen=True, slots=True)
class LedgerReport:
    """Whole-ledger rollup: per-arm summaries plus the headline comparisons."""

    arms: dict[str, ArmSummary]
    cost_reduction_vs_baseline: dict[str, float]
    band_check: dict[str, dict[str, Any]]
    gate0_pass_rate: float | None
    gate0_failures: tuple[str, ...]
    pre_registration: dict[str, Any]

    def to_dict(self) -> dict[str, Any]:
        return {
            'arms': {mode: arm.to_dict() for mode, arm in self.arms.items()},
            'cost_reduction_vs_baseline': dict(self.cost_reduction_vs_baseline),
            'band_check': {mode: dict(payload) for mode, payload in self.band_check.items()},
            'gate0_pass_rate': self.gate0_pass_rate,
            'gate0_failures': list(self.gate0_failures),
            'pre_registration': dict(self.pre_registration),
        }


def _summarize_arm(routing_mode: str, records: Sequence[BenchmarkRecord]) -> ArmSummary:
    count = len(records)
    labeled = [r for r in records if r.correctness_label in ('correct', 'incorrect')]
    correct = sum(1 for r in labeled if r.correctness_label == 'correct')
    return ArmSummary(
        routing_mode=routing_mode,
        operation_count=count,
        total_executor_cost=sum(r.executor_cost for r in records),
        total_gpu_seconds=sum(r.gpu_seconds for r in records),
        total_cpu_seconds=sum(r.cpu_seconds for r in records),
        total_decision_tokens=sum(r.decision_tokens for r in records),
        mean_wall_ms=(sum(r.wall_ms for r in records) / count if count else 0.0),
        correctness_pass_rate=(_rate(correct, len(labeled)) if labeled else None),
        routing_miss_rate=_rate(sum(1 for r in records if r.missed_affordance), count),
        cache_hit_rate=_rate(sum(1 for r in records if r.cache_hit), count),
        stale_receipt_rate=_rate(sum(1 for r in records if r.stale_receipt), count),
        cascade_escalation_rate=_rate(sum(1 for r in records if r.cascade_escalated), count),
    )


def summarize_records(
    records: Sequence[BenchmarkRecord],
    *,
    pre_registration: dict[str, Any] | None = None,
) -> LedgerReport:
    pre = dict(pre_registration or {})
    by_arm: dict[str, list[BenchmarkRecord]] = {}
    for record in records:
        by_arm.setdefault(record.routing_mode, []).append(record)
    arms = {mode: _summarize_arm(mode, rows) for mode, rows in sorted(by_arm.items())}

    baseline = arms.get('A')
    reductions: dict[str, float] = {}
    if baseline and baseline.total_executor_cost > 0:
        base_cost = baseline.total_executor_cost
        for mode, arm in arms.items():
            if mode == 'A':
                continue
            reductions[mode] = (base_cost - arm.total_executor_cost) / base_cost

    band = pre.get('converged_band_pct') or []
    reprice = pre.get('reprice_threshold_pct')
    band_check: dict[str, dict[str, Any]] = {}
    for mode, reduction in reductions.items():
        reduction_pct = reduction * 100.0
        entry: dict[str, Any] = {'reduction_pct': reduction_pct}
        if len(band) == 2:
            entry['within_band'] = band[0] <= reduction_pct <= band[1]
        if reprice is not None:
            entry['below_reprice'] = reduction_pct < reprice
        band_check[mode] = entry

    labeled = [r for r in records if r.correctness_label in ('correct', 'incorrect')]
    gate0_pass_rate = (
        _rate(sum(1 for r in labeled if r.correctness_label == 'correct'), len(labeled))
        if labeled else None
    )
    gate0_failures = tuple(
        r.operation_id for r in records if r.correctness_label == 'incorrect'
    )

    return LedgerReport(
        arms=arms,
        cost_reduction_vs_baseline=reductions,
        band_check=band_check,
        gate0_pass_rate=gate0_pass_rate,
        gate0_failures=gate0_failures,
        pre_registration=pre,
    )


def summarize_ledger(ledger: BenchmarkLedger) -> LedgerReport:
    return summarize_records(
        ledger.records(),
        pre_registration=ledger.pre_registration(),
    )
