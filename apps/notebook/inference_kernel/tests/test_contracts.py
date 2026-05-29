from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_kernel.contracts import (
    ConstrainedChoiceError,
    KNOWN_EPISTEMIC_JOBS,
    KNOWN_INFERENCE_FAMILIES,
    InferenceKernelContract,
    KNOWN_VALIDATORS,
    KNOWN_WRITEBACK_POLICIES,
)


class InferenceKernelContractTests(SimpleTestCase):
    def test_invalid_values_raise(self):
        with self.assertRaises(ConstrainedChoiceError):
            InferenceKernelContract(
                kernel_id='bad',
                epistemic_job='invalid',
                inference_family='graph',
                consumes_view=(),
                produces=(),
            )

        with self.assertRaises(ConstrainedChoiceError):
            InferenceKernelContract(
                kernel_id='bad',
                epistemic_job='ingest',
                inference_family='graph',
                consumes_view=('unknown',),
                produces=(),
            )

    def test_contract_exports_expected_fields(self):
        contract = InferenceKernelContract(
            kernel_id='spacy_entity_extractor',
            epistemic_job='ingest',
            inference_family='lexical',
            consumes_view=('text',),
            produces=('claim', 'edge'),
            truth_type='relevance',
            validator='benchmark',
            writeback_policy='review-required',
        )

        payload = contract.to_dict()

        self.assertEqual(payload['kernel_id'], 'spacy_entity_extractor')
        self.assertEqual(payload['epistemic_job'], 'ingest')
        self.assertEqual(payload['inference_family'], 'lexical')
        self.assertEqual(payload['truth_type'], 'relevance')
        self.assertIn('validator', payload)
        self.assertIn('writeback_policy', payload)

    def test_allowed_constants_include_required_categories(self):
        self.assertIn('neural', KNOWN_INFERENCE_FAMILIES)
        self.assertIn('relate', KNOWN_EPISTEMIC_JOBS)
        self.assertIn('benchmark', KNOWN_VALIDATORS)
        self.assertIn('direct-write', KNOWN_WRITEBACK_POLICIES)
