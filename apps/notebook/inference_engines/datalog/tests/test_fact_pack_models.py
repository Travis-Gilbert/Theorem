from __future__ import annotations

from django.contrib.auth import get_user_model
from django.test import TestCase

from apps.notebook.inference_engines.datalog.engine import DatalogEngine
from apps.notebook.inference_engines.datalog.facts import build_fact_pack_from_models
from apps.notebook.models import (
    Artifact,
    Claim,
    ClaimDependency,
    ContextArtifact,
    ContextAtom,
    Edge,
    EvidenceLink,
    Object,
)


class DatalogModelFactPackTests(TestCase):
    def test_fact_pack_consumes_existing_model_rows(self):
        source = Object.objects.create(title='Source A', body='supporting material')
        target = Object.objects.create(title='Target B', body='other material')
        claim = Claim.objects.create(source_object=source, text='Source A supports the target.')
        ClaimDependency.objects.create(claim=claim, depends_on_object=source)
        Edge.objects.create(
            from_object=source,
            to_object=target,
            edge_type='contradicts',
            reason='test tension edge',
            strength=0.7,
        )
        artifact = Artifact.objects.create(title='Artifact A', raw_text='evidence')
        EvidenceLink.objects.create(
            artifact=artifact,
            claim=claim,
            relation_type=EvidenceLink.RelationType.SUPPORTS,
            attached_by='engine',
            confidence=0.8,
        )
        user = get_user_model().objects.create_user(username='ctx-user')
        context_artifact = ContextArtifact.objects.create(
            account=user,
            title='Context capsule',
            budget_tokens=2000,
        )
        ContextAtom.objects.create(
            artifact=context_artifact,
            kind=ContextAtom.Kind.CODE_SYMBOL,
            title='dangerous_symbol',
            included=True,
            object_pk=source.pk,
            metadata={'failing_postmortem_pattern': True},
        )

        fact_pack = build_fact_pack_from_models(
            object_ids=[source.pk, target.pk],
            claim_ids=[claim.pk],
            artifact_ids=[context_artifact.pk],
        )
        relations = {fact.relation for fact in fact_pack.facts}

        self.assertIn('object', relations)
        self.assertIn('claim', relations)
        self.assertIn('evidence_link', relations)
        self.assertIn('claim_dependency', relations)
        self.assertIn('context_atom', relations)
        self.assertIn('edge', relations)

        receipt = DatalogEngine().derive(fact_pack)
        derived_relations = {fact.relation for fact in receipt.derived_facts}
        self.assertIn('dependent_claim', derived_relations)
        self.assertIn('object_in_unresolved_tension_neighborhood', derived_relations)
        self.assertIn('code_symbol_touched_by_failing_postmortem_pattern', derived_relations)

