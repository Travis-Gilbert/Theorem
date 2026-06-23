//! Agent Space Viewport transport.
//!
//! A live HTTP window into the agent coordination space: agents, rooms, the
//! multihead work graph, and their edits. It rides the existing coordination
//! broadcast bus (widened to `AgentSpaceEvent` in `theorem-harness-runtime`)
//! and the existing MCP read surfaces, exposing two routes:
//!
//! * `GET /v1/agent-space/snapshot?tenant=&room=` -> a point-in-time seed
//!   (room status + presence + work-graph status) plus a `cursor`.
//! * `GET /v1/agent-space/stream?tenant=&room=&since=` -> a Server-Sent-Events
//!   tail of every agent-space event, each frame stamped with a monotonic
//!   `seq`. The client backfills from the snapshot, then tails the stream and
//!   drops `seq <= cursor` so nothing is double-applied across the boundary.
//!
//! Scope: `coordination:read`.

use std::convert::Infallible;

use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use serde::Deserialize;
use serde_json::{json, Value};
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use rustyred_thg_mcp::{handle_mcp_request_with_context, McpRequestContext};
use theorem_harness_runtime::{
    agent_space_event_kind, agent_space_event_matches, agent_space_high_water_seq,
    subscribe_agent_space_events,
};

use crate::auth::require_scope;
use crate::router::mcp_origin_allowed;
use crate::state::AppState;

const AGENT_SPACE_SCOPE: &str = "coordination:read";

#[derive(Debug, Default, Deserialize)]
pub struct AgentSpaceQuery {
    #[serde(default)]
    pub tenant: Option<String>,
    #[serde(default)]
    pub room: Option<String>,
    /// Cursor from a prior snapshot. The stream drops every frame with
    /// `seq <= since`, so a client that opened with the snapshot's cursor never
    /// re-applies an event the snapshot already reflected.
    #[serde(default)]
    pub since: Option<u64>,
}

/// Shared request guards for both agent-space routes. Returns `Some(response)`
/// when the request must be rejected, `None` when it may proceed.
fn guard(state: &AppState, headers: &HeaderMap) -> Option<axum::response::Response> {
    if !state.config.mcp_enabled {
        return Some(StatusCode::NOT_FOUND.into_response());
    }
    if !mcp_origin_allowed(headers, &state.config.allowed_origins) {
        return Some(StatusCode::FORBIDDEN.into_response());
    }
    if let Err(status) = require_scope(
        headers,
        &state.config.api_tokens,
        AGENT_SPACE_SCOPE,
        state.config.require_auth,
    ) {
        return Some(status.into_response());
    }
    None
}

/// `GET /v1/agent-space/stream` -- SSE tail of agent-space events filtered by
/// tenant (and optionally room), de-duped against a snapshot via `since`.
pub async fn agent_space_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AgentSpaceQuery>,
) -> impl IntoResponse {
    if let Some(response) = guard(&state, &headers) {
        return response;
    }

    let tenant = query.tenant.clone();
    let room = query.room.clone();
    let since = query.since.unwrap_or(0);

    let stream = BroadcastStream::new(subscribe_agent_space_events()).filter_map(move |event| {
        let envelope = event.ok()?;
        // Backfill de-dupe: anything the snapshot already reflected is dropped.
        if envelope.seq <= since {
            return None;
        }
        // Tenant/room routing. A stream without a tenant is a firehose (all
        // tenants); with a tenant it is scoped, optionally to one room.
        if let Some(tenant) = tenant.as_deref() {
            if !agent_space_event_matches(&envelope, tenant, room.as_deref()) {
                return None;
            }
        }
        let kind = agent_space_event_kind(&envelope.event);
        let sse_event = Event::default()
            .event(kind)
            .json_data(&envelope)
            .unwrap_or_else(|_| Event::default().event(kind).data("{}"));
        Some(Ok::<Event, Infallible>(sse_event))
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `GET /v1/agent-space/snapshot` -- a point-in-time seed for the scene plus a
/// `cursor`. The client applies this, then opens the stream with
/// `since=cursor`.
pub async fn agent_space_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<AgentSpaceQuery>,
) -> impl IntoResponse {
    if let Some(response) = guard(&state, &headers) {
        return response;
    }

    let tenant = match query.tenant.as_deref() {
        Some(tenant) if !tenant.trim().is_empty() => tenant.trim().to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "missing_tenant",
                    "message": "the agent-space snapshot requires a `tenant` query parameter"
                })),
            )
                .into_response()
        }
    };
    let room_id = query
        .room
        .as_deref()
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .map(str::to_string);

    // Compose the body from the existing MCP read surfaces, then read the
    // cursor AFTER the body. Reading the high-water seq last guarantees the
    // hard acceptance criterion -- no event is double-applied across the
    // snapshot/stream boundary -- because anything reflected in the body has
    // `seq <= cursor` and is dropped by the stream. The cost is a negligible,
    // self-healing gap (an event that lands mid-compose), which the next event
    // or settle reconciles.
    let room = snapshot_tool(
        &state,
        "coordination_room",
        json!({ "tenant": tenant, "room_id": room_id, "action": "status" }),
    );
    let work_graph = snapshot_tool(&state, "harness_kg_status", json!({ "tenant": tenant }));
    let presence = room
        .pointer("/room/members")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let cursor = agent_space_high_water_seq();

    Json(json!({
        "tenant": tenant,
        "room_id": room_id,
        "cursor": cursor,
        "room": room,
        "presence": presence,
        "work_graph": work_graph,
    }))
    .into_response()
}

/// Call a read-only MCP tool through the in-process dispatch (the same path the
/// coordination wake listener uses) and return its structured payload.
fn snapshot_tool(state: &AppState, name: &str, arguments: Value) -> Value {
    let response = handle_mcp_request_with_context(
        state,
        &state.mcp_config(),
        &McpRequestContext::with_scopes(["coordination:read", "graph:read"]),
        json!({
            "jsonrpc": "2.0",
            "id": name,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    );
    response
        .pointer("/result/structuredContent")
        .cloned()
        .unwrap_or(response)
}
