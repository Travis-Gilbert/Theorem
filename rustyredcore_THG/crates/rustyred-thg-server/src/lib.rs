#![recursion_limit = "512"]

mod agent_space;
mod auth;
mod bulk;
pub mod config;
mod coordination_push;
mod cypher;
mod graph_cache;
mod grpc;
mod metrics;
mod observability;
mod openapi;
mod query_surface;
pub mod router;
pub mod state;
mod ttl_sweep;

use std::net::SocketAddr;

pub use config::Config;
pub use state::AppState;

/// Run the THG HTTP/MCP surface as an embedded loopback service.
///
/// Desktop uses this instead of spawning the `rustyred-thg-server` binary so the
/// app remains the local node and owns shutdown.
pub async fn serve_loopback(
    config: Config,
    shutdown: tokio::sync::oneshot::Receiver<()>,
) -> std::io::Result<()> {
    config
        .validate()
        .map_err(|exc| std::io::Error::new(std::io::ErrorKind::InvalidInput, exc))?;
    let addr: SocketAddr = config
        .bind_addr()
        .parse()
        .map_err(|exc| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{exc}")))?;
    let ttl_sweep_ms = config.ttl_sweep_ms;
    let state = AppState::new(config);

    let sweep_handle = ttl_sweep::spawn_sweep_loop(state.clone(), ttl_sweep_ms);
    let wake_handle = coordination_push::spawn_wake_listener(state.clone());
    let app = router::build_router(state.clone())
        .merge(grpc::build_grpc_routes(state.clone()).into_axum_router());
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let serve_result = axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown.await;
        })
        .await;

    state.ttl_sweep.shutdown();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sweep_handle).await;
    wake_handle.abort();
    serve_result
}
