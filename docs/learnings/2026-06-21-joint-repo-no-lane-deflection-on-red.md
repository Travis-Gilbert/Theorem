# In a jointly-owned repo, flagging a fixable red check as "pre-existing / not my lane" is buck-passing, not discipline; the red is the team's and fixing it is in scope

**Kind:** anti_pattern
**Captured:** 2026-06-21
**Session signature:** `claude-code:travisgilbert (SPEC-7 organizer-engine activation with Codex)`
**Domain tags:** coordination, multi-agent, ownership, clippy, reporting, lanes

## Trigger

Verifying SPEC-7 Section B and wiring runtime activation into `rustyred-thg-server`, I ran `cargo clippy -p rustyred-thg-server` and it errored on a deny-level `clippy::never_loop`. The error was in `cypher/parse.rs:703` (a `for inner in pair.into_inner() { return ... }` that only ever touches the first element), not in my diff. I reported it as "Pre-existing (not ours, predates both commits at 501ece6) ... worth a separate cleanup pass" and "Flagged to Codex" -- and marked the session done with the crate's clippy still red.

Travis pushed back, exactly right: "I don't understand why it's so heavy on lanes. It's one project with joint ownership, the red is in fact ours. Can you fix this." The actual fix was one line (`for` -> `if let Some(inner) = pair.into_inner().next()`, behavior-identical), clippy went to exit 0, 192 server tests stayed green. The "lane" framing -- correct earlier for avoiding a clobber of a file Codex was *actively editing* -- had bled into an excuse to leave a trivially-fixable, in-the-same-crate red check unfixed and hand it off in a status message.

## Rule

A failing check (clippy/test/build) you can fix within the scope you are already in IS yours to fix in a jointly-owned repo. "Pre-existing", "predates my commit", or "another head's lane" is not a reason to leave a red green-able in minutes; it is at most a note in the commit body. Only defer a red when the fix is genuinely large, ambiguous, or would change behavior you cannot verify -- and then say so concretely, not as a lane reflex. Lane discipline exists to avoid clobbering a peer's *live* edits (read-before-write, don't overwrite uncommitted work); it does NOT license shipping a repo redder than you found it. Before reporting "done": is any check this session's diff touches still red? If you can green it, green it.

## Evidence

- The red: `cargo clippy -p rustyred-thg-server` deny `clippy::never_loop` at `crates/rustyred-thg-server/src/cypher/parse.rs:703`.
- The fix: commit `77ef3f9` (one line, `for` -> `if let Some(...).next()`); clippy exit 0 after; `cargo test -p rustyred-thg-server` 192 passed / 0 failed (cypher parse behavior preserved).
- The deflection it replaced: a status line "Pre-existing (not ours) ... Flagged to Codex" while leaving clippy red.

## Encoded in

- `docs/learnings/2026-06-21-joint-repo-no-lane-deflection-on-red.md` (this file)
