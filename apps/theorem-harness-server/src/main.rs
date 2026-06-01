//! Theorem harness HTTP transport (binary).
//!
//! A thin Axum server over a durable `RedCoreGraphStore`, exposing the run read
//! contract the Theorem clients consume:
//!   GET /harness/runs            -> { "runs": [...] }
//!   GET /harness/runs/{run_id}   -> { "run": {...}, "events": [...] }  (404 if unknown)
//!   GET /healthz                 -> "ok"
//!
//! Reads the same store the runtime persists runs to. Set the data dir with
//! `THEOREM_HARNESS_DATA_DIR` (default `harness-data`) and the port with `PORT`
//! (default `50080`). Empty store -> empty list (honest; runs appear as the
//! harness writes them).

use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use serde_json::Value;
use theorem_harness_server::{run_json, runs_json};

type SharedStore = Arc<Mutex<RedCoreGraphStore>>;

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
