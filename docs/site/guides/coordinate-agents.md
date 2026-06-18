# Coordinate two agents

When two agents (say Claude Code and Codex) work the same repository, the failure mode is not merge conflicts — it is duplicated work, stale handoffs, and two changes that merge cleanly but disagree at runtime. The Harness coordination tools solve this with *shared awareness*: each head announces what it is doing before it acts, and reads what its peers are doing. The model is one agent with several heads; the discipline is **frequency over fences**.

This guide uses the coordination tools from the [MCP tool catalog](../reference/mcp-tools.md). Everything is scoped to a room — typically one per repo+branch, e.g. `repo:theorem:branch:main`.

## 1. Join the room

```json
// tool: coordination_room
{ "action": "join", "actor": "claude-code", "room_id": "repo:theorem:branch:main",
  "repo": "Theorem", "branch": "main", "task": "documentation build" }
```

`action` is `status` (read), `join`, or `start`. You can pass `room_id` directly, or let it be derived from `repo` + `branch`.

## 2. Announce your footprint before you work

This is the load-bearing step. Before touching files, write an intent naming the files your hands are on. A peer reading the room now knows not to touch them.

```json
// tool: coordination_intent
{ "actor": "claude-code", "room_id": "repo:theorem:branch:main",
  "status": "working", "summary": "Adding generated /openapi.json to the harness server",
  "footprint": ["apps/theorem-harness-server/src/openapi.rs", "apps/theorem-harness-server/src/main.rs"] }
```

`status` is `working`, `paused`, or `done`. `footprint` is the list of files (or surfaces) you are on. Update it as you move; set `status: "done"` to close the announcement as a handoff.

## 3. Read peers before you start

```json
// tool: read_intents_for_room
{ "room_id": "repo:theorem:branch:main" }
```

If a peer's footprint overlaps structurally coupled code, that is the signal to coordinate rather than collide. Build on a peer's finished edit instead of redoing it.

## 4. Send a direct message, optionally waking a head

```json
// tool: coordinate
{ "actor": "claude-code", "room_id": "repo:theorem:branch:main",
  "message": "@codex I landed the OpenAPI module; the spec test is green. Safe to rebase.",
  "urgency": "ask", "mentions": ["codex"], "delivery": "wake" }
```

`urgency` is `info`, `ask`, or `block`. `delivery` is `passive` (a tap — recorded, the peer sees it when they next read) or `wake` (a hold that can spawn the mentioned head to act now). Use `wake` sparingly, for genuine blocks and forks.

## 5. Read your mentions

```json
// tool: mentions
{ "actor": "codex", "consume": true, "limit": 20 }
```

`consume: true` marks them read. Filter by `urgency` (e.g. only `block`) when you only want the things that gate your work.

## 6. Record durable decisions

Messages are ephemeral coordination; decisions should outlive the conversation.

```json
// tool: coordination_record
{ "actor": "claude-code", "room_id": "repo:theorem:branch:main",
  "record_type": "decision", "title": "OpenAPI is generated",
  "summary": "Harness spec is generated from code and test-verified; do not hand-edit the JSON." }
```

`record_type` is `event`, `decision`, `tension`, or `reflection`. These are what a later session reads to understand *why* things are the way they are.

## The same surface over HTTP

Every read here has an HTTP equivalent on the harness server — `GET /harness/rooms/{room_id}/intents`, `/records`, `/presence`, `/actors/{actor}/mentions` — and you can post a message with `POST /harness/rooms/{room_id}/messages` or watch a room live over SSE at `/harness/rooms/{room_id}/stream`. See the [HTTP API](../reference/api-http.md) and the [first-job guide](first-job.md) for the live-stream example.

> The model behind this — heads, rooms, frequency over fences — is explained in [The Harness](../concepts/the-harness.md).
