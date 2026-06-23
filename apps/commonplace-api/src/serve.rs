//! Shared HTTP serving surface for the CommonPlace API.
//!
//! The standalone binary and the desktop embedder both serve the same GraphQL
//! contract. The binary uses environment-driven configuration; the desktop uses
//! [`serve_loopback`] with a durable local data directory and graceful shutdown.

use std::future::Future;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{mpsc::SyncSender, Arc};

use async_graphql::http::GraphiQLSource;
use async_graphql::{EmptySubscription, Request, Schema};
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use commonplace::{BlobStore, EmbeddingGraphStore};
use tower_http::cors::CorsLayer;

use crate::{
    answer_model_from_env, build_schema, build_schema_with_model, in_memory_store, redcore_store,
    AnswerModel, ApiKeyRegistry, ApiKeyToken, Mutation, Query, SharedStore,
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

pub fn build_router<S, B>(store: SharedStore<S, B>, registry: Arc<ApiKeyRegistry>) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let schema = build_schema(store, Arc::clone(&registry));
    build_public_router_from_schema(schema, registry)
}

pub fn build_router_with_model<S, B>(
    store: SharedStore<S, B>,
    registry: Arc<ApiKeyRegistry>,
    model: Arc<dyn AnswerModel>,
) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let schema = build_schema_with_model(store, Arc::clone(&registry), model);
    build_public_router_from_schema(schema, registry)
}

fn build_public_router_from_schema<S, B>(
    schema: Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>,
    registry: Arc<ApiKeyRegistry>,
) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let state = AppState { schema, registry };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/graphql", get(graphiql).post(graphql_handler::<S, B>))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

fn build_loopback_router_from_schema<S, B>(
    schema: Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>,
    registry: Arc<ApiKeyRegistry>,
) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let state = AppState { schema, registry };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/graphql", post(graphql_handler::<S, B>))
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

pub async fn run_from_env() -> Result<(), String> {
    let api_key = std::env::var("COMMONPLACE_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let instance =
        std::env::var("COMMONPLACE_INSTANCE_ID").unwrap_or_else(|_| "default".to_string());
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key, instance));
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50090);
    let model = answer_model_from_env();

    let app = match std::env::var("COMMONPLACE_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let store = redcore_store(&dir).map_err(|error| {
                format!("commonplace-api open durable store at {dir}: {error:?}")
            })?;
            build_router_with_model(store, registry, Arc::clone(&model))
        }
        _ => build_router_with_model(in_memory_store(), registry, model),
    };

    let listener = tokio::net::TcpListener::bind(("::", port))
        .await
        .map_err(|error| format!("commonplace-api bind [::]:{port}: {error}"))?;
    println!("commonplace-api listening on [::]:{port}");
    axum::serve(listener, app)
        .await
        .map_err(|error| format!("commonplace-api serve: {error}"))
}

async fn prepare_loopback_server(
    addr: SocketAddr,
    data_dir: impl AsRef<Path>,
    api_key: impl Into<String>,
    instance: impl Into<String>,
) -> Result<(Router, tokio::net::TcpListener), String> {
    let data_dir = data_dir.as_ref();
    let store = redcore_store(data_dir).map_err(|error| {
        format!(
            "commonplace-api open durable store at {}: {error:?}",
            data_dir.display()
        )
    })?;
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key.into(), instance.into()));
    let app = build_loopback_router_from_schema(
        build_schema_with_model(store, Arc::clone(&registry), answer_model_from_env()),
        registry,
    );
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| format!("commonplace-api bind {addr}: {error}"))?;
    Ok((app, listener))
}

pub async fn serve_loopback(
    addr: SocketAddr,
    data_dir: impl AsRef<Path>,
    api_key: impl Into<String>,
    instance: impl Into<String>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    let (app, listener) = prepare_loopback_server(addr, data_dir, api_key, instance).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(|error| format!("commonplace-api serve: {error}"))
}

pub async fn serve_loopback_with_ready(
    addr: SocketAddr,
    data_dir: impl AsRef<Path>,
    api_key: impl Into<String>,
    instance: impl Into<String>,
    ready: SyncSender<Result<(), String>>,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), String> {
    match prepare_loopback_server(addr, data_dir, api_key, instance).await {
        Ok((app, listener)) => {
            let _ = ready.send(Ok(()));
            axum::serve(listener, app)
                .with_graceful_shutdown(shutdown)
                .await
                .map_err(|error| format!("commonplace-api serve: {error}"))
        }
        Err(error) => {
            let _ = ready.send(Err(error.clone()));
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use axum::http::{header, StatusCode};
    use reqwest::Method;
    use tokio::sync::oneshot;

    use super::prepare_loopback_server;

    #[tokio::test]
    async fn loopback_router_does_not_allow_cross_origin_graphql_preflight() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let data_dir = std::env::current_dir()
            .expect("cwd")
            .join("target")
            .join(format!("loopback-cors-{unique}"));
        std::fs::create_dir_all(&data_dir).expect("create data dir");

        let (app, listener) = prepare_loopback_server(
            ([127, 0, 0, 1], 0).into(),
            &data_dir,
            "loopback-test-key",
            "default",
        )
        .await
        .expect("prepare loopback server");
        let port = listener.local_addr().expect("listener addr").port();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve loopback test server");
        });

        tokio::time::sleep(Duration::from_millis(50)).await;
        let client = reqwest::Client::new();
        let response = client
            .request(Method::OPTIONS, format!("http://127.0.0.1:{port}/graphql"))
            .header(header::ORIGIN, "https://evil.example")
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "x-api-key,content-type",
            )
            .send()
            .await
            .expect("send preflight");
        assert!(
            response
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .is_none(),
            "loopback GraphQL must not opt into cross-origin browser access"
        );

        let get = client
            .get(format!("http://127.0.0.1:{port}/graphql"))
            .send()
            .await
            .expect("send graphiql probe");
        assert_eq!(get.status(), StatusCode::METHOD_NOT_ALLOWED);

        let _ = shutdown_tx.send(());
        let _ = server.await;
        let _ = std::fs::remove_dir_all(&data_dir);
    }
}
