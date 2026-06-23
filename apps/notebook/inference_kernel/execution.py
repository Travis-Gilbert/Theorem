"""Executable BGI inference-kernel routing surface."""

from __future__ import annotations

from typing import Any

from django.utils import timezone

from apps.notebook.inference_engines.common import stable_hash
from apps.notebook.inference_engines.datalog.contracts import (
    DatalogFact,
    fact_pack_from_iterable,
)
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.datalog.native import (
    NativeDatalogEngine,
    native_datalog_can_handle,
)
from apps.notebook.inference_engines.egraph.engine import EGraphTheorem
from apps.notebook.inference_engines.egraph.native import NativeEGraphTheorem
from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.engine import EvolutionEngine
from apps.notebook.inference_engines.evolution.native import (
    NativeEvolutionEngine,
    native_evolution_can_handle,
)
from apps.notebook.inference_engines.proof.engine import ProofEngine
from apps.notebook.inference_engines.probabilistic.native import (
    NativeProbProgEngine,
    native_probabilistic_can_handle,
)
from apps.notebook.inference_engines.simulation.engine import SimulationEngine
from apps.notebook.inference_engines.solver.constraint_builders.context_capsule import (
    build_context_capsule_problem,
)
from apps.notebook.inference_engines.solver.constraint_builders.graph_patch import (
    build_graph_patch_problem,
)
from apps.notebook.inference_engines.solver.providers.alloy_provider import AlloyProvider
from apps.notebook.inference_engines.solver.providers.cvc5_provider import CVC5Provider
from apps.notebook.inference_engines.solver.providers.z3_provider import Z3Provider
from apps.notebook.inference_engines.runtime_adapters import (
    causal_effect_from_observation_groups,
    expected_value_from_validator_records,
    source_reliability_from_records,
    validator_schedule_from_records,
)
from apps.notebook.ingestion_investigation.engine import IngestionInvestigationEngine
from apps.notebook.models import KernelRun
from apps.notebook.discovery_runs.archives import archive_candidates

from .persistence import (
    append_kernel_receipt,
    begin_kernel_run,
    finish_kernel_run,
    render_kernel_run,
)
from .native_strategy import NativeFeatureSpec, native_enabled
from .registry import resolve
from .router import route_kernel
from .settings import bgi_kernel_runs_enabled, bgi_native_symbolic_enabled


def _provider(provider_id: str):
    provider = str(provider_id or 'z3').lower()
    if provider == 'alloy':
        return AlloyProvider()
    if provider == 'cvc5':
        return CVC5Provider()
    return Z3Provider()


def _facts_from_payload(payload: dict[str, Any]):
    facts = payload.get('facts')
    if isinstance(facts, list):
        return fact_pack_from_iterable(
            DatalogFact(
                relation=str(item['relation']),
                entity_id=str(item['entity_id']),
                attributes=dict(item.get('attributes') or {}),
                source_ref=str(item.get('source_ref') or ''),
            )
            for item in facts
        )
    return build_fact_pack_from_records(
        objects=payload.get('objects') or (),
        claims=payload.get('claims') or (),
        edges=payload.get('edges') or (),
        evidence_paths=payload.get('evidence_paths') or (),
        context_atoms=payload.get('context_atoms') or (),
        source=str(payload.get('source') or 'kernel-run'),
    )


def _native_requested(payload: dict[str, Any]) -> bool:
    return bool(payload.get('native', True))


def _native_symbolic_feature_enabled(
    *,
    feature_id: str,
    python_fallback: str,
    native_module: str,
    parity_tests: tuple[str, ...],
) -> bool:
    if not bgi_native_symbolic_enabled():
        return False
    feature = NativeFeatureSpec(
        feature_id=feature_id,
        python_fallback=python_fallback,
        native_module=native_module,
        parity_tests=parity_tests,
    )
    return native_enabled(feature)


def _dispatch(kernel_id: str, payload: dict[str, Any]) -> dict[str, Any]:
    if kernel_id == 'bgi_context_capsule_solver':
        problem = build_context_capsule_problem(
            capsule=dict(payload.get('capsule') or {}),
            budget_tokens=int(payload.get('budget_tokens') or 0),
            token_ledger=dict(payload.get('token_ledger') or {}),
            atoms=list(payload.get('atoms') or []),
            exports=dict(payload.get('exports') or {}),
            input_view_refs=tuple(payload.get('input_view_refs') or ()),
        )
        return _provider(str(payload.get('provider') or 'z3')).solve(
            problem,
            timeout_ms=payload.get('timeout_ms'),
        ).to_dict()

    if kernel_id == 'bgi_graph_patch_solver':
        problem = build_graph_patch_problem(
            patch=dict(payload.get('patch') or {}),
            input_view_refs=tuple(payload.get('input_view_refs') or ()),
        )
        return _provider(str(payload.get('provider') or 'z3')).solve(
            problem,
            timeout_ms=payload.get('timeout_ms'),
        ).to_dict()

    if kernel_id == 'bgi_datalog_deriver':
        fact_pack = _facts_from_payload(payload)
        rule_ids = payload.get('rule_ids') or None
        use_native = (
            _native_requested(payload)
            and native_datalog_can_handle(rule_ids)
            and _native_symbolic_feature_enabled(
                feature_id='bgi_datalog_deriver',
                python_fallback='apps.notebook.inference_engines.datalog.engine.DatalogEngine',
                native_module='theseus_native.bgi_datalog_derive_core_json',
                parity_tests=(
                    'apps/notebook/benchmarks/bgi_native_parity.py',
                    'rustyredcore_THG/tests/test_bgi_parity.py',
                ),
            )
        )
        engine = NativeDatalogEngine() if use_native else DatalogEngine()
        return engine.derive(
            fact_pack,
            rule_ids=rule_ids,
        ).to_dict()

    if kernel_id == 'bgi_egraph_optimizer':
        use_native = _native_requested(payload) and _native_symbolic_feature_enabled(
            feature_id='bgi_egraph_optimizer',
            python_fallback='apps.notebook.inference_engines.egraph.engine.EGraphTheorem',
            native_module='theseus_native.bgi_egraph_extract_context_pack_json',
            parity_tests=('apps/notebook/benchmarks/bgi_native_parity.py',),
        )
        engine = NativeEGraphTheorem() if use_native else EGraphTheorem()
        return engine.context_pack(
            expression_id=str(payload.get('expression_id') or f'expr-{stable_hash(payload)[:12]}'),
            items=list(payload.get('items') or []),
            cost_config=dict(payload.get('cost_config') or {}),
        ).to_dict()

    if kernel_id == 'bgi_probabilistic_source_reliability':
        mode = str(payload.get('mode') or 'source_reliability')
        method = 'expected_value_of_information' if mode == 'expected_value' else 'source_reliability'
        use_native = (
            _native_requested(payload)
            and native_probabilistic_can_handle(method)
            and _native_symbolic_feature_enabled(
                feature_id='bgi_probabilistic_source_reliability',
                python_fallback='apps.notebook.inference_engines.probabilistic.engine.ProbProgEngine',
                native_module='theseus_native.bgi_probabilistic_*',
                parity_tests=(
                    'apps/notebook/benchmarks/bgi_native_parity.py',
                    'rustyredcore_THG/tests/test_bgi_parity.py',
                ),
            )
        )
        prob_engine = NativeProbProgEngine() if use_native else None
        if payload.get('mode') == 'expected_value':
            return expected_value_from_validator_records(
                validator_records=payload.get('validator_records') or [],
                decision_value=float(payload.get('decision_value') or 1.0),
                engine=prob_engine,
            )
        return source_reliability_from_records(
            source_id=str(payload.get('source_id') or 'source'),
            evidence_records=payload.get('evidence_records') or [],
            prior_alpha=float(payload.get('prior_alpha') or 1.0),
            prior_beta=float(payload.get('prior_beta') or 1.0),
            engine=prob_engine,
        )

    if kernel_id == 'bgi_causal_assumption_engine':
        return causal_effect_from_observation_groups(
            question_id=str(payload.get('question_id') or 'causal-question'),
            treatment=str(payload.get('treatment') or 'treatment'),
            outcome=str(payload.get('outcome') or 'outcome_value'),
            treated_records=payload.get('treated_records') or [],
            control_records=payload.get('control_records') or [],
            assumptions=payload.get('assumptions') or (),
            confounders=payload.get('confounders') or (),
        )

    if kernel_id == 'bgi_validator_scheduler':
        return validator_schedule_from_records(
            validator_records=payload.get('validator_records') or payload.get('validators') or [],
            budget=float(payload.get('budget') or 0.0),
        )

    if kernel_id == 'bgi_candidate_archive':
        if payload.get('persist'):
            return archive_candidates(
                candidates=list(payload.get('candidates') or []),
                run_id=str(payload.get('discovery_run_id') or ''),
                elites_per_niche=int(payload.get('elites_per_niche') or 2),
            )
        candidates = [
            EvolutionCandidate(
                candidate_id=str(item['candidate_id']),
                niche=str(item.get('niche') or 'default'),
                score=float(item.get('score') or 0.0),
                novelty=float(item.get('novelty') or 0.0),
                payload=dict(item.get('payload') or item),
            )
            for item in payload.get('candidates') or []
        ]
        use_native = (
            _native_requested(payload)
            and native_evolution_can_handle()
            and _native_symbolic_feature_enabled(
                feature_id='bgi_evolution_archive',
                python_fallback='apps.notebook.inference_engines.evolution.engine.EvolutionEngine',
                native_module='theseus_native.bgi_evolution_archive_json',
                parity_tests=(
                    'apps/notebook/benchmarks/bgi_native_parity.py',
                    'rustyredcore_THG/tests/test_bgi_parity.py',
                    'rustyredcore_THG/tests/test_datalog_derivation_parity.py',
                ),
            )
        )
        engine = NativeEvolutionEngine() if use_native else EvolutionEngine()
        return engine.archive(
            candidates,
            elites_per_niche=int(payload.get('elites_per_niche') or 2),
        ).to_dict()

    if kernel_id == 'bgi_proof_obligation_tracker':
        return ProofEngine().create_obligation(
            statement=str(payload.get('statement') or payload.get('hypothesis') or ''),
            target_system=str(payload.get('target_system') or 'lean'),
            assumptions=tuple(payload.get('assumptions') or ()),
            metadata=dict(payload.get('metadata') or {}),
        ).to_dict()

    if kernel_id == 'bgi_simulation_validator':
        return SimulationEngine().dry_run(
            validator=str(payload.get('validator') or 'simulation'),
            inputs=dict(payload.get('inputs') or {}),
            expected=dict(payload.get('expected') or {}),
        ).to_dict()

    if kernel_id == 'bgi_ingestion_investigation':
        return IngestionInvestigationEngine().evaluate_batch(
            sources=payload.get('sources') or [dict(payload.get('source') or payload)],
            query=str(payload.get('query') or ''),
        ).to_dict()

    contract = resolve(kernel_id)
    return {
        'status': 'routed',
        'kernel': contract.to_dict() if contract else {'kernel_id': kernel_id},
        'writeback_policy': contract.writeback_policy if contract else 'read-only',
    }


def run_kernel(
    *,
    payload: dict[str, Any],
    kernel_id: str | None = None,
    epistemic_job: str | None = None,
    inference_family: str | None = None,
    consumes_view: str | None = None,
    discovery_run_id: str | None = None,
    budget: dict[str, Any] | None = None,
    metadata: dict[str, Any] | None = None,
) -> dict[str, Any]:
    """Route, execute, persist, and render one KernelRun."""

    if not bgi_kernel_runs_enabled():
        raise RuntimeError('BGI kernel runs are disabled by THESEUS_BGI_KERNEL_RUNS_ENABLED.')

    contract = route_kernel(
        kernel_id=kernel_id,
        epistemic_job=epistemic_job,
        inference_family=inference_family,
        consumes_view=consumes_view,
    )
    if contract is None:
        raise ValueError('No inference kernel matched the request.')

    started_at = timezone.now()
    kernel_run = begin_kernel_run(
        contract,
        request_payload=dict(payload or {}),
        budget=budget,
        metadata=metadata,
        discovery_run_id=discovery_run_id,
    )
    try:
        result_payload = _dispatch(contract.kernel_id, dict(payload or {}))
        append_kernel_receipt(
            kernel_run,
            receipt_type=contract.inference_family,
            payload=result_payload,
            validator_id=contract.validator,
            writeback_proposals=list(result_payload.get('writeback_proposals') or []),
        )
        finish_kernel_run(
            kernel_run,
            result_payload=result_payload,
            started_at=started_at,
            status=KernelRun.Status.SUCCEEDED,
        )
    except Exception as exc:
        error_payload = {'error': str(exc), 'error_type': exc.__class__.__name__}
        append_kernel_receipt(
            kernel_run,
            receipt_type=contract.inference_family,
            payload=error_payload,
            status='failed',
            validator_id=contract.validator,
        )
        finish_kernel_run(
            kernel_run,
            result_payload={},
            started_at=started_at,
            status=KernelRun.Status.FAILED,
            error_payload=error_payload,
        )
    return render_kernel_run(KernelRun.objects.get(pk=kernel_run.pk))
