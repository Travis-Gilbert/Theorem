"""CO-1 benchmark ledger.

Economics validation + router training + cascade calibration data, one
append-only JSONL per run. See docs/plans/compute-offload/implementation-plan.md
sections 3-5. Pure-Python, Django-free, file-based dev/eval infrastructure.
"""

from .arms import (
    BenchmarkCase,
    ExecutorObservation,
    record_for_routing_mode,
    run_arm_records,
    select_observation,
)
from .gate0 import (
    Gate0Check,
    Gate0DatalogCase,
    Gate0ExpectedValueCase,
    Gate0Failure,
    Gate0Report,
    Gate0RunResult,
    Gate0SourceReliabilityCase,
    run_gate0,
    run_gate0_cases,
)
from .ledger import BenchmarkLedger
from .preregistration import PRE_REGISTRATION
from .receipts import receipt_hash_for
from .records import CORRECTNESS_LABELS, ROUTING_MODES, BenchmarkRecord

__all__ = [
    'BenchmarkLedger',
    'BenchmarkRecord',
    'BenchmarkCase',
    'ExecutorObservation',
    'Gate0Check',
    'Gate0DatalogCase',
    'Gate0ExpectedValueCase',
    'Gate0Failure',
    'Gate0Report',
    'Gate0RunResult',
    'Gate0SourceReliabilityCase',
    'ROUTING_MODES',
    'CORRECTNESS_LABELS',
    'PRE_REGISTRATION',
    'record_for_routing_mode',
    'run_arm_records',
    'run_gate0',
    'run_gate0_cases',
    'select_observation',
    'receipt_hash_for',
]
