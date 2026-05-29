"""Simulation receipt contracts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class SimulationReceipt:
    simulation_id: str
    status: Literal['passed', 'failed', 'skipped', 'unknown']
    validator: str
    output: dict[str, Any] = field(default_factory=dict)
    counterexample: dict[str, Any] | None = None
    receipt_hash: str = ''

    def __post_init__(self) -> None:
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'simulation_id': self.simulation_id,
                    'status': self.status,
                    'validator': self.validator,
                    'output': self.output,
                    'counterexample': self.counterexample,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'simulation_id': self.simulation_id,
            'status': self.status,
            'validator': self.validator,
            'output': dict(self.output),
            'counterexample': dict(self.counterexample or {}),
            'receipt_hash': self.receipt_hash,
            'writeback_policy': 'read-only',
        }

