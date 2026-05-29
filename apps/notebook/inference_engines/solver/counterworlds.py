"""Counterworld and invariant artifacts for solver-backed BGI checks."""

from __future__ import annotations

from typing import Any

from apps.notebook.inference_engines.common import stable_hash

from .contracts import SolverProblem, SolverResult


def counterworld_from_problem(problem: SolverProblem, *, provider: str) -> dict[str, Any]:
    """Build a bounded finite counterworld from violated constraints."""

    violated = [constraint.to_dict() for constraint in problem.violated_constraints()]
    return {
        'counterworld_id': f'counterworld-{stable_hash({"provider": provider, "violated": violated})[:16]}',
        'provider': provider,
        'target': problem.target,
        'violated_constraints': violated,
        'violations': violated,
        'input_view_refs': list(problem.input_view_refs),
        'graph_patch_safe': not violated,
        'privacy_invariants': _privacy_invariants(problem),
        'writeback_policy': 'proposal-only',
    }


def unsat_core_ref(problem: SolverProblem, *, provider: str) -> str:
    """Return a stable reference for the constraints that proved safe."""

    core = [
        {
            'constraint_id': constraint.constraint_id,
            'description': constraint.description,
        }
        for constraint in problem.constraints
        if not constraint.violated
    ]
    return f'{provider}:unsat-core:{stable_hash({"target": problem.target, "core": core})[:24]}'


def writeback_proposal_for_solver_result(problem: SolverProblem, result: SolverResult) -> tuple[dict[str, Any], ...]:
    if result.status != 'sat':
        return ()
    return ({
        'proposal_id': f'solver-review-{stable_hash(result.to_dict())[:16]}',
        'target': problem.target,
        'reason': 'Solver found a feasible counterexample; review before graph writeback.',
        'payload': {
            'formula_hash': result.formula_hash,
            'counterexample': dict(result.counterexample or {}),
            'provider': result.provider,
        },
        'review_required': True,
        'writeback_policy': 'proposal-only',
    },)


def _privacy_invariants(problem: SolverProblem) -> dict[str, Any]:
    metadata = dict(problem.metadata or {})
    exported_private_refs = metadata.get('exported_private_refs') or []
    return {
        'raw_private_content_excluded': True,
        'exported_private_ref_count': len(exported_private_refs),
        'exported_private_refs_hash': stable_hash(exported_private_refs) if exported_private_refs else '',
    }
