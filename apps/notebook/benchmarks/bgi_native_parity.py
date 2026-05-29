"""Native parity benchmark runner for Beyond Graph Intelligence receipts.

This module compares settled Python receipt contracts against optional Rust
PyO3 helpers in ``theseus_native``. It is intentionally runnable as a plain
script so native hot-path work can be measured without booting Django.
"""

from __future__ import annotations

import json
import hashlib
import time
from collections import Counter
from typing import Any

from apps.notebook.discovery_runs.validators import receipt_from_exit_code
from apps.notebook.inference_engines.common import stable_hash, stable_json
from apps.notebook.inference_engines.datalog.contracts import (
    DatalogFact,
    fact_pack_from_iterable,
)
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.egraph.engine import EGraphTheorem
from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.engine import EvolutionEngine
from apps.notebook.inference_engines.probabilistic.engine import ProbProgEngine


def _json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), default=str)


def _time_average(callback, *, iterations: int) -> tuple[Any, float]:
    result = None
    started = time.perf_counter()
    for _ in range(max(1, iterations)):
        result = callback()
    elapsed = time.perf_counter() - started
    return result, elapsed / max(1, iterations)


def _native_module():
    try:
        import theseus_native  # type: ignore[import-not-found]
    except Exception:
        return None
    required = (
        'bgi_stable_hash_json',
        'bgi_egraph_receipt_summary_json',
        'bgi_datalog_receipt_summary_json',
        'bgi_datalog_verified_rule_ids_json',
        'bgi_datalog_derive_core_json',
        'bgi_fact_pack_hash_rows_json',
        'bgi_compact_receipts_json',
        'bgi_probabilistic_source_reliability_json',
        'bgi_probabilistic_expected_value_json',
        'bgi_evolution_archive_json',
    )
    if not all(hasattr(theseus_native, name) for name in required):
        return None
    return theseus_native


def _sample_egraph_receipt() -> dict[str, Any]:
    items = [
        {
            'id': 'atom-1',
            'channel': 'trusted_repo_memory',
            'text': 'Graph kernels are read-only in this pass.',
            'tokens': 12,
            'semantic_hash': 'sem-1',
        },
        {
            'id': 'atom-1-copy',
            'channel': 'trusted_repo_memory',
            'text': 'Graph kernels are read-only in this pass.',
            'tokens': 12,
            'semantic_hash': 'sem-1',
        },
        {
            'id': 'empty-optional',
            'channel': 'external_content',
            'text': '',
            'tokens': 0,
        },
    ]
    return EGraphTheorem().context_pack(
        expression_id='bgi-parity-context',
        items=items,
    ).to_dict()


def _sample_datalog_fact_pack():
    facts = [
        DatalogFact(
            relation='claim',
            entity_id='claim-1',
            attributes={'status': 'captured'},
            source_ref='unit:claim',
        ),
        DatalogFact(
            relation='object',
            entity_id='object-1',
            attributes={'title': 'Native Receipt'},
            source_ref='unit:object',
        ),
        DatalogFact(
            relation='object',
            entity_id='object-2',
            attributes={'title': 'native receipt'},
            source_ref='unit:object',
        ),
        DatalogFact(
            relation='claim_dependency',
            entity_id='dep-1',
            attributes={
                'claim_id': 'claim-2',
                'depends_on_object_id': 'object-1',
                'justification_type': 'source',
                'strength': 0.8,
            },
            source_ref='unit:dependency',
        ),
    ]
    return fact_pack_from_iterable(facts)


def _sample_datalog_receipt() -> dict[str, Any]:
    return DatalogEngine().derive(_sample_datalog_fact_pack()).to_dict()


def _stable_hash_golden_values() -> list[Any]:
    return [
        {'b': 2, 'a': {'z': [], 'm': {'inner': 'value'}}},
        {'integer_valued_float': 1.0, 'integer': 1},
        {'long_mantissa': 0.6666666666666666, 'confidence': 0.85},
        {'unicode': 'Theseus native theorem path'},
        {'empty': {'list': [], 'dict': {}}},
    ]


def _probabilistic_source_reliability_receipt() -> dict[str, Any]:
    return ProbProgEngine().source_reliability(
        source_id='source-a',
        prior_alpha=2.0,
        prior_beta=2.0,
        corroborated=6,
        contradicted=2,
    ).to_dict()


def _probabilistic_expected_value_receipt() -> dict[str, Any]:
    return ProbProgEngine().expected_value_of_information(
        current_uncertainty=0.6666666666666666,
        expected_uncertainty_after=0.25,
        decision_value=3.0,
        validator_cost=1.0,
    ).to_dict()


def _evolution_archive_candidates() -> list[EvolutionCandidate]:
    return [
        EvolutionCandidate(
            candidate_id='archive-a',
            niche='context|promote|green',
            score=0.8,
            novelty=0.2,
            payload={'policy': {'token_penalty': 0.01}},
        ),
        EvolutionCandidate(
            candidate_id='archive-b',
            niche='context|promote|green',
            score=0.8,
            novelty=0.4,
            payload={'policy': {'token_penalty': 0.02}},
        ),
        EvolutionCandidate(
            candidate_id='archive-c',
            niche='tool|rewrite|blue',
            score=0.6,
            novelty=1.0,
            payload={'patch': {'kind': 'tool'}},
        ),
    ]


def _evolution_archive_receipt() -> dict[str, Any]:
    return EvolutionEngine().archive(
        _evolution_archive_candidates(),
        elites_per_niche=1,
    ).to_dict()


def _evolution_archive_payload() -> dict[str, Any]:
    return {
        'candidates': [candidate.to_dict() for candidate in _evolution_archive_candidates()],
        'elites_per_niche': 1,
    }


def _sample_views() -> list[dict[str, Any]]:
    return [
        {
            'source_type': 'context_artifact',
            'source_artifact_id': 'artifact-2',
            'source_sha': 'sha-2',
            'view_type': 'text',
            'view_hash': stable_hash({
                'excerpt': 'Validator receipts remain proposal-only until reviewed.',
                'symbols': ['validator', 'receipt'],
            }),
            'created_by_process_id': 'bgi-native-parity',
            'lineage_parent_refs': [],
            'payload': {
                'excerpt': 'Validator receipts remain proposal-only until reviewed.',
                'symbols': ['validator', 'receipt'],
            },
        },
        {
            'source_type': 'context_artifact',
            'source_artifact_id': 'artifact-1',
            'source_sha': 'sha-1',
            'view_type': 'claim_graph',
            'view_hash': stable_hash({
                'claims': [{'id': 'claim-1', 'text': 'Native compaction is deterministic.'}],
                'edges': [{'from': 'claim-1', 'to': 'validator-1', 'type': 'tested_by'}],
            }),
            'created_by_process_id': 'bgi-native-parity',
            'lineage_parent_refs': [],
            'payload': {
                'claims': [{'id': 'claim-1', 'text': 'Native compaction is deterministic.'}],
                'edges': [{'from': 'claim-1', 'to': 'validator-1', 'type': 'tested_by'}],
            },
        },
    ]


def _fact_pack_payload_rows(views: list[dict[str, Any]]) -> list[dict[str, Any]]:
    rows: list[dict[str, Any]] = []
    for view in views:
        rows.append({
            'source_type': view['source_type'],
            'source_artifact_id': view['source_artifact_id'],
            'source_sha': view['source_sha'],
            'view_type': view['view_type'],
            'view_hash': view['view_hash'],
            'created_by_process_id': view['created_by_process_id'],
            'lineage_parent_refs': list(view['lineage_parent_refs']),
            'payload': _json(view['payload']),
        })
    return rows


def _fact_pack_hash(views: list[dict[str, Any]]) -> str:
    rows = sorted(
        _fact_pack_payload_rows(views),
        key=lambda item: (item['source_artifact_id'], item['view_type'], item['view_hash']),
    )
    return hashlib.sha256(_json(rows).encode('utf-8')).hexdigest()


def _egraph_summary(receipt: dict[str, Any]) -> dict[str, Any]:
    return {
        'domain': receipt.get('domain', ''),
        'engine': receipt.get('engine', ''),
        'equivalent': bool(receipt.get('equivalent')),
        'extracted_cost': float(receipt.get('extracted_cost') or 0.0),
        'input_hash': receipt.get('input_hash', ''),
        'native_backend': receipt.get('native_backend', ''),
        'original_cost': float(receipt.get('original_cost') or 0.0),
        'output_hash': receipt.get('output_hash', ''),
        'rewrite_count': len(receipt.get('rewrite_trace') or []),
    }


def _datalog_summary(receipt: dict[str, Any]) -> dict[str, Any]:
    return {
        'derived_count': int(receipt.get('derived_count') or 0),
        'engine': receipt.get('engine', ''),
        'fact_pack_hash': receipt.get('fact_pack_hash', ''),
        'rule_ids': list(receipt.get('rule_ids') or []),
        'warning_count': len(receipt.get('warnings') or []),
        'writeback_policy': receipt.get('writeback_policy', ''),
    }


def _receipt_hash(receipt: dict[str, Any]) -> str:
    for key in (
        'receipt_hash',
        'payload_hash',
        'formula_hash',
        'input_hash',
        'output_hash',
        'fact_pack_hash',
    ):
        value = str(receipt.get(key) or '')
        if value:
            return value
    return ''


def compact_receipts(receipts: list[dict[str, Any]]) -> dict[str, Any]:
    hashes = sorted({
        receipt_hash
        for receipt in receipts
        if (receipt_hash := _receipt_hash(receipt))
    })
    status_counts = Counter(
        str(receipt.get('status') or '')
        for receipt in receipts
        if str(receipt.get('status') or '')
    )
    return {
        'count': len(receipts),
        'payload_hash': stable_hash(receipts),
        'receipt_hashes': hashes,
        'status_counts': dict(sorted(status_counts.items())),
    }


def _benchmark_case(name: str, python_callback, native_callback, *, iterations: int) -> dict[str, Any]:
    python_value, python_seconds = _time_average(python_callback, iterations=iterations)
    native_value, native_seconds = _time_average(native_callback, iterations=iterations)
    return {
        'name': name,
        'parity_passed': python_value == native_value,
        'python_seconds': python_seconds,
        'native_seconds': native_seconds,
        'speedup': python_seconds / max(native_seconds, 1e-12),
        'python_value': python_value,
        'native_value': native_value,
    }


def run_all_parity_benchmarks(
    *,
    iterations: int = 100,
    native_module: Any | None = None,
) -> dict[str, Any]:
    native = native_module or _native_module()
    if native is None:
        return {
            'native_available': False,
            'benchmarks': [],
        }

    egraph_receipt = _sample_egraph_receipt()
    datalog_fact_pack = _sample_datalog_fact_pack()
    datalog_receipt = _sample_datalog_receipt()
    views = _sample_views()
    fact_pack_rows = _fact_pack_payload_rows(views)
    validator_receipt = receipt_from_exit_code(
        validator_id='pytest:bgi-native-parity',
        command='pytest apps/notebook/discovery_runs/tests',
        exit_code=0,
        output='2 passed',
        duration_ms=32,
    ).to_dict()
    receipts = [
        egraph_receipt,
        datalog_receipt,
        validator_receipt,
        {'status': 'accepted', 'payload_hash': stable_hash({'accepted': True})},
    ]

    benchmarks = [
        _benchmark_case(
            'stable_hash_golden_vectors',
            lambda: [stable_hash(value) for value in _stable_hash_golden_values()],
            lambda: [native.bgi_stable_hash_json(stable_json(value)) for value in _stable_hash_golden_values()],
            iterations=iterations,
        ),
        _benchmark_case(
            'egraph_receipt_summary',
            lambda: _egraph_summary(egraph_receipt),
            lambda: json.loads(native.bgi_egraph_receipt_summary_json(_json(egraph_receipt))),
            iterations=iterations,
        ),
        _benchmark_case(
            'datalog_receipt_summary',
            lambda: _datalog_summary(datalog_receipt),
            lambda: json.loads(native.bgi_datalog_receipt_summary_json(_json(datalog_receipt))),
            iterations=iterations,
        ),
        _benchmark_case(
            'datalog_derive_core',
            lambda: datalog_receipt,
            lambda: json.loads(native.bgi_datalog_derive_core_json(stable_json({
                'facts': [fact.to_dict() for fact in datalog_fact_pack.facts],
                'rule_ids': [],
            }))),
            iterations=iterations,
        ),
        _benchmark_case(
            'fact_pack_hash',
            lambda: _fact_pack_hash(views),
            lambda: native.bgi_fact_pack_hash_rows_json(_json(fact_pack_rows)),
            iterations=iterations,
        ),
        _benchmark_case(
            'receipt_compaction',
            lambda: compact_receipts(receipts),
            lambda: json.loads(native.bgi_compact_receipts_json(stable_json(receipts))),
            iterations=iterations,
        ),
        _benchmark_case(
            'probabilistic_source_reliability',
            lambda: _probabilistic_source_reliability_receipt(),
            lambda: json.loads(native.bgi_probabilistic_source_reliability_json(stable_json({
                'source_id': 'source-a',
                'prior_alpha': 2.0,
                'prior_beta': 2.0,
                'corroborated': 6,
                'contradicted': 2,
            }))),
            iterations=iterations,
        ),
        _benchmark_case(
            'probabilistic_expected_value',
            lambda: _probabilistic_expected_value_receipt(),
            lambda: json.loads(native.bgi_probabilistic_expected_value_json(stable_json({
                'current_uncertainty': 0.6666666666666666,
                'expected_uncertainty_after': 0.25,
                'decision_value': 3.0,
                'validator_cost': 1.0,
            }))),
            iterations=iterations,
        ),
        _benchmark_case(
            'evolution_archive',
            lambda: _evolution_archive_receipt(),
            lambda: json.loads(native.bgi_evolution_archive_json(stable_json(_evolution_archive_payload()))),
            iterations=iterations,
        ),
    ]
    return {
        'native_available': True,
        'iterations': max(1, iterations),
        'benchmarks': benchmarks,
        'all_parity_passed': all(item['parity_passed'] for item in benchmarks),
    }


def main() -> None:
    print(json.dumps(run_all_parity_benchmarks(), indent=2, sort_keys=True))


if __name__ == '__main__':
    main()
