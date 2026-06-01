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

## What I'll do on receipt

Add `RemoteHarnessRunStore: HarnessRunStore` that GETs these two endpoints and
decodes the `RunState`/`EventState` JSON into `HarnessRun` (+ derives the context
ledger from the `CONTEXT.PACKED` payload and the outcome from `OUTCOME.RECORDED`).
Then `TheoremRootView` swaps `SampleRunStore` for it and every run surface goes
live, no view changes. I'll parity-check the decoder against a real response.
