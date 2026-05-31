//! Theorem's first gRPC server.
//!
//! Serves theseus_search.v1.SearchService over the RustyRed substrate. Pure
//! gRPC (no HTTP surface): the smaller server. Binds 0.0.0.0:$PORT so Railway's
//! IPv4 healthcheck/router reaches it. The civic-atlas-server dials this by
//! setting THEOREM_SEARCH_URL.

mod engine;
mod pb;
mod service;

use std::net::SocketAddr;
use std::sync::Arc;

use engine::Engine;
use pb::SearchServiceServer;
use service::TheoremSearchService;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_env_filter("info").init();

    // Railway injects PORT. Default 50071 for local dev (a free gRPC-ish port).
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(50071);

    // Bind 0.0.0.0 (not [::]) so Railway's IPv4 healthcheck/router reaches it.
    let addr: SocketAddr = format!("0.0.0.0:{port}").parse()?;

    // Build the engine (empty substrate is the honest slice-1 default) and wrap
    // it in an Arc so the owned store outlives every borrowing handler call.
    let engine = Arc::new(Engine::new());
    let svc = SearchServiceServer::new(TheoremSearchService::new(engine));

    tracing::info!("THEOREM_GRPC_READY {}", addr);

    tonic::transport::Server::builder()
        .add_service(svc)
        .serve_with_shutdown(addr, shutdown_signal())
        .await?;

    tracing::info!("theorem-grpc server stopped");
    Ok(())
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
