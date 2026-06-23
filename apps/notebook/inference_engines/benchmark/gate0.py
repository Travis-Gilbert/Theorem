"""Runnable Gate 0 harness for substrate projection parity.

Gate 0 is the proof that the substrate-affordance path produces the same
engine-level result as the existing runtime path. This module composes the
already-tested differential runners with the benchmark ledger and reporting
layer so a Gate 0 run leaves normal `BenchmarkRecord` evidence behind.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Mapping, Sequence

from ..datalog.contracts import DatalogFactPack
from .differential import (
    run_datalog_differential,
    run_probabilistic_expected_value_differential,
    run_probabilistic_source_reliability_differential,
)
from .ledger import BenchmarkLedger
from .records import BenchmarkRecord
from .report import LedgerReport, summarize_records


@dataclass(frozen=True, slots=True)
class Gate0DatalogCase:
    """Datalog runtime-vs-substrate parity case."""

    query_id: str
    django_fact_pack: DatalogFactPack
    substrate_input: Mapping[str, Any]
    rule_ids: tuple[str, ...] = ()
    input_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Gate0SourceReliabilityCase:
    """Probabilistic source-reliability parity case."""

    query_id: str
    evidence_records: tuple[Mapping[str, Any], ...]
    substrate_input: Mapping[str, Any]
    source_id: str
    prior_alpha: float = 1.0
    prior_beta: float = 1.0
    input_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Gate0ExpectedValueCase:
    """Probabilistic expected-value-of-information parity case."""

    query_id: str
    validator_records: tuple[Mapping[str, Any], ...]
    substrate_input: Mapping[str, Any]
    decision_value: float = 1.0
    input_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Gate0Check:
    """Precomputed differential result ready to become a Gate 0 ledger row."""

    query_id: str
    result: Any
    input_refs: tuple[str, ...] = ()


@dataclass(frozen=True, slots=True)
class Gate0Failure:
    """One failed Gate 0 check with its comparator mismatches."""

    query_id: str
    operation_type: str
    mismatches: tuple[str, ...]


@dataclass(frozen=True, slots=True)
class Gate0Report:
    """Lightweight pass/fail report for precomputed Gate 0 checks."""

    records: tuple[BenchmarkRecord, ...]
    failures: tuple[Gate0Failure, ...]

    @property
    def total(self) -> int:
        return len(self.records)

    @property
    def failed(self) -> int:
        return len(self.failures)

    @property
    def passed(self) -> int:
        return self.total - self.failed

    @property
    def gate_passed(self) -> bool:
        return self.total > 0 and self.failed == 0

    def to_dict(self) -> dict[str, Any]:
        return {
            'gate_passed': self.gate_passed,
            'total': self.total,
            'passed': self.passed,
            'failed': self.failed,
            'records': [record.to_dict() for record in self.records],
            'failures': [
                {
                    'query_id': failure.query_id,
                    'operation_type': failure.operation_type,
                    'mismatches': list(failure.mismatches),
                }
                for failure in self.failures
            ],
        }


@dataclass(frozen=True, slots=True)
class Gate0RunResult:
    """Gate 0 records plus the report over exactly those records."""

    records: tuple[BenchmarkRecord, ...]
    report: LedgerReport

    @property
    def passed(self) -> bool:
        return bool(self.records) and self.report.gate0_pass_rate == 1.0 and not self.report.gate0_failures

    def to_dict(self) -> dict[str, Any]:
        return {
            'passed': self.passed,
            'record_count': len(self.records),
            'records': [record.to_dict() for record in self.records],
            'report': self.report.to_dict(),
        }


def run_gate0(
    checks: Sequence[Gate0Check],
    *,
    ledger: BenchmarkLedger | None = None,
) -> Gate0Report:
    """Record precomputed Gate 0 differential checks and report pass/fail."""

    records: list[BenchmarkRecord] = []
    failures: list[Gate0Failure] = []
    for check in checks:
        record = check.result.to_record(
            query_id=check.query_id,
            input_refs=check.input_refs,
        )
        records.append(record)
        if ledger is not None:
            ledger.record(record)
        if not check.result.matched:
            failures.append(
                Gate0Failure(
                    query_id=check.query_id,
                    operation_type=record.operation_type,
                    mismatches=tuple(check.result.mismatches),
                )
            )
    return Gate0Report(records=tuple(records), failures=tuple(failures))


def run_gate0_cases(
    *,
    ledger: BenchmarkLedger,
    datalog_cases: Sequence[Gate0DatalogCase] = (),
    source_reliability_cases: Sequence[Gate0SourceReliabilityCase] = (),
    expected_value_cases: Sequence[Gate0ExpectedValueCase] = (),
) -> Gate0RunResult:
    """Run Gate 0 parity cases, append ledger rows, and report pass/fail."""

    if not datalog_cases and not source_reliability_cases and not expected_value_cases:
        raise ValueError('Gate 0 requires at least one parity case')

    checks: list[Gate0Check] = []

    for case in datalog_cases:
        result = run_datalog_differential(
            django_fact_pack=case.django_fact_pack,
            substrate_input=case.substrate_input,
            rule_ids=case.rule_ids,
        )
        checks.append(Gate0Check(query_id=case.query_id, result=result, input_refs=case.input_refs))

    for case in source_reliability_cases:
        result = run_probabilistic_source_reliability_differential(
            evidence_records=case.evidence_records,
            substrate_input=case.substrate_input,
            source_id=case.source_id,
            prior_alpha=case.prior_alpha,
            prior_beta=case.prior_beta,
        )
        checks.append(Gate0Check(query_id=case.query_id, result=result, input_refs=case.input_refs))

    for case in expected_value_cases:
        result = run_probabilistic_expected_value_differential(
            validator_records=case.validator_records,
            substrate_input=case.substrate_input,
            decision_value=case.decision_value,
        )
        checks.append(Gate0Check(query_id=case.query_id, result=result, input_refs=case.input_refs))

    gate_report = run_gate0(checks, ledger=ledger)
    report = summarize_records(
        gate_report.records,
        pre_registration=ledger.pre_registration(),
    )
    return Gate0RunResult(records=gate_report.records, report=report)


@dataclass(frozen=True, slots=True)
class _EngineAffordanceResult:
    """Projection-fidelity result for one of the eight engine affordances.

    Implements the same `.matched` / `.mismatches` / `.to_record(...)` interface
    as DifferentialResult so it folds into `run_gate0` unchanged. The Gate-0
    invariant for these engines is that the substrate-affordance payload equals
    the direct engine receipt (both call the same Python engine), so a mismatch
    means the affordance projection altered the receipt.
    """

    affordance_id: str
    matched: bool
    substrate_receipt_hash: str
    chosen_executor: str
    mismatches: tuple[str, ...] = ()

    def to_record(self, *, query_id: str, input_refs: Sequence[str] = ()) -> BenchmarkRecord:
        return BenchmarkRecord(
            query_id=query_id,
            operation_type=self.affordance_id,
            routing_mode='B1',
            chosen_executor=self.chosen_executor,
            candidate_executors=(self.chosen_executor, 'llm'),
            input_refs=tuple(input_refs),
            receipt_hash=self.substrate_receipt_hash,
            correctness_label='correct' if self.matched else 'incorrect',
        )


def run_gate0_engine_affordances(*, ledger: BenchmarkLedger | None = None) -> Gate0RunResult:
    """Run the Gate-0 projection-fidelity pass for the eight non-datalog engines.

    Self-contained: pulls validated firing cases from
    `affordances_engines.engine_affordance_differential_cases()`, asserts each
    affordance payload equals the direct engine receipt, and appends one ledger
    row per engine. Composes with `run_gate0_cases` (datalog + probabilistic) to
    cover all ten engines.
    """

    from ..affordances_engines import engine_affordance_differential_cases

    checks: list[Gate0Check] = []
    for case in engine_affordance_differential_cases():
        affordance_id = str(case['affordance_id'])
        affordance_receipt = case['affordance_receipt']
        direct_receipt = case['direct_receipt']
        matched = affordance_receipt.payload == direct_receipt
        mismatches: tuple[str, ...] = ()
        if not matched:
            mismatches = (
                f'{affordance_id}: substrate-affordance payload != direct engine receipt',
            )
        result = _EngineAffordanceResult(
            affordance_id=affordance_id,
            matched=matched,
            substrate_receipt_hash=affordance_receipt.receipt_hash,
            chosen_executor=f"{affordance_id.split('.')[0]}-cpu",
            mismatches=mismatches,
        )
        checks.append(Gate0Check(
            query_id=f'gate0-{affordance_id}',
            result=result,
            input_refs=tuple(affordance_receipt.input_node_refs),
        ))

    gate_report = run_gate0(checks, ledger=ledger)
    report = summarize_records(
        gate_report.records,
        pre_registration=ledger.pre_registration() if ledger is not None else {},
    )
    return Gate0RunResult(records=gate_report.records, report=report)
