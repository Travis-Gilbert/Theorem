# Detect an active Rust build with `pgrep -x`, never `pgrep -f cargo`

**Kind:** anti_pattern
**Captured:** 2026-06-15
**Domain tags:** shell, cargo, tooling, multi-agent

## Trigger

Before `rm -rf rustyredcore_THG/target` (a ~32G reclaim), I guarded against
corrupting a live build with `pgrep -fl 'rustc|cargo'`. It matched ~20 processes
and reported a build active, so it SKIPPED the removal. None were builds: every
match was an MCP server (`npm exec firebase-tools`, `context7`, `morphmcp`,
`perplexity`, ...) whose command line contained `/Users/.../.cargo/bin` in `PATH`.
`pgrep -f` matches the WHOLE command line, so `.cargo` sitting in `PATH` is a
false hit. (Bonus scar: `-fl` also dumped those processes' full env, leaking MCP
API keys in cleartext into the transcript.)

## Rule

To test "is a Rust build running," match the exact process NAME:
`pgrep -x rustc` (the compiler) and `pgrep -x cargo`. Never `pgrep -f cargo` /
`pgrep -fl 'cargo'` — every process with `~/.cargo/bin` in `PATH` (all MCP servers
on this machine) false-positives, and `-l`/`-f` can leak secret-bearing env. The
same trap applies to any tool whose binaries live under a dotdir that appears in
`PATH`.

## Evidence

- `pgrep -fl 'rustc|cargo'` -> 20+ `npm exec ... mcp` matches (PATH contained `/Users/travisgilbert/.cargo/bin`); the guard wrongly skipped the removal.
- `pgrep -x rustc` -> 0 and `pgrep -x cargo` -> 0; removal then proceeded safely, reclaiming ~32G with no build running.
- Depends-on: `docs/learnings/2026-06-14-enospc-masquerades-as-build-failure.md` — its rule "never clean a target/ another agent is building" is only safe if the build is detected correctly.

## Encoded in

- `docs/learnings/2026-06-15-pgrep-f-cargo-false-positives-use-pgrep-x.md` (this file)
