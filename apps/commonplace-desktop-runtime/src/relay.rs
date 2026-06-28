//! Outbound relay tunnel for NAT traversal (phone-control handoff Part B
//! deliverable 2, path 2: relay).
//!
//! The control endpoint binds LOOPBACK and we deliberately open NO inbound port.
//! For a phone that is NOT on the same LAN (cellular, a different network) there
//! is no route to the instance. The relay solves this WITHOUT an inbound
//! port-forward: the *instance* makes an OUTBOUND websocket connection to a relay
//! server; the phone talks to the relay; the relay forwards each request down the
//! already-open outbound socket to the instance, which proxies it to its own
//! loopback control endpoint and streams the response back up.
//!
//! ```text
//!   phone ──HTTPS──▶ relay ──(existing outbound ws)──▶ instance ──loopback──▶ control endpoint
//!                         ◀── response frames ──────────────────────────────┘
//! ```
//!
//! # The relay is a dumb pipe; device-auth is end-to-end
//!
//! The relay NEVER sees or needs a device token. A tunneled [`TunnelRequest`]
//! carries the phone's headers verbatim -- including `authorization: Bearer
//! <device-token>` -- and the instance REPLAYS them onto the loopback control
//! endpoint. So `DeviceAuth` ([`crate::control`]) is enforced at the loopback
//! endpoint exactly as it would be over a direct connection: a request with no
//! token still gets `401`, *through the tunnel*. The relay only authenticates the
//! INSTANCE to itself (an instance-to-relay credential, [`RelayCredential`]); it
//! does not (and must not) understand device tokens.
//!
//! # What is built vs degraded here
//!
//! * [`RelayClient`] (built, tested): connects out over a websocket, registers
//!   with the instance credential, then runs the tunnel loop -- read a request
//!   frame, proxy it to the loopback control endpoint, write the response frame --
//!   with reconnect + capped exponential backoff.
//! * [`MockRelay`] (built, tested): an in-process loopback-websocket relay that
//!   speaks the same protocol, so the whole tunnel is cargo-testable end-to-end
//!   (a request injected at the relay reaches the loopback control endpoint and
//!   the response returns) WITHOUT a deployed relay.
//! * A LIVE deployed relay + a real cross-network NAT traversal is OUT OF SCOPE
//!   for the cargo oracle (no relay to point at); it is flagged as a follow-up.
//!   The wire protocol and the client tunnel loop are fully exercised against the
//!   mock.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::Result;

/// The instance-to-relay credential: a shared secret the instance presents when
/// it registers, so a relay only tunnels to instances it trusts. This is NOT a
/// device token (the relay never sees those); it authenticates the INSTANCE to
/// the relay. Treated as opaque bytes; never logged.
#[derive(Clone)]
pub struct RelayCredential(String);

impl RelayCredential {
    /// Wrap a credential string.
    pub fn new(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    /// The credential value (for the register frame). Crate-internal so it is not
    /// casually logged by callers.
    fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for RelayCredential {
    /// Never print the secret.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RelayCredential(<redacted>)")
    }
}

// ---------------------------------------------------------------------------
// Wire protocol (pure, serde, unit-testable without any socket).
// ---------------------------------------------------------------------------

/// A frame the INSTANCE sends to the relay: first a `register`, then a `response`
/// per tunneled request.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InstanceFrame {
    /// Sent once on connect: authenticate the instance to the relay and announce
    /// the non-secret instance id (so the relay can route the phone to it).
    Register {
        instance_id: String,
        /// The instance-to-relay credential (NOT a device token).
        credential: String,
    },
    /// The proxied response for a tunneled request.
    Response(TunnelResponse),
}

/// A frame the RELAY sends to the instance: a registration verdict, then a
/// `request` per phone call to tunnel.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelayFrame {
    /// The instance registered successfully; the tunnel is open.
    Registered,
    /// Registration was rejected (bad credential); the instance should not retry
    /// with the same credential.
    RegisterRejected { reason: String },
    /// A request from a phone to tunnel to the loopback control endpoint.
    Request(TunnelRequest),
}

/// A request tunneled from a phone (via the relay) to the instance's loopback
/// control endpoint. The headers are carried VERBATIM, which is how the device
/// token rides through end-to-end.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelRequest {
    /// Correlates the response to this request over the multiplexed socket.
    pub request_id: String,
    /// HTTP method (e.g. "GET", "POST").
    pub method: String,
    /// Request path + query (e.g. "/v1/status").
    pub path: String,
    /// Request headers verbatim (lower-cased keys). Carries `authorization` so
    /// device-auth is enforced at the loopback endpoint, not the relay.
    pub headers: BTreeMap<String, String>,
    /// Base64-encoded request body (empty string for no body). Base64 keeps the
    /// JSON frame text-safe for binary bodies.
    #[serde(default)]
    pub body_b64: String,
}

impl TunnelRequest {
    /// Decode the request body bytes from base64.
    pub fn body_bytes(&self) -> Result<Vec<u8>> {
        decode_b64(&self.body_b64)
    }
}

/// The proxied response for a [`TunnelRequest`], sent from the instance back to
/// the relay (and on to the phone).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelResponse {
    /// Echoes the request's `request_id`.
    pub request_id: String,
    /// HTTP status code from the loopback control endpoint (e.g. 200, 401, 404).
    pub status: u16,
    /// Response headers (lower-cased keys).
    pub headers: BTreeMap<String, String>,
    /// Base64-encoded response body.
    #[serde(default)]
    pub body_b64: String,
}

impl TunnelResponse {
    /// Build a response from a status, headers, and raw body bytes.
    pub fn new(
        request_id: impl Into<String>,
        status: u16,
        headers: BTreeMap<String, String>,
        body: &[u8],
    ) -> Self {
        Self {
            request_id: request_id.into(),
            status,
            headers,
            body_b64: encode_b64(body),
        }
    }

    /// Decode the response body bytes from base64.
    pub fn body_bytes(&self) -> Result<Vec<u8>> {
        decode_b64(&self.body_b64)
    }
}

fn encode_b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode_b64(s: &str) -> Result<Vec<u8>> {
    if s.is_empty() {
        return Ok(Vec::new());
    }
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("decode tunnel body base64: {error}").into()
        })
}

// ---------------------------------------------------------------------------
// RelayClient: the instance's outbound tunnel.
// ---------------------------------------------------------------------------

/// Configuration for the outbound [`RelayClient`].
#[derive(Clone, Debug)]
pub struct RelayClientConfig {
    /// The relay's websocket URL (e.g. `wss://relay.example/instance` in
    /// production, or `ws://127.0.0.1:<port>` against the [`MockRelay`]).
    pub relay_url: String,
    /// The non-secret instance id announced on register (matches the mDNS TXT id
    /// so a phone sees one identity across both paths).
    pub instance_id: String,
    /// The base URL of the LOCAL loopback control endpoint to proxy to
    /// (e.g. `http://127.0.0.1:<control-port>`).
    pub control_base_url: String,
    /// Initial reconnect backoff (doubles each failure up to `max_backoff`).
    pub initial_backoff: Duration,
    /// Cap on the reconnect backoff.
    pub max_backoff: Duration,
}

impl RelayClientConfig {
    /// A config with sane backoff defaults.
    pub fn new(
        relay_url: impl Into<String>,
        instance_id: impl Into<String>,
        control_base_url: impl Into<String>,
    ) -> Self {
        Self {
            relay_url: relay_url.into(),
            instance_id: instance_id.into(),
            control_base_url: control_base_url.into(),
            initial_backoff: Duration::from_millis(500),
            max_backoff: Duration::from_secs(30),
        }
    }
}

/// The websocket stream type `connect_async` yields (TLS-capable wrapper over a
/// `TcpStream`; for `ws://` it is the plain variant).
type ClientWs = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The instance's outbound relay tunnel. [`run`](RelayClient::run) connects to
/// the relay, registers, and serves tunneled requests until cancelled,
/// reconnecting with backoff on any drop. A reqwest client proxies each tunneled
/// request to the loopback control endpoint, so device-auth is enforced there.
pub struct RelayClient {
    config: RelayClientConfig,
    credential: RelayCredential,
    http: reqwest::Client,
}

impl RelayClient {
    /// Build a client over a config and instance-to-relay credential.
    pub fn new(config: RelayClientConfig, credential: RelayCredential) -> Self {
        Self {
            config,
            credential,
            // A dedicated client for the loopback proxy. No redirects: the control
            // endpoint's own status (incl. 401/404) must be tunneled back as-is.
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap_or_default(),
        }
    }

    /// Run the tunnel until `cancel` is tripped. On each iteration it connects,
    /// registers, and serves requests; any disconnect (or a connect failure)
    /// triggers a capped exponential backoff and a reconnect, unless the relay
    /// explicitly REJECTED the credential (a config error, not a transient drop),
    /// in which case it stops.
    ///
    /// This is the production entry point the desktop spawns on a task. It is
    /// `cancel`-cooperative: the cancel flag is checked between sessions and the
    /// per-session loop selects on it, so a shutdown returns promptly.
    pub async fn run(&self, cancel: Arc<tokio::sync::Notify>) -> Result<()> {
        let mut backoff = self.config.initial_backoff;
        loop {
            match self.run_one_session(&cancel).await {
                Ok(SessionEnd::Cancelled) => return Ok(()),
                Ok(SessionEnd::Disconnected) => {
                    // Transient: back off and reconnect (unless cancelled while
                    // waiting).
                    if wait_or_cancel(backoff, &cancel).await {
                        return Ok(());
                    }
                    backoff = (backoff * 2).min(self.config.max_backoff);
                }
                Ok(SessionEnd::Rejected) => {
                    // The relay refused our credential: do not hammer it with the
                    // same bad credential. Surface as an error so the operator
                    // fixes the credential.
                    return Err("relay rejected the instance credential".into());
                }
                Err(error) => {
                    // Connect/handshake failure: treat like a disconnect (back off
                    // and retry), but log the cause.
                    eprintln!("commonplace-desktop-runtime: relay session error: {error}");
                    if wait_or_cancel(backoff, &cancel).await {
                        return Ok(());
                    }
                    backoff = (backoff * 2).min(self.config.max_backoff);
                }
            }
        }
    }

    /// One connect -> register -> serve cycle. Returns how the session ended so
    /// [`run`](Self::run) can decide whether to reconnect.
    async fn run_one_session(&self, cancel: &Arc<tokio::sync::Notify>) -> Result<SessionEnd> {
        let (ws, _response) = tokio_tungstenite::connect_async(&self.config.relay_url)
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("connect to relay {}: {error}", self.config.relay_url).into()
            })?;
        let mut ws: ClientWs = ws;

        // Register: announce the instance id + credential.
        let register = InstanceFrame::Register {
            instance_id: self.config.instance_id.clone(),
            credential: self.credential.expose().to_string(),
        };
        ws.send(Message::text(to_json(&register)?)).await.map_err(
            |error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("send register frame: {error}").into()
            },
        )?;

        // Serve frames until the socket drops or we are cancelled.
        loop {
            tokio::select! {
                biased;
                _ = cancel.notified() => {
                    // Best-effort graceful close.
                    let _ = ws.close(None).await;
                    return Ok(SessionEnd::Cancelled);
                }
                frame = ws.next() => {
                    match frame {
                        Some(Ok(message)) => {
                            if let Some(end) = self.handle_message(message, &mut ws).await? {
                                return Ok(end);
                            }
                        }
                        // Stream ended or errored: the session is over; reconnect.
                        Some(Err(_)) | None => return Ok(SessionEnd::Disconnected),
                    }
                }
            }
        }
    }

    /// Handle one inbound websocket message. Returns `Some(SessionEnd)` if the
    /// session should end, `None` to keep serving.
    async fn handle_message(
        &self,
        message: Message,
        ws: &mut ClientWs,
    ) -> Result<Option<SessionEnd>> {
        match message {
            Message::Text(text) => {
                let frame: RelayFrame = match from_json(text.as_str()) {
                    Ok(frame) => frame,
                    // A frame we cannot parse is not fatal to the session; skip it.
                    Err(error) => {
                        eprintln!("commonplace-desktop-runtime: bad relay frame: {error}");
                        return Ok(None);
                    }
                };
                self.handle_frame(frame, ws).await
            }
            // The relay closed: end the session (reconnect decides next).
            Message::Close(_) => Ok(Some(SessionEnd::Disconnected)),
            // Respond to pings so an idle tunnel stays open.
            Message::Ping(payload) => {
                let _ = ws.send(Message::Pong(payload)).await;
                Ok(None)
            }
            // Binary / Pong / frame: not part of this text protocol; ignore.
            _ => Ok(None),
        }
    }

    /// Act on a decoded relay frame.
    async fn handle_frame(&self, frame: RelayFrame, ws: &mut ClientWs) -> Result<Option<SessionEnd>> {
        match frame {
            RelayFrame::Registered => Ok(None),
            RelayFrame::RegisterRejected { reason } => {
                eprintln!("commonplace-desktop-runtime: relay register rejected: {reason}");
                Ok(Some(SessionEnd::Rejected))
            }
            RelayFrame::Request(request) => {
                // Proxy to the loopback control endpoint and tunnel the response
                // back. A proxy failure becomes a 502 tunneled response (the phone
                // sees a gateway error, the session keeps serving).
                let response = self.proxy_to_control(&request).await;
                let frame = InstanceFrame::Response(response);
                ws.send(Message::text(to_json(&frame)?)).await.map_err(
                    |error| -> Box<dyn std::error::Error + Send + Sync> {
                        format!("send tunnel response: {error}").into()
                    },
                )?;
                Ok(None)
            }
        }
    }

    /// Replay a tunneled request onto the LOCAL loopback control endpoint and
    /// capture the response. The phone's headers (incl. `authorization`) are sent
    /// verbatim, so `DeviceAuth` is enforced HERE -- a missing/invalid token gets
    /// a real `401` from the control endpoint, which is tunneled back unchanged.
    async fn proxy_to_control(&self, request: &TunnelRequest) -> TunnelResponse {
        match self.try_proxy_to_control(request).await {
            Ok(response) => response,
            Err(error) => {
                // Surface a gateway error to the phone without tearing down the
                // tunnel. Do not leak the internal error detail to the phone.
                eprintln!("commonplace-desktop-runtime: tunnel proxy failed: {error}");
                TunnelResponse::new(
                    request.request_id.clone(),
                    502,
                    BTreeMap::new(),
                    b"{\"error\":\"bad gateway\"}",
                )
            }
        }
    }

    async fn try_proxy_to_control(&self, request: &TunnelRequest) -> Result<TunnelResponse> {
        let url = format!(
            "{}{}",
            self.config.control_base_url.trim_end_matches('/'),
            request.path
        );
        let method = reqwest::Method::from_bytes(request.method.as_bytes())
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("invalid tunneled method {}: {error}", request.method).into()
            })?;
        let mut builder = self.http.request(method, &url);
        // Replay headers verbatim. This is what carries the device token through.
        // `host` is dropped: it would name the relay, not the loopback target, and
        // reqwest sets the correct host for the loopback URL itself.
        for (name, value) in &request.headers {
            if name.eq_ignore_ascii_case("host") {
                continue;
            }
            builder = builder.header(name, value);
        }
        let body = request.body_bytes()?;
        if !body.is_empty() {
            builder = builder.body(body);
        }
        let response = builder.send().await.map_err(
            |error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("proxy request to control endpoint: {error}").into()
            },
        )?;
        let status = response.status().as_u16();
        let mut headers = BTreeMap::new();
        for (name, value) in response.headers() {
            if let Ok(value_str) = value.to_str() {
                headers.insert(name.as_str().to_ascii_lowercase(), value_str.to_string());
            }
        }
        let body: Bytes = response.bytes().await.map_err(
            |error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("read control response body: {error}").into()
            },
        )?;
        Ok(TunnelResponse::new(
            request.request_id.clone(),
            status,
            headers,
            &body,
        ))
    }
}

/// How one relay session ended (drives the reconnect decision).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SessionEnd {
    /// The cancel signal fired; stop for good.
    Cancelled,
    /// The socket dropped / errored; reconnect after backoff.
    Disconnected,
    /// The relay rejected our credential; stop (a config error, not transient).
    Rejected,
}

/// Sleep for `duration` unless `cancel` fires first. Returns `true` if cancelled
/// (the caller should stop), `false` if the sleep elapsed (retry).
async fn wait_or_cancel(duration: Duration, cancel: &Arc<tokio::sync::Notify>) -> bool {
    tokio::select! {
        biased;
        _ = cancel.notified() => true,
        _ = tokio::time::sleep(duration) => false,
    }
}

fn to_json<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
        format!("serialize relay frame: {error}").into()
    })
}

fn from_json<T: serde::de::DeserializeOwned>(text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
        format!("deserialize relay frame: {error}").into()
    })
}

// ---------------------------------------------------------------------------
// MockRelay: an in-process loopback-websocket relay for tests.
// ---------------------------------------------------------------------------

/// A minimal in-process relay over a loopback websocket, implementing the relay
/// side of the tunnel protocol so the whole [`RelayClient`] path is testable
/// WITHOUT a deployed relay. It:
///
/// * binds `127.0.0.1:0` and accepts ONE instance websocket connection,
/// * validates the instance's `register` credential (rejecting a wrong one),
/// * lets a test inject a [`TunnelRequest`] and awaits the matching
///   [`TunnelResponse`] (exactly how a real relay would forward a phone call),
/// * can drop the connection on command to exercise the client's reconnect.
///
/// It is deliberately tiny: enough protocol to prove the tunnel, not a product
/// relay. A real relay also authenticates the PHONE side and multiplexes many
/// instances; that is out of scope for the cargo oracle.
pub struct MockRelay {
    local_addr: std::net::SocketAddr,
    /// The expected instance credential (a registration with any other is
    /// rejected).
    expected_credential: String,
    /// Receives each instance connection as it registers, so a test can drive
    /// requests over it and force a drop.
    connections: tokio::sync::mpsc::UnboundedReceiver<MockRelayConnection>,
    /// Kept to abort the accept loop on drop.
    accept_task: tokio::task::JoinHandle<()>,
}

impl MockRelay {
    /// Start a mock relay on loopback that expects `expected_credential` from
    /// registering instances. Returns once it is bound and accepting.
    pub async fn start(expected_credential: impl Into<String>) -> Result<Self> {
        let expected_credential = expected_credential.into();
        let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("mock relay bind: {error}").into()
            })?;
        let local_addr = listener.local_addr().map_err(
            |error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("mock relay local_addr: {error}").into()
            },
        )?;
        let (conn_tx, conn_rx) = tokio::sync::mpsc::unbounded_channel();
        let creds = expected_credential.clone();
        let accept_task = tokio::spawn(async move {
            loop {
                let (stream, _peer) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => return,
                };
                let creds = creds.clone();
                let conn_tx = conn_tx.clone();
                tokio::spawn(async move {
                    if let Ok(connection) = mock_accept_and_register(stream, &creds).await {
                        // If the receiver is gone the test is over; drop silently.
                        let _ = conn_tx.send(connection);
                    }
                });
            }
        });
        Ok(Self {
            local_addr,
            expected_credential,
            connections: conn_rx,
            accept_task,
        })
    }

    /// The `ws://` URL an instance should connect to.
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.local_addr)
    }

    /// The credential this relay expects (so a test can build a matching client).
    pub fn expected_credential(&self) -> &str {
        &self.expected_credential
    }

    /// Await the next instance connection that registers successfully.
    pub async fn accept_connection(&mut self) -> Option<MockRelayConnection> {
        self.connections.recv().await
    }
}

impl Drop for MockRelay {
    fn drop(&mut self) {
        self.accept_task.abort();
    }
}

/// One registered instance connection at the mock relay. A test drives a tunneled
/// request over it (mimicking a phone call the relay forwards) and reads back the
/// response, or drops it to force the client to reconnect.
pub struct MockRelayConnection {
    ws: WebSocketStream<TcpStream>,
    /// The instance id the connection registered with.
    instance_id: String,
}

impl MockRelayConnection {
    /// The instance id this connection registered with.
    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    /// Forward a request to the instance and await the matching response. This is
    /// the relay's job for a real phone call: send a `request` frame, read frames
    /// until the `response` with the same `request_id` arrives. Pings are answered
    /// to keep the socket alive while waiting.
    pub async fn send_request(&mut self, request: TunnelRequest) -> Result<TunnelResponse> {
        let want = request.request_id.clone();
        self.ws
            .send(Message::text(to_json(&RelayFrame::Request(request))?))
            .await
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("mock relay send request: {error}").into()
            })?;
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(text))) => {
                    let frame: InstanceFrame = from_json(text.as_str())?;
                    if let InstanceFrame::Response(response) = frame {
                        if response.request_id == want {
                            return Ok(response);
                        }
                    }
                    // A register frame or a mismatched response: keep reading.
                }
                Some(Ok(Message::Ping(payload))) => {
                    let _ = self.ws.send(Message::Pong(payload)).await;
                }
                Some(Ok(_)) => {}
                Some(Err(error)) => {
                    return Err(format!("mock relay read error: {error}").into());
                }
                None => return Err("mock relay connection closed before response".into()),
            }
        }
    }

    /// Drop the underlying socket to simulate a relay outage (exercises the
    /// client's reconnect + backoff). Consumes the connection.
    pub async fn drop_connection(mut self) {
        let _ = self.ws.close(None).await;
        // Dropping `self` closes the TCP stream.
    }
}

/// Accept a websocket on `stream`, read the first frame, and validate it is a
/// `register` with the expected credential. On success returns the live
/// connection; on a bad credential it sends `register_rejected` and errors.
async fn mock_accept_and_register(
    stream: TcpStream,
    expected_credential: &str,
) -> Result<MockRelayConnection> {
    let mut ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("mock relay accept handshake: {error}").into()
        })?;

    // The first frame must be a register.
    let first = ws
        .next()
        .await
        .ok_or("mock relay: connection closed before register")?
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("mock relay read register: {error}").into()
        })?;
    let text = first
        .into_text()
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("mock relay register not text: {error}").into()
        })?;
    let frame: InstanceFrame = from_json(text.as_str())?;
    let (instance_id, credential) = match frame {
        InstanceFrame::Register {
            instance_id,
            credential,
        } => (instance_id, credential),
        _ => {
            return Err("mock relay: first frame was not a register".into());
        }
    };

    if credential != expected_credential {
        let _ = ws
            .send(Message::text(to_json(&RelayFrame::RegisterRejected {
                reason: "bad credential".to_string(),
            })?))
            .await;
        return Err("mock relay: instance credential mismatch".into());
    }

    // Accept: ack and hand the live connection to the test.
    ws.send(Message::text(to_json(&RelayFrame::Registered)?))
        .await
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("mock relay send registered ack: {error}").into()
        })?;
    Ok(MockRelayConnection { ws, instance_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tunnel_request_response_roundtrip_through_json() {
        // The wire protocol is pure + serde-stable: a request with headers + a
        // binary body survives a JSON round trip, and the device token rides in
        // the headers verbatim.
        let mut headers = BTreeMap::new();
        headers.insert("authorization".to_string(), "Bearer dev_abc.deadbeef".to_string());
        headers.insert("content-type".to_string(), "application/json".to_string());
        let request = TunnelRequest {
            request_id: "req-1".to_string(),
            method: "POST".to_string(),
            path: "/v1/runs".to_string(),
            headers: headers.clone(),
            body_b64: encode_b64(&[0u8, 159, 146, 150]), // non-UTF8 bytes
        };
        let json = to_json(&RelayFrame::Request(request.clone())).unwrap();
        let decoded: RelayFrame = from_json(&json).unwrap();
        let RelayFrame::Request(decoded) = decoded else {
            panic!("expected a request frame");
        };
        assert_eq!(decoded, request);
        // The Authorization header is preserved verbatim (end-to-end device auth).
        assert_eq!(
            decoded.headers.get("authorization").map(String::as_str),
            Some("Bearer dev_abc.deadbeef")
        );
        // The binary body decodes back exactly.
        assert_eq!(decoded.body_bytes().unwrap(), vec![0u8, 159, 146, 150]);

        let response = TunnelResponse::new("req-1", 200, BTreeMap::new(), b"{\"ok\":true}");
        let json = to_json(&InstanceFrame::Response(response.clone())).unwrap();
        let decoded: InstanceFrame = from_json(&json).unwrap();
        let InstanceFrame::Response(decoded) = decoded else {
            panic!("expected a response frame");
        };
        assert_eq!(decoded, response);
        assert_eq!(decoded.body_bytes().unwrap(), b"{\"ok\":true}".to_vec());
    }

    #[test]
    fn register_frame_serializes_with_credential() {
        let frame = InstanceFrame::Register {
            instance_id: "inst_1".to_string(),
            credential: "secret".to_string(),
        };
        let json = to_json(&frame).unwrap();
        // Tagged enum: the type discriminator is on the wire.
        assert!(json.contains("\"type\":\"register\""));
        assert!(json.contains("\"instance_id\":\"inst_1\""));
        let back: InstanceFrame = from_json(&json).unwrap();
        match back {
            InstanceFrame::Register {
                instance_id,
                credential,
            } => {
                assert_eq!(instance_id, "inst_1");
                assert_eq!(credential, "secret");
            }
            _ => panic!("expected register"),
        }
    }

    #[test]
    fn relay_credential_debug_is_redacted() {
        let cred = RelayCredential::new("super-secret-value");
        let debug = format!("{cred:?}");
        assert!(
            !debug.contains("super-secret-value"),
            "the credential must never appear in Debug output"
        );
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn empty_body_b64_decodes_to_empty() {
        let response = TunnelResponse {
            request_id: "r".to_string(),
            status: 204,
            headers: BTreeMap::new(),
            body_b64: String::new(),
        };
        assert!(response.body_bytes().unwrap().is_empty());
    }
}
