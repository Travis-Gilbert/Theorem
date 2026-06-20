//! The CommonPlace API HTTP server (plan unit F3 + durable backing).
//!
//! Serves the consumer GraphQL profile over one instance. A client connects with
//! this instance URL plus a key (the `x-api-key` header). `POST /graphql` runs
//! operations, `GET /graphql` serves GraphiQL, `GET /healthz` is liveness.
//!
//! The seed API key comes from `COMMONPLACE_API_KEY` (default `dev-key` for
//! local use); the bind port from `PORT` (default 50090). Set
//! `COMMONPLACE_DATA_DIR` to persist durably (RedCore + disk under that dir);
//! unset = an ephemeral in-memory instance. The schema is generic over the
//! backing, so the same handlers serve either.

use std::path::PathBuf;
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
use commonplace_api::{
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

// Manual Clone so we do not require S: Clone / B: Clone (the Schema and Arc are
// Clone regardless of the backing).
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

#[tokio::main]
async fn main() {
    let api_key = std::env::var("COMMONPLACE_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let instance =
        std::env::var("COMMONPLACE_INSTANCE_ID").unwrap_or_else(|_| "default".to_string());
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key, instance));
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50090);

    match std::env::var("COMMONPLACE_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let store = redcore_store(PathBuf::from(&dir)).expect("open durable store");
            let schema = build_schema(store, Arc::clone(&registry));
            println!("commonplace-api (durable: {dir}) listening on 0.0.0.0:{port}");
            serve(AppState { schema, registry }, port).await;
        }
        _ => {
            let schema = build_schema(in_memory_store(), Arc::clone(&registry));
            println!("commonplace-api (in-memory) listening on 0.0.0.0:{port}");
            serve(AppState { schema, registry }, port).await;
        }
    }
}

async fn serve<S, B>(state: AppState<S, B>, port: u16)
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    let app = Router::new()
        .route("/graphql", get(graphiql).post(graphql_handler::<S, B>))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind commonplace-api port");
    axum::serve(listener, app)
        .await
        .expect("serve commonplace-api");
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
