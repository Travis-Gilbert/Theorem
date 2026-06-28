//! The local instance's authenticated control endpoint (phone-control handoff
//! Part B deliverable 1).
//!
//! A minimal axum HTTP surface, bound to LOOPBACK ONLY (`127.0.0.1`), that lets a
//! paired device drive the local instance. This is a trust boundary, so the rules
//! are deliberately narrow:
//!
//! * **Loopback only.** [`serve`] binds `127.0.0.1` (never `0.0.0.0`). Reaching
//!   the instance from another device is the relay's job (a LATER slice), not an
//!   open bind.
//! * **Pairing is gated.** `POST /pair` requires the one-time pairing code printed
//!   at startup (the `x-pairing-code` header); it is NOT an open endpoint. Only
//!   after pairing does a device hold a token.
//! * **`/v1/*` requires a valid device token.** The [`require_device`] extractor
//!   checks the `authorization: Bearer <token>` header against the
//!   [`DevicePairing`] registry in constant time; a missing/garbage/revoked token
//!   is `401 Unauthorized`. `GET /healthz` is the only unauthenticated route.
//!
//! Routes:
//! * `GET  /healthz`     -> `200 "ok"` (unauthenticated liveness).
//! * `POST /pair`        -> issue a device token; gated by the pairing code.
//! * `GET  /v1/status`   -> authenticated probe (proves the auth extractor works).
//! * `GET  /v1/devices`  -> authenticated: list paired devices (secret-free).
//! * `POST /v1/devices/revoke` -> authenticated: revoke a device by id.
//! * `POST /v1/runs`     -> authenticated: submit a run (B3). Returns its id +
//!   initial state (tier-2/3 runs come back `awaiting_authorization`).
//! * `GET  /v1/runs`     -> authenticated: list run records.
//! * `GET  /v1/runs/:id` -> authenticated: one run record (state + backlog).
//! * `GET  /v1/runs/:id/events` -> authenticated: SSE stream of the run's events
//!   (Trace / Obligation / Diff / Status), backlog first then live tail.
//! * `POST /v1/runs/:id/approve`  -> authenticated: release a held run (B5).
//! * `POST /v1/runs/:id/redirect` -> authenticated: inject an instruction (B5).
//! * `POST /v1/runs/:id/stop`     -> authenticated: cooperatively cancel (B5).
//! * `POST /v1/presence`          -> authenticated: announce/update agent presence.
//! * `GET  /v1/presence`          -> authenticated: list presences + footprints.
//! * `POST /v1/presence/footprint`   -> authenticated: set a pending-edit footprint.
//! * `DELETE /v1/presence/footprint` -> authenticated: clear a pending-edit footprint.
//! * `POST /v1/presence/would-overlap` -> authenticated: peers' overlapping edits.
//!
//! The `/v1/presence*` routes are the agent co-presence layer (spec:
//! `HANDOFF-AGENT-COEDIT-PRESENCE-LAYER.md`): concurrent agent processes share
//! cursor + pending-edit footprints over this one local instance so coordination
//! is ambient presence, not filesystem recon. It is AWARENESS ONLY -- source bytes
//! still merge via git (`CodeContentStrategy::GitMergeOnly`); no route writes file
//! bytes.
//!
//! Credentials/passwords are NOT handled here: pairing issues a device TOKEN, not
//! a user login. Tier-gated action authorization (B4) is grounded against
//! `theorem-harness-core` `agent_binding` (see [`crate::authorization`]): a
//! submitted run is classified to an action tier, and a tier the binding marks
//! `requires_human_authorization` is HELD (`awaiting_authorization`) until an
//! explicit `approve` arrives. The gated action never runs before approval.
//!
//! Every `/v1/runs*` route is behind [`DeviceAuth`], like the rest of `/v1/*`.
//!
//! Nothing here logs a token or the pairing code.

use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::pairing::{DevicePairing, DeviceSummary};
use crate::presence::{
    AgentPresence, CodeEditFootprint, FileRange, FootprintAnnouncement, PresenceAnnouncement,
    PresenceKind, PresenceRegistry,
};
use crate::runs::{RunEvent, RunError, RunRecord, RunRegistry, RunSpec};
use crate::sink::SharedSink;
use crate::Result;

use commonplace::{Collection, Item};
use rustyred_thg_core::ActorId;

/// Default and ceiling page sizes for `GET /v1/items`. A same-machine webapp
/// paginates; an unbounded list over a large store would be a heavy response.
const DEFAULT_ITEM_LIMIT: usize = 100;
const MAX_ITEM_LIMIT: usize = 1000;
/// Default and ceiling result counts for `GET /v1/search`.
const DEFAULT_SEARCH_K: usize = 20;
const MAX_SEARCH_K: usize = 200;

/// Loopback address the control endpoint binds. A local instance never binds a
/// routable interface; cross-device reach is the relay's concern.
pub const LOOPBACK: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// Header carrying the one-time pairing code on `POST /pair`.
pub const PAIRING_CODE_HEADER: &str = "x-pairing-code";

/// Shared state for the control router: the device registry, the active pairing
/// code (the one-time code a device must present to pair), and the run channel
/// (the registry of phone-submitted runs).
#[derive(Clone)]
pub struct ControlState {
    pairing: DevicePairing,
    /// The pairing code required by `POST /pair`. Wrapped in `Arc<str>` so the
    /// state stays cheap to clone across handlers.
    pairing_code: Arc<str>,
    /// The run channel: submit/stream/control phone-driven runs. Cloneable.
    runs: RunRegistry,
    /// The workspace presence registry: cross-process agent co-presence + the
    /// `would_overlap` query (awareness only; bytes still merge via git). Cloneable.
    presence: PresenceRegistry,
    /// The shared durable commonplace the ambient watcher writes to. `Some` enables
    /// the local-first DATA routes (`/v1/items*`, `/v1/collections`, `/v1/search`)
    /// over the SAME graph the watcher maintains (one process, one graph), so a
    /// watcher ingest is immediately visible to a read. `None` when the endpoint
    /// runs without a sink (the data routes then return `503 Service Unavailable`).
    data: Option<SharedSink>,
}

impl ControlState {
    /// Build control state over an open [`DevicePairing`] registry, a pairing
    /// code, and a [`RunRegistry`]. The pairing code is the gate on `POST /pair`;
    /// surface it to the user at startup (the desktop app prints it / shows it in
    /// a settings pane) so a device can pair once. It is compared in constant time
    /// on each pair attempt. The run registry powers the `/v1/runs*` surface.
    ///
    /// A fresh empty [`PresenceRegistry`] is created for the co-presence surface;
    /// use [`with_presence`](Self::with_presence) to share an existing one (e.g.
    /// when the desktop app wants to inspect presence out-of-band).
    pub fn new(pairing: DevicePairing, pairing_code: impl Into<String>, runs: RunRegistry) -> Self {
        Self {
            pairing,
            pairing_code: Arc::from(pairing_code.into().as_str()),
            runs,
            presence: PresenceRegistry::new(),
            data: None,
        }
    }

    /// Use a specific [`PresenceRegistry`] for the `/v1/presence*` surface (rather
    /// than the fresh one [`new`](Self::new) creates), so the desktop app can hold
    /// a clone of the same registry the control endpoint serves.
    pub fn with_presence(mut self, presence: PresenceRegistry) -> Self {
        self.presence = presence;
        self
    }

    /// Wire the local-first DATA routes (`/v1/items*`, `/v1/collections`,
    /// `/v1/search`) over the shared durable commonplace the ambient watcher
    /// writes to. Pass [`AmbientRuntime::shared_sink`](crate::AmbientRuntime::shared_sink)
    /// (or [`SharedSink`]) so the endpoint reads the SAME graph the watcher
    /// maintains -- one process, one graph, no second AOF handle. Without this, the
    /// data routes return `503`.
    pub fn with_data(mut self, data: SharedSink) -> Self {
        self.data = Some(data);
        self
    }

    /// Borrow the shared data handle, if the data routes are wired.
    pub fn data(&self) -> Option<&SharedSink> {
        self.data.as_ref()
    }

    /// Borrow the device registry (for the desktop app to list/revoke devices
    /// out-of-band, or for tests).
    pub fn pairing(&self) -> &DevicePairing {
        &self.pairing
    }

    /// Borrow the run registry (for the desktop app or tests to inspect runs).
    pub fn runs(&self) -> &RunRegistry {
        &self.runs
    }

    /// Borrow the presence registry (for the desktop app or tests to inspect the
    /// workspace co-presence state out-of-band).
    pub fn presence(&self) -> &PresenceRegistry {
        &self.presence
    }

    /// Check a presented pairing code against the active one in constant time.
    fn pairing_code_matches(&self, presented: &str) -> bool {
        crate::pairing::constant_time_str_eq(presented, &self.pairing_code)
    }
}

/// An authenticated device, produced by the [`require_device`](DeviceAuth) axum
/// extractor. Its presence in a handler signature proves the request carried a
/// valid (non-revoked) device token; without one the extractor short-circuits to
/// `401` before the handler runs.
pub struct DeviceAuth;

impl<S> FromRequestParts<S> for DeviceAuth
where
    ControlState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> std::result::Result<Self, Response> {
        let control = ControlState::from_ref(state);
        let token = bearer_token(parts).ok_or_else(unauthorized)?;
        if control.pairing.verify(&token).is_authorized() {
            Ok(DeviceAuth)
        } else {
            Err(unauthorized())
        }
    }
}

/// Re-export of axum's `FromRef` so the extractor bound reads cleanly.
use axum::extract::FromRef;

/// Extract the bearer token from the `authorization` header (`Bearer <token>`),
/// or `None` if absent/malformed. The scheme match is case-insensitive per RFC
/// 7235; the token itself is returned verbatim for the registry to verify.
fn bearer_token(parts: &Parts) -> Option<String> {
    let value = parts.headers.get(axum::http::header::AUTHORIZATION)?;
    let value = value.to_str().ok()?;
    let (scheme, token) = value.split_once(' ')?;
    if !scheme.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

/// The canonical `401 Unauthorized` response for `/v1/*` without a valid token.
/// No detail that would help an attacker distinguish "unknown device" from
/// "wrong/revoked token" (the registry already verifies in constant time).
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(ErrorBody {
            error: "unauthorized".to_string(),
        }),
    )
        .into_response()
}

/// A small JSON error envelope.
#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

/// Request body for `POST /pair`.
#[derive(Debug, Deserialize)]
pub struct PairRequest {
    /// Human-facing device label (e.g. "Travis iPhone").
    pub label: String,
}

/// Response body for `POST /pair`: the new device plus its one-time token.
#[derive(Debug, Serialize)]
pub struct PairResponse {
    pub device: DeviceSummary,
    /// The raw bearer token, returned ONCE. The device stores it and presents it
    /// as `authorization: Bearer <token>` on `/v1/*`.
    pub token: String,
}

/// Response body for `GET /v1/status`.
#[derive(Debug, Serialize)]
pub struct StatusResponse {
    pub status: &'static str,
    /// Number of devices on record (active + revoked).
    pub paired_devices: usize,
}

/// Response body for `GET /v1/devices`.
#[derive(Debug, Serialize)]
pub struct DevicesResponse {
    pub devices: Vec<DeviceSummary>,
}

/// Request body for `POST /v1/devices/revoke`.
#[derive(Debug, Deserialize)]
pub struct RevokeRequest {
    pub device_id: String,
}

/// Response body for `POST /v1/devices/revoke`.
#[derive(Debug, Serialize)]
pub struct RevokeResponse {
    /// Whether a device with that id existed and is now revoked.
    pub revoked: bool,
}

// ---------------------------------------------------------------------------
// Local-first commonplace DATA shapes + handlers (read-only).
//
// The wire shape is a deliberate projection of the commonplace `Item` /
// `Collection`: it carries the fields a same-machine webapp needs to list /
// open / search, and drops the heavy embedding vector and any raw blob bytes
// (a `File` item reports its content hash / byte length / mime, not its bytes).
// ---------------------------------------------------------------------------

/// JSON projection of a commonplace [`Item`] for the data routes.
#[derive(Debug, Serialize)]
pub struct ItemJson {
    pub id: String,
    /// Canonical lowercase kind token (`note` / `doc` / `file` / `link` / ...).
    pub kind: String,
    pub title: String,
    /// Inline text body, when the item has one (notes / docs). A `File`'s bytes
    /// are NOT inlined; see `blob`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// For a `File` item: its content-addressed blob reference (hash + size +
    /// mime). Absent for non-file / empty-body items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<BlobRefJson>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Residency token (`local` / `synced` / `hosted`).
    pub residency: String,
    pub tags: Vec<String>,
    /// Collection ids this item belongs to (edge-canonical).
    pub collections: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A `File` item's content-addressed blob reference (no bytes).
#[derive(Debug, Serialize)]
pub struct BlobRefJson {
    pub content_hash: String,
    pub byte_len: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

impl From<Item> for ItemJson {
    fn from(item: Item) -> Self {
        use commonplace::ItemBody;
        let (text, blob) = match &item.body {
            ItemBody::Inline { text } => (Some(text.clone()), None),
            ItemBody::Blob {
                content_hash,
                byte_len,
                mime,
            } => (
                None,
                Some(BlobRefJson {
                    content_hash: content_hash.clone(),
                    byte_len: *byte_len,
                    mime: mime.clone(),
                }),
            ),
            ItemBody::Empty => (None, None),
        };
        ItemJson {
            id: item.id,
            kind: item.kind.as_str().to_string(),
            title: item.title,
            text,
            blob,
            source: item.source,
            residency: item.residency.as_str().to_string(),
            tags: item.tags,
            collections: item.collections,
            classification: item.classification,
            created_at_ms: item.created_at_ms,
            updated_at_ms: item.updated_at_ms,
        }
    }
}

/// JSON projection of a commonplace [`Collection`].
#[derive(Debug, Serialize)]
pub struct CollectionJson {
    pub id: String,
    pub name: String,
    /// `manual` (user-made) or `auto` (coined by ingest classification).
    pub kind: String,
    pub created_at_ms: i64,
}

impl From<Collection> for CollectionJson {
    fn from(collection: Collection) -> Self {
        CollectionJson {
            id: collection.id,
            name: collection.name,
            kind: collection.kind.as_str().to_string(),
            created_at_ms: collection.created_at_ms,
        }
    }
}

/// Response body for `GET /v1/items`.
#[derive(Debug, Serialize)]
pub struct ItemListResponse {
    pub items: Vec<ItemJson>,
}

/// Response body for `GET /v1/items/{id}`.
#[derive(Debug, Serialize)]
pub struct ItemDetailResponse {
    pub item: ItemJson,
}

/// Response body for `GET /v1/collections`.
#[derive(Debug, Serialize)]
pub struct CollectionListResponse {
    pub collections: Vec<CollectionJson>,
}

/// Query params for `GET /v1/items`.
#[derive(Debug, Deserialize)]
pub struct ItemListParams {
    /// Page size (clamped to `[1, MAX_ITEM_LIMIT]`; defaults to `DEFAULT_ITEM_LIMIT`).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Query params for `GET /v1/search`.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    /// The query string.
    pub q: String,
    /// Result count (clamped to `[1, MAX_SEARCH_K]`; defaults to `DEFAULT_SEARCH_K`).
    #[serde(default)]
    pub k: Option<usize>,
}

/// One search hit: the matched item plus its similarity score.
#[derive(Debug, Serialize)]
pub struct SearchHitJson {
    pub item: ItemJson,
    pub score: f32,
}

/// Response body for `GET /v1/search`.
#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHitJson>,
}

/// Build the control-plane router over the given [`ControlState`]. This is the
/// testable core (drive it with `tower::ServiceExt::oneshot`); [`serve`] wraps it
/// with a loopback bind.
pub fn build_router(state: ControlState) -> Router {
    Router::new()
        // Unauthenticated liveness.
        .route("/healthz", get(healthz))
        // Pairing: gated by the one-time pairing code, NOT open.
        .route("/pair", post(pair_handler))
        // Authenticated control surface.
        .route("/v1/status", get(status_handler))
        .route("/v1/devices", get(devices_handler))
        .route("/v1/devices/revoke", post(revoke_handler))
        // Authenticated run channel (B3 + B5). Every route is DeviceAuth-gated.
        .route("/v1/runs", get(list_runs_handler).post(submit_run_handler))
        .route("/v1/runs/{run_id}", get(run_detail_handler))
        .route("/v1/runs/{run_id}/events", get(run_events_handler))
        .route("/v1/runs/{run_id}/approve", post(approve_run_handler))
        .route("/v1/runs/{run_id}/redirect", post(redirect_run_handler))
        .route("/v1/runs/{run_id}/stop", post(stop_run_handler))
        // Authenticated agent co-presence surface. Every route is DeviceAuth-gated.
        // Announce/list presence, set/clear a pending-edit footprint, and the
        // overlap query. Awareness only: nothing here writes file bytes.
        .route(
            "/v1/presence",
            get(list_presence_handler).post(announce_presence_handler),
        )
        .route(
            "/v1/presence/footprint",
            post(set_footprint_handler).delete(clear_footprint_handler),
        )
        .route("/v1/presence/would-overlap", post(would_overlap_handler))
        // Authenticated local-first DATA surface (read-only). Every route is
        // DeviceAuth-gated and reads the SAME durable commonplace graph the ambient
        // watcher writes to, so a same-machine webapp gets its items/collections/
        // search from this local instance as its primary backend.
        .route("/v1/items", get(list_items_handler))
        .route("/v1/items/{id}", get(item_detail_handler))
        .route("/v1/collections", get(list_collections_handler))
        .route("/v1/search", get(search_handler))
        .with_state(state)
}

async fn healthz() -> &'static str {
    "ok"
}

/// `POST /pair`: issue a device token. Gated by the one-time pairing code in the
/// `x-pairing-code` header; a missing/wrong code is `403 Forbidden` (the resource
/// exists but the caller is not allowed to pair without the code).
async fn pair_handler(
    State(state): State<ControlState>,
    headers: axum::http::HeaderMap,
    Json(request): Json<PairRequest>,
) -> std::result::Result<Json<PairResponse>, Response> {
    let presented = headers
        .get(PAIRING_CODE_HEADER)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !state.pairing_code_matches(presented) {
        return Err((
            StatusCode::FORBIDDEN,
            Json(ErrorBody {
                error: "invalid pairing code".to_string(),
            }),
        )
            .into_response());
    }
    let label = request.label.trim();
    if label.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "label must not be empty".to_string(),
            }),
        )
            .into_response());
    }
    let result = state.pairing.pair_device(label).map_err(|error| {
        // Do not leak internals (e.g. a CSPRNG/persist error) to the client.
        eprintln!("commonplace-desktop-runtime: pairing failed: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "pairing failed".to_string(),
            }),
        )
            .into_response()
    })?;
    Ok(Json(PairResponse {
        device: result.device,
        token: result.token,
    }))
}

/// `GET /v1/status`: authenticated probe. The `DeviceAuth` extractor enforces a
/// valid token before this runs.
async fn status_handler(_auth: DeviceAuth, State(state): State<ControlState>) -> Json<StatusResponse> {
    Json(StatusResponse {
        status: "ok",
        paired_devices: state.pairing.device_count(),
    })
}

/// `GET /v1/devices`: authenticated list of paired devices (secret-free).
async fn devices_handler(_auth: DeviceAuth, State(state): State<ControlState>) -> Json<DevicesResponse> {
    Json(DevicesResponse {
        devices: state.pairing.list_devices(),
    })
}

/// `POST /v1/devices/revoke`: authenticated revoke by device id.
async fn revoke_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<RevokeRequest>,
) -> std::result::Result<Json<RevokeResponse>, Response> {
    let revoked = state.pairing.revoke_device(&request.device_id).map_err(|error| {
        eprintln!("commonplace-desktop-runtime: revoke failed: {error}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody {
                error: "revoke failed".to_string(),
            }),
        )
            .into_response()
    })?;
    Ok(Json(RevokeResponse { revoked }))
}

// ---------------------------------------------------------------------------
// Local-first commonplace DATA handlers (read-only). All are DeviceAuth-gated.
//
// They read the shared durable commonplace the ambient watcher writes to (the
// SAME graph; one process, one graph), so a same-machine webapp uses this local
// instance as its primary backend for items / collections / search. The slice is
// READ-ONLY: nothing here mutates the graph over HTTP (writes stay the watcher's
// job). When the endpoint runs with no sink wired, these return `503`.
// ---------------------------------------------------------------------------

/// `GET /v1/items`: list items (newest first), capped by `limit`. Authenticated.
/// Reads the live durable graph via `SharedSink::list_items` (over
/// `Commonplace::all_items`).
async fn list_items_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Query(params): Query<ItemListParams>,
) -> std::result::Result<Json<ItemListResponse>, Response> {
    let data = state.data.as_ref().ok_or_else(data_unavailable)?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_ITEM_LIMIT)
        .clamp(1, MAX_ITEM_LIMIT);
    let items = data.list_items(limit).map_err(store_error)?;
    Ok(Json(ItemListResponse {
        items: items.into_iter().map(ItemJson::from).collect(),
    }))
}

/// `GET /v1/items/{id}`: one item by id. Authenticated. `404` if no such item.
/// Reads via `SharedSink::get_item` (over `Commonplace::get_item`).
async fn item_detail_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(id): Path<String>,
) -> std::result::Result<Json<ItemDetailResponse>, Response> {
    let data = state.data.as_ref().ok_or_else(data_unavailable)?;
    match data.get_item(&id).map_err(store_error)? {
        Some(item) => Ok(Json(ItemDetailResponse {
            item: ItemJson::from(item),
        })),
        None => Err(item_not_found()),
    }
}

/// `GET /v1/collections`: every collection. Authenticated. Reads via
/// `SharedSink::list_collections` (mirrors the consumer API's `collections`
/// query: `query_nodes(label = Collection)` then `get_collection`).
async fn list_collections_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
) -> std::result::Result<Json<CollectionListResponse>, Response> {
    let data = state.data.as_ref().ok_or_else(data_unavailable)?;
    let collections = data.list_collections().map_err(store_error)?;
    Ok(Json(CollectionListResponse {
        collections: collections.into_iter().map(CollectionJson::from).collect(),
    }))
}

/// `GET /v1/search?q=...&k=...`: similarity search over items. Authenticated.
/// Reads via `SharedSink::search` (the REAL commonplace vector search,
/// `IngestPipeline::search`, over the engine embedding index the ambient ingest
/// populates). A blank `q` is `400`.
async fn search_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Query(params): Query<SearchParams>,
) -> std::result::Result<Json<SearchResponse>, Response> {
    let data = state.data.as_ref().ok_or_else(data_unavailable)?;
    let query = params.q.trim();
    if query.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "q must not be empty".to_string(),
            }),
        )
            .into_response());
    }
    let k = params.k.unwrap_or(DEFAULT_SEARCH_K).clamp(1, MAX_SEARCH_K);
    let hits = data.search(query, k).map_err(store_error)?;
    Ok(Json(SearchResponse {
        hits: hits
            .into_iter()
            .map(|(item, score)| SearchHitJson {
                item: ItemJson::from(item),
                score,
            })
            .collect(),
    }))
}

/// `503 Service Unavailable` for a data route when no sink is wired (the endpoint
/// is up, but the commonplace data backend is not attached).
fn data_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ErrorBody {
            error: "commonplace data backend not available".to_string(),
        }),
    )
        .into_response()
}

/// `404` for an unknown item id.
fn item_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            error: "item not found".to_string(),
        }),
    )
        .into_response()
}

/// Map a graph store error to a generic `500` (its `Debug` carries code + message;
/// log it server-side, return an opaque body so internals do not leak).
fn store_error(error: rustyred_thg_core::GraphStoreError) -> Response {
    eprintln!("commonplace-desktop-runtime: data route store error: {error:?}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorBody {
            error: "data query failed".to_string(),
        }),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Run channel handlers (B3 + B5). All are DeviceAuth-gated.
// ---------------------------------------------------------------------------

/// Request body for `POST /v1/runs`.
#[derive(Debug, Deserialize)]
pub struct SubmitRunRequest {
    /// The run intent / instruction.
    pub intent: String,
    /// Optional action tier id (defaults to tier-1 reversible). Use the
    /// `authorization::TIER_*` ids; an unknown tier fails safe to a hold.
    #[serde(default)]
    pub action_tier: Option<String>,
    /// Whether the human authorized this run up front (lets a tier-2/3 run
    /// proceed without a separate approve).
    #[serde(default)]
    pub human_authorized: bool,
}

/// Response body for `POST /v1/runs`: the new run id and its initial state.
#[derive(Debug, Serialize)]
pub struct SubmitRunResponse {
    pub run_id: String,
    /// The run's state right after submission. A tier-2/3 run with no upfront
    /// authorization comes back `awaiting_authorization`.
    pub state: String,
}

/// Response body for `GET /v1/runs`.
#[derive(Debug, Serialize)]
pub struct RunListResponse {
    pub runs: Vec<RunRecord>,
}

/// Response body for the run-control routes (approve/redirect/stop) and the
/// detail route.
#[derive(Debug, Serialize)]
pub struct RunControlResponse {
    pub run: RunRecord,
}

/// Request body for `POST /v1/runs/:id/redirect`.
#[derive(Debug, Deserialize)]
pub struct RedirectRunRequest {
    pub instruction: String,
}

/// `POST /v1/runs`: submit a run. Authenticated. The run is classified to its
/// action tier and either dispatched immediately (tier-1 / pre-authorized) or
/// held (`awaiting_authorization`) for tier-2/3 -- the gating comes from
/// `agent_binding` via [`crate::authorization`].
async fn submit_run_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<SubmitRunRequest>,
) -> std::result::Result<Json<SubmitRunResponse>, Response> {
    let intent = request.intent.trim();
    if intent.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "intent must not be empty".to_string(),
            }),
        )
            .into_response());
    }
    let spec = RunSpec {
        intent: intent.to_string(),
        action_tier: request
            .action_tier
            .unwrap_or_else(|| crate::authorization::TIER_ONE.to_string()),
        human_authorized: request.human_authorized,
    };
    let run_id = state.runs.submit(spec);
    // Report the post-submission state (held vs running) so the phone knows
    // whether an approve is needed.
    let run_state = state
        .runs
        .record(&run_id)
        .map(|record| run_state_tag(&record))
        .unwrap_or("submitted");
    Ok(Json(SubmitRunResponse {
        run_id,
        state: run_state.to_string(),
    }))
}

/// `GET /v1/runs`: list every run record. Authenticated.
async fn list_runs_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
) -> Json<RunListResponse> {
    Json(RunListResponse {
        runs: state.runs.list(),
    })
}

/// `GET /v1/runs/:id`: one run record (state + event backlog). Authenticated.
async fn run_detail_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(run_id): Path<String>,
) -> std::result::Result<Json<RunControlResponse>, Response> {
    match state.runs.record(&run_id) {
        Some(run) => Ok(Json(RunControlResponse { run })),
        None => Err(run_not_found()),
    }
}

/// `GET /v1/runs/:id/events`: Server-Sent Events stream of the run's events.
/// Authenticated. The stream replays the run's backlog first (so a phone that
/// attaches late does not miss early traces) and then tails live events until
/// the run reaches a terminal state, at which point the stream ends.
async fn run_events_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(run_id): Path<String>,
) -> std::result::Result<Sse<impl Stream<Item = std::result::Result<SseEvent, Infallible>>>, Response>
{
    let (backlog, receiver) = state.runs.subscribe(&run_id).ok_or_else(run_not_found)?;
    let stream = run_event_stream(backlog, receiver);
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(15))))
}

/// `POST /v1/runs/:id/approve`: release a held run. Authenticated. A run that is
/// not awaiting authorization yields `409 Conflict`.
async fn approve_run_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(run_id): Path<String>,
) -> Response {
    match state.runs.approve(&run_id) {
        Ok(()) => run_control_ok(&state, &run_id),
        Err(error) => run_error_response(error),
    }
}

/// `POST /v1/runs/:id/redirect`: inject a steering instruction. Authenticated.
async fn redirect_run_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(run_id): Path<String>,
    Json(request): Json<RedirectRunRequest>,
) -> Response {
    let instruction = request.instruction.trim();
    if instruction.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorBody {
                error: "instruction must not be empty".to_string(),
            }),
        )
            .into_response();
    }
    match state.runs.redirect(&run_id, instruction) {
        Ok(()) => run_control_ok(&state, &run_id),
        Err(error) => run_error_response(error),
    }
}

/// `POST /v1/runs/:id/stop`: cooperatively cancel a run. Authenticated.
async fn stop_run_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Path(run_id): Path<String>,
) -> Response {
    match state.runs.stop(&run_id) {
        Ok(()) => run_control_ok(&state, &run_id),
        Err(error) => run_error_response(error),
    }
}

/// Common success body for the control routes: the run's current record, mapped
/// to the JSON envelope or a `404` if the run vanished between the control op and
/// the read. Returns the `Response` directly (success or error) so the handlers
/// stay free of a large-`Err` `Result` helper.
fn run_control_ok(state: &ControlState, run_id: &str) -> Response {
    match state.runs.record(run_id) {
        Some(run) => Json(RunControlResponse { run }).into_response(),
        None => run_not_found(),
    }
}

/// Map a [`RunError`] to an HTTP response: `404` for an unknown run, `409` for a
/// control invalid in the run's current state.
fn run_error_response(error: RunError) -> Response {
    match error {
        RunError::NotFound => run_not_found(),
        RunError::InvalidState { .. } => (
            StatusCode::CONFLICT,
            Json(ErrorBody {
                error: error.to_string(),
            }),
        )
            .into_response(),
    }
}

fn run_not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorBody {
            error: "run not found".to_string(),
        }),
    )
        .into_response()
}

/// Lower-case tag for a run record's state (for the submit response body).
fn run_state_tag(record: &RunRecord) -> &'static str {
    use crate::runs::RunState;
    match record.state {
        RunState::Submitted => "submitted",
        RunState::AwaitingAuthorization => "awaiting_authorization",
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Stopped => "stopped",
        RunState::Failed => "failed",
    }
}

/// Build the SSE event stream for a run: yield the backlog first, then live
/// events from the broadcast receiver, ending when a terminal `Status` event is
/// seen (or the channel closes). Each [`RunEvent`] is serialized as JSON in the
/// SSE data field, with the event `kind` as the SSE event name.
fn run_event_stream(
    backlog: Vec<RunEvent>,
    receiver: broadcast::Receiver<RunEvent>,
) -> impl Stream<Item = std::result::Result<SseEvent, Infallible>> {
    use futures_util::stream::{self, StreamExt};

    // Whether the backlog already contains a terminal status (so we should not
    // wait on the live channel at all).
    let backlog_terminal = backlog.iter().any(is_terminal_event);

    let backlog_stream = stream::iter(backlog.into_iter().map(|event| Ok(sse_event(&event))));

    // The live tail: unfold over the receiver, stopping after a terminal event.
    let live_stream = stream::unfold(
        (receiver, backlog_terminal),
        |(mut receiver, stop)| async move {
            if stop {
                return None;
            }
            loop {
                match receiver.recv().await {
                    Ok(event) => {
                        let terminal = is_terminal_event(&event);
                        return Some((Ok(sse_event(&event)), (receiver, terminal)));
                    }
                    // Lagged: skip the missed window and keep tailing (the client
                    // can reconcile via the backlog / record route).
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    // Sender dropped: the run is gone; end the stream.
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    );

    backlog_stream.chain(live_stream)
}

/// Whether an event marks a terminal lifecycle state (so the SSE stream can end).
fn is_terminal_event(event: &RunEvent) -> bool {
    use crate::runs::RunEventKind;
    event.kind == RunEventKind::Status
        && matches!(event.body.as_str(), "done" | "stopped" | "failed")
}

/// Serialize a [`RunEvent`] into an SSE event (event name = kind tag, data =
/// the full event JSON). Serialization is infallible for this shape.
fn sse_event(event: &RunEvent) -> SseEvent {
    use crate::runs::RunEventKind;
    let name = match event.kind {
        RunEventKind::Trace => "trace",
        RunEventKind::Obligation => "obligation",
        RunEventKind::Diff => "diff",
        RunEventKind::Status => "status",
    };
    let data = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
    SseEvent::default().event(name).id(event.seq.to_string()).data(data)
}

// ---------------------------------------------------------------------------
// Agent co-presence handlers (spec: HANDOFF-AGENT-COEDIT-PRESENCE-LAYER.md).
// All are DeviceAuth-gated. Awareness only: nothing here writes file bytes.
//
// The wire `actor` is a free-form stable agent label (e.g. "claude-code",
// "codex"); the registry derives an opaque [`ActorId`] from it via
// `ActorId::from_label`, so two processes that pass the same label share one
// presence identity. A hook that runs once per agent process passes that agent's
// canonical name.
// ---------------------------------------------------------------------------

/// A file range on the wire (line/col span), mirroring
/// [`crate::presence::FileRange`].
#[derive(Debug, Deserialize, Serialize)]
pub struct FileRangeBody {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl From<FileRangeBody> for FileRange {
    fn from(body: FileRangeBody) -> Self {
        FileRange::new(body.start_line, body.start_col, body.end_line, body.end_col)
    }
}

/// Request body for `POST /v1/presence` (announce/update presence).
#[derive(Debug, Deserialize)]
pub struct AnnouncePresenceRequest {
    /// Stable agent label (the registry derives the actor id from it).
    pub actor: String,
    /// The file the agent is present in (workspace-relative; opaque to the registry).
    pub path: String,
    pub line: u32,
    pub col: u32,
    /// Human-facing label (defaults to the actor label when omitted).
    #[serde(default)]
    pub label: Option<String>,
    /// Whether this is a human or an agent (defaults to agent).
    #[serde(default)]
    pub kind: Option<PresenceKind>,
}

/// Response body for `POST /v1/presence`: the stored presence.
#[derive(Debug, Serialize)]
pub struct AnnouncePresenceResponse {
    pub presence: AgentPresence,
}

/// Response body for `GET /v1/presence`: every current presence + footprint.
#[derive(Debug, Serialize)]
pub struct PresenceListResponse {
    pub presences: Vec<AgentPresence>,
    pub footprints: Vec<CodeEditFootprint>,
}

/// Request body for `POST /v1/presence/footprint` (set a pending-edit footprint).
#[derive(Debug, Deserialize)]
pub struct SetFootprintRequest {
    pub actor: String,
    pub path: String,
    pub range: FileRangeBody,
    #[serde(default)]
    pub summary: Option<String>,
}

/// Response body for `POST /v1/presence/footprint`: the stored footprint.
#[derive(Debug, Serialize)]
pub struct SetFootprintResponse {
    pub footprint: CodeEditFootprint,
}

/// Request body for `DELETE /v1/presence/footprint` (clear a footprint).
#[derive(Debug, Deserialize)]
pub struct ClearFootprintRequest {
    pub actor: String,
    pub path: String,
}

/// Response body for `DELETE /v1/presence/footprint`.
#[derive(Debug, Serialize)]
pub struct ClearFootprintResponse {
    /// Whether a footprint existed and was removed (idempotent: `false` is fine).
    pub cleared: bool,
}

/// Request body for `POST /v1/presence/would-overlap` (the key query).
#[derive(Debug, Deserialize)]
pub struct WouldOverlapRequest {
    /// The caller's actor label; its own footprint on `path` is excluded.
    pub actor: String,
    pub path: String,
    /// The range the caller intends to edit.
    pub intended: FileRangeBody,
}

/// Response body for `POST /v1/presence/would-overlap`: the peers' overlapping
/// footprints (empty when the intended range is clear).
#[derive(Debug, Serialize)]
pub struct WouldOverlapResponse {
    /// Peers' pending-edit footprints that overlap the intended range (excludes
    /// the caller's own). Non-empty = warn/serialize before writing.
    pub overlaps: Vec<CodeEditFootprint>,
}

/// If `value` is blank, build the `400` response for a missing required field;
/// otherwise `None`. Returning `Option<Response>` (rather than `Result<(),
/// Response>`) keeps the helper free of a large-`Err` `Result` (clippy
/// `result_large_err`): a handler does `if let Some(rejection) = require_field(..)
/// { return Err(rejection); }`.
fn require_field(value: &str, field: &'static str) -> Option<Response> {
    if value.trim().is_empty() {
        Some(
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorBody {
                    error: format!("{field} must not be empty"),
                }),
            )
                .into_response(),
        )
    } else {
        None
    }
}

/// `POST /v1/presence`: announce (or update) an agent's presence. Authenticated.
async fn announce_presence_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<AnnouncePresenceRequest>,
) -> std::result::Result<Json<AnnouncePresenceResponse>, Response> {
    if let Some(rejection) = require_field(&request.actor, "actor") {
        return Err(rejection);
    }
    if let Some(rejection) = require_field(&request.path, "path") {
        return Err(rejection);
    }
    let label = request
        .label
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| request.actor.clone());
    let presence = state.presence.announce(PresenceAnnouncement {
        actor: ActorId::from_label(&request.actor),
        path: request.path,
        line: request.line,
        col: request.col,
        label,
        kind: request.kind.unwrap_or(PresenceKind::Agent),
    });
    Ok(Json(AnnouncePresenceResponse { presence }))
}

/// `GET /v1/presence`: list every current presence and pending-edit footprint.
/// Authenticated. This is the ambient "who is here / what are they about to edit"
/// read a hook (or a UI) uses without filesystem recon.
async fn list_presence_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
) -> Json<PresenceListResponse> {
    Json(PresenceListResponse {
        presences: state.presence.list_presences(),
        footprints: state.presence.list_footprints(),
    })
}

/// `POST /v1/presence/footprint`: set (or replace) an agent's pending-edit
/// footprint on a path. Authenticated. PreToolUse(Edit) calls this with the
/// intended range before writing.
async fn set_footprint_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<SetFootprintRequest>,
) -> std::result::Result<Json<SetFootprintResponse>, Response> {
    if let Some(rejection) = require_field(&request.actor, "actor") {
        return Err(rejection);
    }
    if let Some(rejection) = require_field(&request.path, "path") {
        return Err(rejection);
    }
    let footprint = state.presence.set_footprint(FootprintAnnouncement {
        actor: ActorId::from_label(&request.actor),
        path: request.path,
        range: request.range.into(),
        summary: request.summary,
    });
    Ok(Json(SetFootprintResponse { footprint }))
}

/// `DELETE /v1/presence/footprint`: clear an agent's pending-edit footprint on a
/// path. Authenticated. PostToolUse(Edit) calls this once the write is done.
async fn clear_footprint_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<ClearFootprintRequest>,
) -> std::result::Result<Json<ClearFootprintResponse>, Response> {
    if let Some(rejection) = require_field(&request.actor, "actor") {
        return Err(rejection);
    }
    if let Some(rejection) = require_field(&request.path, "path") {
        return Err(rejection);
    }
    let cleared = state
        .presence
        .clear_footprint(ActorId::from_label(&request.actor), &request.path);
    Ok(Json(ClearFootprintResponse { cleared }))
}

/// `POST /v1/presence/would-overlap`: the key query. Authenticated. Returns the
/// peers' pending-edit footprints that overlap the caller's intended range on the
/// same path, excluding the caller's own. Non-empty means warn/serialize.
async fn would_overlap_handler(
    _auth: DeviceAuth,
    State(state): State<ControlState>,
    Json(request): Json<WouldOverlapRequest>,
) -> std::result::Result<Json<WouldOverlapResponse>, Response> {
    if let Some(rejection) = require_field(&request.actor, "actor") {
        return Err(rejection);
    }
    if let Some(rejection) = require_field(&request.path, "path") {
        return Err(rejection);
    }
    let overlaps = state.presence.would_overlap(
        ActorId::from_label(&request.actor),
        &request.path,
        &request.intended.into(),
    );
    Ok(Json(WouldOverlapResponse { overlaps }))
}

/// Generate a one-time pairing code: 8 bytes of CSPRNG entropy as 16 lowercase
/// hex chars. Surface this to the user at startup so a device can pair once.
pub fn generate_pairing_code() -> Result<String> {
    crate::pairing::random_token_hex(8)
}

/// A running control endpoint on a loopback socket. Hold this to keep serving;
/// the bound address is exposed via [`local_addr`](ControlServer::local_addr) so
/// a caller (or a test) can connect, and [`shutdown`](ControlServer::shutdown)
/// stops the server.
pub struct ControlServer {
    local_addr: SocketAddr,
    shutdown: tokio::sync::oneshot::Sender<()>,
    task: tokio::task::JoinHandle<std::result::Result<(), std::io::Error>>,
}

impl ControlServer {
    /// The actual bound loopback address (the port is resolved if `0` was passed).
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Signal the server to stop and await its task. Returns any serve error.
    pub async fn shutdown(self) -> Result<()> {
        let _ = self.shutdown.send(());
        match self.task.await {
            Ok(serve_result) => serve_result.map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("control endpoint serve error: {error}").into()
            }),
            Err(join_error) => Err(format!("control endpoint task panicked: {join_error}").into()),
        }
    }
}

/// Bind the control endpoint to `127.0.0.1:<port>` (pass `0` for an ephemeral
/// port) and serve it on a background task, returning a [`ControlServer`] handle.
///
/// Loopback is enforced: the bind address is always [`LOOPBACK`], never a
/// routable interface. The server runs until [`ControlServer::shutdown`] is
/// called (or the handle is dropped, which fires the shutdown signal).
pub async fn serve(state: ControlState, port: u16) -> Result<ControlServer> {
    let addr = SocketAddr::new(LOOPBACK, port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("control endpoint bind {addr}: {error}").into()
        })?;
    let local_addr = listener
        .local_addr()
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("control endpoint local_addr: {error}").into()
        })?;
    let app = build_router(state);
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await
    });
    Ok(ControlServer {
        local_addr,
        shutdown: shutdown_tx,
        task,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_state() -> (tempfile::TempDir, ControlState) {
        let dir = tempfile::tempdir().unwrap();
        let pairing = DevicePairing::open(dir.path()).unwrap();
        let runs = RunRegistry::new(std::sync::Arc::new(crate::runs::MockExecutor::new()));
        let state = ControlState::new(pairing, "test-pairing-code", runs);
        (dir, state)
    }

    async fn body_json(response: Response) -> serde_json::Value {
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    /// Pair a device through the router and return its bearer token.
    async fn pair_via_router(router: &Router, code: &str, label: &str) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method("POST")
            .uri("/pair")
            .header(PAIRING_CODE_HEADER, code)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&serde_json::json!({ "label": label })).unwrap()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    #[tokio::test]
    async fn healthz_is_unauthenticated() {
        let (_dir, state) = test_state();
        let router = build_router(state);
        let response = router
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn pair_then_valid_token_authorizes_v1_status() {
        let (_dir, state) = test_state();
        let router = build_router(state);

        let (status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        assert_eq!(status, StatusCode::OK);
        let token = body["token"].as_str().unwrap().to_string();

        let request = Request::builder()
            .uri("/v1/status")
            .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK, "a valid token authorizes /v1/status");
        let status_body = body_json(response).await;
        assert_eq!(status_body["status"], "ok");
        assert_eq!(status_body["paired_devices"], 1);
    }

    #[tokio::test]
    async fn missing_and_garbage_tokens_are_401() {
        let (_dir, state) = test_state();
        let router = build_router(state);

        // No authorization header at all.
        let no_auth = router
            .clone()
            .oneshot(Request::builder().uri("/v1/status").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(no_auth.status(), StatusCode::UNAUTHORIZED, "missing token is 401");

        // A garbage bearer token.
        let garbage = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(axum::http::header::AUTHORIZATION, "Bearer not-a-real-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED, "garbage token is 401");

        // Wrong scheme.
        let wrong_scheme = router
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(axum::http::header::AUTHORIZATION, "Basic abc123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_scheme.status(), StatusCode::UNAUTHORIZED, "non-bearer scheme is 401");
    }

    #[tokio::test]
    async fn pair_requires_the_pairing_code() {
        let (_dir, state) = test_state();
        let router = build_router(state);

        // Wrong code is rejected.
        let (status, _body) = pair_via_router(&router, "wrong-code", "Phone").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "pairing without the code is forbidden");

        // Missing code header is rejected too.
        let request = Request::builder()
            .method("POST")
            .uri("/pair")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&serde_json::json!({ "label": "Phone" })).unwrap()))
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "pairing with no code is forbidden");
    }

    #[tokio::test]
    async fn revoked_device_is_rejected_on_v1() {
        let (_dir, state) = test_state();
        let router = build_router(state);

        let (_status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        let token = body["token"].as_str().unwrap().to_string();
        let device_id = body["device"]["device_id"].as_str().unwrap().to_string();

        // Authorized before revocation.
        let before = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(before.status(), StatusCode::OK);

        // Revoke via the authenticated route (using the device's own token).
        let revoke = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/devices/revoke")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "device_id": device_id })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoke.status(), StatusCode::OK);
        assert_eq!(body_json(revoke).await["revoked"], true);

        // The revoked token no longer authorizes.
        let after = router
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::UNAUTHORIZED, "a revoked device's token is 401");
    }

    #[tokio::test]
    async fn serve_binds_loopback_and_serves_over_a_real_socket() {
        let (_dir, state) = test_state();
        // Keep a registry handle to verify the pairing landed durably.
        let pairing = state.pairing().clone();
        let server = serve(state, 0).await.expect("bind loopback");
        let addr = server.local_addr();
        assert!(addr.ip().is_loopback(), "the control endpoint must bind loopback only");

        let client = reqwest::Client::new();
        // Unauthenticated healthz works.
        let health = client
            .get(format!("http://{addr}/healthz"))
            .send()
            .await
            .unwrap();
        assert_eq!(health.status(), reqwest::StatusCode::OK);

        // Pair over the wire.
        let pair = client
            .post(format!("http://{addr}/pair"))
            .header(PAIRING_CODE_HEADER, "test-pairing-code")
            .json(&serde_json::json!({ "label": "Wire Phone" }))
            .send()
            .await
            .unwrap();
        assert_eq!(pair.status(), reqwest::StatusCode::OK);
        let token = pair.json::<serde_json::Value>().await.unwrap()["token"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(pairing.device_count(), 1, "pairing persisted to the registry");

        // /v1/status without a token is 401 over the wire.
        let unauth = client
            .get(format!("http://{addr}/v1/status"))
            .send()
            .await
            .unwrap();
        assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

        // With the token it authorizes.
        let authed = client
            .get(format!("http://{addr}/v1/status"))
            .bearer_auth(&token)
            .send()
            .await
            .unwrap();
        assert_eq!(authed.status(), reqwest::StatusCode::OK);

        server.shutdown().await.expect("clean shutdown");
    }

    // ----------------------------------------------------------------------
    // Agent co-presence routes (DeviceAuth-gated, in-process via oneshot).
    // ----------------------------------------------------------------------

    /// A bearer-authed JSON request against the router; returns (status, body).
    async fn authed_json(
        router: &Router,
        method: &str,
        uri: &str,
        token: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(axum::http::header::AUTHORIZATION, format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let response = router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        (status, body_json(response).await)
    }

    #[tokio::test]
    async fn presence_announce_set_and_would_overlap_over_http() {
        let (_dir, state) = test_state();
        let router = build_router(state);
        let (_status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        let token = body["token"].as_str().unwrap().to_string();

        // Two agents announce presence on the same file.
        let (s1, _) = authed_json(
            &router,
            "POST",
            "/v1/presence",
            &token,
            serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs", "line": 10, "col": 0 }),
        )
        .await;
        assert_eq!(s1, StatusCode::OK);
        authed_json(
            &router,
            "POST",
            "/v1/presence",
            &token,
            serde_json::json!({ "actor": "codex", "path": "src/lib.rs", "line": 40, "col": 0 }),
        )
        .await;

        // The presence list reflects both announces.
        let (sl, list) = authed_json(&router, "GET", "/v1/presence", &token, serde_json::json!({}))
            .await;
        assert_eq!(sl, StatusCode::OK);
        assert_eq!(
            list["presences"].as_array().unwrap().len(),
            2,
            "presence list reflects both announces"
        );

        // Both set overlapping pending-edit footprints.
        authed_json(
            &router,
            "POST",
            "/v1/presence/footprint",
            &token,
            serde_json::json!({
                "actor": "claude-code",
                "path": "src/lib.rs",
                "range": { "start_line": 10, "start_col": 0, "end_line": 20, "end_col": 0 },
                "summary": "refactor a"
            }),
        )
        .await;
        authed_json(
            &router,
            "POST",
            "/v1/presence/footprint",
            &token,
            serde_json::json!({
                "actor": "codex",
                "path": "src/lib.rs",
                "range": { "start_line": 15, "start_col": 0, "end_line": 25, "end_col": 0 },
                "summary": "rename b"
            }),
        )
        .await;

        // Claude queries would-overlap for its intended range: it sees CODEX's
        // footprint and NOT its own.
        let (so, overlap) = authed_json(
            &router,
            "POST",
            "/v1/presence/would-overlap",
            &token,
            serde_json::json!({
                "actor": "claude-code",
                "path": "src/lib.rs",
                "intended": { "start_line": 10, "start_col": 0, "end_line": 20, "end_col": 0 }
            }),
        )
        .await;
        assert_eq!(so, StatusCode::OK);
        let overlaps = overlap["overlaps"].as_array().unwrap();
        assert_eq!(overlaps.len(), 1, "exactly the peer's footprint is flagged");
        assert_eq!(overlaps[0]["summary"], "rename b");

        // A non-overlapping intended range comes back empty.
        let (_sn, none) = authed_json(
            &router,
            "POST",
            "/v1/presence/would-overlap",
            &token,
            serde_json::json!({
                "actor": "claude-code",
                "path": "src/lib.rs",
                "intended": { "start_line": 100, "start_col": 0, "end_line": 110, "end_col": 0 }
            }),
        )
        .await;
        assert!(
            none["overlaps"].as_array().unwrap().is_empty(),
            "a clear intended range flags nothing"
        );

        // Clearing the peer's footprint removes it from the overlap query.
        let (sc, cleared) = authed_json(
            &router,
            "DELETE",
            "/v1/presence/footprint",
            &token,
            serde_json::json!({ "actor": "codex", "path": "src/lib.rs" }),
        )
        .await;
        assert_eq!(sc, StatusCode::OK);
        assert_eq!(cleared["cleared"], true);
        let (_s2, after) = authed_json(
            &router,
            "POST",
            "/v1/presence/would-overlap",
            &token,
            serde_json::json!({
                "actor": "claude-code",
                "path": "src/lib.rs",
                "intended": { "start_line": 10, "start_col": 0, "end_line": 20, "end_col": 0 }
            }),
        )
        .await;
        assert!(
            after["overlaps"].as_array().unwrap().is_empty(),
            "a cleared footprint no longer overlaps"
        );
    }

    #[tokio::test]
    async fn presence_routes_require_a_device_token() {
        let (_dir, state) = test_state();
        let router = build_router(state);

        // Each /v1/presence* route is 401 without a token.
        let cases = [
            ("POST", "/v1/presence"),
            ("GET", "/v1/presence"),
            ("POST", "/v1/presence/footprint"),
            ("DELETE", "/v1/presence/footprint"),
            ("POST", "/v1/presence/would-overlap"),
        ];
        for (method, uri) in cases {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .header("content-type", "application/json")
                        .body(Body::from("{}"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri} must be 401 without a device token"
            );
        }

        // A garbage bearer token is also 401 (not just a missing header).
        let garbage = router
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/v1/presence")
                    .header(axum::http::header::AUTHORIZATION, "Bearer not-a-real-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(garbage.status(), StatusCode::UNAUTHORIZED, "garbage token is 401");
    }

    #[tokio::test]
    async fn presence_announce_rejects_blank_fields() {
        let (_dir, state) = test_state();
        let router = build_router(state);
        let (_status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        let token = body["token"].as_str().unwrap().to_string();

        let (status, _) = authed_json(
            &router,
            "POST",
            "/v1/presence",
            &token,
            serde_json::json!({ "actor": "", "path": "src/lib.rs", "line": 1, "col": 0 }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a blank actor is 400");
    }

    // ----------------------------------------------------------------------
    // Local-first commonplace DATA routes (DeviceAuth-gated, read-only).
    // ----------------------------------------------------------------------

    /// Control state with a real durable sink wired (a tempdir-backed sidecar), so
    /// the data routes have a live commonplace graph to read.
    fn test_state_with_data() -> (tempfile::TempDir, ControlState, SharedSink) {
        let dir = tempfile::tempdir().unwrap();
        let pairing = DevicePairing::open(dir.path()).unwrap();
        let runs = RunRegistry::new(std::sync::Arc::new(crate::runs::MockExecutor::new()));
        // A WatchConfig whose sidecar is the tempdir itself (the sink creates
        // graph/ + blobs/ under it).
        let config = crate::WatchConfig {
            root: dir.path().to_path_buf(),
            sidecar_dir: dir.path().to_path_buf(),
            debounce: std::time::Duration::from_millis(200),
            extra_ignores: Vec::new(),
        };
        let data = SharedSink::open(&config).unwrap();
        let state = ControlState::new(pairing, "test-pairing-code", runs).with_data(data.clone());
        (dir, state, data)
    }

    /// Seed one commonplace item into the shared sink via the ingest pipeline, so
    /// it is vector-searchable (the pipeline writes the item embedding). Returns
    /// the new item's id.
    fn seed_item(data: &SharedSink, title: &str, text: &str) -> String {
        use commonplace::{IngestInput, IngestPipeline};
        let mut sink = data.lock();
        let receipt = IngestPipeline::default()
            .ingest(
                sink.commonplace_mut(),
                IngestInput::document(title, text),
            )
            .unwrap();
        receipt.item.id
    }

    #[tokio::test]
    async fn data_routes_list_get_and_search_over_seeded_items() {
        let (_dir, state, data) = test_state_with_data();
        let id = seed_item(&data, "Roadmap", "ship the local-first connection path");
        let router = build_router(state);
        let (_status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        let token = body["token"].as_str().unwrap().to_string();

        // GET /v1/items returns the seeded item.
        let (sl, list) = authed_json(&router, "GET", "/v1/items", &token, serde_json::json!({}))
            .await;
        assert_eq!(sl, StatusCode::OK);
        let items = list["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "the seeded item is listed");
        assert_eq!(items[0]["id"], id);
        assert_eq!(items[0]["title"], "Roadmap");

        // GET /v1/items/{id} returns it.
        let (sd, detail) = authed_json(
            &router,
            "GET",
            &format!("/v1/items/{id}"),
            &token,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(sd, StatusCode::OK);
        assert_eq!(detail["item"]["id"], id);

        // An unknown id is 404.
        let (snf, _) = authed_json(
            &router,
            "GET",
            "/v1/items/item:does-not-exist",
            &token,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(snf, StatusCode::NOT_FOUND);

        // GET /v1/search finds the item (real commonplace vector search).
        let (ss, search) = authed_json(
            &router,
            "GET",
            "/v1/search?q=local-first%20connection&k=5",
            &token,
            serde_json::json!({}),
        )
        .await;
        assert_eq!(ss, StatusCode::OK);
        let hits = search["hits"].as_array().unwrap();
        assert!(
            hits.iter().any(|hit| hit["item"]["id"] == serde_json::json!(id)),
            "search surfaces the seeded item: {search}"
        );

        // A blank q is 400.
        let (sb, _) = authed_json(&router, "GET", "/v1/search?q=%20", &token, serde_json::json!({}))
            .await;
        assert_eq!(sb, StatusCode::BAD_REQUEST, "a blank query is 400");
    }

    #[tokio::test]
    async fn data_routes_require_a_device_token() {
        let (_dir, state, data) = test_state_with_data();
        seed_item(&data, "Secret", "should not be readable without a token");
        let router = build_router(state);

        for (method, uri) in [
            ("GET", "/v1/items"),
            ("GET", "/v1/items/whatever"),
            ("GET", "/v1/collections"),
            ("GET", "/v1/search?q=secret"),
        ] {
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri} must be 401 without a device token"
            );
        }
    }

    #[tokio::test]
    async fn data_routes_are_503_without_a_sink() {
        // Default state (no data wired): the routes are reachable + authenticated
        // but report the backend is unavailable rather than 404/panicking.
        let (_dir, state) = test_state();
        let router = build_router(state);
        let (_status, body) = pair_via_router(&router, "test-pairing-code", "Phone").await;
        let token = body["token"].as_str().unwrap().to_string();
        let (status, _) = authed_json(&router, "GET", "/v1/items", &token, serde_json::json!({}))
            .await;
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "data routes are 503 when no commonplace backend is wired"
        );
    }
}
