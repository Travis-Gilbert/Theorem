//! SPEC-2 deliverables 3 + 4: the live Item changefeed.
//!
//! A graph-level hook (deliverable 3) projects every Item-relevant mutation
//! through the SAME [`rustyred_thg_mcp::projection::project_event`] the GraphQL
//! `items` resolver uses, and publishes the resulting `ItemDelta` to a process-
//! global broadcast bus. This module owns that bus, the hook registration that
//! feeds it, and the SSE endpoint that tails it (deliverable 4):
//!
//! * `GET /v1/items/stream?tenant=` -> a Server-Sent-Events tail of `ItemDelta`
//!   JSON, filtered to one tenant. The CommonPlace Auto-Organizer hydrates
//!   initial state through the `items` GraphQL query (deliverable 2), then tails
//!   this stream and applies each delta as it arrives.
//!
//! The sync-hook -> async-SSE seam is crossed by the broadcast channel, never by
//! sharing the non-`Send` MCP backend: the hook (on the std::thread hook
//! dispatcher) does a non-blocking `Sender::send`; the SSE handler (on tokio)
//! tails a `Receiver`. This is the one sync-to-async crossing SPEC-2 calls out,
//! and it is why the changefeed is a separate streaming endpoint rather than a
//! GraphQL subscription on the synchronous MCP transport.
//!
//! Scope: `graph:read`. Enabled by `THEOREM_ITEM_CHANGEFEED` (default off, so a
//! deploy is a deliberate flag flip, matching `THEOREM_GRAPH_HOOKS`).

use std::convert::Infallible;
use std::sync::{Arc, OnceLock};

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse,
    },
    Json,
};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::broadcast;
use tokio_stream::{wrappers::BroadcastStream, StreamExt};

use rustyred_thg_core::hooks::{
    coalesce_per_id, HookContext, HookOutcome, HookRegistration, MutationEvent, MutationKind,
    MutationMatcher,
};
use rustyred_thg_mcp::projection::{project_event, ItemChange, ItemDelta, PROJECTED_LABELS};

use crate::auth::require_scope;
use crate::router::mcp_origin_allowed;
use crate::state::AppState;

const ITEMS_SCOPE: &str = "graph:read";
/// Bounded so a slow SSE consumer cannot grow the bus unboundedly; a lagging
/// receiver gets `Lagged` and re-hydrates through the `items` query.
const CHANGEFEED_CAPACITY: usize = 1024;

/// The process-global Item-delta bus. The per-tenant hook dispatchers all publish
/// here; each `ItemDelta` carries its own `tenant`, so the SSE endpoint filters.
static CHANGEFEED: OnceLock<broadcast::Sender<ItemDelta>> = OnceLock::new();

fn bus() -> &'static broadcast::Sender<ItemDelta> {
    CHANGEFEED.get_or_init(|| broadcast::channel(CHANGEFEED_CAPACITY).0)
}

/// A sender clone for a hook handler to publish on.
pub fn sender() -> broadcast::Sender<ItemDelta> {
    bus().clone()
}

/// A receiver for an SSE tail (or a test).
pub fn subscribe() -> broadcast::Receiver<ItemDelta> {
    bus().subscribe()
}

/// Whether the Item changefeed dispatcher is enabled. Default off (a deliberate
/// flag flip), truthy on `1`/`true`/`on`/`yes`.
pub fn item_changefeed_enabled() -> bool {
    std::env::var("THEOREM_ITEM_CHANGEFEED")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

/// The changefeed hook registration: matches an upsert or delete of any projected
/// label, re-reads the node (for an upsert), projects it through the single
/// projection, and publishes the delta to the bus. It obeys the hook contract
/// unchanged: post-commit, coalesced per id, fail-open (a `send` to a closed bus
/// is ignored), and it makes NO graph writes, so the loop guard never engages.
pub fn changefeed_registration() -> HookRegistration {
    let sender = sender();
    let handler = Arc::new(
        move |ctx: &mut HookContext, events: &[MutationEvent]| {
            for event in events {
                let node = match event.kind {
                    // A delete has no node to re-read; the delta is a tombstone.
                    MutationKind::NodeDeleted => None,
                    // Re-read the just-committed node for its properties. A read
                    // error skips THIS event without aborting the batch.
                    _ => match ctx.store.get_node(&event.id) {
                        Ok(node) => node,
                        Err(_) => continue,
                    },
                };
                if let Some(delta) = project_event(event, node.as_ref()) {
                    // Non-blocking; `Err` only means no live subscribers, which is
                    // fine (a fresh subscriber re-hydrates through the items query).
                    let _ = sender.send(delta);
                }
            }
            Ok(HookOutcome::Done)
        },
    );
    HookRegistration::new(
        "item-changefeed",
        MutationMatcher::any()
            .with_labels(PROJECTED_LABELS)
            .with_kinds([MutationKind::NodeUpserted, MutationKind::NodeDeleted]),
        coalesce_per_id,
        handler,
    )
}

#[derive(Debug, Default, Deserialize)]
pub struct ItemsStreamQuery {
    #[serde(default)]
    pub tenant: Option<String>,
}

/// Shared request guard: MCP enabled, origin allowed, scope satisfied.
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
        ITEMS_SCOPE,
        state.config.require_auth,
    ) {
        return Some(status.into_response());
    }
    None
}

fn change_event_name(change: ItemChange) -> &'static str {
    match change {
        ItemChange::Upserted => "item.upserted",
        ItemChange::Deleted => "item.deleted",
    }
}

fn stream_for_tenant(tenant: String) -> axum::response::Response {
    let stream = BroadcastStream::new(subscribe()).filter_map(move |event| {
        let delta = event.ok()?;
        if delta.tenant != tenant {
            return None;
        }
        let kind = change_event_name(delta.change);
        let sse_event = Event::default()
            .event(kind)
            .json_data(&delta)
            .unwrap_or_else(|_| Event::default().event(kind).data("{}"));
        Some(Ok::<Event, Infallible>(sse_event))
    });

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// `GET /v1/items/stream?tenant=` -- SSE tail of Item deltas for one tenant. A
/// `tenant` is required: the bus is process-global across tenants, so streaming
/// without one would leak other tenants' Items.
pub async fn items_stream(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<ItemsStreamQuery>,
) -> impl IntoResponse {
    if let Some(response) = guard(&state, &headers) {
        return response;
    }
    let tenant = match query.tenant.as_deref().map(str::trim) {
        Some(tenant) if !tenant.is_empty() => tenant.to_string(),
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({
                    "error": "missing_tenant",
                    "message": "the items changefeed requires a `tenant` query parameter"
                })),
            )
                .into_response()
        }
    };

    stream_for_tenant(tenant)
}

/// Back-compat route for the main-line path shape:
/// `GET /v1/tenants/:tenant_id/items/events`.
pub async fn tenant_items_events(
    Path(tenant_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(response) = guard(&state, &headers) {
        return response;
    }
    let tenant = tenant_id.trim();
    if tenant.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({
                "error": "missing_tenant",
                "message": "the items changefeed requires a tenant"
            })),
        )
            .into_response();
    }
    stream_for_tenant(tenant.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_matches_projected_labels_not_other_nodes() {
        let reg = changefeed_registration();
        assert_eq!(reg.name, "item-changefeed");
        let task = MutationEvent::new(
            MutationKind::NodeUpserted,
            "t",
            "task-1",
            vec!["TaskNode".to_string()],
            vec![],
            0,
            0,
        );
        assert!(reg.on.matches(&task), "matches a projected TaskNode upsert");
        let sym = MutationEvent::new(
            MutationKind::NodeUpserted,
            "t",
            "sym-1",
            vec!["CodeSymbol".to_string()],
            vec![],
            0,
            0,
        );
        assert!(!reg.on.matches(&sym), "ignores a non-projected node");
    }

    #[test]
    fn bus_delivers_published_deltas_to_subscribers() {
        let mut rx = subscribe();
        sender()
            .send(ItemDelta {
                change: ItemChange::Deleted,
                id: "bus-test-id".to_string(),
                tenant: "tenant-bus".to_string(),
                item: None,
            })
            .ok();
        // Parallel tests share the global bus; find ours by id.
        let mut found = false;
        loop {
            match rx.try_recv() {
                Ok(delta) => {
                    if delta.id == "bus-test-id" {
                        found = true;
                        break;
                    }
                }
                Err(broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        assert!(found, "subscriber received the published delta");
    }
}
