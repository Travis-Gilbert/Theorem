# SPEC-GRAPHQL-MCP

Status: active implementation spec

Parent: `NORTH-STAR-RUSTYRED-MULTIMODEL.md`

## Purpose

Expose the Harness multi-model substrate through one typed GraphQL MCP surface while preserving the existing flat MCP tools as the source of truth. GraphQL resolvers wrap crate-private payload handlers; they do not reimplement memory, graph, or coordination behavior.

## Scope

- A0/A1: memory recall, relation traversal, archive recall, remember, encode, revise, forget, and handoff.
- A2: coordination room context, coordination records, coordination intent writes, and stream-backed coordination events.
- A3: graph algorithm, node, neighbor, schema, vector, full-text, spatial, symbolic, and bulk graph operations.

## Required Shape

- `graphql_query` executes read-only GraphQL documents.
- `graphql_mutate` executes GraphQL mutations when MCP writes are enabled.
- `graphql_introspect` returns the SDL for the typed schema.
- Tenant is resolved once from the MCP connection/tool call and is not accepted as a GraphQL field argument.
- The flat tools remain available and continue to answer with the same payloads.

## A2 Coordination Requirements

- A room query returns room metadata, presence, intents, messages, records, pending mentions, and counts in one response.
- Coordination GraphQL mutations can write an intent and a coordination record through the same handlers as `coordination_intent` and `coordination_record`.
- A GraphQL mutation can publish a stream event through the same handler as `stream_publish`.
- A GraphQL query can read stream events through the same handler as `stream_read`.
- An event published through GraphQL is readable by another actor through the stream read path.

## Acceptance Criteria

- `cargo test -p rustyred-thg-mcp graphql_coordination --lib`
- `cargo test -p rustyred-thg-mcp --lib`
- `cargo check --workspace`

Full workspace tests should be run before considering the whole North Star lane finished.
