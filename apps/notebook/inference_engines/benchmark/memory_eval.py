"""Memory-specific benchmark helpers.

This is the LongMemEval-style layer over the existing CO-1 ledger. It does not
run a model or a memory system directly; callers pass measured observations and
the adapter records comparable ledger rows for memory abilities such as temporal
reasoning, knowledge updates, abstention, and citation grounding.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Iterable, Mapping, Sequence

from .ledger import BenchmarkLedger
from .records import BenchmarkRecord, CORRECTNESS_LABELS

MEMORY_ABILITIES: tuple[str, ...] = (
    'temporal_reasoning',
    'knowledge_update',
    'abstention',
    'citation_grounding',
    'multi_session_recall',
)


@dataclass(frozen=True, slots=True)
class MemoryObservation:
    """Measured outcome for one memory system on one memory benchmark case."""

    system_id: str
    correctness_label: str
    input_tokens: int
    output_tokens: int = 0
    executor_cost: float = 0.0
    wall_ms: float = 0.0
    answer_grounded: bool | None = None
    abstained: bool = False
    receipt_hash: str = ''

    def __post_init__(self) -> None:
        if not str(self.system_id or '').strip():
            raise ValueError('MemoryObservation requires system_id')
        if self.correctness_label not in CORRECTNESS_LABELS:
            raise ValueError(
                f'correctness_label must be one of {CORRECTNESS_LABELS}, got {self.correctness_label!r}'
            )
        if self.input_tokens < 0 or self.output_tokens < 0:
            raise ValueError('token counts must be non-negative')


@dataclass(frozen=True, slots=True)
class MemoryEvalCase:
    """One memory benchmark query with observations for each compared system."""

    query_id: str
    ability: str
    observations: Mapping[str, MemoryObservation]
    input_refs: tuple[str, ...] = ()
    baseline_system: str = 'long-context'
    harness_system: str = 'theorems-harness'

    def __post_init__(self) -> None:
        if not str(self.query_id or '').strip():
            raise ValueError('MemoryEvalCase requires query_id')
        if self.ability not in MEMORY_ABILITIES:
            raise ValueError(f'ability must be one of {MEMORY_ABILITIES}, got {self.ability!r}')
        observations = dict(self.observations)
        if not observations:
            raise ValueError('MemoryEvalCase requires observations')
        object.__setattr__(self, 'observations', observations)
        object.__setattr__(self, 'input_refs', tuple(str(ref) for ref in self.input_refs))


def record_memory_eval(
    cases: Iterable[MemoryEvalCase],
    *,
    systems: Sequence[str] = ('baseline', 'harness'),
    ledger: BenchmarkLedger | None = None,
) -> list[BenchmarkRecord]:
    """Convert memory-eval cases into comparable ledger records.

    Routing mode `A` is the long-context/filesystem baseline. Routing mode `B1`
    is the harness memory substrate. The records keep the existing report path
    usable while the `operation_type` carries the memory ability being tested.
    """

    records: list[BenchmarkRecord] = []
    for case in cases:
        for system in systems:
            observation, routing_mode = _select(case, system)
            record = BenchmarkRecord(
                query_id=case.query_id,
                operation_type=f'memory.{case.ability}',
                routing_mode=routing_mode,
                chosen_executor=observation.system_id,
                candidate_executors=tuple(obs.system_id for obs in case.observations.values()),
                input_refs=case.input_refs,
                receipt_hash=observation.receipt_hash,
                correctness_label=observation.correctness_label,
                synthesis_used_result=observation.answer_grounded,
                decision_tokens=observation.input_tokens + observation.output_tokens,
                executor_cost=observation.executor_cost,
                wall_ms=observation.wall_ms,
                cache_hit=(routing_mode == 'B1'),
                missed_affordance=(case.ability == 'abstention' and not observation.abstained),
            )
            records.append(record)
            if ledger is not None:
                ledger.record(record)
    return records


def token_reduction_vs_baseline(records: Sequence[BenchmarkRecord]) -> dict[str, float]:
    """Return per-routing-mode token reduction against baseline arm `A`."""

    totals: dict[str, int] = {}
    for record in records:
        totals[record.routing_mode] = totals.get(record.routing_mode, 0) + record.decision_tokens
    baseline = totals.get('A', 0)
    if baseline <= 0:
        return {}
    return {
        mode: (baseline - tokens) / baseline
        for mode, tokens in totals.items()
        if mode != 'A'
    }


def _select(case: MemoryEvalCase, system: str) -> tuple[MemoryObservation, str]:
    if system == 'baseline':
        system_id = case.baseline_system
        routing_mode = 'A'
    elif system == 'harness':
        system_id = case.harness_system
        routing_mode = 'B1'
    else:
        system_id = system
        routing_mode = 'B2'
    if system_id not in case.observations:
        raise ValueError(f'case {case.query_id} has no observation for {system_id!r}')
    return case.observations[system_id], routing_mode
