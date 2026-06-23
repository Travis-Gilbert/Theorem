"""Gate-0 run-level test for the eight engine affordances.

Exercises run_gate0_engine_affordances end to end: it should emit one
BenchmarkRecord per engine, all labelled correct (projection fidelity holds),
the gate should pass, and the records should be appended to the ledger.
"""

from __future__ import annotations

import tempfile
from pathlib import Path

from django.test import SimpleTestCase

from apps.notebook.inference_engines.benchmark.gate0 import run_gate0_engine_affordances
from apps.notebook.inference_engines.benchmark.ledger import BenchmarkLedger

EXPECTED_AFFORDANCES = {
    'causal.intervention_effect',
    'evolution.archive',
    'proof.create_obligation',
    'optimizer.optimize',
    'expression.render',
    'egraph.extract',
    'simulation.dry_run',
    'solver.check',
}


class Gate0EngineAffordanceTests(SimpleTestCase):
    def test_run_without_ledger_passes_for_eight_engines(self):
        result = run_gate0_engine_affordances()
        self.assertEqual(len(result.records), 8)
        self.assertTrue(result.passed, [r.operation_type for r in result.records if r.correctness_label != 'correct'])
        self.assertEqual(
            {record.operation_type for record in result.records},
            EXPECTED_AFFORDANCES,
        )

    def test_records_have_consistent_shape(self):
        result = run_gate0_engine_affordances()
        for record in result.records:
            self.assertEqual(record.correctness_label, 'correct')
            self.assertEqual(record.routing_mode, 'B1')
            self.assertTrue(record.chosen_executor.endswith('-cpu'))
            self.assertIn(record.chosen_executor, record.candidate_executors)
            self.assertTrue(record.receipt_hash)

    def test_run_with_ledger_appends_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            ledger_path = Path(tmp) / 'gate0.jsonl'
            ledger = BenchmarkLedger(ledger_path)
            result = run_gate0_engine_affordances(ledger=ledger)
            self.assertTrue(result.passed)
            persisted = ledger.records()
            self.assertEqual(len(persisted), 8)
            self.assertEqual(
                {record.operation_type for record in persisted},
                EXPECTED_AFFORDANCES,
            )
