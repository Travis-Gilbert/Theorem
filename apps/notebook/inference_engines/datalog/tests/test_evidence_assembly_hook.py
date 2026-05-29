from __future__ import annotations

from django.test import TestCase, override_settings

from apps.notebook.evidence_assembly import assemble_evidence
from apps.notebook.models import Claim, Object


class DatalogEvidenceAssemblyHookTests(TestCase):
    @override_settings(MEMGRAPH_CLAIM_PHASE=False)
    def test_derived_facts_are_optional_evidence_metadata(self):
        source = Object.objects.create(title='Sparse claim source', body='one claim only')
        Claim.objects.create(source_object=source, text='This claim has no independent support.')
        retrieved = [{'object_pk': source.pk, 'rrf_score': 1.0, 'signals': {}}]

        baseline = assemble_evidence(
            'support check',
            retrieved,
            max_pairs_for_nli=0,
            include_existing_edges=False,
            include_temporal=False,
            steiner_mode=False,
        )
        enriched = assemble_evidence(
            'support check',
            retrieved,
            max_pairs_for_nli=0,
            include_existing_edges=False,
            include_temporal=False,
            steiner_mode=False,
            include_derived_facts=True,
        )

        self.assertNotIn('derived_facts', baseline.assembly_metadata)
        self.assertGreater(enriched.assembly_metadata['derived_fact_count'], 0)
        relations = {
            item['relation']
            for item in enriched.assembly_metadata['derived_facts']
        }
        self.assertIn('unsupported_claim', relations)
