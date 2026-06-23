"""Substrate affordance wrappers over existing inference engines.

The wrappers keep engine cores unchanged and add the projection metadata that a
CommonPlace/RustyRed participant or benchmark ledger needs: input identity,
input node refs, provenance, and a stable top-level receipt hash.
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Iterable, Mapping, Sequence

from apps.notebook.inference_engines.common import stable_hash
from apps.notebook.inference_engines.datalog.contracts import DatalogFactPack
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.runtime_adapters import (
    expected_value_from_validator_records,
    source_reliability_from_records,
)


Record = Mapping[str, Any] | Any


def _datalog_engine():
    from apps.notebook.inference_engines.datalog.native import NativeDatalogEngine

    return NativeDatalogEngine()


def _probabilistic_engine():
    from apps.notebook.inference_engines.probabilistic.native import NativeProbProgEngine

    return NativeProbProgEngine()


@dataclass(frozen=True, slots=True)
class AffordanceReceipt:
    """Uniform substrate wrapper around engine-specific receipt payloads."""

    engine_id: str
    affordance_id: str
    input_hash: str
    payload: dict[str, Any]
    input_node_refs: tuple[str, ...] = ()
    writeback_policy: str = 'read-only'
    provenance: dict[str, Any] = field(default_factory=dict)
    metadata: dict[str, Any] = field(default_factory=dict)
    receipt_hash: str = ''

    def __post_init__(self) -> None:
        object.__setattr__(self, 'input_node_refs', tuple(self.input_node_refs))
        object.__setattr__(self, 'payload', dict(self.payload))
        object.__setattr__(self, 'provenance', dict(self.provenance))
        object.__setattr__(self, 'metadata', dict(self.metadata))
        if not self.receipt_hash:
            object.__setattr__(
                self,
                'receipt_hash',
                stable_hash({
                    'engine_id': self.engine_id,
                    'affordance_id': self.affordance_id,
                    'input_hash': self.input_hash,
                    'input_node_refs': list(self.input_node_refs),
                    'payload': self.payload,
                    'writeback_policy': self.writeback_policy,
                    'provenance': self.provenance,
                    'metadata': self.metadata,
                }),
            )

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine_id': self.engine_id,
            'affordance_id': self.affordance_id,
            'receipt_hash': self.receipt_hash,
            'input_hash': self.input_hash,
            'input_node_refs': list(self.input_node_refs),
            'payload': dict(self.payload),
            'writeback_policy': self.writeback_policy,
            'provenance': dict(self.provenance),
            'metadata': dict(self.metadata),
        }

    def __getitem__(self, key: str) -> Any:
        return self.to_dict()[key]

    def get(self, key: str, default: Any = None) -> Any:
        return self.to_dict().get(key, default)


@dataclass(frozen=True, slots=True)
class SubstrateFactPackInput:
    """Record-shaped substrate input for Datalog fact-pack assembly."""

    objects: tuple[Record, ...] = ()
    claims: tuple[Record, ...] = ()
    evidence_links: tuple[Record, ...] = ()
    context_atoms: tuple[Record, ...] = ()
    claim_dependencies: tuple[Record, ...] = ()
    edges: tuple[Record, ...] = ()
    evidence_paths: tuple[Record, ...] = ()
    node_refs: tuple[str, ...] = ()
    source: str = 'substrate'
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_value(cls, value: 'SubstrateFactPackInput | Mapping[str, Any]') -> 'SubstrateFactPackInput':
        if isinstance(value, cls):
            return value
        explicit = _explicit_datalog_groups(value)
        node_groups = _groups_from_substrate_nodes(value.get('nodes', ()))
        groups = {
            key: tuple(explicit.get(key, ())) + tuple(node_groups.get(key, ()))
            for key in _DATALOG_GROUP_KEYS
        }
        return cls(
            **groups,
            node_refs=tuple(value.get('node_refs') or _node_refs_from_groups(groups.values())),
            source=str(value.get('source') or 'substrate'),
            metadata=dict(value.get('metadata') or {}),
        )


@dataclass(frozen=True, slots=True)
class SubstrateEvidenceInput:
    """Substrate evidence records for probabilistic source reliability."""

    evidence_records: tuple[Record, ...] = ()
    node_refs: tuple[str, ...] = ()
    source: str = 'substrate'
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_value(cls, value: 'SubstrateEvidenceInput | Iterable[Record] | Mapping[str, Any]') -> 'SubstrateEvidenceInput':
        if isinstance(value, cls):
            return value
        if isinstance(value, Mapping):
            records = tuple(value.get('evidence_records') or value.get('records') or ())
            return cls(
                evidence_records=records,
                node_refs=tuple(value.get('node_refs') or _node_refs_from_records(records)),
                source=str(value.get('source') or 'substrate'),
                metadata=dict(value.get('metadata') or {}),
            )
        records = tuple(value)
        return cls(evidence_records=records, node_refs=_node_refs_from_records(records))


@dataclass(frozen=True, slots=True)
class SubstrateValidatorInput:
    """Substrate validator records for expected-value-of-information."""

    validator_records: tuple[Record, ...] = ()
    node_refs: tuple[str, ...] = ()
    source: str = 'substrate'
    metadata: dict[str, Any] = field(default_factory=dict)

    @classmethod
    def from_value(cls, value: 'SubstrateValidatorInput | Iterable[Record] | Mapping[str, Any]') -> 'SubstrateValidatorInput':
        if isinstance(value, cls):
            return value
        if isinstance(value, Mapping):
            records = tuple(value.get('validator_records') or value.get('records') or ())
            return cls(
                validator_records=records,
                node_refs=tuple(value.get('node_refs') or _node_refs_from_records(records)),
                source=str(value.get('source') or 'substrate'),
                metadata=dict(value.get('metadata') or {}),
            )
        records = tuple(value)
        return cls(validator_records=records, node_refs=_node_refs_from_records(records))


def build_fact_pack_from_substrate(
    substrate_input: SubstrateFactPackInput | Mapping[str, Any],
) -> DatalogFactPack:
    """Normalize substrate node/record sets into the Datalog fact-pack contract."""

    resolved = SubstrateFactPackInput.from_value(substrate_input)
    return build_fact_pack_from_records(
        objects=resolved.objects,
        claims=resolved.claims,
        evidence_links=resolved.evidence_links,
        context_atoms=resolved.context_atoms,
        claim_dependencies=resolved.claim_dependencies,
        edges=resolved.edges,
        evidence_paths=resolved.evidence_paths,
        source=resolved.source,
    )


def run_datalog_affordance(
    substrate_input: SubstrateFactPackInput | DatalogFactPack | Mapping[str, Any],
    *,
    rule_ids: Sequence[str] = (),
) -> AffordanceReceipt:
    """Run Datalog as a substrate affordance and return a receipt-node payload."""

    if isinstance(substrate_input, DatalogFactPack):
        fact_pack = substrate_input
        input_node_refs = _node_refs_from_records(fact_pack.facts)
        metadata: dict[str, Any] = {}
    else:
        resolved = SubstrateFactPackInput.from_value(substrate_input)
        fact_pack = build_fact_pack_from_substrate(resolved)
        input_node_refs = resolved.node_refs or _node_refs_from_records(fact_pack.facts)
        metadata = dict(resolved.metadata)
    receipt = _datalog_engine().derive(fact_pack, rule_ids=rule_ids or None)
    payload = receipt.to_dict()
    return AffordanceReceipt(
        engine_id=payload['engine'],
        affordance_id='datalog.derive',
        input_hash=fact_pack.pack_hash,
        input_node_refs=tuple(input_node_refs),
        payload=payload,
        writeback_policy=str(payload.get('writeback_policy') or 'read-only'),
        provenance={
            'input_shape': 'substrate_fact_pack',
            'fact_pack_hash': fact_pack.pack_hash,
            'fact_count': len(fact_pack.facts),
            'source': fact_pack.source,
        },
        metadata=metadata,
    )


def run_probabilistic_source_reliability(
    substrate_input: SubstrateEvidenceInput | Iterable[Record] | Mapping[str, Any],
    *,
    source_id: str,
    prior_alpha: float = 1.0,
    prior_beta: float = 1.0,
) -> AffordanceReceipt:
    """Run source reliability as a substrate probabilistic affordance."""

    resolved = SubstrateEvidenceInput.from_value(substrate_input)
    payload = source_reliability_from_records(
        source_id=source_id,
        evidence_records=resolved.evidence_records,
        prior_alpha=prior_alpha,
        prior_beta=prior_beta,
        engine=_probabilistic_engine(),
    )
    input_hash = stable_hash({
        'source_id': source_id,
        'prior_alpha': prior_alpha,
        'prior_beta': prior_beta,
        'evidence_records': [_record_to_dict(record) for record in resolved.evidence_records],
    })
    return AffordanceReceipt(
        engine_id=str(payload['engine']),
        affordance_id='probabilistic.source_reliability',
        input_hash=input_hash,
        input_node_refs=resolved.node_refs,
        payload=payload,
        writeback_policy=str(payload.get('writeback_policy') or 'read-only'),
        provenance={
            'input_shape': 'substrate_evidence_records',
            'record_count': len(resolved.evidence_records),
            'source': resolved.source,
        },
        metadata=dict(resolved.metadata),
    )


def run_probabilistic_expected_value(
    substrate_input: SubstrateValidatorInput | Iterable[Record] | Mapping[str, Any],
    *,
    decision_value: float = 1.0,
) -> AffordanceReceipt:
    """Run expected value of information as a substrate probabilistic affordance."""

    resolved = SubstrateValidatorInput.from_value(substrate_input)
    payload = expected_value_from_validator_records(
        validator_records=resolved.validator_records,
        decision_value=decision_value,
        engine=_probabilistic_engine(),
    )
    input_hash = stable_hash({
        'decision_value': decision_value,
        'validator_records': [_record_to_dict(record) for record in resolved.validator_records],
    })
    return AffordanceReceipt(
        engine_id=str(payload['engine']),
        affordance_id='probabilistic.expected_value_of_information',
        input_hash=input_hash,
        input_node_refs=resolved.node_refs,
        payload=payload,
        writeback_policy=str(payload.get('writeback_policy') or 'read-only'),
        provenance={
            'input_shape': 'substrate_validator_records',
            'record_count': len(resolved.validator_records),
            'source': resolved.source,
        },
        metadata=dict(resolved.metadata),
    )


_DATALOG_GROUP_KEYS = (
    'objects',
    'claims',
    'evidence_links',
    'context_atoms',
    'claim_dependencies',
    'edges',
    'evidence_paths',
)

_NODE_TYPE_TO_GROUP = {
    'object': 'objects',
    'objects': 'objects',
    'claim': 'claims',
    'claims': 'claims',
    'evidence_link': 'evidence_links',
    'evidence_links': 'evidence_links',
    'context_atom': 'context_atoms',
    'context_atoms': 'context_atoms',
    'claim_dependency': 'claim_dependencies',
    'claim_dependencies': 'claim_dependencies',
    'edge': 'edges',
    'edges': 'edges',
    'evidence_path': 'evidence_paths',
    'evidence_paths': 'evidence_paths',
}


def _explicit_datalog_groups(value: Mapping[str, Any]) -> dict[str, tuple[Record, ...]]:
    return {
        key: tuple(value.get(key) or ())
        for key in _DATALOG_GROUP_KEYS
    }


def _groups_from_substrate_nodes(nodes: Iterable[Record]) -> dict[str, list[Record]]:
    groups: dict[str, list[Record]] = {key: [] for key in _DATALOG_GROUP_KEYS}
    for node in nodes:
        node_type = _record_type(node)
        group = _NODE_TYPE_TO_GROUP.get(node_type)
        if group:
            groups[group].append(node)
    return groups


def _record_type(record: Record) -> str:
    for key in ('record_type', 'node_type', 'substrate_type', 'relation'):
        value = _record_get(record, key)
        if value:
            return str(value).strip().lower()
    return ''


def _node_refs_from_groups(groups: Iterable[Iterable[Record]]) -> tuple[str, ...]:
    refs: list[str] = []
    for records in groups:
        refs.extend(_node_refs_from_records(records))
    return tuple(dict.fromkeys(refs))


def _node_refs_from_records(records: Iterable[Record]) -> tuple[str, ...]:
    refs = [_record_ref(record) for record in records]
    return tuple(dict.fromkeys(ref for ref in refs if ref))


def _record_ref(record: Record) -> str:
    for key in ('node_ref', 'node_id', 'source_ref', 'id', 'pk', 'fact_id'):
        value = _record_get(record, key)
        if value not in (None, ''):
            return str(value)
    return ''


def _record_get(record: Record, key: str, default: Any = None) -> Any:
    if isinstance(record, Mapping):
        return record.get(key, default)
    return getattr(record, key, default)


def _record_to_dict(record: Record) -> dict[str, Any]:
    if isinstance(record, Mapping):
        return dict(record)
    if hasattr(record, 'to_dict'):
        return dict(record.to_dict())
    return {
        key: value
        for key, value in vars(record).items()
        if not key.startswith('_')
    }
