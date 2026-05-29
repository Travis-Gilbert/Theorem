"""Fact-pack builders for existing graph, epistemic, and context models."""

from __future__ import annotations

from typing import Any, Iterable

from .contracts import DatalogFact, DatalogFactPack


def _get(record: Any, key: str, default: Any = None) -> Any:
    if isinstance(record, dict):
        return record.get(key, default)
    return getattr(record, key, default)


def _entity_id(record: Any) -> str:
    value = _get(record, 'id', None)
    if value is None:
        value = _get(record, 'pk', None)
    return str(value if value is not None else 'unknown')


def _display_title(record: Any) -> str:
    value = _get(record, 'display_title', None)
    if value:
        return str(value)
    return str(_get(record, 'title', '') or _get(record, 'text', '') or '')


def _json_dict(value: Any) -> dict[str, Any]:
    return dict(value) if isinstance(value, dict) else {}


def _fact(relation: str, entity_id: Any, attributes: dict[str, Any], source_ref: str) -> DatalogFact:
    return DatalogFact(
        relation=relation,
        entity_id=str(entity_id),
        attributes=attributes,
        source_ref=source_ref,
    )


def build_fact_pack_from_records(
    *,
    objects: Iterable[Any] = (),
    claims: Iterable[Any] = (),
    evidence_links: Iterable[Any] = (),
    context_atoms: Iterable[Any] = (),
    claim_dependencies: Iterable[Any] = (),
    edges: Iterable[Any] = (),
    evidence_paths: Iterable[Any] = (),
    source: str = 'records',
) -> DatalogFactPack:
    """Normalize model-like records into stable Datalog facts.

    Inputs may be Django model instances, values() dictionaries, or lightweight
    test records with matching attributes.
    """
    facts: list[DatalogFact] = []

    for obj in objects:
        entity_id = _entity_id(obj)
        facts.append(_fact('object', entity_id, {
            'title': _display_title(obj),
            'is_deleted': bool(_get(obj, 'is_deleted', False)),
            'acceptance_status': str(_get(obj, 'acceptance_status', '') or ''),
            'epistemic_role': str(_get(obj, 'epistemic_role', '') or ''),
            'properties': _json_dict(_get(obj, 'properties', {})),
        }, f'Object:{entity_id}'))

    for claim in claims:
        entity_id = _entity_id(claim)
        facts.append(_fact('claim', entity_id, {
            'source_object_id': str(_get(claim, 'source_object_id', '') or ''),
            'text': str(_get(claim, 'text', '') or ''),
            'status': str(_get(claim, 'status', '') or ''),
            'confidence': float(_get(claim, 'confidence', 0.0) or 0.0),
        }, f'Claim:{entity_id}'))

    for link in evidence_links:
        entity_id = _entity_id(link)
        facts.append(_fact('evidence_link', entity_id, {
            'claim_id': str(_get(link, 'claim_id', '') or ''),
            'artifact_id': str(_get(link, 'artifact_id', '') or ''),
            'relation_type': str(_get(link, 'relation_type', '') or ''),
            'confidence': float(_get(link, 'confidence', 0.0) or 0.0),
            'attached_by': str(_get(link, 'attached_by', '') or ''),
        }, f'EvidenceLink:{entity_id}'))

    for atom in context_atoms:
        entity_id = _entity_id(atom)
        facts.append(_fact('context_atom', entity_id, {
            'artifact_id': str(_get(atom, 'artifact_id', '') or ''),
            'kind': str(_get(atom, 'kind', '') or ''),
            'included': bool(_get(atom, 'included', False)),
            'title': str(_get(atom, 'title', '') or ''),
            'content_hash': str(_get(atom, 'content_hash', '') or ''),
            'object_pk': str(_get(atom, 'object_pk', '') or ''),
            'metadata': _json_dict(_get(atom, 'metadata', {})),
        }, f'ContextAtom:{entity_id}'))

    for dep in claim_dependencies:
        entity_id = _entity_id(dep)
        facts.append(_fact('claim_dependency', entity_id, {
            'claim_id': str(_get(dep, 'claim_id', '') or ''),
            'depends_on_object_id': str(_get(dep, 'depends_on_object_id', '') or ''),
            'via_edge_id': str(_get(dep, 'via_edge_id', '') or ''),
            'strength': float(_get(dep, 'strength', 0.0) or 0.0),
            'justification_group': str(_get(dep, 'justification_group', '') or ''),
            'justification_type': str(_get(dep, 'justification_type', '') or ''),
        }, f'ClaimDependency:{entity_id}'))

    for edge in edges:
        entity_id = _entity_id(edge)
        facts.append(_fact('edge', entity_id, {
            'from_object_id': str(_get(edge, 'from_object_id', '') or ''),
            'to_object_id': str(_get(edge, 'to_object_id', '') or ''),
            'edge_type': str(_get(edge, 'edge_type', '') or ''),
            'acceptance_status': str(_get(edge, 'acceptance_status', '') or ''),
            'strength': float(_get(edge, 'strength', 0.0) or 0.0),
        }, f'Edge:{entity_id}'))

    for path in evidence_paths:
        entity_id = _entity_id(path)
        edge_pks = _get(path, 'edge_pks', []) or []
        facts.append(_fact('evidence_path', entity_id, {
            'edge_pks': [str(item) for item in edge_pks],
            'path_length': len(edge_pks),
            'query_id': str(_get(path, 'query_id', '') or ''),
            'outcome': str(_get(path, 'outcome', '') or ''),
        }, f'EvidencePath:{entity_id}'))

    return DatalogFactPack(facts=tuple(facts), source=source)


def build_fact_pack_from_models(
    *,
    object_ids: Iterable[int | str] | None = None,
    claim_ids: Iterable[int | str] | None = None,
    artifact_ids: Iterable[int | str] | None = None,
    limit: int = 1000,
) -> DatalogFactPack:
    """Build a read-only fact pack from live Django model rows."""
    from django.db.models import Q

    from apps.notebook.models import (
        Claim,
        ClaimDependency,
        ContextAtom,
        Edge,
        EvidenceLink,
        Object,
    )

    object_id_values = [int(value) for value in (object_ids or ()) if str(value).isdigit()]
    claim_id_values = [int(value) for value in (claim_ids or ()) if str(value).isdigit()]
    artifact_id_values = [str(value) for value in (artifact_ids or ()) if str(value)]

    object_qs = Object.objects.filter(is_deleted=False)
    if object_id_values:
        object_qs = object_qs.filter(pk__in=object_id_values)
    objects = list(object_qs.only(
        'id', 'title', 'body', 'is_deleted', 'acceptance_status', 'epistemic_role', 'properties',
    )[:limit])

    claim_qs = Claim.objects.all()
    if claim_id_values:
        claim_qs = claim_qs.filter(pk__in=claim_id_values)
    elif object_id_values:
        claim_qs = claim_qs.filter(source_object_id__in=object_id_values)
    claims = list(claim_qs.only('id', 'source_object_id', 'text', 'status', 'confidence')[:limit])
    claim_ids_from_rows = [claim.pk for claim in claims]

    evidence_qs = EvidenceLink.objects.all()
    if claim_ids_from_rows:
        evidence_qs = evidence_qs.filter(claim_id__in=claim_ids_from_rows)
    elif artifact_id_values:
        evidence_qs = evidence_qs.filter(artifact_id__in=artifact_id_values)
    evidence_links = list(evidence_qs.only(
        'id', 'claim_id', 'artifact_id', 'relation_type', 'confidence', 'attached_by',
    )[:limit])

    dependency_qs = ClaimDependency.objects.all()
    if claim_ids_from_rows:
        dependency_qs = dependency_qs.filter(claim_id__in=claim_ids_from_rows)
    claim_dependencies = list(dependency_qs.only(
        'id', 'claim_id', 'depends_on_object_id', 'via_edge_id',
        'strength', 'justification_group', 'justification_type',
    )[:limit])

    atom_qs = ContextAtom.objects.all()
    if object_id_values or artifact_id_values:
        atom_filter = Q()
        if object_id_values:
            atom_filter |= Q(object_pk__in=object_id_values)
        if artifact_id_values:
            atom_filter |= Q(artifact_id__in=artifact_id_values)
        atom_qs = atom_qs.filter(atom_filter)
    context_atoms = list(atom_qs.only(
        'id', 'artifact_id', 'kind', 'included', 'title', 'content_hash', 'object_pk', 'metadata',
    )[:limit])

    edge_qs = Edge.objects.all()
    if object_id_values:
        edge_qs = edge_qs.filter(Q(from_object_id__in=object_id_values) | Q(to_object_id__in=object_id_values))
    edges = list(edge_qs.only(
        'id', 'from_object_id', 'to_object_id', 'edge_type', 'acceptance_status', 'strength',
    )[:limit])

    return build_fact_pack_from_records(
        objects=objects,
        claims=claims,
        evidence_links=evidence_links,
        context_atoms=context_atoms,
        claim_dependencies=claim_dependencies,
        edges=edges,
        source='django-models',
    )

