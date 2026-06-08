# A Claude Code hook's wired event must equal the hookEventName it emits, or additionalContext is dropped

**Kind:** rule
**Captured:** 2026-06-08
**Session signature:** `claude:1travisgilbert@Theorem:cc-plugin-resync`
**Domain tags:** claude-code, hooks, plugins, theorems-harness, hookSpecificOutput

## Trigger (the scar)

`theorems-harness` 0.4.4 shipped `scripts/posttool-blocker-scan.sh` — a script
whose header says "PostToolUse hook," which reads `.tool_response` /
`.last_assistant_message` and hardcodes
`hookSpecificOutput.hookEventName: "PostToolUse"` in its output — but it was wired
as a second hook group under the **`UserPromptSubmit`** event in BOTH
`hooks/hooks.json` and `hooks/codex-hooks.json`. Consequences:

- On a UserPromptSubmit event there is no `.tool_response` to scan.
- CC silently **drops** the hook's `additionalContext` because the emitted
  `hookEventName` ("PostToolUse") does not match the firing event
  ("UserPromptSubmit"). The official reference (code.claude.com/docs/en/hooks) is
  explicit: `hookEventName` must match the firing event.

Net: the entire "blocker scanning" deliverable was a silent no-op, and it never
ran after tool calls where it was designed to fire. The two other new
context-injecting scripts were correct and are the contrast that proves the rule:
`inject-harness-directives.sh` sets `hookEventName` *dynamically* from
`.hook_event_name` (so it is valid under both SessionStart and UserPromptSubmit),
and `checklist-contract.sh` hardcodes `"UserPromptSubmit"` and is wired there.

## Rule

For any CC hook that injects context via `hookSpecificOutput.additionalContext`,
the event array it is wired under MUST equal the `hookEventName` it emits — CC
discards the context on mismatch with no error. When reviewing a plugin's
`hooks.json`, cross-check each context-injecting script's emitted/hardcoded
`hookEventName` against the event key it sits under. Fix a mismatch by moving the
WIRING to the matching event, not by rewriting the script's identity: the script's
design (which payload fields it reads, its header, its hardcoded event) tells you
the correct event. (Validated as harmless in the same review: `shell`,
`statusMessage`, `timeout`, and the `FileChanged` event are all recognized fields/events.)

## Evidence

- `jq '.hooks | to_entries[] | select(.value|tostring|test("posttool-blocker-scan")) | .key'`
  returned `UserPromptSubmit` before the fix, `PostToolUse` after.
- Fixed in `claude-marketplace` commit `4029594c`: moved the hook out of a
  `UserPromptSubmit` group into the existing `PostToolUse` group in both host
  wirings (`hooks.json` + `codex-hooks.json`), `+13 / -21`.
- Confirmed against the official hooks reference that the mismatch suppresses
  `additionalContext`.

## Encoded in

- `docs/learnings/2026-06-08-cc-hook-eventname-must-match-firing-event.md` (this file)
