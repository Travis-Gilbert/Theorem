//! Acceptance for `theorem-proxy doctor` (roadmap C.3 chain-check): each link is probed
//! independently and reported up or down, never fatal.

use std::net::SocketAddr;

use axum::routing::{get, post};
use axum::Router;
use theorem_proxy::doctor;

async fn spawn(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn doctor_reports_node_and_memory_up() {
    let node = Router::new()
        .route("/ready", get(|| async { "ready" }))
        .route(
            "/mcp",
            post(|| async {
                axum::Json(serde_json::json!({
                    "result": {"structuredContent": {"candidates": []}}
                }))
            }),
        );
    let addr = spawn(node).await;
    let memory_url = format!("http://{addr}/mcp");

    let checks = doctor(Some(&memory_url), None, None).await;
    let node = checks.iter().find(|c| c.name == "node").unwrap();
    let memory = checks.iter().find(|c| c.name == "memory").unwrap();
    assert!(node.ok, "node up: {}", node.detail);
    assert!(memory.ok, "memory retrieval up: {}", memory.detail);
}

#[tokio::test]
async fn doctor_reports_down_when_nothing_listens() {
    // Port 9 (discard) refuses fast: every probed link reports down, none panics.
    let checks = doctor(
        Some("http://127.0.0.1:9/mcp"),
        Some("http://127.0.0.1:9"),
        Some("127.0.0.1:9"),
    )
    .await;
    assert!(!checks.is_empty());
    assert!(
        checks.iter().all(|c| !c.ok),
        "all links down: {:?}",
        checks
    );
}
