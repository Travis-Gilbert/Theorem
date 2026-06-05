use rustyred_thg_mcp::{
    handle_mcp_request_with_context, subscribe_coordination_room_events, McpRequestContext,
    RoomMessageEvent,
};
use serde_json::{json, Value};

use crate::state::AppState;

pub fn spawn_wake_listener(state: AppState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut events = subscribe_coordination_room_events();
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "coordination wake listener lagged");
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if event.delivery != "wake" {
                continue;
            }
            if state.config.mcp_read_only {
                tracing::warn!(
                    room_id = %event.room_id,
                    message_id = %event.message_id,
                    "coordination wake skipped because MCP write mode is disabled"
                );
                continue;
            }
            let actors = wake_targets(&state, &event);
            for actor in actors {
                let response = spawn_target_actor(&state, &event, &actor);
                if response.get("error").is_some() {
                    tracing::warn!(
                        actor = %actor,
                        room_id = %event.room_id,
                        message_id = %event.message_id,
                        response = %response,
                        "coordination wake dispatch failed"
                    );
                }
            }
        }
    })
}

fn wake_targets(state: &AppState, event: &RoomMessageEvent) -> Vec<String> {
    let mut actors = if event.mentions.is_empty() {
        room_members(state, event)
    } else {
        event.mentions.clone()
    };
    actors.sort();
    actors.dedup();
    actors.retain(|actor| !actor.trim().is_empty() && actor != &event.author);
    actors
}

fn room_members(state: &AppState, event: &RoomMessageEvent) -> Vec<String> {
    let response = call_mcp_tool(
        state,
        "coordination_room",
        json!({
            "tenant": event.tenant_slug,
            "room_id": event.room_id,
            "action": "status"
        }),
    );
    response["result"]["structuredContent"]["room"]["members"]
        .as_object()
        .map(|members| members.keys().cloned().collect())
        .unwrap_or_default()
}

fn spawn_target_actor(state: &AppState, event: &RoomMessageEvent, actor: &str) -> Value {
    call_mcp_tool(
        state,
        "spawn_session",
        json!({
            "tenant": event.tenant_slug,
            "room_id": event.room_id,
            "actor": actor,
            "branch": "main",
            "intent": format!(
                "Wake requested by coordination room message {} from {}. Read the room message and pending mentions before acting.",
                event.message_id, event.author
            ),
            "metadata": {
                "wake_message_id": event.message_id,
                "wake_author": event.author,
                "wake_delivery": event.delivery
            },
            "required_scopes": ["*"]
        }),
    )
}

fn call_mcp_tool(state: &AppState, name: &str, arguments: Value) -> Value {
    handle_mcp_request_with_context(
        state,
        &state.mcp_config(),
        &McpRequestContext::with_scopes(["*"]),
        json!({
            "jsonrpc": "2.0",
            "id": name,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        }),
    )
}
