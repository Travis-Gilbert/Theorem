# Harness parity corpus (Claude-Code lane)

Authoritative reference fixtures for the `theorem-harness-core` Rust port. The
Python reference state machine (`Index-API/apps/orchestrate/runtime/`) is the
oracle; this corpus is its recorded behavior. The Rust port is correct for
Phase 1 when it reproduces every `state_hash_after` and every guard code here.

## Files

- `generate_fixtures.py` — drives the live Python `apply_transition` through 11
  scenarios and records real output. Re-runnable; reads Index-API directly.
- `fixtures.json` — the generated corpus (do not hand-edit; re-run the script).

## Regenerate

```bash
python3 generate_fixtures.py --check   # --check asserts two runs are byte-identical
```

`--check` guards determinism: if a `now()` or random value ever leaks into a
hashed field, the two runs diverge and it fails loudly. Today it passes (the
hashed field set excludes `created_at`/`updated_at`; only `run_id` is pinned).

## Schema

```
{
  "meta":   { provenance: reference_source, pinned_run_id, pinned_created_at },
  "anchors": { "empty_state_hash": "<sha256>" },
  "scenarios": [
    {
      "name": "...", "description": "...",
      "steps": [
        // legal step:
        { "input": {type, run_id, payload, actor, created_at},
          "expect": "ok",
          "state_hash_before": "<sha256>", "state_hash_after": "<sha256>",
          "status": "<run status>", "seq": <int> },
        // illegal terminal step (last step of the scenario):
        { "input": {...}, "expect": "guard",
          "guard_code": "<code>", "guard_message": "<text>" }
      ]
    }
  ]
}
```

A scenario's steps run in order from an empty (`None`) start state, threading
the run forward. An `expect: "guard"` step is always terminal: it must raise the
recorded `guard_code`; the scenario ends there.

## How the Rust parity test consumes this

Suggested shape (Codex owns the crate, so the exact wiring is Codex's call):

```rust
// tests/parity.rs in theorem-harness-core
// load fixtures.json (copy into tests/fixtures/ or read via a path const)
for scenario in corpus.scenarios {
    let mut state: Option<RunState> = None;
    for step in scenario.steps {
        let input = TransitionInput::from(step.input);
        match apply_transition(state.as_ref(), &input) {
            Ok(result) => {
                assert_eq!(step.expect, "ok");
                assert_eq!(result.state_hash_after, step.state_hash_after);
                assert_eq!(result.run.status, step.status);
                state = Some(result.run);
            }
            Err(HarnessError::Guard(v)) => {
                assert_eq!(step.expect, "guard");
                assert_eq!(v.code, step.guard_code);
                break;
            }
        }
    }
}
```

## Coverage (11 scenarios, 114 steps)

Legal: `full_lifecycle_to_closed` (created -> ... -> closed),
`memory_patch_branch` (learning_proposed -> memory_patched -> maps_updated ->
closed).

Guards (code captured from the Python reference, not hand-asserted):
`invalid_context_budget`, `context_budget_exceeded`,
`context_artifact_mismatch`, `memory_patch_review_required`, `run_id_mismatch`,
`terminal_run_state`, `missing_payload_fields`, `federation_consent_required`,
`federation_raw_content_blocked`.

Guard-parity review (2026-06-01): all 11 codes are present in the Rust
`state_machine.rs`. The remaining acceptance step is wiring the replay test so
the hashes are compared byte-for-byte, not just the codes.

## Not yet covered (next corpus passes)

cache events (CACHE.CHECKED/HIT/...), oracle events (ORACLE.REQUESTED/...), CUA
device events, the CMH chain (MEMORY.SYNCED/HANDOFF.*), replay/fork
(RUN.REPLAYED/RUN.FORKED). These are status-preserving or parallel-graph
transitions; add them once the linear-lifecycle gate is green in Rust.
