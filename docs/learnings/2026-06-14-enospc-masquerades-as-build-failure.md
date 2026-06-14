# ENOSPC masquerades as a build failure on this machine

**Kind:** gotcha
**Captured:** 2026-06-14
**Session signature:** `cc-session-84baa4a7-e608-46a4-9a5c-748b0572c3b2`
**Domain tags:** environment, cargo, disk, tooling

## Trigger

`cargo test -p rustyred-server` exited 1 with no Rust error in the filtered
output. I almost debugged my test code. The next Bash call revealed the truth:
`ENOSPC: no space left on device` writing the tool's own output file. `df` showed
the data volume at 100% (155Mi free, 416Gi used). A backgrounded cargo build then
consumed the remaining space and deadlocked Bash entirely — even creating an empty
output file under `/tmp` ENOSPC'd, so no command (not even `rm`) could run.
Recovery required `TaskStop` on the build (the task tool needs no `/tmp`), after
which the OS reclaimed the killed process's space and `rm target/debug/incremental`
freed ~1G.

## Rule

On this machine, a build/command failure with no compiler error: run `df -h`
FIRST. When `/tmp` is full, Bash output capture itself ENOSPCs before the command
runs, so use `TaskStop` (not Bash) to kill the disk-hogging background build, then
clean regenerable caches (`target/debug/incremental`, `~/.cargo/registry/cache`).
Never clean a `target/` another agent (Codex) is actively building — only stale
duplicate checkouts or your own incremental cache.

## Evidence

- `cargo test` exit 1, no error lines; next Bash: `ENOSPC ... tasks/bygifdoia.output`.
- `df -h /System/Volumes/Data` → `460Gi 416Gi 155Mi 100%`.
- Freed 2.6G by `rm -rf` the stale duplicate `Tech Dev Local/RustyRed-Graph-Database/target` (confirmed non-canonical: no civic_projection.rs); a single rustyred-server rebuild re-consumed it.
- `TaskStop` on the backgrounded `cargo test` task unblocked Bash.

## Encoded in

- `docs/learnings/2026-06-14-enospc-masquerades-as-build-failure.md` (this file)
