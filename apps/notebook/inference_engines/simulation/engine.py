"""Simulation engine that records auditable dry-run receipts."""

from __future__ import annotations

from apps.notebook.inference_engines.common import stable_hash

from .contracts import SimulationReceipt


class SimulationEngine:
    engine = 'simulation-receipt-fallback'

    def dry_run(self, *, validator: str, inputs: dict, expected: dict | None = None) -> SimulationReceipt:
        expected_payload = dict(expected or {})
        passed = all(inputs.get(key) == value for key, value in expected_payload.items())
        return SimulationReceipt(
            simulation_id=f'sim-{stable_hash({"validator": validator, "inputs": inputs, "expected": expected_payload})[:16]}',
            status='passed' if passed else 'failed',
            validator=validator,
            output={'inputs': dict(inputs), 'expected': expected_payload},
            counterexample=None if passed else {'mismatched_expected': expected_payload, 'actual': dict(inputs)},
        )

    def skipped(self, *, validator: str, reason: str) -> SimulationReceipt:
        return SimulationReceipt(
            simulation_id=f'sim-{stable_hash({"validator": validator, "reason": reason})[:16]}',
            status='skipped',
            validator=validator,
            output={'reason': reason},
        )

