# iOS transport handoff (runtime -> HTTP -> Swift client)

Date: 2026-06-01
From: claude-code (Swift UI lane). To: codex (Rust infra lane).

## The gap

The iOS run surfaces (Runs list, run detail, Evidence/Cost/Outcome rails, trace)
are built and render a `HarnessRun`. The Swift client now reads runs through a
`HarnessRunStore` protocol (`apps/theorem-ios/.../Models/HarnessRunStore.swift`),
defaulting to `SampleRunStore` (the recorded reference run). Swapping in a
runtime-backed `RemoteHarnessRunStore` is a one-line change in `TheoremRootView`
(`RunsListView(theme:store:)`).

What's missing is the transport. `theorem-harness-runtime` persists runs to a
`GraphStore` and exposes an in-process Rust API (`load_run`, `load_events`,
`replay_persisted_run`) but **no HTTP/SDK surface** (spec Part 7). The Swift
client speaks HTTP, so it cannot reach the runtime yet.

## The ask: a thin HTTP surface over the runtime

Two read endpoints are enough to make every run surface live. JSON is exactly
what the runtime already persists (the `RunState` and `EventState` serde shapes),
so no new serialization is needed.

### `GET /harness/runs`

List runs, most recent first. The runtime currently loads a run by id; this needs
a runs index: query the `GraphStore` for `HarnessRun`-labelled nodes (the node id
is `harness:run:{run_id}`, see `event_log.rs::run_node_id`). Response:

```json
{ "runs": [ { <RunState serde> , "state_hash": "..." }, ... ] }
```

The list view only needs `run_id`, `task`, `actor`, `status`, and
`last_event_seq` per run; returning the full run node is fine.

### `GET /harness/runs/{run_id}`

One run plus its ordered event log. Wraps `load_run` + `load_events`:

```json
{
  "run":   { <RunState serde>, "state_hash": "..." },
  "events": [ { <EventState serde> }, ... ]    // ordered by seq
}
```

`EventState` carries `seq`, `type`, `payload`, `state_hash_before`,
`state_hash_after`, `created_at`. Note it does NOT carry the per-event run
status; the client derives status from event type (a small kernel-mirror map) or,
preferably, the endpoint replays and attaches `status` per event (it already has
`replay_persisted_run`). Attaching `status` server-side is cleaner; say which you
pick and I'll match the decoder.

## Where it could live

- Extend the existing `theorem-grpc` crate (already a standalone Rust HTTP/gRPC
  service) with a `HarnessRunService`, OR
- A small Axum surface in a new `theorem-harness-server` crate over
  `theorem-harness-runtime` + a `GraphStore`.

Either is fine; the JSON contract above is what matters. The `RedCoreGraphStore`
(durable, file-backed) is the natural store to read from.

## Coordination + presence (added 2026-06-01, after the native substrate landed)

`theorem-harness-runtime::coordination` now persists rooms, intents, durable
presence, messages, and `@actor` mentions to the same GraphStore. The same
transport exposes these so the Participants surface (UI spec Part 4) can go from
honest-idle to live status, and so agents can coordinate over the reliable
native path instead of git. Read endpoints, wrapping the existing functions:

### `GET /harness/rooms/{room_id}/presence`

Implemented in `apps/theorem-harness-server`. `list_presence` -> who is fresh
right now. Response:
`{ "tenant": "...", "presence": [ { <CoordinationPresenceState serde> }, ... ], "count": 0 }`.
Drives the participant status dots (idle / engaged / contributing /
unreachable).

### `GET /harness/rooms/{room_id}/intents`

Implemented in `apps/theorem-harness-server`. `read_intents_for_room` -> live
intents (actor, summary, claimed_files, status). Response:
`{ "tenant": "...", "room_id": "...", "intents": [ { <CoordinationIntentState serde> }, ... ], "count": 0 }`.
Optional `status` or comma-separated `statuses` filters the live claim list.

### `GET /harness/rooms/{room_id}` and `GET /harness/actors/{actor}/mentions`

Implemented in `apps/theorem-harness-server`. `room_status` and
`read_mentions_for_actor` -> membership/task and the @mention inbox. Mentions
accept optional `limit` and `consume=true` query parameters.

### `GET /harness/rooms/{room_id}/records`

Implemented in `apps/theorem-harness-server`. `read_records_for_room` -> durable
room events, decisions, tensions, and reflections. Optional `record_type` or
comma-separated `record_types` filters the timeline, and optional `limit`
controls the response length.

All coordination endpoints accept `tenant` or `tenant_slug` as a query parameter;
omitting it uses `default`, which is appropriate for local smoke stores but not
hosted tenant routing.

I'll add a `RemoteParticipantStore` that decodes the presence/intent contract the
same way `RemoteHarnessRunStore` decodes runs, so the Participants surface goes
live with the same one-line swap.

### Separately: native coordination in the MCP server

The bigger win is not iOS. `rustyred-thg-mcp` now exposes the native
coordination room, presence, intent, message, mention, and durable record tools
over the Rust runtime-backed graph path, plus a bundled `coordination_context`
packet for turn-start injection and `coordination_contribution` for structured
work capture. Durable record/contribution writes can also carry required-scope
and cost-budget hooks that emit policy receipts. It also exposes
`harness_append_transition` and `harness_run`, so agents can append runtime
transitions and read back the same `{run, events}` contract that the HTTP server
serves to iOS/web clients. That moves the agent write/read surface off the
flaky Python harness while HTTP remains the read transport for app surfaces.

Live data requires one shared RedCore store path: with THG server defaults, set
`THEOREM_HARNESS_DATA_DIR=$RUSTY_RED_DATA_DIR/tenants/<tenant>` so MCP writes
and the standalone HTTP transport read the same `HarnessRun` / `HarnessEvent`
nodes.

## What I'll do on receipt

Add `RemoteHarnessRunStore: HarnessRunStore` that GETs these two endpoints and
decodes the `RunState`/`EventState` JSON into `HarnessRun` (+ derives the context
ledger from the `CONTEXT.PACKED` payload and the outcome from `OUTCOME.RECORDED`).
Then `TheoremRootView` swaps `SampleRunStore` for it and every run surface goes
live, no view changes. I'll parity-check the decoder against a real response.
