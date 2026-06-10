//! Theorem's first gRPC server.
//!
//! Serves theseus_search.v1.SearchService over the RustyRed substrate. Pure
//! gRPC (no HTTP surface): the smaller server. Binds [::]:$PORT (IPv6
//! dual-stack) so Railway's private network (IPv6) reaches it via
//! theorem-grpc.railway.internal, and IPv4 healthchecks work too. The
//! civic-atlas-server dials this by setting THEOREM_SEARCH_URL (or the legacy
//! THESEUS_BRIDGE_URL).

mod app_affordance;
mod code_index;
mod code_service;
mod engine;
mod pb;
mod service;
mod session_delta;

use std::net::SocketAddr;
use std::sync::Arc;

use app_affordance::TheoremAppAffordanceService;
use code_index::CodeIndexRuntime;
use code_service::TheoremCodeCrawlerService;
use engine::Engine;
use pb::{AppAffordanceServiceServer, CodeCrawlerServiceServer, SearchServiceServer};
use service::TheoremSearchService;

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
    // ONE code store for the whole service: this CodeIndexRuntime (RedCore at
    // code_index_data_dir()) is shared by BOTH CodeCrawlerService and
    // AppAffordanceService below. The harness MCP `compute_code` reaches it
    // through ProductMcpBackend's default `invoke_code_search`, which wraps
    // the call as a `theorem_grpc.code_search.*` app affordance and dials this
    // server, so MCP ingest writes and MCP search reads land on the same
    // store the gRPC verbs use. (The in-process plugin MCP path writes its
    // own tenant store directly; both routes keep ingest and search on one
    // store.)
    let code_index = CodeIndexRuntime::try_new().map_err(std::io::Error::other)?;
    let search_svc = SearchServiceServer::new(TheoremSearchService::new(engine));
    let code_svc =
        CodeCrawlerServiceServer::new(TheoremCodeCrawlerService::new(code_index.clone()));
    let app_affordance_svc = AppAffordanceServiceServer::new(
        TheoremAppAffordanceService::try_new_with_code_index(code_index)
            .map_err(std::io::Error::other)?,
    );

    tracing::info!("THEOREM_GRPC_READY {}", addr);

    tonic::transport::Server::builder()
        .add_service(search_svc)
        .add_service(code_svc)
        .add_service(app_affordance_svc)
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
