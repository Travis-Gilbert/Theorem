# Spec 1: Coordination Room on the CRDT Substrate

Depends on Spec 0. Do not start until Spec 0's wiring criteria are green, because this imports the substrate.

First step in any session this spawns: `git pull`.

## Goal

The harness room, today append-only by default rather than by design, becomes a live shared space whose state converges across every participant writing at once, with the full workspace surface people expect.

## Grounding contract

Read the pinned substrate (Spec 0) and the sources below; bind to what is there rather than reconstructing it.

- The current room data model in `Travis-Gilbert/Theorem`, under `rustyredcore_THG`: the present message, record, intent, and footprint shapes, and the coordination tools that write them. Migrate these onto the graph CRDT; do not assume their shape from this spec.
- The Spec 0 graph CRDT and Yjs backend, as built.

Record commit hashes in `REFERENCE.md`. Completion is defined by the wiring criterion observed in a real run.

## Requirements

- **FR-101**: The room's structured state, the migrated messages, records, intents, and footprints, lives in the graph CRDT from Spec 0, written concurrently by every head and by the human.
- **FR-102**: The editable workspace surface runs on AFFiNE over the Yjs backend from Spec 0. The full Notion-class workspace is wanted here.
- **FR-103**: Structured room entities reference AFFiNE Yjs documents for rich content, keeping one source of truth for entities and one for content.
- **FR-104**: PPR retrieval, with hierarchical community summaries and bi-temporal awareness, runs over the room's structured state.
- **FR-105**: The room exposes the awareness channel as a presence and footprint signal, the lightweight "who is on what" broadcast, distinct from the durable graph writes.

## Key entities

The migrated room entities (message, record, intent, footprint) as graph CRDT nodes, bound to their current shapes read from `rustyredcore_THG`, each referencing AFFiNE documents for rich content.

## Success Criteria

- **SC-101**: The append-only history migrates to the graph CRDT with no loss, verifiable by reading the same records before and after.
- **SC-102**: A PPR query returns time-correct results when a fact has changed across transaction time.
- **SC-W**: Two heads, for example Claude Code and the Gemma resident, write room state from separate replicas in the same window; the writes converge with no lost write, and a third participant's PPR query returns the merged result. Observed in a real run through the room, not asserted from a unit test of the parts.

## Out of scope

GNN-based message passing between LLM heads, ruled out: text-in text-out heads cannot consume embeddings. The wake logic itself, which is the resident's scope.

## Assumptions

- The current append-only room is migrated, not discarded.
- AFFiNE's license permits this use; confirm before committing the room to it.
