"""Runtime flags for Beyond Graph Intelligence surfaces."""

from __future__ import annotations

import os


def flag_enabled(name: str, *, default: bool = True) -> bool:
    value = os.getenv(name)
    if value is None:
        return default
    return value.strip().lower() not in {'0', 'false', 'no', 'off'}


def bgi_kernel_runs_enabled() -> bool:
    return flag_enabled('THESEUS_BGI_KERNEL_RUNS_ENABLED', default=True)


def bgi_native_symbolic_enabled() -> bool:
    return flag_enabled('THESEUS_BGI_NATIVE_SYMBOLIC_ENABLED', default=True)


def bgi_federated_receipts_enabled() -> bool:
    return flag_enabled('THESEUS_BGI_FEDERATED_RECEIPTS_ENABLED', default=True)
