"""Benchmark ledger record shape for the substrate query-planner cost test.

One row per routed operation. The field set is the locked minimum record from
docs/plans/compute-offload/implementation-plan.md section 4 (identical to the
minimum record in Codex's pairformer-tool-router-training-plan.md), plus
gpu_seconds / cpu_seconds, which section 3 (CO-1) names as per-query measures
with no other home.

Spec backreference:
  - 19 core fields            -> section 4 "minimum record shape"
  - gpu_seconds, cpu_seconds  -> section 3 CO-1 "Measure per query"
  - routing_mode domain       -> section 3 CO-1.A / CO-1.B0..B3

The same row triple-serves: economics validation, router training, and cascade
calibration (section 4: "one run, three payoffs").
"""

from __future__ import annotations

import uuid
from dataclasses import dataclass
from typing import Any

ROUTING_MODES: tuple[str, ...] = ('A', 'B0', 'B1', 'B2', 'B3')
CORRECTNESS_LABELS: tuple[str, ...] = ('correct', 'incorrect', 'unknown')


def _as_tuple(value: Any) -> tuple[str, ...]:
    if value is None:
        return ()
    if isinstance(value, str):
        return (value,)
    return tuple(str(item) for item in value)


@dataclass(frozen=True, slots=True)
class BenchmarkRecord:
    """One routed operation in the cost-test ledger."""

    query_id: str
    operation_type: str
    routing_mode: str
    chosen_executor: str
    operation_id: str = ''
    candidate_executors: tuple[str, ...] = ()
    input_refs: tuple[str, ...] = ()
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
        if not str(self.query_id or '').strip():
            raise ValueError('BenchmarkRecord requires query_id')
        if not str(self.operation_type or '').strip():
            raise ValueError('BenchmarkRecord requires operation_type')
        if self.routing_mode not in ROUTING_MODES:
            raise ValueError(f'routing_mode must be one of {ROUTING_MODES}, got {self.routing_mode!r}')
        if not str(self.chosen_executor or '').strip():
            raise ValueError('BenchmarkRecord requires chosen_executor')
        if self.correctness_label not in CORRECTNESS_LABELS:
            raise ValueError(f'correctness_label must be one of {CORRECTNESS_LABELS}, got {self.correctness_label!r}')
        if self.quality_score is not None and not (0.0 <= float(self.quality_score) <= 1.0):
            raise ValueError('quality_score must be in [0, 1] or None')
        object.__setattr__(self, 'candidate_executors', _as_tuple(self.candidate_executors))
        object.__setattr__(self, 'input_refs', _as_tuple(self.input_refs))
        if not self.operation_id:
            object.__setattr__(self, 'operation_id', uuid.uuid4().hex)

    def to_dict(self) -> dict[str, Any]:
        return {
            'operation_id': self.operation_id,
            'query_id': self.query_id,
            'operation_type': self.operation_type,
            'candidate_executors': list(self.candidate_executors),
            'chosen_executor': self.chosen_executor,
            'routing_mode': self.routing_mode,
            'input_refs': list(self.input_refs),
            'receipt_hash': self.receipt_hash,
            'correctness_label': self.correctness_label,
            'synthesis_used_result': self.synthesis_used_result,
            'decision_tokens': self.decision_tokens,
            'executor_cost': self.executor_cost,
            'gpu_seconds': self.gpu_seconds,
            'cpu_seconds': self.cpu_seconds,
            'wall_ms': self.wall_ms,
            'retry_count': self.retry_count,
            'missed_affordance': self.missed_affordance,
            'quality_score': self.quality_score,
            'cache_hit': self.cache_hit,
            'stale_receipt': self.stale_receipt,
            'cascade_escalated': self.cascade_escalated,
        }

    @classmethod
    def from_dict(cls, data: dict[str, Any]) -> 'BenchmarkRecord':
        quality = data.get('quality_score')
        return cls(
            query_id=str(data.get('query_id', '')),
            operation_type=str(data.get('operation_type', '')),
            routing_mode=str(data.get('routing_mode', '')),
            chosen_executor=str(data.get('chosen_executor', '')),
            operation_id=str(data.get('operation_id', '')),
            candidate_executors=_as_tuple(data.get('candidate_executors')),
            input_refs=_as_tuple(data.get('input_refs')),
            receipt_hash=str(data.get('receipt_hash', '')),
            correctness_label=str(data.get('correctness_label', 'unknown')),
            synthesis_used_result=data.get('synthesis_used_result'),
            decision_tokens=int(data.get('decision_tokens', 0) or 0),
            executor_cost=float(data.get('executor_cost', 0.0) or 0.0),
            gpu_seconds=float(data.get('gpu_seconds', 0.0) or 0.0),
            cpu_seconds=float(data.get('cpu_seconds', 0.0) or 0.0),
            wall_ms=float(data.get('wall_ms', 0.0) or 0.0),
            retry_count=int(data.get('retry_count', 0) or 0),
            missed_affordance=bool(data.get('missed_affordance', False)),
            quality_score=(None if quality is None else float(quality)),
            cache_hit=bool(data.get('cache_hit', False)),
            stale_receipt=bool(data.get('stale_receipt', False)),
            cascade_escalated=bool(data.get('cascade_escalated', False)),
        )
