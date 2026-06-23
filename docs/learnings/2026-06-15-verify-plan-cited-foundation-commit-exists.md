# A plan's "builds on commit <sha>" can be fictional: verify the cited foundation exists in code before scoping work as a thin layer on it

**Kind:** anti_pattern
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (servo-automation-core / playwright-class)`
**Domain tags:** planning, scope, git, servo-browser-use-agent, grounding

## Trigger

The `servo-automation-core-playwright-class.md` plan opened: "Builds on: the
actuation correction (`9ee18e7e`), which already settled coordinate synthesis, the
JS geometry snapshot, EmbedderControl responses." Taken at face value, that framed
the new Playwright-class core as a thin layer on a built actuation foundation. It
was false: `git cat-file -t 9ee18e7e` returned nothing (the object does not
exist), `git log --all --oneline | grep -i actuat` found the only actuation commit
was `2bb78561`, which is DOCS-ONLY (it added `build-step-1-correction-actuation.md`,
not code), and grepping `notify_input_event|evaluate_javascript|getBoundingClientRect|FilePicker`
across all of `rustyred-web` returned zero hits (the `vendor/` dir held only
`d3.min.js`). So the plan's stated foundation did not exist, and Slices 1-2 were a
from-scratch actuation build -- a material doubling of scope that only surfaced
because I verified the cited commit instead of trusting the prose.

## Rule

When a plan (or a handoff, or another agent) says "builds on commit <sha>" or "X
already landed," verify before scoping: `git cat-file -t <sha>` (does the object
exist?) and `git show --stat <sha>` (is it code, or just a docs/plan commit?),
plus a grep for the named primitives in the actual source. In this repo, plan docs
lead and lag code (CLAUDE.md says "plans lag code") and can cite SHAs that were
never written; a fictional foundation silently turns a "thin layer" into a
ground-up build. Treat the cited commit as a claim to falsify, not a given.

## Evidence

- `git cat-file -t 9ee18e7e` -> fatal (object does not exist); the plan cited it as already-landed.
- `2bb78561` = "Add build-step-1 actuation correction ..." is docs-only (added the plan markdown, no code).
- `grep -rE "notify_input_event|evaluate_javascript|getBoundingClientRect" rustyred-web/src` -> 0 hits before this session's work.

## Encoded in

- `docs/learnings/2026-06-15-verify-plan-cited-foundation-commit-exists.md` (this file)
