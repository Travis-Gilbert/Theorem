from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.affordances import (
    build_fact_pack_from_substrate,
    run_datalog_affordance,
    run_probabilistic_expected_value,
    run_probabilistic_source_reliability,
)
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.runtime_adapters import (
    expected_value_from_validator_records,
    source_reliability_from_records,
)


class InferenceAffordanceTests(SimpleTestCase):
    def test_datalog_substrate_fact_pack_matches_record_builder(self):
        substrate_input = {
            'claims': [
                {
                    'id': 10,
                    'node_ref': 'claim:10',
                    'source_object_id': 1,
                    'text': 'A claim',
                    'status': 'proposed',
                },
            ],
            'node_refs': ['claim:10'],
        }

        substrate_pack = build_fact_pack_from_substrate(substrate_input)
        record_pack = build_fact_pack_from_records(claims=substrate_input['claims'])

        self.assertEqual(substrate_pack.pack_hash, record_pack.pack_hash)
        self.assertEqual(substrate_pack.source, 'substrate')

    def test_datalog_affordance_wraps_unchanged_engine_receipt(self):
        substrate_input = {
            'claims': [
                {
                    'id': 10,
                    'node_ref': 'claim:10',
                    'source_object_id': 1,
                    'text': 'A claim',
                    'status': 'proposed',
                },
            ],
            'metadata': {'query_id': 'q-1'},
        }
        fact_pack = build_fact_pack_from_records(claims=substrate_input['claims'])
        expected = DatalogEngine().derive(fact_pack).to_dict()

        receipt = run_datalog_affordance(substrate_input).to_dict()

        self.assertEqual(receipt['affordance_id'], 'datalog.derive')
        self.assertEqual(receipt['input_hash'], fact_pack.pack_hash)
        self.assertEqual(receipt['payload'], expected)
        self.assertEqual(receipt['provenance']['fact_count'], 1)
        self.assertEqual(receipt['metadata']['query_id'], 'q-1')
        self.assertIn('claim:10', receipt['input_node_refs'])

    def test_datalog_affordance_supports_node_set_classification(self):
        receipt = run_datalog_affordance(
            {
                'nodes': [
                    {
                        'node_type': 'claim',
                        'node_ref': 'claim:11',
                        'id': 11,
                        'source_object_id': 1,
                        'text': 'Another claim',
                        'status': 'proposed',
                    },
                ],
            },
            rule_ids=['unsupported_claim'],
        ).to_dict()

        self.assertEqual(receipt['payload']['rule_ids'], ['unsupported_claim'])
        self.assertEqual(receipt['payload']['derived_facts'][0]['relation'], 'unsupported_claim')
        self.assertEqual(receipt['input_node_refs'], ['claim:11'])

    def test_probabilistic_source_reliability_affordance_matches_runtime_adapter(self):
        evidence_records = [
            {'id': 'e1', 'node_ref': 'evidence:e1', 'status': 'corroborated'},
            {'id': 'e2', 'node_ref': 'evidence:e2', 'status': 'contradicted'},
            {'id': 'e3', 'node_ref': 'evidence:e3', 'status': 'passed'},
        ]
        expected = source_reliability_from_records(
            source_id='source-1',
            evidence_records=evidence_records,
        )

        receipt = run_probabilistic_source_reliability(
            {'evidence_records': evidence_records},
            source_id='source-1',
        ).to_dict()

        self.assertEqual(receipt['affordance_id'], 'probabilistic.source_reliability')
        self.assertEqual(receipt['payload'], expected)
        self.assertEqual(receipt['provenance']['record_count'], 3)
        self.assertEqual(receipt['input_node_refs'], ['evidence:e1', 'evidence:e2', 'evidence:e3'])

    def test_probabilistic_expected_value_affordance_matches_runtime_adapter(self):
        validator_records = [
            {'id': 'proof', 'node_ref': 'validator:proof', 'status': 'passed', 'cost': 2},
            {'id': 'slow', 'node_ref': 'validator:slow', 'status': 'failed', 'cost': 4},
        ]
        expected = expected_value_from_validator_records(
            validator_records=validator_records,
            decision_value=3.0,
        )

        receipt = run_probabilistic_expected_value(
            {'validator_records': validator_records},
            decision_value=3.0,
        ).to_dict()

        self.assertEqual(receipt['affordance_id'], 'probabilistic.expected_value_of_information')
        self.assertEqual(receipt['payload'], expected)
        self.assertEqual(receipt['provenance']['record_count'], 2)
        self.assertEqual(receipt['input_node_refs'], ['validator:proof', 'validator:slow'])

