//! Part B (B3 + B5) acceptance: the run channel over the authenticated control
//! endpoint, exercised end-to-end through a real loopback bind with `reqwest`.
//!
//! Coverage (the mandatory matrix for this slice):
//! * a tier-1 run submitted over HTTP runs and streams Trace/Obligation/Diff
//!   events to completion (read off the real SSE stream);
//! * a tier-3 run submitted over HTTP HOLDS in `awaiting_authorization` and does
//!   NOT run its gated action until `approve` arrives over the endpoint, then
//!   completes;
//! * `stop` over HTTP halts an in-flight run;
//! * unauthenticated access to every `/v1/runs*` route is `401`.
//!
//! All run traffic is behind `DeviceAuth` (loopback bind already). These tests
//! pair a device first (over the wire, with the pairing code) and then use its
//! bearer token, exactly as a phone would.

use std::sync::Arc;
use std::time::Duration;

use commonplace_desktop_runtime::{
    serve_control, ControlServer, ControlState, DevicePairing, MockExecutor, RunRegistry,
    PAIRING_CODE_HEADER, TIER_THREE,
};

const PAIRING_CODE: &str = "test-pairing-code";

/// Bring up a control server on an ephemeral loopback port with a mock executor.
/// Returns the server (hold it to keep serving), the base URL, and a tempdir
/// keeping the device registry alive.
async fn serve_test() -> (tempfile::TempDir, ControlServer, String) {
    let dir = tempfile::tempdir().unwrap();
    let pairing = DevicePairing::open(dir.path()).unwrap();
    let runs = RunRegistry::new(Arc::new(MockExecutor::new()));
    let state = ControlState::new(pairing, PAIRING_CODE, runs);
    let server = serve_control(state, 0).await.expect("bind loopback");
    let base = format!("http://{}", server.local_addr());
    (dir, server, base)
}

/// Bring up a control server whose mock executor pauses after its first event,
/// so a stop test can land the cancel while the run is genuinely in flight.
async fn serve_test_pausing() -> (tempfile::TempDir, ControlServer, String) {
    let dir = tempfile::tempdir().unwrap();
    let pairing = DevicePairing::open(dir.path()).unwrap();
    let runs = RunRegistry::new(Arc::new(MockExecutor::pausing_after_first_event()));
    let state = ControlState::new(pairing, PAIRING_CODE, runs);
    let server = serve_control(state, 0).await.expect("bind loopback");
    let base = format!("http://{}", server.local_addr());
    (dir, server, base)
}

/// Pair a device over the wire and return its bearer token.
async fn pair(client: &reqwest::Client, base: &str) -> String {
    let response = client
        .post(format!("{base}/pair"))
        .header(PAIRING_CODE_HEADER, PAIRING_CODE)
        .json(&serde_json::json!({ "label": "Test Phone" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Poll `GET /v1/runs/:id` until `done(record)` holds or a bound elapses; return
/// the final record JSON.
async fn poll_run(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    run_id: &str,
    mut done: impl FnMut(&serde_json::Value) -> bool,
) -> serde_json::Value {
    let bound = Duration::from_secs(15);
    let step = Duration::from_millis(25);
    let mut waited = Duration::ZERO;
    loop {
        let response = client
            .get(format!("{base}/v1/runs/{run_id}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = response.json::<serde_json::Value>().await.unwrap();
        let record = &body["run"];
        if done(record) || waited >= bound {
            return record.clone();
        }
        tokio::time::sleep(step).await;
        waited += step;
    }
}

fn event_kinds(record: &serde_json::Value) -> Vec<String> {
    record["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["kind"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn tier_one_run_streams_events_to_completion_over_http() {
    let (_dir, server, base) = serve_test().await;
    let client = reqwest::Client::new();
    let token = pair(&client, &base).await;

    // Submit a tier-1 run (default tier).
    let submit = client
        .post(format!("{base}/v1/runs"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "intent": "explain main.rs" }))
        .send()
        .await
        .unwrap();
    assert_eq!(submit.status(), reqwest::StatusCode::OK);
    let submit_body = submit.json::<serde_json::Value>().await.unwrap();
    let run_id = submit_body["run_id"].as_str().unwrap().to_string();
    // A tier-1 run is immediate: it should be running or already done, never held.
    assert_ne!(
        submit_body["state"], "awaiting_authorization",
        "a tier-1 run must not hold for authorization"
    );

    // Read the SSE stream and confirm the three first-class event kinds arrive.
    // We collect the raw SSE text until we see the terminal "done" status or a
    // bound elapses.
    let mut sse = client
        .get(format!("{base}/v1/runs/{run_id}/events"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(sse.status(), reqwest::StatusCode::OK);
    assert!(
        sse.headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.starts_with("text/event-stream"))
            .unwrap_or(false),
        "the events route must be an SSE stream"
    );

    // axum serializes SSE fields as `event: <name>` (a space after the colon),
    // so match on that exact wire form.
    let mut collected = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), sse.chunk()).await {
            Ok(Ok(Some(chunk))) => {
                collected.push_str(&String::from_utf8_lossy(&chunk));
                if collected.contains("event: status") && collected.contains("\"body\":\"done\"") {
                    break;
                }
            }
            // Stream ended.
            Ok(Ok(None)) => break,
            Ok(Err(_)) => break,
            Err(_timeout) => continue,
        }
    }

    assert!(
        collected.contains("event: trace"),
        "SSE stream carries a Trace event; got:\n{collected}"
    );
    assert!(
        collected.contains("event: obligation"),
        "SSE stream carries an Obligation event; got:\n{collected}"
    );
    assert!(
        collected.contains("event: diff"),
        "SSE stream carries a Diff event; got:\n{collected}"
    );

    // The record route agrees the run completed with all three event kinds.
    let record = poll_run(&client, &base, &token, &run_id, |r| r["state"] == "done").await;
    assert_eq!(record["state"], "done");
    let kinds = event_kinds(&record);
    assert!(kinds.contains(&"trace".to_string()));
    assert!(kinds.contains(&"obligation".to_string()));
    assert!(kinds.contains(&"diff".to_string()));

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn tier_three_run_holds_until_approved_over_http() {
    let (_dir, server, base) = serve_test().await;
    let client = reqwest::Client::new();
    let token = pair(&client, &base).await;

    // Submit a tier-3 (irreversible) run.
    let submit = client
        .post(format!("{base}/v1/runs"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "intent": "commit and push", "action_tier": TIER_THREE }))
        .send()
        .await
        .unwrap();
    assert_eq!(submit.status(), reqwest::StatusCode::OK);
    let submit_body = submit.json::<serde_json::Value>().await.unwrap();
    let run_id = submit_body["run_id"].as_str().unwrap().to_string();
    assert_eq!(
        submit_body["state"], "awaiting_authorization",
        "a tier-3 run must hold for authorization"
    );

    // It stays held and runs NO gated action (no Trace/Obligation/Diff yet).
    let held = poll_run(&client, &base, &token, &run_id, |r| {
        r["state"] == "awaiting_authorization"
    })
    .await;
    let executor_events = held["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] != "status")
        .count();
    assert_eq!(
        executor_events, 0,
        "a held tier-3 run must not execute its gated action before approval"
    );

    // Approve over the endpoint; the run then runs to completion.
    let approve = client
        .post(format!("{base}/v1/runs/{run_id}/approve"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(approve.status(), reqwest::StatusCode::OK);

    let done = poll_run(&client, &base, &token, &run_id, |r| r["state"] == "done").await;
    assert_eq!(done["state"], "done", "an approved run completes");
    assert!(
        event_kinds(&done).contains(&"diff".to_string()),
        "the gated action runs only after approval"
    );

    // Re-approving a terminal run is a 409 Conflict.
    let reapprove = client
        .post(format!("{base}/v1/runs/{run_id}/approve"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(reapprove.status(), reqwest::StatusCode::CONFLICT);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn stop_halts_an_in_flight_run_over_http() {
    let (_dir, server, base) = serve_test_pausing().await;
    let client = reqwest::Client::new();
    let token = pair(&client, &base).await;

    let submit = client
        .post(format!("{base}/v1/runs"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "intent": "long task" }))
        .send()
        .await
        .unwrap();
    let run_id = submit.json::<serde_json::Value>().await.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Wait until it is genuinely running (the pausing mock emitted its first
    // Trace and is now waiting on cancellation).
    poll_run(&client, &base, &token, &run_id, |r| r["state"] == "running").await;

    // Stop it over the endpoint.
    let stop = client
        .post(format!("{base}/v1/runs/{run_id}/stop"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(stop.status(), reqwest::StatusCode::OK);

    let stopped = poll_run(&client, &base, &token, &run_id, |r| r["state"] == "stopped").await;
    assert_eq!(
        stopped["state"], "stopped",
        "stop halts the in-flight run cooperatively"
    );
    assert!(
        !event_kinds(&stopped).contains(&"diff".to_string()),
        "a stopped run does not complete its remaining work"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn unauthenticated_run_routes_are_401() {
    let (_dir, server, base) = serve_test().await;
    let client = reqwest::Client::new();

    // Every /v1/runs* route must reject a request with no token.
    let probes: Vec<(reqwest::Method, String)> = vec![
        (reqwest::Method::GET, format!("{base}/v1/runs")),
        (reqwest::Method::POST, format!("{base}/v1/runs")),
        (reqwest::Method::GET, format!("{base}/v1/runs/run_x")),
        (reqwest::Method::GET, format!("{base}/v1/runs/run_x/events")),
        (reqwest::Method::POST, format!("{base}/v1/runs/run_x/approve")),
        (reqwest::Method::POST, format!("{base}/v1/runs/run_x/redirect")),
        (reqwest::Method::POST, format!("{base}/v1/runs/run_x/stop")),
    ];
    for (method, url) in probes {
        let response = client
            .request(method.clone(), &url)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            reqwest::StatusCode::UNAUTHORIZED,
            "{method} {url} must be 401 without a token"
        );
    }

    // A garbage bearer token is also rejected on a run route.
    let garbage = client
        .get(format!("{base}/v1/runs"))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(garbage.status(), reqwest::StatusCode::UNAUTHORIZED);

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn redirect_injects_an_instruction_over_http() {
    let (_dir, server, base) = serve_test_pausing().await;
    let client = reqwest::Client::new();
    let token = pair(&client, &base).await;

    let submit = client
        .post(format!("{base}/v1/runs"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "intent": "initial plan" }))
        .send()
        .await
        .unwrap();
    let run_id = submit.json::<serde_json::Value>().await.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();

    poll_run(&client, &base, &token, &run_id, |r| r["state"] == "running").await;

    // Inject a steering instruction over the endpoint.
    let redirect = client
        .post(format!("{base}/v1/runs/{run_id}/redirect"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "instruction": "use the other module" }))
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.status(), reqwest::StatusCode::OK);

    // The mock (after its bounded pause) drains the redirect into a Trace.
    let done = poll_run(&client, &base, &token, &run_id, |r| r["state"] == "done").await;
    let redirected = done["events"].as_array().unwrap().iter().any(|e| {
        e["kind"] == "trace"
            && e["body"]
                .as_str()
                .map(|b| b.contains("redirected: use the other module"))
                .unwrap_or(false)
    });
    assert!(redirected, "the executor picks up the injected redirect");

    // An empty instruction is a 400.
    // (Use a fresh run so the prior one being terminal does not mask it.)
    let submit2 = client
        .post(format!("{base}/v1/runs"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "intent": "second" }))
        .send()
        .await
        .unwrap();
    let run_id2 = submit2.json::<serde_json::Value>().await.unwrap()["run_id"]
        .as_str()
        .unwrap()
        .to_string();
    let bad = client
        .post(format!("{base}/v1/runs/{run_id2}/redirect"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "instruction": "   " }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), reqwest::StatusCode::BAD_REQUEST);

    server.shutdown().await.expect("clean shutdown");
}
