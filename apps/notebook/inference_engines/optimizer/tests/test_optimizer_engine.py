from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.optimizer.contracts import (
    OptimizationCandidate,
    OptimizationProblem,
)
from apps.notebook.inference_engines.optimizer.engine import OptimizerEngine


class OptimizerEngineTests(SimpleTestCase):
    def test_context_selection_respects_budget_and_tag_coverage(self):
        problem = OptimizationProblem(
            problem_id='ctx',
            objective='max_context_value',
            budget=8,
            min_tag_coverage=('fresh', 'independent'),
            candidates=(
                OptimizationCandidate('pin', value=10, cost=4, tags=('pinned',), hard_required=True),
                OptimizationCandidate('fresh', value=5, cost=2, tags=('fresh',)),
                OptimizationCandidate('independent', value=4, cost=2, tags=('independent',)),
                OptimizationCandidate('too-big', value=99, cost=20, tags=('fresh',)),
            ),
        )

        result = OptimizerEngine().optimize(problem)

        self.assertEqual(result.status, 'feasible')
        self.assertEqual({item.candidate_id for item in result.selected}, {'pin', 'fresh', 'independent'})
        self.assertLessEqual(result.total_cost, 8)

    def test_validator_schedule_uses_expected_value(self):
        result = OptimizerEngine().schedule_validators(
            [
                {'id': 'cheap', 'expected_value': 3, 'cost': 1},
                {'id': 'expensive', 'expected_value': 10, 'cost': 8},
            ],
            budget=2,
        )

        self.assertEqual([item.candidate_id for item in result.selected], ['cheap'])

