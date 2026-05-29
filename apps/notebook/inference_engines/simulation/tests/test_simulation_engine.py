from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.simulation.engine import SimulationEngine


class SimulationEngineTests(SimpleTestCase):
    def test_dry_run_receipt_records_counterexample(self):
        receipt = SimulationEngine().dry_run(
            validator='unit-shape',
            inputs={'status': 'failed'},
            expected={'status': 'passed'},
        )

        self.assertEqual(receipt.status, 'failed')
        self.assertIn('actual', receipt.counterexample)

    def test_dry_run_passes_when_expected_matches(self):
        receipt = SimulationEngine().dry_run(
            validator='unit-shape',
            inputs={'status': 'passed'},
            expected={'status': 'passed'},
        )

        self.assertEqual(receipt.status, 'passed')

