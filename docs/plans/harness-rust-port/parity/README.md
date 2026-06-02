# Harness parity corpus

Authoritative reference fixtures for the `theorem-harness-core` Rust port. The
Python reference state machine (`Index-API/apps/orchestrate/runtime/`) is the
oracle; this corpus is its recorded behavior. The Rust port is correct for
Phase 1 when it reproduces every `state_hash_after` and every guard code here.

## Files

- `generate_fixtures.py` - drives the live Python `apply_transition` through 25
  scenarios and records real output. Re-runnable; reads Index-API directly.
- `fixtures.json` - the generated corpus (do not hand-edit; re-run the script).

## Regenerate

```bash
python3 docs/plans/harness-rust-port/parity/generate_fixtures.py --check
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

`rustyredcore_THG/crates/theorem-harness-core/tests/parity.rs` loads
`fixtures.json` directly from this docs directory with `include_str!`. For each
scenario it threads an `Option<RunState>` through `apply_transition`, then
compares `state_hash_before`, `state_hash_after`, status, event sequence, and
guard code against the Python-recorded output.

## Coverage (25 scenarios, 260 steps)

Legal: `full_lifecycle_to_closed` (created -> ... -> closed),
`memory_patch_branch` (learning_proposed -> memory_patched -> maps_updated ->
closed), `cache_hit_validated`, `cache_miss`, `oracle_status_preserving`,
`cua_status_preserving`, `cmh_handoff_branch`,
`cmh_canonicalization_to_close`, `run_forked_from_closed`, and
`run_replayed_from_closed`,
`domain_toolpack_context_compiled_validation`,
`cache_rejected_reuse_store_invalidate`,
`cua_device_session_terminal_observations`, `run_failed_then_forked`,
`run_cancelled_rejects_followup`, and `cmh_session_event_self_loop`.

Guards (code captured from the Python reference, not hand-asserted):
`invalid_context_budget`, `context_budget_exceeded`,
`context_artifact_mismatch`, `memory_patch_review_required`, `run_id_mismatch`,
`terminal_run_state`, `missing_payload_fields`, `federation_consent_required`,
`federation_raw_content_blocked`.

Guard-parity review (2026-06-01): the codes above are present in Rust
`state_machine.rs`, and the parity test compares both hashes and guard codes
byte-for-byte.
