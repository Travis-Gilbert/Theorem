from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.solver.contracts import (
    ALLOWED_SOLVER_STATUSES,
    SolverConstraint,
    SolverProblem,
    SolverResult,
)
from apps.notebook.inference_engines.solver.providers.alloy_provider import AlloyProvider
from apps.notebook.inference_engines.solver.providers.cvc5_provider import CVC5Provider
from apps.notebook.inference_engines.solver.providers.z3_provider import map_z3_status
from apps.notebook.inference_engines.solver.receipts import result_receipt


class SolverStatusReceiptTests(SimpleTestCase):
    def test_status_contract_rejects_unknown_labels(self):
        with self.assertRaises(ValueError):
            SolverResult(
                provider='test',
                formula_hash='abc',
                input_view_refs=(),
                status='maybe',  # type: ignore[arg-type]
            )

    def test_status_mapping_is_bounded(self):
        self.assertEqual(map_z3_status('sat'), 'sat')
        self.assertEqual(map_z3_status('unsat'), 'unsat')
        self.assertEqual(map_z3_status('unknown'), 'unknown')
        self.assertEqual(map_z3_status('other'), 'invalid')
        self.assertEqual(set(ALLOWED_SOLVER_STATUSES), {'sat', 'unsat', 'unknown', 'timeout', 'invalid'})

    def test_registered_future_providers_return_bounded_receipts(self):
        problem = SolverProblem(
            target='future-provider',
            constraints=(SolverConstraint('c1', 'demo', False),),
        )

        cvc5 = CVC5Provider().solve(problem)
        alloy = AlloyProvider().solve(problem)

        self.assertEqual(cvc5.status, 'unsat')
        self.assertEqual(alloy.status, 'unsat')
        self.assertTrue(cvc5.unsat_core_ref)
        self.assertTrue(alloy.unsat_core_ref)
        self.assertEqual(result_receipt(cvc5)['writeback_proposals'], [])

    def test_registered_future_providers_emit_counterworlds_for_violations(self):
        problem = SolverProblem(
            target='future-provider',
            constraints=(SolverConstraint('c1', 'demo', True, {'x': 1}),),
        )

        cvc5 = CVC5Provider().solve(problem)
        alloy = AlloyProvider().solve(problem)

        self.assertEqual(cvc5.status, 'sat')
        self.assertEqual(alloy.status, 'sat')
        self.assertIn('counterworld_id', cvc5.counterexample)
        self.assertTrue(alloy.writeback_proposals)
