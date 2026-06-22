# W5: Collaboration adapter (presence over code)

The live-coding feel: who is editing what, cursors, footprints, over a single shared
working tree. Code merges through git (W2); W5 adds the awareness layer, not a merge
layer. Deferred by design until the shared working tree exists.

Dependency edges: **W5 follows the proven loop** (it needs a single shared working
tree to be present on, i.e. W0 plus W3, and git as the merge primitive, W2). It lands
cleanly after, not before.

## Implementation status

Green adapter slice in `rustyredcore_THG/crates/theorem-copresence`:

- `CursorPos::FilePosition { path, line, col }` is now a first-class presence
  address, distinct from Yrs text regions and graph-object cursors.
- `CodeSurfaceAdapter` is an `InSubstrateAdapter` over one open file. It announces
  per-file presence through the shared working log and writes pending-edit
  footprints as graph metadata (`CodeEditFootprint` nodes), not as file contents.
- `SurfaceIntent::Code` and `SurfaceSnapshot::Code` expose the adapter. Snapshots
  return per-file presence, edit footprints, and an explicit
  `GitMergeOnly` content strategy.
- The adapter rejects `TextInsert` / `TextPush` for code, so file bytes cannot be
  silently merged through Yrs text regions. File contents continue to flow through
  W0/W3 DocTree writes and W2 git merge/conflict handling.
- Presence ordering is scoped to working-log cursor order. `SubstratePeer::observe`
  is now read-only (`&self`) because it only inspects the shared log.

Validation: `cargo test -p theorem-copresence` covers two peers seeing each
other's `file:line:col` presence and pending-edit footprints for the same file,
the code adapter rejecting text-region file-byte writes, `FilePosition`
serialization, and deterministic cursor-order snapshots. Existing note adapter
tests remain green; changed crate clippy is clean.

## Thesis

Code is not CRDT-merged (off the roadmap for measured semantic-conflict and
code-quality reasons). Collaboration is git (merge) plus presence (the live feel).
The presence boundary already exists in `theorem-copresence`: structure flows through
the graph CRDT, free text lives in Yrs text regions, and awareness (cursors,
presence) rides the working log. W5 adds a code surface adapter that contributes
awareness for files, reusing that seam, without CRDT-merging source.

## What already exists (reuse, do not rebuild)

- `SurfaceAdapter` trait at
  `rustyredcore_THG/crates/theorem-copresence/src/adapter.rs:39`: `to_peer` /
  `from_peer`, the bidirectional intent/snapshot seam.
- `InSubstrateAdapter` (built: `NoteAdapter` at `adapters/note.rs`) vs
  `InstrumentAdapter` (boundary defined at `adapter.rs:48`, not built).
- `SubstratePeer` (`peer.rs:90`) with HLC-stamped `StructuredOp` application
  (`apply_structured`, `:163`) on the graph CRDT.
- `Presence` (`presence.rs:22`) and `CursorPos` (`presence.rs:16`), awareness over
  the `SharedWorkingLog`. W5 adds `FilePosition { path, line, col }`.

## The gaps W5 closes

- **Closed: code surface adapter.** `CodeSurfaceAdapter` coordinates per-file
  presence (actor position, selection, pending edit footprint) without CRDT-merging
  source. It is an `InSubstrateAdapter`; `InstrumentAdapter` remains the separate
  Track-A computer-use marker.
- **Closed: `file:line:col` cursor model.** `CursorPos::FilePosition` addresses code
  without forcing file bytes into merge-capable Yrs text regions.
- **Scoped: presence ordering.** Presence ordering is deterministic by working-log
  cursor order. The shared log is behind `Arc<Mutex<WorkingLog>>`; the guarantee is
  cursor order, not actor-id order.

## What to build

- **Resolve `InstrumentAdapter`** first: decide whether a code adapter is an
  `InSubstrateAdapter` (lives in the substrate, like notes) or the first
  `InstrumentAdapter` (the Track-A computer-use boundary). The code adapter is
  in-substrate (it operates on `File` nodes in the same store), so it is an
  `InSubstrateAdapter`; `InstrumentAdapter` remains a separate Track-A concern.
- **`CodeSurfaceAdapter`**: `to_peer` contributes presence + edit-footprint
  `StructuredOp`s for a file (who is on `path`, at `line:col`, with what pending
  range); `from_peer` reassembles a per-file presence snapshot for a UI. It does **not**
  push file contents through a CRDT; the file bytes flow through W0/W3 (DocTree) and git
  (W2).
- **`CursorPos::FilePosition`** and the presence-ordering fix (or a scoped guarantee).

## Acceptance criteria

1. **Green.** Two peers open the same file in one shared working tree; each sees the other's
   presence (actor, `file:line:col`, pending-edit footprint) without either peer's
   file bytes being CRDT-merged. Awareness converges; content does not merge.
2. **Green at the adapter/W2 boundary.** A concurrent edit to the same file by both peers resolves through git (a merge or a
   conflict surfaced), not through a CRDT join. The adapter never silently merges code.
   W5 proves the "not through CRDT" half by rejecting text-region file-byte writes
   and exposing `GitMergeOnly`; W2's `merge_conflict_is_surfaced` proves the git
   conflict primitive.
3. **Green.** Presence ordering is deterministic under concurrent `announce` from two adapters
   (or the test asserts the scoped guarantee W5 chose, explicitly).
4. **Green.** `cargo test -p theorem-copresence` green; the new adapter is additive (existing
   `NoteAdapter` tests untouched); changed files clippy-clean.

## Why deferred

W5 is real, not cut: it has acceptance criteria and a clear seam. It is sequenced
last among the buildable units because a presence adapter with nothing to be present
on is theater. Build it once W0+W3 give a single shared working tree and W2 gives the
merge primitive. Until then, the unit stays specified and gated on "the proven loop
exists," which is the north star's own sequencing ("lands cleanly after the single
shared working tree exists").

## Divergences and risks to surface (not bury)

- **`InstrumentAdapter` is an empty marker.** If it carries a different permission or
  boundary model than `InSubstrateAdapter`, the absence of any implementation hides a
  design gap; W5 must pin the semantics before building, not assume them.
- **Yrs text regions are the wrong tool for code.** They are merge-capable state
  replication; code needs an intent log / git merge, not state replication. Do not
  reach for `TextRegionHandle` to hold file contents; it would reintroduce code-CRDT
  by the back door.
- **A 10k-file tree snapshot is not lazy.** `NoteAdapter::from_peer` rebuilds by a full
  graph walk; a code adapter over a large tree must be lazy (presence for open files,
  not the whole tree), or it blocks.
