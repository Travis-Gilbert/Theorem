"""Contracts for quality-diversity candidate archives."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any

from apps.notebook.inference_engines.common import stable_hash


@dataclass(frozen=True, slots=True)
class EvolutionCandidate:
    candidate_id: str
    niche: str
    score: float
    payload: dict[str, Any]
    novelty: float = 0.0

    def to_dict(self) -> dict[str, Any]:
        return {
            'candidate_id': self.candidate_id,
            'niche': self.niche,
            'score': float(self.score),
            'novelty': float(self.novelty),
            'payload': dict(self.payload),
        }


@dataclass(frozen=True, slots=True)
class ArchiveReceipt:
    engine: str
    archive_hash: str
    elites_by_niche: dict[str, list[dict[str, Any]]]
    rejected_count: int

    def to_dict(self) -> dict[str, Any]:
        return {
            'engine': self.engine,
            'archive_hash': self.archive_hash,
            'elites_by_niche': self.elites_by_niche,
            'rejected_count': self.rejected_count,
            'writeback_policy': 'read-only',
        }


def archive_hash(candidates: list[EvolutionCandidate]) -> str:
    return stable_hash([candidate.to_dict() for candidate in sorted(candidates, key=lambda item: (item.niche, -item.score, item.candidate_id))])

