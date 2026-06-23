"""Durable quality-diversity archives for DiscoveryRun candidates."""

from __future__ import annotations

from typing import Any

from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate
from apps.notebook.inference_engines.evolution.native import NativeEvolutionEngine
from apps.notebook.models import DiscoveryCandidateArchiveEntry, DiscoveryRun


def archive_candidates(
    *,
    candidates: list[dict[str, Any]],
    run_id: str | None = None,
    elites_per_niche: int = 2,
) -> dict[str, Any]:
    """Archive candidates and persist selected elites as proposal-only rows."""

    run = DiscoveryRun.objects.filter(run_id=run_id).first() if run_id else None
    evolution_candidates = [
        EvolutionCandidate(
            candidate_id=str(item['candidate_id']),
            niche=str(item.get('niche') or item.get('validator_family') or 'default'),
            score=float(item.get('score', item.get('expected_value', 0.0)) or 0.0),
            novelty=float(item.get('novelty') or 0.0),
            payload=dict(item.get('payload') or item),
        )
        for item in candidates
    ]
    receipt = NativeEvolutionEngine().archive(
        evolution_candidates,
        elites_per_niche=elites_per_niche,
    )
    for niche, elites in receipt.elites_by_niche.items():
        for elite in elites:
            DiscoveryCandidateArchiveEntry.objects.update_or_create(
                run=run,
                candidate_id=str(elite['candidate_id']),
                niche=niche,
                defaults={
                    'score': float(elite.get('score') or 0.0),
                    'novelty': float(elite.get('novelty') or 0.0),
                    'archive_hash': receipt.archive_hash,
                    'payload': dict(elite.get('payload') or elite),
                    'validator_receipts': list(elite.get('validator_receipts') or []),
                    'status': DiscoveryCandidateArchiveEntry.Status.ACTIVE,
                    'writeback_policy': 'proposal-only',
                    'canonical_graph_mutation': False,
                },
            )
    payload = receipt.to_dict()
    payload['persisted_elite_count'] = sum(len(elites) for elites in receipt.elites_by_niche.values())
    payload['run_id'] = run.run_id if run is not None else ''
    return payload
