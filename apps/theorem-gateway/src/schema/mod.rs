//! GraphQL schema: root types, shared resolver helpers, and assembly.
//!
//! The schema is `Schema<Query, Mutation, EmptySubscription>`. Resolvers are
//! thin: each is one gRPC call mapped to a browser-facing type from `types`.
//! Cross-cutting concerns (context access, gRPC error mapping, cache-aside,
//! per-IP rate limiting) live here so the resolvers stay readable.

pub mod agent;
pub mod mutation;
pub mod query;
pub mod scene;
pub mod types;

use async_graphql::{Context, Enum, EmptySubscription, Schema};

use crate::cache::ResponseCache;
use crate::clients::{ClientIp, GatewayContext};
use crate::pb::search;

pub use mutation::Mutation;
pub use query::Query;
pub use scene::SceneStore;

/// The assembled gateway schema type.
pub type GatewaySchema = Schema<Query, Mutation, EmptySubscription>;

/// Browser-facing search mode, mapped to `theseus_search.v1.SearchMode`.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug, Default)]
pub enum SearchMode {
    /// Real-time UX. Single round, cheap rerank only.
    #[default]
    Ask,
    /// Deep research. Up to 3 gap-walk rounds, full rerank.
    Deep,
    /// Encode-time bulk retrieval.
    Encode,
    /// Civic-atlas: deep + source-paired.
    CivicAtlas,
}

impl SearchMode {
    /// Map to the proto enum's i32 discriminant for the gRPC request.
    pub fn as_proto(self) -> i32 {
        let mode = match self {
            SearchMode::Ask => search::SearchMode::Ask,
            SearchMode::Deep => search::SearchMode::Deep,
            SearchMode::Encode => search::SearchMode::Encode,
            SearchMode::CivicAtlas => search::SearchMode::CivicAtlas,
        };
        mode as i32
    }
}

/// Borrow the shared `GatewayContext` out of the resolver context.
pub(crate) fn gateway_ctx<'a>(ctx: &'a Context<'_>) -> async_graphql::Result<&'a GatewayContext> {
    ctx.data::<GatewayContext>()
}

/// Map a tonic transport/RPC error into a GraphQL error that is safe to return
/// to the browser (carries the gRPC status code + message, not internal addrs).
pub(crate) fn map_status(status: tonic::Status) -> async_graphql::Error {
    async_graphql::Error::new(format!(
        "upstream error [{:?}]: {}",
        status.code(),
        status.message()
    ))
}

/// Resolve the caller IP for rate limiting; "unknown" if the handler did not
/// inject one (e.g. introspection). All unknown callers share one bucket.
pub(crate) fn caller_ip(ctx: &Context<'_>) -> String {
    ctx.data::<ClientIp>()
        .map(|ip| ip.0.clone())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Enforce the per-IP token-bucket limit for a side-effecting / model resolver.
pub(crate) async fn enforce_rate_limit(ctx: &Context<'_>) -> async_graphql::Result<()> {
    let gw = gateway_ctx(ctx)?;
    let ip = caller_ip(ctx);
    if !gw.limiter.check(&ip).await {
        return Err(async_graphql::Error::new(
            "rate limit exceeded: too many requests from this client, slow down",
        ));
    }
    Ok(())
}

/// Cache-aside wrapper for recomputable read responses. On a cache hit the
/// stored JSON is returned; on a miss `compute` runs and its result is stored.
/// A no-op pass-through when caching is disabled (no `VALKEY_URL`).
pub(crate) async fn cached<T, Fut>(
    cache: &ResponseCache,
    key: &str,
    compute: Fut,
) -> async_graphql::Result<T>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
    Fut: std::future::Future<Output = async_graphql::Result<T>>,
{
    if cache.enabled() {
        if let Some(raw) = cache.get(key).await {
            if let Ok(parsed) = serde_json::from_str::<T>(&raw) {
                return Ok(parsed);
            }
        }
    }
    let value = compute.await?;
    if cache.enabled() {
        if let Ok(raw) = serde_json::to_string(&value) {
            cache.set(key, &raw).await;
        }
    }
    Ok(value)
}

/// Build the runnable schema with the gateway context attached as `Data`.
pub fn build_schema(context: GatewayContext) -> GatewaySchema {
    Schema::build(Query, Mutation, EmptySubscription)
        .data(context)
        .finish()
}

/// Build a schema with no context, only for SDL export (`--export-schema`).
/// SDL is derived from the type graph and never executes a resolver, so no
/// upstream clients are required.
pub fn schema_for_export() -> GatewaySchema {
    Schema::build(Query, Mutation, EmptySubscription).finish()
}

#[cfg(test)]
mod schema_tests {
    use super::*;

    /// Building the schema validates the whole type graph; asserting the SDL
    /// confirms the browser-facing surface (camelCase names) matches the spec.
    #[test]
    fn schema_assembles_with_expected_surface() {
        let sdl = schema_for_export().sdl();

        for field in [
            "search(",
            "gapWalk(",
            "provenance(",
            "searchCode(",
            "exploreCode(",
            "codeContext(",
            "explainCode(",
            "askAgent(",
            "sceneForInput(",
        ] {
            assert!(sdl.contains(field), "SDL missing query field {field}");
        }

        assert!(sdl.contains("type Mutation"));
        assert!(sdl.contains("ingestCodebase("));
        assert!(sdl.contains("reindexCodebase("));

        for ty in [
            "type Query",
            "type KnowledgeGraph",
            "type GraphNode",
            "type GraphEdge",
            "type SearchHit",
            "type CodeSymbol",
            "type CodeContextBlock",
            "type IngestReceipt",
            "type AgentAnswer",
            "contextNodes",
            "input AgentScope",
            "enum SearchMode",
            "type SceneRef",
            "input OriginInput",
        ] {
            assert!(sdl.contains(ty), "SDL missing type/field {ty}");
        }
    }
}
