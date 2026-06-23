# SPEC-RUSTYRED-STREAM-COORDINATION

Status: active implementation spec

Parent: `NORTH-STAR-RUSTYRED-MULTIMODEL.md`

## Purpose

Replace coordination room polling as the only live-update mechanism with append-only, cursor-read event streams that are tenant scoped and durable in RustyRed graph storage.

## Required Shape

- A stream is addressed by tenant and topic.
- Each published event receives a monotonic ordering token.
- Events are persisted as `CoordinationStreamEvent` graph nodes linked to a `CoordinationStream` head node.
- Readers request events after their actor cursor and can choose whether the read advances that cursor.
- Subscriptions store the stream topics an actor should read on passive turn start.
- `ask` and `block` events with a `target_actor` also bridge to the existing mention/wake path.

## Acceptance Criteria

- Publishing two events to the same stream yields distinct, increasing ordering tokens.
- Reading as another actor returns the events in token order.
- Reading with `advance: true` advances that actor's cursor and prevents duplicate delivery on the next read.
- Reading with `advance: false` leaves the cursor unchanged.
- An `ask` or `block` event with a target actor creates a room mention/wake message.
- GraphQL coordination uses the same stream publish/read handlers as the flat MCP tools.

## Validation

- `cargo test -p rustyred-thg-core stream --lib`
- `cargo test -p rustyred-thg-mcp stream_ --lib`
- `cargo test -p rustyred-thg-mcp graphql_coordination --lib`
