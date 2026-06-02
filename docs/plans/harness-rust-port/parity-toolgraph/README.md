# Toolgraph parity corpus (Claude-Code lane)

LANE CLAIM (2026-06-01, substrate down -> git is the channel):

- **claude-code owns this dir** (`docs/plans/harness-rust-port/parity-toolgraph/`):
  the Python reference corpus for the toolkit-selection port (spec step 6,
  `apps/orchestrate/runtime/toolgraph.py`).
- **codex owns** `theorem-harness-core/**`, `parity/**`, and the context-compiler
  port (spec step 4). This corpus does not touch any of those.

The split uses the spec's own ordering to stay orthogonal: Codex follows the
spec toward step 4 (context compiler, the headline); Claude runs ahead on step 6
(toolkit selection, a self-contained pure core that is ready now). Same division
that produced the green kernel gate: Claude builds the Python reference + corpus,
Codex writes the Rust against it.

## What this covers

`toolgraph.py` is fully pure (imports only `dataclasses`, `typing`,
`.contracts.ToolSelectionState`). `compile_task_toolkit(task_type, permissions,
scope)` runs the static `DEFAULT_TOOLS` catalog (10 tools) through:

1. task-type candidacy (`task_type in tool.task_types`),
2. `scope.tool_scope` explicit pull-in (a tool with `task_types=()` can still be
   requested),
3. the always-appended `context_artifact_compile`,
4. permission gating (`tool.permissions - run_permissions` -> blocked with
   `missing_permissions`).

Output is a `CompiledToolkit` (`task_type`, `selected_tools`, `blocked_tools`).
Deterministic: no run_id, no timestamp, no hash. The Rust port is correct when
`select_tools` / `compile_task_toolkit` reproduce `toolkit_fixtures.json`
byte-for-byte (selected order, reasons, blocked set, missing_permissions).

## Files

- `generate_toolkit_fixtures.py` - drives the live Python `compile_task_toolkit`
  through the scenarios; records the real `CompiledToolkit.to_dict()`.
- `toolkit_fixtures.json` - the generated corpus (do not hand-edit; re-run).

## Regenerate

```bash
python3 docs/plans/harness-rust-port/parity-toolgraph/generate_toolkit_fixtures.py
```

## Handoff to Codex

When you port `toolgraph.py` to Rust (spec step 6), wire a parity test (mirror of
`tests/parity.rs`) that loads `toolkit_fixtures.json` and asserts the compiled
toolkit matches per scenario. The corpus is read-only ground truth from the live
Python reference.
