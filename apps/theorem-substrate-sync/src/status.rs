use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::State;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, RwLock};

use crate::Result;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    Connected,
    Disconnected,
    TokenInvalid,
}

impl ConnectionState {
    pub fn as_connected_bool(&self) -> bool {
        matches!(self, Self::Connected)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamState {
    Connected,
    Disconnected,
    Degraded,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    Ready,
    Blocked,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SyncStatus {
    pub connected: bool,
    pub connection: ConnectionState,
    pub sync_enabled: bool,
    pub tenant: String,
    pub last_round: Option<String>,
    pub last_event: Option<String>,
    pub stream: StreamState,
    pub outbox: OutboxState,
    pub current_interval_ms: u64,
    pub stream_retry_after_ms: Option<u64>,
    pub updated_at_unix_ms: u128,
    pub warnings: Vec<String>,
}

impl SyncStatus {
    pub fn new(sync_enabled: bool, tenant: impl Into<String>, idle_interval_ms: u64) -> Self {
        Self {
            connected: false,
            connection: ConnectionState::Disconnected,
            sync_enabled,
            tenant: tenant.into(),
            last_round: None,
            last_event: None,
            stream: StreamState::Disconnected,
            outbox: OutboxState::Ready,
            current_interval_ms: idle_interval_ms,
            stream_retry_after_ms: None,
            updated_at_unix_ms: now_ms(),
            warnings: Vec::new(),
        }
    }

    pub fn set_connection(&mut self, connection: ConnectionState) {
        self.connected = connection.as_connected_bool();
        self.connection = connection;
        self.touch();
    }

    pub fn touch(&mut self) {
        self.updated_at_unix_ms = now_ms();
    }
}

#[derive(Clone)]
pub struct StatusHandle {
    state: Arc<RwLock<SyncStatus>>,
}

impl StatusHandle {
    pub fn new(status: SyncStatus) -> Self {
        Self {
            state: Arc::new(RwLock::new(status)),
        }
    }

    pub async fn get(&self) -> SyncStatus {
        self.state.read().await.clone()
    }

    pub async fn update(&self, update: impl FnOnce(&mut SyncStatus)) {
        let mut status = self.state.write().await;
        update(&mut status);
        status.touch();
    }
}

#[derive(Clone)]
struct AppState {
    status: StatusHandle,
    trigger_tx: mpsc::UnboundedSender<()>,
}

pub async fn serve_status(
    addr: SocketAddr,
    status: StatusHandle,
    trigger_tx: mpsc::UnboundedSender<()>,
) -> Result<()> {
    let app = Router::new()
        .route("/status", get(status_route))
        .route("/trigger", post(trigger_route))
        .with_state(AppState { status, trigger_tx });
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn status_route(State(state): State<AppState>) -> Json<SyncStatus> {
    Json(state.status.get().await)
}

async fn trigger_route(State(state): State<AppState>) -> Json<serde_json::Value> {
    let accepted = state.trigger_tx.send(()).is_ok();
    Json(serde_json::json!({ "ok": accepted, "triggered": accepted }))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}
