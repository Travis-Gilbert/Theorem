# Pass tenant_slug to harness coordination tools; a bare call 404s but the harness is not down

**Kind:** gotcha
**Captured:** 2026-06-14
**Session signature:** `claude:travisgilbert@Traviss-Laptop:b944c683`
**Domain tags:** harness, coordination, mcp, theorem

## Trigger

Asked to coordinate with Codex on the rustyred-web frontier, the first coordination
calls (`coordination_room action=status`, `read_intents_for_room`, `presence`)
returned `404 {"code":404,"message":"Application not found"}`. The obvious-but-wrong
conclusion was "the coordinate endpoint is down" - which CLAUDE.md and prior memory
both assert as the default. The calls had no `tenant_slug`. Re-issuing
`coordination_room action=join` with `tenant_slug: "rustyredcore-theorem-production"`
resolved room `repo:theorem:branch:001-local-loop` immediately (`degraded: false`),
and a later `coordination_intent` write persisted (read back as `count: 1`) even
though every `route_receipt` stamped `readOnly: true` / `nativeWriteMode: "read-only"`.

## Rule

Before concluding the harness coordination plane is down, retry with
`tenant_slug: "rustyredcore-theorem-production"` (the Theorems-Harness V2 prod app).
`read_intents_for_room` also needs an explicit `room_id` or it silently defaults to
`room:ungrouped` and reports `count: 0`. The `readOnly: true` flag in `route_receipt`
describes the graph data-plane, not coordination - coordination writes (join / intent
/ coordinate) persist regardless; confirm by reading the intent back, not by trusting
the receipt. Codex does not join the room (git-only sprint): expect `members` to list
only yourself, `presence: null`, and `coordinate` delivery `passive` with no mention.
That is Codex's absence, not an outage - use the human as relay.

## Evidence

- `coordination_room action=status` (no tenant) -> `404 Application not found`; same
  call with `tenant_slug=rustyredcore-theorem-production` -> full room JSON,
  `degraded:false`, `mode:collaborating`.
- After `coordination_intent`, `read_intents_for_room {room_id:"repo:theorem:branch:001-local-loop", tenant_slug:...}`
  -> `count: 1` with the footprint intact, despite `route_receipt.readOnly == true`.
- `read_intents_for_room` without `room_id` -> `room_id: "room:ungrouped"`, `count: 0`.

## Encoded in

- `docs/learnings/2026-06-14-harness-coordination-needs-tenant-slug.md` (this file)
