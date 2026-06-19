# HTTP API

Theorem exposes two HTTP services. Know which one you are calling.

| Service | Crate | What it serves | OpenAPI |
|---|---|---|---|
| **Harness server** | `apps/theorem-harness-server` | Runs, codebase maps, coordination rooms, jobs, connectors | Generated, served at `GET /openapi.json`; committed snapshot [openapi-harness.json](openapi-harness.json) |
| **Graph engine** | `rustyredcore_THG/crates/rustyred-thg-server` | Graph nodes/edges, Cypher, vector/spatial/full-text search, transactions, algorithms, live web search | Self-published at `GET /openapi.json` |

The **Harness server** is the product transport: it is what most integrations talk to. The **graph engine** is the lower-level database service; you usually reach its capabilities through the Harness or through MCP rather than calling it directly.

## Authentication

> The Harness server performs **no authentication** today. The tenant is a plain `tenant` (or `tenant_slug`) query/body string, or a non-default configured value from `THEOREM_HARNESS_TENANT_SLUG`, `THEOREM_AGENT_TENANT_SLUG`, or `THEOREM_TENANT_ID`; silent fallback to `default` is refused on tenant-sensitive routes. Write endpoints (`POST /harness/jobs`, `POST /connectors/register`, `POST /harness/rooms/{room_id}/messages`) are unauthenticated. Run it on a trusted network, or put an authenticating proxy in front of it, before exposing it publicly.

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

Full request/response schemas are in the [committed OpenAPI snapshot](openapi-harness.json), or fetch the live document from a running server at `GET /openapi.json`. See [Run the Harness and submit a job](../guides/first-job.md) for a worked example.

## Viewing and importing the spec

`openapi-harness.json` is a standard OpenAPI 3.1 document; a running server serves the identical document at `GET /openapi.json`. You can:

- Fetch it live: `curl localhost:50080/openapi.json`.
- Paste the committed file into [editor.swagger.io](https://editor.swagger.io) or render it with Redocly / Scalar.
- Import it into Postman or Insomnia as an API definition.
- Render it inline in this GitBook with an OpenAPI block once the file is wired into `.gitbook.yaml`.

## How this stays accurate

The spec is generated from code, not hand-maintained. `openapi_document()` in `apps/theorem-harness-server/src/openapi.rs` is the single source of truth; the server serves it at `GET /openapi.json`, and the committed `openapi-harness.json` is regenerated from it. The test `committed_openapi_snapshot_matches_code` compares the committed snapshot against the code on every `cargo test`, so the document cannot silently drift from the routes. To change the API: edit the routes, update `openapi_document()`, then regenerate with `UPDATE_OPENAPI=1 cargo test`.
