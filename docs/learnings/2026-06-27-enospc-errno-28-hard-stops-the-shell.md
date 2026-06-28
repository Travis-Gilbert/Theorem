# A build link failing with `errno 28 (No space left on device)` is a full-disk storage blocker, not a code defect, and the shell goes down with it: with zero free bytes you cannot even `echo` or `rm` to self-rescue

**Kind:** gotcha
**Captured:** 2026-06-27
**Session signature:** `claude-code:travisgilbert (DATAWAVE ingest+edge intake; Codex built the reconstruction half)`
**Domain tags:** environment, disk-full, errno-28, build, shell-blocker

## Trigger

After adding a small `EdgeCondition` boolean-composition variant and its test, `cargo test -p rustyred-thg-datawave` failed at the link step: `ld: write() failed, errno=28 (No space left on device)`, `clang: error: linker command failed with exit code 1`, `error: could not compile rustyred-thg-datawave (test "parity")`. My first instinct was to suspect my edge.rs change -- but the lib had compiled; only the test *binary* failed, and the cause was the linker having nowhere to write its output bytes: the disk was 100% full. The trap deepened when I tried to free space: `rm -rf target/debug/incremental` and even `echo probe` both returned `ENOSPC: no space left on device, open '/private/tmp/claude-501/.../tasks/<id>.output'`. The Bash tool writes each command's stdout/stderr to a per-call output file on the tmp volume; with zero free bytes that `open()` fails before the command runs, so NO shell command can launch -- including the `rm` that would free space. I could not self-rescue. Work resumed only after the user freed ~10GB externally; `df -h /` then showed 9.9Gi available and the relink (a few hundred MB) succeeded immediately, all tests green.

## Rule

When a build or link fails with `errno 28` / "No space left on device" (often surfaced as `ld: write() failed` or `clang: linker command failed`), classify it as a storage blocker, not a code defect: do not edit code, do not retry the build. Know that the agent shell itself is down -- the Bash harness needs to write its output file, so a full disk means even `echo`/`rm` return `ENOSPC` and you cannot free space from inside. Escalate to free disk: prefer external, re-downloadable caches (`~/Library/Caches`, `~/.cargo/registry/{src,cache}`) over any project `target/`, and never delete another active agent's `target/` in a shared repo. Once the shell returns, confirm headroom with `df -h /` before re-running. A failing-link `errno 28` is "the SSD is full", purely storage; CPU/compile were fine, the bytes just had nowhere to land.

## Evidence

- `cargo test` link error: `ld: write() failed, errno=28 (No space left on device)` -> `could not compile (test "parity")`; lib compiled, only the test binary link failed.
- `echo probe` failed: `ENOSPC ... open '.../tasks/<id>.output'` -- the harness output file could not be created, so the command never ran.
- After ~10GB freed: `df -h /` showed `9.9Gi` avail; the same `cargo test` passed (33 unit + 2 parity + 1 doc-test; harness 2), no code change.

## Encoded in

- `docs/learnings/2026-06-27-enospc-errno-28-hard-stops-the-shell.md` (this file)
