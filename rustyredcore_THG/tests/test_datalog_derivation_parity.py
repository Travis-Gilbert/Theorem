"""Cross-language byte-parity gate: Python reference vs native theseus_native.

This is the RT-1 / RT-2 acceptance test. It asserts the native Rust symbolic
engines (rustyredcore_THG/crates/rustyred-thg-core/src/symbolic.rs) produce
receipts byte-identical to the Python reference across the current Datalog rule
set, both probabilistic functions, and the MAP-Elites archive path.

Owner: claude-code (verification lane). When a rule diverges, the failure names
the exact rule and prints the Python-vs-native receipts so the symbolic.rs owner
(codex lane) can fix it. The native implementation is gated on this test.
"""

from __future__ import annotations

import sys
from pathlib import Path

import pytest

# Repo root so the pure `apps.notebook.inference_engines.*` reference modules import.
_REPO_ROOT = Path(__file__).resolve().parents[2]
if str(_REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(_REPO_ROOT))

theseus_native = pytest.importorskip("theseus_native")

pytestmark = pytest.mark.skipif(
    not all(
        hasattr(theseus_native, name)
        for name in (
            "bgi_datalog_derive_core_json",
            "bgi_datalog_verified_rule_ids_json",
            "bgi_probabilistic_source_reliability_json",
            "bgi_probabilistic_expected_value_json",
            "bgi_evolution_archive_json",
        )
    ),
    reason="installed theseus_native wheel does not include the symbolic exports",
)


def test_datalog_derivation_byte_parity_all_rules() -> None:
    from apps.notebook.benchmarks.datalog_derivation_parity import run_datalog_derivation_parity

    report = run_datalog_derivation_parity(theseus_native)
    failures = [row for row in report["per_rule"] if not row["equal"]]
    detail = "\n".join(
        f"  {row['rule_id']}: python={row['python_derived_count']} native={row['native_derived_count']} "
        f"fact_pack_hash_match={row['fact_pack_hash_match']}"
        for row in failures
    )
    assert report["all_rules_equal"], "all-rules receipt diverges from Python reference"
    assert not failures, f"datalog rules diverge from Python reference:\n{detail}"


def test_probabilistic_byte_parity() -> None:
    from apps.notebook.benchmarks.datalog_derivation_parity import run_probabilistic_parity

    report = run_probabilistic_parity(theseus_native)
    assert report["passed"], f"probabilistic receipts diverge: {report['failures']}"


def test_evolution_archive_byte_parity() -> None:
    from apps.notebook.benchmarks.datalog_derivation_parity import run_evolution_archive_parity

    report = run_evolution_archive_parity(theseus_native)
    assert report["passed"], (
        "evolution archive receipts diverge: "
        f"{report['failures']} python={report['python_receipt']} native={report['native_receipt']}"
    )


def test_datalog_edge_case_byte_parity() -> None:
    from apps.notebook.benchmarks.datalog_derivation_parity import run_edge_case_parity

    report = run_edge_case_parity(theseus_native)
    assert report["passed"], f"datalog edge-case receipts diverge: {report['failures']}"
