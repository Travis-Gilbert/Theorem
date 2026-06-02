use std::io::Cursor;

use serde_json::{json, Value};

use crate::transport::{McpTransport, StdioTransport, JSONRPC_VERSION};
use crate::ConnectorError;

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
