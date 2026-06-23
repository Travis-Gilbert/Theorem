# Codex coordinates via the git working tree, not the coordination room

**Kind:** gotcha
**Captured:** 2026-06-14
**Session signature:** `cc-session-84baa4a7-e608-46a4-9a5c-748b0572c3b2`
**Domain tags:** coordination, harness, multi-agent, git

## Trigger

Asked to implement SPEC-RUSTYRED-CRDT "with codex," I joined `room:crdt-substrate`
and wrote a coordination_intent, a decision record (the wire-type contract,
`record_a05847db14d39d04`), and an `@codex` mention. The room reported Codex with
zero intents, zero drained mentions, and no presence — so I assumed the lane was
mine to claim. But `git status` across both repos already showed Codex's entire
7-part implementation in flight (crdt/clock.rs + merge.rs, graph_sync.rs, memory
Part 4, overlap.rs), uncommitted. My `@codex` mention was never read; we then
collided on yjs_sync.rs (an Edit raced Codex's write). The room was empty while
git held everything.

## Rule

When coordinating with Codex on a shared task, read the git working tree
(`git status` + `git diff` across every involved repo) as the PRIMARY signal of
what's in progress. Treat room writes (intent/decision/mention) as best-effort
and route real handoffs through the user. An empty coordination room does NOT
mean no work is underway — Codex works git-only and does not drain the room.

## Evidence

- `room:crdt-substrate` read_intents returned only claude-code; mentions(codex) never consumed.
- `git status` showed crdt/, graph_sync.rs, rustyred-thg-memory, overlap.rs all `M`/`??` and uncommitted.
- Edit on `crates/rustyred-server/src/yjs_sync.rs` failed "modified since read" mid-session (Codex's concurrent write).

## Encoded in

- `docs/learnings/2026-06-14-codex-coordinates-via-git-not-the-room.md` (this file)
