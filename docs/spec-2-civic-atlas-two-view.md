# Spec 2: Civic Atlas Two-View on the CRDT Substrate

Depends on Spec 0. Do not start until Spec 0's wiring criteria are green, because this imports the substrate.

First step in any session this spawns: `git pull`.

## Goal

Civic Atlas becomes one app with two views of the same civic-object store, a spatial view and a management view, so PorchFest planning stops being hand-rolled widgets on the map.

## What each piece owns (settled and open)

- Content composition is TipTap, and it works well for the rich text inside a task or note. Keep it. This is not the open question.
- The open piece is the management surface: whether to add AFFiNE's database block for table and kanban views of civic objects, or to build those views on the existing stack. AFFiNE gives table and kanban natively; the cost is its weight. Settle this in execution against how much multi-view you actually want.
- Spatial presentation of a note or task on the 3D map, making it look like it belongs on the terrain, is a separate design track and does not block this data architecture.

## Grounding contract

Read the substrate (Spec 0) and the sources below; bind to the real schema rather than reconstructing it.

- The Civic Atlas backend, `Travis-Gilbert/our-civic-atlas-backend`: the current civic-object schema, the existing RustyRed instance, geohashing, and the GraphQL layer.
- The frontend, `Travis-Gilbert/our-civic-atlas`: the existing deck.gl and MapLibre spatial view, the editable-layers usage, and the existing TipTap integration.

Record commit hashes in `REFERENCE.md`. Completion is the wiring criterion observed live.

## The one open decision to settle here

Schema ownership for a civic object: an AFFiNE database row, or a graph-CRDT entity. For a planning tool the row answer is simpler; for deep epistemic structure the entity wins. Settle it against the backend schema and the relationships civic objects actually carry, and record it. This does not reintroduce a two-store problem, because RustyRed is the one store under either answer.

## Requirements

- **FR-201**: Civic objects, the events, tasks, and notes, are the single source of truth both views read, held in RustyRed under the ownership decision above.
- **FR-202**: The spatial view renders civic objects through the existing deck.gl, MapLibre, and editable-layers stack.
- **FR-203**: The management view presents civic objects, with TipTap composing their rich-text content. If the multi-view decision lands on AFFiNE, table and kanban views draw from RustyRed through the Yjs backend from Spec 0.
- **FR-204**: Creating or editing a civic object in either view updates the shared state, so the other view reflects it with no separate sync step.
- **FR-205**: Spatial representation of a task or note is a custom binding of the civic-object entity to a map feature. No turnkey spatial task library is assumed; this binding is the build.

## Success Criteria

- **SC-201**: A civic object edited in the management view reflects in the spatial view, and the reverse, with no manual sync.
- **SC-202**: Two people edit civic objects concurrently and converge.
- **SC-W**: A PorchFest task created in the management view appears as a feature on the map in the same session, and moving it on the map updates its location in the management view, because both are projections of one civic-object record. Observed live.

## Out of scope

Importing AFFiNE as a separate app with its own store; it draws from RustyRed. A bespoke map engine; the deck.gl stack stays. The spatial-annotation rendering polish, which is its own design track.
