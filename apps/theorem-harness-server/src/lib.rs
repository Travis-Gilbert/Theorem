#![recursion_limit = "512"]
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
pub mod openapi;
pub mod push;

pub use github::{github_router, verify_webhook_signature, GithubWebhookState};
pub use github_app::{GithubApp, GithubAppError, InstallationToken};
pub use openapi::openapi_document;
pub use push::{
    push_router, spawn_wake_listener, CommandSpawnDispatcher, Delivery, MessagePost, PushState,
    RoomBus, RoomMessageEvent, SpawnDispatcher, SpawnOutcome, WakeDispatchContext,
    DEFAULT_BUS_CAPACITY,
};

use rustyred_thg_affordances::{affordance_nodes, AffordanceGraphStore};
use rustyred_thg_code::{CodebaseMapEntry, CodebaseMapProjectionEvent, CodebaseMapProjectionSink};
use rustyred_thg_core::{GraphStore, NodeQuery, NodeRecord, RedCoreGraphStore};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use theorem_harness_core::{
    compile_map_artifact, describe_map_artifact, stable_map_id, MapArtifactCompileInput,
    MapArtifactState,
};
use theorem_harness_runtime::{
    list_presence, load_events, load_run, read_intents_for_room, read_mentions_for_actor,
    read_mentions_for_actor_with_urgencies, read_records_for_room, room_status, CoordinationError,
    HarnessRuntimeError,
};

/// Node label the runtime persists run state under (`event_log::run_node`).
pub const RUN_LABEL: &str = "HarnessRun";

/// Node label for persisted compiled map artifacts.
pub const MAP_ARTIFACT_LABEL: &str = "MapArtifact";

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
    urgencies: &[String],
    consume: bool,
    limit: usize,
) -> Result<Value, CoordinationError> {
    let mentions = if urgencies.is_empty() {
        read_mentions_for_actor(store, tenant_slug, actor_id, consume, limit)?
    } else {
        read_mentions_for_actor_with_urgencies(
            store,
            tenant_slug,
            actor_id,
            urgencies,
            consume,
            limit,
        )?
    };
    Ok(json!({
        "tenant": tenant_slug,
        "actor_id": actor_id,
        "urgencies": urgencies,
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

/// Server-side bridge from code-KG projections to persisted `MapArtifact` nodes.
#[derive(Clone)]
pub struct GraphStoreMapArtifactSink {
    store: Arc<Mutex<RedCoreGraphStore>>,
}

impl GraphStoreMapArtifactSink {
    pub fn new(store: Arc<Mutex<RedCoreGraphStore>>) -> Self {
        Self { store }
    }
}

impl CodebaseMapProjectionSink for GraphStoreMapArtifactSink {
    fn publish_codebase_map(&self, event: &CodebaseMapProjectionEvent) -> Result<(), String> {
        let mut store = self
            .store
            .lock()
            .map_err(|_| "harness store lock poisoned".to_string())?;
        persist_codebase_map_projection(&mut *store, event).map(|_| ())
    }
}

/// Persist a code-KG projection through the existing MapArtifact compiler.
pub fn persist_codebase_map_projection<S: GraphStore>(
    store: &mut S,
    event: &CodebaseMapProjectionEvent,
) -> Result<MapArtifactState, String> {
    let map_kind = "CodebaseMap";
    let scope_kind = "repo";
    let map_id = stable_map_id(map_kind, scope_kind, &event.repo_id);
    let current = store
        .get_node(&map_id)
        .and_then(|node| serde_json::from_value::<MapArtifactState>(node.properties.clone()).ok());
    let artifact = compile_map_artifact(MapArtifactCompileInput {
        map_kind: map_kind.to_string(),
        scope_kind: scope_kind.to_string(),
        scope_ref: event.repo_id.clone(),
        repo: event.repo_id.clone(),
        target: event.repo_id.clone(),
        current,
        precomputed_entries: event.projection.entries.clone(),
        ..MapArtifactCompileInput::default()
    });
    let node = map_artifact_node(
        &artifact,
        &event.tenant_id,
        &event.repo_id,
        &event.operation,
        &event.projection.entries,
    )?;
    GraphStore::upsert_node(store, node)
        .map_err(|error| format!("persist MapArtifact: {}", error.message))?;
    Ok(artifact)
}

fn map_artifact_node(
    artifact: &MapArtifactState,
    tenant_id: &str,
    repo_id: &str,
    operation: &str,
    projection_entries: &[CodebaseMapEntry],
) -> Result<NodeRecord, String> {
    let mut properties = serde_json::to_value(artifact)
        .map_err(|error| error.to_string())?
        .as_object()
        .cloned()
        .ok_or_else(|| "MapArtifactState did not serialize to an object".to_string())?;
    properties.insert("tenant_id".to_string(), json!(tenant_id));
    properties.insert("repo_id".to_string(), json!(repo_id));
    properties.insert("source".to_string(), json!("code_kg_projection"));
    properties.insert("operation".to_string(), json!(operation));
    properties.insert(
        "projection_entry_count".to_string(),
        json!(projection_entries.len()),
    );
    Ok(NodeRecord::new(
        artifact.map_id.clone(),
        [MAP_ARTIFACT_LABEL],
        Value::Object(properties),
    ))
}

/// List persisted map artifacts for the tenant.
pub fn maps_json<S: GraphStore>(store: &S, tenant_slug: &str) -> Value {
    let query = NodeQuery {
        label: Some(MAP_ARTIFACT_LABEL.to_string()),
        properties: Default::default(),
        limit: None,
        include_expired: false,
    };
    let mut maps: Vec<Value> = GraphStore::query_nodes(store, query)
        .into_iter()
        .filter(|node| {
            node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_slug)
        })
        .map(|node| map_artifact_summary(&node.properties))
        .collect();
    maps.sort_by(|a, b| {
        let a_key = (
            a.get("scope_ref").and_then(Value::as_str).unwrap_or(""),
            a.get("map_kind").and_then(Value::as_str).unwrap_or(""),
            a.get("map_id").and_then(Value::as_str).unwrap_or(""),
        );
        let b_key = (
            b.get("scope_ref").and_then(Value::as_str).unwrap_or(""),
            b.get("map_kind").and_then(Value::as_str).unwrap_or(""),
            b.get("map_id").and_then(Value::as_str).unwrap_or(""),
        );
        a_key.cmp(&b_key)
    });
    json!({
        "tenant": tenant_slug,
        "maps": maps,
        "count": maps.len()
    })
}

/// Fetch one persisted map artifact for the tenant.
pub fn map_json<S: GraphStore>(store: &S, tenant_slug: &str, map_id: &str) -> Option<Value> {
    let node = GraphStore::get_node(store, map_id)?;
    if !node.labels.iter().any(|label| label == MAP_ARTIFACT_LABEL) {
        return None;
    }
    if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_slug) {
        return None;
    }
    Some(json!({
        "tenant": tenant_slug,
        "map": node.properties.clone()
    }))
}

fn map_artifact_summary(properties: &Value) -> Value {
    if let Ok(artifact) = serde_json::from_value::<MapArtifactState>(properties.clone()) {
        let mut summary = describe_map_artifact(&artifact);
        for key in ["tenant_id", "repo_id", "source", "operation"] {
            if let Some(value) = properties.get(key) {
                summary.insert(key.to_string(), value.clone());
            }
        }
        return Value::Object(summary);
    }
    properties.clone()
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
    use rustyred_thg_code::{CodebaseMapProjection, CodebaseMapProjectionEvent};
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::Map;
    use std::path::PathBuf;
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

    fn map_entry(kind: &str, id: &str, title: &str) -> CodebaseMapEntry {
        payload(&[
            ("entry_id", json!(id)),
            ("kind", json!(kind)),
            ("title", json!(title)),
            ("summary", json!(format!("{title} summary"))),
            ("pagerank", json!(0.42)),
            ("in_degree", json!(2)),
            ("out_degree", json!(3)),
        ])
    }

    fn projection_event() -> CodebaseMapProjectionEvent {
        let entries = vec![
            map_entry("module", "module:src/lib.rs", "src/lib.rs"),
            map_entry("key_symbol", "symbol:handle", "handle_ingest"),
        ];
        CodebaseMapProjectionEvent {
            tenant_id: "smoke".to_string(),
            repo_id: "theorem/example".to_string(),
            operation: "ingest".to_string(),
            repo_path: Some(PathBuf::from("/workspace/theorem")),
            projection: CodebaseMapProjection {
                tenant_id: "smoke".to_string(),
                repo_id: "theorem/example".to_string(),
                markdown_body: "# CodebaseMap for theorem/example".to_string(),
                entries,
            },
        }
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

        let mentions = mentions_json(&mut store, "smoke", "claude-code", &[], false, 20)
            .expect("mentions json");
        assert_eq!(mentions["count"], json!(1));
        assert_eq!(mentions["mentions"][0]["actor_id"], json!("codex"));

        let consumed = mentions_json(&mut store, "smoke", "claude-code", &[], true, 20)
            .expect("consume mentions");
        assert_eq!(consumed["count"], json!(1));

        let empty_after_consume = mentions_json(&mut store, "smoke", "claude-code", &[], false, 20)
            .expect("mentions empty");
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

    #[test]
    fn persists_and_serves_codebase_map_projection() {
        let mut store = InMemoryGraphStore::default();
        let event = projection_event();

        let artifact =
            persist_codebase_map_projection(&mut store, &event).expect("persist projection");

        assert_eq!(artifact.map_kind, "CodebaseMap");
        assert_eq!(artifact.scope_kind, "repo");
        assert_eq!(artifact.scope_ref, "theorem/example");
        assert!(artifact
            .entries
            .iter()
            .any(|entry| entry.get("kind") == Some(&json!("module"))));
        assert!(artifact
            .entries
            .iter()
            .any(|entry| entry.get("kind") == Some(&json!("key_symbol"))));
        assert!(artifact.markdown_body.contains("## Modules"));
        assert!(artifact.markdown_body.contains("## Key symbols"));

        let listing = maps_json(&store, "smoke");
        assert_eq!(listing["count"], json!(1));
        assert_eq!(listing["maps"][0]["map_id"], json!(artifact.map_id));
        assert_eq!(
            listing["maps"][0]["entry_count"],
            json!(artifact.entries.len())
        );
        assert_eq!(listing["maps"][0]["source"], json!("code_kg_projection"));

        let detail = map_json(&store, "smoke", &artifact.map_id).expect("map detail");
        assert_eq!(detail["map"]["tenant_id"], json!("smoke"));
        assert_eq!(detail["map"]["repo_id"], json!("theorem/example"));
        assert_eq!(detail["map"]["projection_entry_count"], json!(2));
        assert!(detail["map"]["entries"]
            .as_array()
            .expect("entries array")
            .iter()
            .any(|entry| entry.get("kind") == Some(&json!("module"))));

        assert!(map_json(&store, "other", &artifact.map_id).is_none());
    }

    #[test]
    fn codebase_map_projection_updates_preserve_map_identity() {
        let mut store = InMemoryGraphStore::default();
        let event = projection_event();

        let first = persist_codebase_map_projection(&mut store, &event).expect("first projection");
        let second =
            persist_codebase_map_projection(&mut store, &event).expect("second projection");

        assert_eq!(first.map_id, second.map_id);
        assert_eq!(first.created_at, second.created_at);
        assert_eq!(maps_json(&store, "smoke")["count"], json!(1));
    }
}
