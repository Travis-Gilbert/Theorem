#!/usr/bin/env python3
"""Generate context-compiler parity fixtures from the canonical Python context_web.

Claude-Code lane of the harness Rust port (see ./README.md). Drives the live
`apps/orchestrate/context_web` pure core: `ContextWebPack.bounded` (capsule-budget
packing) and the policy functions (`normalize_context_web_node_id`,
`is_generated_artifact`). The running Python is the oracle; nothing is
hand-asserted. The Rust port (spec step 4) is correct when it reproduces this
corpus.

context_web/contracts.py + policy.py are pure (stdlib + a single intra-package
import), so we copy just those two into a throwaway flat package and bypass
context_web/__init__.py (which imports the IO retriever).

Run:  python3 generate_context_fixtures.py --check
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

PURE_SOURCES = ("contracts.py", "policy.py")


def _find_context_web_dir() -> Path:
    here = Path(__file__).resolve()
    for parent in here.parents:
        candidate = parent / "Index-API" / "apps" / "orchestrate" / "context_web"
        if (candidate / "contracts.py").is_file():
            return candidate
    raise SystemExit(
        "Could not locate Index-API/apps/orchestrate/context_web above "
        f"{here}. Pass --context-web <path> explicitly."
    )


def _load_reference(cw_dir: Path):
    tmp = Path(tempfile.mkdtemp(prefix="context_web_ref_"))
    pkg = tmp / "cw_ref"
    pkg.mkdir()
    (pkg / "__init__.py").write_text("")
    for name in PURE_SOURCES:
        shutil.copy2(cw_dir / name, pkg / name)
    sys.path.insert(0, str(tmp))
    import cw_ref.contracts as contracts  # noqa: E402
    import cw_ref.policy as policy  # noqa: E402
    return contracts, policy, tmp


def _atom(id, score, tokens, labels=()):
    return {"id": id, "score": score, "estimated_tokens": tokens, "labels": list(labels)}


# Pack scenarios: each is (name, description, pack_spec). pack_spec keys:
#   budget: dict of ContextWebBudget overrides
#   mode:   str
#   atoms:  list of atom dicts (ContextWebAtom.from_dict input)
#   edges:  list of {from_id,to_id,relation}
#   paths:  list of {node_ids:[...]}
#   ledger: dict of ContextWebTokenLedger overrides
#   policy: dict of ContextWebPolicy overrides
PACK_SCENARIOS = [
    ("basic_all_fit", "Three atoms, all fit; ranked by (-score, id).", {
        "atoms": [_atom("a", 0.9, 100), _atom("b", 0.5, 100), _atom("c", 0.7, 100)],
    }),
    ("token_budget_exhausted", "Mid-rank atom skipped on token budget; later smaller atom fits.", {
        "budget": {"max_tokens": 250},
        "atoms": [_atom("a", 0.9, 200), _atom("b", 0.8, 200), _atom("c", 0.7, 50)],
    }),
    ("atom_budget_exhausted", "max_atoms=2 drops the tail.", {
        "budget": {"max_atoms": 2},
        "atoms": [_atom("a", 0.9, 10), _atom("b", 0.8, 10),
                  _atom("c", 0.7, 10), _atom("d", 0.6, 10)],
    }),
    ("generated_artifact_quarantined", "A high-score generated artifact is quarantined.", {
        "atoms": [_atom("a", 0.9, 50), _atom("file:dist/bundle.js", 0.95, 50)],
    }),
    ("policy_allows_generated", "allow_generated_artifacts lets the artifact in.", {
        "atoms": [_atom("a", 0.9, 50), _atom("file:dist/bundle.js", 0.95, 50)],
        "policy": {"allow_generated_artifacts": True},
    }),
    ("policy_explicit_target_allows", "An explicit target overrides quarantine.", {
        "atoms": [_atom("a", 0.9, 50), _atom("file:dist/bundle.js", 0.95, 50)],
        "policy": {"explicit_targets": ["file:dist/bundle.js"]},
    }),
    ("mini_mode_caps_atoms", "mini mode caps max_atoms to 6.", {
        "mode": "mini",
        "atoms": [_atom(chr(ord('a') + i), 0.9 - i * 0.01, 40) for i in range(8)],
    }),
    ("edges_filtered_to_selected", "Edges with an unselected endpoint are dropped.", {
        "atoms": [_atom("a", 0.9, 50), _atom("b", 0.8, 50), _atom("c", 0.7, 50)],
        "edges": [{"from_id": "a", "to_id": "b", "relation": "calls"},
                  {"from_id": "a", "to_id": "z", "relation": "calls"}],
    }),
    ("paths_filtered_to_selected", "Paths with an unselected node are dropped.", {
        "atoms": [_atom("a", 0.9, 50), _atom("b", 0.8, 50), _atom("c", 0.7, 50)],
        "paths": [{"node_ids": ["a", "b"]}, {"node_ids": ["a", "z"]}],
    }),
    ("tie_break_by_id", "Equal scores break ties by ascending id.", {
        "atoms": [_atom("c", 0.5, 10), _atom("a", 0.5, 10), _atom("b", 0.5, 10)],
    }),
    ("ledger_raw_override", "token_ledger.raw_candidate_tokens overrides the atom sum.", {
        "atoms": [_atom("a", 0.9, 100)],
        "ledger": {"raw_candidate_tokens": 1000},
    }),
    ("empty_pack", "No atoms; ledger zeros.", {"atoms": []}),
]

# Policy cases: (raw_id, labels) -> {normalized, is_generated}.
POLICY_CASES = [
    ("file:apps/foo.py", []),
    ("file:dist/bundle.js", []),
    ("file/foo.py", []),
    ("file/dist/x.js", []),
    ("dist/x.js", []),
    ("node_modules/lib/x.js", []),
    ("plain-id", []),
    ("anything", ["GeneratedArtifact"]),
    ("generated:already", []),
    ("", []),
]


def _build_pack(contracts, spec):
    budget = contracts.ContextWebBudget(**spec.get("budget", {}))
    atoms = tuple(contracts.ContextWebAtom.from_dict(a) for a in spec.get("atoms", []))
    edges = tuple(contracts.ContextWebEdge(**e) for e in spec.get("edges", []))
    paths = tuple(
        contracts.ContextWebPath(
            node_ids=tuple(p["node_ids"]),
            edge_relations=tuple(p.get("edge_relations", ())),
            score=float(p.get("score", 0.0)),
        )
        for p in spec.get("paths", [])
    )
    ledger = contracts.ContextWebTokenLedger(**spec.get("ledger", {}))
    return contracts.ContextWebPack(
        run_id="run-context-fixture",
        query="fixture query",
        mode=spec.get("mode", "standard"),
        budget=budget,
        atoms=atoms,
        edges=edges,
        paths=paths,
        token_ledger=ledger,
    )


def _build(cw_dir: Path) -> dict:
    contracts, policy, tmp = _load_reference(cw_dir)
    try:
        pack_scenarios = []
        for name, description, spec in PACK_SCENARIOS:
            pack = _build_pack(contracts, spec)
            policy_obj = contracts.ContextWebPolicy(**spec.get("policy", {}))
            result = pack.bounded(policy=policy_obj)
            pack_scenarios.append({
                "name": name,
                "description": description,
                "input": spec,
                "expected": result.to_dict(),
            })
        policy_cases = []
        for raw_id, labels in POLICY_CASES:
            policy_cases.append({
                "input": {"id": raw_id, "labels": labels},
                "expected": {
                    "normalized": policy.normalize_context_web_node_id(raw_id),
                    "is_generated": policy.is_generated_artifact(raw_id, labels=tuple(labels)),
                },
            })
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return {
        "meta": {
            "purpose": "Context-compiler pack-core parity: Python reference -> Rust port.",
            "reference_source": "Index-API/apps/orchestrate/context_web/{contracts,policy}.py",
            "note": "Generated; do not hand-edit. Re-run generate_context_fixtures.py.",
            "consumer": "theorem-harness-core context pack port (spec step 4)",
        },
        "pack_scenarios": pack_scenarios,
        "policy_cases": policy_cases,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--context-web", type=Path, default=None)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()

    cw_dir = args.context_web or _find_context_web_dir()
    corpus = _build(cw_dir)
    encoded = json.dumps(corpus, indent=2, sort_keys=True)

    if args.check:
        again = json.dumps(_build(cw_dir), indent=2, sort_keys=True)
        if encoded != again:
            raise SystemExit("DETERMINISM FAILURE: context packing is not stable.")
        print("determinism check: OK (two runs byte-identical)")

    out_path = Path(__file__).resolve().parent / "context_fixtures.json"
    out_path.write_text(encoded + "\n")
    print(f"wrote {out_path}")
    print(f"pack_scenarios={len(corpus['pack_scenarios'])} "
          f"policy_cases={len(corpus['policy_cases'])}")


if __name__ == "__main__":
    main()
