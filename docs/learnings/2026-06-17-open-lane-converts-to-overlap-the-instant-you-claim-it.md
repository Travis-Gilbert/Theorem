# An "open lane" (clean tree, no live intent) can become an overlap lane the instant you announce it; the "file modified since read" error on your FIRST edit is the collision signal — re-check git status and pivot to verifier, don't retry the edit

**Kind:** anti_pattern
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (cuts 4+5 reconcile-with-codex / verifier)`
**Domain tags:** coordination, multi-head, codex, no-text-crdt, verifier-lane, race

## Trigger

Asked to "reconcile with codex and implement plans 4 and 5." I did it by the book: read
the room (only my own stale HippoRAG intent) and `git status` (working tree clean of `.rs`
changes), concluded cuts 4+5 were an OPEN lane, posted a `coordination_intent` claiming it,
wrote a plan, and started implementing. My very FIRST `Edit` to
`rustyred-thg-memory/src/lib.rs` failed: "File has been modified since read."

That error was the collision. A fresh `git status` showed Codex had jumped into every file
I'd claimed — `ensemble/*`, `rustyred-thg-memory/src/lib.rs`, and even
`theorem-harness-runtime/src/memory.rs` which had been clean minutes earlier. Codex was
dispatched on the same task and sprints git-only (does not read the room). Had I re-read and
retried the edit, I'd have clobbered ~700 lines of its uncommitted work on a shared git tree
with no text-CRDT (last writer wins).

## Rule

Reading the room + a clean `git status` makes a lane look open, but that is PROVISIONAL —
a peer head can enter the moment you announce, and it will not announce back. On the first
"file modified since read" (or "file already exists" on a Write you expected to create),
STOP and treat it as a concurrency signal, not a stale-cache nuisance: re-run `git status`,
and if a peer now owns the file, pivot to verifier (author NEW-file acceptance/integration
tests under `tests/`, flag findings via `coordination_record` + @mention, do not edit the
peer's `.rs`). The collision signal is the failed write itself, because the peer never pings.
This is the open-lane variant of "codex-already-building-pivot-to-verifier-not-co-write": the
overlap can appear mid-turn even after a clean-tree start.

## Evidence

- First `Edit` of `rustyred-thg-memory/src/lib.rs` returned "File has been modified since
  read"; `git status` then showed ensemble + memory + harness-runtime/memory.rs all dirty
  (Codex), where harness-runtime/memory.rs had NOT been dirty ~5 min prior.
- Pivot delivered real value with zero source clobber: 2 new acceptance suites
  (`ensemble/tests/cut4_acceptance.rs`, `rustyred-thg-memory/tests/cut5_acceptance.rs`), a
  cross-crate parity guard, and one fix on top, all in commit 21501c67.
- Complements `docs/learnings/2026-06-17-codex-already-building-pivot-to-verifier-not-co-write.md`.
