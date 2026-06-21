# `cargo clippy -p X -- -D warnings` escalates EVERY workspace member's lints to errors, so a dependency's pre-existing lints fail your crate's gate; and `cargo ... | tail` reports tail's exit code, not cargo's

**Kind:** gotcha
**Captured:** 2026-06-19
**Session signature:** `claude-code:travisgilbert (verify + land theorem-copresence handoff)`
**Domain tags:** clippy, cargo, workspace, ci-gate, shell, exit-code, pipestatus

## Trigger

Verifying theorem-copresence against the handoff gate `cargo clippy -p theorem-copresence -- -D warnings`. The Bash wrapper reported exit 0, but the captured output ended in `error: could not compile rustyred-thg-core (lib) due to 19 previous errors` -- ALL 19 lints were in the COMMITTED dependency `rustyred-thg-core` (`versioned_graph.rs:1917` `large_enum_variant`, a `GraphMergeStrategy` `derivable_impls`), ZERO in theorem-copresence. clippy runs as `RUSTC_WORKSPACE_WRAPPER`, so `-D warnings` escalates lints in every workspace member it compiles across the dep graph -- a sibling/dependency crate's pre-existing (toolchain-surfaced) lints fail the gate even when YOUR crate is clean. The "exit 0" was a lie: `cargo clippy ... | tail -40` makes `$?` the exit of `tail`, not cargo.

## Rule

To check ONE crate's own clippy cleanliness inside a workspace that is NOT globally `-D warnings`-clean: run `cargo clippy -p X --all-targets` (no `-D warnings`) and grep the output for the crate's own file paths (`crates/X/(src|tests)`). An empty grep = your crate is clean; the dependency's pre-existing lints are another lane's problem, not a blocker for landing yours. Never trust the exit code of `cargo ... | tail|grep|head` -- it is the last pipe stage's; read `${PIPESTATUS[0]}` or drop the pipe. (theorem-copresence's lib was clean; its only 2 OWN lints were `len_zero` in tests/convergence.rs -- `.len() > 0` -> `!...is_empty()` -- fixed in `cf877fc`.)

## Evidence

- `cargo clippy -p theorem-copresence --all-targets -- -D warnings`: 19 errors, every one citing `crates/rustyred-thg-core/src/versioned_graph.rs` or `GraphMergeStrategy`; the `| tail` pipeline reported exit 0.
- Isolation run `cargo clippy -p theorem-copresence --all-targets 2>&1 | grep -E "theorem-copresence/(src|tests)"` returned exactly 2 lines, both `tests/convergence.rs` `len_zero` -> proved the lib + the rest of the crate clean.
- `git status` for `versioned_graph.rs` is clean (committed) -> the lints are pre-existing on main, surfaced by the clippy 1.95 toolchain, not introduced by this crate.
