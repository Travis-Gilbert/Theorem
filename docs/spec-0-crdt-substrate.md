# Spec 0: CRDT Substrate in RustyRed (critical path)

This is the foundation the coordination room (Spec 1) and Civic Atlas two-view (Spec 2) both stand on. Land this first. It splits into two workstreams, 0A and 0B, that are independent to build and meet at one integration point.

First step in any session this spawns: `git pull`. Commits made through the GitHub MCP are on the remote, not on this local checkout, until you pull.

## Grounding contract

This work depends on CRDT semantics and the Yjs protocol that are lossy in model memory. Do not reconstruct them. Read the pinned sources and bind to what is there. Record each source's commit hash in a `REFERENCE.md` before coding.

- `crdt-graph` (docs.rs/crdt-graph, repo bkbkb-net/crdt-graph, MIT). Reference for the op-based four-set pattern and FlatBuffers serialization only. Not a drop-in: it is a 2P2P-Graph, its two-phase sets tombstone permanently and forbid re-add, and it has no attribute registers.
- y-octo (toeverything/AFFiNE, packages/common/y-octo) and yrs (y-crdt/y-crdt). The two Rust Yjs implementations. Read both before choosing.
- AFFiNE's sync gateway (toeverything/AFFiNE, packages/backend/server/src/core/sync) for the WebSocket Yjs sync protocol, state vectors, and the awareness channel.
- Shapiro et al. 2011, the OR-Set (observed-remove, add-wins) specification, for the graph CRDT semantics crdt-graph does not provide.
- NextGraph docs (docs.nextgraph.org) for the combined graph-CRDT plus document-CRDT pattern as a worked precedent.

Completion is defined by the acceptance and wiring criteria, observed in a real run, not by the code looking right.

---

## Workstream 0A: Graph CRDT layer

### Goal

RustyRed holds a graph of entities and relationships that many participants write at once and that converges without conflict, with belief revision and time built into the element model.

### Requirements

- **FR-A1**: Add-wins observed-remove set (OR-Set) for nodes and edges. A removed identity can be re-added and the state converges, because belief revision retracts then re-asserts.
- **FR-A2**: Attribute registers, last-writer-wins and multi-value, so concurrent edits to one attribute converge. The policy per attribute class is recorded.
- **FR-A3**: Bi-temporal metadata, valid-time and transaction-time, on every entity and edge, from this first version, not retrofitted.
- **FR-A4**: The op-based pattern, precondition discipline, and serialization follow the crdt-graph reference, adapted from its 2P2P-Graph semantics to add-wins. The deviation from crdt-graph is documented where it occurs.
- **FR-A5**: Personalized PageRank runs over the graph CRDT state and accepts a transaction-time bound, so retrieval can ask for state as of a time.

### Key entities

- **Entity**: a node, carrying id, attributes as registers, valid-time and transaction-time stamps, and optional document handles into the Yjs backend (the integration point with 0B).
- **Relationship**: an edge, add-wins, with its own bi-temporal stamps.

### Success Criteria

- **SC-A1**: Two replicas concurrently add the same edge and converge to one edge, not a duplicate or a conflict.
- **SC-A2**: A node removed on one replica and re-added on another converges to present, demonstrating add-wins over the permanent tombstoning a 2P2P-Graph would impose.
- **SC-A3**: Two replicas write different values to one attribute and converge per the register's declared policy.
- **SC-A4**: A PPR query with a transaction-time bound returns the state that held at that time, not the latest.
- **SC-WA**: In a real two-replica run, concurrent mutations, an added edge, a removed-then-re-added node, and a conflicting attribute write, all converge to the add-wins, time-correct state. Observed, not asserted from unit tests of the parts.

---

## Workstream 0B: Yjs document backend

### Goal

RustyRed serves the Yjs update protocol as a sync and persistence backend, so rich-text editors sync their documents through it.

### Requirements

- **FR-B1**: RustyRed implements the Yjs sync protocol, state vectors, update exchange, and the awareness channel, as a persistence and sync backend.
- **FR-B2**: The implementation binds to y-octo or yrs, chosen after reading both, with the choice and its reason recorded.
- **FR-B3**: A BlockSuite or AFFiNE client and a TipTap client (ProseMirror through y-prosemirror) both sync documents through RustyRed.
- **FR-B4**: Documents are addressable so a graph entity can reference one. This is the integration point with 0A.

### Success Criteria

- **SC-B1**: Two clients edit one document through RustyRed and converge with no lost write.
- **SC-B2**: A client edits offline, reconnects, and its state merges rather than clobbering.
- **SC-WB**: In a real run, a BlockSuite client and a second client both edit one document synced through RustyRed in the same window, and the edits converge. Observed live.

---

## Integration point

An entity in 0A carries document handles that reference Yjs documents in 0B. That single reference is where the two workstreams meet; otherwise they build independently. Structured state lives in the graph CRDT and is the source of truth; rich content lives in Yjs documents referenced by entities. This is the NextGraph shape.

## Out of scope

The wake and routing layer, which is the Gemma resident's scope; CRDT converges concurrent writes but does not decide who is roused when a change lands. Replacing RustyRed's existing storage; this is additive.

## Assumptions

- The existing RustyRed graph and PPR are extended, not rebuilt.
- y-octo or yrs can be embedded or bridged to serve AFFiNE's sync protocol, confirmed by reading the gateway and the chosen library.
