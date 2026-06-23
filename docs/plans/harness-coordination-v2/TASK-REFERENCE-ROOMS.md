# Task-Reference Rooms for Theorems Harness Coordination

Date: 2026-06-21
Status: plan

## Problem

Harness coordination is still too easy to miss even when every head is trying to
use it correctly. The current model asks each head to already know the right
room, tenant, actor spelling, and checkout identity. If Claude posts a handoff to
`room:ungrouped` while Codex reads a task-named room, both messages can be
durable and still invisible at the moment they matter.

Streams fixed one class of problem: they made coordination append-only and
cursor-readable. They did not solve discovery, room aliases, task identity,
checkout targeting, or contradictory live claims.

## Goal

Make coordination addressable by task, not by guessed room name. The harness
should resolve a canonical room from task metadata, surface related messages
from aliases/inboxes, preserve a versioned event history, restore explicit pings
for interrupts, and flag contradictory claims in real time.

## Design Principles

- **Task reference first:** agents join by `task_ref`; room names are display
  labels, not routing truth.
- **Canonical plus permissive:** every task has one canonical room, but inboxes
  and aliases can route related messages into it.
- **Streams plus pings:** streams carry ambient history; `ask` and `block`
  create actor-targeted wake/mailbox items with delivery state.
- **Git-like provenance, not Git-like burden:** rooms use immutable events,
  refs, aliases, and merges. Agents do not manually manage branches of chat.
- **Checkout-aware actors:** `@codex` is not precise enough; messages can target
  a branch/worktree/session tuple.
- **Contradictions are graph facts:** inconsistent claims become typed
  contradiction edges as they are written, not a manual review chore.

## Core Model

### TaskRef

`TaskRef` is the stable coordination address. It is computed from normalized
metadata:

```json
{
  "tenant_slug": "Travis-Gilbert",
  "repo": "Travis-Gilbert/Theorem",
  "workstream": "SPEC-9 CommonPlace Desktop",
  "spec_refs": ["docs/plans/commonplace-desktop-tauri/SPEC-9-commonplace-desktop-tauri.md"],
  "external_refs": ["/Users/travisgilbert/Downloads/SPEC-9-commonplace-desktop-tauri.md"],
  "branch": "Travis-Gilbert/spec-9-commonplace-desktop-tauri"
}
```

The resolver returns:

- `task_ref_id`: content hash over the normalized core fields.
- `canonical_room_id`: the room all heads should read/write for this task.
- `aliases`: room names, inboxes, old rooms, and branch/worktree rooms mapped to
  the canonical room.
- `confidence`: exact, strong, weak, or ambiguous.

### Versioned Room State

Rooms should be versioned like a lightweight Git ref over an append-only log:

- `RoomEvent`: immutable message/intent/decision/tension/reflection/checklist
  event with `event_id`, `parent_event_ids`, `task_ref_id`, and `seq`.
- `RoomRef`: mutable pointer such as `refs/rooms/spec-9-commonplace-desktop` or
  `refs/inbox/ungrouped`.
- `RoomAlias`: maps an old/ref/inbox room to a canonical room with a confidence
  and reason.
- `RoomMerge`: records that events from one ref were folded into another.

This gives the useful parts of Git: history, refs, merge provenance, and
replayable state. It avoids forcing agents to decide whether to create a new
room branch. The default operation is still "resolve task, append event."

### Permissive Related-Message Routing

`room:ungrouped` becomes a real inbox. On write, the harness attempts to attach
the event to a canonical task by looking at:

- tenant
- repo path
- branch
- worktree
- spec path or downloaded file path
- mentioned files
- message keywords
- actor/session metadata

If confidence is high, the message appears in the canonical room as a related
event. If confidence is weak, the room digest shows it under "possible related
messages" with a one-click/manual `RoomAlias` confirmation.

### Restored Pings

Every `coordinate` or stream publish with `urgency=ask` or `urgency=block`
creates an `ActorPing`:

```json
{
  "target_actor": "codex",
  "target_worktree": "/Users/travisgilbert/Tech Dev Local/Creative/Website/Theorem-spec9-commonplace-desktop-tauri",
  "target_branch": "Travis-Gilbert/spec-9-commonplace-desktop-tauri",
  "event_id": "event_...",
  "delivery": "wake",
  "status": "pending"
}
```

Streams remain the ambient timeline. Pings are the interrupt channel with
visible delivery/seen/consumed state.

### Coordination Manifest

Each active task can write `.harness/coordination.json` into the worktree:

```json
{
  "schema_version": 1,
  "task_ref_id": "task_...",
  "canonical_room_id": "room:...",
  "tenant_slug": "Travis-Gilbert",
  "repo": "Travis-Gilbert/Theorem",
  "branch": "Travis-Gilbert/spec-9-commonplace-desktop-tauri",
  "worktree": "/Users/travisgilbert/Tech Dev Local/Creative/Website/Theorem-spec9-commonplace-desktop-tauri",
  "actors": {
    "codex": { "role": "primary Theorem-side editor" },
    "claude-code": { "role": "frontend lane and review" }
  },
  "open_questions": []
}
```

This file is a local affordance, not the source of truth. The source of truth is
the harness room graph. The manifest prevents cold-start ambiguity and makes the
current coordination address visible to humans and tools.

### Real-Time Contradiction Edges

Every structured coordination event can emit `Claim` nodes:

- `Claim(subject="D3 frontend export", predicate="status", object="blocked")`
- `Claim(subject="Codex backend", predicate="status", object="done")`
- `Claim(subject="Tauri dev port", predicate="value", object="1420")`
- `Claim(subject="Tauri dev port", predicate="value", object="3000")`

On write, a deterministic contradiction pass compares new claims against current
task claims. When it detects incompatible values, it writes:

- `CONTRADICTS` edge between the claims
- `CoordinationContradiction` room event
- optional `ActorPing` if the contradiction changes an open ask/block

The first implementation should be rule-based and conservative:

- same subject + same predicate + different scalar object
- same checklist item marked done and blocked without a later superseding event
- same file/branch/worktree claimed by two heads with incompatible status
- newer verified evidence conflicts with older prose summary

Later, a model extractor can propose candidate claims from free text, but the
first path should prefer structured metadata so it is debuggable.

## Checklist

| ID | Slice | Files / Surface | Acceptance |
|---|---|---|---|
| TRR-001 | Add `TaskRef` resolver | `theorem-harness-runtime` coordination module; MCP wrapper | Given tenant+repo+spec/branch metadata, both Codex and Claude resolve the same `task_ref_id` and `canonical_room_id`. |
| TRR-002 | Add room refs, aliases, and merges | runtime graph model + MCP read/write tools | A message written to `room:ungrouped` with SPEC-9 metadata appears in the SPEC-9 canonical room as a related event with provenance. |
| TRR-003 | Restore explicit pings | stream publish / coordinate write path; mentions/wake bridge | `urgency=ask` or `block` creates a target actor ping with pending/seen/consumed status, even if the actor is not subscribed to the stream. |
| TRR-004 | Add turn-start discovery | plugin/skill hook layer; `coordination_context` | At turn start, a head sees canonical room, related inbox messages, open pings, active intents, stale intents, and contradictions for its task. |
| TRR-005 | Write/read coordination manifest | repo-local `.harness/coordination.json`; task start/update command | Starting a task writes a manifest; a later head in the worktree can find the canonical room without guessing. |
| TRR-006 | Target checkout identity | actor presence + ping metadata | A ping can target `actor + branch + worktree`; a different checkout sees that it is not the intended target. |
| TRR-007 | Add contradiction claim model | runtime graph model | Structured claim conflicts write `CONTRADICTS` edges and a room-visible contradiction event. |
| TRR-008 | Add room dashboard digest | MCP/context response and desktop/harness-console consumers | Digest displays canonical room, aliases, active actors, stale actors, pending pings, related ungrouped messages, and contradictions. |
| TRR-009 | Backfill compatibility | existing `coordinate`, `read_messages_for_room`, `stream_read` | Existing room users keep working; old room ids become aliases or standalone rooms with no data loss. |
| TRR-010 | Acceptance replay | tests + fixture room transcript | A replay of the SPEC-9 failure mode shows Claude's ungrouped handoff discovered by Codex before edits begin. |

## Implementation Sequence

1. Build `TaskRef` resolution and canonical room lookup with tests.
2. Add alias/merge graph records and make `read_messages_for_room` include
   related alias events.
3. Restore actor pings on top of stream/coordinate writes.
4. Add turn-start discovery and manifest read/write.
5. Add checkout-targeted ping metadata.
6. Add deterministic structured contradiction edges.
7. Expose a digest through MCP and the desktop/harness-console UI.
8. Replay the SPEC-9 miss as an acceptance fixture.

## Validation

- Unit tests for `TaskRef` normalization: path variants, downloaded spec path,
  branch changes, and tenant casing.
- Runtime tests for alias routing: write to ungrouped, read from canonical,
  verify provenance and no duplicate delivery.
- Ping tests: unsubscribed target still receives `ask`/`block`; passive messages
  remain stream-only.
- Manifest tests: create/update/read without overwriting unrelated task state.
- Contradiction tests: deterministic conflicts emit one edge/event; superseded
  claims do not keep re-alerting.
- Integration replay: the SPEC-9 transcript where Claude wrote to ungrouped and
  Codex read the SPEC-9 room must produce a turn-start warning before Codex
  edits.

## Non-Goals

- Do not replace isolated worktrees with shared editing.
- Do not make agents manually manage room branches.
- Do not rely on an LLM-only contradiction detector for v1.
- Do not make every passive progress note wake other heads.

## Open Questions

- Should `TaskRef` include user thread id as a hard field or only as a
  confidence signal? Thread id helps precision but can split the same task
  across resumed sessions.
- Should aliases be auto-created on strong confidence, or should auto-alias only
  happen for system-owned inboxes like `room:ungrouped`?
- Should room refs be stored in the same GraphStore as coordination records, or
  mirrored into a Git-backed file for audit/export?
- Should contradiction pings target the authors of both claims, the current
  primary editor, or only room subscribers?
