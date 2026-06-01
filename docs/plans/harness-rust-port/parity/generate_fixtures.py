#!/usr/bin/env python3
"""Generate harness parity fixtures from the canonical Python state machine.

This is the Claude-Code lane of the harness Rust port (see ../CLAIMS.md): the
authoritative reference corpus that `theorem-harness-core`'s Rust parity test
validates against. It drives the LIVE Theseus executor
(`Index-API/apps/orchestrate/runtime/state_machine.py`) through legal and
illegal transition sequences and records, per step, the real `state_hash_after`,
the resulting status, and (for illegal steps) the guard code Python actually
raises. Nothing here is hand-authored ground truth; the running reference is the
oracle.

Output: ./fixtures.json (the corpus the Rust test loads).

Run:  python3 generate_fixtures.py
      python3 generate_fixtures.py --check   # determinism self-check (run twice, diff)

The three source files are pure stdlib (no Django), so we copy them into a
throwaway flat package and import `apply_transition` directly, bypassing the
coupled `apps/orchestrate/runtime/__init__.py`.
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

# Pinned so digests are deterministic. STATE_HASH_FIELDS excludes created_at /
# updated_at, so only run_id must be pinned; created_at is pinned anyway for
# fully reproducible event records.
RUN_ID = "run-fixture-0001"
TS = "2026-06-01T00:00:00+00:00"

PURE_SOURCES = ("contracts.py", "state_hash.py", "state_machine.py")


def _find_runtime_dir() -> Path:
    """Locate Index-API/apps/orchestrate/runtime by walking up for a sibling."""
    here = Path(__file__).resolve()
    for parent in here.parents:
        candidate = parent / "Index-API" / "apps" / "orchestrate" / "runtime"
        if (candidate / "state_machine.py").is_file():
            return candidate
    raise SystemExit(
        "Could not locate Index-API/apps/orchestrate/runtime above "
        f"{here}. Pass --runtime <path> explicitly."
    )


def _load_reference(runtime_dir: Path):
    """Copy the pure files into a temp package and import apply_transition."""
    tmp = Path(tempfile.mkdtemp(prefix="harness_ref_"))
    pkg = tmp / "harness_ref"
    pkg.mkdir()
    (pkg / "__init__.py").write_text("")
    for name in PURE_SOURCES:
        shutil.copy2(runtime_dir / name, pkg / name)
    sys.path.insert(0, str(tmp))
    import harness_ref.state_machine as sm  # noqa: E402
    return sm, tmp


def _input(event_type: str, payload: dict) -> dict:
    return {
        "type": event_type,
        "run_id": RUN_ID,
        "payload": payload,
        "actor": "claude-code",
        "created_at": TS,
    }


# ----------------------------------------------------------------------------
# Scenario authoring. Each scenario is a list of (input, expect) steps.
# expect == "ok"    -> step must succeed; record state_hash_after + status.
# expect == "guard" -> step must raise HarnessGuardError; record the guard code.
# The legal prefix that sets up an illegal step is shared via _legal_prefix.
# ----------------------------------------------------------------------------

def _legal_prefix(through: str) -> list:
    """Legal steps up to and including the named status checkpoint."""
    steps = [
        ("RUN.CREATED", {
            "task": "port harness to rust",
            "actor": "claude-code",
            "scope": {
                "repo": "Theorem", "branch": "main", "commit_sha": "deadbeef",
                "workstream_id": "ws-harness", "agent_host": "claude-code",
                "agent_model": "opus",
            },
        }),
        ("HOST.OBSERVED", {
            "repo": "Theorem", "branch": "main", "commit_sha": "deadbeef",
            "cwd": "/repo/Theorem",
        }),
        ("TASK.RESOLVED", {"task_signature": "sig-port-harness"}),
        ("PROFILE.SELECTED", {
            "profile_id": "rust-port", "profile_version": "1",
            "policy_hash": "policy-abc",
        }),
        ("TOOLKIT.COMPILED", {
            "selected_tools": ["read", "edit"], "selected_plugins": [],
            "excluded_tools": ["network"],
            "permission_reasons": {"network": "policy:no-egress"},
        }),
        ("MAPS.LOADED", {"maps": [{"id": "codebase", "version": "1"}]}),
        ("CONTEXT.PLANNED", {
            "budget_tokens": 1000, "plan_hash": "plan-1",
            "candidate_token_count": 500,
        }),
        ("CONTEXT.PACKED", {
            "artifact_id": "art-1", "capsule_tokens": 200, "budget_tokens": 1000,
            "included_atom_count": 5, "excluded_atom_count": 2,
            "token_ledger": {"saved": 300},
        }),
        ("CONTEXT.INJECTED", {
            "artifact_id": "art-1", "adapter": "mcp", "target": "claude",
        }),
        ("AGENT.ACTING", {"adapter": "mcp", "started_at": TS}),
        ("OUTCOME.RECORDED", {
            "accepted": True, "tests_passed": True,
            "validator_results": [{"id": "v1", "status": "passed"}],
            "files_changed": ["state_machine.rs"], "summary": "ported",
        }),
        ("LEARNING.PROPOSED", {
            "patch_type": "memory", "confidence": 0.8,
            "review_required": True, "payload_hash": "patch-1",
        }),
        ("REVIEW.QUEUED", {"review_type": "memory", "review_target_id": "patch-1"}),
        ("FEDERATION.SIGNAL_PREPARED", {
            "plugin_id": "core", "profile_id": "rust-port",
            "task_type": "port", "task_signature_hash": "tsh-1",
            "context_shape_hash": "csh-1", "outcome_bucket": "accepted",
            "token_bucket": "small", "raw_content_included": False,
            "consent": True,
        }),
        ("RUN.CLOSED", {"summary": "harness kernel ported", "closed_by": "claude-code"}),
    ]
    checkpoints = {
        "maps_loaded": 6, "context_planned": 7, "context_packed": 8,
        "context_injected": 9, "learning_proposed": 12, "review_queued": 13,
        "closed": 16,
    }
    return steps[: checkpoints[through]]


def _scenarios() -> list:
    ok = lambda et, p: (_input(et, p), "ok")
    guard = lambda et, p: (_input(et, p), "guard")

    scenarios = []

    # 1. Full legal lifecycle to RUN.CLOSED.
    scenarios.append({
        "name": "full_lifecycle_to_closed",
        "description": "Canonical legal chain created -> ... -> closed.",
        "steps": [ok(et, p) for et, p in _legal_prefix("closed")],
    })

    # 2. Memory-patch branch off learning_proposed.
    mp = [ok(et, p) for et, p in _legal_prefix("learning_proposed")]
    mp += [
        ok("MEMORY.PATCHED", {
            "patch_id": "patch-1", "status": "queued",
            "review_required": True, "payload_hash": "patch-1",
        }),
        ok("MAPS.UPDATED", {
            "maps": [{"id": "codebase", "version": "2"}], "state_hash": "sh-x",
        }),
        ok("RUN.CLOSED", {"summary": "closed via memory branch", "closed_by": "claude-code"}),
    ]
    scenarios.append({
        "name": "memory_patch_branch",
        "description": "learning_proposed -> memory_patched -> maps_updated -> closed.",
        "steps": mp,
    })

    # 3. invalid_context_budget: CONTEXT.PLANNED with budget 0.
    s = [ok(et, p) for et, p in _legal_prefix("maps_loaded")]
    s.append(guard("CONTEXT.PLANNED", {
        "budget_tokens": 0, "plan_hash": "plan-bad", "candidate_token_count": 10,
    }))
    scenarios.append({
        "name": "invalid_context_budget",
        "description": "CONTEXT.PLANNED requires a positive token budget.",
        "steps": s,
    })

    # 4. context_budget_exceeded: capsule_tokens > budget_tokens.
    s = [ok(et, p) for et, p in _legal_prefix("context_planned")]
    s.append(guard("CONTEXT.PACKED", {
        "artifact_id": "art-big", "capsule_tokens": 5000, "budget_tokens": 1000,
        "included_atom_count": 9, "excluded_atom_count": 0,
        "token_ledger": {"saved": -4000},
    }))
    scenarios.append({
        "name": "context_budget_exceeded",
        "description": "A capsule cannot exceed its token budget.",
        "steps": s,
    })

    # 5. context_artifact_mismatch: inject a different artifact than packed.
    s = [ok(et, p) for et, p in _legal_prefix("context_packed")]
    s.append(guard("CONTEXT.INJECTED", {
        "artifact_id": "art-OTHER", "adapter": "mcp", "target": "claude",
    }))
    scenarios.append({
        "name": "context_artifact_mismatch",
        "description": "Injected artifact_id must match the packed one.",
        "steps": s,
    })

    # 6. memory_patch_review_required: MEMORY.PATCHED with review_required False.
    s = [ok(et, p) for et, p in _legal_prefix("learning_proposed")]
    s.append(guard("MEMORY.PATCHED", {
        "patch_id": "patch-x", "status": "queued",
        "review_required": False, "payload_hash": "patch-x",
    }))
    scenarios.append({
        "name": "memory_patch_review_required",
        "description": "Memory patches require review before promotion.",
        "steps": s,
    })

    # 7. run_id_mismatch: a non-created event carrying a foreign run_id.
    created = _input("RUN.CREATED", {
        "task": "t", "actor": "claude-code",
        "scope": {"repo": "Theorem", "branch": "main", "commit_sha": "x"},
    })
    foreign = _input("HOST.OBSERVED", {
        "repo": "Theorem", "branch": "main", "commit_sha": "x", "cwd": "/r",
    })
    foreign["run_id"] = "run-SOMEONE-ELSE"
    scenarios.append({
        "name": "run_id_mismatch",
        "description": "Event run_id must match the run's id.",
        "steps": [(created, "ok"), (foreign, "guard")],
    })

    # 8. terminal_run_rejected: any transition after RUN.CLOSED.
    s = [ok(et, p) for et, p in _legal_prefix("closed")]
    s.append(guard("HOST.OBSERVED", {
        "repo": "Theorem", "branch": "main", "commit_sha": "x", "cwd": "/r",
    }))
    scenarios.append({
        "name": "terminal_run_rejected",
        "description": "Terminal runs reject further transitions.",
        "steps": s,
    })

    # 9. missing_required_field: RUN.CREATED without 'actor'.
    bad_created = _input("RUN.CREATED", {"task": "t"})
    scenarios.append({
        "name": "missing_required_field",
        "description": "RUN.CREATED requires task and actor.",
        "steps": [(bad_created, "guard")],
    })

    # 10. federation_consent_required: FEDERATION.SIGNAL_PREPARED without consent.
    s = [ok(et, p) for et, p in _legal_prefix("review_queued")]
    s.append(guard("FEDERATION.SIGNAL_PREPARED", {
        "plugin_id": "core", "profile_id": "rust-port", "task_type": "port",
        "task_signature_hash": "tsh-1", "context_shape_hash": "csh-1",
        "outcome_bucket": "accepted", "token_bucket": "small",
        "raw_content_included": False,
    }))
    scenarios.append({
        "name": "federation_consent_required",
        "description": "Federation signal preparation requires explicit consent.",
        "steps": s,
    })

    # 11. federation_raw_content_blocked: consent given but raw content included.
    s = [ok(et, p) for et, p in _legal_prefix("review_queued")]
    s.append(guard("FEDERATION.SIGNAL_PREPARED", {
        "plugin_id": "core", "profile_id": "rust-port", "task_type": "port",
        "task_signature_hash": "tsh-1", "context_shape_hash": "csh-1",
        "outcome_bucket": "accepted", "token_bucket": "small",
        "raw_content_included": True, "consent": True,
    }))
    scenarios.append({
        "name": "federation_raw_content_blocked",
        "description": "Federation signals cannot include raw content.",
        "steps": s,
    })

    return scenarios


def _run(sm, scenarios: list) -> list:
    """Execute scenarios through the Python reference, recording real output."""
    GuardError = sm.HarnessGuardError
    TransitionError = sm.HarnessTransitionError
    out = []
    for scenario in scenarios:
        state = None
        recorded_steps = []
        for index, (event_input, expect) in enumerate(scenario["steps"]):
            entry = {"input": event_input, "expect": expect}
            try:
                result = sm.apply_transition(state, event_input)
            except (GuardError, TransitionError) as exc:
                code = getattr(getattr(exc, "violation", None), "code", None)
                if expect != "guard":
                    raise SystemExit(
                        f"[{scenario['name']}] step {index} ({event_input['type']}) "
                        f"expected ok but raised guard {code}: {exc}"
                    )
                entry["guard_code"] = code
                entry["guard_message"] = str(exc)
                recorded_steps.append(entry)
                break  # terminal: a guarded step ends the scenario
            else:
                if expect != "ok":
                    raise SystemExit(
                        f"[{scenario['name']}] step {index} ({event_input['type']}) "
                        f"expected guard but succeeded (status={result.run.status})"
                    )
                state = result.run
                entry["state_hash_before"] = result.state_hash_before
                entry["state_hash_after"] = result.state_hash_after
                entry["status"] = result.run.status
                entry["seq"] = result.run.last_event_seq
                recorded_steps.append(entry)
        out.append({**scenario, "steps": recorded_steps})
    return out


def _build(runtime_dir: Path) -> dict:
    sm, tmp = _load_reference(runtime_dir)
    try:
        scenarios = _run(sm, _scenarios())
        anchors = {
            "empty_state_hash": sm.EMPTY_STATE_HASH,
        }
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    return {
        "meta": {
            "purpose": "Harness parity corpus: Python reference -> Rust port acceptance.",
            "reference_source": "Index-API/apps/orchestrate/runtime/state_machine.py",
            "pinned_run_id": RUN_ID,
            "pinned_created_at": TS,
            "note": "Generated; do not hand-edit. Re-run generate_fixtures.py.",
            "consumer": "rustyredcore_THG/crates/theorem-harness-core parity test",
        },
        "anchors": anchors,
        "scenarios": scenarios,
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runtime", type=Path, default=None)
    parser.add_argument("--check", action="store_true",
                        help="Run twice and assert byte-identical output (determinism).")
    args = parser.parse_args()

    runtime_dir = args.runtime or _find_runtime_dir()
    corpus = _build(runtime_dir)
    encoded = json.dumps(corpus, indent=2, sort_keys=True)

    if args.check:
        again = json.dumps(_build(runtime_dir), indent=2, sort_keys=True)
        if encoded != again:
            raise SystemExit("DETERMINISM FAILURE: a now()/random leaked into a hashed field.")
        print("determinism check: OK (two runs byte-identical)")

    out_path = Path(__file__).resolve().parent / "fixtures.json"
    out_path.write_text(encoded + "\n")
    n_steps = sum(len(s["steps"]) for s in corpus["scenarios"])
    print(f"wrote {out_path}")
    print(f"scenarios={len(corpus['scenarios'])} steps={n_steps} "
          f"empty_state_hash={corpus['anchors']['empty_state_hash'][:16]}...")


if __name__ == "__main__":
    main()
