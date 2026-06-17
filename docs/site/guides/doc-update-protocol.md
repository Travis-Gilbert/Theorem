# Doc-update protocol for agents

Code outruns docs. This protocol keeps the navigation map from drifting out from under the code, by making a missing crate or app a signal an agent sees, not a thing someone notices weeks later.

## The order of truth

Read in this order, and trust in this order:

1. `CLAUDE.md` at the repository root: the navigation map. Crate table, app table, build commands, conventions, status.
2. The actual code.
3. The plan docs under `docs/plans/`. Plans lag code; treat them as intent, not state.

The map is first because it is the cheapest correct orientation. It is also the layer most likely to drift, because nothing in the compiler forces it to update when a crate is born. This protocol is that forcing function.

## What counts as drift

A crate exists in `rustyredcore_THG/crates/` but is not in the `CLAUDE.md` crate table. An app exists in `apps/` but is not in the app table. A crate or module was renamed or removed and the map still names the old one. The README `Last sync` line is stale after a re-sync with Theseus.

## The end-of-session checklist

If your session added, renamed, or removed a crate or app, before you end:

1. Add or fix the row in the `CLAUDE.md` crate or app table, and the matching row in `docs/site/reference/`.
2. If the change is architectural (new entrypoint, removed subsystem, renamed module), update the relevant `CLAUDE.md` section, not only the table.
3. If you re-synced with Theseus, bump the README `Last sync` line.
4. Run `scripts/check-doc-drift.sh --refresh` to move the baseline forward.
5. Regenerate the reference tables: `scripts/gen-crate-readmes.sh` and `scripts/gen-crate-reference.sh`.
6. Encode the decision to the harness if it is load-bearing, so the next head inherits it.

This is the same discipline `CLAUDE.md` already states ("update this file before ending the session"). The protocol adds detection so the instruction is not the only thing standing between the code and the map.

## The hook

`scripts/check-doc-drift.sh` compares crates and apps on disk against the `CLAUDE.md` map and against a baseline snapshot in `.harness/`.

- `scripts/check-doc-drift.sh` (default, `--new-only`) reports directories that appeared since the baseline and are not in the map.
- `scripts/check-doc-drift.sh --full` reports the whole standing backlog: everything on disk not yet in the map.
- `scripts/check-doc-drift.sh --refresh` rewrites the baseline to the current disk state. Run it after you update the map.

Two Claude Code hooks wire this in, in `.claude/settings.local.json`:

- A `SessionStart` hook (`scripts/hooks/doc-drift-sessionstart.sh`) injects the current doc-map status into the session as context, so every session starts knowing the backlog and the rule.
- A `Stop` hook (`scripts/hooks/doc-drift-stop.sh`) checks for new undocumented directories. It is advisory by default (it prints to the terminal). Export `THEOREM_DOC_DRIFT_BLOCK=1` to make it block the stop until the new crate or app is documented.

The split is deliberate. The standing backlog is large and is being cleared in rounds; blocking on it would block every session. So the backlog is surfaced as advisory context at session start, and only growth (a new directory after the baseline) can block, and only when you opt in.

## Why a hook and not just a convention

A convention is a sentence. A hook is a sentence with a tripwire. The grounding contract in `AGENTS.md` is explicit that the most important conventions are the ones most often skipped. Detection is how a convention survives a fast week.
