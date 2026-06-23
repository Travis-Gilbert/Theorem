"""Constraint builders for proposed graph patch safety."""

from __future__ import annotations

from typing import Any

from apps.notebook.inference_engines.solver.contracts import SolverConstraint, SolverProblem

from .schema import as_dict_list, truthy


CANONICAL_TRUTH_FIELDS = {
    'text',
    'status',
    'confidence',
    'acceptance_status',
    'epistemic_status',
    'edge_type',
    'from_object',
    'to_object',
    'from_object_id',
    'to_object_id',
}


def _operation_requires_review(operation: dict[str, Any], patch: dict[str, Any]) -> bool:
    return (
        truthy(operation.get('requires_review'))
        or truthy(operation.get('reviewed'))
        or str(operation.get('writeback_policy', '')).lower() in {'review-required', 'proposal-only'}
        or truthy(patch.get('requires_review'))
    )


def build_graph_patch_problem(
    *,
    patch: dict[str, Any],
    input_view_refs: tuple[str, ...] = (),
) -> SolverProblem:
    operations = as_dict_list(patch.get('operations', []))
    silent_rewrites = []
    for index, operation in enumerate(operations):
        op = str(operation.get('op', '') or operation.get('operation', '')).lower()
        target_kind = str(operation.get('target_kind', '') or operation.get('kind', '')).lower()
        fields = set(operation.get('fields', []) or operation.get('changes', {}).keys())
        touches_truth = bool(fields & CANONICAL_TRUTH_FIELDS)
        mutates_canon = op in {'update', 'rewrite', 'delete', 'replace'} and target_kind in {'claim', 'edge', 'object'}
        if mutates_canon and touches_truth and not _operation_requires_review(operation, patch):
            silent_rewrites.append({
                'index': index,
                'operation': operation,
                'canonical_fields': sorted(fields & CANONICAL_TRUTH_FIELDS),
            })

    return SolverProblem(
        target='graph_patch_safety',
        constraints=(
            SolverConstraint(
                constraint_id='graph_patch_no_silent_canonical_rewrite',
                description='Graph patch cannot silently rewrite canonical truth fields.',
                violated=bool(silent_rewrites),
                counterexample={'operations': silent_rewrites},
            ),
        ),
        input_view_refs=input_view_refs,
        metadata={'operation_count': len(operations)},
    )

