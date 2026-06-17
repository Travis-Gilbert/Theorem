# A doc-drift hook must split growth from the standing backlog, or it blocks every session

**Kind:** rule
**Captured:** 2026-06-16
**Session signature:** `claude-ai:1travisgilbert@Theorem:doc-refresh-round-1`
**Domain tags:** claude-code, hooks, docs, navigation-map, drift

## Trigger (the scar)

The `CLAUDE.md` crate table documented 15 of 28 crates and the app table missed 4
of 14. The instinct is a hook that blocks at session end if any crate or app on
disk is missing from the map. With a backlog that large, that hook blocks the
FIRST stop of every session, before any work, until the whole backlog is cleared.
It would be ripped out within a day.

## Rule

A doc-drift hook needs two signals, not one:

- Standing backlog (everything on disk not in the map): informational only.
  Surface it at `SessionStart` as injected context so the session starts aware of
  it. Never block on it. It is cleared in deliberate rounds, not mid-session.
- Growth (a directory that appeared since a committed baseline): this is the
  blockable signal. Snapshot the current dirs to `.harness/doc-map-baseline.*`;
  the hook flags only `disk - baseline`. Make the block opt-in
  (`THEOREM_DOC_DRIFT_BLOCK=1`) so the default is advisory.

The baseline is the whole trick. It lets the hook ship while the backlog still
exists, and it converts "document everything now" into "document the new thing
now." When a round clears backlog and updates the map, refresh the baseline.

## Honoring the earlier hook scar

Per `2026-06-08-cc-hook-eventname-must-match-firing-event.md`, each hook emits a
`hookEventName` equal to the event it is wired under. The SessionStart hook emits
`"SessionStart"` and is wired there; the Stop hook uses the Stop `decision:block`
form. A mismatch makes CC silently drop the injected context.

## Evidence

- `scripts/check-doc-drift.sh --full` reports 13 undocumented crates, 4
  undocumented apps as of capture.
- `scripts/check-doc-drift.sh` (`--new-only`) reports 0 after seeding the baseline,
  so the Stop hook does not block on the existing backlog.
- SessionStart hook output validated as JSON with `hookEventName == "SessionStart"`.

## Encoded in

- `docs/learnings/2026-06-16-doc-drift-hook-split-growth-from-backlog.md` (this file)
- `scripts/check-doc-drift.sh`, `scripts/hooks/doc-drift-sessionstart.sh`,
  `scripts/hooks/doc-drift-stop.sh`, `.harness/doc-map-baseline.*`
- `docs/site/guides/doc-update-protocol.md`
