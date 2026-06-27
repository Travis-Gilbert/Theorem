//! Agent co-presence acceptance (spec: HANDOFF-AGENT-COEDIT-PRESENCE-LAYER.md),
//! exercised end-to-end through a real loopback bind with `reqwest`.
//!
//! The governing claim is that two concurrent agent PROCESSES see each other's
//! cursor + pending-edit footprints over the one local instance, with no
//! filesystem recon. Here each process is a distinct `reqwest::Client` paired as
//! its own device, both hitting the same bound control server -- which is exactly
//! how Claude Code and Codex (each its own OS process, each an HTTP client of the
//! local instance) share presence with the remote harness offline.
//!
//! Coverage (the mandatory matrix for this slice):
//! * two agents announce presence + overlapping pending edits on the same path ->
//!   `would_overlap` flags the peer's footprint AND excludes the caller's own;
//! * non-overlapping ranges -> empty;
//! * the presence list reflects announces (across both agents);
//! * every `/v1/presence*` route is `401` without a valid device token.
//!
//! All presence traffic is behind `DeviceAuth` (loopback bind already); these
//! tests pair a device first (over the wire, with the pairing code) and then use
//! its bearer token, exactly as an agent's hook would.

use std::sync::Arc;

use commonplace_desktop_runtime::{
    serve_control, ControlServer, ControlState, DevicePairing, MockExecutor, PresenceRegistry,
    RunRegistry, PAIRING_CODE_HEADER,
};

const PAIRING_CODE: &str = "test-pairing-code";

/// Bring up a control server on an ephemeral loopback port. Returns the server
/// (hold it to keep serving), the base URL, and a tempdir keeping the device
/// registry alive. A fresh empty presence registry is wired in.
async fn serve_test() -> (tempfile::TempDir, ControlServer, String) {
    let dir = tempfile::tempdir().unwrap();
    let pairing = DevicePairing::open(dir.path()).unwrap();
    let runs = RunRegistry::new(Arc::new(MockExecutor::new()));
    // Explicitly share a known PresenceRegistry to prove the wiring seam, though
    // `ControlState::new` already creates one.
    let state =
        ControlState::new(pairing, PAIRING_CODE, runs).with_presence(PresenceRegistry::new());
    let server = serve_control(state, 0).await.expect("bind loopback");
    let base = format!("http://{}", server.local_addr());
    (dir, server, base)
}

/// Pair a device over the wire and return its bearer token. Each call simulates a
/// distinct agent process pairing its own device.
async fn pair(client: &reqwest::Client, base: &str, label: &str) -> String {
    let response = client
        .post(format!("{base}/pair"))
        .header(PAIRING_CODE_HEADER, PAIRING_CODE)
        .json(&serde_json::json!({ "label": label }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    response.json::<serde_json::Value>().await.unwrap()["token"]
        .as_str()
        .unwrap()
        .to_string()
}

fn range(start_line: u32, end_line: u32) -> serde_json::Value {
    serde_json::json!({ "start_line": start_line, "start_col": 0, "end_line": end_line, "end_col": 0 })
}

#[tokio::test]
async fn two_agent_processes_see_overlapping_pending_edits_excluding_self() {
    let (_dir, server, base) = serve_test().await;

    // Two distinct agent processes: each its own HTTP client + paired device.
    let claude = reqwest::Client::new();
    let codex = reqwest::Client::new();
    let claude_token = pair(&claude, &base, "Claude Code").await;
    let codex_token = pair(&codex, &base, "Codex").await;

    // Both announce presence on the same file (no filesystem recon).
    let r = claude
        .post(format!("{base}/v1/presence"))
        .bearer_auth(&claude_token)
        .json(&serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs", "line": 10, "col": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);
    let r = codex
        .post(format!("{base}/v1/presence"))
        .bearer_auth(&codex_token)
        .json(&serde_json::json!({ "actor": "codex", "path": "src/lib.rs", "line": 40, "col": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), reqwest::StatusCode::OK);

    // Each process sets a pending-edit footprint; the ranges OVERLAP (10-20 vs 15-25).
    codex
        .post(format!("{base}/v1/presence/footprint"))
        .bearer_auth(&codex_token)
        .json(&serde_json::json!({
            "actor": "codex", "path": "src/lib.rs", "range": range(15, 25), "summary": "rename fn b"
        }))
        .send()
        .await
        .unwrap();
    claude
        .post(format!("{base}/v1/presence/footprint"))
        .bearer_auth(&claude_token)
        .json(&serde_json::json!({
            "actor": "claude-code", "path": "src/lib.rs", "range": range(10, 20), "summary": "refactor fn a"
        }))
        .send()
        .await
        .unwrap();

    // Claude (about to edit 10-20) queries: it must see CODEX's overlapping
    // footprint, set in another process, and NOT its own.
    let overlap = claude
        .post(format!("{base}/v1/presence/would-overlap"))
        .bearer_auth(&claude_token)
        .json(&serde_json::json!({
            "actor": "claude-code", "path": "src/lib.rs", "intended": range(10, 20)
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let overlaps = overlap["overlaps"].as_array().unwrap();
    assert_eq!(
        overlaps.len(),
        1,
        "exactly the peer's footprint is flagged across processes"
    );
    assert_eq!(
        overlaps[0]["summary"], "rename fn b",
        "the flagged footprint is the peer's, set from another process"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn non_overlapping_ranges_return_empty_over_http() {
    let (_dir, server, base) = serve_test().await;
    let agent = reqwest::Client::new();
    let token = pair(&agent, &base, "Phone").await;

    // A peer footprint at lines 40-50.
    agent
        .post(format!("{base}/v1/presence/footprint"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "actor": "codex", "path": "src/lib.rs", "range": range(40, 50), "summary": "tail"
        }))
        .send()
        .await
        .unwrap();

    // Claude intends lines 10-20: disjoint, so the result is empty.
    let overlap = agent
        .post(format!("{base}/v1/presence/would-overlap"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "actor": "claude-code", "path": "src/lib.rs", "intended": range(10, 20)
        }))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(
        overlap["overlaps"].as_array().unwrap().is_empty(),
        "non-overlapping ranges flag nothing"
    );

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn presence_list_reflects_announces_over_http() {
    let (_dir, server, base) = serve_test().await;
    let agent = reqwest::Client::new();
    let token = pair(&agent, &base, "Phone").await;

    for (actor, path) in [("claude-code", "src/lib.rs"), ("codex", "src/main.rs")] {
        agent
            .post(format!("{base}/v1/presence"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "actor": actor, "path": path, "line": 1, "col": 0 }))
            .send()
            .await
            .unwrap();
    }

    let list = agent
        .get(format!("{base}/v1/presence"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let presences = list["presences"].as_array().unwrap();
    assert_eq!(presences.len(), 2, "the presence list reflects both announces");
    let labels: Vec<&str> = presences
        .iter()
        .map(|p| p["actor"].as_str().unwrap())
        .collect();
    // Actor ids are hex (ActorId::from_label); just assert two distinct ids.
    assert_ne!(labels[0], labels[1], "two distinct agents are listed");

    server.shutdown().await.expect("clean shutdown");
}

#[tokio::test]
async fn every_presence_route_is_401_without_a_token() {
    let (_dir, server, base) = serve_test().await;
    let client = reqwest::Client::new();

    // GET list, POST announce, POST/DELETE footprint, POST would-overlap.
    let get = client
        .get(format!("{base}/v1/presence"))
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::UNAUTHORIZED);

    let announce = client
        .post(format!("{base}/v1/presence"))
        .json(&serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs", "line": 1, "col": 0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(announce.status(), reqwest::StatusCode::UNAUTHORIZED);

    let set = client
        .post(format!("{base}/v1/presence/footprint"))
        .json(&serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs", "range": range(1, 2) }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), reqwest::StatusCode::UNAUTHORIZED);

    let clear = client
        .delete(format!("{base}/v1/presence/footprint"))
        .json(&serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs" }))
        .send()
        .await
        .unwrap();
    assert_eq!(clear.status(), reqwest::StatusCode::UNAUTHORIZED);

    let overlap = client
        .post(format!("{base}/v1/presence/would-overlap"))
        .json(&serde_json::json!({ "actor": "claude-code", "path": "src/lib.rs", "intended": range(1, 2) }))
        .send()
        .await
        .unwrap();
    assert_eq!(overlap.status(), reqwest::StatusCode::UNAUTHORIZED);

    // A garbage bearer token is also rejected.
    let garbage = client
        .get(format!("{base}/v1/presence"))
        .bearer_auth("not-a-real-token")
        .send()
        .await
        .unwrap();
    assert_eq!(garbage.status(), reqwest::StatusCode::UNAUTHORIZED);

    server.shutdown().await.expect("clean shutdown");
}
