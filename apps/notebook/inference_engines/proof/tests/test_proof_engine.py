from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.proof.engine import ProofEngine


class ProofEngineTests(SimpleTestCase):
    def test_create_obligation_receipt(self):
        receipt = ProofEngine().create_obligation(
            statement='Private source is never exported by default.',
            assumptions=('exports respect private flag',),
        )

        self.assertEqual(receipt.status, 'created')
        self.assertEqual(receipt.obligation.target_system, 'lean')
        self.assertEqual(receipt.to_dict()['writeback_policy'], 'read-only')

    def test_unknown_receipt_does_not_claim_proof(self):
        created = ProofEngine().create_obligation(statement='schema invariant')
        receipt = ProofEngine().mark_unknown(created.obligation, reason='Lean unavailable')

        self.assertEqual(receipt.status, 'unknown')
        self.assertIn('Lean unavailable', receipt.counterexample['reason'])

