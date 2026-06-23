"""Contracts for turning structured inference results into useful artifacts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Callable

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class ExpressionInput:
    result: dict[str, Any]
    audience: str = 'operator'
    metadata: dict[str, Any] = field(default_factory=dict)


@dataclass(frozen=True, slots=True)
class ExpressionResult:
    engine_id: str
    artifact_type: str
    payload: dict[str, Any]
    receipt_hash: str = ''

    def __post_init__(self) -> None:
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'engine_id': self.engine_id,
                    'artifact_type': self.artifact_type,
                    'payload': self.payload,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine_id': self.engine_id,
            'artifact_type': self.artifact_type,
            'payload': dict(self.payload),
            'receipt_hash': self.receipt_hash,
            'writeback_policy': 'read-only',
        }


@dataclass(frozen=True, slots=True)
class ExpressionEngineRegistration:
    engine_id: str
    artifact_type: str
    renderer: Callable[[ExpressionInput], ExpressionResult]
    description: str = ''

