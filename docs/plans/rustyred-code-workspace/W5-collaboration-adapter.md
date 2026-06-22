# W5: Collaboration adapter (presence over code)

The live-coding feel: who is editing what, cursors, footprints, over a single shared
working tree. Code merges through git (W2); W5 adds the awareness layer, not a merge
layer. Deferred by design until the shared working tree exists.

Dependency edges: **W5 follows the proven loop** (it needs a single shared working
tree to be present on, i.e. W0 plus W3, and git as the merge primitive, W2). It lands
cleanly after, not before.

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
- `Presence` (`presence.rs:22`) and `CursorPos` (`presence.rs:16`, variants
  `TextIndex { region_id, index }` and `Object { object_id }`), awareness over the
  `SharedWorkingLog`.

## The gaps W5 closes

- **No code surface adapter.** A `CodeSurfaceAdapter` must coordinate per-file
  presence (actor position, selection, pending edit footprint) without CRDT-merging
  source. It is a new `InSubstrateAdapter` (or whatever `InstrumentAdapter` resolves
  to; that marker trait is unbuilt and its semantics are undefined, which W5 must
  resolve first).
- **No `file:line:col` cursor model.** `CursorPos` addresses a text region or a graph
  object, not a file position. Code presence needs a `file:line:col` address. W5 adds
  a `CursorPos` variant (e.g. `FilePosition { path, line, col }`) rather than forcing
  code into the text-region model (Yrs text regions are merge-capable, which is exactly
  what code must not be).
- **Presence ordering races.** `SharedWorkingLog` is not thread-safe for ordering;
  concurrent `announce` calls have undefined presence ordering. A multi-actor code
  session needs deterministic presence ordering; W5 must address this or scope to the
  ordering the working log can guarantee.

## What to build

- **Resolve `InstrumentAdapter`** first: decide whether a code adapter is an
  `InSubstrateAdapter` (lives in the substrate, like notes) or the first
  `InstrumentAdapter` (the Track-A computer-use boundary). The code adapter is
  in-substrate (it operates on `File` nodes in the same store), so it is an
  `InSubstrateAdapter`; close the `InstrumentAdapter` marker as a separate concern.
- **`CodeSurfaceAdapter`**: `to_peer` contributes presence + edit-footprint
  `StructuredOp`s for a file (who is on `path`, at `line:col`, with what pending
  range); `from_peer` reassembles a per-file presence snapshot for a UI. It does **not**
  push file contents through a CRDT; the file bytes flow through W0/W3 (DocTree) and git
  (W2).
- **`CursorPos::FilePosition`** and the presence-ordering fix (or a scoped guarantee).

## Acceptance criteria

1. Two peers open the same file in one shared working tree; each sees the other's
   presence (actor, `file:line:col`, pending-edit footprint) without either peer's
   file bytes being CRDT-merged. Awareness converges; content does not merge.
2. A concurrent edit to the same file by both peers resolves through git (a merge or a
   conflict surfaced), not through a CRDT join. The adapter never silently merges code.
3. Presence ordering is deterministic under concurrent `announce` from two adapters
   (or the test asserts the scoped guarantee W5 chose, explicitly).
4. `cargo test -p theorem-copresence` green; the new adapter is additive (existing
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
