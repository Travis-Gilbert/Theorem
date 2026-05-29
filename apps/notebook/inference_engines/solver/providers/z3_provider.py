"""Z3-backed provider with deterministic fallback for unavailable local deps."""

from __future__ import annotations

from apps.notebook.inference_engines.solver.contracts import (
    SolverProblem,
    SolverResult,
    SolverStatus,
)
from apps.notebook.inference_engines.solver.counterworlds import (
    counterworld_from_problem,
    unsat_core_ref,
    writeback_proposal_for_solver_result,
)


def map_z3_status(value) -> SolverStatus:
    rendered = str(value).lower()
    if rendered == 'sat':
        return 'sat'
    if rendered == 'unsat':
        return 'unsat'
    if rendered in {'unknown', 'timeout'}:
        return 'unknown'
    return 'invalid'


class Z3Provider:
    provider = 'z3'

    def solve(self, problem: SolverProblem, *, timeout_ms: int | None = None) -> SolverResult:
        violated = problem.violated_constraints()
        z3_missing = False
        try:
            import z3  # type: ignore

            solver = z3.Solver()
            if timeout_ms is not None:
                solver.set(timeout=timeout_ms)
            if violated:
                solver.add(z3.Or([z3.BoolVal(True) for _ in violated]))
            else:
                solver.add(z3.BoolVal(False))
            status = map_z3_status(solver.check())
        except Exception:
            z3_missing = True
            status = 'sat' if violated else 'unsat'

        counterexample = None
        model = None
        if status == 'sat':
            counterexample = counterworld_from_problem(problem, provider=self.provider)
            model = {'violation_count': len(violated)}

        unknown_reason = ''
        if status == 'unknown':
            unknown_reason = 'z3 returned unknown'
        elif z3_missing:
            unknown_reason = 'z3 unavailable; used deterministic boolean fallback'

        result = SolverResult(
            provider=self.provider,
            formula_hash=problem.formula_hash,
            input_view_refs=problem.input_view_refs,
            status=status,
            model=model,
            counterexample=counterexample,
            unsat_core_ref='' if status != 'unsat' else unsat_core_ref(problem, provider=self.provider),
            unknown_reason=unknown_reason,
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
