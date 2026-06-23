# W6: FUSE endgame

The upgrade that collapses the materialize copy. The `DocTree` is mounted as a real
POSIX filesystem, so the toolchain sees RustyRed's files directly and there is one
copy: edits-via-RustyRed and edits-via-toolchain land in the same place, with no sync
step and no drift surface. The heaviest unit, deferred by design until the
materialize loop (W0+W3) is proven.

Dependency edges: **W6 follows a proven materialize loop** (it replaces W0/W3's
materialize-and-sync-back with a mount). It is the layer-3 endgame; the north star's
recommended arc is materialize-to-run first, FUSE as the upgrade once the loop works.

## Thesis

Materialize-to-run (W0+W3) has a sync step and a drift surface: the materialized dir
and the `DocTree` are two copies that must be reconciled. FUSE removes both by making
the `DocTree` the filesystem the toolchain reads and writes. Execution still runs as a
real process against the mount, so the sandbox (W3's `SandboxRuntime`) does not go
away; only the copy does.

## Ship decision: no macFUSE required

For this PR, W6 ships without requiring macFUSE/fuse3 on the developer or CI
host. The merge target is the platform-neutral `DocTreeMount` behavior, the
default pure-Rust `rustyred-fuse` proof crate, and the `fuse-host` adapter's
compile/test coverage through its backend seam. A real kernel mount and mounted
execution parity remain optional live smokes for a host that deliberately opts
into a platform FUSE driver; they are not merge blockers for the no-macFUSE
ship.

## What to build (scoped; full spec when W0+W3 are proven)

- **Current green slice (W6.1): mount core over the W0 seam.**
  `apps/rustyred-workspace::DocTreeMount` is a platform-neutral adapter that a
  macFUSE/fuse3 host can call. It maps `getattr`, `readdir`, `read`, and
  source-like `write/create`/`unlink`/`rename` onto `Engine::list_paths`,
  `fs_read`, `fs_write`, `fs_unlink`, and `fs_rename` under a mounted prefix.
  Tests prove single-copy behavior in both directions: mount writes are
  immediately visible through `fs_read`, direct `Engine::fs_write` is
  immediately visible through the mount core, mount source writes fire the same
  post-write hooks as W0, and mount unlink/rename update the DocTree plus `File`
  graph metadata through RedCore's durable `NodeDelete` path.
- **Current green slice (W6.2): feature-gated FUSE host adapter.**
  `apps/rustyred-workspace::fuse_host` is available behind `--features
  fuse-host`. It uses `fuser` 0.17 to provide a read/write filesystem host shell
  over a `DocTreeFuseBackend: Send + Sync` contract: inode/path translation,
  `lookup`/`getattr`/`readdir`, `read`, offset-aware `write`, `create`, `unlink`,
  and `rename`, plus conservative single-threaded mount options. On macOS the
  optional dependency is compiled with `macos-no-mount`, so this host adapter can
  compile on machines without macFUSE while still keeping the live mount truth
  explicit.
- **Current green slice (W6.3): actor-backed embedded engine backend.**
  `fuse_host::EngineFuseBackend` opens the real `rustyred-embedded::Engine` on a
  dedicated actor thread, runs optional setup hooks there, and services the
  thread-safe FUSE backend contract through `DocTreeMount`. Tests prove
  read/write/getattr/readdir/rename/unlink round-trip through the actor and that
  setup-installed File-write hooks fire on FUSE writes. This keeps the current
  single-threaded `SharedStore<Rc<RefCell<_>>>` architecture intact while making
  the platform host usable with the real engine.
- **Current green slice (W6.4): directory POSIX subset.**
  Synthetic DocTree directories now support prefix rename through the mount core
  and actor backend: renaming `src` to `crates/app/src` moves every source file
  under that prefix through `Engine::fs_rename`, so destination writes still fire
  W1 hooks and source nodes are durably removed. The FUSE host forwards `rmdir`
  and maps source-directory outcomes to POSIX errors (`ENOENT`, `ENOTDIR`,
  `ENOTEMPTY`) while artifact-directory removals stay in the throwaway lane.
  Cached inode paths are renamed recursively so known child inodes follow a
  directory move.
- **Current green slice (W6.5): persistent empty directories.**
  `rustyred-embedded::Engine` now has explicit durable directory markers
  (`fs_mkdir`, `fs_rmdir`, `fs_is_dir`, `list_directories`) that rehydrate from
  the DocTree snapshot without creating zero-byte `File` graph nodes. The mount
  core exposes `mkdir`, reports explicit empty directories through
  `getattr`/`readdir`, materializes them as real OS directories for W3, and
  forwards them through `EngineFuseBackend` plus the `fuser` host `mkdir`/`rmdir`
  callbacks.
- **Current green slice (W6.6): configurable POSIX attrs and inode cleanup.**
  `fuse_host::DocTreeFuseHost` now takes a `FuseAttrPolicy`, defaulting to the
  current process uid/gid with source-friendly `0644` file and `0755` directory
  modes, while tests can inject stable ownership and permission values. Cached
  inode paths are also forgotten recursively, so dropping a known directory evicts
  its children instead of leaving stale descendant path mappings behind.
- **Current green slice (W6.7): host-side throwaway disk tree.**
  The `fuser` host now turns mount-core `Throwaway` receipts into real, private
  temp-backed filesystem entries. Artifact paths can be created, written, read,
  listed, renamed, unlinked, and removed through the host boundary without entering
  DocTree or firing source-index hooks. This is the missing local behavior that
  makes `target/`/`node_modules`-style output usable by a mounted toolchain.
- **Current green slice (W6.8): source/artifact rename crossings.**
  The FUSE host now treats renames across the source/artifact boundary as byte
  transfers between the temp tree and the DocTree backend: artifact-to-source
  writes through the backend so W1 hooks can fire, while source-to-artifact writes
  into throwaway disk and unlinks the DocTree source entry. This prevents hidden or
  skipped temp files from becoming invisible source files and prevents source files
  moved into `target/` from leaving stale DocTree nodes behind.
- **Current green slice (W6.9): inode forget/write-buffer cleanup.**
  `DocTreeFuseHost` now implements `fuser::Filesystem::forget` and ties inode
  eviction to buffered-write cleanup. Forgetting or removing a directory evicts
  cached descendant inode paths and their write buffers, while siblings keep their
  inodes and buffers. This narrows the remaining inode-lifetime work to behavior
  that needs a real kernel FUSE mount to tune.
- **Current green slice (W6.10): directory-tree source/artifact crossings.**
  Source/artifact rename crossings now recurse for whole directory trees, not only
  files. A generated directory under `target/` can be renamed into `src/` and have
  every file written through the backend, while a source directory moved into a
  skipped path is copied to throwaway storage and recursively unlinked/rmdir-ed from
  the backend.
- **A FUSE host over the `DocTree`.** A read/write FUSE filesystem (macFUSE on macOS,
  fuse3 on Linux; the named platform dependency) that maps `getattr`/`readdir`/`read`
  onto `DocTree` reads (`fs_ls`/`fs_read`/`list_paths` via the W0 seam) and
  `write`/`create`/`unlink`/`rename` onto `DocTree` writes (the W0 batch path), firing
  the W1 on-write maintenance on writes to source files.
- **The same source/artifact boundary.** Mounted source is indexed and read-write;
  build output (`target/`, `node_modules/`) is routed to throwaway disk and is not
  indexed (the boundary W1/W3 already enforce, now at the mount layer). The mount must
  not index a compiler's intermediate output, or builds become unusable. W6.1 proves
  the routing decision: artifact-like writes, unlinks, and renames return
  `Throwaway`, do not enter `DocTree`, and do not fire W1 hooks; the future FUSE
  host owns the throwaway-disk backing store. Green for W6.7 at the host boundary:
  artifact paths are backed by a private temp tree and merged into FUSE
  read/getattr/readdir while remaining outside DocTree.
- **Execution against the mount.** W3's bridge runs the toolchain against the mount
  path instead of a materialized copy; the `SandboxRuntime` trait is unchanged, only
  the working directory is the mount.

## Acceptance criteria

The no-macFUSE PR requires the platform-neutral and feature-gated host criteria
below to stay green. Criteria that require a live kernel mount are endgame live
smokes, not prerequisites for this merge.

1. A FUSE mount over a `DocTree` lets the real `cargo`/`python`/`node` toolchain read
   and write files that are simultaneously live `File` graph nodes; an edit through the
   mount fires the W1 on-write overlay without any explicit sync step.
2. There is exactly one copy: a file written through the mount is immediately readable
   via `fs_read` and vice versa, with no materialize/sync-back round trip.
3. Build artifacts written, unlinked, or renamed in the mount's throwaway region are
   never indexed and never become `File` nodes. Green for W6.1 at the mount-core
   boundary.
4. The `fuser` host adapter compiles behind an opt-in feature and exposes the
   expected read/write/unlink/rename operations through a `Send + Sync` backend
   seam. Green for W6.2 without requiring a platform mount.
5. The `Send + Sync` backend seam is wired to the real embedded engine through a
   single-threaded actor, including setup-installed write hooks. Green for W6.3
   without mutating the direct `Engine` handle into a shared concurrent object.
6. Directory rename and directory-removal outcomes are represented at the mount
   boundary and actor boundary: prefix directory rename moves source files without
   leaving stale old paths, and `rmdir` distinguishes missing, file-not-directory,
   non-empty source directories, and throwaway artifact directories. Green for
   W6.4 without requiring a platform mount.
7. Explicit empty directories persist across embedded engine restart, do not
   appear as `File` graph nodes, materialize as real OS directories, and round-trip
   through the actor-backed FUSE backend. Green for W6.5 without requiring a live
   platform mount.
8. FUSE host attrs expose configurable file/dir permissions plus process/default
   ownership, and inode cleanup evicts cached descendants for forgotten
   directories. Green for W6.6 without requiring a live platform mount.
9. Artifact files and directories in skipped paths are backed by host-side
   throwaway disk and support read/list/rename/unlink/rmdir without DocTree
   persistence. Green for W6.7 without requiring a live platform mount.
10. Renames crossing between skipped artifact paths and source paths move bytes
   across the FUSE host boundary without trapping source-like paths in throwaway
   storage or leaving stale DocTree entries. Green for W6.8 without requiring a
   live platform mount.
11. Inode forget/removal also evicts descendant buffered writes instead of leaving
   stale memory behind. Green for W6.9 without requiring a live platform mount.
12. Directory-tree renames crossing the source/artifact boundary recursively move
   files and directories between backend storage and throwaway disk without stale
   source entries. Green for W6.10 without requiring a live platform mount.
13. Execution against the mount produces the same `ProofReceipt` as execution against a
   materialized copy (W3 parity), proving the mount is a drop-in for the materialize
   step. Live-platform follow-up; not required for the no-macFUSE PR.

## Still open

- Optional live platform FUSE mount: the `fuser` host adapter now compiles, but
  this PR intentionally does not require macFUSE/fuse3. A real mounted path has
  not been exercised on this host and remains an opt-in live smoke.
- Direct shared-engine FUSE handle: `EngineFuseBackend` wires the real engine via
  a single-threaded actor. A direct shared `DocTreeMount -> Engine` backend is not
  pursued for now because `fuser::Filesystem` requires `Send + Sync` while the
  embedded `Engine` remains intentionally single-threaded through
  `Rc<RefCell<_>>` in `SharedStore`.
- Full POSIX host semantics: file-level `unlink`/`rename`, directory prefix rename,
  rmdir outcome mapping, recursive inode-path rename, persistent empty
  directories, configurable permissions/uid/gid attrs, recursive inode forget, and
  host-side throwaway disk backing including source/artifact rename crossings are
  green. W6.9 also cleans write buffers on inode forget/removal. Still open:
  platform-specific error/performance tuning and live-host inode lifetime behavior
  in a real macFUSE/fuse3 mount. W6.10 extends the source/artifact crossing behavior
  to whole directory trees before that live-host pass.
- Execution parity against a mounted path: W3 parity remains an optional live
  smoke until a platform mount exists.

## Why the live mount remains deferred

W6 was deferred until W0+W3 were demonstrable. That condition is now true enough
to ship the no-macFUSE slice: a platform-neutral mount core, feature-gated host
adapter, actor-backed real-engine backend, artifact routing, directory behavior,
and source/artifact crossings. The remaining live-platform pass is still heavy:
kernel mount behavior, platform-specific POSIX/performance tuning, and mounted
execution parity. Those checks add confidence to the endgame, but the default
developer/CI path now stays free of macFUSE.

## Divergences and risks to surface (not bury)

- **Platform dependency.** FUSE is macFUSE (macOS, kernel-extension friction) or fuse3
  (Linux); there is no portable single answer, and macOS in particular has tightened
  kernel-extension loading. The materialize path (W0+W3) has no such dependency, which
  is exactly why it is the near-term shape and FUSE is the upgrade. The current
  ship path keeps the platform driver optional.
- **Performance.** A FUSE filesystem backed by a graph store and content-addressed
  object store has very different latency from a native filesystem; a `cargo build`
  doing thousands of stats and reads will expose it. The throwaway-disk routing for
  artifacts is not optional; it is what keeps builds usable.
- **The sandbox does not go away.** "Move the sandbox inside RustyRed" still means real
  processes against the mount, governed by W3's `SandboxRuntime`. FUSE removes the copy,
  not the execution sandbox; do not conflate the two.
- **Consistency under concurrent writes.** The mount and direct `fs_write` are two
  write paths into one `DocTree` behind a `RefCell`; W6 must define the concurrency
  model (single-writer per scope, or a lock) so a toolchain write and an engine write
  do not race the `RefCell` borrow. The first `fuser` shell is intentionally
  single-threaded and backend-trait-based until the engine handle itself is safe to
  share with a FUSE worker.
