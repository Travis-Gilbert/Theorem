"""Executable CO-1 routing-arm harness.

This module does not collect real LLM or substrate timings. It takes measured
executor observations and turns them into comparable A/B0/B1/B2/B3 ledger rows,
so the benchmark can separate "recording the experiment correctly" from
"collecting expensive observations correctly."
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Iterable, Mapping, Sequence

from .ledger import BenchmarkLedger
from .records import CORRECTNESS_LABELS, ROUTING_MODES, BenchmarkRecord


@dataclass(frozen=True, slots=True)
class ExecutorObservation:
    """Measured outcome for one executor candidate on one operation."""

    executor_id: str
    receipt_hash: str = ''
    correctness_label: str = 'unknown'
    synthesis_used_result: bool | None = None
    decision_tokens: int = 0
    executor_cost: float = 0.0
    gpu_seconds: float = 0.0
    cpu_seconds: float = 0.0
    wall_ms: float = 0.0
    retry_count: int = 0
    missed_affordance: bool = False
    quality_score: float | None = None
    cache_hit: bool = False
    stale_receipt: bool = False
    cascade_escalated: bool = False

    def __post_init__(self) -> None:
        if not str(self.executor_id or '').strip():
            raise ValueError('ExecutorObservation requires executor_id')
        if self.correctness_label not in CORRECTNESS_LABELS:
            raise ValueError(
                f'correctness_label must be one of {CORRECTNESS_LABELS}, got {self.correctness_label!r}'
            )
        if self.quality_score is not None and not (0.0 <= float(self.quality_score) <= 1.0):
            raise ValueError('quality_score must be in [0, 1] or None')


@dataclass(frozen=True, slots=True)
class BenchmarkCase:
    """One operation with candidate observations for the CO-1 routing arms."""

    query_id: str
    operation_type: str
    observations: Mapping[str, ExecutorObservation]
    input_refs: tuple[str, ...] = ()
    baseline_executor: str = 'llm'
    deterministic_executor: str = 'substrate'
    tool_call_executor: str = 'llm-tool-call'
    pairformer_executor: str = 'pairformer-router'
    oracle_executor: str = ''

    def __post_init__(self) -> None:
        if not str(self.query_id or '').strip():
            raise ValueError('BenchmarkCase requires query_id')
        if not str(self.operation_type or '').strip():
            raise ValueError('BenchmarkCase requires operation_type')
        observations = dict(self.observations)
        if not observations:
            raise ValueError('BenchmarkCase requires at least one executor observation')
        object.__setattr__(self, 'observations', observations)
        object.__setattr__(self, 'input_refs', tuple(str(ref) for ref in self.input_refs))

    @property
    def candidate_executors(self) -> tuple[str, ...]:
        return tuple(self.observations)


def select_observation(case: BenchmarkCase, routing_mode: str) -> ExecutorObservation:
    """Select the observation implied by a CO-1 routing arm."""

    if routing_mode not in ROUTING_MODES:
        raise ValueError(f'routing_mode must be one of {ROUTING_MODES}, got {routing_mode!r}')
    if routing_mode == 'B0':
        return _oracle_observation(case)
    executor_id = {
        'A': case.baseline_executor,
        'B1': case.deterministic_executor,
        'B2': case.tool_call_executor,
        'B3': case.pairformer_executor,
    }.get(routing_mode, '')
    if executor_id not in case.observations:
        raise ValueError(f'{routing_mode} requires observation for executor {executor_id!r}')
    return case.observations[executor_id]


def record_for_routing_mode(case: BenchmarkCase, routing_mode: str) -> BenchmarkRecord:
    """Build one ledger row for a case under a routing arm."""

    observation = select_observation(case, routing_mode)
    return BenchmarkRecord(
        query_id=case.query_id,
        operation_type=case.operation_type,
        routing_mode=routing_mode,
        chosen_executor=observation.executor_id,
        candidate_executors=case.candidate_executors,
        input_refs=case.input_refs,
        receipt_hash=observation.receipt_hash,
        correctness_label=observation.correctness_label,
        synthesis_used_result=observation.synthesis_used_result,
        decision_tokens=observation.decision_tokens,
        executor_cost=observation.executor_cost,
        gpu_seconds=observation.gpu_seconds,
        cpu_seconds=observation.cpu_seconds,
        wall_ms=observation.wall_ms,
        retry_count=observation.retry_count,
        missed_affordance=observation.missed_affordance,
        quality_score=observation.quality_score,
        cache_hit=observation.cache_hit,
        stale_receipt=observation.stale_receipt,
        cascade_escalated=observation.cascade_escalated,
    )


def run_arm_records(
    cases: Iterable[BenchmarkCase],
    *,
    routing_modes: Sequence[str] = ROUTING_MODES,
    ledger: BenchmarkLedger | None = None,
) -> list[BenchmarkRecord]:
    """Generate and optionally append all requested arm rows."""

    records: list[BenchmarkRecord] = []
    for case in cases:
        for routing_mode in routing_modes:
            record = record_for_routing_mode(case, routing_mode)
            records.append(record)
            if ledger is not None:
                ledger.record(record)
    return records


def _oracle_observation(case: BenchmarkCase) -> ExecutorObservation:
    if case.oracle_executor:
        if case.oracle_executor not in case.observations:
            raise ValueError(f'B0 oracle_executor {case.oracle_executor!r} has no observation')
        return case.observations[case.oracle_executor]
    correct = [
        observation
        for observation in case.observations.values()
        if observation.correctness_label == 'correct'
    ]
    pool = correct or list(case.observations.values())
    return min(
        pool,
        key=lambda observation: (
            float(observation.executor_cost),
            float(observation.gpu_seconds),
            float(observation.wall_ms),
            observation.executor_id,
        ),
    )
