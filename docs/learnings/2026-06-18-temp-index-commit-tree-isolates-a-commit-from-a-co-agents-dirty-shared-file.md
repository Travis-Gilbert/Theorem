# Committing a one-line `mod` addition into a file a co-agent has +454 uncommitted lines in: use a temp-index commit-tree against HEAD, not `git add <file>` (which sweeps the co-agent's WIP into your commit)

**Kind:** method
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (obsidian navigable vault + edges-over-memory)`
**Domain tags:** git, co-edit, codex-coordination, shared-dirty-file, temp-index, commit-tree, isolation, no-text-crdt

## Trigger

I needed to land a new module `rustyred-thg-memory/src/similarity.rs` plus its one-line
wiring (`pub mod similarity;` + a 5-name re-export) in `lib.rs`. But `lib.rs` had Codex's
+454 uncommitted lines (storage-spine / cold-tier WIP). A plain
`git add rustyredcore_THG/crates/rustyred-thg-memory/src/lib.rs` would have staged ALL of
Codex's in-flight work and committed it as mine — the exact hazard the repo's "commit only
with an explicit pathspec" rule exists for, except a pathspec does NOT help when MY change
and the co-agent's change live in the SAME file (source has no text-CRDT; same-file edits
race). `git add -p` is unavailable (interactive flags are blocked in this environment).

My first two hand-built single-hunk patches failed `git apply --check`: (1) a no-trailing-
context hunk `@@ -20,2 +20,8 @@` would not anchor; (2) the trailing blank context line and
exact line counts were fragile. It only applied once I used full context AND
`--recount --ignore-whitespace`.

## Rule

To commit your isolated change into a file a co-agent has dirty, build a commit against HEAD
with a temp index, never touching the working tree:

```
export GIT_INDEX_FILE=/tmp/iso.idx; rm -f "$GIT_INDEX_FILE"
git read-tree HEAD                                   # seed temp index from HEAD (co-agent-free)
git apply --cached --recount --ignore-whitespace -p1 mychange.patch   # your single hunk vs HEAD
git add -- path/to/new_file_a path/to/new_file_b     # new files are entirely yours; safe
TREE=$(git write-tree); NEW=$(git commit-tree "$TREE" -p HEAD -F msg.txt)
# GUARD before advancing: assert only your files + small added-line count
git diff --name-only HEAD "$NEW"                      # must be exactly your N files
git diff HEAD "$NEW" -- the_shared_file | grep -cE '^\+'   # must be ~your lines, NOT the co-agent's count
git update-ref refs/heads/main "$NEW"                # only if guard passes
unset GIT_INDEX_FILE
```

Two preconditions and one verification make this safe:
- **Precondition:** every symbol your isolated change depends on must already exist in HEAD
  (`git show HEAD:file | grep ...`). If the co-agent ADDED a helper you call, you cannot
  isolate against HEAD — fall back to leaving it uncommitted.
- **Precondition:** your edit must sit in its own diff hunk (`git diff HEAD | grep '^@@'` —
  confirm the co-agent's hunks are elsewhere; a pure offset shift is fine).
- **Verification:** prove the isolated commit compiles by stashing ONLY the co-agent's file
  (`git stash push -- <shared_file>`; the working tree then equals your commit), `cargo check`
  from the workspace dir capturing the SUBSHELL exit (not a pipe's), then `git stash pop`.

## Evidence

- Commit `bd6f3cab` landed `similarity.rs` + test + a 7-line `lib.rs` diff while Codex's +454
  stayed uncommitted in the working tree; guard confirmed 3 files / +7 lines (not +454).
- HEAD already had `memory_nodes`/`memory_edge_id`/`prop_str`/`normalized_tenant_pair` +
  `MEMORY_DOCUMENT_LABEL` (verified via `git show HEAD:...lib.rs | grep`), so the module
  compiled against HEAD.
- Stash-check: `git stash push -- .../lib.rs` then `( cd rustyredcore_THG && cargo check -p
  rustyred-thg-memory )` exit 0, then `git stash pop` restored Codex's `M lib.rs`.
- Pitfall logged: `( cargo check | tail )` captures `tail`'s exit, not cargo's; and running
  cargo from the repo root fails ("could not find Cargo.toml") because the workspace is
  `rustyredcore_THG/`. Capture `$?` right after a `( cd ... && cargo ... )` subshell.
- The prior session's same-problem solution (obsidian-sync, commit `14e889eb`) used the same
  temp-index technique but at FILE granularity; this one extends it to HUNK granularity within
  a shared file via `git apply --cached`.
