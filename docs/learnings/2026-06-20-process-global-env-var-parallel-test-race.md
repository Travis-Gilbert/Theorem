# A process-global env var mutated by one test and read by another races under parallel `cargo test`; restore-on-drop does NOT help — the reader must hold the same lock

**Kind:** gotcha
**Captured:** 2026-06-20
**Session signature:** `claude-code:travisgilbert (review + fix multimodal-planner-unify)`
**Domain tags:** cargo-test, parallelism, env-var, flaky-test, ScopedEnv, THEOREM_AGENT_HEADS, rustyred-thg-mcp

## Trigger

`cargo test -p rustyred-thg-mcp` (default, parallel) intermittently failed `native_coordination_tools_round_trip_through_mcp` at `lib.rs:15279`, asserting `binding_active_head_set == ["claude","codex","deepseek"]`. It passed when run alone and passed with `--test-threads=1`. Root cause: a sibling test, `composed_agent_run_round_trips_through_mcp_with_provider_heads`, sets `THEOREM_AGENT_HEADS=mistral,deepseek` via a `ScopedEnv` helper (save + `set_var` + restore on `Drop`, guarded by a shared `ENV_LOCK` mutex). The coordination test derives `binding_active_head_set` from that same process-global env but read it WITHOUT acquiring `ENV_LOCK`. `cargo test` runs tests as parallel threads in ONE process, so the read landed inside the other test's mutation window. `ScopedEnv`'s Drop-restore guarantees cleanup but cannot prevent a concurrent reader from observing the mutated value mid-scope.

## Rule

Any test that READS a process-global mutable (env var, `static mut`, `set_var` target) that another test MUTATES must acquire the SAME lock for its read-sensitive section — guarding only the mutator is insufficient. Here the fix is `let _env = ScopedEnv::new(vec![]);` at the top of the *reading* test: it pins the var to its default AND serializes against the mutator via `ENV_LOCK`. Validate a parallel-only flake by running the FULL suite in parallel several times (not once, not just `--test-threads=1`), because the race window is narrow.

## Evidence

- Reproduced: full parallel suite failed on the coordination test; `cargo test ... native_coordination_tools_round_trip_through_mcp` (alone) and `-- --test-threads=1` (102/102) both passed — the signature of an isolation race, not a logic bug.
- Fix: added `let _env = ScopedEnv::new(vec![]);` (the helper's name list already includes `THEOREM_AGENT_HEADS`, so `vec![]` removes it = default head set) as the first line of the coordination test.
- After fix: 3 consecutive full parallel runs green (102/102, then 103/103 with a new test added).
