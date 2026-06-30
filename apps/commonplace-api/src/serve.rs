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
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::Router;
use commonplace::{BlobStore, EmbeddingGraphStore};
use tower_http::cors::CorsLayer;

use crate::{
    answer_model_from_env, build_schema, build_schema_with_model,
    build_schema_with_model_and_repository_connector, connector_from_env, in_memory_store,
    redcore_store, AnswerModel, ApiKeyRegistry, ApiKeyToken, Mutation, Query,
    RepositoryConnectorRef, SharedStore,
};

struct AppState<S, B>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    schema: Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>,
    registry: Arc<ApiKeyRegistry>,
    repository_connector: Option<RepositoryConnectorRef>,
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
            repository_connector: self.repository_connector.clone(),
        }
    }
}

pub fn build_router<S, B>(store: SharedStore<S, B>, registry: Arc<ApiKeyRegistry>) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let schema = build_schema(store, Arc::clone(&registry));
    build_public_router_from_schema(schema, registry, None)
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
    build_public_router_from_schema(schema, registry, None)
}

pub fn build_router_with_model_and_repository_connector<S, B>(
    store: SharedStore<S, B>,
    registry: Arc<ApiKeyRegistry>,
    model: Arc<dyn AnswerModel>,
    repository_connector: Option<RepositoryConnectorRef>,
) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let schema = build_schema_with_model_and_repository_connector(
        store,
        Arc::clone(&registry),
        model,
        repository_connector.clone(),
    );
    build_public_router_from_schema(schema, registry, repository_connector)
}

fn build_public_router_from_schema<S, B>(
    schema: Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>,
    registry: Arc<ApiKeyRegistry>,
    repository_connector: Option<RepositoryConnectorRef>,
) -> Router
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let state = AppState {
        schema,
        registry,
        repository_connector,
    };
    Router::new()
        .route("/healthz", get(healthz))
        .route("/metrics", get(metrics_handler::<S, B>))
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
    let state = AppState {
        schema,
        registry,
        repository_connector: None,
    };
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

async fn metrics_handler<S, B>(
    State(state): State<AppState<S, B>>,
) -> Result<impl IntoResponse, StatusCode>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let metrics = state
        .repository_connector
        .as_ref()
        .and_then(|connector| connector.audit_prometheus())
        .ok_or(StatusCode::NOT_FOUND)?;
    Ok((
        [(header::CONTENT_TYPE, "text/plain; version=0.0.4")],
        metrics,
    ))
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
    let repository_connector = connector_from_env()?;

    let app = match std::env::var("COMMONPLACE_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let store = redcore_store(&dir).map_err(|error| {
                format!("commonplace-api open durable store at {dir}: {error:?}")
            })?;
            build_router_with_model_and_repository_connector(
                store,
                registry,
                Arc::clone(&model),
                repository_connector,
            )
        }
        _ => build_router_with_model_and_repository_connector(
            in_memory_store(),
            registry,
            model,
            repository_connector,
        ),
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
    repository_connector: Option<RepositoryConnectorRef>,
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
        build_schema_with_model_and_repository_connector(
            store,
            Arc::clone(&registry),
            answer_model_from_env(),
            repository_connector,
        ),
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
    let (app, listener) = prepare_loopback_server(addr, data_dir, api_key, instance, None).await?;
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
    match prepare_loopback_server(addr, data_dir, api_key, instance, None).await {
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
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use axum::http::{header, StatusCode};
    use reqwest::Method;
    use tokio::sync::oneshot;

    use super::{build_router_with_model_and_repository_connector, prepare_loopback_server};
    use crate::{
        in_memory_store, ApiKeyRegistry, NoModel, RepositoryConnectInput, RepositoryConnectReceipt,
        RepositoryConnector,
    };

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
            None,
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

    #[tokio::test]
    async fn public_router_exposes_repository_mirror_metrics_when_configured() {
        let registry = Arc::new(ApiKeyRegistry::new().with_key("metrics-key", "default"));
        let app = build_router_with_model_and_repository_connector(
            in_memory_store(),
            registry,
            Arc::new(NoModel),
            Some(Arc::new(StaticMetricsConnector)),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind metrics test");
        let port = listener.local_addr().expect("listener addr").port();
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .expect("serve metrics test");
        });

        let body = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
            .await
            .expect("GET metrics")
            .text()
            .await
            .expect("metrics body");
        assert!(
            body.contains("rustyred_workspace_mirror_audit_divergence_count 3"),
            "mirror audit metrics should be exposed on /metrics: {body}"
        );

        let _ = shutdown_tx.send(());
        let _ = server.await;
    }

    struct StaticMetricsConnector;

    impl RepositoryConnector for StaticMetricsConnector {
        fn connect_repository(
            &self,
            _input: RepositoryConnectInput,
        ) -> Result<RepositoryConnectReceipt, String> {
            Err("not used".to_string())
        }

        fn audit_prometheus(&self) -> Option<String> {
            Some("rustyred_workspace_mirror_audit_divergence_count 3\n".to_string())
        }
    }
}
