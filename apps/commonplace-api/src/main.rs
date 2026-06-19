//! The CommonPlace API HTTP server (plan unit F3).
//!
//! Serves the consumer GraphQL profile over one instance. A client connects with
//! this instance URL plus a key (the `x-api-key` header). `POST /graphql` runs
//! operations, `GET /graphql` serves GraphiQL, `GET /healthz` is liveness.
//!
//! The seed API key comes from `COMMONPLACE_API_KEY` (default `dev-key` for
//! local use); the bind port from `PORT` (default 50090). Backing is the
//! in-memory store for this slice; a durable backing is the named follow-up.

use std::sync::Arc;

use async_graphql::http::GraphiQLSource;
use async_graphql::Request;
use async_graphql_axum::{GraphQLRequest, GraphQLResponse};
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse};
use axum::routing::get;
use axum::Router;
use commonplace_api::{build_schema, in_memory_store, ApiKeyRegistry, ApiKeyToken, ConsumerSchema};

#[derive(Clone)]
struct AppState {
    schema: ConsumerSchema,
    registry: Arc<ApiKeyRegistry>,
}

#[tokio::main]
async fn main() {
    let api_key = std::env::var("COMMONPLACE_API_KEY").unwrap_or_else(|_| "dev-key".to_string());
    let instance =
        std::env::var("COMMONPLACE_INSTANCE_ID").unwrap_or_else(|_| "default".to_string());
    let registry = Arc::new(ApiKeyRegistry::new().with_key(api_key, instance));
    let schema = build_schema(in_memory_store(), Arc::clone(&registry));
    let state = AppState { schema, registry };

    let app = Router::new()
        .route("/graphql", get(graphiql).post(graphql_handler))
        .route("/healthz", get(|| async { "ok" }))
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(50090);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind commonplace-api port");
    println!("commonplace-api listening on 0.0.0.0:{port}");
    axum::serve(listener, app)
        .await
        .expect("serve commonplace-api");
}

async fn graphiql() -> impl IntoResponse {
    Html(GraphiQLSource::build().endpoint("/graphql").finish())
}

async fn graphql_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    req: GraphQLRequest,
) -> Result<GraphQLResponse, StatusCode> {
    let key = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .filter(|key| state.registry.resolve(key).is_some())
        .ok_or(StatusCode::FORBIDDEN)?;

    let mut request: Request = req.into_inner();
    request = request.data(ApiKeyToken(key.to_string()));
    Ok(state.schema.execute(request).await.into())
}
