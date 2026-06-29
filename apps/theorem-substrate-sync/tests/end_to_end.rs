//! SPEC §7 end-to-end acceptance for the substrate-sync daemon.
//!
//! Default mode runs each scenario against an in-process fake harness MCP
//! server. Exercises the daemon's wire behavior for Roles 1 (Prolly round +
//! bootstrap) and 2 (stream tail + offline outbox); Role 3 (CRDT merge
//! registry) is covered by `rustyred-thg-core`'s merge_registry unit tests
//! (PT-012..PT-014) and intentionally skipped here.
//!
//! Live mode: set `THEOREM_SYNC_LIVE_E2E=1` along with
//! `THEOREM_SYNC_RAILWAY_URL` and optionally `THEOREM_SYNC_RAILWAY_TOKEN`
//! to run the stream smoke against a real Railway tenant. Full-pack live
//! round verification additionally requires `THEOREM_SYNC_LIVE_FULL_PACK_E2E=1`
//! and should target a fresh/small tenant, not the production Travis-Gilbert
//! graph.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio::sync::Mutex;

use theorem_substrate_sync::bootstrap::{
    bootstrap_from_remote, bootstrap_memory_documents_from_remote,
};
use theorem_substrate_sync::cursor::{CursorStore, InMemoryCursorStore};
use theorem_substrate_sync::drainer::OutboxDrainer;
use theorem_substrate_sync::outbox::{InMemoryOutbox, OutboxStore, QueuedOutboxEvent};
use theorem_substrate_sync::railway_client::McpClient;
use theorem_substrate_sync::round::run_round;
use theorem_substrate_sync::status::{OutboxState, StatusHandle, StreamState, SyncStatus};
use theorem_substrate_sync::subscriber::read_and_apply_once;

// ----- in-process fake harness MCP server -----

#[derive(Clone)]
struct FakeState {
    inner: Arc<Mutex<FakeInner>>,
    stream_up: Arc<AtomicBool>,
    remote_up: Arc<AtomicBool>,
    publish_count: Arc<AtomicU64>,
    ref_count: Arc<AtomicU64>,
}

#[derive(Default)]
struct FakeInner {
    nodes: HashMap<String, Value>,
    edges: Vec<Value>,
    events: Vec<Value>,
    next_event_id: u64,
}

impl FakeState {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FakeInner::default())),
            stream_up: Arc::new(AtomicBool::new(true)),
            remote_up: Arc::new(AtomicBool::new(true)),
            publish_count: Arc::new(AtomicU64::new(0)),
            ref_count: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn seed_node(&self, id: &str) {
        let mut g = self.inner.lock().await;
        g.nodes.insert(id.to_string(), make_node_payload(id));
    }

    async fn counts(&self) -> (usize, usize) {
        let g = self.inner.lock().await;
        (g.nodes.len(), g.edges.len())
    }

    #[allow(dead_code)] // reserved for future round-test assertions; counter is live
    fn ref_count(&self) -> u64 {
        self.ref_count.load(Ordering::SeqCst)
    }

    /// Inject a stream event as if it were committed on the remote.
    /// Mirrors what the daemon's drainer would publish.
    async fn inject_stream_snapshot(&self, nodes: Vec<Value>, edges: Vec<Value>) {
        let mut g = self.inner.lock().await;
        g.next_event_id += 1;
        let id = g.next_event_id;
        g.events.push(json!({
            "id": id.to_string(),
            "payload": {
                "snapshot": { "nodes": nodes, "edges": edges }
            }
        }));
    }
}

async fn fake_handler(State(state): State<FakeState>, Json(req): Json<Value>) -> Json<Value> {
    let id = req.get("id").cloned().unwrap_or(json!(0));

    // Per-state circuit breaker: simulate a fully-down remote for the
    // offline-outbox + bootstrap-failure scenarios.
    if !state.remote_up.load(Ordering::SeqCst) {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32099, "message": "fake: remote down" }
        }));
    }

    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    if method != "tools/call" {
        return Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": -32601, "message": format!("unknown method: {method}") }
        }));
    }

    let params = req.get("params").cloned().unwrap_or(json!({}));
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    let result = handle_tool_call(&state, name, &args).await;

    match result {
        Ok(v) => Json(json!({ "jsonrpc": "2.0", "id": id, "result": v })),
        Err((code, msg)) => Json(json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": { "code": code, "message": msg }
        })),
    }
}

async fn handle_tool_call(
    state: &FakeState,
    name: &str,
    args: &Value,
) -> Result<Value, (i64, String)> {
    let mut g = state.inner.lock().await;

    match name {
        "rustyred_thg_graph_version_compile" | "rustyred_thg_graph_version_ref" => {
            if name == "rustyred_thg_graph_version_ref" {
                state.ref_count.fetch_add(1, Ordering::SeqCst);
            }
            let mut objects: Vec<Value> = g
                .nodes
                .values()
                .map(|n| json!({ "kind": "node", "payload": n }))
                .collect();
            for e in &g.edges {
                objects.push(json!({ "kind": "edge", "payload": e }));
            }
            let nodes_total = g.nodes.len() as u64;
            let edges_total = g.edges.len() as u64;
            Ok(json!({
                "structuredContent": {
                    "pack": {
                        "manifest": {
                            "graph_version": 1,
                            "nodes_total": nodes_total,
                            "edges_total": edges_total
                        },
                        "objects": objects
                    }
                }
            }))
        }
        "graphql_mutate" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let variables = args.get("variables").cloned().unwrap_or(json!({}));
            if query.contains("bulkNodes") {
                if let Some(nodes) = variables.get("n").and_then(Value::as_array) {
                    for node in nodes {
                        if let Some(node_id) = node.get("id").and_then(Value::as_str) {
                            g.nodes.insert(node_id.to_string(), node.clone());
                        }
                    }
                }
            } else if query.contains("bulkEdges") {
                if let Some(edges) = variables.get("e").and_then(Value::as_array) {
                    for edge in edges {
                        g.edges.push(edge.clone());
                    }
                }
            }
            Ok(json!({ "structuredContent": { "ok": true } }))
        }
        "stream_publish" => {
            if !state.stream_up.load(Ordering::SeqCst) {
                return Err((-32603, "stream unavailable (test)".to_string()));
            }
            state.publish_count.fetch_add(1, Ordering::SeqCst);
            let payload = args.get("payload").cloned().unwrap_or(json!({}));
            g.next_event_id += 1;
            let event_id = g.next_event_id;
            g.events.push(json!({
                "id": event_id.to_string(),
                "payload": payload
            }));
            Ok(json!({ "structuredContent": { "ok": true } }))
        }
        "stream_read" => {
            if !state.stream_up.load(Ordering::SeqCst) {
                return Err((-32603, "stream unavailable (test)".to_string()));
            }
            let stream_key = args.get("stream").and_then(Value::as_str).unwrap_or("");
            let advance = args
                .get("advance")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let events_to_return: Vec<Value> = g.events.clone();
            if advance {
                g.events.clear();
            }
            let cursor = g.next_event_id;
            Ok(json!({
                "structuredContent": {
                    "events": events_to_return,
                    "new_cursors": { stream_key: cursor }
                }
            }))
        }
        "rustyred_thg_graph_version_merge" => {
            // For the fake, a "merge" is the union of ours+theirs payloads.
            // Real Prolly merge with auto_confidence + the registry lives in
            // rustyred-thg-core; its semantics are covered by that crate's
            // own unit tests. We only need this verb to return a snapshot
            // shape the daemon's apply_snapshot can ingest.
            let mut merged_nodes: HashMap<String, Value> = HashMap::new();
            let mut merged_edges: Vec<Value> = Vec::new();
            for side in ["ours", "theirs", "base"] {
                let snap = args.get(side);
                if let Some(nodes) = snap.and_then(|s| s.get("nodes")).and_then(Value::as_array) {
                    for n in nodes {
                        if let Some(id) = n.get("id").and_then(Value::as_str) {
                            merged_nodes.insert(id.to_string(), n.clone());
                        }
                    }
                }
                if let Some(edges) = snap.and_then(|s| s.get("edges")).and_then(Value::as_array) {
                    for e in edges {
                        merged_edges.push(e.clone());
                    }
                }
            }
            let nodes_vec: Vec<Value> = merged_nodes.values().cloned().collect();
            Ok(json!({
                "structuredContent": {
                    "merge": {
                        "merged_snapshot": {
                            "version": 1,
                            "nodes": nodes_vec,
                            "edges": merged_edges
                        }
                    }
                }
            }))
        }
        "harness_kg_status" => Ok(json!({
            "structuredContent": {
                "stats": {
                    "nodes_total": g.nodes.len(),
                    "edges_total": g.edges.len()
                }
            }
        })),
        "memory_documents_dump" => {
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(500) as usize;
            let before = args
                .get("before")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let mut nodes: Vec<Value> = g.nodes.values().cloned().collect();
            nodes.sort_by(|left, right| {
                let left_updated_at = left
                    .get("properties")
                    .and_then(|properties| properties.get("updated_at"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let right_updated_at = right
                    .get("properties")
                    .and_then(|properties| properties.get("updated_at"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                right_updated_at.cmp(left_updated_at)
            });
            if !before.is_empty() {
                nodes.retain(|node| {
                    node.get("properties")
                        .and_then(|properties| properties.get("updated_at"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        < before
                });
            }
            let truncated = nodes.len() > limit;
            if truncated {
                nodes.truncate(limit);
            }
            let next_before = if truncated {
                nodes
                    .last()
                    .and_then(|node| node.get("properties"))
                    .and_then(|properties| properties.get("updated_at"))
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string()
            } else {
                String::new()
            };
            let docs = nodes
                .iter()
                .filter_map(|node| node.get("properties").cloned())
                .collect::<Vec<_>>();
            Ok(json!({
                "structuredContent": {
                    "tenant": "Travis-Gilbert",
                    "count": docs.len(),
                    "limit": limit,
                    "truncated": truncated,
                    "next_before": next_before,
                    "docs": docs,
                    "nodes": nodes
                }
            }))
        }
        _ => Ok(json!({
            "structuredContent": { "ok": false, "warning": format!("unhandled verb: {name}") }
        })),
    }
}

async fn spawn_fake(state: FakeState) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .route("/", post(fake_handler))
        .with_state(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

fn make_node_payload(id: &str) -> Value {
    json!({
        "id": id,
        "labels": ["MemoryDocument"],
        "properties": {
            "created_at_ms": 1,
            "updated_at": id
        }
    })
}

fn handle() -> StatusHandle {
    StatusHandle::new(SyncStatus::new(true, "Travis-Gilbert", 5_000))
}

// ----- SPEC §7 scenarios -----

/// Step 1 + 3: cold local node onboards atomically from Railway's head.
#[tokio::test]
async fn step_1_3_bootstrap_is_atomic_from_remote_head() {
    let remote = FakeState::new();
    let local = FakeState::new();
    remote.seed_node("n:1").await;
    remote.seed_node("n:2").await;
    remote.seed_node("n:3").await;

    let remote_url = spawn_fake(remote.clone()).await;
    let local_url = spawn_fake(local.clone()).await;

    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");
    let l = McpClient::unauthenticated(local_url, "Travis-Gilbert");

    let (n_before, _) = local.counts().await;
    assert_eq!(n_before, 0, "local must start empty");

    let receipt = bootstrap_from_remote(&l, &r, &handle()).await.unwrap();
    assert_eq!(receipt.remote_nodes_total, 3);
    assert!(receipt.applied);

    let (n_after, _) = local.counts().await;
    assert_eq!(n_after, 3, "all remote nodes applied locally");
}

#[tokio::test]
async fn step_1_3_bootstrap_can_use_paginated_memory_documents() {
    let remote = FakeState::new();
    let local = FakeState::new();
    for idx in 0..501 {
        remote
            .seed_node(&format!("2026-06-27T00:00:{idx:04}Z"))
            .await;
    }

    let remote_url = spawn_fake(remote.clone()).await;
    let local_url = spawn_fake(local.clone()).await;

    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");
    let l = McpClient::unauthenticated(local_url, "Travis-Gilbert");

    let receipt = bootstrap_memory_documents_from_remote(&l, &r, &handle())
        .await
        .unwrap();
    assert_eq!(receipt.remote_nodes_total, 501);
    assert_eq!(receipt.pages, 2);
    assert!(receipt.applied);

    let (n_after, _) = local.counts().await;
    assert_eq!(
        n_after, 501,
        "all paged memory document nodes applied locally"
    );
}

/// Step 4: a remote-side write reaches the local node via the stream tail
/// in a single subscriber poll, no Prolly round required.
#[tokio::test]
async fn step_4_stream_tail_delivers_remote_write_to_local() {
    let remote = FakeState::new();
    let local = FakeState::new();

    let remote_url = spawn_fake(remote.clone()).await;
    let local_url = spawn_fake(local.clone()).await;

    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");
    let l = McpClient::unauthenticated(local_url, "Travis-Gilbert");

    // Simulate a Codex-on-Railway write by injecting a stream event remotely.
    let injected_node = make_node_payload("n:codex");
    remote
        .inject_stream_snapshot(vec![injected_node.clone()], vec![])
        .await;

    let cursors = InMemoryCursorStore::default();
    let status = handle();
    let applied = read_and_apply_once(&l, &r, &cursors, "Travis-Gilbert", &status)
        .await
        .unwrap();

    assert_eq!(applied, 1, "exactly one stream event applied");

    let (n_after, _) = local.counts().await;
    assert_eq!(
        n_after, 1,
        "remote-side write reached local via the stream tail"
    );

    let st = status.get().await;
    assert_eq!(st.stream, StreamState::Connected);
    assert!(
        cursors.load("Travis-Gilbert").unwrap().stream_cursor > 0,
        "cursor advanced"
    );
}

/// Step 5: a local-side mutation reaches Railway via a Prolly round.
#[tokio::test]
async fn step_5_round_pushes_local_write_to_remote() {
    let remote = FakeState::new();
    let local = FakeState::new();
    local.seed_node("n:local-1").await;

    let remote_url = spawn_fake(remote.clone()).await;
    let local_url = spawn_fake(local.clone()).await;

    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");
    let l = McpClient::unauthenticated(local_url, "Travis-Gilbert");

    let (rn_before, _) = remote.counts().await;
    assert_eq!(rn_before, 0);

    let status = handle();
    let _receipt = run_round(&l, &r, &status).await.unwrap();

    let (rn_after, _) = remote.counts().await;
    assert_eq!(
        rn_after, 1,
        "local mutation now visible on remote after one round"
    );
    assert_eq!(
        remote.ref_count(),
        0,
        "round must not call graph_version_ref; it is a full-pack write verb"
    );
}

/// Step 6: when the stream is blocked, status reports `Disconnected` and the
/// round path still converges state. No data loss.
#[tokio::test]
async fn step_6_stream_down_does_not_lose_data() {
    let remote = FakeState::new();
    let local = FakeState::new();
    remote.stream_up.store(false, Ordering::SeqCst);
    local.seed_node("n:offline-1").await;

    let remote_url = spawn_fake(remote.clone()).await;
    let local_url = spawn_fake(local.clone()).await;

    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");
    let l = McpClient::unauthenticated(local_url, "Travis-Gilbert");

    let cursors = InMemoryCursorStore::default();
    let status = handle();

    // Subscriber must fail cleanly when the stream is down.
    let sub_result = read_and_apply_once(&l, &r, &cursors, "Travis-Gilbert", &status).await;
    assert!(sub_result.is_err(), "stream read fails when stream is down");
    let st = status.get().await;
    assert_eq!(st.stream, StreamState::Disconnected);

    // The Prolly round path is independent of the stream; it MUST still converge.
    let _ = run_round(&l, &r, &status).await.unwrap();

    let (rn_after, _) = remote.counts().await;
    assert_eq!(
        rn_after, 1,
        "round converged the local mutation even with stream down"
    );
}

/// Step 7: a local mutation enqueued while the remote is unreachable drains
/// to Railway when the remote comes back. Outbox is durable across that gap.
#[tokio::test]
async fn step_7_outbox_drains_after_offline_window() {
    let remote = FakeState::new();
    remote.remote_up.store(false, Ordering::SeqCst);

    let remote_url = spawn_fake(remote.clone()).await;
    let r = McpClient::unauthenticated(remote_url, "Travis-Gilbert");

    let outbox: Arc<dyn OutboxStore> = Arc::new(InMemoryOutbox::default());

    // Queue an event while the remote is down. The hook would have done this
    // for a real local commit; here we synthesize the queued event directly.
    let event = QueuedOutboxEvent {
        content_hash: "deadbeef0001".to_string(),
        event: rustyred_thg_core::SubstrateSyncEvent {
            tenant: "Travis-Gilbert".to_string(),
            op_kind: "NodeUpsert".to_string(),
            id: "n:queued".to_string(),
            labels: vec!["MemoryDocument".to_string()],
            changed_props: vec!["created_at_ms".to_string()],
            property_delta: json!({"created_at_ms": 1}),
            committed_at_ms: 1,
            hlc: Default::default(),
            content_hash: "deadbeef0001".to_string(),
        },
    };
    outbox.push_event("Travis-Gilbert", event.clone()).unwrap();
    assert_eq!(outbox.len("Travis-Gilbert").unwrap(), 1);

    let status = handle();
    let drainer = OutboxDrainer::new("Travis-Gilbert", r.clone(), outbox.clone(), status.clone());

    // Drain attempt while remote is down: must fail, event must remain.
    let res = drainer.drain_once().await;
    assert!(res.is_err(), "drain fails while remote is down");
    assert_eq!(
        outbox.len("Travis-Gilbert").unwrap(),
        1,
        "queued event must be retained"
    );

    // Bring remote back up. Next drain succeeds.
    remote.remote_up.store(true, Ordering::SeqCst);
    let popped = drainer.drain_once().await.unwrap();
    assert!(
        popped.is_some(),
        "drain succeeded once remote was reachable"
    );
    assert_eq!(
        outbox.len("Travis-Gilbert").unwrap(),
        0,
        "outbox drained after remote recovery"
    );
    let published = remote.publish_count.load(Ordering::SeqCst);
    assert_eq!(published, 1, "exactly one event reached the remote stream");

    let st = status.get().await;
    assert_eq!(st.outbox, OutboxState::Ready);
}

// ----- live mode (gated) -----

/// Helper: build an McpClient against the configured live Railway endpoint
/// using the supplied tenant. Returns `None` if the live-mode gate vars are
/// not set. Callers that need a fresh empty tenant (e.g. the Role 1 round
/// test, which proves the `pack.objects`-omitted wire shape) pass an
/// explicit unique tenant; callers that just need a tenant slot for
/// stream-scoped writes pass `None` to honor `THEOREM_SYNC_TENANT` (default
/// `Travis-Gilbert`).
fn live_remote_client_for(tenant_override: Option<&str>) -> Option<McpClient> {
    if std::env::var("THEOREM_SYNC_LIVE_E2E").as_deref() != Ok("1") {
        return None;
    }
    let url = std::env::var("THEOREM_SYNC_RAILWAY_URL").ok()?;
    let tenant = match tenant_override {
        Some(t) => t.to_string(),
        None => std::env::var("THEOREM_SYNC_TENANT").unwrap_or_else(|_| "Travis-Gilbert".into()),
    };
    Some(match std::env::var("THEOREM_SYNC_RAILWAY_TOKEN").ok() {
        Some(token) if !token.is_empty() => McpClient::new(
            url,
            tenant,
            theorem_substrate_sync::railway_client::TenantToken::Present(token),
        ),
        _ => McpClient::unauthenticated(url, tenant),
    })
}

fn live_remote_client() -> Option<McpClient> {
    live_remote_client_for(None)
}

/// Live smoke for Role 2 (stream-tail freshness). Exercises the same
/// `stream_publish` + `stream_read` verbs the daemon's drainer and
/// subscriber use, against a fresh tenant on rustyredcore-theorem-
/// production. If this passes the daemon CAN reach Railway and round-
/// trip events end-to-end; if it fails the daemon's Role 2 path is
/// broken or the live endpoint is unreachable.
#[tokio::test]
#[ignore = "set THEOREM_SYNC_LIVE_E2E=1 + THEOREM_SYNC_RAILWAY_URL (and optionally THEOREM_SYNC_RAILWAY_TOKEN, THEOREM_SYNC_TENANT) to run"]
async fn live_stream_smoke_against_railway() {
    let Some(remote) = live_remote_client() else {
        eprintln!("skipping: live mode env not configured");
        return;
    };
    let stream_name = format!(
        "tenant:substrate-sync-pt015-smoke-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );

    let publish = remote
        .call_tool(
            "stream_publish",
            json!({
                "stream": stream_name,
                "actor": "theorem-substrate-sync-pt015",
                "kind": "substrate_sync_smoke",
                "urgency": "info",
                "payload": { "hello": "world" }
            }),
        )
        .await
        .expect("live stream_publish must succeed");
    eprintln!("live publish: {publish}");
    assert_eq!(
        publish.get("ok").and_then(Value::as_bool),
        Some(true),
        "stream_publish ok"
    );
    let published_event_id = publish
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .expect("publish returned event_id");

    let read = remote
        .call_tool(
            "stream_read",
            json!({
                "stream": stream_name,
                "actor": "theorem-substrate-sync-pt015",
                "advance": false,
                "limit": 5,
            }),
        )
        .await
        .expect("live stream_read must succeed");
    eprintln!("live read: {read}");
    let events = read
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    assert!(
        events.iter().any(|event| {
            event.get("id").and_then(Value::as_str) == Some(published_event_id.as_str())
        }),
        "the just-published event must be readable back from live"
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Live Role 1 acceptance: drive a full Prolly round
/// (compile -> merge -> apply local -> apply remote) against the real
/// Railway endpoint. This stays behind a separate full-pack opt-in because
/// full graph compile is not a safe heartbeat shape for the production
/// Travis-Gilbert graph.
#[tokio::test]
#[ignore = "set THEOREM_SYNC_LIVE_E2E=1 + THEOREM_SYNC_LIVE_FULL_PACK_E2E=1 + THEOREM_SYNC_RAILWAY_URL to run against a throwaway tenant"]
async fn live_round_against_railway() {
    if std::env::var("THEOREM_SYNC_LIVE_FULL_PACK_E2E").as_deref() != Ok("1") {
        eprintln!("skipping: full-pack live mode not enabled");
        return;
    }
    // Generate a unique throwaway tenant. The regression this test covers
    // (server omitting `pack.objects` via `skip_serializing_if =
    // Vec::is_empty`) only fires on an EMPTY tenant, and we must not
    // upsert the merged snapshot back into shared production data
    // (e.g. `Travis-Gilbert`, 12+GB of real graph state). This makes the
    // test self-contained: it provisions its own slate, proves the
    // empty-pack wire shape, then leaves the slate behind.
    let tenant = format!(
        "substrate-sync-pt015-round-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    let Some(remote) = live_remote_client_for(Some(&tenant)) else {
        eprintln!("skipping: live mode env not configured");
        return;
    };
    eprintln!("live round tenant: {tenant}");

    // Belt-and-suspenders: prove the chosen tenant is empty so the
    // `pack.objects`-omitted wire path is actually exercised. If a future
    // namespacing change leaks a non-empty tenant in here this assertion
    // makes that failure mode loud instead of silently regressing the
    // regression test.
    let bootstrap = remote
        .call_tool(
            "rustyred_thg_graph_version_compile",
            json!({ "include_payloads": false }),
        )
        .await
        .expect("compile against fresh tenant must succeed");
    let manifest = bootstrap
        .get("pack")
        .and_then(|p| p.get("manifest"))
        .expect("bootstrap response must include pack.manifest");
    let nodes_total = manifest
        .get("nodes_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let edges_total = manifest
        .get("edges_total")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    assert_eq!(
        (nodes_total, edges_total),
        (0, 0),
        "fresh tenant {tenant} must report zero nodes/edges (the pack.objects-omitted path); got nodes={nodes_total} edges={edges_total}"
    );

    let local_fake = FakeState::new();
    let local_url = spawn_fake(local_fake.clone()).await;
    let local = McpClient::unauthenticated(local_url, "irrelevant");

    let status = handle();
    let receipt = run_round(&local, &remote, &status)
        .await
        .expect("live round must complete end-to-end");
    eprintln!("live round receipt: {receipt:?}");

    assert!(
        receipt.applied_local,
        "round must mark applied_local=true (snapshot applied to local)"
    );
    assert!(
        receipt.applied_remote,
        "round must mark applied_remote=true (merged snapshot pushed to remote)"
    );
    assert!(!receipt.merged_hash.is_empty(), "merged_hash must be set");

    tokio::time::sleep(Duration::from_millis(50)).await;
}
