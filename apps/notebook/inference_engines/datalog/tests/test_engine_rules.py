from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.datalog.contracts import DatalogFact
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records


class DatalogEngineRuleTests(SimpleTestCase):
    def test_unsupported_claim_and_no_independent_support_are_explainable(self):
        fact_pack = build_fact_pack_from_records(
            claims=[{'id': 10, 'source_object_id': 1, 'text': 'A claim', 'status': 'proposed'}],
        )

        receipt = DatalogEngine().derive(fact_pack)
        derived = {fact.relation: fact for fact in receipt.derived_facts}

        self.assertIn('unsupported_claim', derived)
        self.assertIn('claim_has_no_independent_support', derived)
        self.assertEqual(derived['unsupported_claim'].writeback_policy, 'read-only')
        self.assertTrue(derived['unsupported_claim'].reason)
        self.assertEqual(receipt.to_dict()['writeback_policy'], 'read-only')

    def test_duplicate_private_and_generated_rules_fire(self):
        fact_pack = build_fact_pack_from_records(
            objects=[
                {'id': 1, 'title': 'Alpha Node', 'properties': {'visibility': 'private'}},
                {'id': 2, 'title': 'alpha-node', 'properties': {}},
            ],
            context_atoms=[
                {
                    'id': 'atom-1',
                    'artifact_id': 'ctx-1',
                    'kind': 'claim',
                    'included': True,
                    'object_pk': 1,
                    'metadata': {'export_candidate': True},
                },
                {
                    'id': 'atom-2',
                    'artifact_id': 'ctx-1',
                    'kind': 'policy',
                    'included': True,
                    'metadata': {'generated': True},
                },
            ],
        )

        receipt = DatalogEngine().derive(fact_pack)
        relations = {fact.relation for fact in receipt.derived_facts}

        self.assertIn('likely_duplicate_entity', relations)
        self.assertIn('private_source_reaches_export_candidate', relations)
        self.assertIn('context_atom_tainted_by_generated_artifact', relations)

    def test_evidence_path_rule_can_consume_explicit_path_facts(self):
        fact_pack = build_fact_pack_from_records(
            evidence_paths=[{'id': 'path-1', 'edge_pks': [1, 2, 3, 4], 'query_id': 'q-1'}],
        )

        receipt = DatalogEngine().derive(fact_pack, rule_ids=['evidence_path_too_long'])

        self.assertEqual(len(receipt.derived_facts), 1)
        self.assertEqual(receipt.derived_facts[0].relation, 'evidence_path_too_long')

    def test_unknown_rule_is_reported_not_raised(self):
        receipt = DatalogEngine().derive(
            build_fact_pack_from_records(),
            rule_ids=['missing_rule'],
        )

        self.assertEqual(receipt.derived_facts, ())
        self.assertIn('Unknown Datalog rule skipped: missing_rule', receipt.warnings)

    def test_fact_requires_stable_relation_and_entity(self):
        with self.assertRaises(ValueError):
            DatalogFact(relation='', entity_id='1')

