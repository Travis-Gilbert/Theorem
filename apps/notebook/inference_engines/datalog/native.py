"""Optional Rust/Datafrog-backed Datalog adapter."""

from __future__ import annotations

import json
from typing import Iterable

from apps.notebook.inference_engines.common import stable_json

from .contracts import DatalogFactPack, DatalogReceipt, DerivedFact
from .engine import DatalogEngine
from .rules import DEFAULT_RULES


DEFAULT_DATALOG_RULE_IDS = tuple(rule.rule_id for rule in DEFAULT_RULES)
VERIFIED_NATIVE_DATALOG_RULE_IDS: frozenset[str] = frozenset(DEFAULT_DATALOG_RULE_IDS)


def verified_native_datalog_rule_ids() -> tuple[str, ...]:
    return tuple(sorted(VERIFIED_NATIVE_DATALOG_RULE_IDS))


def native_datalog_can_handle(rule_ids: Iterable[str] | None) -> bool:
    requested = tuple(rule_ids or ())
    if not requested:
        return set(DEFAULT_DATALOG_RULE_IDS).issubset(VERIFIED_NATIVE_DATALOG_RULE_IDS)
    return set(requested).issubset(VERIFIED_NATIVE_DATALOG_RULE_IDS)


def _native_feature_enabled() -> bool:
    from apps.notebook.inference_kernel.native_strategy import NativeFeatureSpec, native_enabled

    feature = NativeFeatureSpec(
        feature_id='bgi_datalog_deriver',
        python_fallback='apps.notebook.inference_engines.datalog.engine.DatalogEngine',
        native_module='theseus_native.bgi_datalog_derive_core_json',
        parity_tests=(
            'apps/notebook/benchmarks/bgi_native_parity.py',
            'rustyredcore_THG/tests/test_bgi_parity.py',
        ),
    )
    return native_enabled(feature)


def _native_symbolic_enabled() -> bool:
    from apps.notebook.inference_kernel.settings import bgi_native_symbolic_enabled

    return bgi_native_symbolic_enabled()


def _native_module():
    if not _native_symbolic_enabled():
        return None
    if not _native_feature_enabled():
        return None
    try:
        import theseus_native  # type: ignore[import-not-found]
    except Exception:
        return None
    if not hasattr(theseus_native, 'bgi_datalog_derive_core_json'):
        return None
    if not hasattr(theseus_native, 'bgi_datalog_verified_rule_ids_json'):
        return None
    return theseus_native


def _native_verified_rule_ids(native) -> frozenset[str]:
    try:
        payload = json.loads(native.bgi_datalog_verified_rule_ids_json())
    except Exception:
        return frozenset()
    return frozenset(str(item) for item in payload)


def _receipt_from_dict(payload: dict) -> DatalogReceipt:
    return DatalogReceipt(
        engine=str(payload.get('engine') or 'python-reference-datalog'),
        fact_pack_hash=str(payload.get('fact_pack_hash') or ''),
        rule_ids=tuple(payload.get('rule_ids') or ()),
        derived_facts=tuple(
            DerivedFact(
                rule_id=str(item.get('rule_id') or ''),
                relation=str(item.get('relation') or ''),
                subject_id=str(item.get('subject_id') or ''),
                reason=str(item.get('reason') or ''),
                dependency_fact_ids=tuple(item.get('dependency_fact_ids') or ()),
                attributes=dict(item.get('attributes') or {}),
                confidence=float(item.get('confidence') or 0.0),
                writeback_policy=str(item.get('writeback_policy') or 'read-only'),
                fact_id=str(item.get('fact_id') or ''),
            )
            for item in payload.get('derived_facts') or []
        ),
        warnings=tuple(payload.get('warnings') or ()),
    )


class NativeDatalogEngine(DatalogEngine):
    """Use Rust/Datafrog core rules when the native wheel is available."""

    def derive(
        self,
        fact_pack: DatalogFactPack,
        *,
        rule_ids=None,
    ) -> DatalogReceipt:
        native = _native_module()
        requested = tuple(rule_ids or ())
        if native is None or not native_datalog_can_handle(requested):
            return super().derive(fact_pack, rule_ids=rule_ids)
        native_rule_ids = _native_verified_rule_ids(native)
        if not set(DEFAULT_DATALOG_RULE_IDS).issubset(native_rule_ids):
            return super().derive(fact_pack, rule_ids=rule_ids)
        payload = {
            'facts': [fact.to_dict() for fact in fact_pack.facts],
            'rule_ids': list(requested) if requested else [],
        }
        return _receipt_from_dict(
            json.loads(native.bgi_datalog_derive_core_json(stable_json(payload))),
        )
