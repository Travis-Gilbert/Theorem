"""Inference-kernel contracts and registry surface."""

from .contracts import (
    ConstrainedChoiceError,
    InferenceKernelContract,
)
from .registry import get_registry, registry_report, resolve_kernels
from .execution import run_kernel

__all__ = [
    'ConstrainedChoiceError',
    'InferenceKernelContract',
    'get_registry',
    'registry_report',
    'resolve_kernels',
    'run_kernel',
]
