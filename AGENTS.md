# AGENTS.md, Theorem

This file briefs coding agents working in this repository. CLAUDE.md carries the full project context, architecture, build, test, and layout; read it. This file carries the conventions that apply to every session regardless of the task, and they take precedence when a task tempts you to skip them.

## Start of every session

- Run `git pull` first. Commits made through the GitHub MCP land on the remote, not on this local checkout, until you pull.
- This is a Cargo workspace; cargo operates from the repo root, so start here. Let the spec point you at the specific crates and files for the task rather than discovering them by wandering.

## The harness (Theorems-Harness V2)

This project has a persistent memory and coordination substrate. Use it reflexively, not on request.

- The tenant slug is `Travis-Gilbert`, capitalized and hyphenated. Not lowercase, not default.
- Before answering an architecture question or a "did we decide X" question from your own training data, recall from the harness. Prior decisions, conventions, and the reasons behind them live there.
- When you make or are handed a load-bearing decision, a constraint, a convention, a thing ruled out, encode it to the harness so the next head and the next session inherit it, instead of fixing it by hand later.
- Coordinate with other heads by footprint, what you are doing and which files your hands are on, not by dividing files into rigid lanes. Heads do their best work on the same task with tight sync, not separated into worktrees that produce duplicate work.

## The grounding contract

This is the most important convention here, because it is the one most often skipped.

Agents are strong at translation and verification and weak at reconstruction. You default to training data, not the web or this codebase, unless you are given the source. A task where the answer or a checkable proxy sits in context succeeds. A task that asks you to reconstruct precise external knowledge from memory, a published architecture, a library's real API, a spec or wire format, fails in the details, and it fails silently because the output looks plausible.

So, for any task that depends on precise external knowledge:

- Read the named authoritative source before writing code. If the spec names a reference repo or file, read it at the pinned commit and bind to what is actually there. Do not reconstruct it from memory.
- If a spec names a tool, path, or signature, that is a requirement by position. If you disagree, surface the disagreement; do not silently substitute something else.
- Completion is defined by an oracle, not by the code looking right: a test that passes, a numerical parity check, a reference output matched, a conformance check. "Looks right" is not done.
- For a library port, parity-test module by module against the pinned reference, and load real reference weights and inputs rather than synthetic ones. Watch framework differences, for example the Burn Linear weight is laid out as [in, out] while PyTorch is [out, in], so transpose on load.

## Review and correctness

Do not look for problems by reading. Stand up the oracles and fix what they flag. For Rust that means miri for undefined behavior in unsafe code, proptest for invariants and round-trips, criterion for benchmarks so "slow" becomes a number, ThreadSanitizer with a stress test for data races, and a soak test watching resident memory for unbounded growth. For a database especially, correctness under concurrency and unbounded memory growth bite harder than inefficiency and are nearly invisible to eye review.

## Scope discipline

Implement what the spec says, fully. Do not insert conservative defaults that contradict the spec, do not downgrade to an MVP that was not asked for, and do not frame in-scope work as deferred. A named choice in the spec is a requirement, not a suggestion. If something genuinely cannot be done, say so plainly and name the blocker, rather than quietly shrinking the work.

## Doc-update protocol (end of every session)

Code outruns docs. If your session added, renamed, or removed a crate or app, before you end the session: update the crate or app table in `CLAUDE.md` and the matching row in `docs/site/reference/`, fix any `CLAUDE.md` section the change makes wrong, bump the README `Last sync` line if you re-synced with Theseus, then run `scripts/check-doc-drift.sh --refresh`. Encode the decision to the harness if it is load-bearing.

Detection backs the rule. `scripts/check-doc-drift.sh` compares crates and apps on disk against the `CLAUDE.md` map and a baseline. A `SessionStart` hook injects the current doc-map status into every session. A `Stop` hook flags new undocumented directories; export `THEOREM_DOC_DRIFT_BLOCK=1` to make it block until they are documented. Full guide: `docs/site/guides/doc-update-protocol.md`.
