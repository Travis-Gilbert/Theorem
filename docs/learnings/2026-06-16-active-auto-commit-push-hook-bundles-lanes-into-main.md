# An active auto-commit/push hook bundles ALL working-tree lanes into `main` and pushes — your edits reach origin/main mid-session without an explicit `git commit`

**Kind:** anti_pattern
**Captured:** 2026-06-16
**Session signature:** `claude:travisgilbert (graph-hook-primitive)`
**Domain tags:** git, workflow, auto-commit-hook, main-branch, scoped-commit

## Trigger

I planned to make a single scoped-pathspec commit of the hook primitive at the
end of the session (the repo norm is "uncommitted; commit only when asked"). When
the user finally said "commit and push," I discovered the plan was already moot:

- `git rev-parse --abbrev-ref HEAD` had moved from `feat/crdt-substrate` (session
  start) to `main`.
- `git log --oneline` showed my `hooks.rs` + handlers ALREADY committed in
  `887027ee` / `3fc91866` ("add/merge ... graph hook surfaces"), **bundled with
  unrelated jobintel + pilot-core changes** from other lanes.
- `git rev-list --left-right --count origin/main...HEAD` was `0 0` — already
  pushed to origin/main.

A PostToolUse-style auto-commit/push hook had been doing bare `git commit` of the
whole working tree (all lanes) to `main` and pushing, throughout the session. The
same hook also reformats/`git add`s files right after an edit (it added an
`#[allow(dead_code)]` and staged files I never staged).

## Rule

In this repo, assume an auto-commit/push hook may commit your edits to `main` and
push them at ANY time, bundled with other lanes' uncommitted work. Do not assume
work stays local until you commit, and do not rely on a "scoped commit at the
end" for isolation — by then the hook may have already pushed a bundled commit.
If isolation matters, branch FIRST (before editing), not last. Check
`git log --oneline` + `git status` + `git rev-list --count origin/main...HEAD` at
the start and periodically, not just at commit time. When you do commit manually,
still use an explicit pathspec (`git add -- <paths>` then `git commit -m "..." -- <paths>`)
so you add only your files to whatever the hook has already staged.

## Evidence

- Session HEAD `0c15f314` (my explicit scoped completion commit) sat directly on
  top of the hook's bundled `ee91c737`/`3fc91866`/`887027ee` on `main`.
- The earlier `#[allow(dead_code)]` on `CodeIndexRuntime.hook_dispatcher` appeared
  without my edit (a linter/fix hook), and `git status` showed many `M `/`A `
  *staged* entries I never staged.
