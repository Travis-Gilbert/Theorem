from __future__ import annotations

import json

from django.test import SimpleTestCase

from apps.notebook.inference_kernel import registry_report
from apps.notebook.inference_kernel.registry import get_registry, resolve_kernels


class InferenceKernelRegistryTests(SimpleTestCase):
    def test_builtins_include_core_pipeline_capabilities(self):
        report = registry_report()

        families = {entry['inference_family'] for entry in report['entries']}
        jobs = {entry['epistemic_job'] for entry in report['entries']}
        ids = {entry['kernel_id'] for entry in report['entries']}

        self.assertIn('neural', families)
        self.assertIn('graph', families)
        self.assertIn('planner', families)
        self.assertIn('ingest', jobs)
        self.assertIn('evaluate', jobs)

        self.assertIn('spacy_entity_extractor', ids)
        self.assertIn('bm25_tfidf_search', ids)
        self.assertIn('sbert_embedding_search', ids)
        self.assertIn('nli_contradiction_check', ids)
        self.assertIn('ppr_pagerank_graph', ids)
        self.assertIn('gnn_kge_spacetime', ids)
        self.assertIn('learned_scorer', ids)
        self.assertIn('tms_belief_revision', ids)
        self.assertIn('context_web_packer', ids)
        self.assertIn('search_kernel', ids)
        self.assertIn('scene_os_compiler', ids)
        self.assertIn('orchestrate_toolgraph', ids)
        self.assertIn('thg_command_router', ids)

    def test_report_is_json_serializable(self):
        report = registry_report()
        encoded = json.dumps(report)

        decoded = json.loads(encoded)
        self.assertEqual(decoded['count'], report['count'])

    def test_gnn_kge_truth_type_is_plausibility_and_has_validator_policy(self):
        gnn_kge = get_registry().get('gnn_kge_spacetime')

        self.assertIsNotNone(gnn_kge)
        if gnn_kge is not None:
            self.assertEqual(gnn_kge.truth_type, 'plausibility')
            self.assertEqual(gnn_kge.validator, 'benchmark')
            self.assertIn(gnn_kge.writeback_policy, ('proposal-only', 'review-required', 'read-only', 'direct-write'))

    def test_router_can_filter_by_family(self):
        neural = resolve_kernels(inference_family='neural')

        self.assertTrue(len(neural) >= 2)
        self.assertEqual({item.inference_family for item in neural}, {'neural'})
