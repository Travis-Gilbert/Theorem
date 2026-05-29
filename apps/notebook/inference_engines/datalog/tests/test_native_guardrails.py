from __future__ import annotations

from unittest.mock import patch

from django.test import SimpleTestCase

from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_records
from apps.notebook.inference_engines.datalog.native import (
    NativeDatalogEngine,
    native_datalog_can_handle,
    verified_native_datalog_rule_ids,
)
from apps.notebook.inference_engines.datalog.rules import DEFAULT_RULES


class _ExplodingNative:
    def bgi_datalog_derive_core_json(self, _payload: str) -> str:
        raise AssertionError('native Datalog path should be parity-gated')


class NativeDatalogGuardrailTests(SimpleTestCase):
    def test_all_default_rules_are_verified_after_native_parity(self):
        default_rule_ids = {rule.rule_id for rule in DEFAULT_RULES}

        self.assertEqual(set(verified_native_datalog_rule_ids()), default_rule_ids)
        self.assertTrue(native_datalog_can_handle(None))
        self.assertTrue(native_datalog_can_handle(['unsupported_claim']))

    def test_default_derivation_uses_python_even_when_native_module_exists(self):
        fact_pack = build_fact_pack_from_records(
            claims=[{'id': 'claim-1', 'status': 'proposed'}],
        )

        with patch(
            'apps.notebook.inference_engines.datalog.native._native_module',
            return_value=_ExplodingNative(),
        ):
            receipt = NativeDatalogEngine().derive(fact_pack)

        self.assertEqual(receipt.engine, 'python-reference-datalog')
        self.assertIn('unsupported_claim', {fact.relation for fact in receipt.derived_facts})

    def test_explicit_unverified_rule_uses_python_even_when_native_module_exists(self):
        fact_pack = build_fact_pack_from_records(
            claims=[{'id': 'claim-1', 'status': 'proposed'}],
        )

        with patch(
            'apps.notebook.inference_engines.datalog.native._native_module',
            return_value=_ExplodingNative(),
        ):
            receipt = NativeDatalogEngine().derive(fact_pack, rule_ids=['unsupported_claim'])

        self.assertEqual(receipt.engine, 'python-reference-datalog')
        self.assertEqual(receipt.rule_ids, ('unsupported_claim',))
