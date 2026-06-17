//! Theorem harness HTTP transport (binary).
//!
//! A thin Axum server over a durable `RedCoreGraphStore`, exposing the run read
//! contract the Theorem clients consume:
//!   GET /harness/runs            -> { "runs": [...] }
//!   GET /harness/runs/{run_id}   -> { "run": {...}, "events": [...] }  (404 if unknown)
//!   GET /harness/maps            -> { "maps": [...] }
//!   GET /harness/maps/{map_id}   -> { "map": {...} }  (404 if unknown)
//!   GET /harness/rooms/{room_id}          -> { "room": {...} }
//!   GET /harness/rooms/{room_id}/presence -> { "presence": [...] }
//!   GET /harness/rooms/{room_id}/intents  -> { "intents": [...] }
//!   GET /harness/rooms/{room_id}/records  -> { "records": [...] }
//!   POST /harness/rooms/{room_id}/messages -> write a message + emit push (tap/hold)
//!   GET  /harness/rooms/{room_id}/stream    -> SSE of this room's messages (live)
//!   GET /harness/actors/{actor}/mentions  -> { "mentions": [...] }
//!   GET /connectors              -> { "connectors": [...], "affordances": [...] }
//!   POST /connectors/register    -> connect an MCP server, register its tools as affordances
//!   GET /healthz                 -> "ok"
//!
//! Reads the same store the runtime persists runs to. Set the data dir with
//! `THEOREM_HARNESS_DATA_DIR` (default `harness-data`) and the port with `PORT`
//! (default `50080`). Empty store -> empty list (honest; runs appear as the
//! harness writes them).

use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use rustyred_thg_affordances::registry::register_connector_with_target;
use rustyred_thg_code::{CodeIndexRuntime, CodebaseMapProjectionSink, GitCredentialResolver};
use rustyred_thg_connectors::{
    connect_transport, connector_manifest, initialize_params, parse_initialize, parse_tools_list,
    tools_list_params, ConnectionTarget, McpTransport,
};
use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use theorem_harness_server::{
    connectors_json, github_router, intents_json, map_json, maps_json, mentions_json,
    presence_json, push_router, records_json, room_json, run_json, runs_json, spawn_wake_listener,
    GithubApp, GithubWebhookState, GraphStoreMapArtifactSink, PushState, RoomBus,
    DEFAULT_BUS_CAPACITY,
};

type SharedStore = Arc<Mutex<RedCoreGraphStore>>;

#[derive(Debug, Default, Deserialize)]
struct CoordinationQuery {
    tenant: Option<String>,
    tenant_slug: Option<String>,
    status: Option<String>,
    statuses: Option<String>,
    record_type: Option<String>,
    record_types: Option<String>,
    consume: Option<bool>,
    limit: Option<usize>,
}

impl CoordinationQuery {
    fn tenant_slug(&self) -> String {
        self.tenant_slug
            .as_deref()
            .or(self.tenant.as_deref())
            .map(str::trim)
            .filter(|tenant| !tenant.is_empty())
            .unwrap_or("default")
            .to_string()
    }

    fn statuses(&self) -> Vec<String> {
        self.statuses
            .as_deref()
            .or(self.status.as_deref())
            .map(split_csv)
            .unwrap_or_default()
    }

    fn record_types(&self) -> Vec<String> {
        self.record_types
            .as_deref()
            .or(self.record_type.as_deref())
            .map(split_csv)
            .unwrap_or_default()
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_dir =
        std::env::var("THEOREM_HARNESS_DATA_DIR").unwrap_or_else(|_| "harness-data".to_string());
    let store = RedCoreGraphStore::open(&data_dir, RedCoreOptions::default())
        .expect("open RedCore graph store");
    let state: SharedStore = Arc::new(Mutex::new(store));

    // The in-process push bus: the room write endpoint emits onto it, the SSE
    // stream and the spawn-listener subscribe. The listener rides on this server,
    // so the always-on cost of the whole push feature is one subscription.
    let bus = RoomBus::with_command_spawn(DEFAULT_BUS_CAPACITY);
    spawn_wake_listener(bus.clone(), state.clone());
    let push = push_router(PushState {
        store: state.clone(),
        bus: bus.clone(),
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/harness/runs", get(list_runs))
        .route("/harness/runs/:run_id", get(get_run))
        .route("/harness/maps", get(list_maps))
        .route("/harness/maps/:map_id", get(get_map))
        .route("/harness/rooms/:room_id", get(get_room))
        .route("/harness/rooms/:room_id/presence", get(get_room_presence))
        .route("/harness/rooms/:room_id/intents", get(get_room_intents))
        .route("/harness/rooms/:room_id/records", get(get_room_records))
        .route(
            "/harness/actors/:actor_id/mentions",
            get(get_actor_mentions),
        )
        .route("/connectors", get(list_connectors))
        .route("/connectors/register", post(register_connector_route))
        .with_state(state.clone())
        .merge(push);
    let app = maybe_mount_github_router(app, state.clone());

    let port = std::env::var("PORT").unwrap_or_else(|_| "50080".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!(%addr, %data_dir, "theorem-harness-server listening");
    axum::serve(listener, app).await.expect("serve");
}

fn maybe_mount_github_router(app: Router, store: SharedStore) -> Router {
    let Ok(github_app) = GithubApp::from_env() else {
        tracing::info!("GitHub App webhook router disabled: missing GitHub App env config");
        return app;
    };
    let github_app = Arc::new(github_app);
    let resolver: Arc<dyn GitCredentialResolver> = github_app.clone();
    let map_sink: Arc<dyn CodebaseMapProjectionSink> =
        Arc::new(GraphStoreMapArtifactSink::new(store));
    let code_index = match CodeIndexRuntime::try_new_with_integrations(
        Some(resolver),
        Some(map_sink),
    ) {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::warn!(%error, "GitHub App webhook router disabled: code index failed to open");
            return app;
        }
    };
    let tenant_slug = std::env::var("THEOREM_GITHUB_TENANT_SLUG")
        .or_else(|_| std::env::var("THEOREM_TENANT_ID"))
        .unwrap_or_else(|_| "Travis-Gilbert".to_string());
    app.merge(github_router(GithubWebhookState::new(
        github_app,
        code_index,
        tenant_slug,
    )))
}

async fn healthz() -> &'static str {
    "ok"
}

async fn list_maps(
    State(store): State<SharedStore>,
    Query(query): Query<CoordinationQuery>,
) -> Json<Value> {
    let store = store.lock().expect("store lock");
    Json(maps_json(&*store, &query.tenant_slug()))
}

async fn get_map(
    State(store): State<SharedStore>,
    Path(map_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    map_json(&*store, &query.tenant_slug(), &map_id)
        .map(Json)
        .ok_or(StatusCode::NOT_FOUND)
}

async fn list_runs(State(store): State<SharedStore>) -> Json<Value> {
    let store = store.lock().expect("store lock");
    Json(runs_json(&*store))
}

async fn get_run(
    State(store): State<SharedStore>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    match run_json(&*store, &run_id) {
        Ok(Some(value)) => Ok(Json(value)),
        Ok(None) => Err(StatusCode::NOT_FOUND),
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn get_room(
    State(store): State<SharedStore>,
    Path(room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    room_json(&*store, &query.tenant_slug(), &room_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_room_presence(
    State(store): State<SharedStore>,
    Path(_room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    presence_json(&*store, &query.tenant_slug())
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_room_intents(
    State(store): State<SharedStore>,
    Path(room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    intents_json(&*store, &query.tenant_slug(), &room_id, &query.statuses())
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_actor_mentions(
    State(store): State<SharedStore>,
    Path(actor_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let mut store = store.lock().expect("store lock");
    mentions_json(
        &mut *store,
        &query.tenant_slug(),
        &actor_id,
        query.consume.unwrap_or(false),
        query.limit.unwrap_or(20),
    )
    .map(Json)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_room_records(
    State(store): State<SharedStore>,
    Path(room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let store = store.lock().expect("store lock");
    records_json(
        &*store,
        &query.tenant_slug(),
        &room_id,
        &query.record_types(),
        query.limit.unwrap_or(50),
    )
    .map(Json)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

/// Request body for `POST /connectors/register`: how to reach an MCP server and
/// what to name the connector. The server is spawned, handshaken, and its tools
/// registered as `Affordance` nodes under `(tenant, server_id)`, with the reach
/// (`target`) persisted so a selected tool can be invoked later.
#[derive(Debug, Deserialize)]
struct RegisterConnectorBody {
    #[serde(default)]
    tenant: Option<String>,
    server_id: String,
    #[serde(default)]
    label: String,
    target: ConnectionTarget,
}

/// `GET /connectors?tenant=...` -> the registered connectors + tool affordances.
/// Read-only and fast; no server is contacted.
async fn list_connectors(
    State(store): State<SharedStore>,
    Query(query): Query<CoordinationQuery>,
) -> Json<Value> {
    let store = store.lock().expect("store lock");
    Json(connectors_json(&*store, &query.tenant_slug()))
}

/// `POST /connectors/register` -> connect to an MCP server, list its tools, and
/// register them. The handshake + tools/list (blocking subprocess I/O) runs OFF
/// the async runtime via `spawn_blocking` and OUTSIDE the store lock; the store is
/// locked only for the fast register write, so a slow server cannot freeze the
/// read endpoints. (Until the `StdioTransport` read-timeout follow-up, a hung
/// server still ties up that one blocking task.)
async fn register_connector_route(
    State(store): State<SharedStore>,
    Json(body): Json<RegisterConnectorBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tenant = body
        .tenant
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("default")
        .to_string();
    let server_id = body.server_id.trim().to_string();
    if server_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "server_id is required".to_string()));
    }
    let label = if body.label.trim().is_empty() {
        server_id.clone()
    } else {
        body.label.trim().to_string()
    };
    let target = body.target;

    let outcome = tokio::task::spawn_blocking(move || -> Result<Value, String> {
        // Network OUTSIDE the store lock: spawn the server and run the handshake.
        let mut transport = connect_transport(&target).map_err(|e| e.to_string())?;
        let init = transport
            .request("initialize", initialize_params())
            .map_err(|e| e.to_string())?;
        let server_info = parse_initialize(&init);
        transport
            .notify("notifications/initialized", json!({}))
            .map_err(|e| e.to_string())?;
        let tools = transport
            .request("tools/list", tools_list_params())
            .map_err(|e| e.to_string())?;
        let descriptors = parse_tools_list(&tools).map_err(|e| e.to_string())?;
        let manifest = connector_manifest(&tenant, &server_id, &label, &descriptors);
        let target_value = serde_json::to_value(&target).map_err(|e| e.to_string())?;

        // Lock the store only for the fast register write.
        let mut store = store
            .lock()
            .map_err(|_| "store lock poisoned".to_string())?;
        let registration = register_connector_with_target(
            &mut *store,
            manifest,
            Some(target_value),
            Some("operator"),
        )
        .map_err(|e| format!("{e:?}"))?;
        Ok(json!({
            "server": {
                "name": server_info.server_name,
                "version": server_info.server_version,
                "protocol": server_info.protocol_version,
            },
            "tenant": tenant,
            "server_id": server_id,
            "affordance_ids": registration.affordance_node_ids,
            "count": registration.affordance_node_ids.len(),
        }))
    })
    .await
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("join error: {e}"),
        )
    })?;

    outcome
        .map(Json)
        .map_err(|message| (StatusCode::BAD_GATEWAY, message))
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}
