"""Contracts for constrained feasible design/search optimization."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

from apps.notebook.inference_engines.common import stable_hash

OptimizerStatus = Literal['optimal', 'feasible', 'infeasible', 'unknown']


@dataclass(frozen=True, slots=True)
class OptimizationCandidate:
    candidate_id: str
    value: float
    cost: float
    tags: tuple[str, ...] = ()
    hard_required: bool = False
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            'candidate_id': self.candidate_id,
            'value': float(self.value),
            'cost': float(self.cost),
            'tags': list(self.tags),
            'hard_required': self.hard_required,
            'metadata': dict(self.metadata),
        }


@dataclass(frozen=True, slots=True)
class OptimizationProblem:
    problem_id: str
    objective: str
    candidates: tuple[OptimizationCandidate, ...]
    budget: float
    min_tag_coverage: tuple[str, ...] = ()
    problem_hash: str = ''

    def __post_init__(self) -> None:
        object.__setattr__(self, 'candidates', tuple(self.candidates))
        object.__setattr__(self, 'min_tag_coverage', tuple(self.min_tag_coverage))
        if not self.problem_hash:
            object.__setattr__(
                self,
                'problem_hash',
                stable_hash({
                    'problem_id': self.problem_id,
                    'objective': self.objective,
                    'candidates': [candidate.to_dict() for candidate in self.candidates],
                    'budget': self.budget,
                    'min_tag_coverage': self.min_tag_coverage,
                }),
            )


@dataclass(frozen=True, slots=True)
class OptimizationResult:
    engine: str
    problem_hash: str
    status: OptimizerStatus
    selected: tuple[OptimizationCandidate, ...]
    total_value: float
    total_cost: float
    reason: str

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'problem_hash': self.problem_hash,
            'status': self.status,
            'selected': [candidate.to_dict() for candidate in self.selected],
            'total_value': float(self.total_value),
            'total_cost': float(self.total_cost),
            'reason': self.reason,
            'writeback_policy': 'read-only',
        }

