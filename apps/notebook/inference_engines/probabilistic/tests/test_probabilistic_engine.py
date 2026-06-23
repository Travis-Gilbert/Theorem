from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.probabilistic.engine import ProbProgEngine


class ProbProgEngineTests(SimpleTestCase):
    def test_source_reliability_receipt_uses_beta_posterior(self):
        receipt = ProbProgEngine().source_reliability(
            source_id='source-a',
            prior_alpha=2,
            prior_beta=2,
            corroborated=6,
            contradicted=2,
        )

        self.assertEqual(receipt.posterior['alpha'], 8)
        self.assertEqual(receipt.posterior['beta'], 4)
        self.assertAlmostEqual(receipt.mean, 8 / 12)
        self.assertEqual(receipt.to_dict()['writeback_policy'], 'read-only')

    def test_expected_value_of_information_receipt(self):
        receipt = ProbProgEngine().expected_value_of_information(
            current_uncertainty=0.6,
            expected_uncertainty_after=0.2,
            decision_value=10,
            validator_cost=1,
        )

        self.assertGreater(receipt.posterior['expected_value'], 0)

