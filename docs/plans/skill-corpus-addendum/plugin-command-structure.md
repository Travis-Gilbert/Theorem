# Plugin command structure: two commands plus ambient coordination

A short plugin-design pass (distinct from the substrate work-graph). The plugin lives in the
`claude-marketplace` / `codex-marketplace` repos, not in Theorem, so this specifies the change;
the manifest edit lands there.

## Target (from the addendum)
Two commands plus ambient coordination, not three:
- `/harness` -- everything across the session (adaptive: observe, route, plan, coordinate, execute,
  validate, peer-review, encode, report).
- `/planned-execution` -- brainstorm, research, plan-with-checkboxes, execute, checked-boxes.
- Coordinate is **not** a top-level command. It is ambient: auto-triggered when another head is
  present, and folded into `/harness`.

## Current plugin set (reconciled)
The installed `theorems-harness` plugin already has:
- `/harness` -- the primary adaptive command (`skills/theorems-harness/SKILL.md`). Already "use the
  harness for everything," already describes the coordination-as-part-of-the-loop model.
- Utility/compat commands: `/coordinate`, `/peer-review`, `/encode`, `/research`, `/compute_code`,
  `/context-refresh`, and compat `/execute`, `/planning-theorem`, `/theorize`.
- A hook layer that already does ambient work: `SessionStart` / build-shaped `UserPromptSubmit`
  inject the ambition frame; handoff-shaped prompts emit `.harness/checklist.json` and mirror it into
  the coordination substrate; `PostToolUse` records action + coordination events; `UserPromptSubmit`
  injects a code-neighborhood block when a tenant is set.

So the target is **partly reflected**: `/harness` exists; coordination is already substrate-backed
and partly hook-driven. The gaps are (1) no single `/planned-execution` checkbox command -- planning
and execution are split across `/planning-theorem` + `/execute`; and (2) `/coordinate` is still a
top-level command rather than ambient.

## Delta

1. **Add `/planned-execution`** as the second primary command. It chains the existing skills into one
   checkbox-driven flow: brainstorm -> `/research` (fractal/gap discovery) -> plan with stable
   checklist IDs (the `planning-theorem` skill + the existing `.harness/checklist.json` hook) ->
   execute (the `execute` skill) -> check the boxes (the `Stop` hook already blocks completion while
   checklist items are unresolved without evidence). Implementation reuses `planning-theorem` +
   `execute`; `/planned-execution` is the unified front door, so those two become internal stages
   rather than separate user-facing commands.

2. **Make coordination ambient.** Demote `/coordinate` from a top-level command to a hook-triggered
   behavior folded into `/harness`:
   - A `SessionStart` / `UserPromptSubmit` hook checks for peer presence (other active heads in the
     room via `presence` / pending `mentions`) and, when present, injects a coordination frame:
     read the room + drain mentions at turn-start, write a `coordination_intent` footprint before
     edits, close it with a `coordination_reflection` at turn-end. This is the protocol the harness
     SKILL.md already documents; the change makes it fire automatically instead of needing a command.
   - Keep `coordinate` reachable as an internal capability `/harness` routes to (for an explicit
     `@actor` block/fork ping), but remove it from the top-level command list.

3. **Keep `/harness` primary.** The focused utilities (`/peer-review`, `/encode`, `/research`,
   `/compute_code`, `/context-refresh`) remain as optional escape hatches but are not the primary
   surface; `/harness` routes to them adaptively. The two *primary* commands the user reaches for are
   `/harness` and `/planned-execution`.

## Open / follow-up
- The peer-presence auto-trigger needs a cheap, reliable signal (short-TTL `presence` read or a
  pending-`mentions` check) that does not add latency to every prompt; gate it on a tenant being set
  (`THEOREM_TENANT_ID`), matching the existing code-neighborhood hook.
- The manifest edit + hook wiring land in the marketplace repo; this doc is the spec for that change.
- Decide whether `/planning-theorem`, `/theorize`, `/execute` stay as hidden compat aliases or are
  removed once `/planned-execution` + `/harness` cover them.
