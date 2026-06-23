//! The CommonPlace API HTTP surface, factored so it serves three callers from
//! one router: the standalone `commonplace-api` binary ([`run_from_env`]), the
//! durable acceptance tests, and an in-process embedder such as the Theorem
//! desktop shell ([`serve_loopback`]), which spawns a local instance the
//! CommonPlace frontend points at in desktop mode (SPEC-9).
//!
//! `POST /graphql` runs operations (gated by the `x-api-key` header), `GET
//! /graphql` serves GraphiQL, `GET /healthz` is liveness.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;

use async_graphql::http::GraphiQLSource;
use async_graphql::{EmptySubscription, Request, Schema};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use commonplace::{BlobStore, EmbeddingGraphStore};

use crate::{
    build_schema, in_memory_store, redcore_store, ApiKeyRegistry, ApiKeyToken, Mutation, Query,
};

struct AppState<S, B>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    schema: Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>,
    registry: Arc<ApiKeyRegistry>,
}

impl<S, B> Clone for AppState<S, B>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            schema: self.schema.clone(),
            registry: Arc::clone(&self.registry),
        }
    }
}

fn build_router<S, B>(store: crate::SharedStore<S, B>, registry: Arc<ApiKeyRegistry>) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let schema = build_schema(store, Arc::clone(&registry));
    let state = AppState { schema, registry };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/graphql", get(graphiql).post(graphql_handler::<S, B>))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

async fn graphiql() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

async fn graphql_handler<S, B>(
    State(state): State<AppState<S, B>>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> Result<GraphQLResponse, StatusCode>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .filter(|key| state.registry.resolve(key).is_some())
        .ok_or(StatusCode::FORBIDDEN)?;

    let request: Request = req.into_inner().data(ApiKeyToken(key.to_string()));
    Ok(state.schema.execute(request).await.into())
}

/// Run the standalone binary's server: env-driven backing (durable when
/// `COMMONPLACE_DATA_DIR` is set, else in-memory), bound on `[::]:PORT`.
pub async fn run_from_env() -> Result<(), String> {
    let api_key = std::env::var("COMMONPLACE_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let instance =
        std::env::var("COMMONPLACE_INSTANCE_ID").unwrap_or_else(|_| "default".to_string());
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key, instance));
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50090);

    let app = match std::env::var("COMMONPLACE_DATA_DIR") {
        Ok(dir) if !dir.is_empty() => {
            let store = redcore_store(&dir).map_err(|error| {
                format!("commonplace-api open durable store at {dir}: {error:?}")
            })?;
            build_router(store, registry)
        }
        _ => build_router(in_memory_store(), registry),
    };

    let listener = tokio::net::TcpListener::bind(("::", port))
        .await
        .map_err(|error| format!("commonplace-api bind [::]:{port}: {error}"))?;
    println!("commonplace-api listening on [::]:{port}");
    axum::serve(listener, app)
        .await
        .map_err(|error| format!("commonplace-api serve: {error}"))
}

/// Serve a durable CommonPlace instance on a loopback address for an in-process
/// embedder (the desktop shell). Backed by RedCore + disk under `data_dir`;
/// shuts down gracefully when `shutdown` resolves.
pub async fn serve_loopback(
    addr: SocketAddr,
    data_dir: impl AsRef<Path>,
    api_key: impl Into<String>,
    instance: impl Into<String>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let data_dir = data_dir.as_ref();
    let store = redcore_store(data_dir).map_err(|error| {
        format!(
            "commonplace-api open durable store at {}: {error:?}",
            data_dir.display()
        )
    })?;
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key.into(), instance.into()));
    let app = build_router(store, registry);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("commonplace-api bind {addr}: {error}"))?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|error| format!("commonplace-api serve: {error}"))
}
