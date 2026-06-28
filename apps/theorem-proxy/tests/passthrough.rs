//! Deliverable-1 acceptance: the proxy is a faithful passthrough. Proven against a
//! mock upstream (no real Anthropic credential needed to prove the passthrough
//! contract); the full live-session acceptance is a manual Claude Code run.

use std::net::SocketAddr;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use axum::routing::post;
use axum::Router;

async fn spawn(router: Router) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn faithful_passthrough_preserves_headers_status_and_body() {
    // Mock upstream echoes the auth-relevant headers and the raw body, and returns a
    // non-200 status to prove the status code passes through too.
    let upstream = Router::new().route(
        "/v1/messages",
        post(|headers: HeaderMap, body: Bytes| async move {
            let get = |name: &str| {
                headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string()
            };
            (
                StatusCode::ACCEPTED,
                axum::Json(serde_json::json!({
                    "version": get("anthropic-version"),
                    "beta": get("anthropic-beta"),
                    "api_key": get("x-api-key"),
                    "body": String::from_utf8_lossy(&body),
                })),
            )
        }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        upstream: format!("http://{upstream_addr}"),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/messages"))
        .header("anthropic-version", "2023-06-01")
        .header("anthropic-beta", "prompt-caching-2024-07-31,oauth-2025-04-20")
        .header("x-api-key", "sk-ant-test")
        .body("{\"model\":\"claude\",\"messages\":[]}")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status().as_u16(), 202, "status passes through");
    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["version"], "2023-06-01");
    assert_eq!(
        json["beta"],
        "prompt-caching-2024-07-31,oauth-2025-04-20",
        "anthropic-beta (incl. oauth subscription cap) preserved"
    );
    assert_eq!(
        json["api_key"], "sk-ant-test",
        "client credential reaches upstream untouched"
    );
    assert_eq!(json["body"], "{\"model\":\"claude\",\"messages\":[]}");
}

#[tokio::test]
async fn streaming_sse_body_pipes_through_in_order() {
    use futures_util::stream;

    let upstream = Router::new().route(
        "/v1/messages",
        post(|| async {
            let events = vec![
                Ok::<_, std::io::Error>(Bytes::from_static(
                    b"event: message_start\ndata: {\"a\":1}\n\n",
                )),
                Ok(Bytes::from_static(
                    b"event: content_block_delta\ndata: {\"b\":2}\n\n",
                )),
                Ok(Bytes::from_static(b"event: message_stop\ndata: {}\n\n")),
            ];
            Response::builder()
                .header("content-type", "text/event-stream")
                .body(Body::from_stream(stream::iter(events)))
                .unwrap()
        }),
    );
    let upstream_addr = spawn(upstream).await;
    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        upstream: format!("http://{upstream_addr}"),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/messages"))
        .body("{}")
        .send()
        .await
        .unwrap();

    assert_eq!(
        response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok()),
        Some("text/event-stream"),
        "SSE content-type preserved"
    );
    let text = response.text().await.unwrap();
    assert!(text.contains("message_start"));
    assert!(text.contains("content_block_delta"));
    assert!(text.contains("message_stop"));
    assert!(
        text.find("message_start").unwrap() < text.find("message_stop").unwrap(),
        "events arrive in order"
    );
}
