# Re-read the exact source you're about to edit at session start in a Codex-shared repo — your in-context copy is a stale cache, and a co-agent's commit can change a function signature under you

**Kind:** rule
**Captured:** 2026-06-18
**Session signature:** `claude-code:travisgilbert (harness product docs + generated OpenAPI)`
**Domain tags:** codex-coordination, stale-context, session-boundary, compaction, re-grounding, theorem-harness-server

## Trigger

Returning to `theorem-harness-server` after a context compaction, I had `main.rs` and `lib.rs` in context from the prior session and was about to wire a new route against them. Re-reading `lib.rs` first revealed that `mentions_json` now took a NEW `urgencies: &[String]` parameter (6 args) and returned a `"urgencies"` field — but my in-context `main.rs` still called it with the old 5-arg signature. Codex's commit `03dd7624` ("compound attribution and wake drain") had added `urgency`/`urgencies` to the mentions endpoint and the `CoordinationQuery` struct since my last read. Separately, `git status` showed the prior session's `docs/site` files were already committed (an auto-commit hook had swept them) — so my belief "the docs are uncommitted" was also false. Editing against either stale fact would have produced a wrong OpenAPI mentions spec and/or an Edit whose `old_string` no longer matched.

## Rule

At session start in this repo — and ALWAYS after a compaction — before editing any file:
- `git log --oneline -8` and `git status --porcelain` first. The auto-commit-push hook may have already committed your prior work; the working tree may carry a co-agent's uncommitted changes. Don't trust your memory of "what's committed."
- Re-`Read` the specific file and lines you are about to edit. Treat in-context source as a cache a co-agent's commits silently invalidate — especially function signatures and the call sites that depend on them.
- Confusable-basename trap: `theorem-harness-core/src/lib.rs` (one crate) and `rustyred-thg-core/src/lib.rs` (a different crate) share the basename `lib.rs`. Before a pathspec commit, confirm the FULL path — Codex was dirty in `rustyred-thg-core/lib.rs` while I edited `theorem-harness-core/lib.rs`; only the full path distinguishes "mine" from "theirs."

## Evidence

- `lib.rs:121` `pub fn mentions_json<S>(.., urgencies: &[String], consume: bool, limit: usize)`; `main.rs:62-74` `CoordinationQuery { .., urgency, urgencies, .. }` + `:146 fn urgencies()`. My prior-context `main.rs` call site had 5 args and no `urgency` query field.
- Source: Codex commit `03dd7624`, present in the local history but added after my prior-session read.
- The re-read let the generated OpenAPI mentions endpoint correctly document `urgency`/`urgencies`; skipping it would have shipped a spec that contradicts the code.
- `git status` at session start showed my prior `docs/site/*.md` as committed (tracked, modified) not untracked — confirming the auto-commit hook had already landed them.
