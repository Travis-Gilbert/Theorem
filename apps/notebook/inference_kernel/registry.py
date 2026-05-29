"""In-memory inference-kernel registry.

This registry is intentionally additive: call-sites can safely introspect and
route through it once later stages need the registry as a hard dependency.
"""

from __future__ import annotations

from typing import Any, Iterable

from .contracts import (
    InferenceKernelContract,
    KNOWN_CONSUMES_VIEWS,
    KNOWN_EPISTEMIC_JOBS,
    KNOWN_INFERENCE_FAMILIES,
    KNOWN_PRODUCES,
    KNOWN_VALIDATORS,
    KNOWN_WRITEBACK_POLICIES,
)


DEFAULT_REGISTRY_VERSION = '2026.05.01'


def _registry_definition() -> list[InferenceKernelContract]:
    return [
        InferenceKernelContract(
            kernel_id='spacy_entity_extractor',
            epistemic_job='ingest',
            inference_family='lexical',
            consumes_view=('text',),
            produces=('claim', 'edge'),
            truth_type='plausibility',
            validator='benchmark',
            writeback_policy='review-required',
            source_module='apps.notebook.engine',
            owner='engine',
            description='spaCy entity extraction pass that generates relational anchors.',
            source='engine',
            tags=('engine', 'entities', 'pass-1'),
        ),
        InferenceKernelContract(
            kernel_id='bm25_tfidf_search',
            epistemic_job='structure',
            inference_family='lexical',
            consumes_view=('text',),
            produces=('score', 'edge'),
            truth_type='relevance',
            validator='benchmark',
            writeback_policy='review-required',
            source_module='apps.notebook.engine',
            owner='engine',
            description='TF-IDF and BM25 candidate ranking.',
            source='engine',
            tags=('engine', 'pass-4', 'tfidf'),
        ),
        InferenceKernelContract(
            kernel_id='sbert_embedding_search',
            epistemic_job='structure',
            inference_family='neural',
            consumes_view=('text',),
            produces=('score', 'claim'),
            truth_type='plausibility',
            validator='benchmark',
            writeback_policy='review-required',
            source_module='apps.notebook.engine',
            owner='engine',
            description='SBERT semantic candidate retrieval and rerank helpers.',
            source='engine',
            tags=('engine', 'pass-5', 'sbert'),
        ),
        InferenceKernelContract(
            kernel_id='nli_contradiction_check',
            epistemic_job='evaluate',
            inference_family='deductive',
            consumes_view=('claims', 'text'),
            produces=('claim', 'score', 'counterexample'),
            truth_type='plausibility',
            validator='human review',
            writeback_policy='review-required',
            source_module='apps.notebook.nli',
            owner='engine',
            description='Pass-6 entailment and contradiction detection.',
            source='engine',
            tags=('engine', 'pass-6', 'nli'),
        ),
        InferenceKernelContract(
            kernel_id='ppr_pagerank_graph',
            epistemic_job='structure',
            inference_family='graph',
            consumes_view=('graph',),
            produces=('score', 'edge'),
            truth_type='plausibility',
            validator='benchmark',
            writeback_policy='review-required',
            source_module='apps.notebook.sparse_ppr',
            owner='engine',
            description='PageRank/Personalized PageRank ranking pass.',
            source='engine',
            tags=('engine', 'pagerank', 'graph-pass'),
        ),
        InferenceKernelContract(
            kernel_id='gnn_kge_spacetime',
            epistemic_job='relate',
            inference_family='neural',
            consumes_view=('graph', 'text'),
            produces=('score', 'edge', 'claim'),
            truth_type='plausibility',
            validator='benchmark',
            writeback_policy='proposal-only',
            source_module='apps.notebook.gnn_engine',
            owner='engine',
            description='GNN/KGE/spacetime structural inference family.',
            source='engine',
            tags=('engine', 'pass-7', 'gnn', 'kge', 'spacetime'),
        ),
        InferenceKernelContract(
            kernel_id='learned_scorer',
            epistemic_job='evaluate',
            inference_family='neural',
            consumes_view=('graph', 'claims', 'text'),
            produces=('score',),
            truth_type='plausibility',
            validator='benchmark',
            writeback_policy='proposal-only',
            source_module='apps.notebook.learned_scorer',
            owner='ranking',
            description='Learned rank model that produces proposal scores.',
            source='engine',
            tags=('engine', 'ranking', 'learned'),
        ),
        InferenceKernelContract(
            kernel_id='tms_belief_revision',
            epistemic_job='evaluate',
            inference_family='deductive',
            consumes_view=('claims', 'constraints'),
            produces=('claim', 'counterexample', 'score'),
            truth_type='plausibility',
            validator='source corroboration',
            writeback_policy='review-required',
            source_module='apps.notebook.tms',
            owner='tms',
            description='TMS and belief-revision propagation for claims.',
            source='context',
            tags=('tms', 'belief-revision', 'pipeline'),
        ),
        InferenceKernelContract(
            kernel_id='context_web_packer',
            epistemic_job='ingest',
            inference_family='expression',
            consumes_view=('text', 'graph', 'claims'),
            produces=('artifact', 'score'),
            truth_type='relevance',
            validator='source corroboration',
            writeback_policy='proposal-only',
            source_module='apps.notebook.context_compiler',
            owner='context',
            description='Context-Web packer that constructs task-specific evidence capsules.',
            source='pipeline',
            tags=('context-web', 'artifact'),
        ),
        InferenceKernelContract(
            kernel_id='search_kernel',
            epistemic_job='relate',
            inference_family='graph',
            consumes_view=('text', 'trace'),
            produces=('artifact', 'edge', 'score'),
            truth_type='relevance',
            validator='source corroboration',
            writeback_policy='read-only',
            source_module='apps.notebook.search_kernel',
            owner='search',
            description='Ask-time search kernel that augments personal graph evidence.',
            source='search',
            tags=('search', 'ask', 'browser'),
        ),
        InferenceKernelContract(
            kernel_id='scene_os_compiler',
            epistemic_job='express',
            inference_family='expression',
            consumes_view=('claims', 'constraints', 'trace'),
            produces=('scene', 'artifact'),
            truth_type='feasibility',
            validator='simulation',
            writeback_policy='proposal-only',
            source_module='apps.notebook.scene_os',
            owner='scene-os',
            description='Scene OS compiler that turns structured inference into scenes.',
            source='scene-os',
            tags=('scene-os', 'scene', 'expression'),
        ),
        InferenceKernelContract(
            kernel_id='orchestrate_toolgraph',
            epistemic_job='structure',
            inference_family='planner',
            consumes_view=('constraints', 'trace', 'text'),
            produces=('plan', 'artifact'),
            truth_type='feasibility',
            validator='human review',
            writeback_policy='read-only',
            source_module='apps.orchestrate.runtime.toolgraph',
            owner='orchestrate',
            description='ToolGraph planner that returns a constrained tool execution plan.',
            source='orchestrate',
            tags=('orchestrate', 'toolgraph'),
        ),
        InferenceKernelContract(
            kernel_id='thg_command_router',
            epistemic_job='act',
            inference_family='planner',
            consumes_view=('graph', 'constraints', 'trace'),
            produces=('edge', 'artifact', 'action'),
            truth_type='causality',
            validator='human review',
            writeback_policy='review-required',
            source_module='apps.notebook.graph_kernel.rustyred_thg',
            owner='thg',
            description='THG command surface used for graph-graph operations and state queries.',
            source='thg',
            tags=('thg', 'command', 'runtime'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_context_capsule_solver',
            epistemic_job='validate',
            inference_family='constraint',
            consumes_view=('constraints', 'claims'),
            produces=('counterexample', 'proof'),
            truth_type='feasibility',
            validator='proof',
            writeback_policy='proposal-only',
            source_module='apps.notebook.inference_engines.solver',
            owner='bgi',
            description='Context-capsule policy and export safety solver receipt.',
            source='bgi',
            tags=('bgi', 'solver', 'context-capsule'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_graph_patch_solver',
            epistemic_job='validate',
            inference_family='constraint',
            consumes_view=('graph', 'constraints'),
            produces=('counterexample', 'proof'),
            truth_type='feasibility',
            validator='proof',
            writeback_policy='proposal-only',
            source_module='apps.notebook.inference_engines.solver.constraint_builders.graph_patch',
            owner='bgi',
            description='Graph patch safety solver for canonical truth-field rewrites.',
            source='bgi',
            tags=('bgi', 'solver', 'graph-patch'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_datalog_deriver',
            epistemic_job='evaluate',
            inference_family='deductive',
            consumes_view=('graph', 'claims'),
            produces=('claim', 'counterexample'),
            truth_type='consequence',
            validator='test',
            writeback_policy='read-only',
            source_module='apps.notebook.inference_engines.datalog',
            owner='bgi',
            description='Read-only symbolic consequence derivation over normalized fact packs.',
            source='bgi',
            tags=('bgi', 'datalog', 'facts'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_egraph_optimizer',
            epistemic_job='structure',
            inference_family='egraph',
            consumes_view=('egraph expression', 'text'),
            produces=('artifact', 'score'),
            truth_type='equivalence',
            validator='test',
            writeback_policy='read-only',
            source_module='apps.notebook.inference_engines.egraph',
            owner='bgi',
            description='Equivalence-preserving expression and context-pack extraction.',
            source='bgi',
            tags=('bgi', 'egraph', 'native'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_probabilistic_source_reliability',
            epistemic_job='evaluate',
            inference_family='probabilistic',
            consumes_view=('claims', 'trace'),
            produces=('posterior', 'score'),
            truth_type='probability',
            validator='benchmark',
            writeback_policy='proposal-only',
            source_module='apps.notebook.inference_engines.probabilistic',
            owner='bgi',
            description='Posterior source reliability and expected-value receipts from real evidence records.',
            source='bgi',
            tags=('bgi', 'probabilistic', 'evi'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_causal_assumption_engine',
            epistemic_job='evaluate',
            inference_family='causal',
            consumes_view=('claims', 'constraints'),
            produces=('posterior', 'counterexample'),
            truth_type='causality',
            validator='human review',
            writeback_policy='proposal-only',
            source_module='apps.notebook.inference_engines.causal',
            owner='bgi',
            description='Assumption-bound causal estimates from treated/control summaries.',
            source='bgi',
            tags=('bgi', 'causal', 'assumptions'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_validator_scheduler',
            epistemic_job='validate',
            inference_family='optimizer',
            consumes_view=('constraints', 'trace'),
            produces=('plan', 'score'),
            truth_type='feasibility',
            validator='test',
            writeback_policy='read-only',
            source_module='apps.notebook.inference_engines.optimizer',
            owner='bgi',
            description='Budget-aware validator and context scheduling optimizer.',
            source='bgi',
            tags=('bgi', 'optimizer', 'validators'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_candidate_archive',
            epistemic_job='revise',
            inference_family='evolutionary',
            consumes_view=('trace', 'claims'),
            produces=('artifact', 'score'),
            truth_type='empirical result',
            validator='simulation',
            writeback_policy='proposal-only',
            source_module='apps.notebook.inference_engines.evolution',
            owner='bgi',
            description='Quality-diversity archive for discovery candidates.',
            source='bgi',
            tags=('bgi', 'evolution', 'archive'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_proof_obligation_tracker',
            epistemic_job='validate',
            inference_family='proof',
            consumes_view=('constraints', 'claims'),
            produces=('proof', 'counterexample'),
            truth_type='proof',
            validator='proof',
            writeback_policy='read-only',
            source_module='apps.notebook.inference_engines.proof',
            owner='bgi',
            description='Proof-obligation receipt tracker for theorem-backed validators.',
            source='bgi',
            tags=('bgi', 'proof', 'validators'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_simulation_validator',
            epistemic_job='validate',
            inference_family='simulator',
            consumes_view=('trace', 'constraints'),
            produces=('counterexample', 'score'),
            truth_type='empirical result',
            validator='simulation',
            writeback_policy='read-only',
            source_module='apps.notebook.inference_engines.simulation',
            owner='bgi',
            description='Auditable dry-run simulation validator.',
            source='bgi',
            tags=('bgi', 'simulation', 'validators'),
        ),
        InferenceKernelContract(
            kernel_id='bgi_ingestion_investigation',
            epistemic_job='ingest',
            inference_family='expression',
            consumes_view=('text', 'trace'),
            produces=('artifact', 'score'),
            truth_type='relevance',
            validator='source corroboration',
            writeback_policy='proposal-only',
            source_module='apps.notebook.ingestion_investigation',
            owner='bgi',
            description='Source-as-investigation admission receipts over Search Kernel and WebDoc candidates.',
            source='bgi',
            tags=('bgi', 'ingestion', 'search-kernel'),
        ),
    ]


class _InferenceKernelRegistry:
    def __init__(self, entries: Iterable[InferenceKernelContract] | None = None) -> None:
        self._entries: dict[str, InferenceKernelContract] = {}
        for entry in entries or ():
            self.register(entry)

    def register(self, contract: InferenceKernelContract) -> None:
        self._entries[contract.kernel_id] = contract

    def get(self, kernel_id: str) -> InferenceKernelContract | None:
        return self._entries.get(str(kernel_id or '').strip())

    def all(self) -> tuple[InferenceKernelContract, ...]:
        return tuple(
            self._entries[kernel_id]
            for kernel_id in sorted(self._entries)
        )

    def for_inference_family(self, inference_family: str) -> tuple[InferenceKernelContract, ...]:
        normalized = str(inference_family or '').strip()
        return tuple(
            contract
            for contract in self.all()
            if contract.inference_family == normalized
        )

    def by_epistemic_job(self, epistemic_job: str) -> tuple[InferenceKernelContract, ...]:
        normalized = str(epistemic_job or '').strip()
        return tuple(
            contract
            for contract in self.all()
            if contract.epistemic_job == normalized
        )

    def to_dict(self) -> dict[str, Any]:
        return {
            'version': DEFAULT_REGISTRY_VERSION,
            'count': len(self._entries),
            'entries': [entry.to_dict() for entry in self.all()],
            'index': {
                entry.kernel_id: entry.to_dict() for entry in self.all()
            },
        }


def get_registry() -> _InferenceKernelRegistry:
    return _INFERENCE_KERNEL_REGISTRY


def registry_report() -> dict[str, Any]:
    return get_registry().to_dict()


def resolve_kernels(*, inference_family: str) -> tuple[InferenceKernelContract, ...]:
    return get_registry().for_inference_family(inference_family)


def by_epistemic_job(epistemic_job: str) -> tuple[InferenceKernelContract, ...]:
    return get_registry().by_epistemic_job(epistemic_job)


def resolve(kernel_id: str) -> InferenceKernelContract | None:
    return get_registry().get(kernel_id)


_INFERENCE_KERNEL_REGISTRY = _InferenceKernelRegistry(_registry_definition())


__all__ = [
    'resolve',
    'resolve_kernels',
    'by_epistemic_job',
    'registry_report',
    'get_registry',
]
