# Execution Handoff: CRDT Substrate, Coordination Room, Civic Atlas Two-View

Three execution specs in one document. Split into three handoffs if you want; Spec 0 is the critical path and the other two depend on it. Rationale for the architecture lives in the prior north-star scope; this document is what an executing head builds against.

First step in any session this spawns: `git pull`. Commits made through the GitHub MCP are on the remote, not on the local filesystem, until you pull.

---

## Grounding contract (applies to all three specs)

This work depends on external protocols and CRDT semantics that are lossy in model memory. Do not reconstruct them. Read the pinned sources and bind to what is there.

Pinned references, to be recorded with commit hashes in a `REFERENCE.md` before coding:

- `crdt-graph` (docs.rs/crdt-graph, repo bkbkb-net/crdt-graph, MIT). A studied reference for the op-based four-set pattern and FlatBuffers serialization. Not a drop-in: it is a 2P2P-Graph, its two-phase sets tombstone permanently and forbid re-add, and it has no attribute registers.
- y-octo (in toeverything/AFFiNE, packages/common/y-octo) and yrs (y-crdt/y-crdt). The two Rust Yjs implementations. Read both before choosing.
- AFFiNE's sync gateway (toeverything/AFFiNE, packages/backend/server/src/core/sync) for the WebSocket Yjs sync protocol, state vectors, and the awareness channel.
- Shapiro et al. 2011, the OR-Set (observed-remove, add-wins) specification, for the graph CRDT semantics crdt-graph does not provide.
- NextGraph docs (docs.nextgraph.org) for the combined graph-CRDT plus document-CRDT pattern, as a worked precedent.

Completion is defined by the acceptance and wiring criteria, observed in a real run, not by the code looking right.

---

## Spec 0: CRDT substrate in RustyRed (critical path)

### Goal

RustyRed can hold shared state that many participants write at once and that converges without conflict, in two forms: a graph of entities and relationships, and Yjs documents for rich content. This is the foundation both the room and Civic Atlas stand on.

### Requirements

- **FR-001**: RustyRed implements an add-wins observed-remove set CRDT for nodes and edges. A removed node or edge identity can be re-added and the state converges, because belief revision retracts then re-asserts.
- **FR-002**: RustyRed implements attribute registers, last-writer-wins and multi-value, so concurrent edits to the same entity attribute converge. The choice of LWW versus MV per attribute class is recorded.
- **FR-003**: The graph CRDT element model carries bi-temporal metadata, valid-time and transaction-time, on every entity and edge, from this first version.
- **FR-004**: RustyRed serves the Yjs update protocol as a sync and persistence backend, so a BlockSuite or TipTap client syncs documents through RustyRed. The implementation binds to y-octo or yrs, chosen after reading both, with the choice and its reason recorded.
- **FR-005**: The op-based pattern, precondition discipline, and serialization follow the crdt-graph reference, adapted from its 2P2P-Graph semantics to add-wins semantics. The deviation from crdt-graph is documented where it occurs.
- **FR-006**: Personalized PageRank runs over the graph CRDT state and accepts a time bound, so retrieval can ask for state as of a transaction time.

### Key entities

- **Entity**: a node in the graph CRDT, carrying id, attributes as registers, valid-time and transaction-time stamps, and optional references to Yjs documents.
- **Relationship**: an edge in the graph CRDT, add-wins, with its own bi-temporal stamps.
- **Document handle**: a reference from an entity to a Yjs document held by the Yjs backend.

### Success Criteria

- **SC-001**: Two replicas concurrently add the same edge and converge to one edge, not a conflict or a duplicate.
- **SC-002**: A node removed on one replica and re-added on another converges to present, demonstrating add-wins over the permanent tombstoning a 2P2P-Graph would impose.
- **SC-003**: Two replicas write different values to the same attribute and converge per the register's declared policy.
- **SC-004**: A PPR query with a transaction-time bound returns the state that held at that time, not the latest.
- **SC-W**: A BlockSuite client and a second client both edit one document synced through RustyRed in the same window; the edits converge with no lost write, and concurrently, an entity referencing that document is removed and re-added on two replicas and survives. Observed in a real run, both the document path and the graph path converging together.

### Out of scope

The wake and routing layer, which is the Gemma resident's scope. Replacing RustyRed's existing storage; this is additive.

### Assumptions

- The existing RustyRed graph and PPR are extended, not rebuilt.
- y-octo or yrs can be embedded or bridged to serve AFFiNE's sync protocol, confirmed by reading the gateway and the chosen library.

---

## Spec 1: Coordination room on the CRDT substrate

Depends on Spec 0.

### Goal

The harness room, today append-only by default rather than by design, becomes a live shared space whose state converges across every participant writing at once, with the full workspace surface people expect.

### Read and bind before building

Read the current room data model in `Travis-Gilbert/Theorem`, under `rustyredcore_THG`, the present message, record, intent, and footprint shapes and the coordination tools that write them. Migrate those onto the graph CRDT; do not assume their shape from this spec.

### Requirements

- **FR-101**: The room's structured state, the migrated messages, records, intents, and footprints, lives in the graph CRDT from Spec 0, written concurrently by every head and by the human.
- **FR-102**: The editable workspace surface runs on AFFiNE over the Yjs backend from Spec 0. This is where the full Notion-class workspace is wanted.
- **FR-103**: Structured room entities reference AFFiNE Yjs documents for rich content, keeping one source of truth for entities and one for content.
- **FR-104**: PPR retrieval, with hierarchical community summaries and bi-temporal awareness, runs over the room's structured state.
- **FR-105**: The room exposes the awareness channel as a presence and footprint signal, the lightweight "who is on what" broadcast, distinct from the durable graph writes.

### Success Criteria

- **SC-101**: The append-only history migrates to the graph CRDT with no loss, verifiable by reading the same records before and after.
- **SC-102**: A PPR query returns time-correct results when a fact has changed across transaction time.
- **SC-W**: Two heads, for example Claude Code and the Gemma resident, write room state from separate replicas in the same window; the writes converge with no lost write, and a third participant's PPR query returns the merged result. Observed in a real run via the room, not asserted from a unit test of the parts.

### Out of scope

GNN-based message passing between LLM heads, ruled out. The wake logic itself.

---

## Spec 2: Civic Atlas two-view on the CRDT substrate

Depends on Spec 0.

### Goal

Civic Atlas becomes one app with two views of the same civic-object store, a spatial view and an AFFiNE management view, so PorchFest planning stops being hand-rolled widgets on the map.

### Read and bind before building

Read the Civic Atlas backend, `Travis-Gilbert/our-civic-atlas-backend`, for the current civic-object schema, the existing RustyRed instance, geohashing, and the GraphQL layer. Read the frontend, `Travis-Gilbert/our-civic-atlas`, for the existing deck.gl and MapLibre spatial view and the editable-layers usage. Bind the new views to the real schema; do not assume it from this spec.

### The one open decision to settle here

Schema ownership for a civic object: an AFFiNE database row versus a graph-CRDT entity. For a planning tool the AFFiNE-row answer is simpler; for deep epistemic structure the graph-CRDT entity wins. Settle it against the backend schema and the relationships civic objects actually carry, and record it. This is not a blocker and does not reintroduce a two-store problem, because RustyRed is the one store under both answers.

### Requirements

- **FR-201**: Civic objects, the events, tasks, and notes, are the single source of truth both views read, held in RustyRed under the ownership decision above.
- **FR-202**: The spatial view renders civic objects through the existing deck.gl, MapLibre, and editable-layers stack.
- **FR-203**: The management view is AFFiNE, drawing from RustyRed through the Yjs backend from Spec 0, giving table and kanban views of the same civic objects natively. Confirm whether a calendar view exists in AFFiNE if a month view is load-bearing; if not, that view is a separate build.
- **FR-204**: Creating or editing a civic object in either view updates the shared state, so the other view reflects it with no separate sync step.
- **FR-205**: Spatial representation of a task or note is a custom binding of the civic-object entity to a map feature. No turnkey spatial task library is assumed; this binding is the build.

### Success Criteria

- **SC-201**: A civic object edited in the management view reflects in the spatial view, and the reverse, with no manual sync.
- **SC-202**: Two people edit civic objects concurrently and converge.
- **SC-W**: A PorchFest task created in the AFFiNE management view appears as a feature on the map in the same session, and moving it on the map updates its location in the management view, because both are projections of one civic-object record. Observed live.

### Out of scope

Importing AFFiNE as a separate app with its own store; it draws from RustyRed. A bespoke map engine; the deck.gl stack stays.

---

## Traceability

Spec 0 is the foundation. Spec 1 FR-101 and FR-102 consume Spec 0 FR-001 through FR-004. Spec 2 FR-201 and FR-203 consume the same. The bi-temporal element model, Spec 0 FR-003, is what the deferred Discovery auto-search would later read, so it is built now even though that feature is not.
