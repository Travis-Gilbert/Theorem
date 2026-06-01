#!/usr/bin/env python3
"""Generate harness parity fixtures from the canonical Python state machine.

This is the authoritative reference corpus for the harness Rust port (see
../CLAIMS.md): the shared acceptance artifact that `theorem-harness-core`'s
Rust parity test validates against. It drives the LIVE Theseus executor
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
        "created": 1,
        "resolved": 3,
        "maps_loaded": 6, "context_planned": 7, "context_packed": 8,
        "context_injected": 9, "agent_acting": 10, "outcome_recorded": 11,
        "learning_proposed": 12, "review_queued": 13,
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

    # 12. Cache hit validated chain off TASK.RESOLVED.
    s = [ok(et, p) for et, p in _legal_prefix("resolved")]
    s += [
        ok("CACHE.CHECKED", {"backend": "rustyred", "outcome": "candidate"}),
        ok("CACHE.HIT", {"cache_entry_id": "cache-1", "backend": "rustyred"}),
        ok("CACHE.HIT_VALIDATED", {
            "cache_entry_id": "cache-1",
            "graph_state_hash": "graph-hash-1",
        }),
    ]
    scenarios.append({
        "name": "cache_hit_validated",
        "description": "Cache side-events advance through checked -> hit -> hit_validated.",
        "steps": s,
    })

    # 13. Cache miss chain off TASK.RESOLVED.
    s = [ok(et, p) for et, p in _legal_prefix("resolved")]
    s += [
        ok("CACHE.CHECKED", {"backend": "rustyred", "outcome": "candidate"}),
        ok("CACHE.MISS", {"backend": "rustyred", "outcome": "miss"}),
    ]
    scenarios.append({
        "name": "cache_miss",
        "description": "Cache side-events advance through checked -> miss.",
        "steps": s,
    })

    # 14. Oracle observation events are status-preserving.
    s = [ok(et, p) for et, p in _legal_prefix("agent_acting")]
    s += [
        ok("ORACLE.REQUESTED", {"tool_name": "deepseek_reason", "request_id": "oracle-1"}),
        ok("ORACLE.RETURNED", {
            "tool_name": "deepseek_reason",
            "request_id": "oracle-1",
            "oracle_packet": {"status": "ok"},
        }),
        ok("STATE.PATCHED", {
            "request_id": "oracle-1",
            "applied_patch_ids": ["patch-a"],
            "rejected_patch_ids": [],
        }),
        ok("ADAPTER.SELECTED", {
            "adapter_id": "deepseek-mcp",
            "role": "reasoning",
        }),
    ]
    scenarios.append({
        "name": "oracle_status_preserving",
        "description": "Oracle events are recorded without advancing the run lifecycle status.",
        "steps": s,
    })

    # 15. CUA / device timeline observation events are status-preserving.
    s = [ok(et, p) for et, p in _legal_prefix("agent_acting")]
    s += [
        ok("DEVICE.SESSION.STARTED", {"device_session_id": "dev-1", "provider": "cua"}),
        ok("CUA.SANDBOX.OPENED", {"device_session_id": "dev-1", "sandbox_id": "sbx-1"}),
        ok("CUA.ACTION.OBSERVED", {
            "sandbox_id": "sbx-1",
            "action_id": "act-1",
            "kind": "click",
            "seq": 1,
        }),
        ok("CUA.OBSERVATION.RECORDED", {
            "sandbox_id": "sbx-1",
            "observation_id": "obs-1",
            "kind": "screenshot",
            "seq": 2,
        }),
        ok("CUA.SANDBOX.CLOSED", {"sandbox_id": "sbx-1"}),
        ok("CUA.TRAJECTORY.EXPORTED", {
            "sandbox_id": "sbx-1",
            "trajectory_id": "traj-1",
            "action_count": 1,
            "observation_count": 1,
        }),
    ]
    scenarios.append({
        "name": "cua_status_preserving",
        "description": "CUA device timeline events are recorded without advancing lifecycle status.",
        "steps": s,
    })

    # 16. CMH handoff branch starts from created state.
    s = [ok(et, p) for et, p in _legal_prefix("created")]
    s += [
        ok("MEMORY.SYNCED", {"workstream_id": "ws-harness"}),
        ok("HANDOFF.COMPILED", {"handoff_id": "handoff-1", "token_estimate": 512}),
        ok("HANDOFF.INJECTED", {"delivered_to": "codex", "delivered_at": TS}),
    ]
    scenarios.append({
        "name": "cmh_handoff_branch",
        "description": "Continuous Agent Memory handoff transitions advance their own branch.",
        "steps": s,
    })

    # 17. CMH canonicalization branch can close after an outcome exists.
    s = [ok(et, p) for et, p in _legal_prefix("outcome_recorded")]
    s += [
        ok("MEMORY.CANONICALIZED", {
            "atoms_created": 2,
            "atoms_updated": 1,
            "atoms_superseded": 0,
        }),
        ok("WORKSTREAM.UPDATED", {
            "workstream_id": "ws-harness",
            "new_task_state": "validating",
        }),
        ok("NEXT_AGENT.READY", {"next_handoff_id": "handoff-next"}),
        ok("RUN.CLOSED", {"summary": "closed after cmh", "closed_by": "claude-code"}),
    ]
    scenarios.append({
        "name": "cmh_canonicalization_to_close",
        "description": "CMH memory canonicalization can lead to NEXT_AGENT.READY and close.",
        "steps": s,
    })

    # 18. RUN.FORKED is allowed from closed and resets the state to created.
    s = [ok(et, p) for et, p in _legal_prefix("closed")]
    s.append(ok("RUN.FORKED", {
        "source_run_id": RUN_ID,
        "through_event_seq": 9,
    }))
    scenarios.append({
        "name": "run_forked_from_closed",
        "description": "RUN.FORKED is permitted from closed and resets mutable run state.",
        "steps": s,
    })

    # 19. RUN.REPLAYED is allowed from closed and resets the state to created.
    s = [ok(et, p) for et, p in _legal_prefix("closed")]
    s.append(ok("RUN.REPLAYED", {"source_run_id": RUN_ID}))
    scenarios.append({
        "name": "run_replayed_from_closed",
        "description": "RUN.REPLAYED is permitted from closed and resets mutable run state.",
        "steps": s,
    })

    # 20. Domain/toolpack/context-compiled/validation alternate legal path.
    s = [ok(et, p) for et, p in _legal_prefix("resolved")]
    s += [
        ok("DOMAIN.RESOLVED", {
            "domain": "harness-port",
            "domain_version": "1",
            "policy_hash": "domain-policy-1",
        }),
        ok("TOOLPACK.COMPILED", {
            "selected_tools": ["read", "edit"],
            "selected_plugins": ["theorems-harness"],
            "excluded_tools": [],
            "permission_reasons": {},
        }),
        ok("MAPS.LOADED", {"maps": [{"id": "domain", "version": "1"}]}),
        ok("CONTEXT.PLANNED", {
            "budget_tokens": 1200,
            "plan_hash": "plan-domain",
            "candidate_token_count": 700,
        }),
        ok("CONTEXT.COMPILED", {
            "artifact_id": "art-domain",
            "capsule_tokens": 400,
            "budget_tokens": 1200,
            "included_atom_count": 7,
            "excluded_atom_count": 3,
            "token_ledger": {"saved": 300},
        }),
        ok("CONTEXT.INJECTED", {
            "artifact_id": "art-domain",
            "adapter": "stdio",
            "target": "codex",
        }),
        ok("AGENT.ACTING", {"adapter": "stdio", "started_at": TS}),
        ok("VALIDATION.STARTED", {"validator_id": "cargo-test", "command": "cargo test"}),
        ok("VALIDATION.RUNNING", {"validator_id": "cargo-test", "command": "cargo test"}),
        ok("VALIDATION.FINISHED", {
            "validator_id": "cargo-test",
            "status": "passed",
            "exit_code": 0,
            "summary": "ok",
        }),
        ok("OUTCOME.RECORDED", {
            "accepted": True,
            "tests_passed": True,
            "validator_results": [{"id": "cargo-test", "status": "passed"}],
            "files_changed": ["state_machine.rs"],
            "summary": "alternate path ported",
        }),
        ok("RUN.CLOSED", {"summary": "closed alternate path", "closed_by": "claude-code"}),
    ]
    scenarios.append({
        "name": "domain_toolpack_context_compiled_validation",
        "description": "Alternate profile/toolpack/context-compiled path with validation events.",
        "steps": s,
    })

    # 21. Remaining cache events: rejected, stage reused, entry stored, invalidated.
    s = [ok(et, p) for et, p in _legal_prefix("resolved")]
    s += [
        ok("CACHE.CHECKED", {"backend": "rustyred", "outcome": "candidate"}),
        ok("CACHE.HIT", {"cache_entry_id": "cache-2", "backend": "rustyred"}),
        ok("CACHE.HIT_REJECTED", {
            "cache_entry_id": "cache-2",
            "rejection_reason": "stale_graph_state",
        }),
        ok("CACHE.STAGE_REUSED", {"stage": "context", "cache_entry_id": "cache-3"}),
        ok("CACHE.ENTRY_STORED", {"cache_entry_id": "cache-4", "backend": "rustyred"}),
        ok("CACHE.INVALIDATED", {"cache_entry_id": "cache-4", "reason": "new_evidence"}),
    ]
    scenarios.append({
        "name": "cache_rejected_reuse_store_invalidate",
        "description": "Covers cache rejection and ungated cache bookkeeping events.",
        "steps": s,
    })

    # 22. Remaining CUA device-session events.
    s = [ok(et, p) for et, p in _legal_prefix("agent_acting")]
    s += [
        ok("DEVICE.SESSION.STARTED", {"device_session_id": "dev-2", "provider": "cua"}),
        ok("DEVICE.SESSION.CLOSED", {"device_session_id": "dev-2"}),
        ok("DEVICE.SESSION.ERRORED", {
            "device_session_id": "dev-2",
            "error_code": "sandbox_exit",
        }),
    ]
    scenarios.append({
        "name": "cua_device_session_terminal_observations",
        "description": "Device session close/error events remain status-preserving.",
        "steps": s,
    })

    # 23. RUN.FAILED is terminal, and RUN.FORKED may recover from failed.
    s = [ok(et, p) for et, p in _legal_prefix("created")]
    s += [
        ok("RUN.FAILED", {"error_code": "validation_failed", "message": "tests failed"}),
        ok("RUN.FORKED", {"source_run_id": RUN_ID, "through_event_seq": 1}),
    ]
    scenarios.append({
        "name": "run_failed_then_forked",
        "description": "RUN.FAILED creates failure outcome; RUN.FORKED resets from failed.",
        "steps": s,
    })

    # 24. RUN.CANCELLED is terminal and rejects further transitions.
    s = [ok(et, p) for et, p in _legal_prefix("created")]
    s += [
        ok("RUN.CANCELLED", {"reason": "user_requested", "cancelled_by": "travis"}),
        guard("HOST.OBSERVED", {
            "repo": "Theorem", "branch": "main", "commit_sha": "x", "cwd": "/r",
        }),
    ]
    scenarios.append({
        "name": "run_cancelled_rejects_followup",
        "description": "RUN.CANCELLED creates cancellation outcome and becomes terminal.",
        "steps": s,
    })

    # 25. SESSION.EVENT_RECORDED is a CMH self-loop from agent_acting.
    s = [ok(et, p) for et, p in _legal_prefix("agent_acting")]
    s.append(ok("SESSION.EVENT_RECORDED", {"event_subtype": "handoff_note"}))
    scenarios.append({
        "name": "cmh_session_event_self_loop",
        "description": "SESSION.EVENT_RECORDED records CMH activity without leaving agent_acting.",
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
