from __future__ import annotations

import json
import sys
from types import SimpleNamespace
from unittest import mock

from django.test import SimpleTestCase

from apps.notebook.inference_engines.common import stable_json
from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.datalog.native import (
    NativeDatalogEngine,
    native_datalog_can_handle,
)
from apps.notebook.inference_engines.datalog.rules import DEFAULT_RULES


class NativeDatalogEngineTests(SimpleTestCase):
    def test_all_rules_are_native_safe_after_verified_parity(self):
        self.assertTrue(native_datalog_can_handle(None))
        self.assertTrue(native_datalog_can_handle(['unsupported_claim']))

    def test_native_payload_preserves_all_rule_receipt_surface(self):
        fact_pack = build_fact_pack_from_records(
            claims=[{'id': 10, 'source_object_id': 1, 'text': 'A claim', 'status': 'proposed'}],
        )
        expected = DatalogEngine().derive(fact_pack).to_dict()
        native_calls: list[str] = []

        def derive(payload_json: str) -> str:
            native_calls.append(payload_json)
            payload = json.loads(payload_json)
            self.assertEqual(payload['rule_ids'], [])
            return stable_json(expected)

        # Track the live DEFAULT_RULES so the fake native always declares exactly
        # the current rule set. A hardcoded list silently falls behind when a new
        # rule lands (the subset check fails, native admission drops to Python, and
        # the fake derive callback below stops being exercised). Dynamic = the test
        # keeps proving the native receipt surface as the civic rule set grows.
        fake_native = SimpleNamespace(
            bgi_datalog_derive_core_json=derive,
            bgi_datalog_verified_rule_ids_json=lambda: stable_json(
                [rule.rule_id for rule in DEFAULT_RULES]
            ),
        )

        with mock.patch.dict(sys.modules, {'theseus_native': fake_native}):
            receipt = NativeDatalogEngine().derive(fact_pack).to_dict()

        # The native callback MUST run: a stale verified-rule list would drop
        # admission to the Python fallback, which returns the same receipt and
        # would pass this assertion silently. Asserting the callback fired makes
        # that regression fail loudly.
        self.assertEqual(len(native_calls), 1)
        self.assertEqual(receipt, expected)

    def test_unknown_rule_stays_on_python_reference_path(self):
        fact_pack = build_fact_pack_from_records()

        receipt = NativeDatalogEngine().derive(fact_pack, rule_ids=['missing_rule'])

        self.assertIn('Unknown Datalog rule skipped: missing_rule', receipt.warnings)
