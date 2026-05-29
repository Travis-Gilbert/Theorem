"""Substrate affordance projection for the eight engines not yet wrapped.

Part 4 Part C ("inference engines as affordances") requires every one of the ten
symbolic engines to be callable as a substrate affordance that returns a
content-addressed receipt. `affordances.py` already projects two engines
(datalog, probabilistic). This module completes the projection for the
remaining eight: causal, evolution, proof, optimizer, expression, egraph,
simulation, solver.

Design contract (kept identical to `affordances.py`):

- The engine core is unchanged. Each wrapper runs the existing reference engine
  and folds its `receipt.to_dict()` into a uniform `AffordanceReceipt`, adding
  projection metadata (input identity, provenance) the benchmark ledger needs.
- `AffordanceReceipt` is imported from `affordances.py`, not redefined, so the
  substrate has exactly one affordance-receipt contract.
- The Gate-0 invariant for these engines is projection fidelity:
  `run_<engine>_affordance(...).payload == Engine().<method>(...).to_dict()`.
  Both sides call the same Python reference engine, so they agree by
  construction; the differential gate (offload lane) asserts it explicitly.

This module is intentionally a sibling of `affordances.py` rather than an edit
to it, because `affordances.py` is co-owned with the Rust-theorem lane
(RT-3.2 engine-factory seam). It is designed to fold into `affordances.py` once
that lane settles; the offload lane wires these wrappers into
`benchmark/gate0.py` + `benchmark/differential.py`.
"""

from __future__ import annotations

from typing import Any, Iterable, Mapping, Sequence

from apps.notebook.inference_engines.affordances import AffordanceReceipt
from apps.notebook.inference_engines.common import stable_hash

Record = Mapping[str, Any] | Any


# ---------------------------------------------------------------------------
# Shared projection helper
# ---------------------------------------------------------------------------

def _project(
    *,
    engine_id: str,
    affordance_id: str,
    receipt: Any,
    input_hash: str,
    input_node_refs: Sequence[str] = (),
    provenance: Mapping[str, Any] | None = None,
    metadata: Mapping[str, Any] | None = None,
    writeback_policy: str | None = None,
) -> AffordanceReceipt:
    """Fold an engine receipt into the uniform substrate AffordanceReceipt."""

    payload = receipt.to_dict() if hasattr(receipt, 'to_dict') else dict(receipt)
    resolved_writeback = writeback_policy or str(payload.get('writeback_policy') or 'read-only')
    return AffordanceReceipt(
        engine_id=engine_id,
        affordance_id=affordance_id,
        input_hash=input_hash,
        input_node_refs=tuple(ref for ref in input_node_refs if ref),
        payload=payload,
        writeback_policy=resolved_writeback,
        provenance=dict(provenance or {}),
        metadata=dict(metadata or {}),
    )


def _refs(values: Iterable[Any]) -> tuple[str, ...]:
    """Deterministic, de-duplicated node-ref tuple from an id iterable."""

    out: list[str] = []
    for value in values:
        if value in (None, ''):
            continue
        out.append(str(value))
    return tuple(dict.fromkeys(out))


# ---------------------------------------------------------------------------
# Causal (assumption-bound intervention estimates)
# ---------------------------------------------------------------------------

def run_causal_affordance(
    *,
    question_id: str,
    treatment: str,
    outcome: str,
    treated_mean: float | None = None,
    control_mean: float | None = None,
    assumptions: Sequence[str] = (),
    confounders: Sequence[str] = (),
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project the causal intervention-effect estimate as a substrate affordance."""

    from apps.notebook.inference_engines.causal.engine import CausalEngine

    receipt = CausalEngine().intervention_effect(
        question_id=question_id,
        treatment=treatment,
        outcome=outcome,
        treated_mean=treated_mean,
        control_mean=control_mean,
        assumptions=tuple(assumptions),
        confounders=tuple(confounders),
    )
    input_hash = stable_hash({
        'question_id': question_id,
        'treatment': treatment,
        'outcome': outcome,
        'treated_mean': treated_mean,
        'control_mean': control_mean,
        'assumptions': list(assumptions),
        'confounders': list(confounders),
    })
    return _project(
        engine_id=str(receipt.to_dict()['engine']),
        affordance_id='causal.intervention_effect',
        receipt=receipt,
        input_hash=input_hash,
        input_node_refs=_refs([question_id]),
        provenance={'input_shape': 'causal_intervention', 'question_id': question_id},
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Evolution (quality-diversity archive)
# ---------------------------------------------------------------------------

def run_evolution_affordance(
    candidates: Sequence[Any] | Sequence[Mapping[str, Any]],
    *,
    elites_per_niche: int = 2,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project the quality-diversity archive selection as a substrate affordance."""

    from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
    from apps.notebook.inference_engines.evolution.native import NativeEvolutionEngine

    resolved: list[EvolutionCandidate] = []
    for candidate in candidates:
        if isinstance(candidate, EvolutionCandidate):
            resolved.append(candidate)
        else:
            resolved.append(EvolutionCandidate(
                candidate_id=str(candidate['candidate_id']),
                niche=str(candidate['niche']),
                score=float(candidate['score']),
                payload=dict(candidate.get('payload', {})),
                novelty=float(candidate.get('novelty', 0.0)),
            ))
    receipt = NativeEvolutionEngine().archive(resolved, elites_per_niche=elites_per_niche)
    input_hash = stable_hash({
        'candidates': [candidate.to_dict() for candidate in resolved],
        'elites_per_niche': elites_per_niche,
    })
    return _project(
        engine_id=str(receipt.to_dict()['engine']),
        affordance_id='evolution.archive',
        receipt=receipt,
        input_hash=input_hash,
        input_node_refs=_refs(candidate.candidate_id for candidate in resolved),
        provenance={'input_shape': 'evolution_candidates', 'candidate_count': len(resolved)},
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Proof (obligation tracking)
# ---------------------------------------------------------------------------

def run_proof_affordance(
    *,
    statement: str,
    target_system: str = 'lean',
    assumptions: Sequence[str] = (),
    obligation_metadata: Mapping[str, Any] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project a proof-obligation creation as a substrate affordance."""

    from apps.notebook.inference_engines.proof.engine import ProofEngine

    receipt = ProofEngine().create_obligation(
        statement=statement,
        target_system=target_system,
        assumptions=tuple(assumptions),
        metadata=dict(obligation_metadata or {}),
    )
    input_hash = stable_hash({
        'statement': statement,
        'target_system': target_system,
        'assumptions': list(assumptions),
        'obligation_metadata': dict(obligation_metadata or {}),
    })
    return _project(
        engine_id=str(receipt.to_dict()['engine']),
        affordance_id='proof.create_obligation',
        receipt=receipt,
        input_hash=input_hash,
        provenance={'input_shape': 'proof_obligation', 'target_system': target_system},
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Optimizer (constrained feasible selection)
# ---------------------------------------------------------------------------

def run_optimizer_affordance(
    problem: Any | Mapping[str, Any],
    *,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project a constrained optimization solve as a substrate affordance."""

    from apps.notebook.inference_engines.optimizer.contracts import (
        OptimizationCandidate,
        OptimizationProblem,
    )
    from apps.notebook.inference_engines.optimizer.engine import OptimizerEngine

    if isinstance(problem, OptimizationProblem):
        resolved = problem
    else:
        candidates = tuple(
            OptimizationCandidate(
                candidate_id=str(item['candidate_id']),
                value=float(item['value']),
                cost=float(item['cost']),
                tags=tuple(item.get('tags', ())),
                hard_required=bool(item.get('hard_required', False)),
                metadata=dict(item.get('metadata', {})),
            )
            for item in problem.get('candidates', ())
        )
        resolved = OptimizationProblem(
            problem_id=str(problem['problem_id']),
            objective=str(problem['objective']),
            candidates=candidates,
            budget=float(problem['budget']),
            min_tag_coverage=tuple(problem.get('min_tag_coverage', ())),
        )
    receipt = OptimizerEngine().optimize(resolved)
    return _project(
        engine_id=str(receipt.to_dict()['engine']),
        affordance_id='optimizer.optimize',
        receipt=receipt,
        input_hash=resolved.problem_hash,
        input_node_refs=_refs(candidate.candidate_id for candidate in resolved.candidates),
        provenance={
            'input_shape': 'optimization_problem',
            'problem_hash': resolved.problem_hash,
            'candidate_count': len(resolved.candidates),
        },
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Expression (structured result -> artifact)
# ---------------------------------------------------------------------------

def run_expression_affordance(
    result: Mapping[str, Any],
    *,
    engine_id: str = 'deterministic_brief',
    render_metadata: Mapping[str, Any] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project an expression render as a substrate affordance.

    Defaults to the deterministic_brief renderer so the projection is
    reproducible; the llm_speaker_fallback renderer is non-deterministic and is
    not appropriate for the Gate-0 parity surface.
    """

    from apps.notebook.inference_engines.expression.registry import get_expression_registry

    receipt = get_expression_registry().render(
        engine_id,
        dict(result),
        metadata=dict(render_metadata or {}),
    )
    input_hash = stable_hash({
        'engine_id': engine_id,
        'result': dict(result),
        'render_metadata': dict(render_metadata or {}),
    })
    return _project(
        engine_id=str(receipt.to_dict()['engine_id']),
        affordance_id=f'expression.{engine_id}',
        receipt=receipt,
        input_hash=input_hash,
        provenance={'input_shape': 'expression_result', 'renderer': engine_id},
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# E-graph (equivalence-preserving extraction)
# ---------------------------------------------------------------------------

def run_egraph_affordance(
    expression: Any | Mapping[str, Any],
    *,
    max_rounds: int = 8,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project equivalence-preserving extraction as a substrate affordance.

    Wraps the pure-Python EGraphTheorem reference engine. The native context_pack
    path is owned by the Rust-theorem lane and is intentionally not imported here.
    """

    from apps.notebook.inference_engines.egraph.contracts import EGraphExpression
    from apps.notebook.inference_engines.egraph.engine import EGraphTheorem

    if isinstance(expression, EGraphExpression):
        resolved = expression
    else:
        resolved = EGraphExpression(
            expression_id=str(expression['expression_id']),
            domain=str(expression['domain']),
            items=tuple(dict(item) for item in expression.get('items', ())),
            metadata=dict(expression.get('metadata', {})),
        )
    receipt = EGraphTheorem().extract(resolved, max_rounds=max_rounds)
    return _project(
        engine_id=str(receipt.to_dict()['engine']),
        affordance_id='egraph.extract',
        receipt=receipt,
        input_hash=resolved.expression_hash,
        input_node_refs=_refs([resolved.expression_id]),
        provenance={
            'input_shape': 'egraph_expression',
            'domain': resolved.domain,
            'expression_hash': resolved.expression_hash,
        },
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Simulation (auditable dry-run)
# ---------------------------------------------------------------------------

def run_simulation_affordance(
    *,
    validator: str,
    inputs: Mapping[str, Any],
    expected: Mapping[str, Any] | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project a validator dry-run as a substrate affordance."""

    from apps.notebook.inference_engines.simulation.engine import SimulationEngine

    receipt = SimulationEngine().dry_run(
        validator=validator,
        inputs=dict(inputs),
        expected=dict(expected) if expected is not None else None,
    )
    input_hash = stable_hash({
        'validator': validator,
        'inputs': dict(inputs),
        'expected': dict(expected or {}),
    })
    return _project(
        engine_id=getattr(SimulationEngine, 'engine', 'simulation-receipt-fallback'),
        affordance_id='simulation.dry_run',
        receipt=receipt,
        input_hash=input_hash,
        input_node_refs=_refs([validator]),
        provenance={'input_shape': 'simulation_dry_run', 'validator': validator},
        metadata=metadata,
    )


# ---------------------------------------------------------------------------
# Solver (SMT / constraint violation search)
# ---------------------------------------------------------------------------

def run_solver_affordance(
    problem: Any | Mapping[str, Any],
    *,
    timeout_ms: int | None = None,
    metadata: Mapping[str, Any] | None = None,
) -> AffordanceReceipt:
    """Project a solver violation search as a substrate affordance.

    Uses Z3Provider, which has a deterministic boolean fallback when the z3
    binary is unavailable, so the affordance is reproducible in any environment.
    Solver output proposes (never writes) canonical state, so the substrate
    writeback policy is proposal-only.
    """

    from apps.notebook.inference_engines.solver.contracts import (
        SolverConstraint,
        SolverProblem,
    )
    from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider

    if isinstance(problem, SolverProblem):
        resolved = problem
    else:
        constraints = tuple(
            SolverConstraint(
                constraint_id=str(item['constraint_id']),
                description=str(item.get('description', '')),
                violated=bool(item['violated']),
                counterexample=dict(item.get('counterexample', {})),
                severity=str(item.get('severity', 'error')),
            )
            for item in problem.get('constraints', ())
        )
        resolved = SolverProblem(
            target=str(problem['target']),
            constraints=constraints,
            input_view_refs=tuple(problem.get('input_view_refs', ())),
            metadata=dict(problem.get('metadata', {})),
        )
    receipt = Z3Provider().solve(resolved, timeout_ms=timeout_ms)
    return _project(
        engine_id=str(receipt.to_dict()['provider']),
        affordance_id='solver.check',
        receipt=receipt,
        input_hash=resolved.formula_hash,
        input_node_refs=_refs(resolved.input_view_refs),
        provenance={
            'input_shape': 'solver_problem',
            'formula_hash': resolved.formula_hash,
            'constraint_count': len(resolved.constraints),
        },
        metadata=metadata,
        writeback_policy='proposal-only',
    )


# Map of affordance_id -> public callable, for registry-style discovery by the
# benchmark differential and the MCP symbolic shim. datalog/probabilistic live
# in affordances.py; this registry covers the remaining eight engines.
ENGINE_AFFORDANCES = {
    'causal.intervention_effect': run_causal_affordance,
    'evolution.archive': run_evolution_affordance,
    'proof.create_obligation': run_proof_affordance,
    'optimizer.optimize': run_optimizer_affordance,
    'expression.render': run_expression_affordance,
    'egraph.extract': run_egraph_affordance,
    'simulation.dry_run': run_simulation_affordance,
    'solver.check': run_solver_affordance,
}


# ---------------------------------------------------------------------------
# Differential-ready parity cases (for the offload-lane Gate-0 fold-in)
# ---------------------------------------------------------------------------
#
# Each case carries a representative input chosen so the engine produces a
# NON-TRIVIAL receipt, not an empty one. This is the lesson from the datalog
# civic-rule gate: a differential that runs the engines on inputs that derive
# nothing passes on empty==empty and proves nothing. The offload lane should
# import these validated cases rather than re-deriving inputs that might not
# fire each engine.

def engine_affordance_differential_cases() -> list[dict[str, Any]]:
    """Representative firing inputs for the eight engines, each run through both
    the affordance projection and the direct engine.

    Returns one dict per engine: {affordance_id, affordance_receipt (AffordanceReceipt),
    direct_receipt (dict)}. The Gate-0 invariant is affordance_receipt.payload ==
    direct_receipt; both call the same engine adapter on the same firing input.
    """

    from apps.notebook.inference_engines.causal.engine import CausalEngine
    from apps.notebook.inference_engines.egraph.contracts import EGraphExpression
    from apps.notebook.inference_engines.egraph.engine import EGraphTheorem
    from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
    from apps.notebook.inference_engines.evolution.native import NativeEvolutionEngine
    from apps.notebook.inference_engines.optimizer.contracts import (
        OptimizationCandidate,
        OptimizationProblem,
    )
    from apps.notebook.inference_engines.optimizer.engine import OptimizerEngine
    from apps.notebook.inference_engines.proof.engine import ProofEngine
    from apps.notebook.inference_engines.expression.registry import get_expression_registry
    from apps.notebook.inference_engines.simulation.engine import SimulationEngine
    from apps.notebook.inference_engines.solver.contracts import SolverConstraint, SolverProblem
    from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider

    cases: list[dict[str, Any]] = []

    # causal: treated/control given -> identified estimate (not 'unknown').
    cases.append({
        'affordance_id': 'causal.intervention_effect',
        'affordance_receipt': run_causal_affordance(
            question_id='q-diff', treatment='exposure', outcome='recovery',
            treated_mean=2.0, control_mean=1.0,
        ),
        'direct_receipt': CausalEngine().intervention_effect(
            question_id='q-diff', treatment='exposure', outcome='recovery',
            treated_mean=2.0, control_mean=1.0,
        ).to_dict(),
    })

    # evolution: two niches -> non-empty elites_by_niche.
    evo_candidates = [
        EvolutionCandidate(candidate_id='c1', niche='n1', score=0.9, payload={'k': 1}, novelty=0.2),
        EvolutionCandidate(candidate_id='c2', niche='n1', score=0.4, payload={}, novelty=0.1),
        EvolutionCandidate(candidate_id='c3', niche='n2', score=0.7, payload={'k': 2}),
    ]
    cases.append({
        'affordance_id': 'evolution.archive',
        'affordance_receipt': run_evolution_affordance(evo_candidates, elites_per_niche=2),
        'direct_receipt': NativeEvolutionEngine().archive(evo_candidates, elites_per_niche=2).to_dict(),
    })

    # proof: obligation created.
    cases.append({
        'affordance_id': 'proof.create_obligation',
        'affordance_receipt': run_proof_affordance(
            statement='forall x: safe(x)', target_system='lean', assumptions=('well_typed',),
        ),
        'direct_receipt': ProofEngine().create_obligation(
            statement='forall x: safe(x)', target_system='lean', assumptions=('well_typed',), metadata={},
        ).to_dict(),
    })

    # optimizer: feasible problem -> non-empty selection.
    opt_problem = OptimizationProblem(
        problem_id='p-diff', objective='max_value',
        candidates=(
            OptimizationCandidate(candidate_id='a', value=5.0, cost=2.0, tags=('core',), hard_required=True),
            OptimizationCandidate(candidate_id='b', value=3.0, cost=1.0, tags=('aux',)),
            OptimizationCandidate(candidate_id='c', value=4.0, cost=3.0, tags=('aux',)),
        ),
        budget=4.0, min_tag_coverage=('aux',),
    )
    cases.append({
        'affordance_id': 'optimizer.optimize',
        'affordance_receipt': run_optimizer_affordance(opt_problem),
        'direct_receipt': OptimizerEngine().optimize(opt_problem).to_dict(),
    })

    # expression: deterministic brief over a real result.
    expr_result = {'status': 'feasible', 'engine': 'optimizer', 'receipt_hash': 'abc123'}
    cases.append({
        'affordance_id': 'expression.render',
        'affordance_receipt': run_expression_affordance(expr_result, engine_id='deterministic_brief'),
        'direct_receipt': get_expression_registry().render(
            'deterministic_brief', dict(expr_result), metadata={},
        ).to_dict(),
    })

    # egraph: two identical context_pack items -> a dedupe rewrite fires.
    egraph_expression = EGraphExpression(
        expression_id='ctx-diff', domain='context_pack',
        items=(
            {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
            {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
        ),
    )
    cases.append({
        'affordance_id': 'egraph.extract',
        'affordance_receipt': run_egraph_affordance(egraph_expression, max_rounds=8),
        'direct_receipt': EGraphTheorem().extract(egraph_expression, max_rounds=8).to_dict(),
    })

    # simulation: dry-run with a matching expectation -> status passed.
    sim_inputs = {'status': 'ok', 'count': 3}
    cases.append({
        'affordance_id': 'simulation.dry_run',
        'affordance_receipt': run_simulation_affordance(
            validator='schema_check', inputs=sim_inputs, expected={'status': 'ok'},
        ),
        'direct_receipt': SimulationEngine().dry_run(
            validator='schema_check', inputs=sim_inputs, expected={'status': 'ok'},
        ).to_dict(),
    })

    # solver: a violated constraint -> status sat with a counterexample.
    solver_problem = SolverProblem(
        target='no_private_export',
        constraints=(
            SolverConstraint(constraint_id='priv-1', description='private source reaches export', violated=True),
        ),
        input_view_refs=('view:export-candidates',),
    )
    cases.append({
        'affordance_id': 'solver.check',
        'affordance_receipt': run_solver_affordance(solver_problem),
        'direct_receipt': Z3Provider().solve(solver_problem).to_dict(),
    })

    return cases


def _engine_cost_thunks() -> list[tuple[str, Any]]:
    """Zero-arg thunks that re-run each engine affordance on its representative
    input, for repeatable CPU-cost measurement. Inputs mirror the differential
    cases so cost is measured on the same firing workload that parity verifies.
    """

    from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
    from apps.notebook.inference_engines.egraph.contracts import EGraphExpression
    from apps.notebook.inference_engines.optimizer.contracts import (
        OptimizationCandidate,
        OptimizationProblem,
    )
    from apps.notebook.inference_engines.solver.contracts import SolverConstraint, SolverProblem

    evo_candidates = [
        EvolutionCandidate(candidate_id='c1', niche='n1', score=0.9, payload={'k': 1}, novelty=0.2),
        EvolutionCandidate(candidate_id='c2', niche='n1', score=0.4, payload={}, novelty=0.1),
        EvolutionCandidate(candidate_id='c3', niche='n2', score=0.7, payload={'k': 2}),
    ]
    opt_problem = OptimizationProblem(
        problem_id='p-cost', objective='max_value',
        candidates=(
            OptimizationCandidate(candidate_id='a', value=5.0, cost=2.0, tags=('core',), hard_required=True),
            OptimizationCandidate(candidate_id='b', value=3.0, cost=1.0, tags=('aux',)),
            OptimizationCandidate(candidate_id='c', value=4.0, cost=3.0, tags=('aux',)),
        ),
        budget=4.0, min_tag_coverage=('aux',),
    )
    egraph_expression = EGraphExpression(
        expression_id='ctx-cost', domain='context_pack',
        items=(
            {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
            {'channel': 'read_first', 'obligation': 'cite_source', 'optional': False},
        ),
    )
    solver_problem = SolverProblem(
        target='no_private_export',
        constraints=(
            SolverConstraint(constraint_id='priv-1', description='private source reaches export', violated=True),
        ),
        input_view_refs=('view:export-candidates',),
    )

    return [
        ('causal.intervention_effect', lambda: run_causal_affordance(
            question_id='q-cost', treatment='exposure', outcome='recovery',
            treated_mean=2.0, control_mean=1.0)),
        ('evolution.archive', lambda: run_evolution_affordance(evo_candidates, elites_per_niche=2)),
        ('proof.create_obligation', lambda: run_proof_affordance(
            statement='forall x: safe(x)', target_system='lean', assumptions=('well_typed',))),
        ('optimizer.optimize', lambda: run_optimizer_affordance(opt_problem)),
        ('expression.render', lambda: run_expression_affordance(
            {'status': 'feasible', 'engine': 'optimizer', 'receipt_hash': 'abc123'},
            engine_id='deterministic_brief')),
        ('egraph.extract', lambda: run_egraph_affordance(egraph_expression, max_rounds=8)),
        ('simulation.dry_run', lambda: run_simulation_affordance(
            validator='schema_check', inputs={'status': 'ok', 'count': 3}, expected={'status': 'ok'})),
        ('solver.check', lambda: run_solver_affordance(solver_problem)),
    ]


def measure_affordance_cost(*, iterations: int = 200) -> list[dict[str, Any]]:
    """Measure CPU cost per engine affordance over `iterations` runs.

    Returns one row per engine: {affordance_id, iterations, cpu_seconds_total,
    cpu_us_per_call}. CPU time (time.process_time) is the relevant economic
    measure for the offload thesis: it is what running the symbolic path costs,
    excluding wall-clock sleep/IO. A warmup call absorbs first-run import/JIT
    cost so the measured cost reflects steady-state execution.

    This is the symbolic-side half of the offload cost claim (CPU symbolic vs
    GPU inference); the GPU side and the comparison live in the offload lane's
    CO-1 ledger. Timing is observational, not a parity assertion.
    """

    import time

    rows: list[dict[str, Any]] = []
    for affordance_id, thunk in _engine_cost_thunks():
        thunk()  # warmup
        start = time.process_time()
        for _ in range(max(1, iterations)):
            thunk()
        elapsed = time.process_time() - start
        rows.append({
            'affordance_id': affordance_id,
            'iterations': iterations,
            'cpu_seconds_total': elapsed,
            'cpu_us_per_call': (elapsed / max(1, iterations)) * 1_000_000.0,
        })
    return rows


def run_engine_affordance_parity() -> dict[str, Any]:
    """Self-contained Gate-0 projection-fidelity report over the eight engines.

    Mirrors the shape of benchmarks/datalog_derivation_parity.run_datalog_derivation_parity
    so the offload lane can fold it into gate0 with the same record-writing loop.
    """

    per_engine: list[dict[str, Any]] = []
    failures: list[str] = []
    for case in engine_affordance_differential_cases():
        affordance_id = case['affordance_id']
        affordance_receipt: AffordanceReceipt = case['affordance_receipt']
        equal = affordance_receipt.payload == case['direct_receipt']
        per_engine.append({
            'affordance_id': affordance_id,
            'equal': equal,
            'receipt_hash': affordance_receipt.receipt_hash,
        })
        if not equal:
            failures.append(affordance_id)
    return {
        'passed': not failures,
        'engine_count': len(per_engine),
        'per_engine': per_engine,
        'failures': failures,
    }
