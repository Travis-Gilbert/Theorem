# "N sessions using main" is git worktree accumulation, not a session lock

**Kind:** gotcha
**Captured:** 2026-06-15
**Domain tags:** git, worktrees, tooling, multi-agent

## Trigger

A Claude Code session reported it could not "work from main" because "30 open
sessions are using main." There is no app-level session lock behind that message.
`git worktree list` showed 16 registered worktrees: the dispatch/receiver system
and co-active agents had spawned them, mostly under `/private/tmp/theorem-*`, and
one (`/private/tmp/theorem-railway-grpc-fix-1781388377`) held `main`. When `/tmp`
was cleared, those working dirs vanished but `.git/worktrees/<name>` bookkeeping
still claimed the branches, so the primary checkout refused `git switch main` with
`fatal: 'main' is already used by worktree at /private/tmp/theorem-...`.

## Rule

When a session "can't work from main" / reports "N sessions using main," run
`git worktree list` FIRST. If branches (especially `main`) are held by worktrees
whose dirs are gone (git marks them `prunable`), run `git worktree prune -v`
(preview with `--dry-run -v`). It removes only dead bookkeeping; every branch and
commit survives, and existing worktrees are left untouched. The `/private/tmp/theorem-*`
worktrees are disposable agent spawns and accumulate forever unless pruned.

## Evidence

- `git switch main` -> `fatal: 'main' is already used by worktree at /private/tmp/theorem-railway-grpc-fix-1781388377` (that dir did not exist).
- `git worktree prune --dry-run -v` listed 13 worktrees with "gitdir file points to non-existent location"; the prune removed them and released `main` (error changed to an ordinary uncommitted-changes guard).
- 3 real worktrees (dirs still present) were correctly left untouched.

## Encoded in

- `docs/learnings/2026-06-15-sessions-using-main-is-worktree-accumulation.md` (this file)
