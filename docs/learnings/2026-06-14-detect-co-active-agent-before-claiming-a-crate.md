# The session-start git snapshot is stale; detect a co-active agent before editing a crate

**Kind:** anti_pattern
**Captured:** 2026-06-14
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** git, multi-agent, coordination, codex

## Trigger

Handed a frontier handoff for `rustyredcore_THG/crates/rustyred-web`, the session-start
`git status` block showed that crate clean, so the plan was to write all 7 frontier
files myself. A LIVE `git status` run a few minutes later showed `Cargo.toml` + `lib.rs`
modified and `src/frontier/` untracked; `ls -la` showed Codex had already created
`model.rs` and `queue.rs`, and `queue.rs`'s mtime was 80 seconds before the check.
Codex was writing the exact same files in real time. Trusting the (already stale)
session-start snapshot and writing into `src/frontier/` would have collided head-on on
every file.

## Rule

In this repo Codex is frequently co-active. The git status captured at session start is
a point-in-time snapshot and goes stale within the session. Before editing or claiming
any crate, run a LIVE `git status --porcelain -- <crate>` and `ls -lt <dir>`, and
compare the newest file mtime to the wall clock (`date`). A file touched seconds ago
means another agent is mid-write - do not edit those files. Claim a non-colliding seam
(new files, a different directory, or verification) and declare it via
`coordination_intent` before proceeding. Verification (`cargo build`/`test`) is
read-only and safe even while another agent holds the source.

## Evidence

- Session-start snapshot: `rustyred-web` absent from the dirty set. Live check minutes
  later: `M rustyred-web/Cargo.toml`, `M rustyred-web/src/lib.rs`, `?? rustyred-web/src/frontier/`.
- `ls -la src/frontier/`: `model.rs` (14:55), `queue.rs` (14:56) vs `date` 14:57:20 -> an 80s-old write.
- The two files became seven (`mod.rs` 766 lines, etc.) over the next ~15 minutes while
  I stayed on a verification seam; zero collisions resulted.

## Encoded in

- `docs/learnings/2026-06-14-detect-co-active-agent-before-claiming-a-crate.md` (this file)
