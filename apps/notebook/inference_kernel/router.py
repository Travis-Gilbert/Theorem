"""Routing helpers for choosing kernels from the inference registry."""

from __future__ import annotations

from typing import Any

from .contracts import KNOWN_CONSUMES_VIEWS, KNOWN_EPISTEMIC_JOBS
from .contracts import InferenceKernelContract
from .registry import by_epistemic_job, resolve, resolve_kernels


def _normalize_request(value: Any) -> str:
    return str(value or '').strip()


def route_kernel(
    *,
    kernel_id: str | None = None,
    epistemic_job: str | None = None,
    inference_family: str | None = None,
    consumes_view: str | None = None,
) -> InferenceKernelContract | None:
    """Return a single best-match kernel contract.

    Resolution rules:
    - explicit `kernel_id` always wins
    - explicit `inference_family` returns first match for that family
    - explicit `epistemic_job` returns first match for that job
    - explicit `consumes_view` narrows by first matching view
    - fallback returns Search Kernel when available
    """

    if kernel_id:
        found = resolve(_normalize_request(kernel_id))
        if found:
            return found

    if inference_family:
        family_match = resolve_kernels(inference_family=_normalize_request(inference_family))
        if family_match:
            return family_match[0]

    if epistemic_job:
        job_match = by_epistemic_job(_normalize_request(epistemic_job))
        if job_match:
            return job_match[0]

    if consumes_view:
        view = _normalize_request(consumes_view)
        if view in KNOWN_CONSUMES_VIEWS:
            matches = [
                kernel
                for kernel in resolve_kernels(inference_family='neural')
                + resolve_kernels(inference_family='graph')
                if view in kernel.consumes_view
            ]
            if matches:
                return matches[0]

    return resolve('search_kernel')


def route_candidates(
    *,
    epistemic_job: str | None = None,
    inference_family: str | None = None,
) -> tuple[InferenceKernelContract, ...]:
    """Return all candidate contracts for explicit request filters."""

    if inference_family:
        return resolve_kernels(inference_family=_normalize_request(inference_family))

    if epistemic_job:
        normalized = _normalize_request(epistemic_job)
        if normalized and normalized not in KNOWN_EPISTEMIC_JOBS:
            return ()
        return by_epistemic_job(normalized)

    return ()


def report() -> dict[str, Any]:
    from .registry import registry_report

    return registry_report()


__all__ = [
    'route_candidates',
    'route_kernel',
    'report',
]
