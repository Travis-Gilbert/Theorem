"""Proof obligation tracking contracts."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Literal

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class ProofObligation:
    obligation_id: str
    statement: str
    target_system: str
    assumptions: tuple[str, ...] = ()
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            'obligation_id': self.obligation_id,
            'statement': self.statement,
            'target_system': self.target_system,
            'assumptions': list(self.assumptions),
            'metadata': dict(self.metadata),
        }


@dataclass(frozen=True, slots=True)
class ProofReceipt:
    engine: str
    obligation: ProofObligation
    status: Literal['created', 'proved', 'failed', 'unknown']
    proof_ref: str = ''
    counterexample: dict[str, Any] | None = None
    receipt_hash: str = ''

    def __post_init__(self) -> None:
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'engine': self.engine,
                    'obligation': self.obligation.to_dict(),
                    'status': self.status,
                    'proof_ref': self.proof_ref,
                    'counterexample': self.counterexample,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'obligation': self.obligation.to_dict(),
            'status': self.status,
            'proof_ref': self.proof_ref,
            'counterexample': dict(self.counterexample or {}),
            'receipt_hash': self.receipt_hash,
            'writeback_policy': 'read-only',
        }

