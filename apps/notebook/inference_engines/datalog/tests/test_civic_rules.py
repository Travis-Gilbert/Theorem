"""Tests for civic Datalog rules (demolition_window, conflict_set, vacancy_duration, ownership_chain)."""

from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.datalog.contracts import (
    DatalogFact,
    fact_pack_from_iterable,
)
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.rules import (
    CIVIC_RULES,
    conflict_set,
    demolition_window,
    ownership_chain,
    vacancy_duration,
)


def _present(parcel: str, year, source_id: str) -> DatalogFact:
    return DatalogFact(
        relation='structure_present',
        entity_id=f'{parcel}:present:{year}',
        attributes={'parcel_id': parcel, 'year': year, 'source_id': source_id},
    )


def _absent(parcel: str, year, source_id: str) -> DatalogFact:
    return DatalogFact(
        relation='structure_absent',
        entity_id=f'{parcel}:absent:{year}',
        attributes={'parcel_id': parcel, 'year': year, 'source_id': source_id},
    )


def _assertion(parcel: str, field: str, value, source_id: str) -> DatalogFact:
    return DatalogFact(
        relation='source_assertion',
        entity_id=f'{source_id}:{parcel}:{field}',
        attributes={'parcel_id': parcel, 'field': field, 'value': value, 'source_id': source_id},
    )


def _status(parcel: str, year, status: str, source_id: str) -> DatalogFact:
    return DatalogFact(
        relation='assessor_status',
        entity_id=f'{source_id}:{parcel}:{year}',
        attributes={'parcel_id': parcel, 'year': year, 'status': status, 'source_id': source_id},
    )


def _ownership(parcel: str, owner: str, owner_type: str, from_year, source_id: str, to_year=None) -> DatalogFact:
    attributes = {
        'parcel_id': parcel,
        'owner': owner,
        'owner_type': owner_type,
        'from_year': from_year,
        'source_id': source_id,
    }
    if to_year is not None:
        attributes['to_year'] = to_year
    return DatalogFact(
        relation='ownership',
        entity_id=f'{source_id}:{parcel}:{from_year}',
        attributes=attributes,
    )


class DemolitionWindowRuleTests(SimpleTestCase):
    def test_present_then_absent_derives_window_with_proof(self):
        present = _present('parcel-42', 1955, 'sanborn:1955:sheet-12')
        absent = _absent('parcel-42', 1978, 'aerial:1978')
        pack = fact_pack_from_iterable([present, absent])

        derived = demolition_window(pack)

        self.assertEqual(len(derived), 1)
        fact = derived[0]
        self.assertEqual(fact.rule_id, 'demolition_window')
        self.assertEqual(fact.relation, 'demolition_between')
        self.assertEqual(fact.subject_id, 'parcel-42')
        self.assertEqual(fact.attributes['earliest_year'], 1955)
        self.assertEqual(fact.attributes['latest_year'], 1978)
        self.assertEqual(fact.attributes['window_years'], 23)
        self.assertIn(present.fact_id, fact.dependency_fact_ids)
        self.assertIn(absent.fact_id, fact.dependency_fact_ids)
        self.assertEqual(fact.confidence, 1.0)
        self.assertEqual(fact.writeback_policy, 'read-only')
        self.assertIn('1955', fact.reason)
        self.assertIn('1978', fact.reason)
        self.assertIn('sanborn:1955:sheet-12', fact.attributes['present_source_ids'])
        self.assertIn('aerial:1978', fact.attributes['absent_source_ids'])

    def test_tightest_bracket_uses_latest_present_and_earliest_absent(self):
        facts = [
            _present('p1', 1940, 'sanborn:1940'),
            _present('p1', 1955, 'sanborn:1955'),
            _absent('p1', 1978, 'aerial:1978'),
            _absent('p1', 1990, 'assessor:1990'),
        ]
        derived = demolition_window(fact_pack_from_iterable(facts))

        self.assertEqual(len(derived), 1)
        self.assertEqual(derived[0].attributes['earliest_year'], 1955)
        self.assertEqual(derived[0].attributes['latest_year'], 1978)

    def test_messy_year_strings_are_coerced(self):
        facts = [
            _present('p2', 'circa 1923', 'genesee-historical'),
            _absent('p2', '1961-06-01', 'assessor:1961'),
        ]
        derived = demolition_window(fact_pack_from_iterable(facts))

        self.assertEqual(len(derived), 1)
        self.assertEqual(derived[0].attributes['earliest_year'], 1923)
        self.assertEqual(derived[0].attributes['latest_year'], 1961)

    def test_absent_before_present_is_not_demolition(self):
        facts = [
            _absent('p3', 1955, 'aerial:1955'),
            _present('p3', 1978, 'sanborn:1978'),
        ]
        self.assertEqual(demolition_window(fact_pack_from_iterable(facts)), ())

    def test_present_only_yields_nothing(self):
        pack = fact_pack_from_iterable([_present('p5', 1955, 'sanborn:1955')])
        self.assertEqual(demolition_window(pack), ())


class ConflictSetRuleTests(SimpleTestCase):
    def test_disagreeing_sources_form_conflict_with_proof(self):
        a = _assertion('p1', 'build_year', 1923, 'assessor:2020')
        b = _assertion('p1', 'build_year', 1920, 'sanborn:1920')
        derived = conflict_set(fact_pack_from_iterable([a, b]))

        self.assertEqual(len(derived), 1)
        fact = derived[0]
        self.assertEqual(fact.rule_id, 'conflict_set')
        self.assertEqual(fact.relation, 'source_conflict')
        self.assertEqual(fact.subject_id, 'p1:build_year')
        self.assertEqual(fact.attributes['distinct_value_count'], 2)
        self.assertIn('assessor:2020', fact.attributes['source_ids'])
        self.assertIn('sanborn:1920', fact.attributes['source_ids'])
        self.assertIn(a.fact_id, fact.dependency_fact_ids)
        self.assertIn(b.fact_id, fact.dependency_fact_ids)
        self.assertEqual(fact.writeback_policy, 'read-only')

    def test_agreeing_sources_are_not_a_conflict(self):
        a = _assertion('p1', 'build_year', 1923, 's1')
        b = _assertion('p1', 'build_year', 1923, 's2')
        self.assertEqual(conflict_set(fact_pack_from_iterable([a, b])), ())

    def test_conflicts_are_scoped_per_field(self):
        facts = [
            _assertion('p1', 'build_year', 1923, 's1'),
            _assertion('p1', 'build_year', 1920, 's2'),
            _assertion('p1', 'stories', 2, 's1'),
        ]
        derived = conflict_set(fact_pack_from_iterable(facts))
        self.assertEqual({fact.subject_id for fact in derived}, {'p1:build_year'})


class VacancyDurationRuleTests(SimpleTestCase):
    def test_prolonged_vacancy_flagged_with_span(self):
        facts = [
            _status('p1', 1980, 'vacant', 'assessor:1980'),
            _status('p1', 1990, 'vacant', 'assessor:1990'),
            _status('p1', 2000, 'vacant', 'assessor:2000'),
        ]
        derived = vacancy_duration(fact_pack_from_iterable(facts))

        self.assertEqual(len(derived), 1)
        self.assertEqual(derived[0].relation, 'prolonged_vacancy')
        self.assertEqual(derived[0].attributes['earliest_year'], 1980)
        self.assertEqual(derived[0].attributes['latest_year'], 2000)
        self.assertEqual(derived[0].attributes['vacant_years'], 20)

    def test_occupied_breaks_the_run(self):
        facts = [
            _status('p2', 1980, 'vacant', 's'),
            _status('p2', 1982, 'occupied', 's'),
            _status('p2', 1984, 'vacant', 's'),
        ]
        self.assertEqual(vacancy_duration(fact_pack_from_iterable(facts)), ())

    def test_short_vacancy_below_threshold_is_ignored(self):
        facts = [
            _status('p3', 1980, 'vacant', 's'),
            _status('p3', 1982, 'vacant', 's'),
        ]
        self.assertEqual(vacancy_duration(fact_pack_from_iterable(facts)), ())

    def test_threshold_is_configurable(self):
        facts = [_status('p4', 1980, 'vacant', 's'), _status('p4', 1983, 'vacant', 's')]
        self.assertEqual(vacancy_duration(fact_pack_from_iterable(facts)), ())
        self.assertEqual(len(vacancy_duration(fact_pack_from_iterable(facts), min_years=3)), 1)


class OwnershipChainRuleTests(SimpleTestCase):
    def test_lineage_is_ordered_with_proof(self):
        a = _ownership('p1', 'Smith', 'private', 1960, 'deeds')
        b = _ownership('p1', 'Jones', 'private', 1985, 'deeds')
        derived = ownership_chain(fact_pack_from_iterable([b, a]))  # deliberately unordered input

        self.assertEqual(len(derived), 1)
        fact = derived[0]
        self.assertEqual(fact.relation, 'ownership_chain')
        self.assertEqual(fact.subject_id, 'p1')
        self.assertEqual(fact.attributes['owner_count'], 2)
        self.assertEqual([link['from_year'] for link in fact.attributes['chain']], [1960, 1985])
        self.assertFalse(fact.attributes['tax_foreclosure_to_land_bank'])
        self.assertIn(a.fact_id, fact.dependency_fact_ids)
        self.assertIn(b.fact_id, fact.dependency_fact_ids)

    def test_tax_foreclosure_to_land_bank_is_flagged(self):
        facts = [
            _ownership('p2', 'Owner', 'private', 1970, 'deeds'),
            _ownership('p2', 'Genesee County', 'tax_foreclosure', 2008, 'foreclosure-roll'),
            _ownership('p2', 'Genesee Land Bank', 'land_bank', 2011, 'land-bank'),
        ]
        derived = ownership_chain(fact_pack_from_iterable(facts))

        self.assertEqual(len(derived), 1)
        attributes = derived[0].attributes
        self.assertTrue(attributes['tax_foreclosure_to_land_bank'])
        self.assertEqual(attributes['foreclosure_year'], 2008)
        self.assertEqual(attributes['land_bank_year'], 2011)
        self.assertIn('land bank', derived[0].reason)

    def test_single_owner_is_not_a_chain(self):
        pack = fact_pack_from_iterable([_ownership('p3', 'Solo', 'private', 1990, 's')])
        self.assertEqual(ownership_chain(pack), ())

    def test_record_missing_year_sorts_last(self):
        dated = _ownership('p4', 'Early', 'private', 1950, 's')
        undated = DatalogFact(
            relation='ownership',
            entity_id='s:p4:unknown',
            attributes={'parcel_id': 'p4', 'owner': 'Unknown', 'owner_type': 'private', 'source_id': 's'},
        )
        derived = ownership_chain(fact_pack_from_iterable([undated, dated]))

        self.assertEqual(len(derived), 1)
        chain = derived[0].attributes['chain']
        self.assertEqual(chain[0]['from_year'], 1950)
        self.assertIsNone(chain[1]['from_year'])


class CivicRuleRegistrationTests(SimpleTestCase):
    def test_all_civic_rules_registered_by_id(self):
        self.assertEqual(
            {rule.rule_id for rule in CIVIC_RULES},
            {'demolition_window', 'conflict_set', 'vacancy_duration', 'ownership_chain'},
        )

    def test_conflict_set_selectable_through_engine(self):
        facts = [
            _assertion('p9', 'build_year', 1923, 'assessor'),
            _assertion('p9', 'build_year', 1920, 'sanborn'),
        ]
        receipt = DatalogEngine().derive(fact_pack_from_iterable(facts), rule_ids=['conflict_set'])

        self.assertEqual(receipt.rule_ids, ('conflict_set',))
        self.assertEqual(len(receipt.derived_facts), 1)
        self.assertEqual(receipt.derived_facts[0].relation, 'source_conflict')
        self.assertEqual(receipt.to_dict()['writeback_policy'], 'read-only')

    def test_ownership_chain_selectable_through_engine(self):
        facts = [
            _ownership('p10', 'County', 'tax_foreclosure', 2008, 'roll'),
            _ownership('p10', 'Land Bank', 'land_bank', 2011, 'lb'),
        ]
        receipt = DatalogEngine().derive(fact_pack_from_iterable(facts), rule_ids=['ownership_chain'])

        self.assertEqual(len(receipt.derived_facts), 1)
        self.assertEqual(receipt.derived_facts[0].relation, 'ownership_chain')
        self.assertTrue(receipt.derived_facts[0].attributes['tax_foreclosure_to_land_bank'])
