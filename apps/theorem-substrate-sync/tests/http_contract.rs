use serde_json::json;
use theorem_substrate_sync::railway_client::{McpClient, TenantToken};
use theorem_substrate_sync::status::{serve_status, StatusHandle, SyncStatus};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

#[tokio::test]
async fn mcp_client_sends_bearer_authorization_header() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test server");
    let addr = listener.local_addr().expect("local addr");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut buf = vec![0u8; 8192];
        let n = socket.read(&mut buf).await.expect("read request");
        let request = String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase();
        assert!(request.contains("authorization: bearer tenant-token"));
        assert!(request.contains("\"name\":\"stream_publish\""));
        let body = r#"{"jsonrpc":"2.0","id":"test","result":{"structuredContent":{"ok":true}}}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
            body.len(),
            body
        );
        socket
            .write_all(response.as_bytes())
            .await
            .expect("write response");
    });

    let client = McpClient::new(
        format!("http://{addr}"),
        "Travis-Gilbert",
        TenantToken::Present("tenant-token".to_string()),
    );

    let response = client
        .call_tool("stream_publish", json!({"stream": "tenant:Travis-Gilbert"}))
        .await
        .expect("tool call");

    assert_eq!(response["ok"], true);
    server.await.expect("server task");
}

#[tokio::test]
async fn status_endpoint_reports_default_disabled_state() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind probe");
    let addr = listener.local_addr().expect("local addr");
    drop(listener);
    let status = StatusHandle::new(SyncStatus::new(false, "Travis-Gilbert", 30_000));
    let (tx, _rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(serve_status(addr, status, tx));

    let url = format!("http://{addr}/status");
    let mut response = None;
    for _ in 0..50 {
        match reqwest::get(&url).await {
            Ok(ok) => {
                response = Some(ok);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(20)).await,
        }
    }
    let response: serde_json::Value = response
        .expect("GET /status")
        .json()
        .await
        .expect("status json");

    assert_eq!(response["connected"], false);
    assert_eq!(response["sync_enabled"], false);
    assert_eq!(response["tenant"], "Travis-Gilbert");
    server.abort();
}
