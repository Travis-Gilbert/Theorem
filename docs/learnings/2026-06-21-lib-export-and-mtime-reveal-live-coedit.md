# A symbol exported in lib.rs that your just-read source file does not define means a peer head is editing that file right now; read-before-write turns the collision into a clean split

**Kind:** method
**Captured:** 2026-06-21
**Session signature:** `claude-code:travisgilbert (SPEC-7 organizer-engine with Codex)`
**Domain tags:** coordination, multi-agent, codex, shared-worktree, verify-before-build

## Trigger

Asked to "complete SPEC-7 with Codex", I planned to build the standing-pass organizer engine and was about to write a geo generator (`SpatialRuleStandingGenerator`) into `rustyred-thg-adapters/src/standing_pass.rs`. Before writing I read the file: it already existed (736 lines, untracked) with the whole spine -- Codex had pre-built it. I then read `lib.rs` and its `pub use standing_pass::{...}` block listed `SpatialStandingGenerator` -- a type my full Read of `standing_pass.rs` minutes earlier did NOT contain. That contradiction (an exported symbol absent from the file I had just read in full) could only mean the file had changed between my two reads. `ls -la --time-style` confirmed: `standing_pass.rs` mtime was ~90 seconds old, `lib.rs` ~30s. Codex was adding the geo generator live, in the same working tree, this minute. Had I written my own geo generator I would have produced a duplicate and likely clobbered a mid-keystroke file.

## Rule

Before writing into any file in a shared worktree, read it AND cross-check its module's `lib.rs`/`mod.rs` exports and the file mtime. Two cheap tells that a peer head is mid-edit: (1) an exported name that the source file does not define (export landed, definition incoming, or vice-versa -- the tree is between two of the peer's saves), and (2) a source mtime newer than your session's last edit. When you see them, do NOT write that file; re-read to its current state, take the non-overlapping slice (here: runtime activation in a different crate, `rustyred-thg-server/src/state.rs`, behind a new `THEOREM_STANDING_PASS` flag), and announce the split. The handoff that followed -- Codex committed the engine (`c966acc`), I committed activation (`a870f6c`) -- only worked because the read came before the write. See also `docs/learnings/2026-06-19-write-blocked-on-untracked-file-signals-coedit.md`.

## Evidence

- `rustyred-thg-adapters/src/lib.rs` exported `SpatialStandingGenerator` while a prior full Read of `standing_pass.rs` (the same session) lacked it; mtime 11:59 vs session-now 12:01.
- Clean split outcome: engine `c966acc` (Codex), activation `a870f6c` (claude-code), both green, zero file overlap.

## Encoded in

- `docs/learnings/2026-06-21-lib-export-and-mtime-reveal-live-coedit.md` (this file)
