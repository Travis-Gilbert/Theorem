# Loop status: RustyRed as Code Workspace

Durable state for the `/theorems-harness:execute` self-paced build loop over this
plan tree, so the next session (or Codex) resumes cold. Coordination room:
`rustyred-code-workspace` (tenant `Travis-Gilbert`). Two heads: `claude-code` and
`codex`.

## Lane split (current, after Codex's sprint)

| Head | Lanes | Files |
|---|---|---|
| `codex` | W0 (import + seam), W2 (incl. `GixWorkspaceRepo`), W3 (materialize/run/sync); heading to W1, W2-remotes, W3-sidecar, W4 | `apps/rustyred-workspace`, `apps/rustyred-embedded/src/lib.rs`, `apps/rustyred-git` |
| `claude-code` | W5 (code presence), then W6 (FUSE) | `rustyredcore_THG/crates/theorem-copresence`, later `apps/rustyred-fuse` |

History: both heads first grabbed W0; Codex claimed it first, `claude-code` yielded
and built W2 (CLI backend). Codex then sprinted: freed the disk (~2.8 to 42 GB by
clearing Cargo artifacts), imported `claude-code`'s W2 CLI backend and added a
`GixWorkspaceRepo`, and landed W3, all green by its report. To avoid re-colliding,
`claude-code` moved to the lanes Codex is farthest from and that live in separate
crates: W5 then W6. The coordination room (`rustyred-code-workspace`) is timing out
on writes, so this file is the durable record (git as the fallback channel).

The `claude-code` W0 importer at `apps/rustyred-workspace` and W2 at
`apps/rustyred-git` on branch `claude/nice-newton-56f8dd` are **review references
only** now (Codex owns the canonical versions); do not commit them (same-path
collision at merge).

## Built this loop (claude-code lane)

| Unit | Slice | State | Tests |
|---|---|---|---|
| W0 | per-file `fs_write` importer (artifact filter, source-only) | green, review-reference only (Codex owns canonical) | 2 |
| W2 | local git VCS: init/commit/read-back/branch/divergent heads; three-way merge (clean+conflict+abort); push-to-bare + clone | green, review-reference only (Codex extended with gix) | 5 |
| W5 | `CursorPos::FilePosition` + `CodeSurfaceAdapter`: file:line:col presence converges across peers; code-CRDT refused (text-insert intent errors; bytes flow through git) | green, clippy-clean | 1 |
| W5.2 | structural footprint converges peer-to-peer via the graph CRDT (`apply_structured` -> `delta_since`/`merge_delta`) while content never CRDT-merges | green, clippy-clean | 1 |
| W5.3 | presence-ordering determinism: sequential announces get monotonic cursors, latest wins (acceptance #3) | green, clippy-clean | 1 |
| W6 | read-only DocTree FUSE mount: pure translation (DirView/Inodes/FileSource) + fuser glue; **macFUSE optional** behind the `mount` feature (default build needs no macFUSE) | green; live mount `#[ignore]` (kext) | 4 |

`theorem-copresence` W5: additive only (new `CursorPos::FilePosition` variant, new
`SurfaceSnapshot::Code` variant, new `adapters/code.rs`, +2 tests in
`tests/code_presence.rs`). 2 new + 2 existing copresence tests green, clippy-clean.
Genuinely non-colliding (Codex's footprint is `apps/*`, this is `crates/theorem-copresence`),
so it is safe to commit independently. Uncommitted (commit only on explicit request).

## Disk: freed (Codex cleared Cargo artifacts, ~2.8 -> 42 GB)

The earlier 100%-disk block is resolved. The CLI-backend W2 decision still stands as
shipped; Codex added the `gix` backend on top of it.

## Shipping (claude-code lane complete, macFUSE-optional)

W5 and W6 are done and committed. W6 ships **without** macFUSE: the kext-dependent
`fuser` mount glue is behind the `mount` feature, so the default build needs no
system dependency. The remaining units are Codex's.

| Unit | Status | Gate |
|---|---|---|
| W5 code presence (convergence + boundary + structure + ordering) | MET (4 tests, clippy-clean), committed | none |
| W6 read-only DocTree FUSE mount | MET (macFUSE-optional, 4 pure tests, clippy-clean), committed | live mount only: needs the macFUSE kext loaded (macOS 26.3 / Apple Silicon environment, not a code gate) |
| W0, W1, W2(+gix), W2-remotes, W3, W3-sidecar, W4 | Codex's lanes | Codex is sprinting them |

claude-code built green review references for W0 (`apps/rustyred-workspace`) and W2
(`apps/rustyred-git`); Codex owns the canonical versions of both. Do not commit those
two (same-path collision). W5 (`theorem-copresence`) is the one claude-code lane safe
to land independently.

## Resume

1. To do W6: install macFUSE, then `/loop` resumes with a new `apps/rustyred-fuse`
   crate (read-only DocTree mount first).
2. Otherwise the productive claude-code moves are: commit W5; or review Codex's
   `GixWorkspaceRepo` (built on the claude-code CLI backend) for CLI-semantics parity
   once Codex commits it; or pick up a Codex lane by explicit hand-off.
3. Coordination room `rustyred-code-workspace` has been timing out on writes; this
   file is the durable record (git is the fallback channel).
