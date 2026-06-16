#![recursion_limit = "512"]

mod agent_space;
mod auth;
mod bulk;
mod config;
mod coordination_push;
mod cypher;
mod graph_cache;
mod grpc;
mod metrics;
mod observability;
mod openapi;
mod query_surface;
mod router;
mod state;
mod ttl_sweep;

use std::net::SocketAddr;

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt().with_env_filter("info").init();
    let config = Config::from_env();
    config
        .validate()
        .map_err(|exc| std::io::Error::new(std::io::ErrorKind::InvalidInput, exc))?;
    let addr: SocketAddr = config
        .bind_addr()
        .parse()
        .map_err(|exc| std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{exc}")))?;
    let ttl_sweep_ms = config.ttl_sweep_ms;
    let state = AppState::new(config);

    // TTL-04: spawn the background TTL sweep BEFORE serving traffic
    // so the first request sees an active sweep loop. The spawn returns
    // a JoinHandle we'll await on shutdown so the process doesn't exit
    // mid-AOF write. The loop is cancellable via state.ttl_sweep.shutdown().
    let sweep_handle = ttl_sweep::spawn_sweep_loop(state.clone(), ttl_sweep_ms);
    let wake_handle = coordination_push::spawn_wake_listener(state.clone());
    tracing::info!(ttl_sweep_ms, "TTL background sweep started");

    let http_router = router::build_router(state.clone());
    let grpc_router = grpc::build_grpc_routes(state.clone()).into_axum_router();
    let app = http_router.merge(grpc_router);

    tracing::info!("RUSTYRED_THG_PRODUCT_READY {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Run the axum server until SIGTERM/SIGINT, then signal sweep
    // shutdown and await the sweep loop's clean exit before returning.
    let serve_future = axum::serve(listener, app).with_graceful_shutdown(shutdown_signal());
    let serve_result = serve_future.await;

    tracing::info!("HTTP server stopped; signaling TTL sweep shutdown");
    state.ttl_sweep.shutdown();
    // Bound the wait so a stuck sweep tick doesn't hang the process.
    // Sweep ticks should complete in <1s under normal conditions; the
    // 5s bound gives generous headroom for slow disk fsync.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sweep_handle).await;
    wake_handle.abort();
    tracing::info!("TTL sweep loop exited; process shutting down");

    serve_result
}

/// Wait for SIGTERM (production / Docker / Railway) or Ctrl-C (dev).
/// First signal to fire wins; both are clean shutdown paths.
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
