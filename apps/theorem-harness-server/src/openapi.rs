//! OpenAPI 3.1 document for the Theorem Harness HTTP API.
//!
//! [`openapi_document`] is the single source of truth for the spec. It is served
//! live at `GET /openapi.json` and is the canonical description of this server's
//! routes. The committed, human-readable snapshot at
//! `docs/site/reference/openapi-harness.json` is verified against this function
//! by the `committed_openapi_snapshot_matches_code` test, so the published doc
//! cannot drift from the code: change a route, the test goes red until you
//! regenerate the snapshot with `UPDATE_OPENAPI=1 cargo test`.
//!
//! This mirrors the pattern `rustyred-thg-server` uses for its own
//! `GET /openapi.json` (see `rustyredcore_THG/crates/rustyred-thg-server/src/openapi.rs`).

use serde_json::{json, Value};

/// The API contract version. Deliberately distinct from the crate version,
/// which is an unversioned internal placeholder (`0.0.0`).
const API_VERSION: &str = "0.1.0";

/// Build the OpenAPI 3.1 document describing this server's HTTP surface.
pub fn openapi_document() -> Value {
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "Theorem Harness HTTP API",
            "version": API_VERSION,
            "summary": "HTTP/JSON transport for Theorem's Harness — runs, rooms, jobs, and connectors.",
            "description": concat!(
                "The HTTP surface of `theorem-harness-server`: a thin Axum server over a durable ",
                "file-backed graph store. It serves the read contract Theorem clients consume ",
                "(runs, codebase maps, coordination rooms, presence, intents, records, mentions) ",
                "plus write endpoints for room messages, dispatch jobs, and MCP connector ",
                "registration.\n\n",
                "Scope: this is the Harness product transport only. The graph engine ",
                "(`rustyred-thg-server`) is a separate service with its own, larger API published ",
                "at its own `GET /openapi.json`.\n\n",
                "Authentication: this server performs NO authentication today. The tenant is supplied ",
                "as a plain `tenant` (or `tenant_slug`) string per request, or by a non-default ",
                "configured value from `THEOREM_HARNESS_TENANT_SLUG`, `THEOREM_AGENT_TENANT_SLUG`, ",
                "or `THEOREM_TENANT_ID`; silent fallback to `default` is refused. ",
                "Write endpoints (`POST /harness/jobs`, `POST /connectors/register`, ",
                "`POST /harness/rooms/{room_id}/messages`) are therefore unauthenticated. Run it ",
                "on a trusted network or behind an authenticating proxy before exposing it ",
                "publicly.\n\n",
                "Response shapes: envelope keys are stable and documented here. Inner objects ",
                "(a run, an event, a room, an intent) are serialized graph-node state; their ",
                "fields are described where pinned and otherwise left open rather than guessed."
            )
        },
        "servers": [
            { "url": "http://localhost:50080", "description": "Local default (PORT defaults to 50080)" },
            {
                "url": "https://{host}",
                "description": "Deployed instance",
                "variables": { "host": { "default": "harness.example.com" } }
            }
        ],
        "tags": [
            { "name": "operations", "description": "Liveness and the OpenAPI document." },
            { "name": "runs", "description": "Harness run read contract (typed event log per run)." },
            { "name": "maps", "description": "Codebase map artifacts projected into the graph." },
            { "name": "rooms", "description": "Coordination rooms: presence, intents, records, messages, live stream." },
            { "name": "actors", "description": "Per-actor mention inbox." },
            { "name": "jobs", "description": "Dispatch v2 job board and Postgres dispatch counts." },
            { "name": "connectors", "description": "MCP connector registration and learnable tool affordances." }
        ],
        "paths": {
            "/healthz": {
                "get": {
                    "tags": ["operations"],
                    "summary": "Liveness probe",
                    "responses": {
                        "200": {
                            "description": "Server is up.",
                            "content": { "text/plain": { "schema": { "type": "string", "example": "ok" } } }
                        }
                    }
                }
            },
            "/openapi.json": {
                "get": {
                    "tags": ["operations"],
                    "summary": "This OpenAPI 3.1 document",
                    "responses": { "200": { "description": "The OpenAPI document for this server." } }
                }
            },
            "/harness/runs": {
                "get": {
                    "tags": ["runs"],
                    "summary": "List harness runs",
                    "responses": {
                        "200": {
                            "description": "All runs in the store (empty array if none yet).",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": { "runs": { "type": "array", "items": { "$ref": "#/components/schemas/Run" } } },
                                "required": ["runs"]
                            } } }
                        }
                    }
                }
            },
            "/harness/runs/{run_id}": {
                "get": {
                    "tags": ["runs"],
                    "summary": "Get one run with its ordered event log",
                    "parameters": [{ "$ref": "#/components/parameters/RunId" }],
                    "responses": {
                        "200": {
                            "description": "The run plus its append-ordered transition events.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "run": { "$ref": "#/components/schemas/Run" },
                                    "events": { "type": "array", "items": { "$ref": "#/components/schemas/Event" } }
                                },
                                "required": ["run", "events"]
                            } } }
                        },
                        "404": { "description": "Unknown run_id." }
                    }
                }
            },
            "/harness/maps": {
                "get": {
                    "tags": ["maps"],
                    "summary": "List codebase map artifacts for a tenant",
                    "parameters": [{ "$ref": "#/components/parameters/Tenant" }],
                    "responses": {
                        "200": {
                            "description": "Map artifacts for the tenant.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "maps": { "type": "array", "items": { "$ref": "#/components/schemas/MapArtifact" } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/maps/{map_id}": {
                "get": {
                    "tags": ["maps"],
                    "summary": "Get one codebase map artifact",
                    "parameters": [
                        { "$ref": "#/components/parameters/MapId" },
                        { "$ref": "#/components/parameters/Tenant" }
                    ],
                    "responses": {
                        "200": {
                            "description": "The map artifact, including its rendered markdown body.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "map": { "$ref": "#/components/schemas/MapArtifact" }
                                }
                            } } }
                        },
                        "404": { "description": "Unknown map_id, or map belongs to another tenant." }
                    }
                }
            },
            "/harness/rooms/{room_id}": {
                "get": {
                    "tags": ["rooms"],
                    "summary": "Get coordination room state",
                    "parameters": [
                        { "$ref": "#/components/parameters/RoomId" },
                        { "$ref": "#/components/parameters/Tenant" }
                    ],
                    "responses": {
                        "200": {
                            "description": "Room membership and task state.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "room_id": { "type": "string" },
                                    "room": { "type": "object", "additionalProperties": true }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/rooms/{room_id}/presence": {
                "get": {
                    "tags": ["rooms"],
                    "summary": "List member presence for a room's tenant",
                    "parameters": [
                        { "$ref": "#/components/parameters/RoomId" },
                        { "$ref": "#/components/parameters/Tenant" }
                    ],
                    "responses": {
                        "200": {
                            "description": "Presence entries.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "presence": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/rooms/{room_id}/intents": {
                "get": {
                    "tags": ["rooms"],
                    "summary": "List live intents (announced footprints) for a room",
                    "parameters": [
                        { "$ref": "#/components/parameters/RoomId" },
                        { "$ref": "#/components/parameters/Tenant" },
                        { "name": "status", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Single status filter (e.g. working, paused, done). Alias of `statuses`." },
                        { "name": "statuses", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Comma-separated status filter." }
                    ],
                    "responses": {
                        "200": {
                            "description": "Intent records, optionally filtered by status.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "room_id": { "type": "string" },
                                    "intents": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/rooms/{room_id}/records": {
                "get": {
                    "tags": ["rooms"],
                    "summary": "List durable room records (decisions, tensions, reflections, events)",
                    "parameters": [
                        { "$ref": "#/components/parameters/RoomId" },
                        { "$ref": "#/components/parameters/Tenant" },
                        { "name": "record_type", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Single record-type filter. Alias of `record_types`." },
                        { "name": "record_types", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Comma-separated record-type filter (event, decision, tension, reflection)." },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "default": 50 } }
                    ],
                    "responses": {
                        "200": {
                            "description": "Durable records, newest-bounded by limit.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "room_id": { "type": "string" },
                                    "records": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/rooms/{room_id}/messages": {
                "post": {
                    "tags": ["rooms"],
                    "summary": "Write a message to a room (and emit a push event)",
                    "description": "Writes a coordination message and publishes it on the in-process bus. `delivery: passive` is a tap (recorded, no spawn); `delivery: wake` is a hold the wake-listener can act on. Defaults to passive.",
                    "parameters": [{ "$ref": "#/components/parameters/RoomId" }],
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/MessagePost" } } }
                    },
                    "responses": {
                        "200": {
                            "description": "The written message plus the emitted room event.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "message": { "type": "object", "additionalProperties": true },
                                    "event": { "$ref": "#/components/schemas/RoomMessageEvent" }
                                }
                            } } }
                        },
                        "400": { "description": "Missing or empty actor_id or message." }
                    }
                }
            },
            "/harness/rooms/{room_id}/stream": {
                "get": {
                    "tags": ["rooms"],
                    "summary": "Subscribe to a room's messages over Server-Sent Events",
                    "description": "A long-lived SSE stream; each event is a JSON RoomMessageEvent. The `tenant` query parameter is required and scopes the subscription.",
                    "parameters": [
                        { "$ref": "#/components/parameters/RoomId" },
                        { "name": "tenant", "in": "query", "required": true, "schema": { "type": "string" } }
                    ],
                    "responses": {
                        "200": {
                            "description": "An event stream of room messages.",
                            "content": { "text/event-stream": { "schema": { "type": "string" } } }
                        },
                        "400": { "description": "Missing tenant query parameter." }
                    }
                }
            },
            "/harness/actors/{actor_id}/mentions": {
                "get": {
                    "tags": ["actors"],
                    "summary": "Read (and optionally consume) an actor's mention inbox",
                    "parameters": [
                        { "name": "actor_id", "in": "path", "required": true, "schema": { "type": "string" } },
                        { "$ref": "#/components/parameters/Tenant" },
                        { "name": "urgency", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Single urgency filter (info, ask, block). Alias of `urgencies`." },
                        { "name": "urgencies", "in": "query", "required": false, "schema": { "type": "string" },
                          "description": "Comma-separated urgency filter." },
                        { "name": "consume", "in": "query", "required": false, "schema": { "type": "boolean", "default": false },
                          "description": "If true, mark the returned mentions as read." },
                        { "name": "limit", "in": "query", "required": false, "schema": { "type": "integer", "default": 20 } }
                    ],
                    "responses": {
                        "200": {
                            "description": "Pending mentions for the actor.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "actor_id": { "type": "string" },
                                    "urgencies": { "type": "array", "items": { "type": "string" } },
                                    "mentions": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                                    "count": { "type": "integer" },
                                    "consumed": { "type": "boolean" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/harness/jobs": {
                "post": {
                    "tags": ["jobs"],
                    "summary": "Submit a dispatch job",
                    "description": "Creates or upserts a pending Job. The tenant must be supplied as `tenant`/`tenant_slug` unless the server has a non-default tenant env configured. Defaults: room -> repo:theorem:branch:main, submitted_by -> theorem-harness-server. If THEOREM_DISPATCH_DATABASE_URL is set, the job is mirrored into the Postgres dispatch queue and a wake event is emitted to the target head.",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/JobSubmitBody" } } }
                    },
                    "responses": {
                        "200": {
                            "description": "The created/updated job and the emitted wake event.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "room_id": { "type": "string" },
                                    "job_id": { "type": "string" },
                                    "created": { "type": "boolean" },
                                    "dispatch_mirrored": { "type": "boolean" },
                                    "job": { "$ref": "#/components/schemas/Job" },
                                    "wake_event": { "$ref": "#/components/schemas/RoomMessageEvent" }
                                }
                            } } }
                        },
                        "400": { "description": "Invalid submission." },
                        "502": { "description": "Dispatch queue (Postgres) unavailable." }
                    }
                }
            },
            "/harness/jobs/counts": {
                "get": {
                    "tags": ["jobs"],
                    "summary": "Inspect Postgres dispatch state counts",
                    "responses": {
                        "200": {
                            "description": "Dispatch state counts. If no dispatch database is configured, dispatch_configured is false and counts is empty.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "dispatch_configured": { "type": "boolean" },
                                    "counts": { "type": "array", "items": {
                                        "type": "object",
                                        "properties": { "state": { "type": "string" }, "count": { "type": "integer" } }
                                    } }
                                }
                            } } }
                        }
                    }
                }
            },
            "/connectors": {
                "get": {
                    "tags": ["connectors"],
                    "summary": "List registered MCP connectors and their tool affordances",
                    "description": "Read-only and fast; contacts no external server.",
                    "parameters": [{ "$ref": "#/components/parameters/Tenant" }],
                    "responses": {
                        "200": {
                            "description": "Registered connectors and the affordances learned from them.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "tenant": { "type": "string" },
                                    "connectors": { "type": "array", "items": { "type": "string" } },
                                    "affordances": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        }
                    }
                }
            },
            "/connectors/register": {
                "post": {
                    "tags": ["connectors"],
                    "summary": "Connect to an MCP server and register its tools as affordances",
                    "description": "Spawns the target MCP server, performs the initialize -> notifications/initialized -> tools/list handshake, and registers each tool as a learnable affordance node under (tenant, server_id). The connection target is persisted so a selected tool can be invoked later.",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RegisterConnectorBody" } } }
                    },
                    "responses": {
                        "200": {
                            "description": "The server info and the registered affordance ids.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "server": { "type": "object", "properties": {
                                        "name": { "type": "string" }, "version": { "type": "string" }, "protocol": { "type": "string" }
                                    } },
                                    "tenant": { "type": "string" },
                                    "server_id": { "type": "string" },
                                    "affordance_ids": { "type": "array", "items": { "type": "string" } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        },
                        "400": { "description": "Missing server_id." },
                        "502": { "description": "MCP server handshake failed." }
                    }
                }
            },
            "/connectors/register/content-core": {
                "post": {
                    "tags": ["connectors"],
                    "summary": "Register the content-core MCP server",
                    "description": "Uses the configured content-core stdio target (default `uvx content-core mcp`) and persists it as the `content-core` connector. The registered tools surface in the content_extraction affordance family.",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/RegisterContentCoreConnectorBody" } } }
                    },
                    "responses": {
                        "200": {
                            "description": "The content-core server info and registered affordance ids.",
                            "content": { "application/json": { "schema": {
                                "type": "object",
                                "properties": {
                                    "server": { "type": "object", "properties": {
                                        "name": { "type": "string" }, "version": { "type": "string" }, "protocol": { "type": "string" }
                                    } },
                                    "tenant": { "type": "string" },
                                    "server_id": { "type": "string", "example": "content-core" },
                                    "affordance_ids": { "type": "array", "items": { "type": "string" } },
                                    "count": { "type": "integer" }
                                }
                            } } }
                        },
                        "400": { "description": "Missing tenant." },
                        "502": { "description": "content-core MCP handshake failed." }
                    }
                }
            },
            "/github/webhook": {
                "post": {
                    "tags": ["operations"],
                    "summary": "GitHub App webhook (conditionally mounted)",
                    "description": "Only mounted when GitHub App env config is present. Verifies the X-Hub-Signature-256 HMAC, deduplicates by X-GitHub-Delivery, and ingests push/PR/issue/review events into the code graph. Also available at /github/webhooks. Omitted entirely when GitHub App config is unset.",
                    "requestBody": {
                        "required": true,
                        "content": { "application/json": { "schema": { "type": "object", "additionalProperties": true } } }
                    },
                    "responses": {
                        "202": { "description": "Accepted (possibly a duplicate delivery)." },
                        "400": { "description": "Missing signature, event, or delivery header." },
                        "401": { "description": "Invalid signature." }
                    }
                }
            }
        },
        "components": {
            "parameters": {
                "Tenant": {
                    "name": "tenant", "in": "query", "required": false,
                    "schema": { "type": "string", "example": "Travis-Gilbert" },
                    "description": "Tenant slug. Alias `tenant_slug` is also accepted. Required unless the server has a non-default tenant env configured."
                },
                "RunId": { "name": "run_id", "in": "path", "required": true, "schema": { "type": "string" } },
                "MapId": { "name": "map_id", "in": "path", "required": true, "schema": { "type": "string" } },
                "RoomId": {
                    "name": "room_id", "in": "path", "required": true, "schema": { "type": "string" },
                    "description": "e.g. repo:theorem:branch:main."
                }
            },
            "schemas": {
                "Run": {
                    "type": "object",
                    "description": "Serialized harness run state. Core fields shown; additional fields may be present.",
                    "properties": {
                        "run_id": { "type": "string" },
                        "task": { "type": "string" },
                        "actor": { "type": "string" },
                        "state_hash": { "type": "string" }
                    },
                    "additionalProperties": true
                },
                "Event": {
                    "type": "object",
                    "description": "One transition in a run's append-ordered event log.",
                    "properties": {
                        "type": { "type": "string", "description": "e.g. RUN.CREATED, HOST.OBSERVED" },
                        "state_hash_after": { "type": "string" }
                    },
                    "additionalProperties": true
                },
                "MapArtifact": {
                    "type": "object",
                    "description": "A codebase map projected into the graph.",
                    "properties": {
                        "map_id": { "type": "string" },
                        "map_kind": { "type": "string", "example": "CodebaseMap" },
                        "scope_kind": { "type": "string", "example": "repo" },
                        "scope_ref": { "type": "string" },
                        "repo_id": { "type": "string" },
                        "entry_count": { "type": "integer" },
                        "entries": { "type": "array", "items": { "type": "object", "additionalProperties": true } },
                        "markdown_body": { "type": "string" }
                    },
                    "additionalProperties": true
                },
                "MessagePost": {
                    "type": "object",
                    "required": ["actor_id", "message"],
                    "properties": {
                        "tenant_slug": { "type": "string", "example": "Travis-Gilbert" },
                        "actor_id": { "type": "string" },
                        "message": { "type": "string" },
                        "urgency": { "type": "string", "description": "Coordination urgency.", "enum": ["info", "ask", "block"] },
                        "delivery": { "type": "string", "enum": ["passive", "wake"], "default": "passive" },
                        "mentions": { "type": "array", "items": { "type": "string" } },
                        "metadata": { "type": "object", "additionalProperties": true }
                    }
                },
                "RoomMessageEvent": {
                    "type": "object",
                    "description": "A coordination room message event (also the SSE payload).",
                    "properties": {
                        "tenant_slug": { "type": "string" },
                        "room_id": { "type": "string" },
                        "message_id": { "type": "string" },
                        "author": { "type": "string" },
                        "urgency": { "type": "string" },
                        "message": { "type": "string" },
                        "mentions": { "type": "array", "items": { "type": "string" } },
                        "delivery": { "type": "string", "enum": ["Passive", "Wake"] }
                    },
                    "additionalProperties": true
                },
                "JobSubmission": {
                    "type": "object",
                    "required": ["title", "repo"],
                    "description": "Core dispatch job fields (flattened into JobSubmitBody).",
                    "properties": {
                        "job_id": { "type": "string", "description": "Omit to auto-mint a job- ULID." },
                        "title": { "type": "string" },
                        "repo": { "type": "string" },
                        "spec_ref": { "type": "string" },
                        "spec_inline": { "type": "string" },
                        "priority": { "type": "string", "enum": ["P0", "P1", "P2"], "default": "P2" },
                        "target_head": { "type": "string", "enum": ["claude", "codex", "either"], "default": "either" },
                        "not_before": { "type": "string", "format": "date-time" },
                        "source_task_id": { "type": "string" },
                        "source_project_id": { "type": "string" },
                        "idempotency_key": { "type": "string" }
                    },
                    "additionalProperties": true
                },
                "JobSubmitBody": {
                    "allOf": [
                        {
                            "type": "object",
                            "properties": {
                                "tenant": { "type": "string" },
                                "tenant_slug": { "type": "string" },
                                "submitted_by": { "type": "string" },
                                "room_id": { "type": "string" }
                            }
                        },
                        { "$ref": "#/components/schemas/JobSubmission" }
                    ]
                },
                "Job": {
                    "type": "object",
                    "description": "The persisted job thread.",
                    "properties": {
                        "job_id": { "type": "string" },
                        "title": { "type": "string" },
                        "repo": { "type": "string" },
                        "priority": { "type": "string", "enum": ["P0", "P1", "P2"] },
                        "target_head": { "type": "string", "enum": ["claude", "codex", "either"] }
                    },
                    "additionalProperties": true
                },
                "ConnectionTarget": {
                    "type": "object",
                    "description": "How to reach an MCP server. The stdio transport is shown.",
                    "required": ["type"],
                    "properties": {
                        "type": { "type": "string", "enum": ["stdio"] },
                        "command": { "type": "string" },
                        "args": { "type": "array", "items": { "type": "string" } },
                        "env": { "type": "object", "additionalProperties": { "type": "string" } }
                    },
                    "additionalProperties": true
                },
                "RegisterConnectorBody": {
                    "type": "object",
                    "required": ["server_id", "target"],
                    "properties": {
                        "tenant": { "type": "string", "example": "Travis-Gilbert" },
                        "server_id": { "type": "string" },
                        "label": { "type": "string", "description": "Defaults to server_id if empty." },
                        "target": { "$ref": "#/components/schemas/ConnectionTarget" }
                    }
                },
                "RegisterContentCoreConnectorBody": {
                    "type": "object",
                    "properties": {
                        "tenant": { "type": "string", "example": "Travis-Gilbert" },
                        "tenant_slug": { "type": "string", "example": "Travis-Gilbert" },
                        "label": { "type": "string", "description": "Defaults to Content Core." }
                    }
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openapi_document_is_well_formed() {
        let doc = openapi_document();
        assert_eq!(doc["openapi"], "3.1.0");
        let paths = doc["paths"].as_object().expect("paths object");
        for path in [
            "/healthz",
            "/openapi.json",
            "/harness/runs",
            "/harness/runs/{run_id}",
            "/harness/maps",
            "/harness/rooms/{room_id}/messages",
            "/harness/rooms/{room_id}/stream",
            "/harness/actors/{actor_id}/mentions",
            "/harness/jobs",
            "/connectors",
            "/connectors/register",
            "/connectors/register/content-core",
        ] {
            assert!(paths.contains_key(path), "missing documented path: {path}");
        }
    }

    /// The published snapshot in the docs site is generated from this function.
    /// If a route changes, regenerate it with `UPDATE_OPENAPI=1 cargo test`,
    /// otherwise this test fails and the docs cannot silently drift from code.
    #[test]
    fn committed_openapi_snapshot_matches_code() {
        let doc = openapi_document();
        let snapshot_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/site/reference/openapi-harness.json");

        if std::env::var("UPDATE_OPENAPI").is_ok() {
            let pretty = format!(
                "{}\n",
                serde_json::to_string_pretty(&doc).expect("serialize openapi document")
            );
            std::fs::write(&snapshot_path, pretty).expect("write openapi snapshot");
            return;
        }

        let committed = std::fs::read_to_string(&snapshot_path).expect(
            "committed docs/site/reference/openapi-harness.json missing; \
             generate it with `UPDATE_OPENAPI=1 cargo test`",
        );
        let committed: Value =
            serde_json::from_str(&committed).expect("committed openapi snapshot is valid JSON");
        assert_eq!(
            doc, committed,
            "openapi-harness.json is stale; regenerate with `UPDATE_OPENAPI=1 cargo test`"
        );
    }
}
