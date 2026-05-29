"""Contracts for solver-backed feasibility and counterexample receipts."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from typing import Any, Literal


SolverStatus = Literal['sat', 'unsat', 'unknown', 'timeout', 'invalid']
ALLOWED_SOLVER_STATUSES = ('sat', 'unsat', 'unknown', 'timeout', 'invalid')


def stable_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), default=str)


def stable_hash(value: Any) -> str:
    return hashlib.sha256(stable_json(value).encode('utf-8')).hexdigest()


@dataclass(frozen=True, slots=True)
class SolverConstraint:
    """One safety obligation encoded as a violation predicate."""

    constraint_id: str
    description: str
    violated: bool
    counterexample: dict[str, Any] = field(default_factory=dict)
    severity: str = 'error'

    def to_dict(self) -> dict[str, Any]:
        return {
            'constraint_id': self.constraint_id,
            'description': self.description,
            'violated': bool(self.violated),
            'counterexample': dict(self.counterexample),
            'severity': self.severity,
        }


@dataclass(frozen=True, slots=True)
class SolverProblem:
    """A solver problem whose satisfiability means a violation exists."""

    target: str
    constraints: tuple[SolverConstraint, ...]
    input_view_refs: tuple[str, ...] = ()
    metadata: dict[str, Any] = field(default_factory=dict)
    formula_hash: str = ''

    def __post_init__(self) -> None:
        object.__setattr__(self, 'constraints', tuple(self.constraints))
        object.__setattr__(self, 'input_view_refs', tuple(self.input_view_refs))
        if not self.formula_hash:
            object.__setattr__(
                self,
                'formula_hash',
                stable_hash({
                    'target': self.target,
                    'constraints': [constraint.to_dict() for constraint in self.constraints],
                    'input_view_refs': self.input_view_refs,
                    'metadata': self.metadata,
                }),
            )

    def violated_constraints(self) -> tuple[SolverConstraint, ...]:
        return tuple(constraint for constraint in self.constraints if constraint.violated)

    def to_dict(self) -> dict[str, Any]:
        return {
            'target': self.target,
            'formula_hash': self.formula_hash,
            'constraints': [constraint.to_dict() for constraint in self.constraints],
            'input_view_refs': list(self.input_view_refs),
            'metadata': dict(self.metadata),
        }


@dataclass(frozen=True, slots=True)
class SolverResult:
    """Auditable solver output. It never writes canonical graph state."""

    provider: str
    formula_hash: str
    input_view_refs: tuple[str, ...]
    status: SolverStatus
    model: dict[str, Any] | None = None
    counterexample: dict[str, Any] | None = None
    unsat_core_ref: str = ''
    unknown_reason: str = ''
    timeout_ms: int | None = None
    writeback_proposals: tuple[dict[str, Any], ...] = ()

    def __post_init__(self) -> None:
        if self.status not in ALLOWED_SOLVER_STATUSES:
            raise ValueError(f'Unknown solver status: {self.status}')
        object.__setattr__(self, 'input_view_refs', tuple(self.input_view_refs))
        object.__setattr__(self, 'writeback_proposals', tuple(self.writeback_proposals))

    def to_dict(self) -> dict[str, Any]:
        return {
            'provider': self.provider,
            'formula_hash': self.formula_hash,
            'input_view_refs': list(self.input_view_refs),
            'status': self.status,
            'model': dict(self.model or {}),
            'counterexample': dict(self.counterexample or {}),
            'unsat_core_ref': self.unsat_core_ref,
            'unknown_reason': self.unknown_reason,
            'timeout_ms': self.timeout_ms,
            'writeback_proposals': list(self.writeback_proposals),
        }

