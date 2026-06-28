//! Acceptance tests for phone-control Part B deliverable 2 (connectivity).
//!
//! These prove the OUTBOUND RELAY TUNNEL end-to-end against an in-process
//! [`MockRelay`] (no deployed relay needed): a request injected at the relay is
//! tunnelled down the instance's outbound websocket, proxied to the REAL loopback
//! [`ControlServer`], and the response is tunnelled back. Crucially they prove the
//! device-auth boundary is preserved END-TO-END through the tunnel -- the relay is
//! a dumb pipe that never sees a device token, and an un-authenticated tunnelled
//! request still gets a `401` from the loopback control endpoint.
//!
//! What is validated here vs. degraded:
//! * VALIDATED: the tunnel protocol + the `RelayClient` loop (connect, register,
//!   proxy, respond, reconnect) against the mock relay, with device-auth enforced
//!   end-to-end at the loopback control endpoint.
//! * DEGRADED / out of scope for the cargo oracle: a LIVE deployed relay and a
//!   real cross-network NAT traversal (there is no relay to point at in a test).
//!   The mock is wire-compatible with the documented protocol, so a real relay is
//!   a deploy-and-point-`relay_url` follow-up, not new client logic.
//!
//! mDNS discovery (the other connectivity path) is unit-tested for service/TXT
//! construction in `src/discovery.rs`; its live multicast round-trip is an
//! `#[ignore]`d test there (multicast is commonly blocked in CI/sandbox).

use std::sync::Arc;
use std::time::Duration;

use commonplace_desktop_runtime::{
    serve_control, ControlState, DevicePairing, MockRelay, RelayClient, RelayClientConfig,
    RelayCredential, RunRegistry, TunnelRequest,
};
use commonplace_desktop_runtime::{MockExecutor, ControlServer};

/// Spin up a real loopback control endpoint with one paired device, returning the
/// running server, the device's bearer token, and the temp dir (kept alive so the
/// sidecar registry file is not dropped).
async fn start_control() -> (ControlServer, String, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let pairing = DevicePairing::open(dir.path()).unwrap();
    // Pair a device up front so we have a valid token for the authed cases.
    let paired = pairing.pair_device("Tunnel Phone").unwrap();
    let runs = RunRegistry::new(Arc::new(MockExecutor::new()));
    let state = ControlState::new(pairing, "pairing-code-xyz", runs);
    let server = serve_control(state, 0).await.expect("bind loopback control");
    (server, paired.token, dir)
}

/// Build a request frame helper (lower-cased headers, like a real HTTP proxy).
fn request(
    request_id: &str,
    method: &str,
    path: &str,
    auth: Option<&str>,
) -> TunnelRequest {
    let mut headers = std::collections::BTreeMap::new();
    if let Some(token) = auth {
        headers.insert("authorization".to_string(), format!("Bearer {token}"));
    }
    TunnelRequest {
        request_id: request_id.to_string(),
        method: method.to_string(),
        path: path.to_string(),
        headers,
        body_b64: String::new(),
    }
}

/// The end-to-end happy path PLUS the device-auth boundary, all through the
/// tunnel: healthz (unauth) succeeds, /v1/status WITH a valid token succeeds, and
/// /v1/status WITHOUT a token is 401 -- the relay never saw the token; the
/// loopback control endpoint enforced it.
#[tokio::test]
async fn tunneled_requests_reach_control_endpoint_and_preserve_device_auth() {
    let (control, token, _dir) = start_control().await;
    let control_base = format!("http://{}", control.local_addr());

    // Start the in-process mock relay and an outbound RelayClient pointed at it.
    let mut relay = MockRelay::start("instance-relay-secret")
        .await
        .expect("start mock relay");
    let config = RelayClientConfig::new(
        relay.ws_url(),
        "inst_tunnel_test",
        control_base.clone(),
    );
    let credential = RelayCredential::new(relay.expected_credential());
    let client = RelayClient::new(config, credential);

    // Run the client tunnel on a task; cancel it at the end.
    let cancel = Arc::new(tokio::sync::Notify::new());
    let client_cancel = Arc::clone(&cancel);
    let client_task = tokio::spawn(async move { client.run(client_cancel).await });

    // The relay accepts the instance's registration.
    let mut conn = tokio::time::timeout(Duration::from_secs(5), relay.accept_connection())
        .await
        .expect("instance connects to relay in time")
        .expect("relay yields the registered connection");
    assert_eq!(
        conn.instance_id(),
        "inst_tunnel_test",
        "the instance registered with its non-secret id"
    );

    // 1) Unauthenticated /healthz tunnels through to a 200.
    let health = conn
        .send_request(request("r-health", "GET", "/healthz", None))
        .await
        .expect("healthz tunnels");
    assert_eq!(health.status, 200, "healthz reaches the control endpoint via the tunnel");
    assert_eq!(
        String::from_utf8(health.body_bytes().unwrap()).unwrap(),
        "ok"
    );

    // 2) /v1/status WITH a valid device token tunnels through to a 200.
    let authed = conn
        .send_request(request("r-status", "GET", "/v1/status", Some(&token)))
        .await
        .expect("authed status tunnels");
    assert_eq!(
        authed.status, 200,
        "a valid device token authorizes /v1/status through the tunnel"
    );
    let status_json: serde_json::Value =
        serde_json::from_slice(&authed.body_bytes().unwrap()).unwrap();
    assert_eq!(status_json["status"], "ok");

    // 3) /v1/status WITHOUT a token is 401 -- device-auth is enforced END-TO-END
    //    at the loopback control endpoint, NOT at the relay (which never sees a
    //    token). This is the security keystone of the slice.
    let unauth = conn
        .send_request(request("r-unauth", "GET", "/v1/status", None))
        .await
        .expect("unauthed status tunnels");
    assert_eq!(
        unauth.status, 401,
        "an un-authenticated request is rejected through the tunnel, exactly as over loopback"
    );

    // Shut down cleanly.
    cancel.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), client_task).await;
    control.shutdown().await.expect("control shuts down");
}

/// The RelayClient reconnects after the relay drops the connection: a request
/// works, the relay drops the socket, and a request over a freshly accepted
/// connection works again.
#[tokio::test]
async fn relay_client_reconnects_after_a_drop() {
    let (control, token, _dir) = start_control().await;
    let control_base = format!("http://{}", control.local_addr());

    let mut relay = MockRelay::start("reconnect-secret")
        .await
        .expect("start mock relay");
    // A short initial backoff so the test does not wait long for the reconnect.
    let mut config = RelayClientConfig::new(relay.ws_url(), "inst_reconnect", control_base);
    config.initial_backoff = Duration::from_millis(50);
    config.max_backoff = Duration::from_millis(200);
    let credential = RelayCredential::new(relay.expected_credential());
    let client = RelayClient::new(config, credential);

    let cancel = Arc::new(tokio::sync::Notify::new());
    let client_cancel = Arc::clone(&cancel);
    let client_task = tokio::spawn(async move { client.run(client_cancel).await });

    // First connection: a request works.
    let mut conn = tokio::time::timeout(Duration::from_secs(5), relay.accept_connection())
        .await
        .expect("first connect in time")
        .expect("first connection");
    let first = conn
        .send_request(request("r1", "GET", "/v1/status", Some(&token)))
        .await
        .expect("first request tunnels");
    assert_eq!(first.status, 200);

    // Drop the connection to simulate a relay outage.
    conn.drop_connection().await;

    // The client should reconnect; the relay accepts a SECOND connection.
    let mut conn2 = tokio::time::timeout(Duration::from_secs(10), relay.accept_connection())
        .await
        .expect("client reconnects after the drop")
        .expect("second connection");
    assert_eq!(conn2.instance_id(), "inst_reconnect");

    // A request over the reconnected tunnel works again.
    let second = conn2
        .send_request(request("r2", "GET", "/v1/status", Some(&token)))
        .await
        .expect("second request tunnels after reconnect");
    assert_eq!(
        second.status, 200,
        "the tunnel works again after an automatic reconnect"
    );

    cancel.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), client_task).await;
    control.shutdown().await.expect("control shuts down");
}

/// A POST through the tunnel with a JSON body reaches the control endpoint and a
/// real run is submitted -- proving the tunnel carries request bodies (not just
/// GETs) and that an authed mutation works end-to-end.
#[tokio::test]
async fn tunneled_post_with_body_submits_a_run() {
    let (control, token, _dir) = start_control().await;
    let control_base = format!("http://{}", control.local_addr());

    let mut relay = MockRelay::start("post-secret").await.expect("start mock relay");
    let config = RelayClientConfig::new(relay.ws_url(), "inst_post", control_base);
    let client = RelayClient::new(config, RelayCredential::new(relay.expected_credential()));

    let cancel = Arc::new(tokio::sync::Notify::new());
    let client_cancel = Arc::clone(&cancel);
    let client_task = tokio::spawn(async move { client.run(client_cancel).await });

    let mut conn = tokio::time::timeout(Duration::from_secs(5), relay.accept_connection())
        .await
        .expect("connect in time")
        .expect("connection");

    // POST /v1/runs with a JSON body and the device token.
    let body = serde_json::to_vec(&serde_json::json!({ "intent": "explain main.rs" })).unwrap();
    let mut req = request("r-post", "POST", "/v1/runs", Some(&token));
    req.headers
        .insert("content-type".to_string(), "application/json".to_string());
    req.body_b64 = base64_encode(&body);

    let response = conn.send_request(req).await.expect("post tunnels");
    assert_eq!(
        response.status, 200,
        "an authed POST with a body submits a run through the tunnel"
    );
    let json: serde_json::Value = serde_json::from_slice(&response.body_bytes().unwrap()).unwrap();
    assert!(
        json["run_id"].as_str().is_some(),
        "the tunneled POST returned a run id (the body was delivered)"
    );

    cancel.notify_waiters();
    let _ = tokio::time::timeout(Duration::from_secs(5), client_task).await;
    control.shutdown().await.expect("control shuts down");
}

/// Standard base64 (matches the relay module's body encoding).
fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
