"""Parity tests for the eight-engine affordance projection.

Each test asserts the Gate-0 invariant for one engine: the affordance payload is
byte-identical to the direct engine receipt's `to_dict()`, the affordance receipt
hash is deterministic, and the projection metadata (engine_id, affordance_id,
writeback_policy) is correct. Pure functions only, so SimpleTestCase suffices.
"""

from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.affordances import AffordanceReceipt
from apps.notebook.inference_engines.affordances_engines import (
    ENGINE_AFFORDANCES,
    engine_affordance_differential_cases,
    measure_affordance_cost,
    run_causal_affordance,
    run_egraph_affordance,
    run_engine_affordance_parity,
    run_evolution_affordance,
    run_expression_affordance,
    run_optimizer_affordance,
    run_proof_affordance,
    run_simulation_affordance,
    run_solver_affordance,
)


class CausalAffordanceParityTests(SimpleTestCase):
    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.causal.engine import CausalEngine

        direct = CausalEngine().intervention_effect(
            question_id='q-1',
            treatment='exposure',
            outcome='recovery',
            treated_mean=2.0,
            control_mean=1.0,
        )
        affordance = run_causal_affordance(
            question_id='q-1',
            treatment='exposure',
            outcome='recovery',
            treated_mean=2.0,
            control_mean=1.0,
        )
        self.assertIsInstance(affordance, AffordanceReceipt)
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'causal.intervention_effect')
        self.assertEqual(affordance.engine_id, 'assumption-bound-causal-fallback')
        self.assertEqual(affordance.writeback_policy, 'proposal-only')

    def test_receipt_hash_is_deterministic(self):
        a = run_causal_affordance(question_id='q', treatment='t', outcome='o')
        b = run_causal_affordance(question_id='q', treatment='t', outcome='o')
        self.assertEqual(a.receipt_hash, b.receipt_hash)
        self.assertTrue(a.receipt_hash)


class EvolutionAffordanceParityTests(SimpleTestCase):
    def _candidates(self):
        from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate

        return [
            EvolutionCandidate(candidate_id='c1', niche='n1', score=0.9, payload={'k': 1}, novelty=0.2),
            EvolutionCandidate(candidate_id='c2', niche='n1', score=0.4, payload={}, novelty=0.1),
            EvolutionCandidate(candidate_id='c3', niche='n2', score=0.7, payload={'k': 2}),
        ]

    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.evolution.engine import EvolutionEngine

        candidates = self._candidates()
        direct = EvolutionEngine().archive(candidates, elites_per_niche=2)
        affordance = run_evolution_affordance(candidates, elites_per_niche=2)
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'evolution.archive')
        self.assertEqual(affordance.engine_id, 'quality-diversity-python-fallback')

    def test_dict_input_matches_typed_input(self):
        candidates = self._candidates()
        typed = run_evolution_affordance(candidates, elites_per_niche=2)
        as_dicts = run_evolution_affordance(
            [candidate.to_dict() for candidate in candidates],
            elites_per_niche=2,
        )
        self.assertEqual(typed.payload, as_dicts.payload)


class ProofAffordanceParityTests(SimpleTestCase):
    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.proof.engine import ProofEngine

        direct = ProofEngine().create_obligation(
            statement='forall x: safe(x)',
            target_system='lean',
            assumptions=('well_typed',),
            metadata={'origin': 'unit'},
        )
        affordance = run_proof_affordance(
            statement='forall x: safe(x)',
            target_system='lean',
            assumptions=('well_typed',),
            obligation_metadata={'origin': 'unit'},
        )
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'proof.create_obligation')
        self.assertEqual(affordance.engine_id, 'proof-obligation-tracker')


class OptimizerAffordanceParityTests(SimpleTestCase):
    def _problem(self):
        from apps.notebook.inference_engines.optimizer.contracts import (
            OptimizationCandidate,
            OptimizationProblem,
        )

        return OptimizationProblem(
            problem_id='p-1',
            objective='max_value',
            candidates=(
                OptimizationCandidate(candidate_id='a', value=5.0, cost=2.0, tags=('core',), hard_required=True),
                OptimizationCandidate(candidate_id='b', value=3.0, cost=1.0, tags=('aux',)),
                OptimizationCandidate(candidate_id='c', value=4.0, cost=3.0, tags=('aux',)),
            ),
            budget=4.0,
            min_tag_coverage=('aux',),
        )

    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.optimizer.engine import OptimizerEngine

        problem = self._problem()
        direct = OptimizerEngine().optimize(problem)
        affordance = run_optimizer_affordance(problem)
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'optimizer.optimize')
        self.assertEqual(affordance.engine_id, 'python-deterministic-optimizer')
        self.assertEqual(affordance.input_hash, problem.problem_hash)


class ExpressionAffordanceParityTests(SimpleTestCase):
    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.expression.registry import get_expression_registry

        result = {'status': 'feasible', 'engine': 'optimizer', 'receipt_hash': 'abc123'}
        direct = get_expression_registry().render('deterministic_brief', dict(result), metadata={})
        affordance = run_expression_affordance(result, engine_id='deterministic_brief')
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'expression.deterministic_brief')
        self.assertEqual(affordance.engine_id, 'deterministic_brief')


class EGraphAffordanceParityTests(SimpleTestCase):
    def _expression(self):
        from apps.notebook.inference_engines.egraph.contracts import EGraphExpression

        return EGraphExpression(
            expression_id='ctx-1',
            domain='context_pack',
            items=(
                {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
                {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
            ),
        )

    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.egraph.engine import EGraphTheorem

        expression = self._expression()
        direct = EGraphTheorem().extract(expression, max_rounds=8)
        affordance = run_egraph_affordance(expression, max_rounds=8)
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'egraph.extract')
        self.assertEqual(affordance.engine_id, 'egraph-theorem')
        self.assertEqual(affordance.input_hash, expression.expression_hash)


class SimulationAffordanceParityTests(SimpleTestCase):
    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.simulation.engine import SimulationEngine

        direct = SimulationEngine().dry_run(
            validator='schema_check',
            inputs={'status': 'ok', 'count': 3},
            expected={'status': 'ok'},
        )
        affordance = run_simulation_affordance(
            validator='schema_check',
            inputs={'status': 'ok', 'count': 3},
            expected={'status': 'ok'},
        )
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'simulation.dry_run')


class SolverAffordanceParityTests(SimpleTestCase):
    def _problem(self):
        from apps.notebook.inference_engines.solver.contracts import (
            SolverConstraint,
            SolverProblem,
        )

        return SolverProblem(
            target='no_private_export',
            constraints=(
                SolverConstraint(
                    constraint_id='priv-1',
                    description='private source reaches export',
                    violated=True,
                ),
            ),
            input_view_refs=('view:export-candidates',),
        )

    def test_payload_matches_direct_engine(self):
        from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider

        problem = self._problem()
        direct = Z3Provider().solve(problem)
        affordance = run_solver_affordance(problem)
        self.assertEqual(affordance.payload, direct.to_dict())
        self.assertEqual(affordance.affordance_id, 'solver.check')
        self.assertEqual(affordance.engine_id, 'z3')
        self.assertEqual(affordance.writeback_policy, 'proposal-only')
        self.assertEqual(affordance.input_hash, problem.formula_hash)


class EngineAffordanceRegistryTests(SimpleTestCase):
    def test_registry_covers_eight_engines(self):
        expected = {
            'causal.intervention_effect',
            'evolution.archive',
            'proof.create_obligation',
            'optimizer.optimize',
            'expression.render',
            'egraph.extract',
            'simulation.dry_run',
            'solver.check',
        }
        self.assertEqual(set(ENGINE_AFFORDANCES), expected)

    def test_registry_callables_are_the_public_functions(self):
        self.assertIs(ENGINE_AFFORDANCES['causal.intervention_effect'], run_causal_affordance)
        self.assertIs(ENGINE_AFFORDANCES['solver.check'], run_solver_affordance)


class EngineAffordanceDifferentialTests(SimpleTestCase):
    def test_parity_report_passes_for_eight_engines(self):
        report = run_engine_affordance_parity()
        self.assertTrue(report['passed'], report['failures'])
        self.assertEqual(report['engine_count'], 8)
        self.assertEqual(len(report['per_engine']), 8)

    def test_cost_measurement_covers_eight_engines(self):
        rows = measure_affordance_cost(iterations=3)
        self.assertEqual(len(rows), 8)
        self.assertEqual(
            {row['affordance_id'] for row in rows},
            {
                'causal.intervention_effect', 'evolution.archive', 'proof.create_obligation',
                'optimizer.optimize', 'expression.render', 'egraph.extract',
                'simulation.dry_run', 'solver.check',
            },
        )
        for row in rows:
            self.assertEqual(row['iterations'], 3)
            # Timing is observational; assert non-negative + present, not exact.
            self.assertGreaterEqual(row['cpu_seconds_total'], 0.0)
            self.assertGreaterEqual(row['cpu_us_per_call'], 0.0)

    def test_differential_cases_fire_non_trivial_receipts(self):
        # Guard against the empty==empty hollow-gate trap: each representative
        # input must produce a substantive receipt, not a degenerate one.
        by_id = {case['affordance_id']: case['direct_receipt'] for case in engine_affordance_differential_cases()}
        self.assertEqual(by_id['causal.intervention_effect']['identifiability_status'], 'identified_under_assumptions')
        self.assertIsNotNone(by_id['causal.intervention_effect']['estimate'])
        self.assertTrue(by_id['evolution.archive']['elites_by_niche'])
        self.assertEqual(by_id['proof.create_obligation']['status'], 'created')
        self.assertEqual(by_id['optimizer.optimize']['status'], 'feasible')
        self.assertTrue(by_id['optimizer.optimize']['selected'])
        self.assertTrue(by_id['expression.render']['payload'])
        self.assertTrue(by_id['egraph.extract']['rewrite_trace'])
        self.assertEqual(by_id['simulation.dry_run']['status'], 'passed')
        self.assertEqual(by_id['solver.check']['status'], 'sat')
