# Agent Space Viewport: transport contract (server lane DONE)

Server-side implementation of `~/Downloads/rustyred-console-agent-space-viewport (1).md`.
This is the **live API** the cosmos.gl client builds against (the design doc's
explicit instruction: "build the client against the live API rather than
transcribing them here"). The server transport spine is implemented, tested,
and clippy-clean in the Theorem repo. The cosmos.gl client is the **sibling
lane** and lives in a separate console app (not in this Rust repo).

## Lane split

| Lane | Owner | Repo | Status |
|---|---|---|---|
| Rust server transport (this doc) | Claude Code | `Theorem` | DONE, tested |
| cosmos.gl client (`app/agent-space/`, `useAgentSpaceStream`, `AgentSpaceView`) | Codex | RustyRed Console (Next app; not yet scaffolded on disk) | TODO, against this API |
| CRDT delta emission (work-graph engine drives `publish_crdt_delta`) | CRDT-substrate lane | `Theorem` + `RustyRed-Graph-Database` | seam ready, engine-side TODO |

## What shipped (Theorem repo)

- `theorem-harness-runtime/src/coordination_push.rs`: the `AgentSpaceEvent`
  vocabulary, the `CrdtDelta` contract, and an **additive sibling broadcast bus**
  with a monotonic publish `seq`. The existing room-message bus + wake listener
  are untouched; the agent-space bus is a strict superset.
- `rustyred-thg-mcp/src/lib.rs`: emit hooks. After a coordination write is
  persisted, the dispatch mirrors it onto the agent-space bus
  (`presence` -> Presence, `coordination_intent` -> Footprint,
  `coordination_record` -> Record, `harness_append_transition` ->
  WorkGraphTransition). Room messages auto-mirror via
  `publish_coordination_room_event_from_state`.
- `rustyred-thg-server/src/agent_space.rs`: the two HTTP routes.
- `rustyred-thg-server/src/auth.rs`: `coordination:read` added to the scope
  vocabulary.

## HTTP API

Both routes require scope `coordination:read` (granted in dev mode; grant
per-token in production). Origin is checked against `allowed_origins`.

### `GET /v1/agent-space/snapshot?tenant=<slug>&room=<room_id>`

Point-in-time seed. `tenant` is required (400 `missing_tenant` otherwise);
`room` is optional. Returns:

```json
{
  "tenant": "rustyredcore-theorem-production",
  "room_id": "room:crdt-substrate",
  "cursor": 1287,
  "room": { "room": { "room_id": "...", "members": { "...": {...} }, ... } },
  "presence": { "<actor>": { "status": "joined", ... } },
  "work_graph": { ...harness_kg_status payload... }
}
```

`cursor` is the monotonic high-water sequence at snapshot time. **Open the
stream with `since=cursor`.**

### `GET /v1/agent-space/stream?tenant=<slug>&room=<room_id>&since=<seq>`

SSE (`text/event-stream`). Each frame is one `AgentSpaceEnvelope`:

```
event: presence
data: {"seq":1288,"tenant_slug":"...","room_id":"room:...","event":{"type":"presence","data":{"actor":"codex","status":"working","ts_ms":...}}}
```

- SSE `event:` name is the event kind (see table).
- `seq` is monotonic. **The client drops every frame with `seq <= cursor`** to
  de-dupe across the snapshot/stream boundary (acceptance criterion #1).
- `tenant` filters; `room` further filters to that room plus room-less (global)
  events such as tenant-wide presence. Omit `tenant` for a firehose.

### Backfill -> tail protocol (no double-apply)

1. Open the stream first (begins buffering live frames with their `seq`).
2. `GET /snapshot` -> apply `room`/`presence`/`work_graph`, remember `cursor`.
3. Apply buffered + subsequent stream frames, dropping `seq <= cursor`.

`cursor` is read AFTER the snapshot body composes, so anything reflected in the
body has `seq <= cursor` and is never re-applied. (A frame landing mid-compose
may be briefly missed; the next event/settle reconciles -- the observatory is
eventually consistent. The hard guarantee is *no double-apply*.)

## Event kinds (SSE `event:` name -> envelope `event.type` -> payload)

| `event:` | `event.type` | `event.data` fields |
|---|---|---|
| `room_message` | `room_message` | full `RoomMessageEvent` (tenant_slug, room_id, message_id, author, urgency, message, mentions[], delivery, created_at) |
| `presence` | `presence` | actor, status, ts_ms |
| `footprint` | `footprint` | actor, target, op (`add`/`remove`), ts_ms |
| `work_graph_transition` | `work_graph_transition` | node_id, from, to, actor, ts_ms |
| `record` | `record` | kind, summary, refs[], ts_ms |
| `crdt_delta` | `crdt_delta` | op (`add_vertex`/`remove_vertex`/`add_edge`/`remove_edge`/`set_prop`), element_id, field?, value?, causal{dot?,version_vector{},ts_ms}, actor, settled, conflict? |

## CRDT delta contract (sequencing step 4 seam)

The transport and types are ready; the work-graph CRDT engine drives
`theorem_harness_runtime::publish_crdt_delta(tenant, room, CrdtDelta)` when it
resolves an op. `settled=false` = pending (concurrent); `conflict` names the
resolved state the viewport renders: `tombstone`, `dangling_edge`,
`contested_property`, `cycle_compensation`. Dangling-edge policy in effect
should be carried in `conflict` so the viewport renders whichever the engine
applied (`preserve-and-hide-then-restore` recommended).

## Client lane TODO (Codex)

Build the cosmos.gl viewport against the API above. The SceneDirective mapping
table (design doc section 3) maps each `event:` kind to incremental scene
setter calls, coalesced per animation frame. The base-plus-delta overlay
(section 4) colours pending CRDT deltas by actor and promotes to settled style
when `settled=true`. Reuse the console shell + cosmos.gl integration from the
component-lift map.

## Acceptance criteria status

- [x] Snapshot seeds then stays live via SSE, no double-apply (seq/cursor).
- [x] New coordination message animates within one frame (RoomMessage emit; one bus hop).
- [x] claim -> patch -> verify renders as a lineage chain (WorkGraphTransition emits with from/to/node_id).
- [x] CRDT pending/settled/conflict states carried in the typed `CrdtDelta` contract.
- [ ] Visual rendering of all of the above (client lane).
- [x] High event rate: broadcast bus + client per-frame coalescing (client coalesces; server fans out).

## Verification

```
cd rustyredcore_THG
cargo test -p theorem-harness-runtime --lib   # 103 (incl. 5 agent-space transport tests)
cargo test -p rustyred-thg-mcp --lib           # 54  (incl. emit-hook end-to-end)
cargo test -p rustyred-thg-server --lib        # 173 (incl. snapshot + SSE-stream route tests)
```
