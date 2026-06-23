"""Nightly MAP-Elites tick (E8-T3).

Spec ref: Spec2 sec 7.v2 + sec 12.Phase6. Reads recent EvolutionPatch
rows, maps them through `patch_to_candidate`, hands them to the
native-aware `NativeEvolutionEngine.archive()` to compute the per-niche
elites, and writes the result into EvolutionCell rows.

The tick is intentionally simple and idempotent: rerunning produces
the same result for the same input window. The RQ wrapper lives in
`apps/orchestrate/tasks.py::tick_map_elites`.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass
from typing import Any

from django.utils import timezone

from apps.notebook.inference_engines.evolution.native import (
    NativeEvolutionEngine,
)
from apps.orchestrate.models import (
    AgentBlueprint,
    EvolutionCell,
    EvolutionPatch,
)
from apps.orchestrate.runtime.map_elites_bridge import (
    niche_to_coords,
    patch_to_candidate,
)

logger = logging.getLogger(__name__)


@dataclass(frozen=True, slots=True)
class TickReport:
    """Summary of a single tick.

    `cells_updated` counts cells whose `best_blueprint_version` or
    `fitness` changed. `cells_created` counts brand-new cells. The
    summary is what the RQ task returns, so the worker log shows
    one tidy line.
    """

    inspected_patches: int
    cells_inspected: int
    cells_created: int
    cells_updated: int
    elites_per_niche: int
    window_hours: int


def run_map_elites_tick(
    *,
    window_hours: int = 24,
    elites_per_niche: int = 2,
) -> TickReport:
    """Run a single MAP-Elites archive tick.

    Args:
        window_hours: only look at EvolutionPatch rows created in
            the last N hours. Pass 0 or negative to inspect every
            patch (full-archive rebuild).
        elites_per_niche: how many elites to keep per niche (Spec2
            sec 7.v2: "cell value: best ToolGraph config"; default 2
            so the engine can fall back to the runner-up when the
            top elite is unavailable).

    Returns:
        TickReport summary.
    """

    qs = EvolutionPatch.objects.all().order_by('-created_at')
    if window_hours and window_hours > 0:
        cutoff = timezone.now() - timezone.timedelta(hours=window_hours)
        qs = qs.filter(created_at__gte=cutoff)

    candidates = []
    inspected_patches = 0
    for patch in qs.select_related('target_blueprint'):
        try:
            cand = patch_to_candidate(patch)
        except Exception:
            logger.exception(
                'map_elites_tick.patch_to_candidate_failed',
                extra={'patch_id': patch.pk},
            )
            continue
        candidates.append((patch, cand))
        inspected_patches += 1

    if not candidates:
        return TickReport(
            inspected_patches=0,
            cells_inspected=0,
            cells_created=0,
            cells_updated=0,
            elites_per_niche=elites_per_niche,
            window_hours=window_hours,
        )

    engine = NativeEvolutionEngine()
    receipt = engine.archive(
        [c for _patch, c in candidates],
        elites_per_niche=elites_per_niche,
    )

    # Map every candidate back to the originating patch so we can
    # read `target_blueprint_id` when writing the cell row. The
    # candidate_id is the patch's pk (saved patches).
    patch_by_cand_id: dict[str, EvolutionPatch] = {}
    for patch, cand in candidates:
        patch_by_cand_id[cand.candidate_id] = patch

    cells_inspected = 0
    cells_created = 0
    cells_updated = 0
    for niche, niche_elites in (receipt.elites_by_niche or {}).items():
        try:
            task_type, domain, complexity = niche_to_coords(niche)
        except ValueError:
            logger.warning(
                'map_elites_tick.niche_not_coords',
                extra={'niche': niche},
            )
            continue
        cells_inspected += 1
        if not niche_elites:
            continue
        best = niche_elites[0]
        candidate_id = str(best.get('candidate_id') or '')
        score = float(best.get('score') or 0.0)
        owning_patch = patch_by_cand_id.get(candidate_id)
        blueprint: AgentBlueprint | None = (
            owning_patch.target_blueprint if owning_patch else None
        )
        cell, created = EvolutionCell.objects.get_or_create(
            task_type=task_type,
            domain=domain,
            complexity=complexity,
            defaults={
                'best_blueprint_version': blueprint,
                'fitness': score,
            },
        )
        if created:
            cells_created += 1
            continue
        # Update only if the new candidate's score is strictly higher.
        if score > float(cell.fitness):
            cell.fitness = score
            cell.best_blueprint_version = blueprint
            cell.save(update_fields=[
                'fitness',
                'best_blueprint_version',
                'updated_at',
            ])
            cells_updated += 1

    return TickReport(
        inspected_patches=inspected_patches,
        cells_inspected=cells_inspected,
        cells_created=cells_created,
        cells_updated=cells_updated,
        elites_per_niche=elites_per_niche,
        window_hours=window_hours,
    )


def report_to_dict(report: TickReport) -> dict[str, Any]:
    """Convert a TickReport to a JSON-safe dict for RQ return values."""

    return {
        'inspected_patches': report.inspected_patches,
        'cells_inspected': report.cells_inspected,
        'cells_created': report.cells_created,
        'cells_updated': report.cells_updated,
        'elites_per_niche': report.elites_per_niche,
        'window_hours': report.window_hours,
    }


__all__ = ['TickReport', 'report_to_dict', 'run_map_elites_tick']
