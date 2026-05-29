"""Optional Rust-backed probabilistic-programming adapter."""

from __future__ import annotations

import json

from apps.notebook.inference_engines.common import stable_json

from .contracts import PosteriorReceipt
from .engine import ProbProgEngine


VERIFIED_NATIVE_PROBABILISTIC_METHODS = frozenset({
    'source_reliability',
    'expected_value_of_information',
})


def native_probabilistic_can_handle(method: str) -> bool:
    return method in VERIFIED_NATIVE_PROBABILISTIC_METHODS


def _native_feature_enabled() -> bool:
    from apps.notebook.inference_kernel.native_strategy import NativeFeatureSpec, native_enabled

    feature = NativeFeatureSpec(
        feature_id='bgi_probabilistic_source_reliability',
        python_fallback='apps.notebook.inference_engines.probabilistic.engine.ProbProgEngine',
        native_module='theseus_native.bgi_probabilistic_*',
        parity_tests=(
            'apps/notebook/benchmarks/bgi_native_parity.py',
            'rustyredcore_THG/tests/test_bgi_parity.py',
        ),
    )
    return native_enabled(feature)


def _native_symbolic_enabled() -> bool:
    from apps.notebook.inference_kernel.settings import bgi_native_symbolic_enabled

    return bgi_native_symbolic_enabled()


def _native_module(required: str):
    if not _native_symbolic_enabled():
        return None
    if not _native_feature_enabled():
        return None
    try:
        import theseus_native  # type: ignore[import-not-found]
    except Exception:
        return None
    if not hasattr(theseus_native, required):
        return None
    return theseus_native


def _receipt_from_dict(payload: dict) -> PosteriorReceipt:
    return PosteriorReceipt(
        engine=str(payload.get('engine') or 'beta-binomial-python-fallback'),
        model_id=str(payload.get('model_id') or ''),
        prior=dict(payload.get('prior') or {}),
        observations=dict(payload.get('observations') or {}),
        posterior=dict(payload.get('posterior') or {}),
        metadata=dict(payload.get('metadata') or {}),
        receipt_hash=str(payload.get('receipt_hash') or ''),
    )


class NativeProbProgEngine(ProbProgEngine):
    """Use Rust scalar math when the native wheel is available and verified."""

    def source_reliability(
        self,
        *,
        source_id: str,
        prior_alpha: float = 1.0,
        prior_beta: float = 1.0,
        corroborated: int = 0,
        contradicted: int = 0,
    ) -> PosteriorReceipt:
        native = _native_module('bgi_probabilistic_source_reliability_json')
        if native is None or not native_probabilistic_can_handle('source_reliability'):
            return super().source_reliability(
                source_id=source_id,
                prior_alpha=prior_alpha,
                prior_beta=prior_beta,
                corroborated=corroborated,
                contradicted=contradicted,
            )
        payload = {
            'source_id': source_id,
            'prior_alpha': float(prior_alpha),
            'prior_beta': float(prior_beta),
            'corroborated': int(corroborated),
            'contradicted': int(contradicted),
        }
        return _receipt_from_dict(
            json.loads(native.bgi_probabilistic_source_reliability_json(stable_json(payload))),
        )

    def expected_value_of_information(
        self,
        *,
        current_uncertainty: float,
        expected_uncertainty_after: float,
        decision_value: float,
        validator_cost: float,
    ) -> PosteriorReceipt:
        native = _native_module('bgi_probabilistic_expected_value_json')
        if native is None or not native_probabilistic_can_handle('expected_value_of_information'):
            return super().expected_value_of_information(
                current_uncertainty=current_uncertainty,
                expected_uncertainty_after=expected_uncertainty_after,
                decision_value=decision_value,
                validator_cost=validator_cost,
            )
        payload = {
            'current_uncertainty': float(current_uncertainty),
            'expected_uncertainty_after': float(expected_uncertainty_after),
            'decision_value': float(decision_value),
            'validator_cost': float(validator_cost),
        }
        return _receipt_from_dict(
            json.loads(native.bgi_probabilistic_expected_value_json(stable_json(payload))),
        )
