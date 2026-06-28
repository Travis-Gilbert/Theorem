//! Integration acceptance for the substrate-backed memory source (roadmap A.3 /
//! SPEC-PROXY-PROVE-AND-PRUNE D1): the proxy retrieves live node memory over the
//! node's MCP `/mcp` endpoint (`hippo_retrieve`), and fails open when the node is
//! absent so a down node never blocks or alters a turn.

use std::net::SocketAddr;

use axum::routing::post;
use axum::Router;
use theorem_proxy::memory::{HttpMemorySource, MemorySource};

async fn spawn(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn injects_ranked_hits_from_a_live_node_over_mcp() {
    // Mock node: /mcp answers a hippo_retrieve tools/call with two candidates.
    let node = Router::new().route(
        "/mcp",
        post(|body: String| async move {
            assert!(body.contains("hippo_retrieve"), "calls the retrieval tool");
            assert!(body.contains("planner"), "forwards the turn query");
            axum::Json(serde_json::json!({
                "jsonrpc": "2.0",
                "id": "theorem-proxy-recall",
                // Real rustyred-thg-server /mcp envelope: payload under structuredContent.
                "result": {
                    "structuredContent": {
                        "candidates": [
                            {"node_id": "mem:planner", "text": "planner.rs does boolean pushdown", "ppr_proximity": 0.91},
                            {"node_id": "mem:cats", "text": "cats are nice", "ppr_proximity": 0.10}
                        ]
                    }
                }
            }))
        }),
    );
    let addr = spawn(node).await;
    let endpoint = format!("http://{addr}/mcp");

    // retrieve() is blocking (sync trait); run it off the async runtime.
    let hits = tokio::task::spawn_blocking(move || {
        HttpMemorySource::new(endpoint, None).retrieve("tell me about the planner", 8)
    })
    .await
    .unwrap();

    assert_eq!(hits.len(), 2, "both candidates returned");
    assert_eq!(hits[0].title, "mem:planner");
    assert!(hits[0].body.contains("pushdown"));
    assert_eq!(hits[0].score, 0.91);
}

#[tokio::test]
async fn fails_open_when_the_node_is_unreachable() {
    // No server on this port (9 = discard): a down node yields no hits, so the turn
    // passes through unchanged.
    let hits = tokio::task::spawn_blocking(|| {
        HttpMemorySource::new("http://127.0.0.1:9/mcp", None).retrieve("anything", 8)
    })
    .await
    .unwrap();
    assert!(hits.is_empty(), "down node fails open to passthrough");
}

#[tokio::test]
async fn memory_request_carries_no_anthropic_credential() {
    // D7 two-token isolation: HttpMemorySource builds its own request to the node, so the
    // client's Anthropic credential (x-api-key / authorization) must never reach it.
    use std::sync::{Arc, Mutex};
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let recorder = seen.clone();
    let node = Router::new().route(
        "/mcp",
        post(move |headers: axum::http::HeaderMap, _body: String| {
            let recorder = recorder.clone();
            async move {
                let mut names = recorder.lock().unwrap();
                for key in headers.keys() {
                    names.push(key.as_str().to_lowercase());
                }
                axum::Json(serde_json::json!({
                    "result": {"structuredContent": {"candidates": []}}
                }))
            }
        }),
    );
    let addr = spawn(node).await;
    let endpoint = format!("http://{addr}/mcp");
    let _ = tokio::task::spawn_blocking(move || {
        HttpMemorySource::new(endpoint, None).retrieve("anything", 3)
    })
    .await
    .unwrap();

    let names = seen.lock().unwrap();
    assert!(
        !names.iter().any(|h| h == "x-api-key" || h == "authorization"),
        "memory-node request must not carry the Anthropic credential; saw: {names:?}"
    );
}

/// End-to-end against a REAL running local node: `rustyred-thg-server` on
/// 127.0.0.1:8380 (scripts/node-local.sh) with a memory encoded for the query. Ignored
/// by default (needs the node up); run with:
///   cargo test --test substrate_memory -- --ignored
#[tokio::test]
#[ignore]
async fn live_local_node_returns_hits() {
    let hits = tokio::task::spawn_blocking(|| {
        HttpMemorySource::new("http://127.0.0.1:8380/mcp", Some("default".to_string()))
            .retrieve("theorem-proxy local node ambient memory hippo_retrieve", 5)
    })
    .await
    .unwrap();
    assert!(
        !hits.is_empty(),
        "live local node returned ambient memory hits"
    );
}
