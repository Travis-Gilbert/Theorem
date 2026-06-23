"""Contracts for read-only symbolic derivation receipts."""

from __future__ import annotations

import hashlib
import json
from dataclasses import dataclass, field
from typing import Any, Iterable


def _stable_json(value: Any) -> str:
    return json.dumps(value, sort_keys=True, separators=(',', ':'), default=str)


def _stable_hash(value: Any) -> str:
    return hashlib.sha256(_stable_json(value).encode('utf-8')).hexdigest()


@dataclass(frozen=True, slots=True)
class DatalogFact:
    """A normalized relational fact consumed by the reference engine."""

    relation: str
    entity_id: str
    attributes: dict[str, Any] = field(default_factory=dict)
    source_ref: str = ''
    fact_id: str = ''

    def __post_init__(self) -> None:
        if not str(self.relation or '').strip():
            raise ValueError('DatalogFact requires relation')
        if not str(self.entity_id or '').strip():
            raise ValueError('DatalogFact requires entity_id')
        if not self.fact_id:
            object.__setattr__(
                self,
                'fact_id',
                _stable_hash({
                    'relation': self.relation,
                    'entity_id': str(self.entity_id),
                    'attributes': self.attributes,
                    'source_ref': self.source_ref,
                }),
            )

    def attr(self, key: str, default: Any = None) -> Any:
        return self.attributes.get(key, default)

    def to_dict(self) -> dict[str, Any]:
        return {
            'fact_id': self.fact_id,
            'relation': self.relation,
            'entity_id': str(self.entity_id),
            'attributes': dict(self.attributes),
            'source_ref': self.source_ref,
        }


@dataclass(frozen=True, slots=True)
class DatalogFactPack:
    """Immutable set of normalized facts with a deterministic pack hash."""

    facts: tuple[DatalogFact, ...] = ()
    source: str = 'in-memory'
    pack_hash: str = ''

    def __post_init__(self) -> None:
        facts = tuple(self.facts)
        object.__setattr__(self, 'facts', facts)
        if not self.pack_hash:
            object.__setattr__(
                self,
                'pack_hash',
                _stable_hash([fact.to_dict() for fact in sorted(facts, key=lambda item: item.fact_id)]),
            )

    def by_relation(self) -> dict[str, tuple[DatalogFact, ...]]:
        index: dict[str, list[DatalogFact]] = {}
        for fact in self.facts:
            index.setdefault(fact.relation, []).append(fact)
        return {
            relation: tuple(sorted(items, key=lambda item: item.fact_id))
            for relation, items in index.items()
        }

    def to_dict(self) -> dict[str, Any]:
        return {
            'source': self.source,
            'pack_hash': self.pack_hash,
            'facts': [fact.to_dict() for fact in self.facts],
        }


@dataclass(frozen=True, slots=True)
class DerivedFact:
    """An explainable read-only consequence derived from base facts."""

    rule_id: str
    relation: str
    subject_id: str
    reason: str
    dependency_fact_ids: tuple[str, ...] = ()
    attributes: dict[str, Any] = field(default_factory=dict)
    confidence: float = 1.0
    writeback_policy: str = 'read-only'
    fact_id: str = ''

    def __post_init__(self) -> None:
        if not self.rule_id:
            raise ValueError('DerivedFact requires rule_id')
        if not self.relation:
            raise ValueError('DerivedFact requires relation')
        if not self.subject_id:
            raise ValueError('DerivedFact requires subject_id')
        if not self.reason:
            raise ValueError('DerivedFact requires plain-English reason')
        object.__setattr__(self, 'dependency_fact_ids', tuple(self.dependency_fact_ids))
        if not self.fact_id:
            object.__setattr__(
                self,
                'fact_id',
                _stable_hash({
                    'rule_id': self.rule_id,
                    'relation': self.relation,
                    'subject_id': str(self.subject_id),
                    'attributes': self.attributes,
                    'dependency_fact_ids': self.dependency_fact_ids,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'fact_id': self.fact_id,
            'rule_id': self.rule_id,
            'relation': self.relation,
            'subject_id': str(self.subject_id),
            'reason': self.reason,
            'dependency_fact_ids': list(self.dependency_fact_ids),
            'attributes': dict(self.attributes),
            'confidence': float(self.confidence),
            'writeback_policy': self.writeback_policy,
        }


@dataclass(frozen=True, slots=True)
class RuleDefinition:
    """Reference-engine rule contract."""

    rule_id: str
    description: str
    function: Any


@dataclass(frozen=True, slots=True)
class DatalogReceipt:
    """Auditable receipt for one symbolic derivation run."""

    engine: str
    fact_pack_hash: str
    rule_ids: tuple[str, ...]
    derived_facts: tuple[DerivedFact, ...]
    warnings: tuple[str, ...] = ()

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'fact_pack_hash': self.fact_pack_hash,
            'rule_ids': list(self.rule_ids),
            'derived_count': len(self.derived_facts),
            'derived_facts': [fact.to_dict() for fact in self.derived_facts],
            'warnings': list(self.warnings),
            'writeback_policy': 'read-only',
        }


def fact_pack_from_iterable(facts: Iterable[DatalogFact], *, source: str = 'in-memory') -> DatalogFactPack:
    return DatalogFactPack(facts=tuple(facts), source=source)

