"""Proof engine fallback for tracking obligations before automation exists."""

from __future__ import annotations

from apps.notebook.inference_engines.common import stable_hash

from .contracts import ProofObligation, ProofReceipt


class ProofEngine:
    engine = 'proof-obligation-tracker'

    def create_obligation(
        self,
        *,
        statement: str,
        target_system: str = 'lean',
        assumptions: tuple[str, ...] = (),
        metadata: dict | None = None,
    ) -> ProofReceipt:
        obligation = ProofObligation(
            obligation_id=f'proof-{stable_hash({"statement": statement, "target": target_system})[:16]}',
            statement=statement,
            target_system=target_system,
            assumptions=tuple(assumptions),
            metadata=dict(metadata or {}),
        )
        return ProofReceipt(
            engine=self.engine,
            obligation=obligation,
            status='created',
        )

    def mark_unknown(self, obligation: ProofObligation, *, reason: str) -> ProofReceipt:
        return ProofReceipt(
            engine=self.engine,
            obligation=obligation,
            status='unknown',
            counterexample={'reason': reason},
        )

