//! Local Anthropic Messages proxy for Claude Code and compatible clients.
//!
//! The proxy keeps the Anthropic credential on the local model path and uses the
//! harness credential for optional ambient retrieval and proxy-resident tool
//! execution. Ambient context is injected into the latest user turn; resident
//! capabilities are injected as hidden gateway tools and consumed locally before
//! the client sees a response.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::body::{Body, Bytes};
use axum::extract::{Query, State};
use axum::http::header::{CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::TryStreamExt;
use rustyred_thg_core::{now_ms, ActorId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_copresence::{CodeEditFootprint, FileRange, PresenceKind};

use crate::config::AgentdConfig;
use crate::{AgentdError, AgentdResult};

pub const DEFAULT_PROXY_PORT: u16 = 8484;
pub const DEFAULT_ANTHROPIC_UPSTREAM: &str = "https://api.anthropic.com";
pub const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;
pub const DEFAULT_AMBIENT_BUDGET_BYTES: usize = 4 * 1024;
pub const DEFAULT_RESIDENT_MAX_ROUNDS: usize = 4;

static TOOL_RESULT_BODIES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

#[derive(Clone, Debug, Default)]
pub struct ProxyCli {
    pub bind: Option<IpAddr>,
    pub port: Option<u16>,
    pub data_dir: Option<PathBuf>,
    pub upstream_base_url: Option<String>,
    pub harness_mcp_url: Option<String>,
    pub room_id: Option<String>,
    pub enable_ambient: Option<bool>,
}

#[derive(Clone, Debug)]
pub struct ProxyConfig {
    pub bind: IpAddr,
    pub port: u16,
    pub data_dir: PathBuf,
    pub upstream_base_url: String,
    pub harness_mcp_url: String,
    pub harness_bearer: Option<String>,
    pub harness_token_env: Option<String>,
    pub tenant_slug: String,
    pub default_room_id: String,
    pub enable_ambient: bool,
    pub resident_capabilities_enabled: bool,
    pub local_upstream_base_url: Option<String>,
    pub cascade_calibration_path: Option<PathBuf>,
    pub verification_claims_path: Option<PathBuf>,
    pub resident_max_rounds: usize,
    pub tool_result_budget_bytes: usize,
    pub ambient_budget_bytes: usize,
}

impl ProxyConfig {
    pub fn from_agentd(config: &AgentdConfig, cli: &ProxyCli) -> Self {
        let data_dir = cli
            .data_dir
            .clone()
            .or_else(|| std::env::var_os("THEOREM_PROXY_DATA_DIR").map(PathBuf::from))
            .unwrap_or_else(default_proxy_data_dir);
        let upstream_base_url = cli
            .upstream_base_url
            .clone()
            .or_else(|| std::env::var("THEOREM_ANTHROPIC_UPSTREAM").ok())
            .unwrap_or_else(|| DEFAULT_ANTHROPIC_UPSTREAM.to_string());
        let harness_mcp_url = cli
            .harness_mcp_url
            .clone()
            .or_else(|| std::env::var("THEOREM_PROXY_HARNESS_MCP_URL").ok())
            .unwrap_or_else(|| config.harness.url.clone());
        let tenant_slug = std::env::var("THEOREM_PROXY_TENANT")
            .ok()
            .filter(|tenant| !tenant.trim().is_empty())
            .unwrap_or_else(|| {
                if config.operator_memory_tenant.trim().is_empty() {
                    config.harness.tenant_slug.clone()
                } else {
                    config.operator_memory_tenant.clone()
                }
            });
        let default_room_id = cli
            .room_id
            .clone()
            .or_else(|| std::env::var("THEOREM_PROXY_ROOM_ID").ok())
            .filter(|room_id| !room_id.trim().is_empty())
            .unwrap_or_else(|| config.default_room_id.clone());
        let enable_ambient = cli.enable_ambient.unwrap_or_else(|| {
            std::env::var("THEOREM_PROXY_AMBIENT")
                .map(|value| value != "0" && !value.eq_ignore_ascii_case("false"))
                .unwrap_or(true)
        });
        let local_upstream_base_url = std::env::var("THEOREM_PROXY_LOCAL_ANTHROPIC_UPSTREAM")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let cascade_calibration_path =
            std::env::var_os("THEOREM_PROXY_CASCADE_CALIBRATION").map(PathBuf::from);
        let verification_claims_path =
            std::env::var_os("THEOREM_PROXY_VERIFICATION_CLAIMS").map(PathBuf::from);
        Self {
            bind: cli.bind.unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            port: cli
                .port
                .or_else(|| {
                    std::env::var("THEOREM_PROXY_PORT")
                        .ok()
                        .and_then(|port| port.parse().ok())
                })
                .unwrap_or(DEFAULT_PROXY_PORT),
            data_dir,
            upstream_base_url,
            harness_mcp_url,
            harness_bearer: None,
            harness_token_env: config
                .harness
                .token_env
                .clone()
                .or_else(|| Some("THEOREM_HARNESS_TOKEN".to_string())),
            tenant_slug,
            default_room_id,
            enable_ambient,
            resident_capabilities_enabled: crate::resident::resident_enabled_from_env(),
            local_upstream_base_url,
            cascade_calibration_path,
            verification_claims_path,
            resident_max_rounds: std::env::var("THEOREM_PROXY_RESIDENT_MAX_ROUNDS")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|rounds| *rounds > 0)
                .unwrap_or(DEFAULT_RESIDENT_MAX_ROUNDS),
            tool_result_budget_bytes: std::env::var("THEOREM_PROXY_TOOL_RESULT_BUDGET_BYTES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_TOOL_RESULT_BUDGET_BYTES),
            ambient_budget_bytes: std::env::var("THEOREM_PROXY_AMBIENT_BUDGET_BYTES")
                .ok()
                .and_then(|value| value.parse().ok())
                .unwrap_or(DEFAULT_AMBIENT_BUDGET_BYTES),
        }
    }

    fn addr(&self) -> SocketAddr {
        SocketAddr::new(self.bind, self.port)
    }

    fn upstream_messages_url(&self, request_uri: &Uri) -> String {
        self.messages_url_for_base(&self.upstream_base_url, request_uri)
    }

    fn messages_url_for_base(&self, base_url: &str, request_uri: &Uri) -> String {
        let mut url = format!("{}/v1/messages", base_url.trim_end_matches('/'));
        if let Some(query) = request_uri.query() {
            url.push('?');
            url.push_str(query);
        }
        url
    }
}

fn default_proxy_data_dir() -> PathBuf {
    if let Some(home) = std::env::var_os("THEOREM_HOME") {
        return PathBuf::from(home).join("proxy");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".theorem").join("proxy");
    }
    PathBuf::from(".theorem").join("proxy")
}

#[derive(Clone)]
struct ProxyState {
    config: ProxyConfig,
    http: reqwest::Client,
    presence: LocalPresenceRegistry,
}

pub async fn serve_proxy(config: ProxyConfig) -> AgentdResult<()> {
    std::fs::create_dir_all(&config.data_dir)?;
    let addr = config.addr();
    let state = ProxyState {
        config,
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(AgentdError::from)?,
        presence: LocalPresenceRegistry::new(),
    };
    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/messages", post(proxy_messages))
        .route(
            "/v1/presence",
            get(list_local_presence).post(announce_local_presence),
        )
        .route(
            "/v1/presence/footprint",
            post(set_local_footprint).delete(clear_local_footprint),
        )
        .route("/v1/presence/would-overlap", post(would_overlap_local))
        .route("/v1/agents/context", get(read_agent_context))
        .route(
            "/v1/agents/presence",
            get(read_agent_presence).post(refresh_agent_presence),
        )
        .route("/v1/agents/presence/end", post(end_agent_presence))
        .route("/v1/agents/records", post(write_agent_record))
        .route("/v1/tool-result-fetch", get(fetch_tool_result))
        .with_state(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("[theorem-agentd] proxy listening on http://{addr}");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct LocalAgentPresence {
    actor: ActorId,
    path: String,
    line: u32,
    col: u32,
    label: String,
    kind: PresenceKind,
    updated_at_ms: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
struct LocalPresenceListResponse {
    presences: Vec<LocalAgentPresence>,
    footprints: Vec<CodeEditFootprint>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ActorPath {
    actor: ActorId,
    path: String,
}

#[derive(Clone, Debug)]
struct StoredPresence {
    presence: LocalAgentPresence,
    expires_at_ms: Option<i64>,
}

#[derive(Clone, Debug)]
struct StoredFootprint {
    range: FileRange,
    summary: Option<String>,
}

#[derive(Debug, Default)]
struct LocalPresenceInner {
    presences: Mutex<BTreeMap<ActorPath, StoredPresence>>,
    footprints: Mutex<BTreeMap<ActorPath, StoredFootprint>>,
}

#[derive(Clone, Debug, Default)]
struct LocalPresenceRegistry {
    inner: Arc<LocalPresenceInner>,
}

impl LocalPresenceRegistry {
    fn new() -> Self {
        Self::default()
    }

    fn announce(&self, request: AnnounceLocalPresenceRequest) -> LocalAgentPresence {
        let actor = ActorId::from_label(&request.actor);
        let updated_at_ms = now_ms();
        let label = request
            .label
            .filter(|label| !label.trim().is_empty())
            .unwrap_or_else(|| request.actor.clone());
        let presence = LocalAgentPresence {
            actor,
            path: request.path.clone(),
            line: request.line,
            col: request.col,
            label,
            kind: request.kind.unwrap_or(PresenceKind::Agent),
            updated_at_ms,
        };
        let expires_at_ms = request
            .ttl_seconds
            .map(|ttl| updated_at_ms.saturating_add((ttl as i64).saturating_mul(1000)));
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                ActorPath {
                    actor,
                    path: request.path,
                },
                StoredPresence {
                    presence: presence.clone(),
                    expires_at_ms,
                },
            );
        presence
    }

    fn list_presences(&self) -> Vec<LocalAgentPresence> {
        self.prune_expired_presences();
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .map(|stored| stored.presence.clone())
            .collect()
    }

    fn set_footprint(&self, request: SetLocalFootprintRequest) -> CodeEditFootprint {
        let actor = ActorId::from_label(&request.actor);
        let footprint = CodeEditFootprint {
            actor,
            path: request.path.clone(),
            range: request.range,
            summary: request.summary,
        };
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                ActorPath {
                    actor,
                    path: request.path,
                },
                StoredFootprint {
                    range: footprint.range.clone(),
                    summary: footprint.summary.clone(),
                },
            );
        footprint
    }

    fn clear_footprint(&self, request: &ClearLocalFootprintRequest) -> bool {
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&ActorPath {
                actor: ActorId::from_label(&request.actor),
                path: request.path.clone(),
            })
            .is_some()
    }

    fn list_footprints(&self) -> Vec<CodeEditFootprint> {
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(key, stored)| CodeEditFootprint {
                actor: key.actor,
                path: key.path.clone(),
                range: stored.range.clone(),
                summary: stored.summary.clone(),
            })
            .collect()
    }

    fn would_overlap(&self, request: &WouldOverlapLocalRequest) -> Vec<CodeEditFootprint> {
        let caller = ActorId::from_label(&request.actor);
        self.inner
            .footprints
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|(key, _)| key.path == request.path && key.actor != caller)
            .filter(|(_, stored)| ranges_overlap(&stored.range, &request.intended))
            .map(|(key, stored)| CodeEditFootprint {
                actor: key.actor,
                path: key.path.clone(),
                range: stored.range.clone(),
                summary: stored.summary.clone(),
            })
            .collect()
    }

    fn prune_expired_presences(&self) {
        let now = now_ms();
        self.inner
            .presences
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|_, stored| {
                stored
                    .expires_at_ms
                    .map(|expires_at| now <= expires_at)
                    .unwrap_or(true)
            });
    }
}

#[derive(Debug, Deserialize)]
struct AnnounceLocalPresenceRequest {
    actor: String,
    path: String,
    line: u32,
    col: u32,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    kind: Option<PresenceKind>,
    #[serde(default)]
    ttl_seconds: Option<u64>,
}

#[derive(Debug, Serialize)]
struct AnnounceLocalPresenceResponse {
    presence: LocalAgentPresence,
}

#[derive(Debug, Deserialize)]
struct SetLocalFootprintRequest {
    actor: String,
    path: String,
    range: FileRange,
    #[serde(default)]
    summary: Option<String>,
}

#[derive(Debug, Serialize)]
struct SetLocalFootprintResponse {
    footprint: CodeEditFootprint,
}

#[derive(Debug, Deserialize)]
struct ClearLocalFootprintRequest {
    actor: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct ClearLocalFootprintResponse {
    cleared: bool,
}

#[derive(Debug, Deserialize)]
struct WouldOverlapLocalRequest {
    actor: String,
    path: String,
    intended: FileRange,
}

#[derive(Debug, Serialize)]
struct WouldOverlapLocalResponse {
    overlaps: Vec<CodeEditFootprint>,
}

async fn announce_local_presence(
    State(state): State<ProxyState>,
    Json(request): Json<AnnounceLocalPresenceRequest>,
) -> Response {
    if let Some(response) = require_local_presence_field(&request.actor, "actor") {
        return response;
    }
    if let Some(response) = require_local_presence_field(&request.path, "path") {
        return response;
    }
    Json(AnnounceLocalPresenceResponse {
        presence: state.presence.announce(request),
    })
    .into_response()
}

async fn list_local_presence(State(state): State<ProxyState>) -> Response {
    Json(LocalPresenceListResponse {
        presences: state.presence.list_presences(),
        footprints: state.presence.list_footprints(),
    })
    .into_response()
}

async fn set_local_footprint(
    State(state): State<ProxyState>,
    Json(request): Json<SetLocalFootprintRequest>,
) -> Response {
    if let Some(response) = require_local_presence_field(&request.actor, "actor") {
        return response;
    }
    if let Some(response) = require_local_presence_field(&request.path, "path") {
        return response;
    }
    Json(SetLocalFootprintResponse {
        footprint: state.presence.set_footprint(request),
    })
    .into_response()
}

async fn clear_local_footprint(
    State(state): State<ProxyState>,
    Json(request): Json<ClearLocalFootprintRequest>,
) -> Response {
    if let Some(response) = require_local_presence_field(&request.actor, "actor") {
        return response;
    }
    if let Some(response) = require_local_presence_field(&request.path, "path") {
        return response;
    }
    Json(ClearLocalFootprintResponse {
        cleared: state.presence.clear_footprint(&request),
    })
    .into_response()
}

async fn would_overlap_local(
    State(state): State<ProxyState>,
    Json(request): Json<WouldOverlapLocalRequest>,
) -> Response {
    if let Some(response) = require_local_presence_field(&request.actor, "actor") {
        return response;
    }
    if let Some(response) = require_local_presence_field(&request.path, "path") {
        return response;
    }
    Json(WouldOverlapLocalResponse {
        overlaps: state.presence.would_overlap(&request),
    })
    .into_response()
}

fn require_local_presence_field(value: &str, field: &'static str) -> Option<Response> {
    if value.trim().is_empty() {
        Some(proxy_json_error(
            StatusCode::BAD_REQUEST,
            "theorem_proxy_invalid_presence",
            &format!("{field} must not be empty"),
        ))
    } else {
        None
    }
}

fn ranges_overlap(a: &FileRange, b: &FileRange) -> bool {
    let a_start = (a.start_line, a.start_col);
    let a_end = (a.end_line, a.end_col);
    let b_start = (b.start_line, b.start_col);
    let b_end = (b.end_line, b.end_col);
    !(a_end < b_start || b_end < a_start)
}

#[derive(Debug, Default, Deserialize)]
struct AgentPresenceQuery {
    actor: Option<String>,
    actor_id: Option<String>,
    room_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct AgentContextQuery {
    actor: Option<String>,
    actor_id: Option<String>,
    room_id: Option<String>,
    record_type: Option<String>,
    limit: Option<u64>,
    message_limit: Option<u64>,
    record_limit: Option<u64>,
    mention_limit: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
struct AgentRecordRequest {
    actor: Option<String>,
    actor_id: Option<String>,
    room_id: Option<String>,
    record_type: Option<String>,
    title: Option<String>,
    summary: Option<String>,
    body: Option<String>,
    metadata: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
struct AgentPresenceRequest {
    actor: Option<String>,
    actor_id: Option<String>,
    room_id: Option<String>,
    session_id: Option<String>,
    surface: Option<String>,
    status: Option<String>,
    worktree: Option<String>,
    branch: Option<String>,
    head: Option<String>,
    #[serde(default)]
    changed_files: Vec<String>,
    ttl_seconds: Option<u64>,
    summary: Option<String>,
    #[serde(default)]
    footprint: Vec<String>,
    expected_completion: Option<String>,
    repo: Option<String>,
    task: Option<String>,
    lane: Option<String>,
    endpoint: Option<String>,
    #[serde(default)]
    capabilities: Vec<String>,
    announce_intent: Option<bool>,
}

#[derive(Clone, Debug)]
struct ResolvedAgentPresence {
    actor_id: String,
    room_id: String,
    session_id: String,
    surface: String,
    status: String,
    worktree: String,
    branch: String,
    head: String,
    changed_files: Vec<String>,
    ttl_seconds: u64,
    summary: String,
    footprint: Vec<String>,
    expected_completion: String,
    repo: String,
    task: String,
    lane: String,
    endpoint: String,
    capabilities: Vec<String>,
    announce_intent: bool,
}

impl AgentPresenceRequest {
    fn resolve(&self, config: &ProxyConfig, ending: bool) -> ResolvedAgentPresence {
        let actor_id = clean_text(self.actor_id.as_deref())
            .or_else(|| clean_text(self.actor.as_deref()))
            .unwrap_or_else(|| "codex".to_string());
        let room_id =
            clean_text(self.room_id.as_deref()).unwrap_or_else(|| config.default_room_id.clone());
        let endpoint = clean_text(self.endpoint.as_deref()).unwrap_or_default();
        let capabilities = clean_string_vec(&self.capabilities);
        let surface = clean_text(self.surface.as_deref()).unwrap_or_else(|| {
            if endpoint.is_empty() {
                "theorem-proxy".to_string()
            } else {
                "codex:app-server".to_string()
            }
        });
        let status = clean_text(self.status.as_deref()).unwrap_or_else(|| {
            if ending {
                "inactive".to_string()
            } else {
                "active".to_string()
            }
        });
        let summary = clean_text(self.summary.as_deref()).unwrap_or_else(|| {
            default_presence_summary(&actor_id, &endpoint, &capabilities, ending)
        });
        ResolvedAgentPresence {
            actor_id,
            room_id,
            session_id: clean_text(self.session_id.as_deref()).unwrap_or_default(),
            surface,
            status,
            worktree: clean_text(self.worktree.as_deref()).unwrap_or_default(),
            branch: clean_text(self.branch.as_deref()).unwrap_or_default(),
            head: clean_text(self.head.as_deref()).unwrap_or_default(),
            changed_files: clean_string_vec(&self.changed_files),
            ttl_seconds: self.ttl_seconds.unwrap_or(if ending { 1 } else { 60 }),
            summary,
            footprint: clean_string_vec(&self.footprint),
            expected_completion: clean_text(self.expected_completion.as_deref())
                .unwrap_or_default(),
            repo: clean_text(self.repo.as_deref()).unwrap_or_default(),
            task: clean_text(self.task.as_deref()).unwrap_or_default(),
            lane: clean_text(self.lane.as_deref()).unwrap_or_default(),
            endpoint,
            capabilities,
            announce_intent: self.announce_intent.unwrap_or(true),
        }
    }
}

async fn read_agent_presence(
    State(state): State<ProxyState>,
    Query(query): Query<AgentPresenceQuery>,
) -> Response {
    let room_id = clean_text(query.room_id.as_deref())
        .unwrap_or_else(|| state.config.default_room_id.clone());
    let mut presence_args = json!({
        "tenant_slug": state.config.tenant_slug,
        "mode": "get"
    });
    if let Some(actor_id) =
        clean_text(query.actor_id.as_deref()).or_else(|| clean_text(query.actor.as_deref()))
    {
        presence_args["actor"] = json!(actor_id);
    }
    let room_args = json!({
        "tenant_slug": state.config.tenant_slug,
        "action": "status",
        "room_id": room_id
    });
    let room = match call_harness_tool(&state, "coordination_room", room_args).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let presence = match call_harness_tool(&state, "presence", presence_args).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    Json(json!({
        "ok": true,
        "tenant_slug": state.config.tenant_slug,
        "room_id": room_id,
        "room": room.get("room").cloned().unwrap_or(room),
        "presence": presence.get("presence").cloned().unwrap_or(presence)
    }))
    .into_response()
}

async fn read_agent_context(
    State(state): State<ProxyState>,
    Query(query): Query<AgentContextQuery>,
) -> Response {
    let room_id = clean_text(query.room_id.as_deref())
        .unwrap_or_else(|| state.config.default_room_id.clone());
    let actor_id =
        clean_text(query.actor_id.as_deref()).or_else(|| clean_text(query.actor.as_deref()));
    let args = context_arguments(&state.config, &room_id, actor_id.as_deref(), &query);
    let context = match call_harness_tool(&state, "coordination_context", args).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    Json(json!({
        "ok": true,
        "tenant_slug": state.config.tenant_slug,
        "room_id": room_id,
        "actor_id": actor_id.unwrap_or_default(),
        "context": context
    }))
    .into_response()
}

async fn write_agent_record(
    State(state): State<ProxyState>,
    Json(input): Json<AgentRecordRequest>,
) -> Response {
    let Some(summary) = clean_text(input.summary.as_deref()) else {
        return proxy_json_error(
            StatusCode::BAD_REQUEST,
            "theorem_proxy_invalid_record",
            "summary is required",
        );
    };
    let actor_id = clean_text(input.actor_id.as_deref())
        .or_else(|| clean_text(input.actor.as_deref()))
        .unwrap_or_else(|| "codex".to_string());
    let room_id = clean_text(input.room_id.as_deref())
        .unwrap_or_else(|| state.config.default_room_id.clone());
    let record_type =
        clean_text(input.record_type.as_deref()).unwrap_or_else(|| "reflection".to_string());
    let args = record_arguments(
        &state.config,
        &actor_id,
        &room_id,
        &record_type,
        summary,
        input,
    );
    let record = match call_harness_tool(&state, "coordination_record", args).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    Json(json!({
        "ok": true,
        "tenant_slug": state.config.tenant_slug,
        "room_id": room_id,
        "actor_id": actor_id,
        "record": record.get("record").cloned().unwrap_or(record)
    }))
    .into_response()
}

async fn refresh_agent_presence(
    State(state): State<ProxyState>,
    Json(input): Json<AgentPresenceRequest>,
) -> Response {
    write_agent_presence(state, input, false).await
}

async fn end_agent_presence(
    State(state): State<ProxyState>,
    Json(input): Json<AgentPresenceRequest>,
) -> Response {
    write_agent_presence(state, input, true).await
}

async fn write_agent_presence(
    state: ProxyState,
    input: AgentPresenceRequest,
    ending: bool,
) -> Response {
    let resolved = input.resolve(&state.config, ending);
    let room = if ending {
        None
    } else {
        let args = room_join_arguments(&state.config, &resolved);
        match call_harness_tool(&state, "coordination_room", args).await {
            Ok(value) => Some(value),
            Err(response) => return response,
        }
    };
    let presence_args = presence_arguments(&state.config, &resolved, ending);
    let presence = match call_harness_tool(&state, "presence", presence_args).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let intent = if resolved.announce_intent {
        let args = intent_arguments(&state.config, &resolved, ending);
        match call_harness_tool(&state, "coordination_intent", args).await {
            Ok(value) => Some(value),
            Err(response) => return response,
        }
    } else {
        None
    };

    Json(json!({
        "ok": true,
        "tenant_slug": state.config.tenant_slug,
        "room_id": resolved.room_id,
        "actor_id": resolved.actor_id,
        "endpoint": empty_string_as_null(&resolved.endpoint),
        "capabilities": resolved.capabilities,
        "room": room.map(|value| value.get("room").cloned().unwrap_or(value)),
        "presence": presence.get("presence").cloned().unwrap_or(presence),
        "intent": intent.map(|value| value.get("intent").cloned().unwrap_or(value))
    }))
    .into_response()
}

fn room_join_arguments(config: &ProxyConfig, resolved: &ResolvedAgentPresence) -> Value {
    json!({
        "tenant_slug": config.tenant_slug,
        "action": "join",
        "actor": resolved.actor_id,
        "room_id": resolved.room_id,
        "session_id": resolved.session_id,
        "surface": resolved.surface,
        "repo": resolved.repo,
        "branch": resolved.branch,
        "task": resolved.task,
        "worktree": resolved.worktree,
        "head": resolved.head,
        "changed_files": resolved.changed_files,
        "lane": resolved.lane
    })
}

fn presence_arguments(
    config: &ProxyConfig,
    resolved: &ResolvedAgentPresence,
    ending: bool,
) -> Value {
    json!({
        "tenant_slug": config.tenant_slug,
        "mode": if ending { "end" } else { "heartbeat" },
        "actor": resolved.actor_id,
        "session_id": resolved.session_id,
        "surface": resolved.surface,
        "status": resolved.status,
        "worktree": resolved.worktree,
        "branch": resolved.branch,
        "head": resolved.head,
        "changed_files": resolved.changed_files,
        "ttl_seconds": resolved.ttl_seconds
    })
}

fn intent_arguments(config: &ProxyConfig, resolved: &ResolvedAgentPresence, ending: bool) -> Value {
    json!({
        "tenant_slug": config.tenant_slug,
        "room_id": resolved.room_id,
        "actor": resolved.actor_id,
        "status": if ending { "done" } else { "working" },
        "summary": resolved.summary,
        "footprint": resolved.footprint,
        "expected_completion": resolved.expected_completion,
        "repo": resolved.repo,
        "branch": resolved.branch,
        "task": resolved.task
    })
}

fn context_arguments(
    config: &ProxyConfig,
    room_id: &str,
    actor_id: Option<&str>,
    query: &AgentContextQuery,
) -> Value {
    let mut args = json!({
        "tenant_slug": config.tenant_slug,
        "room_id": room_id
    });
    if let Some(actor_id) = actor_id.and_then(|actor| clean_text(Some(actor))) {
        args["actor"] = json!(actor_id);
    }
    if let Some(record_type) = clean_text(query.record_type.as_deref()) {
        args["record_type"] = json!(record_type);
    }
    if let Some(limit) = query.limit {
        args["limit"] = json!(limit);
    }
    if let Some(limit) = query.message_limit {
        args["message_limit"] = json!(limit);
    }
    if let Some(limit) = query.record_limit {
        args["record_limit"] = json!(limit);
    }
    if let Some(limit) = query.mention_limit {
        args["mention_limit"] = json!(limit);
    }
    args
}

fn record_arguments(
    config: &ProxyConfig,
    actor_id: &str,
    room_id: &str,
    record_type: &str,
    summary: String,
    input: AgentRecordRequest,
) -> Value {
    let mut args = json!({
        "tenant_slug": config.tenant_slug,
        "room_id": room_id,
        "actor": actor_id,
        "record_type": record_type,
        "summary": summary
    });
    if let Some(title) = clean_text(input.title.as_deref()) {
        args["title"] = json!(title);
    }
    if let Some(body) = clean_text(input.body.as_deref()) {
        args["body"] = json!(body);
    }
    if let Some(metadata) = input.metadata {
        args["metadata"] = metadata;
    }
    args
}

fn default_presence_summary(
    actor_id: &str,
    endpoint: &str,
    capabilities: &[String],
    ending: bool,
) -> String {
    if ending {
        return format!("{actor_id} local proxy presence ended.");
    }
    let mut summary = if endpoint.is_empty() {
        format!("{actor_id} is active through the local Theorem proxy.")
    } else {
        format!("{actor_id} local endpoint ready at {endpoint}.")
    };
    if !capabilities.is_empty() {
        summary.push_str(" Capabilities: ");
        summary.push_str(&capabilities.join(", "));
        summary.push('.');
    }
    summary
}

fn clean_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn clean_string_vec(values: &[String]) -> Vec<String> {
    values
        .iter()
        .filter_map(|value| clean_text(Some(value)))
        .collect()
}

fn empty_string_as_null(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

async fn proxy_messages(
    State(state): State<ProxyState>,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let ambient = fetch_ambient_context(&state, &body).await;
    let transformed = transform_messages_request(
        &body,
        ambient.as_deref(),
        state.config.tool_result_budget_bytes,
    )
    .unwrap_or_else(|| body.to_vec());

    if state.config.resident_capabilities_enabled {
        if let Ok(value) = serde_json::from_slice::<Value>(&transformed) {
            return proxy_messages_with_resident_loop(&state, &uri, &headers, value).await;
        }
    }

    proxy_messages_passthrough(&state, &uri, &headers, transformed).await
}

async fn proxy_messages_passthrough(
    state: &ProxyState,
    uri: &Uri,
    headers: &HeaderMap,
    body: Vec<u8>,
) -> Response {
    let url = state.config.upstream_messages_url(&uri);
    let mut request = state
        .http
        .request(Method::POST, url)
        .body(body)
        .header("content-type", "application/json");
    for (name, value) in headers.iter() {
        if request_header_allowed(name) {
            request = request.header(name, value);
        }
    }

    let upstream = match request.send().await {
        Ok(response) => response,
        Err(error) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "type": "theorem_proxy_upstream_error",
                        "message": error.to_string()
                    }
                })),
            )
                .into_response();
        }
    };

    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let mut builder = Response::builder().status(status);
    for (name, value) in upstream.headers() {
        if response_header_allowed(name) {
            builder = builder.header(name, value);
        }
    }
    let stream = upstream.bytes_stream().map_err(std::io::Error::other);
    builder
        .body(Body::from_stream(stream))
        .unwrap_or_else(|error| {
            (
                StatusCode::BAD_GATEWAY,
                Json(json!({
                    "error": {
                        "type": "theorem_proxy_response_error",
                        "message": error.to_string()
                    }
                })),
            )
                .into_response()
        })
}

async fn proxy_messages_with_resident_loop(
    state: &ProxyState,
    uri: &Uri,
    headers: &HeaderMap,
    mut request: Value,
) -> Response {
    let original_stream = request
        .get("stream")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    request["stream"] = json!(false);
    crate::resident::inject_resident_tools(&mut request);

    let mut verification_advised = false;
    let max_rounds = state.config.resident_max_rounds.max(1);
    for _ in 0..max_rounds {
        let latest_user = latest_user_text_from_value(&request);
        let decision = crate::resident::route_with_calibration_file(
            state.config.cascade_calibration_path.as_deref(),
            &latest_user,
            state.config.local_upstream_base_url.is_some(),
        );
        let upstream_base = match decision.selected {
            crate::resident::CascadeRouteTarget::Local => state
                .config
                .local_upstream_base_url
                .as_deref()
                .unwrap_or(&state.config.upstream_base_url),
            crate::resident::CascadeRouteTarget::Upstream
            | crate::resident::CascadeRouteTarget::CalibrationRequired => {
                &state.config.upstream_base_url
            }
        };
        let assistant = match send_upstream_json(state, uri, headers, &request, upstream_base).await
        {
            Ok(value) => value,
            Err(response) => return response,
        };

        let tool_uses = crate::resident::resident_tool_uses(&assistant);
        if !tool_uses.is_empty() {
            let mut results = Vec::with_capacity(tool_uses.len());
            for tool_use in tool_uses {
                results.push(execute_resident_tool_use(state, tool_use).await);
            }
            crate::resident::append_tool_results(&mut request, &assistant, results);
            continue;
        }

        if !verification_advised {
            let claims = crate::resident::load_verification_claims(
                state.config.verification_claims_path.as_deref(),
                &state.config.data_dir,
            );
            let findings = crate::resident::verification_findings(
                &crate::resident::assistant_text(&assistant),
                &claims,
            );
            if !findings.is_empty() {
                verification_advised = true;
                crate::resident::append_verification_advisory(&mut request, &assistant, &findings);
                continue;
            }
        }

        return final_messages_response(assistant, original_stream);
    }

    proxy_json_error(
        StatusCode::LOOP_DETECTED,
        "theorem_proxy_resident_round_limit",
        "resident tool loop exceeded THEOREM_PROXY_RESIDENT_MAX_ROUNDS",
    )
}

async fn send_upstream_json(
    state: &ProxyState,
    uri: &Uri,
    headers: &HeaderMap,
    body: &Value,
    base_url: &str,
) -> Result<Value, Response> {
    let bytes = serde_json::to_vec(body).map_err(|error| {
        proxy_json_error(
            StatusCode::BAD_GATEWAY,
            "theorem_proxy_request_encode_error",
            &error.to_string(),
        )
    })?;
    let url = state.config.messages_url_for_base(base_url, uri);
    let mut request = state
        .http
        .request(Method::POST, url)
        .body(bytes)
        .header("content-type", "application/json");
    for (name, value) in headers.iter() {
        if request_header_allowed(name) {
            request = request.header(name, value);
        }
    }

    let upstream = request.send().await.map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            Json(json!({
                "error": {
                    "type": "theorem_proxy_upstream_error",
                    "message": error.to_string()
                }
            })),
        )
            .into_response()
    })?;
    let status =
        StatusCode::from_u16(upstream.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let response_headers = upstream.headers().clone();
    let bytes = upstream.bytes().await.map_err(|error| {
        proxy_json_error(
            StatusCode::BAD_GATEWAY,
            "theorem_proxy_upstream_read_error",
            &error.to_string(),
        )
    })?;
    if !status.is_success() {
        let mut builder = Response::builder().status(status);
        for (name, value) in response_headers.iter() {
            if response_header_allowed(name) {
                builder = builder.header(name, value);
            }
        }
        return Err(builder.body(Body::from(bytes)).unwrap_or_else(|error| {
            proxy_json_error(
                StatusCode::BAD_GATEWAY,
                "theorem_proxy_response_error",
                &error.to_string(),
            )
        }));
    }
    serde_json::from_slice::<Value>(&bytes).map_err(|error| {
        proxy_json_error(
            StatusCode::BAD_GATEWAY,
            "theorem_proxy_upstream_invalid_json",
            &format!("upstream Messages response was not JSON: {error}"),
        )
    })
}

async fn execute_resident_tool_use(
    state: &ProxyState,
    tool_use: crate::resident::ResidentToolUse,
) -> Value {
    if let Some(hold) = crate::resident::approval_required_payload(&tool_use) {
        return crate::resident::tool_result_block(&tool_use.id, hold, false);
    }
    let (gateway_name, arguments) =
        crate::resident::gateway_call_for_tool_use(&tool_use, &state.config.tenant_slug);
    match call_harness_tool_value(state, &gateway_name, arguments.clone()).await {
        Ok(payload) => crate::resident::tool_result_block(&tool_use.id, payload, false),
        Err(message) => {
            let fallback_input =
                if crate::resident::resident_tool_name_to_affordance_id(&tool_use.name).is_some() {
                    &tool_use.input
                } else {
                    &arguments
                };
            if let Some(payload) = crate::resident::fallback_tool_result(
                &tool_use.name,
                fallback_input,
                &state.config.tenant_slug,
            ) {
                return crate::resident::tool_result_block(&tool_use.id, payload, false);
            }
            crate::resident::tool_result_block(
                &tool_use.id,
                json!({
                    "error": "resident_tool_failed",
                    "tool_name": tool_use.name,
                    "gateway_tool_name": gateway_name,
                    "message": message
                }),
                true,
            )
        }
    }
}

fn final_messages_response(message: Value, stream: bool) -> Response {
    if stream {
        return Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .body(Body::from(crate::resident::anthropic_sse_from_message(
                &message,
            )))
            .unwrap_or_else(|error| {
                proxy_json_error(
                    StatusCode::BAD_GATEWAY,
                    "theorem_proxy_sse_encode_error",
                    &error.to_string(),
                )
            });
    }
    Json(message).into_response()
}

fn request_header_allowed(name: &HeaderName) -> bool {
    !matches!(
        *name,
        HOST | CONTENT_LENGTH | CONNECTION | TRANSFER_ENCODING
    )
}

fn response_header_allowed(name: &HeaderName) -> bool {
    !matches!(*name, CONTENT_LENGTH | CONNECTION | TRANSFER_ENCODING)
}

async fn call_harness_tool(
    state: &ProxyState,
    name: &str,
    arguments: Value,
) -> Result<Value, Response> {
    call_harness_tool_value(state, name, arguments)
        .await
        .map_err(|message| {
            let status = if message.contains("timed out") {
                StatusCode::GATEWAY_TIMEOUT
            } else {
                StatusCode::BAD_GATEWAY
            };
            proxy_json_error(status, "theorem_proxy_harness_error", &message)
        })
}

async fn call_harness_tool_value(
    state: &ProxyState,
    name: &str,
    arguments: Value,
) -> Result<Value, String> {
    let request = state.http.post(&state.config.harness_mcp_url).json(&json!({
        "jsonrpc": "2.0",
        "id": format!("theorem-proxy-{name}"),
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": arguments
        }
    }));
    let request = apply_harness_auth(&state.config, request);
    let response = match tokio::time::timeout(Duration::from_secs(5), request.send()).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => return Err(format!("harness MCP {name} call failed: {error}")),
        Err(_) => return Err(format!("harness MCP {name} call timed out")),
    };
    let status = response.status();
    let value = match response.json::<Value>().await {
        Ok(value) => value,
        Err(error) => return Err(format!("harness MCP {name} returned invalid JSON: {error}")),
    };
    if !status.is_success() {
        return Err(format!(
            "harness MCP {name} returned HTTP {status}: {value}"
        ));
    }
    mcp_payload_from_response(&value)
        .map_err(|message| format!("harness MCP {name} failed: {message}"))
}

fn apply_harness_auth(
    config: &ProxyConfig,
    request: reqwest::RequestBuilder,
) -> reqwest::RequestBuilder {
    if let Some(token) = &config.harness_bearer {
        if !token.trim().is_empty() {
            return request.bearer_auth(token);
        }
    } else if let Some(env) = &config.harness_token_env {
        if let Ok(token) = std::env::var(env) {
            if !token.trim().is_empty() {
                return request.bearer_auth(token);
            }
        }
    }
    request
}

fn mcp_payload_from_response(value: &Value) -> Result<Value, String> {
    if let Some(error) = value.get("error") {
        return Err(format!("jsonrpc error: {error}"));
    }
    let Some(result) = value.get("result") else {
        return Err("response missing result".to_string());
    };
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        return Err(result
            .get("structuredContent")
            .or_else(|| result.get("content"))
            .map(Value::to_string)
            .unwrap_or_else(|| "tool returned isError".to_string()));
    }
    if let Some(structured) = result.get("structuredContent") {
        return Ok(structured.clone());
    }
    if let Some(text) = result
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| content.first())
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
    {
        return serde_json::from_str::<Value>(text)
            .map(|payload| payload.get("result").cloned().unwrap_or(payload))
            .map_err(|error| format!("result.content[0].text is not JSON: {error}"));
    }
    Ok(result.clone())
}

fn proxy_json_error(status: StatusCode, error_type: &str, message: &str) -> Response {
    (
        status,
        Json(json!({
            "error": {
                "type": error_type,
                "message": message
            }
        })),
    )
        .into_response()
}

async fn fetch_ambient_context(state: &ProxyState, raw_body: &[u8]) -> Option<String> {
    if !state.config.enable_ambient {
        return None;
    }
    if let Ok(text) = std::env::var("THEOREM_PROXY_AMBIENT_TEXT") {
        let text = text.trim();
        if !text.is_empty() {
            return Some(limit_utf8(text, state.config.ambient_budget_bytes));
        }
    }
    let ambient_file = state.config.data_dir.join("ambient.md");
    if let Ok(text) = std::fs::read_to_string(ambient_file) {
        let text = text.trim();
        if !text.is_empty() {
            return Some(limit_utf8(text, state.config.ambient_budget_bytes));
        }
    }
    let query = latest_user_text(raw_body)?;
    if query.trim().is_empty() {
        return None;
    }
    let request = state.http.post(&state.config.harness_mcp_url).json(&json!({
        "jsonrpc": "2.0",
        "id": "theorem-proxy-ambient",
        "method": "tools/call",
        "params": {
            "name": "hippo_retrieve",
            "arguments": {
                "tenant_slug": state.config.tenant_slug,
                "query": query,
                "k": 3,
                "include_hubs": true
            }
        }
    }));
    let request = apply_harness_auth(&state.config, request);
    let response = match tokio::time::timeout(Duration::from_secs(2), request.send()).await {
        Ok(Ok(response)) => response,
        _ => return None,
    };
    let value = match response.json::<Value>().await {
        Ok(value) => value,
        Err(_) => return None,
    };
    ambient_text_from_mcp(&value).map(|text| limit_utf8(&text, state.config.ambient_budget_bytes))
}

fn ambient_text_from_mcp(value: &Value) -> Option<String> {
    let mut chunks = Vec::new();
    if let Some(content) = value.pointer("/result/content").and_then(Value::as_array) {
        for block in content {
            if let Some(text) = block.get("text").and_then(Value::as_str) {
                chunks.push(text.trim().to_string());
            }
        }
    }
    if chunks.is_empty() {
        if let Some(structured) = value.pointer("/result/structuredContent") {
            chunks.push(structured.to_string());
        }
    }
    let text = chunks
        .into_iter()
        .filter(|chunk| !chunk.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!text.is_empty()).then_some(format!(
        "<theorem_ambient_context>\n{text}\n</theorem_ambient_context>"
    ))
}

pub fn latest_user_text(raw_body: &[u8]) -> Option<String> {
    let value: Value = serde_json::from_slice(raw_body).ok()?;
    Some(latest_user_text_from_value(&value))
}

fn latest_user_text_from_value(value: &Value) -> String {
    let Some(messages) = value.get("messages").and_then(Value::as_array) else {
        return String::new();
    };
    for message in messages.iter().rev() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        if let Some(content) = message.get("content") {
            return content_text(content);
        }
    }
    String::new()
}

fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub fn transform_messages_request(
    raw_body: &[u8],
    ambient_context: Option<&str>,
    budget_bytes: usize,
) -> Option<Vec<u8>> {
    let mut value: Value = serde_json::from_slice(raw_body).ok()?;
    let mut sampled = value.clone();
    sample_tool_results(&mut sampled, budget_bytes);
    let sampled_bytes = serde_json::to_vec(&sampled).ok()?;
    if sampled_bytes.len() < raw_body.len() {
        value = sampled;
    }
    if let Some(context) = ambient_context.filter(|context| !context.trim().is_empty()) {
        inject_ambient_context(&mut value, context);
    }
    serde_json::to_vec(&value).ok()
}

fn sample_tool_results(value: &mut Value, budget_bytes: usize) {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages {
        let Some(content) = message.get_mut("content") else {
            continue;
        };
        let Value::Array(blocks) = content else {
            continue;
        };
        for block in blocks {
            if block.get("type").and_then(Value::as_str) != Some("tool_result") {
                continue;
            }
            sample_tool_result_block(block, budget_bytes);
        }
    }
}

fn sample_tool_result_block(block: &mut Value, budget_bytes: usize) {
    let Some(content) = block.get_mut("content") else {
        return;
    };
    let raw = match &*content {
        Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    if raw.len() <= budget_bytes {
        return;
    }
    let Some(sampled) = sampled_tool_text(&raw, budget_bytes) else {
        return;
    };
    if sampled.len() >= raw.len() {
        return;
    }
    *content = Value::String(sampled);
}

fn sampled_tool_text(raw: &str, budget_bytes: usize) -> Option<String> {
    let handle = store_tool_result(raw);
    let sample = if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(raw) {
        sample_json_array(&items, budget_bytes)
    } else {
        sample_lines(raw, budget_bytes)
    };
    let marker = format!(
        "\n\n[theorem proxy sampled tool_result: original_bytes={}, returned_bytes={}, fetch_handle={}; call tool_result_fetch with fetch_handle/offset/max_bytes or GET /v1/tool-result-fetch?fetch_handle={}]",
        raw.len(),
        sample.len(),
        handle,
        handle
    );
    Some(format!("{sample}{marker}"))
}

fn sample_json_array(items: &[Value], budget_bytes: usize) -> String {
    let selected = selected_item_indices(items, budget_bytes);
    let sample = selected
        .into_iter()
        .filter_map(|index| items.get(index).cloned())
        .collect::<Vec<_>>();
    serde_json::to_string_pretty(&sample).unwrap_or_else(|_| Value::Array(sample).to_string())
}

fn selected_item_indices(items: &[Value], budget_bytes: usize) -> Vec<usize> {
    if items.is_empty() {
        return Vec::new();
    }
    let max_items = (budget_bytes / 512).clamp(3, 24).min(items.len());
    let mut selected = BTreeSet::new();
    selected.insert(0);
    selected.insert(items.len() - 1);
    for (index, item) in items.iter().enumerate() {
        if selected.len() >= max_items {
            break;
        }
        if looks_anomalous(&item.to_string()) {
            selected.insert(index);
        }
    }
    let denominator = max_items.saturating_sub(1).max(1);
    for slot in 0..max_items {
        if selected.len() >= max_items {
            break;
        }
        let index = slot * (items.len().saturating_sub(1)) / denominator;
        selected.insert(index);
    }
    selected.into_iter().collect()
}

fn sample_lines(raw: &str, budget_bytes: usize) -> String {
    let lines = raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return limit_utf8(raw, budget_bytes / 2);
    }
    let max_lines = (budget_bytes / 256).clamp(4, 48).min(lines.len());
    let mut selected = BTreeSet::new();
    selected.insert(0);
    selected.insert(lines.len() - 1);
    for (index, line) in lines.iter().enumerate() {
        if selected.len() >= max_lines {
            break;
        }
        if looks_anomalous(line) {
            selected.insert(index);
        }
    }
    let denominator = max_lines.saturating_sub(1).max(1);
    for slot in 0..max_lines {
        if selected.len() >= max_lines {
            break;
        }
        let index = slot * (lines.len().saturating_sub(1)) / denominator;
        selected.insert(index);
    }
    selected
        .into_iter()
        .filter_map(|index| lines.get(index).copied())
        .collect::<Vec<_>>()
        .join("\n")
}

fn looks_anomalous(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    [
        "error",
        "failed",
        "failure",
        "panic",
        "exception",
        "warning",
        "denied",
        "no such file",
        "traceback",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn inject_ambient_context(value: &mut Value, context: &str) {
    let Some(messages) = value.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    for message in messages.iter_mut().rev() {
        if message.get("role").and_then(Value::as_str) != Some("user") {
            continue;
        }
        let ambient_block = json!({
            "type": "text",
            "text": context
        });
        match message.get_mut("content") {
            Some(Value::String(text)) => {
                let original = std::mem::take(text);
                message["content"] = json!([
                    { "type": "text", "text": original },
                    ambient_block
                ]);
            }
            Some(Value::Array(blocks)) => blocks.push(ambient_block),
            _ => message["content"] = json!([ambient_block]),
        }
        return;
    }
    messages.push(json!({
        "role": "user",
        "content": [{ "type": "text", "text": context }]
    }));
}

fn store_tool_result(body: &str) -> String {
    let handle = format!("proxy-tool-result:{:016x}", stable_hash(body.as_bytes()));
    let store = TOOL_RESULT_BODIES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut bodies) = store.lock() {
        bodies.insert(handle.clone(), body.to_string());
    }
    handle
}

fn stable_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

#[derive(Debug, Deserialize)]
struct FetchQuery {
    fetch_handle: Option<String>,
    handle: Option<String>,
    offset: Option<usize>,
    max_bytes: Option<usize>,
}

async fn fetch_tool_result(Query(query): Query<FetchQuery>) -> Response {
    let Some(handle) = query.fetch_handle.or(query.handle) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "fetch_handle is required"})),
        )
            .into_response();
    };
    match fetch_tool_result_slice(
        &handle,
        query.offset.unwrap_or(0),
        query.max_bytes.unwrap_or(DEFAULT_TOOL_RESULT_BUDGET_BYTES),
    ) {
        Some(payload) => Json(payload).into_response(),
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "fetch_handle not found"})),
        )
            .into_response(),
    }
}

pub fn fetch_tool_result_slice(handle: &str, offset: usize, max_bytes: usize) -> Option<Value> {
    let store = TOOL_RESULT_BODIES.get_or_init(|| Mutex::new(HashMap::new()));
    let bodies = store.lock().ok()?;
    let body = bodies.get(handle)?;
    let start = floor_char_boundary(body, offset.min(body.len()));
    let end = floor_char_boundary(body, start.saturating_add(max_bytes).min(body.len()));
    Some(json!({
        "fetch_handle": handle,
        "offset": start,
        "next_offset": (end < body.len()).then_some(end),
        "total_bytes": body.len(),
        "text": &body[start..end],
    }))
}

fn limit_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let cut = floor_char_boundary(value, max_bytes);
    value[..cut].to_string()
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proxy_config() -> ProxyConfig {
        ProxyConfig {
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            port: DEFAULT_PROXY_PORT,
            data_dir: PathBuf::from("/tmp/theorem-proxy-test"),
            upstream_base_url: DEFAULT_ANTHROPIC_UPSTREAM.to_string(),
            harness_mcp_url: "http://127.0.0.1:8380/mcp".to_string(),
            harness_bearer: None,
            harness_token_env: None,
            tenant_slug: "Travis-Gilbert".to_string(),
            default_room_id: "repo:theorem:branch:main".to_string(),
            enable_ambient: false,
            resident_capabilities_enabled: true,
            local_upstream_base_url: None,
            cascade_calibration_path: None,
            verification_claims_path: None,
            resident_max_rounds: DEFAULT_RESIDENT_MAX_ROUNDS,
            tool_result_budget_bytes: DEFAULT_TOOL_RESULT_BUDGET_BYTES,
            ambient_budget_bytes: DEFAULT_AMBIENT_BUDGET_BYTES,
        }
    }

    fn base_request(content: Value) -> Value {
        json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "system": "stable system prefix",
            "tools": [{"name": "demo", "input_schema": {"type": "object"}}],
            "messages": [
                {
                    "role": "user",
                    "content": content
                }
            ]
        })
    }

    #[test]
    fn sampling_preserves_tool_use_id_and_keeps_errors() {
        let rows = (0..80)
            .map(|index| {
                if index == 42 {
                    json!({"path": "src/lib.rs", "line": index, "text": "ERROR important failure"})
                } else {
                    json!({"path": "src/lib.rs", "line": index, "text": format!("boring {index}")})
                }
            })
            .collect::<Vec<_>>();
        let request = base_request(json!([
            {
                "type": "tool_result",
                "tool_use_id": "toolu_123",
                "content": serde_json::to_string(&rows).unwrap()
            }
        ]));
        let raw = serde_json::to_vec(&request).unwrap();
        let transformed = transform_messages_request(&raw, None, 1024).unwrap();
        let value: Value = serde_json::from_slice(&transformed).unwrap();
        let block = &value["messages"][0]["content"][0];
        assert_eq!(block["tool_use_id"], "toolu_123");
        let content = block["content"].as_str().unwrap();
        assert!(content.contains("ERROR important failure"));
        assert!(content.contains("fetch_handle="));
        assert!(transformed.len() < raw.len());
    }

    #[test]
    fn small_tool_result_is_unchanged() {
        let request = base_request(json!([
            {
                "type": "tool_result",
                "tool_use_id": "toolu_123",
                "content": "small result"
            }
        ]));
        let raw = serde_json::to_vec(&request).unwrap();
        let transformed = transform_messages_request(&raw, None, 1024).unwrap();
        let value: Value = serde_json::from_slice(&transformed).unwrap();
        assert_eq!(
            value["messages"][0]["content"][0]["content"],
            "small result"
        );
    }

    #[test]
    fn ambient_injection_uses_latest_user_turn_without_touching_prefix() {
        let request = base_request(json!("what did we decide?"));
        let raw = serde_json::to_vec(&request).unwrap();
        let transformed = transform_messages_request(
            &raw,
            Some("<theorem_ambient_context>x</theorem_ambient_context>"),
            1024,
        )
        .unwrap();
        let value: Value = serde_json::from_slice(&transformed).unwrap();
        assert_eq!(value["system"], "stable system prefix");
        assert_eq!(value["tools"][0]["name"], "demo");
        let content = value["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["text"], "what did we decide?");
        assert!(content[1]["text"]
            .as_str()
            .unwrap()
            .contains("theorem_ambient_context"));
    }

    #[test]
    fn resident_mode_injects_gateway_tools_without_touching_prefix() {
        let mut request = base_request(json!("find compute affordances"));
        crate::resident::inject_resident_tools(&mut request);
        assert_eq!(request["system"], "stable system prefix");
        let tool_names = request["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(tool_names.contains(&"demo"));
        assert!(tool_names.contains(&crate::resident::TOOL_SEARCH));
        assert!(tool_names.contains(&crate::resident::DESCRIBE));
        assert!(tool_names.contains(&crate::resident::INVOKE));
        assert!(tool_names.contains(&crate::resident::DIRECT_COMPUTE_OFFLOAD_ROUTE));
    }

    #[test]
    fn messages_url_for_base_preserves_request_query() {
        let config = proxy_config();
        let uri = "/v1/messages?beta=1".parse::<Uri>().unwrap();
        assert_eq!(
            config.messages_url_for_base("http://127.0.0.1:11434/", &uri),
            "http://127.0.0.1:11434/v1/messages?beta=1"
        );
    }

    #[tokio::test]
    async fn resident_direct_affordance_resolves_inline_without_installed_mcp() {
        let mut config = proxy_config();
        config.harness_mcp_url = "http://127.0.0.1:9/mcp".to_string();
        let state = ProxyState {
            config,
            http: reqwest::Client::builder().build().unwrap(),
            presence: LocalPresenceRegistry::new(),
        };
        let block = execute_resident_tool_use(
            &state,
            crate::resident::ResidentToolUse {
                id: "toolu_direct".to_string(),
                name: crate::resident::DIRECT_COMPUTE_OFFLOAD_ROUTE.to_string(),
                input: json!({
                    "operations": [{
                        "operation_id": "verify",
                        "kind": "verification_check",
                        "quality_floor": 0.9
                    }]
                }),
            },
        )
        .await;
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["is_error"], false);
        let content = block["content"].as_str().unwrap();
        assert!(content.contains("offload_plan"));
        assert!(content.contains("verify"));
    }

    #[test]
    fn stored_tool_result_fetches_byte_slices() {
        let handle = store_tool_result("abcdef");
        let fetched = fetch_tool_result_slice(&handle, 2, 3).unwrap();
        assert_eq!(fetched["text"], "cde");
        assert_eq!(fetched["next_offset"], json!(5));
    }

    #[test]
    fn latest_user_text_ignores_tool_result_blocks() {
        let request = base_request(json!([
            {"type": "tool_result", "tool_use_id": "toolu_1", "content": "result"},
            {"type": "text", "text": "actual user words"}
        ]));
        let raw = serde_json::to_vec(&request).unwrap();
        assert_eq!(latest_user_text(&raw).unwrap(), "actual user words");
    }

    #[test]
    fn agent_presence_defaults_to_codex_in_config_room() {
        let config = proxy_config();
        let request = AgentPresenceRequest::default();
        let resolved = request.resolve(&config, false);
        assert_eq!(resolved.actor_id, "codex");
        assert_eq!(resolved.room_id, "repo:theorem:branch:main");
        assert_eq!(resolved.status, "active");

        let room_args = room_join_arguments(&config, &resolved);
        assert_eq!(room_args["tenant_slug"], "Travis-Gilbert");
        assert_eq!(room_args["action"], "join");
        assert_eq!(room_args["actor"], "codex");
        assert_eq!(room_args["room_id"], "repo:theorem:branch:main");
    }

    #[test]
    fn agent_presence_summary_advertises_codex_endpoint() {
        let config = proxy_config();
        let request = AgentPresenceRequest {
            endpoint: Some("ws://127.0.0.1:18489".to_string()),
            capabilities: vec!["codex.app-server".to_string(), "codex.exec".to_string()],
            ..AgentPresenceRequest::default()
        };
        let resolved = request.resolve(&config, false);
        assert_eq!(resolved.surface, "codex:app-server");
        assert!(resolved.summary.contains("ws://127.0.0.1:18489"));
        assert!(resolved.summary.contains("codex.app-server"));
    }

    #[test]
    fn local_presence_registry_matches_pr60_overlap_contract() {
        let registry = LocalPresenceRegistry::new();
        registry.announce(AnnounceLocalPresenceRequest {
            actor: "claude-code".to_string(),
            path: "src/lib.rs".to_string(),
            line: 10,
            col: 0,
            label: None,
            kind: None,
            ttl_seconds: None,
        });
        registry.announce(AnnounceLocalPresenceRequest {
            actor: "codex".to_string(),
            path: "src/lib.rs".to_string(),
            line: 40,
            col: 0,
            label: None,
            kind: None,
            ttl_seconds: None,
        });
        registry.set_footprint(SetLocalFootprintRequest {
            actor: "claude-code".to_string(),
            path: "src/lib.rs".to_string(),
            range: FileRange::new(10, 0, 20, 0),
            summary: Some("refactor fn a".to_string()),
        });
        registry.set_footprint(SetLocalFootprintRequest {
            actor: "codex".to_string(),
            path: "src/lib.rs".to_string(),
            range: FileRange::new(15, 0, 25, 0),
            summary: Some("rename fn b".to_string()),
        });

        let overlaps = registry.would_overlap(&WouldOverlapLocalRequest {
            actor: "claude-code".to_string(),
            path: "src/lib.rs".to_string(),
            intended: FileRange::new(10, 0, 20, 0),
        });

        assert_eq!(overlaps.len(), 1);
        assert_eq!(overlaps[0].actor, ActorId::from_label("codex"));
        assert_eq!(overlaps[0].summary.as_deref(), Some("rename fn b"));
    }

    #[test]
    fn local_presence_registry_replaces_actor_path_and_clears_footprint() {
        let registry = LocalPresenceRegistry::new();
        registry.announce(AnnounceLocalPresenceRequest {
            actor: "codex".to_string(),
            path: "src/main.rs".to_string(),
            line: 1,
            col: 0,
            label: None,
            kind: None,
            ttl_seconds: None,
        });
        registry.announce(AnnounceLocalPresenceRequest {
            actor: "codex".to_string(),
            path: "src/main.rs".to_string(),
            line: 12,
            col: 4,
            label: None,
            kind: None,
            ttl_seconds: None,
        });
        let presences = registry.list_presences();
        assert_eq!(presences.len(), 1);
        assert_eq!((presences[0].line, presences[0].col), (12, 4));

        registry.set_footprint(SetLocalFootprintRequest {
            actor: "codex".to_string(),
            path: "src/main.rs".to_string(),
            range: FileRange::new(12, 0, 14, 0),
            summary: None,
        });
        assert_eq!(registry.list_footprints().len(), 1);
        assert!(registry.clear_footprint(&ClearLocalFootprintRequest {
            actor: "codex".to_string(),
            path: "src/main.rs".to_string(),
        }));
        assert!(registry.list_footprints().is_empty());
    }

    #[test]
    fn agent_context_arguments_target_native_reflections() {
        let config = proxy_config();
        let query = AgentContextQuery {
            actor: Some(" codex ".to_string()),
            room_id: Some("repo:theorem:branch:main".to_string()),
            record_type: Some("reflection".to_string()),
            limit: Some(12),
            ..AgentContextQuery::default()
        };
        let args = context_arguments(&config, "repo:theorem:branch:main", Some(" codex "), &query);
        assert_eq!(args["tenant_slug"], "Travis-Gilbert");
        assert_eq!(args["room_id"], "repo:theorem:branch:main");
        assert_eq!(args["actor"], "codex");
        assert_eq!(args["record_type"], "reflection");
        assert_eq!(args["limit"], 12);
    }

    #[test]
    fn agent_record_defaults_to_reflection_contract() {
        let config = proxy_config();
        let request = AgentRecordRequest {
            title: Some("Codex continuity".to_string()),
            body: Some("Turn context read; endpoint is live.".to_string()),
            metadata: Some(json!({"protocol": "native-coordination-continuity-v1"})),
            ..AgentRecordRequest::default()
        };
        let args = record_arguments(
            &config,
            "codex",
            "repo:theorem:branch:main",
            "reflection",
            "Codex endpoint ready".to_string(),
            request,
        );
        assert_eq!(args["tenant_slug"], "Travis-Gilbert");
        assert_eq!(args["actor"], "codex");
        assert_eq!(args["record_type"], "reflection");
        assert_eq!(args["summary"], "Codex endpoint ready");
        assert_eq!(
            args["metadata"]["protocol"],
            "native-coordination-continuity-v1"
        );
    }

    #[test]
    fn mcp_payload_prefers_structured_content() {
        let value = json!({
            "jsonrpc": "2.0",
            "id": "x",
            "result": {
                "content": [{"type": "text", "text": "{\"ignored\":true}"}],
                "structuredContent": {"presence": {"actor_id": "codex"}}
            }
        });
        let payload = mcp_payload_from_response(&value).unwrap();
        assert_eq!(payload["presence"]["actor_id"], "codex");
    }
}
