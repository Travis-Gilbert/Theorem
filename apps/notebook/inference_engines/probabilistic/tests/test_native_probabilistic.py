from __future__ import annotations

import json
import sys
from types import SimpleNamespace
from unittest import mock

from django.test import SimpleTestCase

from apps.notebook.inference_engines.common import stable_json
from apps.notebook.inference_engines.probabilistic.engine import ProbProgEngine
from apps.notebook.inference_engines.probabilistic.native import NativeProbProgEngine


class NativeProbProgEngineTests(SimpleTestCase):
    def test_source_reliability_native_receipt_matches_python_contract(self):
        expected = ProbProgEngine().source_reliability(
            source_id='source-a',
            prior_alpha=2.0,
            prior_beta=2.0,
            corroborated=6,
            contradicted=2,
        ).to_dict()

        def source_reliability(payload_json: str) -> str:
            payload = json.loads(payload_json)
            self.assertEqual(payload['source_id'], 'source-a')
            return stable_json(expected)

        fake_native = SimpleNamespace(
            bgi_probabilistic_source_reliability_json=source_reliability,
        )

        with mock.patch.dict(sys.modules, {'theseus_native': fake_native}):
            receipt = NativeProbProgEngine().source_reliability(
                source_id='source-a',
                prior_alpha=2.0,
                prior_beta=2.0,
                corroborated=6,
                contradicted=2,
            ).to_dict()

        self.assertEqual(receipt, expected)

    def test_expected_value_native_receipt_matches_python_contract(self):
        expected = ProbProgEngine().expected_value_of_information(
            current_uncertainty=0.6,
            expected_uncertainty_after=0.2,
            decision_value=10.0,
            validator_cost=1.0,
        ).to_dict()

        fake_native = SimpleNamespace(
            bgi_probabilistic_expected_value_json=lambda payload: stable_json(expected),
        )

        with mock.patch.dict(sys.modules, {'theseus_native': fake_native}):
            receipt = NativeProbProgEngine().expected_value_of_information(
                current_uncertainty=0.6,
                expected_uncertainty_after=0.2,
                decision_value=10.0,
                validator_cost=1.0,
            ).to_dict()

        self.assertEqual(receipt, expected)
