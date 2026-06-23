from __future__ import annotations

from django.test import SimpleTestCase

from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.engine import EvolutionEngine


class EvolutionEngineTests(SimpleTestCase):
    def test_archive_preserves_elites_by_niche(self):
        receipt = EvolutionEngine().archive([
            EvolutionCandidate('a', niche='simplicity', score=0.8, novelty=0.1, payload={}),
            EvolutionCandidate('b', niche='simplicity', score=0.7, novelty=0.9, payload={}),
            EvolutionCandidate('c', niche='novelty', score=0.4, novelty=1.0, payload={}),
        ], elites_per_niche=1)

        self.assertEqual(set(receipt.elites_by_niche), {'simplicity', 'novelty'})
        self.assertEqual(receipt.elites_by_niche['simplicity'][0]['candidate_id'], 'a')
        self.assertEqual(receipt.rejected_count, 1)

