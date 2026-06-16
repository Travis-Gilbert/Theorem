//! JSON/HTTP transport handlers over `theorem-harness-runtime`.
//!
//! The two read shapes the Theorem clients consume (see
//! docs/plans/harness-rust-port/ios-transport-handoff.md):
//!   - `runs_json`  -> { "runs":   [ <run node properties> ] }
//!   - `run_json`   -> { "run": <RunState>, "events": [ <EventState> ] }
//!   - coordination read helpers -> presence, intents, room status, mentions
//!
//! Kept as pure functions over `GraphStore` so they are unit-testable without a
//! live server; `main.rs` is a thin Axum shell that calls them.
//!
//! Push (docs/plans/coordination-room-push) lives in [`push`]: the room write
//! endpoint, the in-process emit, the SSE stream the app subscribes to, and the
//! spawn-listener that wakes agents on `delivery = wake` messages.

pub mod github;
pub mod github_app;
pub mod push;

pub use github::{github_router, verify_webhook_signature, GithubWebhookState};
pub use github_app::{GithubApp, GithubAppError, InstallationToken};
pub use push::{
    push_router, spawn_wake_listener, CommandSpawnDispatcher, Delivery, MessagePost, PushState,
    RoomBus, RoomMessageEvent, SpawnDispatcher, SpawnOutcome, DEFAULT_BUS_CAPACITY,
};

use rustyred_thg_affordances::{affordance_nodes, AffordanceGraphStore};
use rustyred_thg_core::{GraphStore, NodeQuery};
use serde_json::{json, Value};
use theorem_harness_runtime::{
    list_presence, load_events, load_run, read_intents_for_room, read_mentions_for_actor,
    read_records_for_room, room_status, CoordinationError, HarnessRuntimeError,
};

/// Node label the runtime persists run state under (`event_log::run_node`).
pub const RUN_LABEL: &str = "HarnessRun";

/// List runs as the client list contract. Each run node's properties are the
/// `RunState` serde shape plus `state_hash`.
pub fn runs_json<S: GraphStore>(store: &S) -> Value {
    let query = NodeQuery {
        label: Some(RUN_LABEL.to_string()),
        properties: Default::default(),
        limit: None,
        include_expired: false,
    };
    let nodes = GraphStore::query_nodes(store, query);
    let runs: Vec<Value> = nodes.into_iter().map(|node| node.properties).collect();
    json!({ "runs": runs })
}

/// One run plus its ordered event log as the client detail contract. Returns
/// `None` when the run is unknown (the handler maps that to 404).
pub fn run_json<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> Result<Option<Value>, HarnessRuntimeError> {
    match load_run(store, run_id)? {
        None => Ok(None),
        Some(run) => {
            let events = load_events(store, run_id)?;
            Ok(Some(json!({ "run": run, "events": events })))
        }
    }
}

/// Room membership/task state for the coordination view.
pub fn room_json<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
) -> Result<Value, CoordinationError> {
    Ok(json!({
        "tenant": tenant_slug,
        "room_id": room_id,
        "room": room_status(store, tenant_slug, room_id)?
    }))
}

/// Fresh actor presence for the Participants surface.
pub fn presence_json<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
) -> Result<Value, CoordinationError> {
    let presence = list_presence(store, tenant_slug)?;
    Ok(json!({
        "tenant": tenant_slug,
        "presence": presence,
        "count": presence.len()
    }))
}

/// Live room intents. Empty `statuses` means all statuses.
pub fn intents_json<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
    statuses: &[String],
) -> Result<Value, CoordinationError> {
    let intents = read_intents_for_room(store, tenant_slug, room_id, statuses)?;
    Ok(json!({
        "tenant": tenant_slug,
        "room_id": room_id,
        "intents": intents,
        "count": intents.len()
    }))
}

/// Actor mention inbox. `consume=true` updates the underlying message records.
pub fn mentions_json<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    actor_id: &str,
    consume: bool,
    limit: usize,
) -> Result<Value, CoordinationError> {
    let mentions = read_mentions_for_actor(store, tenant_slug, actor_id, consume, limit)?;
    Ok(json!({
        "tenant": tenant_slug,
        "actor_id": actor_id,
        "mentions": mentions,
        "count": mentions.len(),
        "consumed": consume
    }))
}

/// Durable room records: events, decisions, tensions, and reflections.
pub fn records_json<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
    record_types: &[String],
    limit: usize,
) -> Result<Value, CoordinationError> {
    let records = read_records_for_room(store, tenant_slug, room_id, record_types, limit)?;
    Ok(json!({
        "tenant": tenant_slug,
        "room_id": room_id,
        "records": records,
        "count": records.len()
    }))
}

/// Registered connectors and their tool affordances for the Connectors surface.
/// Read-only and fast (no subprocess, no network): lists the tenant's `Affordance`
/// nodes (one per registered tool) plus the distinct owning servers. Safe to serve
/// under the shared store lock, unlike the register path which spawns a server.
/// Errors degrade to an empty listing (a fresh store legitimately has none).
pub fn connectors_json<S: AffordanceGraphStore>(store: &S, tenant_slug: &str) -> Value {
    let affordances: Vec<Value> = affordance_nodes(store)
        .unwrap_or_default()
        .into_iter()
        .filter(|node| {
            node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_slug)
        })
        .map(|node| {
            let p = node.properties;
            json!({
                "affordance_id": p.get("affordance_id"),
                "server_id": p.get("server_id"),
                "tool_name": p.get("tool_name"),
                "label": p.get("label"),
                "description": p.get("description"),
                "writeback_policy": p.get("writeback_policy"),
                "fitness": p.get("fitness"),
            })
        })
        .collect();
    let mut servers: Vec<String> = affordances
        .iter()
        .filter_map(|a| {
            a.get("server_id")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .collect();
    servers.sort();
    servers.dedup();
    json!({
        "tenant": tenant_slug,
        "connectors": servers,
        "affordances": affordances,
        "count": affordances.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::Map;
    use theorem_harness_core::TransitionInput;
    use theorem_harness_runtime::{
        append_transition_from_store, heartbeat_presence, join_room, write_intent, write_message,
        write_record, JoinRoomInput, PresenceInput, WriteIntentInput, WriteMessageInput,
        WriteRecordInput,
    };

    fn payload(pairs: &[(&str, Value)]) -> Map<String, Value> {
        let mut map = Map::new();
        for (key, value) in pairs {
            map.insert((*key).to_string(), value.clone());
        }
        map
    }

    fn seed_run(store: &mut InMemoryGraphStore) {
        let created = TransitionInput::new(
            "RUN.CREATED",
            payload(&[
                ("task", json!("port harness")),
                ("actor", json!("claude-code")),
            ]),
        )
        .with_run_id("run-http-test");
        append_transition_from_store(store, created).expect("RUN.CREATED");

        let observed = TransitionInput::new(
            "HOST.OBSERVED",
            payload(&[
                ("repo", json!("Theorem")),
                ("branch", json!("main")),
                ("commit_sha", json!("deadbeef")),
                ("cwd", json!("/repo")),
            ]),
        )
        .with_run_id("run-http-test");
        append_transition_from_store(store, observed).expect("HOST.OBSERVED");
    }

    fn seed_coordination(store: &mut InMemoryGraphStore) {
        join_room(
            store,
            JoinRoomInput {
                tenant_slug: "smoke".to_string(),
                actor_id: "codex".to_string(),
                room_id: "repo:theorem:branch:main".to_string(),
                repo: "Theorem".to_string(),
                branch: "main".to_string(),
                task: "transport coordination".to_string(),
                updated_at: "2026-06-01T16:00:00Z".to_string(),
                ..JoinRoomInput::default()
            },
        )
        .expect("join room");
        heartbeat_presence(
            store,
            PresenceInput {
                tenant_slug: "smoke".to_string(),
                actor_id: "codex".to_string(),
                status: "active".to_string(),
                refreshed_at: "2026-06-01T16:01:00Z".to_string(),
                ..PresenceInput::default()
            },
        )
        .expect("presence");
        write_intent(
            store,
            WriteIntentInput {
                tenant_slug: "smoke".to_string(),
                room_id: "repo:theorem:branch:main".to_string(),
                actor_id: "codex".to_string(),
                status: "working".to_string(),
                summary: "Expose coordination HTTP endpoints".to_string(),
                footprint: vec!["apps/theorem-harness-server/src/lib.rs".to_string()],
                updated_at: "2026-06-01T16:02:00Z".to_string(),
                ..WriteIntentInput::default()
            },
        )
        .expect("intent");
        write_message(
            store,
            WriteMessageInput {
                tenant_slug: "smoke".to_string(),
                room_id: "repo:theorem:branch:main".to_string(),
                actor_id: "codex".to_string(),
                urgency: "ask".to_string(),
                message: "@claude-code please verify the HTTP transport".to_string(),
                created_at: "2026-06-01T16:03:00Z".to_string(),
                ..WriteMessageInput::default()
            },
        )
        .expect("message");
        write_record(
            store,
            WriteRecordInput {
                tenant_slug: "smoke".to_string(),
                room_id: "repo:theorem:branch:main".to_string(),
                actor_id: "codex".to_string(),
                record_type: "decision".to_string(),
                title: "Expose read endpoints".to_string(),
                summary: "Use HTTP for participant state".to_string(),
                created_at: "2026-06-01T16:04:00Z".to_string(),
                ..WriteRecordInput::default()
            },
        )
        .expect("record");
    }

    #[test]
    fn serves_list_and_detail_in_client_contract() {
        let mut store = InMemoryGraphStore::default();
        seed_run(&mut store);

        let list = runs_json(&store);
        let runs = list["runs"].as_array().expect("runs array");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["run_id"], json!("run-http-test"));
        assert_eq!(runs[0]["task"], json!("port harness"));

        let detail = run_json(&store, "run-http-test")
            .expect("load ok")
            .expect("run present");
        assert_eq!(detail["run"]["run_id"], json!("run-http-test"));
        let events = detail["events"].as_array().expect("events array");
        assert_eq!(events.len(), 2);
        assert_eq!(events[0]["type"], json!("RUN.CREATED"));
        assert_eq!(events[1]["type"], json!("HOST.OBSERVED"));
        assert!(events[1]["state_hash_after"].as_str().is_some());

        assert!(run_json(&store, "unknown-run").expect("load ok").is_none());
    }

    #[test]
    fn serves_coordination_contracts() {
        let mut store = InMemoryGraphStore::default();
        seed_coordination(&mut store);

        let room =
            room_json(&store, "smoke", "repo:theorem:branch:main").expect("room status json");
        assert_eq!(room["room"]["members"]["codex"]["status"], json!("joined"));

        let presence = presence_json(&store, "smoke").expect("presence json");
        assert_eq!(presence["count"], json!(1));
        assert_eq!(presence["presence"][0]["actor_id"], json!("codex"));

        let intents =
            intents_json(&store, "smoke", "repo:theorem:branch:main", &[]).expect("intents json");
        assert_eq!(intents["count"], json!(1));
        assert_eq!(
            intents["intents"][0]["summary"],
            json!("Expose coordination HTTP endpoints")
        );

        let mentions =
            mentions_json(&mut store, "smoke", "claude-code", false, 20).expect("mentions json");
        assert_eq!(mentions["count"], json!(1));
        assert_eq!(mentions["mentions"][0]["actor_id"], json!("codex"));

        let consumed =
            mentions_json(&mut store, "smoke", "claude-code", true, 20).expect("consume mentions");
        assert_eq!(consumed["count"], json!(1));

        let empty_after_consume =
            mentions_json(&mut store, "smoke", "claude-code", false, 20).expect("mentions empty");
        assert_eq!(empty_after_consume["count"], json!(0));

        let records =
            records_json(&store, "smoke", "repo:theorem:branch:main", &[], 20).expect("records");
        assert_eq!(records["count"], json!(1));
        assert_eq!(records["records"][0]["record_type"], json!("decision"));

        let filtered = records_json(
            &store,
            "smoke",
            "repo:theorem:branch:main",
            &["reflection".to_string()],
            20,
        )
        .expect("filtered records");
        assert_eq!(filtered["count"], json!(0));
    }

    #[test]
    fn serves_connectors_contract() {
        use rustyred_thg_affordances::{register_connector, ConnectorManifest};

        let mut store = InMemoryGraphStore::default();
        // Build the manifest via JSON so the test does not break if the charter
        // work adds serde-defaulted fields to the manifest structs.
        let manifest: ConnectorManifest = serde_json::from_value(json!({
            "tenant_id": "smoke",
            "server_id": "websearch",
            "label": "Web Search",
            "tools": [
                { "name": "search", "description": "Search the web", "input_schema": {} }
            ]
        }))
        .expect("manifest");
        register_connector(&mut store, manifest, Some("operator")).expect("register");

        let listing = connectors_json(&store, "smoke");
        assert_eq!(listing["count"], json!(1));
        assert_eq!(listing["connectors"][0], json!("websearch"));
        assert_eq!(listing["affordances"][0]["tool_name"], json!("search"));
        assert_eq!(listing["affordances"][0]["server_id"], json!("websearch"));

        // A different tenant sees nothing (tenant scoping).
        assert_eq!(connectors_json(&store, "other")["count"], json!(0));
    }
}
