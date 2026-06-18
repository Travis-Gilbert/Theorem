# HTTP API

Theorem exposes two HTTP services. Know which one you are calling.

| Service | Crate | What it serves | OpenAPI |
|---|---|---|---|
| **Harness server** | `apps/theorem-harness-server` | Runs, codebase maps, coordination rooms, jobs, connectors | [openapi-harness.yaml](openapi-harness.yaml) (in this repo) |
| **Graph engine** | `rustyredcore_THG/crates/rustyred-thg-server` | Graph nodes/edges, Cypher, vector/spatial/full-text search, transactions, algorithms, live web search | Self-published at `GET /openapi.json` |

The **Harness server** is the product transport: it is what most integrations talk to. The **graph engine** is the lower-level database service; you usually reach its capabilities through the Harness or through MCP rather than calling it directly.

## Authentication

> The Harness server performs **no authentication** today. The tenant is a plain `tenant` (or `tenant_slug`) query/body string, defaulting to `default`. Write endpoints (`POST /harness/jobs`, `POST /connectors/register`, `POST /harness/rooms/{room_id}/messages`) are unauthenticated. Run it on a trusted network, or put an authenticating proxy in front of it, before exposing it publicly.

The graph engine is different: it requires `Authorization: Bearer <token>` and enforces per-command scopes (see its own `/openapi.json` and `/v1/diagnostics/config`).

## Harness server endpoints at a glance

Default bind: `0.0.0.0:50080` (set `PORT`). Data directory: `THEOREM_HARNESS_DATA_DIR` (default `harness-data`).

| Method | Path | Purpose |
|---|---|---|
| GET | `/healthz` | Liveness. |
| GET | `/harness/runs` | List runs. |
| GET | `/harness/runs/{run_id}` | One run + its event log. |
| GET | `/harness/maps` | List codebase map artifacts. |
| GET | `/harness/maps/{map_id}` | One map artifact. |
| GET | `/harness/rooms/{room_id}` | Room state. |
| GET | `/harness/rooms/{room_id}/presence` | Member presence. |
| GET | `/harness/rooms/{room_id}/intents` | Announced work footprints. |
| GET | `/harness/rooms/{room_id}/records` | Decisions, tensions, reflections. |
| POST | `/harness/rooms/{room_id}/messages` | Write a message (tap or wake). |
| GET | `/harness/rooms/{room_id}/stream` | SSE stream of room messages. |
| GET | `/harness/actors/{actor_id}/mentions` | Mention inbox. |
| POST | `/harness/jobs` | Submit a dispatch job. |
| GET | `/harness/jobs/counts` | Dispatch state counts. |
| GET | `/connectors` | List MCP connectors and tool affordances. |
| POST | `/connectors/register` | Register an MCP server's tools. |
| POST | `/github/webhook` | GitHub App webhook (only if configured). |

Full request/response schemas are in [openapi-harness.yaml](openapi-harness.yaml). See [Run the Harness and submit a job](../guides/first-job.md) for a worked example.

## Viewing and importing the spec

The `openapi-harness.yaml` file is a standard OpenAPI 3.1 document. You can:

- Paste it into [editor.swagger.io](https://editor.swagger.io) or render it with Redocly / Scalar.
- Import it into Postman or Insomnia as an API definition.
- Render it inline in this GitBook with an OpenAPI block once the file is wired into `.gitbook.yaml`.

## A note on keeping this accurate

This static YAML is committed for immediate use, but it can drift from the code the same way prose docs do. The graph engine avoids that by *generating* its spec from a single source served at `/openapi.json`. The recommended hardening for the Harness server is to do the same — add a generated `/openapi.json` route mirroring `rustyred-thg-server/src/openapi.rs` — so the document cannot lag the routes. Until then, treat `main.rs` route registrations as the source of truth and update this file alongside route changes.
