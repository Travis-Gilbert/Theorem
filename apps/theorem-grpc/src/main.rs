//! Theorem's first gRPC server.
//!
//! Serves theseus_search.v1.SearchService over the RustyRed substrate. gRPC is
//! merged with tiny HTTP `/health` and `/ready` routes on the same listener so
//! deploys can distinguish "process is alive" from "RedCore is still
//! recovering". Binds [::]:$PORT (IPv6 dual-stack) so Railway's private network
//! reaches it via theorem-grpc.railway.internal, and IPv4 healthchecks work too.
//! The civic-atlas-server dials this by setting THEOREM_SEARCH_URL (or the
//! legacy THESEUS_BRIDGE_URL).

mod app_affordance;
mod code_index;
mod code_kg;
mod code_service;
mod engine;
mod pb;
mod service;
mod session_delta;
mod valkey_cache;

use std::net::SocketAddr;
use std::sync::Arc;

use app_affordance::TheoremAppAffordanceService;
use axum::{extract::State, http::StatusCode, routing::get, Json};
use code_index::CodeIndexRuntime;
use code_service::TheoremCodeCrawlerService;
use engine::Engine;
use pb::{AppAffordanceServiceServer, CodeCrawlerServiceServer, SearchServiceServer};
use serde_json::{json, Value};
use service::TheoremSearchService;
use tokio::net::TcpListener;
use valkey_cache::ValkeyCache;

#[derive(Clone)]
struct ReadinessState {
    code_index: CodeIndexRuntime,
    app_affordance: TheoremAppAffordanceService,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Railway injects PORT. Default 50071 for local dev (a free gRPC-ish port).
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(50071);

    // Bind [::] (IPv6 dual-stack) so Railway's IPv6 private network reaches it
    // via the railway.internal domain; it also accepts IPv4 for healthchecks.
    let addr: SocketAddr = format!("[::]:{port}").parse()?;

    // Build the engine (empty substrate is the honest slice-1 default) and wrap
    // it in an Arc so the owned store outlives every borrowing handler call.
    let engine = Arc::new(Engine::new());
    let valkey_cache = ValkeyCache::from_env();
    match valkey_cache.ping() {
        Ok(Some(pong)) => tracing::info!("THEOREM_GRPC_VALKEY_READY {}", pong),
        Ok(None) => tracing::info!("THEOREM_GRPC_VALKEY_DISABLED"),
        Err(error) => tracing::warn!("THEOREM_GRPC_VALKEY_UNREACHABLE {}", error),
    }
    // ONE code store for the whole service. It starts in "recovering" mode so
    // the socket can bind before RedCore replays /data. Code calls return
    // UNAVAILABLE until the background recovery swaps in the durable store.
    let code_index = CodeIndexRuntime::recovering().map_err(std::io::Error::other)?;
    let app_affordance = TheoremAppAffordanceService::recovering_with_code_index_and_cache(
        code_index.clone(),
        valkey_cache.clone(),
    )
    .map_err(std::io::Error::other)?;
    let readiness = ReadinessState {
        code_index: code_index.clone(),
        app_affordance: app_affordance.clone(),
    };
    let search_svc =
        SearchServiceServer::new(TheoremSearchService::new(engine, valkey_cache.clone()));
    let code_svc =
        CodeCrawlerServiceServer::new(TheoremCodeCrawlerService::new(code_index.clone()));
    let app_affordance_svc = AppAffordanceServiceServer::new(app_affordance);

    let grpc = tonic::transport::Server::builder()
        .add_service(search_svc)
        .add_service(code_svc)
        .add_service(app_affordance_svc);
    #[allow(deprecated)]
    let grpc = grpc.into_router();
    let app = axum::Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .with_state(readiness)
        .merge(grpc);

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("THEOREM_GRPC_BOUND {}", addr);

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("theorem-grpc server stopped");
    Ok(())
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "status": "alive" }))
}

async fn ready(State(state): State<ReadinessState>) -> (StatusCode, Json<Value>) {
    let code_index = state.code_index.diagnostics();
    let app_affordance = state.app_affordance.recovery_snapshot();
    let code_phase = code_index
        .as_ref()
        .map(|diagnostics| diagnostics.recovery.phase.as_str())
        .unwrap_or("failed");
    let app_phase = app_affordance.phase.as_str();
    let ready = code_phase == "ready" && app_phase == "ready";
    let failed = code_phase == "failed" || app_phase == "failed" || code_index.is_err();
    let status = if ready {
        "ready"
    } else if failed {
        "failed"
    } else {
        "recovering"
    };
    let status_code = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let code_index_json = match code_index {
        Ok(diagnostics) => diagnostics.to_json(),
        Err(error) => json!({
            "recovery": {
                "phase": "failed",
                "error": error.to_string(),
            }
        }),
    };
    (
        status_code,
        Json(json!({
            "ok": ready,
            "status": status,
            "code_index": code_index_json,
            "app_affordance": app_affordance.to_json(),
        })),
    )
}

/// Wait for SIGTERM (production / Docker / Railway) or Ctrl-C (dev). First
/// signal to fire wins; both are clean shutdown paths. Copied from
/// rustyred-thg-server/src/main.rs for clean Railway restarts.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
