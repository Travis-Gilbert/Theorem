from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.runtime_adapters import (
    causal_effect_from_observation_groups,
    source_reliability_from_records,
    validator_schedule_from_records,
)


class RuntimeAdapterTests(SimpleTestCase):
    def test_probabilistic_adapter_counts_real_evidence_statuses(self):
        receipt = source_reliability_from_records(
            source_id='source-1',
            evidence_records=[
                {'status': 'corroborated'},
                {'status': 'contradicted'},
                {'status': 'passed'},
            ],
        )

        self.assertEqual(receipt['observations']['corroborated'], 2)
        self.assertEqual(receipt['observations']['contradicted'], 1)
        self.assertEqual(receipt['metadata']['input_shape'], 'evidence_records')

    def test_causal_adapter_computes_treated_control_effect(self):
        receipt = causal_effect_from_observation_groups(
            question_id='q1',
            treatment='review',
            outcome='accuracy',
            treated_records=[{'accuracy': 0.9}, {'accuracy': 0.7}],
            control_records=[{'accuracy': 0.4}],
        )

        self.assertEqual(receipt['identifiability_status'], 'identified_under_assumptions')
        self.assertAlmostEqual(receipt['estimate'], 0.4)

    def test_optimizer_adapter_schedules_validator_records(self):
        receipt = validator_schedule_from_records(
            budget=3.0,
            validator_records=[
                {'id': 'proof', 'expected_value': 5, 'cost': 2, 'tags': ['proof']},
                {'id': 'slow', 'expected_value': 4, 'cost': 5},
            ],
        )

        self.assertEqual(receipt['status'], 'feasible')
        self.assertEqual(receipt['selected'][0]['candidate_id'], 'proof')
