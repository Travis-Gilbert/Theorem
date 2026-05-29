"""Reference Datalog-style engine over normalized fact packs."""

from __future__ import annotations

from typing import Iterable

from .contracts import DatalogFactPack, DatalogReceipt, DerivedFact, RuleDefinition
from .facts import build_fact_pack_from_models
from .rules import DEFAULT_RULES


class DatalogEngine:
    """Small read-only derivation engine with stable receipts.

    The implementation is deliberately pure Python while the contracts settle.
    Rust Datafrog and Souffle/C++ adapters can implement the same receipt
    surface once hot workloads justify native execution.
    """

    def __init__(self, rules: Iterable[RuleDefinition] = DEFAULT_RULES) -> None:
        self._rules = tuple(rules)
        self._rule_index = {rule.rule_id: rule for rule in self._rules}

    def derive(
        self,
        fact_pack: DatalogFactPack,
        *,
        rule_ids: Iterable[str] | None = None,
    ) -> DatalogReceipt:
        selected_ids = tuple(rule_ids or self._rule_index.keys())
        warnings: list[str] = []
        derived: list[DerivedFact] = []
        for rule_id in selected_ids:
            rule = self._rule_index.get(rule_id)
            if rule is None:
                warnings.append(f'Unknown Datalog rule skipped: {rule_id}')
                continue
            derived.extend(rule.function(fact_pack))

        deduped = {
            fact.fact_id: fact
            for fact in derived
        }
        return DatalogReceipt(
            engine='python-reference-datalog',
            fact_pack_hash=fact_pack.pack_hash,
            rule_ids=selected_ids,
            derived_facts=tuple(sorted(deduped.values(), key=lambda fact: (fact.rule_id, fact.subject_id, fact.fact_id))),
            warnings=tuple(warnings),
        )

    def derive_from_models(
        self,
        *,
        object_ids: Iterable[int | str] | None = None,
        claim_ids: Iterable[int | str] | None = None,
        artifact_ids: Iterable[int | str] | None = None,
        rule_ids: Iterable[str] | None = None,
    ) -> DatalogReceipt:
        fact_pack = build_fact_pack_from_models(
            object_ids=object_ids,
            claim_ids=claim_ids,
            artifact_ids=artifact_ids,
        )
        return self.derive(fact_pack, rule_ids=rule_ids)

