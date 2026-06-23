# zsh does not word-split an unquoted scalar, and `git ... -- <pathspec>` treats everything after `--` as a path: two ways a pathspec'd multi-file commit fails

**Kind:** gotcha
**Captured:** 2026-06-15
**Session signature:** `claude:travisgilbert (servo-automation-core / playwright-class)`
**Domain tags:** git, zsh, shell, pathspec, multi-agent-commit

## Trigger

Committing a 13-file browser bundle with an explicit pathspec (to avoid sweeping
in Codex's other dirty files from the shared index), I first wrote
`PATHS="path1 path2 ..."` then `git add -- $PATHS`. The login shell here is zsh,
and zsh does NOT word-split an unquoted scalar parameter the way bash does, so the
entire string was passed as a single pathspec:
`fatal: pathspec 'path1 path2 ... ' did not match any files`. I switched to a zsh
array `P=(path1 path2 ...)` + `git add -- "${P[@]}"` (correct), but the very next
command `git commit -- "${P[@]}" -m "subject" -m "body"` failed with
`error: pathspec '-m' did not match any file(s) known to git` -- because in
`git commit -- <pathspec>`, EVERYTHING after `--` is interpreted as a path, so
`-m` and both messages were read as filenames. The fix was flag order:
`git commit -m "subject" -m "body" -- "${P[@]}"`. (The `git add` had already
staged the files, so re-running the corrected commit needed no re-add.)

## Rule

In zsh, never count on word-splitting an unquoted `$VAR`; build a real array and
expand `"${ARR[@]}"`. In ANY `git <verb> -- <pathspec>`, put every flag (`-m`,
`-F`, `--amend`, etc.) BEFORE the `--`; everything after `--` is a pathspec. For a
pathspec'd commit that shares the index with another agent: `git add -- "${P[@]}"`
then `git commit -m ... -- "${P[@]}"` is the safe shape (explicit paths on both,
flags before `--`).

## Evidence

- `git add -- $PATHS` (unquoted scalar) -> one giant pathspec, `did not match any files`.
- `git commit -- "${P[@]}" -m "..."` -> `error: pathspec '-m' did not match any file(s)`.
- `git commit -m "..." -m "..." -- "${P[@]}"` -> committed `1fd442a4`, 13 files, 3038 insertions, exactly the intended set.

## Encoded in

- `docs/learnings/2026-06-15-zsh-no-word-split-and-git-doubledash-flag-order.md` (this file)
