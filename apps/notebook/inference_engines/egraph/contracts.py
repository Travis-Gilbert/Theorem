"""Contracts for equivalence-preserving rewrite receipts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class EGraphExpression:
    """A small expression over context/query/code fragments.

    The Python fallback treats expressions as ordered item lists with explicit
    obligations and channels. Native egg/egglog can later implement the same
    receipt surface with richer equivalence classes.
    """

    expression_id: str
    domain: str
    items: tuple[dict[str, Any], ...]
    metadata: dict[str, Any] = field(default_factory=dict)
    expression_hash: str = ''

    def __post_init__(self) -> None:
        object.__setattr__(self, 'items', tuple(dict(item) for item in self.items))
        if not self.expression_hash:
            object.__setattr__(
                self,
                'expression_hash',
                stable_hash({
                    'expression_id': self.expression_id,
                    'domain': self.domain,
                    'items': self.items,
                    'metadata': self.metadata,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'expression_id': self.expression_id,
            'domain': self.domain,
            'items': [dict(item) for item in self.items],
            'metadata': dict(self.metadata),
            'expression_hash': self.expression_hash,
        }


@dataclass(frozen=True, slots=True)
class RewriteStep:
    rule_id: str
    before_hash: str
    after_hash: str
    reason: str
    delta_cost: float
    data: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            'rule_id': self.rule_id,
            'before_hash': self.before_hash,
            'after_hash': self.after_hash,
            'reason': self.reason,
            'delta_cost': float(self.delta_cost),
            'data': dict(self.data),
        }


@dataclass(frozen=True, slots=True)
class EGraphReceipt:
    engine: str
    input_hash: str
    output_hash: str
    domain: str
    equivalent: bool
    original_cost: float
    extracted_cost: float
    extraction: EGraphExpression
    rewrite_trace: tuple[RewriteStep, ...] = ()
    native_backend: str = 'python-fallback'

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'native_backend': self.native_backend,
            'input_hash': self.input_hash,
            'output_hash': self.output_hash,
            'domain': self.domain,
            'equivalent': self.equivalent,
            'original_cost': float(self.original_cost),
            'extracted_cost': float(self.extracted_cost),
            'rewrite_trace': [step.to_dict() for step in self.rewrite_trace],
            'extraction': self.extraction.to_dict(),
        }

