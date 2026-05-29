"""Contracts for lightweight probabilistic receipts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from apps.notebook.inference_engines.common import clamp01, stable_hash


@dataclass(frozen=True, slots=True)
class PosteriorReceipt:
    engine: str
    model_id: str
    prior: dict[str, float]
    observations: dict[str, float]
    posterior: dict[str, float]
    receipt_hash: str = ''
    metadata: dict[str, Any] = field(default_factory=dict)

    def __post_init__(self) -> None:
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'engine': self.engine,
                    'model_id': self.model_id,
                    'prior': self.prior,
                    'observations': self.observations,
                    'posterior': self.posterior,
                    'metadata': self.metadata,
                }),
            )

    @property
    def mean(self) -> float:
        return clamp01(float(self.posterior.get('mean', 0.0)))

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'model_id': self.model_id,
            'prior': dict(self.prior),
            'observations': dict(self.observations),
            'posterior': dict(self.posterior),
            'metadata': dict(self.metadata),
            'receipt_hash': self.receipt_hash,
            'writeback_policy': 'read-only',
        }

