"""Optional Rust-backed quality-diversity archive adapter."""

from __future__ import annotations

import json

from apps.notebook.inference_engines.common import stable_json

from .contracts import ArchiveReceipt, EvolutionCandidate
from .engine import EvolutionEngine


def native_evolution_can_handle() -> bool:
    return True


def _native_feature_enabled() -> bool:
    from apps.notebook.inference_kernel.native_strategy import NativeFeatureSpec, native_enabled

    feature = NativeFeatureSpec(
        feature_id='bgi_evolution_archive',
        python_fallback='apps.notebook.inference_engines.evolution.engine.EvolutionEngine',
        native_module='theseus_native.bgi_evolution_archive_json',
        parity_tests=(
            'apps/notebook/benchmarks/bgi_native_parity.py',
            'rustyredcore_THG/tests/test_bgi_parity.py',
            'rustyredcore_THG/tests/test_datalog_derivation_parity.py',
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
    if not hasattr(theseus_native, 'bgi_evolution_archive_json'):
        return None
    return theseus_native


def _receipt_from_dict(payload: dict) -> ArchiveReceipt:
    return ArchiveReceipt(
        engine=str(payload.get('engine') or 'quality-diversity-python-fallback'),
        archive_hash=str(payload.get('archive_hash') or ''),
        elites_by_niche={
            str(niche): [dict(candidate) for candidate in elites]
            for niche, elites in dict(payload.get('elites_by_niche') or {}).items()
        },
        rejected_count=int(payload.get('rejected_count') or 0),
    )


class NativeEvolutionEngine(EvolutionEngine):
    """Use Rust archive ranking when the native wheel is available and verified."""

    def archive(
        self,
        candidates: list[EvolutionCandidate],
        *,
        elites_per_niche: int = 2,
    ) -> ArchiveReceipt:
        native = _native_module()
        if native is None or not native_evolution_can_handle():
            return super().archive(candidates, elites_per_niche=elites_per_niche)
        payload = {
            'candidates': [candidate.to_dict() for candidate in candidates],
            'elites_per_niche': int(elites_per_niche),
        }
        return _receipt_from_dict(
            json.loads(native.bgi_evolution_archive_json(stable_json(payload))),
        )
