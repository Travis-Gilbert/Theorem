"""Receipt helpers for solver outputs."""

from __future__ import annotations

from .contracts import SolverResult


def result_receipt(result: SolverResult) -> dict:
    payload = result.to_dict()
    return {
        'provider': payload['provider'],
        'formula_hash': payload['formula_hash'],
        'input_view_refs': payload['input_view_refs'],
        'status': payload['status'],
        'counterexample_ref': (
            f'counterexample:{payload["formula_hash"][:16]}'
            if payload['counterexample']
            else ''
        ),
        'unsat_core_ref': payload['unsat_core_ref'],
        'unknown_reason': payload['unknown_reason'],
        'timeout_ms': payload['timeout_ms'],
        'writeback_proposals': payload['writeback_proposals'],
    }

