from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.egraph.engine import EGraphTheorem
from apps.notebook.inference_engines.egraph.receipts import rewrite_trace_as_context_atom


class EGraphTheoremTests(SimpleTestCase):
    def test_context_pack_extracts_lower_cost_equivalent_form(self):
        receipt = EGraphTheorem().context_pack(
            expression_id='ctx-1',
            items=[
                {'id': 'a', 'channel': 'trusted_repo_memory', 'obligation_id': 'o1', 'text': 'Keep tests green', 'tokens': 20},
                {'id': 'b', 'channel': 'trusted_repo_memory', 'obligation_id': 'o1', 'text': 'Keep tests green', 'tokens': 20},
                {'id': 'c', 'channel': 'external_content', 'obligation_id': 'o2', 'text': '', 'tokens': 5},
            ],
        )

        self.assertTrue(receipt.equivalent)
        self.assertLess(receipt.extracted_cost, receipt.original_cost)
        self.assertEqual(len(receipt.extraction.items), 1)
        self.assertEqual(
            [step.rule_id for step in receipt.rewrite_trace],
            ['drop_empty_optional', 'dedupe_same_obligation'],
        )

    def test_rewrite_trace_can_become_context_atom(self):
        receipt = EGraphTheorem().context_pack(
            expression_id='ctx-2',
            items=[
                {'id': 'a', 'channel': 'user_task', 'obligation_id': 'o1', 'text': 'Ship it', 'tokens': 5},
            ],
        )

        atom = rewrite_trace_as_context_atom(receipt)

        self.assertEqual(atom['kind'], 'policy')
        self.assertIn('rewrite_trace', atom['metadata'])

