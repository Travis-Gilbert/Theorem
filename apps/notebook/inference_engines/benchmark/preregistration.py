"""Locked pre-registered predictions (spec section 5).

Written to the ledger header before any run so cost-reduction goalposts cannot
move post-hoc. These numbers are the contract: two independent estimators
(Claude, Codex) recorded before the test. Treat this module as append-only
history if a v2 prediction set is ever needed; do not silently edit after a run.
"""

from __future__ import annotations

PRE_REGISTRATION: dict = {
    'spec': 'docs/plans/compute-offload/implementation-plan.md#5',
    'locked_on': '2026-05-28',
    'estimators': ['claude', 'codex'],
    'axes': {
        'symbolic_offload_mixed': {
            'claude_pct': [10, 25],
            'codex_pct': [10, 25],
        },
        'cascade_synthesis_heavy': {
            'claude_pct': [25, 45],
            'codex_pct': [20, 40],
            'quality_retention_pct': [88, 93],
        },
        'reuse_realistic_locality': {
            'claude_pct': [15, 35],
            'codex_pct': [10, 30],
            'novel_stream_pct': [0, 0],
        },
        'combined_vs_baseline_a': {
            'claude_pct': [40, 60],
            'codex_pct': [35, 55],
        },
    },
    'converged_band_pct': [35, 60],
    'reprice_threshold_pct': 30,
    'cascade_quality_retention_pct': [88, 93],
    'cascade_literature_value_rejected_pct': 95,
    'reuse_requires_invalidation': True,
    'go_target': {
        'combined_pct_min': 40,
        'quality_retention_pct_min': 90,
        'reuse_hit_rate_grows_with_corpus': True,
    },
    'notes': (
        'Combined 35-60% is a real, widening structural margin, not orders of '
        'magnitude: synthesis tokens dominate and synthesis stays on GPU. A 45% '
        'result is a win, not a letdown. Both estimators reprice/narrow the '
        'product claim if combined < 30%.'
    ),
}
