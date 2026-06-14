# "Codex committed" can mean no commit exists on any ref

**Kind:** gotcha
**Captured:** 2026-06-14
**Session signature:** `cc-session-84baa4a7-e608-46a4-9a5c-748b0572c3b2`
**Domain tags:** git, coordination, multi-agent

## Trigger

The user signaled "codex committed and is updating the plugin," which I took as
"the CRDT source is committed and stable, go verify." I checked
`git log --all --remotes`, `git fetch origin`, and the reflog on BOTH repos:
there was NO CRDT commit anywhere — local or remote. Every line of Codex's
implementation plus my new tests were still uncommitted in the working trees.
"Committed" referred to a *different* repo (the theorems-harness plugin) and/or
the commit-tree/temp-index isolation pattern this workspace uses, which records a
commit object without updating a findable ref and without cleaning the working
tree.

## Rule

Before building on a "X committed" claim, verify against the working tree plus
`git log --all --remotes` + reflog. On these repos, commits are sometimes made
via `git commit-tree`/temp-index isolation (to avoid clobbering a peer's dirty
tree), which produces no branch/ref you can find and leaves the files dirty. The
working tree, not the commit graph, is the source of truth here.

## Evidence

- `git log --all --remotes --oneline | grep -iE "crdt|hlc|sync"` → empty on both repos after `git fetch origin`.
- `git status` showed the full CRDT implementation as `M`/`??` (uncommitted) on `feat/crdt-substrate`.
- Prior memory: obsidian-sync was "pushed via temp-index commit-tree to isolate from Codex's dirty tree."

## Encoded in

- `docs/learnings/2026-06-14-committed-can-mean-no-findable-commit.md` (this file)
