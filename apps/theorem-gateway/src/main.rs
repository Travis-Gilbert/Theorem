//! Theorem's browser-facing GraphQL gateway.
//!
//! An Axum + async-graphql server whose resolvers call theorem-grpc
//! (theseus_search.v1.SearchService + theorem_code.v1.CodeCrawlerService) and
//! the GL-Fusion model endpoint, and shape the results for the web. It stores
//! nothing durable: a front door and a translator. Mirrors
//! our-civic-atlas-backend's GraphQL-over-gRPC shape.
//!
//! Routes:
//!   POST /graphql   GraphQL operations
//!   GET  /graphql   GraphiQL playground
//!   GET  /healthz   liveness ("ok")
//!
//! `--export-schema` prints the SDL and exits (for frontend codegen).

mod cache;
mod clients;
mod config;
mod middleware;
mod pb;
mod scene_serve;
mod schema;

use std::net::SocketAddr;
use std::sync::Arc;

use async_graphql::http::GraphiQLSource;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::{ConnectInfo, State};
use axum::http::HeaderMap;
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use tower_http::trace::TraceLayer;

use crate::cache::{connect_valkey, RateLimiter, ResponseCache};
use crate::clients::{ClientIp, GatewayContext};
use crate::config::GatewayConfig;
use crate::schema::{build_schema, schema_for_export, GatewaySchema, SceneStore};

/// Shared axum state: the GraphQL schema plus the scene store. The
/// `GET /scene/{id}` handler reads the same `Arc<SceneStore>` the
/// `sceneForInput` resolver writes.
#[derive(Clone)]
pub struct AppState {
    pub schema: GatewaySchema,
    pub scenes: Arc<SceneStore>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // --export-schema: emit SDL and exit (no server, no upstreams needed).
    if std::env::args().any(|arg| arg == "--export-schema") {
        print!("{}", schema_for_export().sdl());
        return Ok(());
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let config = Arc::new(GatewayConfig::from_env());

    // Optional Valkey for rate-limit counters + recomputable response caching.
    // Falls back to in-memory if unset or unreachable (accelerator, not a dep).
    let valkey_conn = match config.valkey_url.as_deref() {
        Some(url) => connect_valkey(url).await,
        None => {
            tracing::info!("VALKEY_URL unset: in-memory rate limiting, no response cache");
            None
        }
    };
    let limiter = Arc::new(RateLimiter::new(
        config.rate_limit_burst,
        config.rate_limit_per_minute,
        valkey_conn.clone(),
    ));
    let cache = ResponseCache::new(valkey_conn, config.valkey_cache_ttl);

    // Shared bounded scene store: the resolver writes, the /scene handler reads.
    let scenes = Arc::new(SceneStore::new(config.scene_cache_size));

    let context = GatewayContext::new(config.clone(), limiter, cache, scenes.clone())?;
    if context.model.is_configured() {
        tracing::info!("GL-Fusion endpoint configured for askAgent");
    } else {
        tracing::warn!(
            "GLFUSION_URL unset: askAgent returns assembled graph context with an honest \
             'model not configured' answer"
        );
    }
    let gql_schema = build_schema(context);
    let app_state = AppState {
        schema: gql_schema,
        scenes,
    };

    let cors = middleware::build_cors_layer(&config.cors_allow_origins);

    let app = Router::new()
        .route("/graphql", get(graphiql).post(graphql_handler))
        .route("/scene/{scene_id}", get(scene_serve::serve_scene))
        .route("/healthz", get(healthz))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    // Spec: bind 0.0.0.0:$PORT (public HTTP surface; the website calls the
    // gateway's public domain, the gateway dials theorem-grpc privately).
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("THEOREM_GATEWAY_READY {addr} -> grpc {}", config.grpc_url);

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    tracing::info!("theorem-gateway server stopped");
    Ok(())
}

/// Execute a GraphQL operation. Injects the resolved client IP into the request
/// context so rate-limited resolvers can key the limiter per IP.
async fn graphql_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: GraphQLRequest,
) -> GraphQLResponse {
    let ip = middleware::client_ip(&headers, peer);
    let request = req.into_inner().data(ClientIp(ip));
    state.schema.execute(request).await.into()
}

/// GraphiQL playground for interactive exploration at GET /graphql.
async fn graphiql() -> impl IntoResponse {
    Html(
        GraphiQLSource::build()
            .endpoint("/graphql")
            .title("Theorem Gateway")
            .finish(),
    )
}

/// Liveness probe. Railway healthcheckPath = /healthz.
async fn healthz() -> &'static str {
    "ok"
}

/// Clean shutdown on SIGTERM (Railway/Docker) or Ctrl-C (dev).
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
