//! Acceptance for the D2 native-tool membrane through the live proxy: an oversized
//! tool_result reaches upstream truncated, and the full content is retrievable
//! out-of-band at /tool_result/{id}.

use std::net::SocketAddr;

use axum::routing::post;
use axum::Router;
use theorem_proxy::ProxyConfig;

async fn spawn(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn membrane_truncates_to_upstream_and_serves_full_out_of_band() {
    // Upstream echoes the (membraned) body it received.
    let upstream = Router::new().route(
        "/v1/messages",
        post(|body: String| async move { axum::Json(serde_json::json!({ "received": body })) }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(ProxyConfig {
        upstream: format!("http://{upstream_addr}"),
        membrane_threshold: 500,
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let big = "Z".repeat(4000);
    let request = serde_json::json!({
        "model": "claude",
        "messages": [
            {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t", "content": big}
            ]}
        ]
    });
    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/messages"))
        .body(serde_json::to_vec(&request).unwrap())
        .send()
        .await
        .unwrap();
    let value: serde_json::Value = response.json().await.unwrap();
    let received = value["received"].as_str().unwrap();

    assert!(
        received.contains("theorem-membrane"),
        "upstream saw the truncated tool_result"
    );
    assert!(
        received.len() < big.len() + 600,
        "the oversized result was shrunk, not forwarded whole"
    );

    // Pull the retrieval id out of the stub marker (.../tool_result/<id>]) and fetch it.
    let id = received
        .split("/tool_result/")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap()
        .trim()
        .to_string();
    let full = reqwest::Client::new()
        .get(format!("http://{proxy_addr}/tool_result/{id}"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert_eq!(full, big, "full tool output is retrievable out-of-band");
}
