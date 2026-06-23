from __future__ import annotations

import os

from django.test import SimpleTestCase

from apps.notebook.inference_kernel.native_strategy import (
    NativeFeatureSpec,
    native_enabled,
    native_gate_report,
)


class NativeStrategyTests(SimpleTestCase):
    def test_disable_native_escape_hatch(self):
        feature = NativeFeatureSpec(
            feature_id='egraph',
            python_fallback='apps.notebook.inference_engines.egraph',
            native_module='theseus_native.egraph',
            parity_tests=('apps/notebook/inference_engines/egraph/tests',),
        )
        previous = os.environ.get('THESEUS_DISABLE_NATIVE')
        os.environ['THESEUS_DISABLE_NATIVE'] = '1'
        try:
            self.assertFalse(native_enabled(feature))
        finally:
            if previous is None:
                os.environ.pop('THESEUS_DISABLE_NATIVE', None)
            else:
                os.environ['THESEUS_DISABLE_NATIVE'] = previous

    def test_native_cannot_write_canon(self):
        feature = NativeFeatureSpec(
            feature_id='bad',
            python_fallback='fallback',
            native_module='native',
            can_write_canon=True,
        )

        report = native_gate_report(feature)

        self.assertFalse(report['enabled'])
        self.assertTrue(report['requires_python_fallback'])

