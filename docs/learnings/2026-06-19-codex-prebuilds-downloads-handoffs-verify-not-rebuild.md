# When handed a Downloads handoff to "implement with codex," Codex may have already built the entire crate untracked before you start — check the working tree FIRST and switch to verify-not-rebuild

**Kind:** method
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (verify + land theorem-copresence handoff)`
**Domain tags:** coordination, codex, multi-agent, handoff, scope-control, verify-lane

## Trigger

Given `~/Downloads/handoff-copresence-substrate-peer.md` and asked to "implement this with codex." I grounded into the substrate APIs (crdt, working_log, doc_tree, graph_store) and started to scaffold `rustyredcore_THG/crates/theorem-copresence`. My first `Write` to its `Cargo.toml` failed with "File has not been read yet" -- because Codex had ALREADY built the entire crate: all 10 handoff files were present (untracked `?? crates/theorem-copresence/`) and the workspace member was pre-registered in `rustyredcore_THG/Cargo.toml`. Codex sprints whole crates from Downloads handoffs git-only, frequently before CC's first edit. Rebuilding would have collided on the same new files.

## Rule

On any "implement <Downloads handoff> with codex" task, BEFORE writing a line: `find`/`ls` the target crate dir and `git status` it, and `git diff <workspace>/Cargo.toml` for a pre-registered member. If Codex already built it, switch to the verify lane: read every file -> map each to the handoff's explicit acceptance criteria -> `cargo test` + ISOLATED clippy (see the clippy-workspace learning) -> announce the verdict WITH honest divergences in the coordination room -> land it scoped. Edit only clean/new files. A `Write` failing with "File has not been read yet" on a path you believe is brand-new is the tell that another agent created it first -- stop and `find` the dir.

## Evidence

- `find crates/theorem-copresence -type f` returned all 10 files (peer.rs, text_region.rs, adapter.rs, presence.rs, adapters/{note,mod}.rs, lib.rs, Cargo.toml, tests/{convergence,presence}.rs) while still `?? untracked`.
- `git diff rustyredcore_THG/Cargo.toml` already contained `+    "crates/theorem-copresence",`.
- Verdict: 6/6 handoff acceptance criteria met, 2 tests green, clippy-clean after a 2-line `len_zero` fix; landed as `cf877fc`. Honest divergence surfaced (and recorded in CLAUDE.md + room): text-region persistence is process-scoped (per-peer `DocTree` is in-memory, no cross-restart rehydration). Captured in memory `copresence-verify-codex-prebuild`.
