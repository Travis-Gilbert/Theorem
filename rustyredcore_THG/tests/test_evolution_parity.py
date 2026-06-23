"""Cross-language byte-parity gate for the native evolution archive (RT-5.2a).

Asserts the native Rust `theseus_native.bgi_evolution_archive_json` produces an
ArchiveReceipt byte-identical to the Python `EvolutionEngine().archive()`
reference on the surface that matters: archive_hash, elites_by_niche (including
the (score, novelty, candidate_id) reverse-tiebreak ordering), and
rejected_count. Float-mantissa scores exercise serialization parity.

Distinct from `apps/notebook/inference_engines/evolution/tests/test_native_evolution.py`
which tests the bridge/fallback wiring against a FAKE native module; this hits
the real built wheel. Owner: claude-code (verification lane); the native impl
lives in rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs (codex lane).
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

import pytest

_REPO_ROOT = Path(__file__).resolve().parents[2]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

theseus_native = pytest.importorskip("theseus_native")

pytestmark = pytest.mark.skipif(
    not hasattr(theseus_native, "bgi_evolution_archive_json"),
    reason="installed theseus_native wheel does not include the evolution export",
)


def _candidates():
    from apps.notebook.inference_engines.evolution.contracts import EvolutionCandidate

    return [
        EvolutionCandidate("c1", "alpha", 0.9, {"x": 1}, 0.1),
        EvolutionCandidate("c2", "alpha", 0.9, {"x": 2}, 0.2),  # score tie with c1; novelty breaks
        EvolutionCandidate("c3", "alpha", 0.8, {"x": 3}, 0.5),  # dropped at elites=2
        EvolutionCandidate("c4", "beta", 0.7333333333333333, {"y": 1}, 0.0),
        EvolutionCandidate("c5", "beta", 0.7333333333333333, {"y": 2}, 0.0),  # full tie; candidate_id breaks
        EvolutionCandidate("c6", "gamma", 0.5, {"z": 9}, 0.9),
    ]


def _native_archive(candidates, elites_per_niche):
    from apps.notebook.inference_engines.common import stable_json

    payload = {
        "candidates": [candidate.to_dict() for candidate in candidates],
        "elites_per_niche": elites_per_niche,
    }
    return json.loads(theseus_native.bgi_evolution_archive_json(stable_json(payload)))


@pytest.mark.parametrize("elites_per_niche", [1, 2])
def test_evolution_archive_byte_parity(elites_per_niche: int) -> None:
    from apps.notebook.inference_engines.evolution.engine import EvolutionEngine

    candidates = _candidates()
    python_receipt = EvolutionEngine().archive(candidates, elites_per_niche=elites_per_niche).to_dict()
    native_receipt = _native_archive(candidates, elites_per_niche)

    for field in ("archive_hash", "elites_by_niche", "rejected_count"):
        assert python_receipt[field] == native_receipt.get(field), (
            f"evolution archive diverges on {field} at elites_per_niche={elites_per_niche}:\n"
            f"  python: {json.dumps(python_receipt.get(field), sort_keys=True)}\n"
            f"  native: {json.dumps(native_receipt.get(field), sort_keys=True)}"
        )
