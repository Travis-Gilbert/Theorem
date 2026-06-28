//! theorem-proxy: local Anthropic Messages passthrough proxy.
//!
//! SPEC-LOCAL-PROXY-MVP deliverable 1. A faithful local reverse proxy that sits on
//! every Claude Code (or any Anthropic-Messages client) turn: `POST /v1/messages`
//! forwards to the configured upstream (default `https://api.anthropic.com`),
//! streaming and non-streaming, with the client's headers, body, and SSE event
//! stream preserved byte-for-byte. Nothing in the request or response is parsed or
//! mutated here -- so `tool_use` ids, the `anthropic-beta` header (including the
//! OAuth subscription capability), and prompt-cache breakpoints all survive intact.
//!
//! This is the foundation. The native-tool membrane (D2) and ambient memory /
//! directive injection (D3) extend the request path on top of this passthrough; the
//! governing rule they must keep is the one this layer trivially satisfies by doing
//! nothing: never mutate the cached prefix (system, tools), and fail open.

pub mod inject;
pub mod memory;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::http::{HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get};
use axum::Router;

/// How the proxy reaches the upstream model API, and what it injects.
#[derive(Clone)]
pub struct ProxyConfig {
    /// Upstream base URL the `/v1/messages` path is forwarded to.
    pub upstream: String,
    /// Ambient memory source (D3). `None` is faithful passthrough (D1); `Some`
    /// injects relevant memory at the cache-stable suffix of each turn.
    pub memory: Option<Arc<dyn memory::MemorySource>>,
    /// Maximum memories injected per turn.
    pub max_memories: usize,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstream: "https://api.anthropic.com".to_string(),
            memory: None,
            max_memories: 8,
        }
    }
}

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    upstream: String,
    memory: Option<Arc<dyn memory::MemorySource>>,
    max_memories: usize,
}

/// Build the proxy router (exposed for tests and for embedding behind another
/// listener). `/healthz` is liveness; `/v1/messages` is the passthrough.
pub fn router(config: ProxyConfig) -> Router {
    let client = reqwest::Client::builder()
        // A model turn can stream for minutes; do not impose a short timeout.
        .timeout(Duration::from_secs(600))
        .build()
        .expect("failed to build reqwest client");
    let state = ProxyState {
        client,
        upstream: config.upstream.trim_end_matches('/').to_string(),
        memory: config.memory,
        max_memories: config.max_memories,
    };
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/messages", any(proxy_messages))
        .with_state(state)
}

/// Bind `addr` and serve the proxy until the process is stopped.
pub async fn serve(addr: SocketAddr, config: ProxyConfig) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, router(config)).await
}

async fn proxy_messages(
    State(state): State<ProxyState>,
    method: Method,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    // D3: inject relevant memory at the cache-stable suffix (the last user turn),
    // never into system or tools. Fail open -- an unparseable body, or one with no
    // relevant memory, is forwarded unchanged.
    let body = match &state.memory {
        Some(source) => Bytes::from(crate::inject::inject_memory(
            &body,
            source.as_ref(),
            state.max_memories,
        )),
        None => body,
    };
    let url = format!("{}/v1/messages", state.upstream);
    // Forward the client's request headers verbatim, except hop-by-hop headers and
    // the ones reqwest must recompute for the new connection. Crucially this keeps
    // `authorization` / `x-api-key`, `anthropic-version`, and `anthropic-beta`.
    // `append` (not `insert`) preserves any repeated header values.
    let mut forward = HeaderMap::new();
    for (name, value) in headers.iter() {
        if !is_hop_by_hop(name) {
            forward.append(name.clone(), value.clone());
        }
    }
    let builder = state.client.request(method, &url).headers(forward).body(body);

    let upstream = match builder.send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                format!("theorem-proxy: upstream request failed: {error}"),
            )
                .into_response();
        }
    };

    // Faithful response passthrough: status, headers (minus hop-by-hop), and the
    // body streamed straight through. For an SSE turn this pipes events as they
    // arrive; it is never buffered.
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .unwrap_or(StatusCode::BAD_GATEWAY);
    let mut response_headers = HeaderMap::new();
    for (name, value) in upstream.headers().iter() {
        if is_hop_by_hop(name) {
            continue;
        }
        response_headers.insert(name.clone(), value.clone());
    }
    let mut response = (status, Body::from_stream(upstream.bytes_stream())).into_response();
    *response.headers_mut() = response_headers;
    response
}

/// Hop-by-hop headers (and the ones the proxied connection recomputes) that must
/// not be forwarded in either direction.
fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "host"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "content-length"
    )
}

/// Build the ambient `MemorySource` from the CLI/env options, plus a human description
/// of what was selected. A live node URL wins over a directory; neither set is faithful
/// passthrough. Shared by the `proxy` and `wrap` subcommands.
pub fn resolve_memory(
    memory_url: Option<&str>,
    tenant: Option<String>,
    memory_dir: Option<&std::path::Path>,
) -> (Option<Arc<dyn memory::MemorySource>>, String) {
    if let Some(url) = memory_url {
        (
            Some(Arc::new(memory::HttpMemorySource::new(url.to_string(), tenant))),
            format!("live local node memory at {url}"),
        )
    } else if let Some(dir) = memory_dir {
        (
            Some(Arc::new(memory::DirectoryMemorySource::new(dir))),
            format!("relevant memory from {}", dir.display()),
        )
    } else {
        (None, "off (faithful passthrough)".to_string())
    }
}

/// Serve the proxy on `addr`, wait until it answers `/healthz`, then run `command` with
/// `ANTHROPIC_BASE_URL` pointed at it; return the child's exit code. One command instead
/// of a manual base-URL export (SPEC-LOCAL-PROXY-MVP D5 / one-click connect).
pub async fn run_wrapped(
    addr: SocketAddr,
    config: ProxyConfig,
    command: Vec<String>,
) -> std::io::Result<i32> {
    let (program, args) = command.split_first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "wrap: empty command")
    })?;
    // Serve in the background; aborted when the wrapped command exits.
    let server = tokio::spawn(serve(addr, config));
    wait_until_healthy(addr).await;
    let status = tokio::process::Command::new(program)
        .args(args)
        .env("ANTHROPIC_BASE_URL", format!("http://{addr}"))
        .status()
        .await?;
    server.abort();
    Ok(status.code().unwrap_or(0))
}

/// Poll `/healthz` until the proxy answers (bounded ~5s). Best-effort: if it never comes
/// up the command still runs and surfaces its own connection error.
async fn wait_until_healthy(addr: SocketAddr) {
    let client = reqwest::Client::new();
    let url = format!("http://{addr}/healthz");
    for _ in 0..100 {
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                return;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
