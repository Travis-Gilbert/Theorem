"""Alloy-compatible provider for finite counterworld receipts."""

from __future__ import annotations

from apps.notebook.inference_engines.solver.contracts import SolverProblem, SolverResult
from apps.notebook.inference_engines.solver.counterworlds import (
    counterworld_from_problem,
    unsat_core_ref,
    writeback_proposal_for_solver_result,
)


class AlloyProvider:
    provider = 'alloy'

    def solve(self, problem: SolverProblem, *, timeout_ms: int | None = None) -> SolverResult:
        violated = problem.violated_constraints()
        result = SolverResult(
            provider=self.provider,
            formula_hash=problem.formula_hash,
            input_view_refs=problem.input_view_refs,
            status='sat' if violated else 'unsat',
            model={'finite_scope': problem.metadata.get('scope', 'bounded'), 'target': problem.target} if violated else {},
            counterexample=counterworld_from_problem(problem, provider=self.provider) if violated else {},
            unsat_core_ref='' if violated else unsat_core_ref(problem, provider=self.provider),
            timeout_ms=timeout_ms,
        )
        return SolverResult(
            provider=result.provider,
            formula_hash=result.formula_hash,
            input_view_refs=result.input_view_refs,
            status=result.status,
            model=result.model,
            counterexample=result.counterexample,
            unsat_core_ref=result.unsat_core_ref,
            unknown_reason=result.unknown_reason,
            timeout_ms=result.timeout_ms,
            writeback_proposals=writeback_proposal_for_solver_result(problem, result),
        )
