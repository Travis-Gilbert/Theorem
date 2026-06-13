# Verify the active, tracked checkout before grounding a spec against it

**Kind:** rule
**Captured:** 2026-06-12
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** index-api, git, grounding

## Trigger

Grounding the slice-4 spec against Index-API surfaced two stale-state traps:

1. There are TWO Index-API checkouts. `~/Tech Dev Local/Index-API` is empty;
   `~/Tech Dev Local/Creative/Website/Index-API` (branch main, HEAD 30ec019a) is
   the real one. Grounding code anchors against the empty clone would have
   invented file:line references that look authoritative but point at nothing.
2. The entire `docs/plans/skill-encoder/` tree (README, slice-2/3, backref-index,
   etc.) was UNTRACKED in git, despite the README stating the plan tree was
   "Shipped this session." So "it's already committed" was false. Committing the
   slice-4 backref-index pulled ~120 lines of someone else's never-committed
   content into my commit as a side effect.

## Rule

Before grounding a spec against a checkout, or assuming prior work is committed:
confirm `git -C <path> rev-parse --abbrev-ref HEAD` + `git log -1` (is this the
active clone, not an empty sibling with the same repo name?) and
`git status --short -- <paths>` (is the file actually tracked?). A populated
working tree is not the same as a committed one. When a "?? path" file must be
committed, expect it to carry all its prior untracked content, and say so.

## Evidence

- `git -C ~/Tech\ Dev\ Local/Index-API rev-parse HEAD` -> empty; `…/Website/Index-API`
  -> `main`, `30ec019a feat(ingestion): add graph-first corpus ledger`.
- `git status --short -- docs/plans/skill-encoder/` showed `??` for backref-index.md,
  README.md, slice-2/3 while README.md claimed "Plan tree ... Shipped this session".
- Encode tests run only via `.venv/bin/python -m pytest` with
  `DATABASE_URL='' DEBUG=True DISABLE_RQ=True` (root conftest self-runs
  `django.setup()`; pytest.ini sets `-p no:django`).

## Encoded in

- `docs/learnings/2026-06-12-verify-active-tracked-checkout-before-grounding.md` (this file)
