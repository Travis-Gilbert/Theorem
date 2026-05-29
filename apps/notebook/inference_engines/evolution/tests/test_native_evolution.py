from __future__ import annotations

import json
import os
import sys
from unittest.mock import patch

from django.test import SimpleTestCase

from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.engine import EvolutionEngine
from apps.notebook.inference_engines.evolution.native import NativeEvolutionEngine


class _FakeNativeEvolutionModule:
    def __init__(self):
        self.payloads: list[dict] = []

    def bgi_evolution_archive_json(self, payload_json: str) -> str:
        payload = json.loads(payload_json)
        self.payloads.append(payload)
        candidates = [
            EvolutionCandidate(
                candidate_id=str(item['candidate_id']),
                niche=str(item.get('niche') or 'default'),
                score=float(item.get('score') or 0.0),
                novelty=float(item.get('novelty') or 0.0),
                payload=dict(item.get('payload') or {}),
            )
            for item in payload.get('candidates') or []
        ]
        receipt = EvolutionEngine().archive(
            candidates,
            elites_per_niche=int(payload.get('elites_per_niche') or 2),
        )
        return json.dumps(receipt.to_dict(), sort_keys=True)


class NativeEvolutionEngineTests(SimpleTestCase):
    def _candidates(self):
        return [
            EvolutionCandidate(candidate_id='c1', niche='n1', score=0.8, novelty=0.1, payload={'k': 1}),
            EvolutionCandidate(candidate_id='c2', niche='n1', score=0.8, novelty=0.9, payload={'k': 2}),
            EvolutionCandidate(candidate_id='c3', niche='n2', score=0.4, novelty=0.2, payload={}),
        ]

    def test_uses_native_archive_when_export_is_available(self):
        fake_native = _FakeNativeEvolutionModule()
        env = {
            **os.environ,
            'THESEUS_DISABLE_NATIVE': '',
            'THESEUS_BGI_NATIVE_SYMBOLIC_ENABLED': '1',
        }
        with (
            patch.dict(os.environ, env, clear=True),
            patch.dict(sys.modules, {'theseus_native': fake_native}),
        ):
            receipt = NativeEvolutionEngine().archive(self._candidates(), elites_per_niche=1)

        expected = EvolutionEngine().archive(self._candidates(), elites_per_niche=1)
        self.assertEqual(receipt.to_dict(), expected.to_dict())
        self.assertEqual(len(fake_native.payloads), 1)
        self.assertEqual(fake_native.payloads[0]['elites_per_niche'], 1)
        self.assertEqual(fake_native.payloads[0]['candidates'][0]['candidate_id'], 'c1')

    def test_falls_back_to_python_when_native_export_is_missing(self):
        with patch('apps.notebook.inference_engines.evolution.native._native_module', return_value=None):
            receipt = NativeEvolutionEngine().archive(self._candidates(), elites_per_niche=1)

        expected = EvolutionEngine().archive(self._candidates(), elites_per_niche=1)
        self.assertEqual(receipt.to_dict(), expected.to_dict())
