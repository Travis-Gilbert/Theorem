"""Native runtime guardrails for Beyond Graph Intelligence accelerators."""

from __future__ import annotations

import os
from dataclasses import dataclass, field
from typing import Any


@dataclass(frozen=True, slots=True)
class NativeFeatureSpec:
    feature_id: str
    python_fallback: str
    native_module: str
    benchmark_required: bool = True
    parity_tests: tuple[str, ...] = ()
    can_write_canon: bool = False
    metadata: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return {
            'feature_id': self.feature_id,
            'python_fallback': self.python_fallback,
            'native_module': self.native_module,
            'benchmark_required': self.benchmark_required,
            'parity_tests': list(self.parity_tests),
            'can_write_canon': self.can_write_canon,
            'metadata': dict(self.metadata),
        }


def native_enabled(feature: NativeFeatureSpec) -> bool:
    if os.environ.get('THESEUS_DISABLE_NATIVE', '').strip():
        return False
    if feature.can_write_canon:
        return False
    return True


def native_gate_report(feature: NativeFeatureSpec) -> dict[str, Any]:
    enabled = native_enabled(feature)
    return {
        **feature.to_dict(),
        'enabled': enabled,
        'disabled_reason': '' if enabled else 'native disabled or feature attempts canonical writes',
        'requires_python_fallback': True,
        'requires_parity_tests': bool(feature.parity_tests),
    }

