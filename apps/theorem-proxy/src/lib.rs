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
pub mod membrane;
pub mod memory;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
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
    /// D2 native-tool membrane: max inline tool_result length before the latest turn's
    /// oversized results are sampled to a head+tail stub (full output served at
    /// `/tool_result/{id}`). `0` disables the membrane (the default).
    pub membrane_threshold: usize,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            upstream: "https://api.anthropic.com".to_string(),
            memory: None,
            max_memories: 8,
            membrane_threshold: 0,
        }
    }
}

#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    upstream: String,
    memory: Option<Arc<dyn memory::MemorySource>>,
    max_memories: usize,
    membrane_threshold: usize,
    /// Full content of tool_result blocks the membrane elided, keyed by retrieval id,
    /// served at `GET /tool_result/{id}`.
    tool_results: Arc<Mutex<HashMap<String, String>>>,
}

/// Build the proxy router (exposed for tests and for embedding behind another
/// listener). `/healthz` is liveness; `/v1/messages` is the passthrough.
pub fn router(config: ProxyConfig) -> Router {
    let client = reqwest::Client::builder()
        // Streaming model turns can legitimately run longer than a fixed total
        // response deadline; keep only the connection-establishment bound.
        .connect_timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");
    let state = ProxyState {
        client,
        upstream: config.upstream.trim_end_matches('/').to_string(),
        memory: config.memory,
        max_memories: config.max_memories,
        membrane_threshold: config.membrane_threshold,
        tool_results: Arc::new(Mutex::new(HashMap::new())),
    };
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/v1/messages", any(proxy_messages))
        .route("/tool_result/{id}", get(serve_tool_result))
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
    // D2: membrane the latest turn's oversized tool_result blocks (cache-safe: last
    // message only). The full content is stashed for out-of-band retrieval.
    let body = if state.membrane_threshold > 0 {
        let (membraned, stored) = crate::membrane::apply_membrane(&body, state.membrane_threshold);
        if !stored.is_empty() {
            if let Ok(mut store) = state.tool_results.lock() {
                for (id, content) in stored {
                    store.insert(id, content);
                }
            }
        }
        Bytes::from(membraned)
    } else {
        body
    };
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

/// Serve the full content of a tool_result the membrane elided (D2). Out-of-band
/// retrieval: the truncated stub points here. 404 if unknown or already evicted.
async fn serve_tool_result(
    State(state): State<ProxyState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Response {
    match state.tool_results.lock().ok().and_then(|store| store.get(&id).cloned()) {
        Some(content) => content.into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
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
///
/// If the proxy never comes up (commonly: the port is already in use), the wrapped command
/// is NOT launched -- returning an error beats pointing the child at a dead endpoint.
pub async fn run_wrapped(
    addr: SocketAddr,
    config: ProxyConfig,
    command: Vec<String>,
) -> std::io::Result<i32> {
    let (program, args) = command.split_first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "wrap: empty command")
    })?;
    // Bind the listener up front so a port already in use fails HERE -- this proves the
    // proxy we point the child at is OURS, not another process already answering /healthz
    // on the same port (whose memory/upstream settings would differ).
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| std::io::Error::other(format!("theorem-proxy could not bind {addr}: {error}")))?;
    let server = tokio::spawn(async move { axum::serve(listener, router(config)).await });

    // Wait until our server answers /healthz (it owns the port now); bail if the task dies.
    // The probe is time-bounded so a transient stall can't hang the wait.
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(1))
        .build()
        .expect("failed to build health-check client");
    let health = format!("http://{addr}/healthz");
    let mut healthy = false;
    for _ in 0..100 {
        if server.is_finished() {
            break;
        }
        if let Ok(response) = client.get(&health).send().await {
            if response.status().is_success() {
                healthy = true;
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !healthy {
        let detail = if server.is_finished() {
            match server.await {
                Ok(Err(error)) => error.to_string(),
                Ok(Ok(())) => "proxy exited before becoming healthy".to_string(),
                Err(join) => join.to_string(),
            }
        } else {
            server.abort();
            "proxy did not become healthy within the startup window".to_string()
        };
        return Err(std::io::Error::other(format!(
            "theorem-proxy could not start on {addr}: {detail}"
        )));
    }

    let status = tokio::process::Command::new(program)
        .args(args)
        .env("ANTHROPIC_BASE_URL", format!("http://{addr}"))
        .status()
        .await?;
    server.abort();
    Ok(status.code().unwrap_or(0))
}

/// One link in the local-stack chain check.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

impl Check {
    fn new(name: &str, ok: bool, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            ok,
            detail: detail.into(),
        }
    }
}

/// Probe the local stack (roadmap C.3 `theorem doctor`): the Valkey warm tier, the local
/// node (`/ready` + a real memory retrieval round-trip), and a running proxy. Each link
/// is probed independently; a down link is reported, never fatal. This is also where
/// B.5's value readout will hang. `valkey_addr` is `host:port`.
pub async fn doctor(
    memory_url: Option<&str>,
    proxy_url: Option<&str>,
    valkey_addr: Option<&str>,
) -> Vec<Check> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
        .expect("failed to build reqwest client");
    let mut checks = Vec::new();

    if let Some(addr) = valkey_addr {
        let ok = matches!(
            tokio::time::timeout(
                Duration::from_millis(500),
                tokio::net::TcpStream::connect(addr),
            )
            .await,
            Ok(Ok(_))
        );
        checks.push(Check::new(
            "valkey",
            ok,
            if ok {
                format!("reachable at {addr}")
            } else {
                format!("no TCP connect to {addr}")
            },
        ));
    }

    if let Some(url) = memory_url {
        let base = url.strip_suffix("/mcp").unwrap_or(url);
        let ready = client.get(format!("{base}/ready")).send().await;
        let ready_ok = ready.as_ref().map(|r| r.status().is_success()).unwrap_or(false);
        checks.push(Check::new(
            "node",
            ready_ok,
            match &ready {
                Ok(response) => format!("/ready -> {}", response.status()),
                Err(error) => format!("unreachable: {error}"),
            },
        ));

        let body = serde_json::json!({
            "jsonrpc": "2.0", "id": "doctor", "method": "tools/call",
            "params": {"name": "hippo_retrieve", "arguments": {"query": "doctor", "top_k": 1}}
        });
        let retrieval = client
            .post(url)
            .header("content-type", "application/json")
            .body(serde_json::to_vec(&body).unwrap_or_default())
            .send()
            .await;
        let retrieval_ok = retrieval
            .as_ref()
            .map(|r| r.status().is_success())
            .unwrap_or(false);
        checks.push(Check::new(
            "memory",
            retrieval_ok,
            match &retrieval {
                Ok(response) => format!("hippo_retrieve -> {}", response.status()),
                Err(error) => format!("retrieval failed: {error}"),
            },
        ));
    }

    if let Some(url) = proxy_url {
        let base = url.trim_end_matches('/');
        let health = client.get(format!("{base}/healthz")).send().await;
        let ok = health.as_ref().map(|r| r.status().is_success()).unwrap_or(false);
        checks.push(Check::new(
            "proxy",
            ok,
            match &health {
                Ok(response) => format!("/healthz -> {}", response.status()),
                Err(error) => format!("unreachable: {error}"),
            },
        ));
    }

    checks
}
