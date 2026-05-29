"""Quality-diversity archive fallback."""

from __future__ import annotations

from collections import defaultdict

from .contracts import ArchiveReceipt, EvolutionCandidate, archive_hash


class EvolutionEngine:
    engine = 'quality-diversity-python-fallback'

    def archive(self, candidates: list[EvolutionCandidate], *, elites_per_niche: int = 2) -> ArchiveReceipt:
        niches: dict[str, list[EvolutionCandidate]] = defaultdict(list)
        for candidate in candidates:
            niches[candidate.niche].append(candidate)

        elites: dict[str, list[dict]] = {}
        selected: list[EvolutionCandidate] = []
        for niche, niche_candidates in sorted(niches.items()):
            ranked = sorted(
                niche_candidates,
                key=lambda item: (item.score, item.novelty, item.candidate_id),
                reverse=True,
            )[:elites_per_niche]
            selected.extend(ranked)
            elites[niche] = [candidate.to_dict() for candidate in ranked]

        return ArchiveReceipt(
            engine=self.engine,
            archive_hash=archive_hash(selected),
            elites_by_niche=elites,
            rejected_count=max(0, len(candidates) - len(selected)),
        )

