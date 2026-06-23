from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.causal.engine import CausalEngine


class CausalEngineTests(SimpleTestCase):
    def test_intervention_receipt_is_assumption_bound(self):
        receipt = CausalEngine().intervention_effect(
            question_id='q1',
            treatment='search refresh',
            outcome='answer quality',
            treated_mean=0.8,
            control_mean=0.5,
            assumptions=('no hidden freshness confounder',),
        )

        self.assertEqual(receipt.identifiability_status, 'identified_under_assumptions')
        self.assertAlmostEqual(receipt.estimate, 0.3)
        self.assertIn('exchangeability', receipt.assumptions)
        self.assertEqual(receipt.to_dict()['writeback_policy'], 'proposal-only')

    def test_missing_data_recommends_experiment(self):
        receipt = CausalEngine().intervention_effect(
            question_id='q2',
            treatment='validator',
            outcome='defect catch',
        )

        self.assertEqual(receipt.identifiability_status, 'unknown')
        self.assertIn('Collect', receipt.recommendation)

