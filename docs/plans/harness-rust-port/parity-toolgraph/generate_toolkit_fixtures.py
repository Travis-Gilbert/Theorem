#!/usr/bin/env python3
"""Generate toolkit-selection parity fixtures from the canonical Python toolgraph.

Claude-Code lane of the harness Rust port (see ./README.md). Drives the live
`apps/orchestrate/runtime/toolgraph.py` `compile_task_toolkit` through a set of
(task_type, permissions, scope) inputs and records the real
`CompiledToolkit.to_dict()`. The running Python is the oracle; nothing is
hand-asserted. The Rust port of toolgraph.py (spec step 6) is correct when it
reproduces this corpus byte-for-byte.

toolgraph.py is pure (imports only dataclasses, typing, .contracts), so we copy
it plus contracts.py into a throwaway flat package and import directly.

Run:  python3 generate_toolkit_fixtures.py
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

PURE_SOURCES = ("contracts.py", "toolgraph.py")


def _find_runtime_dir() -> Path:
    here = Path(__file__).resolve()
    for parent in here.parents:
        candidate = parent / "Index-API" / "apps" / "orchestrate" / "runtime"
        if (candidate / "toolgraph.py").is_file():
            return candidate
    raise SystemExit(
        "Could not locate Index-API/apps/orchestrate/runtime above "
        f"{here}. Pass --runtime <path> explicitly."
    )


def _load_reference(runtime_dir: Path):
    tmp = Path(tempfile.mkdtemp(prefix="toolgraph_ref_"))
    pkg = tmp / "tg_ref"
    pkg.mkdir()
    (pkg / "__init__.py").write_text("")
    for name in PURE_SOURCES:
        shutil.copy2(runtime_dir / name, pkg / name)
    sys.path.insert(0, str(tmp))
    import tg_ref.toolgraph as tg  # noqa: E402
    return tg, tmp


# Each scenario: (name, description, task_type, permissions, scope).
# permissions: list[str] grants exactly those; [] grants none (blocks all);
# None falls through to the catalog default (web_browse + graph_read).
SCENARIOS = [
    ("research_full", "All research-candidate tools selected with full perms.",
     "research", ["web_browse", "graph_read"], {}),
    ("search_full", "Search task type, full perms.",
     "search", ["web_browse", "graph_read"], {}),
    ("fix_full", "Fix task type has a narrower candidate set.",
     "fix", ["web_browse", "graph_read"], {}),
    ("remember_full", "Remember selects memory_patch_validation + artifact compile.",
     "remember", ["web_browse", "graph_read"], {}),
    ("memory_full", "Memory task type (alias) keeps memory_patch_validation.",
     "memory", ["web_browse", "graph_read"], {}),
    ("other_full", "Other selects only the always-on tools.",
     "other", ["web_browse", "graph_read"], {}),
    ("unknown_task_normalizes_to_other", "Unknown task types normalize to 'other'.",
     "frobnicate", ["web_browse", "graph_read"], {}),
    ("research_graph_read_only", "graph_read-only blocks web_browse tools (native_search).",
     "research", ["graph_read"], {}),
    ("research_no_permissions", "Empty permission set blocks every tool.",
     "research", [], {}),
    ("research_default_permissions", "permissions=None falls through to catalog default.",
     "research", None, {}),
    ("research_tool_scope_pull_in",
     "scope.tool_scope pulls in a tool whose task_types is empty.",
     "research", ["web_browse", "graph_read"], {"tool_scope": ["neural_search.search"]}),
]


def _build(runtime_dir: Path) -> dict:
    tg, tmp = _load_reference(runtime_dir)
    try:
        scenarios = []
        for name, description, task_type, permissions, scope in SCENARIOS:
            toolkit = tg.compile_task_toolkit(
                task_type, permissions=permissions, scope=scope,
            )
            scenarios.append({
                "name": name,
                "description": description,
                "input": {
                    "task_type": task_type,
                    "permissions": permissions,
                    "scope": scope,
                },
                "expected": toolkit.to_dict(),
            })
        catalog = tg.catalog_as_dicts()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return {
        "meta": {
            "purpose": "Toolkit-selection parity: Python reference -> Rust port acceptance.",
            "reference_source": "Index-API/apps/orchestrate/runtime/toolgraph.py",
            "note": "Generated; do not hand-edit. Re-run generate_toolkit_fixtures.py.",
            "consumer": "theorem-harness-core toolgraph port (spec step 6)",
        },
        "catalog": catalog,
        "scenarios": scenarios,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, default=None)
    parser.add_argument("--check", action="store_true",
                        help="Run twice and assert byte-identical output.")
    args = parser.parse_args()

    runtime_dir = args.runtime or _find_runtime_dir()
    corpus = _build(runtime_dir)
    encoded = json.dumps(corpus, indent=2, sort_keys=True)

    if args.check:
        again = json.dumps(_build(runtime_dir), indent=2, sort_keys=True)
        if encoded != again:
            raise SystemExit("DETERMINISM FAILURE: toolkit compilation is not stable.")
        print("determinism check: OK (two runs byte-identical)")

    out_path = Path(__file__).resolve().parent / "toolkit_fixtures.json"
    out_path.write_text(encoded + "\n")
    sel = sum(len(s["expected"]["selected_tools"]) for s in corpus["scenarios"])
    blk = sum(len(s["expected"]["blocked_tools"]) for s in corpus["scenarios"])
    print(f"wrote {out_path}")
    print(f"scenarios={len(corpus['scenarios'])} catalog_tools={len(corpus['catalog'])} "
          f"total_selected={sel} total_blocked={blk}")


if __name__ == "__main__":
    main()
