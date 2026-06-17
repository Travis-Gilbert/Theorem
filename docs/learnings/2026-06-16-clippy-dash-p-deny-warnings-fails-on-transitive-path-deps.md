# `cargo clippy -p <crate> -- -D warnings` fails on lints in TRANSITIVE workspace path-deps, not just your crate: a red clippy run is not proof your lane is dirty

**Kind:** gotcha
**Captured:** 2026-06-16
**Session signature:** `claude-code:travisgilbert (http-sse-transport + github-app handoffs)`
**Domain tags:** clippy, cargo, workspace, path-deps, deny-warnings, false-signal

## Trigger

To gate Lane A I ran `cargo clippy -p rustyred-thg-connectors --all-targets -- -D warnings`. It exited non-zero with `error: could not compile theorem-harness-core (lib) due to 3 previous errors` -- a `clippy::unnecessary_lazy_evaluations` lint in `theorem-harness-core/src/job.rs` around line 233. That crate is a TRANSITIVE path-dep (connectors -> affordances -> theorem-harness-core), is NOT in Lane A's diff, and is unchanged from HEAD (the lint pre-exists on main). The connectors crate itself had zero clippy findings. Reading the failure naively would push me to "fix" another lane's committed code or to believe my lane failed its quality gate.

## Rule

`cargo clippy -p X` runs the clippy driver on X AND every workspace path-dependency it compiles from source, so `-D warnings` turns a pre-existing lint anywhere in that path-dep subtree into a hard failure attributed to the build, not to X. Before treating a `-D warnings` clippy failure as yours, read which crate the `could not compile <crate>` line names. To gate just your crate, run `cargo clippy -p X --all-targets` WITHOUT `-D warnings` and grep for warnings whose file path is under your crate's `src/`; only fix a path-dep lint if your own change introduced it (confirm with `git diff --stat` on that crate). Registry (crates.io) deps are compiled with plain rustc and never trip this; only workspace path-deps do.

## Evidence

- `git diff --stat` on `theorem-harness-core` was empty (untouched by either lane), yet it produced the clippy hard error.
- Re-running scoped (`cargo clippy -p rustyred-thg-connectors --all-targets 2>&1 | grep connectors/src`) returned zero connectors-source warnings: the crate was clean.
