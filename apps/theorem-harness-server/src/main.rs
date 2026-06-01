//! Theorem harness HTTP transport (binary).
//!
//! A thin Axum server over a durable `RedCoreGraphStore`, exposing the run read
//! contract the Theorem clients consume:
//!   GET /harness/runs            -> { "runs": [...] }
//!   GET /harness/runs/{run_id}   -> { "run": {...}, "events": [...] }  (404 if unknown)
//!   GET /harness/rooms/{room_id}          -> { "room": {...} }
//!   GET /harness/rooms/{room_id}/presence -> { "presence": [...] }
//!   GET /harness/rooms/{room_id}/intents  -> { "intents": [...] }
//!   GET /harness/rooms/{room_id}/records  -> { "records": [...] }
//!   GET /harness/actors/{actor}/mentions  -> { "mentions": [...] }
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
    routing::get,
    Json, Router,
};
use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use serde::Deserialize;
use serde_json::Value;
use theorem_harness_server::{
    intents_json, mentions_json, presence_json, records_json, room_json, run_json, runs_json,
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

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/harness/runs", get(list_runs))
        .route("/harness/runs/:run_id", get(get_run))
        .route("/harness/rooms/:room_id", get(get_room))
        .route("/harness/rooms/:room_id/presence", get(get_room_presence))
        .route("/harness/rooms/:room_id/intents", get(get_room_intents))
        .route("/harness/rooms/:room_id/records", get(get_room_records))
        .route(
            "/harness/actors/:actor_id/mentions",
            get(get_actor_mentions),
        )
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "50080".to_string());
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind listener");
    tracing::info!(%addr, %data_dir, "theorem-harness-server listening");
    axum::serve(listener, app).await.expect("serve");
}

async fn healthz() -> &'static str {
    "ok"
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

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}
