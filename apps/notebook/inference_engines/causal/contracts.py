"""Contracts for assumption-bound causal receipts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class CausalReceipt:
    engine: str
    question_id: str
    assumptions: tuple[str, ...]
    identifiability_status: str
    estimate: float | None = None
    recommendation: str = ''
    receipt_hash: str = ''
    metadata: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        object.__setattr__(self, 'assumptions', tuple(self.assumptions))
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'engine': self.engine,
                    'question_id': self.question_id,
                    'assumptions': self.assumptions,
                    'identifiability_status': self.identifiability_status,
                    'estimate': self.estimate,
                    'recommendation': self.recommendation,
                    'metadata': self.metadata,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'question_id': self.question_id,
            'assumptions': list(self.assumptions),
            'identifiability_status': self.identifiability_status,
            'estimate': self.estimate,
            'recommendation': self.recommendation,
            'metadata': dict(self.metadata),
            'receipt_hash': self.receipt_hash,
            'truth_type': 'causality',
            'writeback_policy': 'proposal-only',
        }

