//! Deliverable-1 acceptance: the proxy is a faithful passthrough. Proven against a
//! mock upstream (no real Anthropic credential needed to prove the passthrough
//! contract); the full live-session acceptance is a manual Claude Code run.

use std::net::SocketAddr;
use std::sync::Arc;

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
        .header(
            "anthropic-beta",
            "prompt-caching-2024-07-31,oauth-2025-04-20",
        )
        .header("x-api-key", "sk-ant-test")
        .body("{\"model\":\"claude\",\"messages\":[]}")
        .send()
        .await
        .unwrap();

    assert_eq!(response.status().as_u16(), 202, "status passes through");
    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["version"], "2023-06-01");
    assert_eq!(
        json["beta"], "prompt-caching-2024-07-31,oauth-2025-04-20",
        "anthropic-beta (incl. oauth subscription cap) preserved"
    );
    assert_eq!(
        json["api_key"], "sk-ant-test",
        "client credential reaches upstream untouched"
    );
    assert_eq!(json["body"], "{\"model\":\"claude\",\"messages\":[]}");
}

#[tokio::test]
async fn upstream_auth_override_strips_client_gateway_key() {
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
            axum::Json(serde_json::json!({
                "authorization": get("authorization"),
                "api_key": get("x-api-key"),
                "beta": get("anthropic-beta"),
                "body": String::from_utf8_lossy(&body),
            }))
        }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        upstream: format!("http://{upstream_addr}"),
        upstream_auth: Some(theorem_proxy::UpstreamAuth::ApiKey(
            "sk-ant-upstream".to_string(),
        )),
        upstream_beta: Some("oauth-2025-04-20".to_string()),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/messages"))
        .header("authorization", "Bearer local-desktop-key")
        .body("{\"model\":\"claude\",\"messages\":[]}")
        .send()
        .await
        .unwrap();

    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["authorization"], "", "client bearer is stripped");
    assert_eq!(
        json["api_key"], "sk-ant-upstream",
        "proxy-owned upstream API key is applied"
    );
    assert_eq!(json["beta"], "oauth-2025-04-20");
    assert_eq!(json["body"], "{\"model\":\"claude\",\"messages\":[]}");
}

#[tokio::test]
async fn model_discovery_path_forwards_with_upstream_auth() {
    let upstream = Router::new().route(
        "/v1/models",
        axum::routing::get(|headers: HeaderMap| async move {
            let api_key = headers
                .get("x-api-key")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            axum::Json(serde_json::json!({
                "api_key": api_key,
                "data": [{"id": "claude-sonnet-4-6", "type": "model"}]
            }))
        }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        upstream: format!("http://{upstream_addr}"),
        upstream_auth: Some(theorem_proxy::UpstreamAuth::ApiKey(
            "sk-ant-upstream".to_string(),
        )),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .get(format!("http://{proxy_addr}/v1/models"))
        .header("authorization", "Bearer local-desktop-key")
        .send()
        .await
        .unwrap();

    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["api_key"], "sk-ant-upstream");
    assert_eq!(json["data"][0]["id"], "claude-sonnet-4-6");
}

#[tokio::test]
async fn openai_responses_route_injects_memory_and_preserves_bearer() {
    let upstream = Router::new().route(
        "/v1/responses",
        post(|headers: HeaderMap, body: Bytes| async move {
            let authorization = headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_string();
            axum::Json(serde_json::json!({
                "authorization": authorization,
                "body": serde_json::from_slice::<serde_json::Value>(&body).unwrap()
            }))
        }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        openai_upstream: format!("http://{upstream_addr}"),
        memory: Some(Arc::new(theorem_proxy::memory::VecMemorySource::new(vec![
            ("planner", "planner.rs pushdown"),
        ]))),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/responses"))
        .header("authorization", "Bearer sk-openai-client")
        .json(&serde_json::json!({
            "model": "gpt-5.5",
            "tools": [{"type": "function", "name": "shell"}],
            "input": [
                {"role": "system", "content": [{"type": "input_text", "text": "SYSTEM"}]},
                {"role": "user", "content": [{"type": "input_text", "text": "tell me about planner pushdown"}]}
            ]
        }))
        .send()
        .await
        .unwrap();

    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["authorization"], "Bearer sk-openai-client");
    assert_eq!(json["body"]["tools"][0]["name"], "shell");
    assert_eq!(json["body"]["input"][0]["role"], "system");
    let last = serde_json::to_string(&json["body"]["input"][1]).unwrap();
    assert!(last.contains("tell me about planner pushdown"));
    assert!(last.contains("planner.rs pushdown"));
    assert!(last.contains("theorem-memory"));
}

#[tokio::test]
async fn openai_auth_override_strips_client_key() {
    let upstream = Router::new().route(
        "/v1/responses",
        post(|headers: HeaderMap, body: Bytes| async move {
            let get = |name: &str| {
                headers
                    .get(name)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string()
            };
            axum::Json(serde_json::json!({
                "authorization": get("authorization"),
                "api_key": get("x-api-key"),
                "body": String::from_utf8_lossy(&body),
            }))
        }),
    );
    let upstream_addr = spawn(upstream).await;

    let proxy = theorem_proxy::router(theorem_proxy::ProxyConfig {
        openai_upstream: format!("http://{upstream_addr}"),
        openai_upstream_api_key: Some("sk-openai-upstream".to_string()),
        ..Default::default()
    });
    let proxy_addr = spawn(proxy).await;

    let response = reqwest::Client::new()
        .post(format!("http://{proxy_addr}/v1/responses"))
        .header("authorization", "Bearer local-codex-key")
        .body("{\"model\":\"gpt-5.5\",\"input\":\"hello\"}")
        .send()
        .await
        .unwrap();

    let json: serde_json::Value = response.json().await.unwrap();
    assert_eq!(json["authorization"], "Bearer sk-openai-upstream");
    assert_eq!(json["api_key"], "");
    assert_eq!(json["body"], "{\"model\":\"gpt-5.5\",\"input\":\"hello\"}");
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
