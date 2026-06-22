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

## What to build (scoped; full spec when W0+W3 are proven)

- **A FUSE host over the `DocTree`.** A read/write FUSE filesystem (macFUSE on macOS,
  fuse3 on Linux; the named platform dependency) that maps `getattr`/`readdir`/`read`
  onto `DocTree` reads (`fs_ls`/`fs_read`/`list_paths` via the W0 seam) and
  `write`/`create`/`unlink`/`rename` onto `DocTree` writes (the W0 batch path), firing
  the W1 on-write maintenance on writes to source files.
- **The same source/artifact boundary.** Mounted source is indexed and read-write;
  build output (`target/`, `node_modules/`) is routed to throwaway disk and is not
  indexed (the boundary W1/W3 already enforce, now at the mount layer). The mount must
  not index a compiler's intermediate output, or builds become unusable.
- **Execution against the mount.** W3's bridge runs the toolchain against the mount
  path instead of a materialized copy; the `SandboxRuntime` trait is unchanged, only
  the working directory is the mount.

## Acceptance criteria (to be detailed when unblocked)

1. A FUSE mount over a `DocTree` lets the real `cargo`/`python`/`node` toolchain read
   and write files that are simultaneously live `File` graph nodes; an edit through the
   mount fires the W1 on-write overlay without any explicit sync step.
2. There is exactly one copy: a file written through the mount is immediately readable
   via `fs_read` and vice versa, with no materialize/sync-back round trip.
3. Build artifacts written to the mount's throwaway region are never indexed and never
   become `File` nodes.
4. Execution against the mount produces the same `ProofReceipt` as execution against a
   materialized copy (W3 parity), proving the mount is a drop-in for the materialize
   step.

## Why deferred

W6 is the endgame, not the start. It is the heaviest unit (a real FUSE filesystem with
POSIX semantics, platform-specific, with performance and consistency concerns the
materialize path does not have), and its entire value is removing a copy that only
exists once W0+W3 are built. Building it before the materialize loop is proven would be
speculative scaffolding for a loop that does not yet run. The detailed spec is written
when W0+W3 land and the materialize loop is demonstrable; until then this unit holds
the scope and the dependency, not an implementation.

## Divergences and risks to surface (not bury)

- **Platform dependency.** FUSE is macFUSE (macOS, kernel-extension friction) or fuse3
  (Linux); there is no portable single answer, and macOS in particular has tightened
  kernel-extension loading. The materialize path (W0+W3) has no such dependency, which
  is exactly why it is the near-term shape and FUSE is the upgrade.
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
  do not race the `RefCell` borrow.
