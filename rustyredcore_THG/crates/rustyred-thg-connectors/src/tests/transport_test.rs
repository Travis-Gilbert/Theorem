use std::io::Cursor;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};

use serde_json::{json, Value};

use crate::transport::{
    connect_transport, read_sse_response, spawn_stdio, ConnectedTransport, ConnectionTarget,
    ConnectorAuth, McpTransport, StdioTransport, JSONRPC_VERSION,
};
use crate::ConnectorError;

#[derive(Debug)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
}

impl CapturedRequest {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

fn response(status: &str, headers: &[(&str, &str)], body: &str) -> String {
    let mut out = format!(
        "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (key, value) in headers {
        out.push_str(key);
        out.push_str(": ");
        out.push_str(value);
        out.push_str("\r\n");
    }
    out.push_str("\r\n");
    out.push_str(body);
    out
}

fn start_stub(responses: Vec<String>) -> (String, JoinHandle<Vec<CapturedRequest>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let url = format!("http://{}", listener.local_addr().expect("local addr"));
    let handle = thread::spawn(move || {
        let mut captured = Vec::new();
        for expected_response in responses {
            let (mut stream, _) = listener.accept().expect("accept");
            let request = read_request(&mut stream).expect("read request");
            stream
                .write_all(expected_response.as_bytes())
                .expect("write response");
            captured.push(request);
        }
        captured
    });
    (url, handle)
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<CapturedRequest> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut first = String::new();
    reader.read_line(&mut first)?;
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            let value = value.trim().to_string();
            if key.eq_ignore_ascii_case("content-length") {
                content_length = value.parse().unwrap_or(0);
            }
            headers.push((key.to_string(), value));
        }
    }
    let mut body = vec![0; content_length];
    reader.read_exact(&mut body)?;
    let _ = String::from_utf8(body).expect("utf8 body");
    Ok(CapturedRequest { headers })
}

/// A transport whose reader is pre-loaded with newline-framed response lines and
/// whose writer is an in-memory buffer. No process spawn: the framing and id
/// correlation are exercised over `Cursor`.
fn transport_with_responses(lines: &[Value]) -> StdioTransport<Cursor<Vec<u8>>, Vec<u8>> {
    let mut buf = String::new();
    for line in lines {
        buf.push_str(&serde_json::to_string(line).unwrap());
        buf.push('\n');
    }
    StdioTransport::new(Cursor::new(buf.into_bytes()), Vec::new())
}

#[test]
fn request_correlates_response_by_id_and_returns_result() {
    // The first request is assigned id = 1; provide a matching response.
    let mut t = transport_with_responses(&[json!({
        "jsonrpc": "2.0", "id": 1, "result": { "ok": true }
    })]);
    let result = t.request("ping", json!({})).expect("request");
    assert_eq!(result, json!({ "ok": true }));
}

#[test]
fn request_skips_notifications_and_mismatched_ids() {
    let mut t = transport_with_responses(&[
        json!({ "jsonrpc": "2.0", "method": "notifications/message", "params": { "level": "info" } }),
        json!({ "jsonrpc": "2.0", "id": 99, "result": { "stale": true } }),
        json!({ "jsonrpc": "2.0", "id": 1, "result": { "ok": true } }),
    ]);
    let result = t.request("tools/list", json!({})).expect("request");
    assert_eq!(result, json!({ "ok": true }));
}

#[test]
fn jsonrpc_error_maps_to_connector_error() {
    let mut t = transport_with_responses(&[json!({
        "jsonrpc": "2.0", "id": 1,
        "error": { "code": -32601, "message": "method not found" }
    })]);
    let err = t.request("bogus", json!({})).unwrap_err();
    match err {
        ConnectorError::Rpc { code, message } => {
            assert_eq!(code, -32601);
            assert_eq!(message, "method not found");
        }
        other => panic!("expected Rpc error, got {other:?}"),
    }
}

#[test]
fn closed_stream_before_response_is_a_transport_error() {
    let mut t = transport_with_responses(&[]); // empty reader
    let err = t.request("ping", json!({})).unwrap_err();
    assert!(matches!(err, ConnectorError::Transport(_)));
}

#[test]
fn writes_framed_requests_with_incrementing_ids() {
    let mut t = transport_with_responses(&[
        json!({ "jsonrpc": "2.0", "id": 1, "result": {} }),
        json!({ "jsonrpc": "2.0", "id": 2, "result": {} }),
    ]);
    t.request("first", json!({ "a": 1 })).unwrap();
    t.request("second", json!({})).unwrap();

    let written = String::from_utf8(t.writer().clone()).unwrap();
    let lines: Vec<&str> = written.lines().collect();
    assert_eq!(lines.len(), 2);

    let first: Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(first["jsonrpc"], json!(JSONRPC_VERSION));
    assert_eq!(first["id"], json!(1));
    assert_eq!(first["method"], json!("first"));
    assert_eq!(first["params"]["a"], json!(1));

    let second: Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(second["id"], json!(2));
    assert_eq!(second["method"], json!("second"));
}

#[test]
fn notify_writes_a_request_without_an_id() {
    let mut t = transport_with_responses(&[]);
    t.notify("notifications/initialized", json!({})).unwrap();
    let written = String::from_utf8(t.writer().clone()).unwrap();
    let value: Value = serde_json::from_str(written.trim()).unwrap();
    assert_eq!(value["method"], json!("notifications/initialized"));
    assert!(value.get("id").is_none());
}

#[test]
fn http_target_round_trips_with_bearer_auth() {
    let mut headers = std::collections::BTreeMap::new();
    headers.insert("X-Connector".to_string(), "github".to_string());
    let target = ConnectionTarget::Http {
        url: "https://example.com/mcp".to_string(),
        headers,
        auth: Some(ConnectorAuth::Bearer {
            token: "token-123".to_string(),
        }),
    };
    let encoded = serde_json::to_value(&target).expect("serialize");
    assert_eq!(encoded["transport"], json!("http"));
    assert_eq!(encoded["auth"]["kind"], json!("bearer"));
    let decoded: ConnectionTarget = serde_json::from_value(encoded).expect("deserialize");
    assert_eq!(decoded, target);
}

#[test]
fn sse_parser_skips_notifications_and_mismatched_ids() {
    let body = concat!(
        "event: message\n",
        "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n",
        "\n",
        "data: {\"jsonrpc\":\"2.0\",\"id\":99,\"result\":{\"stale\":true}}\n",
        "\n",
        "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ok\":true}}\n",
        "\n"
    );
    let message = read_sse_response(Cursor::new(body.as_bytes()), 1).expect("sse response");
    assert_eq!(message["result"], json!({ "ok": true }));
}

#[test]
fn http_request_reads_sse_response_and_sends_session_id_next_time() {
    let sse = concat!(
        "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n",
        "\n",
        "data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"ready\":true}}\n",
        "\n"
    );
    let responses = vec![
        response(
            "200 OK",
            &[
                ("Content-Type", "text/event-stream"),
                ("Mcp-Session-Id", "sess-1"),
            ],
            sse,
        ),
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            r#"{"jsonrpc":"2.0","id":2,"result":{"listed":true}}"#,
        ),
    ];
    let (url, handle) = start_stub(responses);
    let target = ConnectionTarget::Http {
        url,
        headers: std::collections::BTreeMap::new(),
        auth: None,
    };
    let ConnectedTransport::Http(mut transport) = connect_transport(&target).expect("http") else {
        panic!("expected http transport");
    };
    assert_eq!(
        transport.request("initialize", json!({})).expect("first"),
        json!({ "ready": true })
    );
    assert_eq!(transport.session_id(), Some("sess-1"));
    assert_eq!(
        transport.request("tools/list", json!({})).expect("second"),
        json!({ "listed": true })
    );
    let requests = handle.join().expect("server");
    assert!(requests[0].header("Mcp-Session-Id").is_none());
    assert_eq!(requests[1].header("Mcp-Session-Id"), Some("sess-1"));
}

#[test]
fn http_jsonrpc_error_maps_to_rpc_error() {
    let responses = vec![response(
        "200 OK",
        &[("Content-Type", "application/json")],
        r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"boom"}}"#,
    )];
    let (url, handle) = start_stub(responses);
    let target = ConnectionTarget::Http {
        url,
        headers: std::collections::BTreeMap::new(),
        auth: None,
    };
    let ConnectedTransport::Http(mut transport) = connect_transport(&target).expect("http") else {
        panic!("expected http transport");
    };
    let err = transport.request("initialize", json!({})).unwrap_err();
    match err {
        ConnectorError::Rpc { code, message } => {
            assert_eq!(code, -32000);
            assert_eq!(message, "boom");
        }
        other => panic!("expected rpc error, got {other:?}"),
    }
    handle.join().expect("server");
}

#[test]
fn http_non_success_status_maps_to_transport_error() {
    let responses = vec![response(
        "500 Internal Server Error",
        &[("Content-Type", "text/plain")],
        "nope",
    )];
    let (url, handle) = start_stub(responses);
    let target = ConnectionTarget::Http {
        url,
        headers: std::collections::BTreeMap::new(),
        auth: None,
    };
    let ConnectedTransport::Http(mut transport) = connect_transport(&target).expect("http") else {
        panic!("expected http transport");
    };
    assert!(matches!(
        transport.request("initialize", json!({})).unwrap_err(),
        ConnectorError::Transport(_)
    ));
    handle.join().expect("server");
}

#[test]
fn connect_transport_dispatches_http_and_stdio() {
    let http_target = ConnectionTarget::Http {
        url: "http://127.0.0.1:9/mcp".to_string(),
        headers: std::collections::BTreeMap::new(),
        auth: None,
    };
    assert!(matches!(
        connect_transport(&http_target).expect("http dispatch"),
        ConnectedTransport::Http(_)
    ));

    let stdio_target = ConnectionTarget::Stdio {
        command: "cat".to_string(),
        args: Vec::new(),
        env: std::collections::BTreeMap::new(),
    };
    assert!(matches!(
        connect_transport(&stdio_target).expect("stdio dispatch"),
        ConnectedTransport::Stdio(_)
    ));
    let err = match spawn_stdio(&http_target) {
        Ok(_) => panic!("http target must not spawn stdio"),
        Err(err) => err,
    };
    assert!(matches!(err, ConnectorError::Transport(_)));
}
