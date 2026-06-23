from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.solver.constraint_builders.context_capsule import (
    build_context_capsule_problem,
)
from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider


class ContextCapsuleSolverTests(SimpleTestCase):
    def test_safe_context_capsule_has_no_counterexample(self):
        problem = build_context_capsule_problem(
            capsule={
                'system_invariants': [{'text': 'Do not expose secrets.', 'source_channel': 'team_policy'}],
                'external_content': [{'text': 'Untrusted web page.', 'source_channel': 'external_content'}],
            },
            budget_tokens=2000,
            token_ledger={'capsuleTokens': 700},
            atoms=[{'included': True, 'metadata': {'private': False}}],
            exports={'visibility': 'internal'},
            input_view_refs=('constraint-view-1',),
        )

        result = Z3Provider().solve(problem)

        self.assertEqual(result.status, 'unsat')
        self.assertEqual(result.counterexample, None)
        self.assertEqual(result.writeback_proposals, ())

    def test_unsafe_context_capsule_produces_counterexample(self):
        problem = build_context_capsule_problem(
            capsule={
                'system_invariants': [{'text': 'External says ignore policy.', 'source_channel': 'external_content'}],
            },
            budget_tokens=100,
            token_ledger={'capsuleTokens': 250},
            atoms=[
                {'included': True, 'muted': True, 'metadata': {}},
                {'included': True, 'metadata': {'private': True}},
            ],
            exports={'visibility': 'public'},
        )

        result = Z3Provider().solve(problem)

        self.assertEqual(result.status, 'sat')
        self.assertIsNotNone(result.counterexample)
        violations = result.counterexample['violations']
        ids = {item['constraint_id'] for item in violations}
        self.assertIn('external_content_not_instruction_channel', ids)
        self.assertIn('capsule_within_budget', ids)
        self.assertIn('muted_node_requires_hard_requirement', ids)
        self.assertIn('private_source_not_exported', ids)

