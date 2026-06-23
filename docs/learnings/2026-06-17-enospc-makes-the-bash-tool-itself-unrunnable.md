# A 0-byte disk makes the Bash tool itself unrunnable (ENOSPC on its own output file, before your command runs); recover with ONE `rm -rf rustyredcore_THG/target` and expect to retry once

**Kind:** gotcha
**Captured:** 2026-06-17
**Session signature:** `claude-code:travisgilbert (cuts 4+5 reconcile-with-codex / verifier)`
**Domain tags:** disk, enospc, cargo, bash-harness, recovery, this-machine

## Trigger

To verify nothing downstream broke before pushing, I ran `cargo build --workspace`. It
filled the APFS volume to zero free bytes. After that, EVERY Bash command failed with
`ENOSPC: no space left on device, open '/private/tmp/claude-501/.../tasks/<id>.output'` —
the harness could not allocate the tool's own output file, so even `df`, `du`, and `rm`
could not run. The shell was locked out, not just the build.

`rm -rf .../target/debug/incremental` (first attempt) still ENOSPC'd. A second attempt,
`rm -rf rustyredcore_THG/target`, succeeded — the OS had purged just enough headroom
between attempts — and reclaimed ~32G (df then showed 14G free).

## Rule

On this machine, do NOT run `cargo build --workspace` / `cargo test` broadly when disk is
tight; it can drive free space to 0 and lock out the Bash tool entirely (the failure is at
the harness output-file layer, so no diagnostic command works). To recover from a 0-byte
ENOSPC: issue ONE `rm -rf rustyredcore_THG/target` (build artifacts only, fully
regenerable, ~32G) and retry once if the first `rm` also ENOSPCs — macOS purges a little
between attempts. Crucially, source files are untouched, so any green test run you did
BEFORE the disk filled still holds: you do not need to rebuild before committing/pushing.
Prefer per-crate `cargo test -p <crate>` over `--workspace` to bound artifact growth.

## Evidence

- `cargo build --workspace` -> ENOSPC; subsequent `df`/`du`/`rm` all ENOSPC'd on
  `/private/tmp/claude-501/.../tasks/*.output`.
- `rm -rf rustyredcore_THG/target/debug/incremental` failed; `rm -rf rustyredcore_THG/target`
  succeeded on the 2nd shot; `df -h /` then reported 14G free.
- Recurs with memory `sessions-using-main-worktree-accumulation` (hogs: target ~32G,
  .git/objects ~43G). Committed + pushed 21501c67 fine afterward on the freed space.
