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
//!   GET /harness/compound-engineering -> { "compound_engineering": {...} }
//!   POST /harness/rooms/{room_id}/messages -> write a message + emit push (tap/hold)
//!   GET  /harness/rooms/{room_id}/stream    -> SSE of this room's messages (live)
//!   GET /harness/actors/{actor}/mentions  -> { "mentions": [...] }
//!   POST /harness/jobs             -> create THG job, mirror to Postgres, emit wake
//!   GET /harness/jobs/counts       -> inspect Postgres dispatch state counts
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
    connect_transport, connector_manifest, content_core_mcp_target_from_env, initialize_params,
    parse_initialize, parse_tools_list, tools_list_params, ConnectionTarget, McpTransport,
    CONTENT_CORE_SERVER_ID,
};
use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use theorem_dispatch::{priority_from_harness, DispatchError, DispatchQueue, Job as DispatchJob};
use theorem_harness_core::{JobSubmission, Priority, TargetHead};
use theorem_harness_runtime::{
    job_submit, run_agent_room_cycle, AgentRoomRunnerConfig, CoordinationError,
    HarnessRuntimeError, RealHeadInvoker, DEFAULT_BINDING_ID,
};
use theorem_harness_server::push::write_room_message;
use theorem_harness_server::{
    compound_engineering_json, connectors_json, gepa_trainset_json, github_router, intents_json,
    map_json, maps_json, mentions_json, openapi_document, presence_json, push_router, records_json,
    room_json, run_json, runs_json, spawn_wake_listener, Delivery, GithubApp, GithubWebhookState,
    GraphStoreMapArtifactSink, MessagePost, PushState, RoomBus, DEFAULT_BUS_CAPACITY,
};

type SharedStore = Arc<Mutex<RedCoreGraphStore>>;
const DISPATCH_DATABASE_URL_ENV: &str = "THEOREM_DISPATCH_DATABASE_URL";
const DEFAULT_JOB_ROOM_ID: &str = "repo:theorem:branch:main";

#[derive(Debug, Default, Deserialize)]
struct CoordinationQuery {
    tenant: Option<String>,
    tenant_slug: Option<String>,
    status: Option<String>,
    statuses: Option<String>,
    urgency: Option<String>,
    urgencies: Option<String>,
    record_type: Option<String>,
    record_types: Option<String>,
    cluster_key: Option<String>,
    since: Option<String>,
    consume: Option<bool>,
    limit: Option<usize>,
}

#[derive(Clone)]
struct JobHttpState {
    store: SharedStore,
    bus: RoomBus,
    dispatch: Option<DispatchQueue>,
}

#[derive(Debug, Deserialize)]
struct JobSubmitHttpBody {
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    tenant_slug: Option<String>,
    #[serde(default)]
    submitted_by: Option<String>,
    #[serde(default)]
    room_id: Option<String>,
    #[serde(flatten)]
    submission: JobSubmission,
}

impl JobSubmitHttpBody {
    fn tenant_slug(&self) -> Result<String, (StatusCode, String)> {
        request_tenant_slug(self.tenant_slug.as_deref(), self.tenant.as_deref())
            .map_err(|message| (StatusCode::BAD_REQUEST, message))
    }

    fn submitted_by(&self) -> String {
        self.submitted_by
            .as_deref()
            .map(str::trim)
            .filter(|actor| !actor.is_empty())
            .unwrap_or("theorem-harness-server")
            .to_string()
    }

    fn room_id(&self) -> String {
        self.room_id
            .as_deref()
            .map(str::trim)
            .filter(|room| !room.is_empty())
            .unwrap_or(DEFAULT_JOB_ROOM_ID)
            .to_string()
    }
}

impl CoordinationQuery {
    fn tenant_slug(&self) -> Result<String, StatusCode> {
        request_tenant_slug(self.tenant_slug.as_deref(), self.tenant.as_deref())
            .map_err(|_| StatusCode::BAD_REQUEST)
    }

    fn statuses(&self) -> Vec<String> {
        self.statuses
            .as_deref()
            .or(self.status.as_deref())
            .map(split_csv)
            .unwrap_or_default()
    }

    fn urgencies(&self) -> Vec<String> {
        self.urgencies
            .as_deref()
            .or(self.urgency.as_deref())
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

fn request_tenant_slug(tenant_slug: Option<&str>, tenant: Option<&str>) -> Result<String, String> {
    tenant_slug
        .into_iter()
        .chain(tenant)
        .map(str::trim)
        .filter(|tenant| !tenant.is_empty())
        .find(|_| true)
        .map(str::to_string)
        .or_else(configured_request_tenant_slug)
        .ok_or_else(|| "tenant or tenant_slug is required".to_string())
}

fn configured_request_tenant_slug() -> Option<String> {
    [
        "THEOREM_HARNESS_TENANT_SLUG",
        "THEOREM_AGENT_TENANT_SLUG",
        "THEOREM_TENANT_ID",
    ]
    .iter()
    .find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && value != "default")
    })
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
    maybe_spawn_theorem_agent_runner(state.clone());
    let push = push_router(PushState {
        store: state.clone(),
        bus: bus.clone(),
    });
    let dispatch = dispatch_queue_from_env().await;
    let jobs = jobs_router(JobHttpState {
        store: state.clone(),
        bus: bus.clone(),
        dispatch,
    });

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/openapi.json", get(openapi_json))
        .route("/harness/runs", get(list_runs))
        .route("/harness/runs/:run_id", get(get_run))
        .route("/harness/gepa/trainsets/:intent_id", get(get_gepa_trainset))
        .route("/harness/maps", get(list_maps))
        .route("/harness/maps/:map_id", get(get_map))
        .route("/harness/rooms/:room_id", get(get_room))
        .route("/harness/rooms/:room_id/presence", get(get_room_presence))
        .route("/harness/rooms/:room_id/intents", get(get_room_intents))
        .route("/harness/rooms/:room_id/records", get(get_room_records))
        .route(
            "/harness/compound-engineering",
            get(get_compound_engineering),
        )
        .route(
            "/harness/actors/:actor_id/mentions",
            get(get_actor_mentions),
        )
        .route("/connectors", get(list_connectors))
        .route("/connectors/register", post(register_connector_route))
        .route(
            "/connectors/register/content-core",
            post(register_content_core_connector_route),
        )
        .with_state(state.clone())
        .merge(push)
        .merge(jobs);
    let app = maybe_mount_github_router(app, state.clone());

    let port = std::env::var("PORT").unwrap_or_else(|_| "50080".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!(%addr, %data_dir, "theorem-harness-server listening");
    axum::serve(listener, app).await.expect("serve");
}

async fn dispatch_queue_from_env() -> Option<DispatchQueue> {
    let database_url = std::env::var(DISPATCH_DATABASE_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())?;
    Some(
        DispatchQueue::connect(&database_url)
            .await
            .expect("connect Postgres dispatch queue"),
    )
}

fn maybe_spawn_theorem_agent_runner(state: SharedStore) {
    if !env_truthy("THEOREM_AGENT_ROOM_RUNNER") {
        tracing::info!("Theorem agent room runner disabled");
        return;
    }
    let tenant_slug = std::env::var("THEOREM_AGENT_TENANT_SLUG")
        .or_else(|_| std::env::var("THEOREM_TENANT_ID"))
        .unwrap_or_else(|_| "Travis-Gilbert".to_string());
    let room_id =
        std::env::var("THEOREM_AGENT_ROOM_ID").unwrap_or_else(|_| DEFAULT_JOB_ROOM_ID.to_string());
    let binding_id = std::env::var("THEOREM_AGENT_BINDING_ID")
        .unwrap_or_else(|_| DEFAULT_BINDING_ID.to_string());
    let interval_ms = std::env::var("THEOREM_AGENT_RUNNER_INTERVAL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(15_000);

    tracing::info!(%tenant_slug, %room_id, %binding_id, interval_ms, "starting Theorem agent room runner");
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));
        loop {
            interval.tick().await;
            let state = state.clone();
            let tenant_slug = tenant_slug.clone();
            let room_id = room_id.clone();
            let binding_id = binding_id.clone();
            let outcome = tokio::task::spawn_blocking(move || {
                let invoker = RealHeadInvoker::from_env().map_err(|error| error.to_string())?;
                let mut store = state.lock().map_err(|_| "store lock".to_string())?;
                let mut config = AgentRoomRunnerConfig::new(tenant_slug, room_id, binding_id);
                config.repo = "Theorem".to_string();
                config.branch = "main".to_string();
                config.task = "theorem agent room runner".to_string();
                run_agent_room_cycle(&mut *store, config, &invoker)
                    .map_err(|error| error.to_string())
            })
            .await;
            match outcome {
                Ok(Ok(cycle)) if !cycle.turns.is_empty() => {
                    tracing::info!(
                        turns = cycle.turns.len(),
                        "Theorem agent runner processed room turns"
                    );
                }
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Theorem agent room runner cycle failed");
                }
                Err(error) => {
                    tracing::warn!(%error, "Theorem agent room runner task failed");
                }
            }
        }
    });
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
}

fn jobs_router(state: JobHttpState) -> Router {
    Router::new()
        .route("/harness/jobs", post(submit_job))
        .route("/harness/jobs/counts", get(dispatch_job_counts))
        .with_state(state)
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

async fn openapi_json() -> Json<Value> {
    Json(openapi_document())
}

async fn submit_job(
    State(state): State<JobHttpState>,
    Json(body): Json<JobSubmitHttpBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tenant_slug = body.tenant_slug()?;
    let submitted_by = body.submitted_by();
    let room_id = body.room_id();
    let submission = body.submission;

    let outcome = {
        let mut store = state
            .store
            .lock()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store lock".to_string()))?;
        job_submit(&mut *store, submission, submitted_by.clone()).map_err(runtime_status)?
    };

    let dispatch_mirrored = if let Some(queue) = &state.dispatch {
        let dispatch_job = DispatchJob::from_harness(&outcome.job);
        let priority = priority_from_harness(outcome.job.priority);
        queue
            .submit(dispatch_job, priority)
            .await
            .map_err(dispatch_status)?;
        true
    } else {
        false
    };

    let wake_event = {
        let mut metadata = Map::new();
        metadata.insert("source".to_string(), json!("job_submit"));
        metadata.insert("job_id".to_string(), json!(outcome.job.job_id));
        metadata.insert("dispatch_mirrored".to_string(), json!(dispatch_mirrored));
        let post = MessagePost {
            tenant_slug: tenant_slug.clone(),
            actor_id: submitted_by.clone(),
            message: format!(
                "dispatch job submitted: {} ({})",
                outcome.job.job_id, outcome.job.title
            ),
            urgency: priority_urgency(outcome.job.priority),
            delivery: Delivery::Wake,
            mentions: wake_mentions(outcome.job.target_head),
            metadata,
        };
        let mut store = state
            .store
            .lock()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store lock".to_string()))?;
        let (_message, event) =
            write_room_message(&mut *store, &room_id, post).map_err(coordination_status)?;
        event
    };
    state.bus.publish(wake_event.clone());

    Ok(Json(json!({
        "tenant": tenant_slug,
        "room_id": room_id,
        "job_id": outcome.job.job_id,
        "created": outcome.created,
        "dispatch_mirrored": dispatch_mirrored,
        "job": outcome.job,
        "wake_event": wake_event
    })))
}

async fn dispatch_job_counts(
    State(state): State<JobHttpState>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let Some(queue) = &state.dispatch else {
        return Ok(Json(json!({
            "dispatch_configured": false,
            "counts": []
        })));
    };
    let counts = queue.state_counts().await.map_err(dispatch_status)?;
    Ok(Json(json!({
        "dispatch_configured": true,
        "counts": counts
    })))
}

async fn list_maps(
    State(store): State<SharedStore>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    Ok(Json(maps_json(&*store, &tenant_slug)))
}

async fn get_map(
    State(store): State<SharedStore>,
    Path(map_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    map_json(&*store, &tenant_slug, &map_id)
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

async fn get_gepa_trainset(
    State(store): State<SharedStore>,
    Path(intent_id): Path<String>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let store = store.lock().expect("store lock");
    gepa_trainset_json(&*store, &intent_id)
        .map(Json)
        .map_err(runtime_status)
}

async fn get_room(
    State(store): State<SharedStore>,
    Path(room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    room_json(&*store, &tenant_slug, &room_id)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_room_presence(
    State(store): State<SharedStore>,
    Path(_room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    presence_json(&*store, &tenant_slug)
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_room_intents(
    State(store): State<SharedStore>,
    Path(room_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    intents_json(&*store, &tenant_slug, &room_id, &query.statuses())
        .map(Json)
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_actor_mentions(
    State(store): State<SharedStore>,
    Path(actor_id): Path<String>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let mut store = store.lock().expect("store lock");
    mentions_json(
        &mut *store,
        &tenant_slug,
        &actor_id,
        &query.urgencies(),
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
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    records_json(
        &*store,
        &tenant_slug,
        &room_id,
        &query.record_types(),
        query.limit.unwrap_or(50),
    )
    .map(Json)
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}

async fn get_compound_engineering(
    State(store): State<SharedStore>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    compound_engineering_json(
        &*store,
        &tenant_slug,
        query.cluster_key.as_deref(),
        query.since.as_deref(),
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
    #[serde(default)]
    tenant_slug: Option<String>,
    server_id: String,
    #[serde(default)]
    label: String,
    target: ConnectionTarget,
}

#[derive(Debug, Default, Deserialize)]
struct RegisterContentCoreConnectorBody {
    #[serde(default)]
    tenant: Option<String>,
    #[serde(default)]
    tenant_slug: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

/// `GET /connectors?tenant=...` -> the registered connectors + tool affordances.
/// Read-only and fast; no server is contacted.
async fn list_connectors(
    State(store): State<SharedStore>,
    Query(query): Query<CoordinationQuery>,
) -> Result<Json<Value>, StatusCode> {
    let tenant_slug = query.tenant_slug()?;
    let store = store.lock().expect("store lock");
    Ok(Json(connectors_json(&*store, &tenant_slug)))
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
    let tenant = request_tenant_slug(body.tenant_slug.as_deref(), body.tenant.as_deref())
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
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

    register_connector_target(store, tenant, server_id, label, target, "operator").await
}

async fn register_content_core_connector_route(
    State(store): State<SharedStore>,
    Json(body): Json<RegisterContentCoreConnectorBody>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let tenant = request_tenant_slug(body.tenant_slug.as_deref(), body.tenant.as_deref())
        .map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let label = body
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or("Content Core")
        .to_string();
    register_connector_target(
        store,
        tenant,
        CONTENT_CORE_SERVER_ID.to_string(),
        label,
        content_core_mcp_target_from_env(),
        "operator",
    )
    .await
}

async fn register_connector_target(
    store: SharedStore,
    tenant: String,
    server_id: String,
    label: String,
    target: ConnectionTarget,
    actor: &'static str,
) -> Result<Json<Value>, (StatusCode, String)> {
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
        let registration =
            register_connector_with_target(&mut *store, manifest, Some(target_value), Some(actor))
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

fn priority_urgency(priority: Priority) -> String {
    match priority {
        Priority::P0 => "block",
        Priority::P1 => "ask",
        Priority::P2 => "info",
    }
    .to_string()
}

fn wake_mentions(target_head: TargetHead) -> Vec<String> {
    match target_head {
        TargetHead::Claude => vec!["claude-code".to_string()],
        TargetHead::Codex => vec!["codex".to_string()],
        TargetHead::Either => vec!["codex".to_string(), "claude-code".to_string()],
    }
}

fn runtime_status(error: HarnessRuntimeError) -> (StatusCode, String) {
    match error {
        HarnessRuntimeError::Deserialization(_) => (StatusCode::BAD_REQUEST, error.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn coordination_status(error: CoordinationError) -> (StatusCode, String) {
    match error {
        CoordinationError::InvalidInput { .. } => (StatusCode::BAD_REQUEST, error.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

fn dispatch_status(error: DispatchError) -> (StatusCode, String) {
    match error {
        DispatchError::Invalid(_) => (StatusCode::BAD_REQUEST, error.to_string()),
        DispatchError::NotFound(_) => (StatusCode::NOT_FOUND, error.to_string()),
        DispatchError::Sqlx(_) => (StatusCode::BAD_GATEWAY, error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_priority_maps_to_coordination_urgency_vocabulary() {
        assert_eq!(priority_urgency(Priority::P0), "block");
        assert_eq!(priority_urgency(Priority::P1), "ask");
        assert_eq!(priority_urgency(Priority::P2), "info");
    }

    #[test]
    fn request_tenant_preserves_explicit_casing() {
        assert_eq!(
            request_tenant_slug(Some(" Travis-Gilbert "), None).unwrap(),
            "Travis-Gilbert"
        );
        assert_eq!(
            request_tenant_slug(None, Some(" Travis-Gilbert ")).unwrap(),
            "Travis-Gilbert"
        );
        assert_eq!(
            request_tenant_slug(Some(" "), Some(" Travis-Gilbert ")).unwrap(),
            "Travis-Gilbert"
        );
    }

    #[test]
    fn target_head_maps_to_wake_mentions() {
        assert_eq!(wake_mentions(TargetHead::Claude), vec!["claude-code"]);
        assert_eq!(wake_mentions(TargetHead::Codex), vec!["codex"]);
        assert_eq!(
            wake_mentions(TargetHead::Either),
            vec!["codex", "claude-code"]
        );
    }
}
