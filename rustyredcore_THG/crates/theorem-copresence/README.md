# theorem-copresence

A headless co-presence peer and surface-adapter seam over the RustyRed substrate: the general interaction model as a peer, so any surface (or agent) co-edits rather than living inside one editor. Structure converges on the graph CRDT and the THG executor command path; free text lives in `yrs` text regions (browser-Yjs wire-compatible); awareness rides the working log.

## Key API

- Peer (`peer.rs`): `SubstratePeer` (`new<E>(executor, PeerConfig)`, `apply_structured(StructuredOp) -> VersionVector`, `merge_delta(StampedBatch) -> JoinReport`, `delta_since(&VersionVector) -> StampedBatch`, the text-region ops, `announce(Presence)`, `observe(since_cursor)`, `graph_snapshot`). `PeerConfig` (`with_data_dir`/`with_working_log`/`with_text_client_id`), `StructuredOp { SetObjectProperty, AddEdge, RemoveEdge }`, `PeerEvent`. The structure CRDT (`join_delta`/`diff_since`) lives in `rustyred-thg-core::crdt`; the peer surfaces it as `merge_delta`/`delta_since`.
- Surface adapter (`adapter.rs`): `SurfaceAdapter` (`to_peer`/`from_peer`), marker traits `InSubstrateAdapter` / `InstrumentAdapter`, `SurfaceIntent`, `SurfaceSnapshot`.
- Note adapter (`adapters/note.rs`): `NoteAdapter`, `NoteIntent`, `NoteSnapshot`.
- Code adapter (`adapters/code.rs`): `CodeSurfaceAdapter`, `CodeIntent`, `CodeSnapshot`, `CodeEditFootprint`, `CodeContentStrategy` (incl. `GitMergeOnly`, so source merges through W2 git, not Yrs), `FileRange`.
- Presence (`presence.rs`): `Presence`, `PresenceKind { Human, Agent }`, `CursorPos { TextIndex, Object, FilePosition }`.
- Text regions (`text_region.rs`): `TextRegionHandle`, `TextRegionUpdate`.

Path dep: `rustyred-thg-core`. Other: `yrs 0.27`, `thiserror`.

Scope caveat: persistence is process-scoped (the per-peer `DocTree` is in-memory, so a text region does not rehydrate across a peer restart). Durable cross-restart is a named follow-up.

## Build and test

```bash
cd rustyredcore_THG && cargo test -p theorem-copresence
```

Tests: `tests/convergence.rs` (two-peer concurrent structured plus same-region text converge identically), `tests/presence.rs`, `tests/code_surface.rs`. No `#[ignore]`.

Part of the `rustyredcore_THG` workspace. See [the workspace README](../../README.md) for the crate map.
