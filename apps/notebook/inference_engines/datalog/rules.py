"""First-pass symbolic rules for the Datalog reference engine."""

from __future__ import annotations

import re
from collections import defaultdict
from typing import Iterable

from .contracts import DatalogFact, DatalogFactPack, DerivedFact, RuleDefinition


def _normalize_title(value: str) -> str:
    return re.sub(r'\s+', ' ', re.sub(r'[^a-z0-9]+', ' ', value.lower())).strip()


def _truthy(value) -> bool:
    if isinstance(value, bool):
        return value
    if isinstance(value, str):
        return value.lower() in {'1', 'true', 'yes', 'private', 'public'}
    return bool(value)


def _coerce_year(value) -> int | None:
    """Extract a 4-digit year from ints, floats, or messy strings.

    Civic sources phrase dates many ways ('1955', 'circa 1955', '1955-06-01',
    1955). This pulls the first 4-digit year so the rule can compare them as
    integers. Returns None when no year is present.
    """
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value)
    match = re.search(r'(\d{4})', str(value))
    return int(match.group(1)) if match else None


def _facts_by_id(facts: Iterable[DatalogFact]) -> dict[str, DatalogFact]:
    return {fact.entity_id: fact for fact in facts}


def _claim_support_indexes(index: dict[str, tuple[DatalogFact, ...]]) -> tuple[dict[str, list[DatalogFact]], dict[str, list[DatalogFact]]]:
    support_evidence: dict[str, list[DatalogFact]] = defaultdict(list)
    dependencies: dict[str, list[DatalogFact]] = defaultdict(list)
    for link in index.get('evidence_link', ()):
        relation_type = str(link.attr('relation_type', '')).lower()
        if relation_type in {'supports', 'derived_from', 'cites', 'references'}:
            support_evidence[str(link.attr('claim_id', ''))].append(link)
    for dep in index.get('claim_dependency', ()):
        dependencies[str(dep.attr('claim_id', ''))].append(dep)
    return support_evidence, dependencies


def unsupported_claim(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    support_evidence, dependencies = _claim_support_indexes(index)
    out: list[DerivedFact] = []
    for claim in index.get('claim', ()):
        status = str(claim.attr('status', '')).lower()
        if status in {'archived', 'refuted', 'superseded'}:
            continue
        claim_id = claim.entity_id
        if not support_evidence.get(claim_id) and not dependencies.get(claim_id):
            out.append(DerivedFact(
                rule_id='unsupported_claim',
                relation='unsupported_claim',
                subject_id=claim_id,
                reason='This claim has no supporting EvidenceLink or ClaimDependency in the current fact pack.',
                dependency_fact_ids=(claim.fact_id,),
                attributes={'status': claim.attr('status', '')},
            ))
    return tuple(out)


def dependent_claim(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    out: list[DerivedFact] = []
    for dep in index.get('claim_dependency', ()):
        claim_id = str(dep.attr('claim_id', ''))
        if not claim_id:
            continue
        out.append(DerivedFact(
            rule_id='dependent_claim',
            relation='dependent_claim',
            subject_id=claim_id,
            reason='This claim depends on another graph object for its justification.',
            dependency_fact_ids=(dep.fact_id,),
            attributes={
                'depends_on_object_id': dep.attr('depends_on_object_id', ''),
                'justification_type': dep.attr('justification_type', ''),
                'strength': dep.attr('strength', 0.0),
            },
        ))
    return tuple(out)


def source_reused_support(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    by_source: dict[str, list[DatalogFact]] = defaultdict(list)
    for dep in index.get('claim_dependency', ()):
        source_id = str(dep.attr('depends_on_object_id', ''))
        if source_id:
            by_source[f'object:{source_id}'].append(dep)
    for link in index.get('evidence_link', ()):
        artifact_id = str(link.attr('artifact_id', ''))
        if artifact_id:
            by_source[f'artifact:{artifact_id}'].append(link)

    out: list[DerivedFact] = []
    for source_id, facts in by_source.items():
        claim_ids = sorted({str(fact.attr('claim_id', '')) for fact in facts if fact.attr('claim_id', '')})
        if len(claim_ids) < 2:
            continue
        out.append(DerivedFact(
            rule_id='source_reused_support',
            relation='source_reused_support',
            subject_id=source_id,
            reason='The same source is reused as support for multiple claims.',
            dependency_fact_ids=tuple(fact.fact_id for fact in facts),
            attributes={'claim_ids': claim_ids},
            confidence=0.8,
        ))
    return tuple(out)


def likely_duplicate_entity(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    by_title: dict[str, list[DatalogFact]] = defaultdict(list)
    for obj in index.get('object', ()):
        title = _normalize_title(str(obj.attr('title', '')))
        if title:
            by_title[title].append(obj)

    out: list[DerivedFact] = []
    for title, objects in by_title.items():
        if len(objects) < 2:
            continue
        out.append(DerivedFact(
            rule_id='likely_duplicate_entity',
            relation='likely_duplicate_entity',
            subject_id=objects[0].entity_id,
            reason='Multiple graph objects share the same normalized title.',
            dependency_fact_ids=tuple(obj.fact_id for obj in objects),
            attributes={
                'normalized_title': title,
                'duplicate_object_ids': [obj.entity_id for obj in objects[1:]],
            },
            confidence=0.7,
            writeback_policy='proposal-only',
        ))
    return tuple(out)


def evidence_path_too_long(fact_pack: DatalogFactPack, *, max_length: int = 3) -> tuple[DerivedFact, ...]:
    out: list[DerivedFact] = []
    for path in fact_pack.by_relation().get('evidence_path', ()):
        path_length = int(path.attr('path_length', 0) or 0)
        if path_length <= max_length:
            continue
        out.append(DerivedFact(
            rule_id='evidence_path_too_long',
            relation='evidence_path_too_long',
            subject_id=path.entity_id,
            reason='The evidence path exceeds the configured symbolic derivation depth.',
            dependency_fact_ids=(path.fact_id,),
            attributes={'path_length': path_length, 'max_length': max_length},
            confidence=0.9,
        ))
    return tuple(out)


def claim_has_no_independent_support(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    support_evidence, dependencies = _claim_support_indexes(index)
    out: list[DerivedFact] = []
    for claim in index.get('claim', ()):
        claim_id = claim.entity_id
        source_refs = {
            f'object:{dep.attr("depends_on_object_id")}'
            for dep in dependencies.get(claim_id, ())
            if dep.attr('depends_on_object_id', '')
        }
        source_refs.update({
            f'artifact:{link.attr("artifact_id")}'
            for link in support_evidence.get(claim_id, ())
            if link.attr('artifact_id', '')
        })
        if len(source_refs) >= 2:
            continue
        deps = tuple(
            [claim.fact_id]
            + [fact.fact_id for fact in dependencies.get(claim_id, ())]
            + [fact.fact_id for fact in support_evidence.get(claim_id, ())]
        )
        out.append(DerivedFact(
            rule_id='claim_has_no_independent_support',
            relation='claim_has_no_independent_support',
            subject_id=claim_id,
            reason='This claim does not have two independent support sources in the current fact pack.',
            dependency_fact_ids=deps,
            attributes={'support_source_count': len(source_refs), 'support_sources': sorted(source_refs)},
            confidence=0.85,
        ))
    return tuple(out)


def object_in_unresolved_tension_neighborhood(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    out: list[DerivedFact] = []
    for edge in fact_pack.by_relation().get('edge', ()):
        edge_type = str(edge.attr('edge_type', '')).lower()
        status = str(edge.attr('acceptance_status', '')).lower()
        if edge_type != 'contradicts' and status != 'contested':
            continue
        for object_id in (edge.attr('from_object_id', ''), edge.attr('to_object_id', '')):
            if object_id:
                out.append(DerivedFact(
                    rule_id='object_in_unresolved_tension_neighborhood',
                    relation='object_in_unresolved_tension_neighborhood',
                    subject_id=str(object_id),
                    reason='This object is adjacent to a contradicting or contested edge.',
                    dependency_fact_ids=(edge.fact_id,),
                    attributes={'edge_id': edge.entity_id, 'edge_type': edge_type, 'acceptance_status': status},
                    confidence=0.8,
                ))
    return tuple(out)


def code_symbol_touched_by_failing_postmortem_pattern(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    out: list[DerivedFact] = []
    for atom in fact_pack.by_relation().get('context_atom', ()):
        metadata = atom.attr('metadata', {}) or {}
        if atom.attr('kind', '') != 'code_symbol':
            continue
        if not any(_truthy(metadata.get(key)) for key in ('failing_postmortem_pattern', 'postmortem_failure', 'failed_tests')):
            continue
        out.append(DerivedFact(
            rule_id='code_symbol_touched_by_failing_postmortem_pattern',
            relation='code_symbol_touched_by_failing_postmortem_pattern',
            subject_id=atom.entity_id,
            reason='This code symbol is linked to a failing postmortem or failed-test pattern.',
            dependency_fact_ids=(atom.fact_id,),
            attributes={'title': atom.attr('title', '')},
            confidence=0.85,
        ))
    return tuple(out)


def context_atom_tainted_by_generated_artifact(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    out: list[DerivedFact] = []
    for atom in fact_pack.by_relation().get('context_atom', ()):
        metadata = atom.attr('metadata', {}) or {}
        generated = (
            _truthy(metadata.get('generated'))
            or str(metadata.get('source_kind', '')).lower() == 'generated_artifact'
            or str(metadata.get('provenance', '')).lower() == 'generated'
        )
        if not generated:
            continue
        out.append(DerivedFact(
            rule_id='context_atom_tainted_by_generated_artifact',
            relation='context_atom_tainted_by_generated_artifact',
            subject_id=atom.entity_id,
            reason='This context atom was derived from generated material and should not be treated as independent evidence.',
            dependency_fact_ids=(atom.fact_id,),
            attributes={'artifact_id': atom.attr('artifact_id', '')},
            confidence=0.9,
        ))
    return tuple(out)


def private_source_reaches_export_candidate(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    index = fact_pack.by_relation()
    objects = _facts_by_id(index.get('object', ()))
    out: list[DerivedFact] = []
    for atom in index.get('context_atom', ()):
        metadata = atom.attr('metadata', {}) or {}
        object_id = str(atom.attr('object_pk', '') or '')
        properties = (objects.get(object_id).attr('properties', {}) if object_id in objects else {}) or {}
        is_private = (
            _truthy(metadata.get('private'))
            or str(metadata.get('source_visibility', '')).lower() == 'private'
            or _truthy(properties.get('private'))
            or str(properties.get('visibility', '')).lower() == 'private'
        )
        is_export_candidate = (
            _truthy(metadata.get('export_candidate'))
            or _truthy(metadata.get('public_export'))
            or str(metadata.get('export_visibility', '')).lower() == 'public'
        )
        if not (is_private and is_export_candidate):
            continue
        dependency_ids = [atom.fact_id]
        if object_id in objects:
            dependency_ids.append(objects[object_id].fact_id)
        out.append(DerivedFact(
            rule_id='private_source_reaches_export_candidate',
            relation='private_source_reaches_export_candidate',
            subject_id=atom.entity_id,
            reason='A private source is marked as a public export candidate.',
            dependency_fact_ids=tuple(dependency_ids),
            attributes={'object_pk': object_id, 'artifact_id': atom.attr('artifact_id', '')},
            confidence=0.95,
            writeback_policy='proposal-only',
        ))
    return tuple(out)


def demolition_window(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    """Derive a demolition window from present-then-absent structure observations.

    Civic fact vocabulary consumed (the civic fact-relation vocabulary):
      structure_present : a structure stood at a parcel in a given year.
      structure_absent  : the parcel had no structure in a given year.
    Both carry attributes parcel_id (the subject parcel), year, and source_id
    (the surveying source, e.g. 'sanborn:1955:sheet-12'). When parcel_id is
    absent the fact's entity_id is used as the parcel.

    For each parcel observed present at year y1 and absent at a later year y2,
    derives demolition_between(parcel) over the tightest (y1, y2] bracket: the
    latest present year strictly before the earliest absent year that follows a
    present observation. The derivation is exact (a structure present then absent
    was removed in between); only the date range is uncertain, so confidence
    stays 1.0 and the uncertainty lives in window_years. dependency_fact_ids
    carries the supporting fact ids so the consumer can render the proof as
    provenance.
    """
    index = fact_pack.by_relation()

    def _by_parcel(relation: str) -> dict[str, list[tuple[int, DatalogFact]]]:
        grouped: dict[str, list[tuple[int, DatalogFact]]] = defaultdict(list)
        for fact in index.get(relation, ()):
            parcel = str(fact.attr('parcel_id', '') or fact.entity_id).strip()
            year = _coerce_year(fact.attr('year'))
            if parcel and year is not None:
                grouped[parcel].append((year, fact))
        return grouped

    present = _by_parcel('structure_present')
    absent = _by_parcel('structure_absent')

    out: list[DerivedFact] = []
    for parcel in sorted(present):
        absent_obs = absent.get(parcel)
        if not absent_obs:
            continue
        present_years = sorted({year for year, _ in present[parcel]})
        earliest_absent = None  # earliest absent year that follows a present observation
        for absent_year in sorted({year for year, _ in absent_obs}):
            if any(py < absent_year for py in present_years):
                earliest_absent = absent_year
                break
        if earliest_absent is None:
            continue
        latest_present = max(py for py in present_years if py < earliest_absent)

        present_facts = [fact for year, fact in present[parcel] if year == latest_present]
        absent_facts = [fact for year, fact in absent_obs if year == earliest_absent]
        present_sources = sorted({str(f.attr('source_id', '')) for f in present_facts if f.attr('source_id', '')})
        absent_sources = sorted({str(f.attr('source_id', '')) for f in absent_facts if f.attr('source_id', '')})
        present_label = ', '.join(present_sources) or 'an earlier survey'
        absent_label = ', '.join(absent_sources) or 'a later survey'

        out.append(DerivedFact(
            rule_id='demolition_window',
            relation='demolition_between',
            subject_id=parcel,
            reason=(
                f'A structure stood at parcel {parcel} in {latest_present} '
                f'(per {present_label}) and was absent by {earliest_absent} '
                f'(per {absent_label}); demolition occurred between '
                f'{latest_present} and {earliest_absent}.'
            ),
            dependency_fact_ids=(
                tuple(f.fact_id for f in present_facts)
                + tuple(f.fact_id for f in absent_facts)
            ),
            attributes={
                'parcel_id': parcel,
                'earliest_year': latest_present,
                'latest_year': earliest_absent,
                'window_years': earliest_absent - latest_present,
                'present_source_ids': present_sources,
                'absent_source_ids': absent_sources,
            },
        ))
    return tuple(out)


def conflict_set(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    """Detect sources that disagree on the same parcel attribute.

    Civic fact vocabulary consumed:
      source_assertion : a source asserts a value for a parcel attribute.
        attributes parcel_id, field (e.g. 'build_year'), value, source_id.

    Groups assertions by (parcel_id, field). When two or more distinct values
    are asserted for the same attribute, emits source_conflict carrying every
    conflicting assertion as a dependency. This derived conflict set is the
    structural input to the probabilistic.source_reliability affordance: exact
    logic finds the disagreement, probability resolves which source to trust.
    """
    index = fact_pack.by_relation()
    grouped: dict[tuple[str, str], list[DatalogFact]] = defaultdict(list)
    for fact in index.get('source_assertion', ()):
        parcel = str(fact.attr('parcel_id', '') or fact.entity_id).strip()
        field = str(fact.attr('field', '')).strip()
        if parcel and field:
            grouped[(parcel, field)].append(fact)

    out: list[DerivedFact] = []
    for (parcel, field), assertions in grouped.items():
        values_to_sources: dict[str, list[str]] = defaultdict(list)
        for fact in assertions:
            value = str(fact.attr('value', '')).strip()
            if not value:
                continue
            values_to_sources[value].append(str(fact.attr('source_id', '')) or fact.entity_id)
        if len(values_to_sources) < 2:
            continue
        rendered = '; '.join(
            f'{value} (per {", ".join(sorted(set(sources)))})'
            for value, sources in sorted(values_to_sources.items())
        )
        out.append(DerivedFact(
            rule_id='conflict_set',
            relation='source_conflict',
            subject_id=f'{parcel}:{field}',
            reason=f'Sources disagree on {field} for parcel {parcel}: {rendered}.',
            dependency_fact_ids=tuple(sorted(fact.fact_id for fact in assertions)),
            attributes={
                'parcel_id': parcel,
                'field': field,
                'values': {value: sorted(set(sources)) for value, sources in values_to_sources.items()},
                'source_ids': sorted({str(fact.attr('source_id', '')) or fact.entity_id for fact in assertions}),
                'distinct_value_count': len(values_to_sources),
            },
        ))
    return tuple(out)


def vacancy_duration(fact_pack: DatalogFactPack, *, min_years: int = 5) -> tuple[DerivedFact, ...]:
    """Flag parcels assessed vacant across a prolonged span.

    Civic fact vocabulary consumed:
      assessor_status : the assessed status of a parcel in a year.
        attributes parcel_id, status ('vacant'/'occupied'/'demolished'/...),
        year, source_id.

    Walks each parcel's status observations chronologically and finds the longest
    run of 'vacant' uninterrupted by an 'occupied' observation. Emits
    prolonged_vacancy when the run spans at least min_years. Other statuses
    (demolished, unknown) neither extend nor break the run.
    """
    index = fact_pack.by_relation()
    by_parcel: dict[str, list[tuple[int, str, DatalogFact]]] = defaultdict(list)
    for fact in index.get('assessor_status', ()):
        parcel = str(fact.attr('parcel_id', '') or fact.entity_id).strip()
        year = _coerce_year(fact.attr('year'))
        status = str(fact.attr('status', '')).strip().lower()
        if parcel and year is not None and status:
            by_parcel[parcel].append((year, status, fact))

    out: list[DerivedFact] = []
    for parcel, observations in by_parcel.items():
        observations.sort(key=lambda item: item[0])
        best_run: list[tuple[int, str, DatalogFact]] = []
        current: list[tuple[int, str, DatalogFact]] = []
        for year, status, fact in observations:
            if status == 'vacant':
                current.append((year, status, fact))
            elif status == 'occupied':
                if len(current) > len(best_run):
                    best_run = current
                current = []
        if len(current) > len(best_run):
            best_run = current
        if not best_run:
            continue
        earliest = best_run[0][0]
        latest = best_run[-1][0]
        span = latest - earliest
        if span < min_years:
            continue
        run_facts = [fact for _, _, fact in best_run]
        sources = sorted({str(f.attr('source_id', '')) for f in run_facts if f.attr('source_id', '')})
        out.append(DerivedFact(
            rule_id='vacancy_duration',
            relation='prolonged_vacancy',
            subject_id=parcel,
            reason=(
                f'Parcel {parcel} was assessed vacant from {earliest} to {latest} '
                f'({span} years) with no intervening occupied record.'
            ),
            dependency_fact_ids=tuple(sorted(f.fact_id for f in run_facts)),
            attributes={
                'parcel_id': parcel,
                'earliest_year': earliest,
                'latest_year': latest,
                'vacant_years': span,
                'min_years': min_years,
                'observation_count': len(best_run),
                'source_ids': sources,
            },
        ))
    return tuple(out)


def ownership_chain(fact_pack: DatalogFactPack) -> tuple[DerivedFact, ...]:
    """Order a parcel's ownership records into a lineage and flag distressed acquisition.

    Civic fact vocabulary consumed:
      ownership : a recorded owner of a parcel over a period. attributes
        parcel_id, owner, owner_type ('private'/'tax_foreclosure'/'land_bank'/...),
        from_year, to_year (optional), source_id.

    For each parcel with two or more ownership records, emits ownership_chain
    carrying the chronologically ordered lineage as proof. When a tax_foreclosure
    owner precedes a land_bank owner, flags tax_foreclosure_to_land_bank with the
    foreclosure and acquisition years (the distressed-acquisition pattern a land
    bank tracks). Records missing a from_year sort last while preserving order.
    """
    index = fact_pack.by_relation()
    by_parcel: dict[str, list[DatalogFact]] = defaultdict(list)
    for fact in index.get('ownership', ()):
        parcel = str(fact.attr('parcel_id', '') or fact.entity_id).strip()
        if parcel:
            by_parcel[parcel].append(fact)

    out: list[DerivedFact] = []
    for parcel, records in by_parcel.items():
        if len(records) < 2:
            continue
        dated = [(_coerce_year(rec.attr('from_year')), rec) for rec in records]
        ordered = [rec for _, rec in sorted(dated, key=lambda pair: (pair[0] is None, pair[0] or 0, pair[1].fact_id))]
        chain = [
            {
                'owner': str(rec.attr('owner', '')),
                'owner_type': str(rec.attr('owner_type', '')).strip().lower(),
                'from_year': _coerce_year(rec.attr('from_year')),
                'to_year': _coerce_year(rec.attr('to_year')),
                'source_id': str(rec.attr('source_id', '')),
            }
            for rec in ordered
        ]
        foreclosure_year = None
        land_bank_year = None
        for link in chain:
            if link['owner_type'] == 'tax_foreclosure' and foreclosure_year is None:
                foreclosure_year = link['from_year']
            elif link['owner_type'] == 'land_bank' and foreclosure_year is not None and land_bank_year is None:
                land_bank_year = link['from_year']
        distressed = foreclosure_year is not None and land_bank_year is not None

        owners_label = ', '.join(
            (link['owner'] or link['owner_type'] or 'unknown')
            + (f" ({link['from_year']})" if link['from_year'] is not None else '')
            for link in chain
        )
        reason = f'Parcel {parcel} passed through {len(chain)} owners: {owners_label}.'
        if distressed:
            reason += f' Acquired by the land bank via tax foreclosure ({foreclosure_year} to {land_bank_year}).'

        out.append(DerivedFact(
            rule_id='ownership_chain',
            relation='ownership_chain',
            subject_id=parcel,
            reason=reason,
            dependency_fact_ids=tuple(sorted(rec.fact_id for rec in records)),
            attributes={
                'parcel_id': parcel,
                'owner_count': len(chain),
                'chain': chain,
                'tax_foreclosure_to_land_bank': distressed,
                'foreclosure_year': foreclosure_year,
                'land_bank_year': land_bank_year,
            },
        ))
    return tuple(out)


CIVIC_RULES = (
    RuleDefinition(
        'demolition_window',
        'A structure present then absent implies demolition in the intervening window.',
        demolition_window,
    ),
    RuleDefinition(
        'conflict_set',
        'Sources that disagree on the same parcel attribute form a conflict set.',
        conflict_set,
    ),
    RuleDefinition(
        'vacancy_duration',
        'A parcel assessed vacant across a multi-year span is flagged as prolonged vacancy.',
        vacancy_duration,
    ),
    RuleDefinition(
        'ownership_chain',
        'A parcel ownership lineage, flagging tax foreclosure to land bank acquisition.',
        ownership_chain,
    ),
)


DEFAULT_RULES = (
    RuleDefinition('unsupported_claim', 'Claim lacks direct supporting evidence or dependencies.', unsupported_claim),
    RuleDefinition('dependent_claim', 'Claim has an explicit dependency edge to another object.', dependent_claim),
    RuleDefinition('source_reused_support', 'One source supports multiple claims.', source_reused_support),
    RuleDefinition('likely_duplicate_entity', 'Objects share a normalized title.', likely_duplicate_entity),
    RuleDefinition('evidence_path_too_long', 'Evidence path exceeds configured derivation depth.', evidence_path_too_long),
    RuleDefinition('claim_has_no_independent_support', 'Claim has fewer than two independent supports.', claim_has_no_independent_support),
    RuleDefinition('object_in_unresolved_tension_neighborhood', 'Object touches a contradicting or contested edge.', object_in_unresolved_tension_neighborhood),
    RuleDefinition('code_symbol_touched_by_failing_postmortem_pattern', 'Code atom is linked to failing postmortem/test patterns.', code_symbol_touched_by_failing_postmortem_pattern),
    RuleDefinition('context_atom_tainted_by_generated_artifact', 'Context atom derives from generated material.', context_atom_tainted_by_generated_artifact),
    RuleDefinition('private_source_reaches_export_candidate', 'Private source is marked for public export.', private_source_reaches_export_candidate),
) + CIVIC_RULES

