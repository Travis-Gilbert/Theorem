"""CO-0.3 differential-receipt runner (Gate 0 — the strongest experimental element).

Run the same logical inputs through the existing runtime path
(build_fact_pack_from_models -> derive for Datalog, runtime adapters for
probabilistic receipts) and the substrate-affordance path, then assert the
receipts are identical at the engine payload boundary. If they disagree, Gate 0
fails and nothing downstream (the cost arms) is meaningful.

Spec backreference: docs/plans/compute-offload/implementation-plan.md CO-0.3, and
the section-4 ledger which collects Gate 0 differential results as correctness
labels.

Scope today: the comparators and explicit-input runners are live and tested. The
fully independent live leg (Django ORM vs RustyRed substrate read of the same
entities) plugs the RustyRed node set into substrate_input once CO-0.1's RustyRed
read path lands; the comparators do not change.
"""

from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Iterable, Mapping, Sequence

from ..affordances import (
    AffordanceReceipt,
    run_datalog_affordance,
    run_probabilistic_expected_value,
    run_probabilistic_source_reliability,
)
from ..datalog.contracts import DatalogFactPack, DatalogReceipt
from ..datalog.engine import DatalogEngine
from ..runtime_adapters import (
    expected_value_from_validator_records,
    source_reliability_from_records,
)
from .receipts import receipt_hash_for
from .records import BenchmarkRecord


@dataclass(frozen=True, slots=True)
class DifferentialResult:
    """Outcome of one Django-path vs substrate-path receipt comparison."""

    affordance_id: str
    matched: bool
    fact_pack_hash_django: str
    fact_pack_hash_substrate: str
    derived_count_django: int
    derived_count_substrate: int
    django_receipt_hash: str
    substrate_receipt_hash: str
    mismatches: tuple[str, ...] = ()

    def to_dict(self) -> dict[str, Any]:
        return {
            'affordance_id': self.affordance_id,
            'matched': self.matched,
            'fact_pack_hash_django': self.fact_pack_hash_django,
            'fact_pack_hash_substrate': self.fact_pack_hash_substrate,
            'derived_count_django': self.derived_count_django,
            'derived_count_substrate': self.derived_count_substrate,
            'django_receipt_hash': self.django_receipt_hash,
            'substrate_receipt_hash': self.substrate_receipt_hash,
            'mismatches': list(self.mismatches),
        }

    def to_record(
        self,
        *,
        query_id: str,
        routing_mode: str = 'B1',
        chosen_executor: str = 'datalog-cpu',
        input_refs: Sequence[str] = (),
    ) -> BenchmarkRecord:
        return BenchmarkRecord(
            query_id=query_id,
            operation_type=self.affordance_id,
            routing_mode=routing_mode,
            chosen_executor=chosen_executor,
            candidate_executors=('datalog-cpu', 'llm'),
            input_refs=tuple(input_refs),
            receipt_hash=self.substrate_receipt_hash,
            correctness_label='correct' if self.matched else 'incorrect',
        )


@dataclass(frozen=True, slots=True)
class ProbabilisticDifferentialResult:
    """Outcome of one runtime-adapter vs substrate-affordance receipt comparison."""

    affordance_id: str
    matched: bool
    model_id_django: str
    model_id_substrate: str
    django_payload_hash: str
    substrate_payload_hash: str
    substrate_receipt_hash: str
    mismatches: tuple[str, ...] = ()

    def to_dict(self) -> dict[str, Any]:
        return {
            'affordance_id': self.affordance_id,
            'matched': self.matched,
            'model_id_django': self.model_id_django,
            'model_id_substrate': self.model_id_substrate,
            'django_payload_hash': self.django_payload_hash,
            'substrate_payload_hash': self.substrate_payload_hash,
            'substrate_receipt_hash': self.substrate_receipt_hash,
            'mismatches': list(self.mismatches),
        }

    def to_record(
        self,
        *,
        query_id: str,
        routing_mode: str = 'B1',
        chosen_executor: str = 'probabilistic-cpu',
        input_refs: Sequence[str] = (),
    ) -> BenchmarkRecord:
        return BenchmarkRecord(
            query_id=query_id,
            operation_type=self.affordance_id,
            routing_mode=routing_mode,
            chosen_executor=chosen_executor,
            candidate_executors=('probabilistic-cpu', 'llm'),
            input_refs=tuple(input_refs),
            receipt_hash=self.substrate_receipt_hash,
            correctness_label='correct' if self.matched else 'incorrect',
        )


def compare_datalog_receipts(
    django_receipt: DatalogReceipt,
    affordance_receipt: AffordanceReceipt,
) -> DifferentialResult:
    """Assert a Django-path receipt and a substrate-affordance receipt agree."""

    django = django_receipt.to_dict()
    payload = dict(affordance_receipt.payload)
    mismatches: list[str] = []

    django_pack_hash = django_receipt.fact_pack_hash
    substrate_pack_hash = affordance_receipt.input_hash
    if django_pack_hash != substrate_pack_hash:
        mismatches.append(
            f'fact_pack_hash differs: django={django_pack_hash} substrate={substrate_pack_hash}'
        )

    payload_pack_hash = str(payload.get('fact_pack_hash') or '')
    if payload_pack_hash and payload_pack_hash != substrate_pack_hash:
        mismatches.append(
            f'affordance input_hash {substrate_pack_hash} != payload fact_pack_hash {payload_pack_hash}'
        )

    django_facts = django.get('derived_facts', [])
    substrate_facts = payload.get('derived_facts', [])
    if len(django_facts) != len(substrate_facts):
        mismatches.append(
            f'derived_count differs: django={len(django_facts)} substrate={len(substrate_facts)}'
        )
    if django_facts != substrate_facts:
        django_ids = {fact.get('fact_id') for fact in django_facts}
        substrate_ids = {fact.get('fact_id') for fact in substrate_facts}
        only_django = sorted(filter(None, django_ids - substrate_ids))
        only_substrate = sorted(filter(None, substrate_ids - django_ids))
        if only_django:
            mismatches.append(f'derived facts only in django: {only_django}')
        if only_substrate:
            mismatches.append(f'derived facts only in substrate: {only_substrate}')
        if not only_django and not only_substrate:
            mismatches.append('derived facts share ids but differ in content')

    return DifferentialResult(
        affordance_id=affordance_receipt.affordance_id,
        matched=not mismatches,
        fact_pack_hash_django=django_pack_hash,
        fact_pack_hash_substrate=substrate_pack_hash,
        derived_count_django=len(django_facts),
        derived_count_substrate=len(substrate_facts),
        django_receipt_hash=receipt_hash_for(django_receipt),
        substrate_receipt_hash=affordance_receipt.receipt_hash,
        mismatches=tuple(mismatches),
    )


def compare_probabilistic_receipts(
    django_payload: Mapping[str, Any],
    affordance_receipt: AffordanceReceipt,
) -> ProbabilisticDifferentialResult:
    """Assert a runtime-adapter receipt and a substrate-affordance receipt agree."""

    direct = dict(django_payload)
    payload = dict(affordance_receipt.payload)
    mismatches: list[str] = []

    direct_model = str(direct.get('model_id') or '')
    substrate_model = str(payload.get('model_id') or '')
    if direct_model != substrate_model:
        mismatches.append(f'model_id differs: django={direct_model} substrate={substrate_model}')

    for key in ('engine', 'prior', 'observations', 'posterior', 'metadata', 'receipt_hash'):
        if direct.get(key) != payload.get(key):
            mismatches.append(f'{key} differs')

    if direct != payload:
        direct_keys = set(direct)
        substrate_keys = set(payload)
        only_direct = sorted(direct_keys - substrate_keys)
        only_substrate = sorted(substrate_keys - direct_keys)
        if only_direct:
            mismatches.append(f'keys only in django: {only_direct}')
        if only_substrate:
            mismatches.append(f'keys only in substrate: {only_substrate}')

    return ProbabilisticDifferentialResult(
        affordance_id=affordance_receipt.affordance_id,
        matched=not mismatches,
        model_id_django=direct_model,
        model_id_substrate=substrate_model,
        django_payload_hash=receipt_hash_for(direct),
        substrate_payload_hash=receipt_hash_for(payload),
        substrate_receipt_hash=affordance_receipt.receipt_hash,
        mismatches=tuple(mismatches),
    )


def run_datalog_differential(
    *,
    django_fact_pack: DatalogFactPack,
    substrate_input: Mapping[str, Any],
    rule_ids: Sequence[str] = (),
) -> DifferentialResult:
    """Drive both paths from explicit inputs and compare (DB-free)."""

    rule_arg = tuple(rule_ids) or None
    django_receipt = DatalogEngine().derive(django_fact_pack, rule_ids=rule_arg)
    affordance_receipt = run_datalog_affordance(substrate_input, rule_ids=tuple(rule_ids))
    return compare_datalog_receipts(django_receipt, affordance_receipt)


def run_probabilistic_source_reliability_differential(
    *,
    evidence_records: Iterable[Mapping[str, Any]],
    substrate_input: Mapping[str, Any],
    source_id: str,
    prior_alpha: float = 1.0,
    prior_beta: float = 1.0,
) -> ProbabilisticDifferentialResult:
    """Drive source reliability through both paths and compare."""

    direct_payload = source_reliability_from_records(
        source_id=source_id,
        evidence_records=evidence_records,
        prior_alpha=prior_alpha,
        prior_beta=prior_beta,
    )
    affordance_receipt = run_probabilistic_source_reliability(
        substrate_input,
        source_id=source_id,
        prior_alpha=prior_alpha,
        prior_beta=prior_beta,
    )
    return compare_probabilistic_receipts(direct_payload, affordance_receipt)


def run_probabilistic_expected_value_differential(
    *,
    validator_records: Iterable[Mapping[str, Any]],
    substrate_input: Mapping[str, Any],
    decision_value: float = 1.0,
) -> ProbabilisticDifferentialResult:
    """Drive expected value of information through both paths and compare."""

    direct_payload = expected_value_from_validator_records(
        validator_records=validator_records,
        decision_value=decision_value,
    )
    affordance_receipt = run_probabilistic_expected_value(
        substrate_input,
        decision_value=decision_value,
    )
    return compare_probabilistic_receipts(direct_payload, affordance_receipt)


def run_datalog_differential_from_models(
    *,
    object_ids: Sequence[int | str] | None = None,
    claim_ids: Sequence[int | str] | None = None,
    artifact_ids: Sequence[int | str] | None = None,
    substrate_input: Mapping[str, Any],
    rule_ids: Sequence[str] = (),
) -> DifferentialResult:
    """Live Gate 0: Django ORM fetch vs an independent substrate node set."""

    from ..datalog.facts import build_fact_pack_from_models

    django_pack = build_fact_pack_from_models(
        object_ids=object_ids,
        claim_ids=claim_ids,
        artifact_ids=artifact_ids,
    )
    return run_datalog_differential(
        django_fact_pack=django_pack,
        substrate_input=substrate_input,
        rule_ids=rule_ids,
    )
