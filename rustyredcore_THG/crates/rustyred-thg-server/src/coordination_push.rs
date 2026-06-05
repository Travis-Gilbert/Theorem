use rustyred_thg_mcp::{handle_mcp_request_with_context, McpRequestContext};
use serde_json::{json, Value};
use theorem_harness_runtime::{subscribe_coordination_room_events, wake_targets, RoomMessageEvent};

use crate::state::AppState;

#[derive(Clone, Debug)]
struct WakeDispatchOutcome {
    actor: String,
    response: Value,
}

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
            if !event.delivery.is_wake() {
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
            let dispatch_state = state.clone();
            let dispatch_event = event.clone();
            match tokio::task::spawn_blocking(move || {
                dispatch_wake_event(&dispatch_state, &dispatch_event)
            })
            .await
            {
                Ok(outcomes) => {
                    for outcome in outcomes {
                        if outcome.response.get("error").is_some() {
                            tracing::warn!(
                                actor = %outcome.actor,
                                room_id = %event.room_id,
                                message_id = %event.message_id,
                                response = %outcome.response,
                                "coordination wake dispatch failed"
                            );
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(
                        room_id = %event.room_id,
                        message_id = %event.message_id,
                        error = %error,
                        "coordination wake dispatch task failed"
                    );
                }
            }
        }
    })
}

fn dispatch_wake_event(state: &AppState, event: &RoomMessageEvent) -> Vec<WakeDispatchOutcome> {
    if !wake_author_allowed(&event.author) {
        return vec![WakeDispatchOutcome {
            actor: event.author.clone(),
            response: json!({
                "error": "wake_author_not_trusted",
                "message": "coordination wake author is not in the trusted wake-author allowlist"
            }),
        }];
    }
    let room_agents = if event.mentions.is_empty() {
        room_members(state, event)
    } else {
        Vec::new()
    };
    wake_targets(event, &room_agents)
        .into_iter()
        .map(|actor| WakeDispatchOutcome {
            response: spawn_target_actor(state, event, &actor),
            actor,
        })
        .collect()
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
        McpRequestContext::with_scopes(["coordination:read"]),
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
                "wake_delivery": event.delivery.as_str()
            },
            "required_scopes": ["coordination:wake"]
        }),
        McpRequestContext::with_scopes(["coordination:wake"]),
    )
}

fn call_mcp_tool(
    state: &AppState,
    name: &str,
    arguments: Value,
    context: McpRequestContext,
) -> Value {
    handle_mcp_request_with_context(
        state,
        &state.mcp_config(),
        &context,
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

fn wake_author_allowed(author: &str) -> bool {
    let config = std::env::var("THEOREM_COORDINATION_WAKE_ALLOWED_AUTHORS")
        .ok()
        .or_else(|| std::env::var("THEOREM_WAKE_ALLOWED_AUTHORS").ok());
    wake_author_allowed_by_config(author, config.as_deref())
}

fn wake_author_allowed_by_config(author: &str, csv: Option<&str>) -> bool {
    let author = author.trim();
    if author.is_empty() {
        return false;
    }
    let configured = csv
        .map(str::to_string)
        .unwrap_or_else(|| "travis,codex,claude-code,claude-ai".to_string());
    configured
        .split(',')
        .map(str::trim)
        .any(|allowed| allowed == "*" || allowed.eq_ignore_ascii_case(author))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wake_author_allowlist_defaults_to_known_trusted_actors() {
        assert!(wake_author_allowed_by_config("travis", None));
        assert!(wake_author_allowed_by_config("codex", None));
        assert!(wake_author_allowed_by_config("claude-code", None));
        assert!(!wake_author_allowed_by_config("drive-by", None));
        assert!(!wake_author_allowed_by_config("", None));
    }

    #[test]
    fn wake_author_allowlist_accepts_explicit_csv_or_wildcard() {
        assert!(wake_author_allowed_by_config(
            "operator",
            Some("travis, operator")
        ));
        assert!(wake_author_allowed_by_config("anyone", Some("*")));
        assert!(!wake_author_allowed_by_config("other", Some("operator")));
    }
}
