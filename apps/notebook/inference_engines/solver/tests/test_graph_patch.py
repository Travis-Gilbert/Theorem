from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.solver.constraint_builders.graph_patch import (
    build_graph_patch_problem,
)
from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider


class GraphPatchSolverTests(SimpleTestCase):
    def test_graph_patch_counterexample_for_silent_truth_rewrite(self):
        problem = build_graph_patch_problem(
            patch={
                'operations': [
                    {
                        'op': 'update',
                        'target_kind': 'claim',
                        'target_id': 12,
                        'fields': ['text', 'confidence'],
                    },
                ],
            },
            input_view_refs=('graph-patch-view-1',),
        )

        result = Z3Provider().solve(problem)

        self.assertEqual(result.status, 'sat')
        self.assertTrue(result.writeback_proposals)
        self.assertEqual(
            result.counterexample['violated_constraints'][0]['constraint_id'],
            'graph_patch_no_silent_canonical_rewrite',
        )

    def test_review_required_patch_is_safe(self):
        problem = build_graph_patch_problem(
            patch={
                'operations': [
                    {
                        'op': 'update',
                        'target_kind': 'claim',
                        'target_id': 12,
                        'fields': ['text'],
                        'writeback_policy': 'review-required',
                    },
                ],
            },
        )

        self.assertEqual(Z3Provider().solve(problem).status, 'unsat')
