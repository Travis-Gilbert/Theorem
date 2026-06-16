use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};

use serde_json::{json, Value};

use rustyred_thg_affordances::affordance_nodes;
use rustyred_thg_core::InMemoryGraphStore;

use crate::bridge::{connect_and_register, connect_and_register_with_target};
use crate::transport::{connect_http, ConnectionTarget, McpTransport};
use crate::{ConnectorError, ConnectorResult};

#[derive(Debug)]
struct CapturedRequest {
    headers: Vec<(String, String)>,
    body: String,
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
    Ok(CapturedRequest {
        headers,
        body: String::from_utf8(body).expect("utf8 body"),
    })
}

/// A transport that returns canned results per JSON-RPC method, so the bridge can
/// be tested end-to-end against the real `register_connector` without a live
/// server or a process spawn.
struct FakeTransport {
    responses: HashMap<String, Value>,
    notifications: Vec<String>,
}

impl FakeTransport {
    fn everything_server() -> Self {
        let mut responses = HashMap::new();
        responses.insert(
            "initialize".to_string(),
            json!({
                "protocolVersion": "2025-06-18",
                "serverInfo": { "name": "everything", "version": "1.0.0" }
            }),
        );
        responses.insert(
            "tools/list".to_string(),
            json!({
                "tools": [
                    { "name": "echo", "description": "Echo input", "inputSchema": { "type": "object" } },
                    { "name": "add", "description": "Add two numbers", "inputSchema": { "type": "object" } }
                ]
            }),
        );
        Self {
            responses,
            notifications: Vec::new(),
        }
    }
}

impl McpTransport for FakeTransport {
    fn request(&mut self, method: &str, _params: Value) -> ConnectorResult<Value> {
        self.responses
            .get(method)
            .cloned()
            .ok_or_else(|| ConnectorError::Protocol(format!("no canned response for {method}")))
    }

    fn notify(&mut self, method: &str, _params: Value) -> ConnectorResult<()> {
        self.notifications.push(method.to_string());
        Ok(())
    }
}

#[test]
fn connect_and_register_turns_tools_into_affordance_nodes() {
    let mut store = InMemoryGraphStore::default();
    let mut transport = FakeTransport::everything_server();

    let result = connect_and_register(
        &mut transport,
        &mut store,
        "acme",
        "everything",
        "Everything Server",
        Some("operator"),
    )
    .expect("connect + register");

    // The handshake ran: initialize parsed, initialized notification sent.
    assert_eq!(result.server_info.server_name, "everything");
    assert!(transport
        .notifications
        .iter()
        .any(|m| m == "notifications/initialized"));

    // Both tools registered.
    assert_eq!(result.registration.affordance_node_ids.len(), 2);

    // And they are real, queryable Affordance nodes in the graph, not just a
    // return value: this is the proof that a live tools/list becomes learnable
    // substrate the affordances crate can select over.
    let nodes = affordance_nodes(&store).expect("affordance_nodes");
    let names: Vec<String> = nodes
        .iter()
        .filter_map(|n| {
            n.properties
                .get("tool_name")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .collect();
    assert_eq!(nodes.len(), 2);
    assert!(names.contains(&"echo".to_string()));
    assert!(names.contains(&"add".to_string()));
}

#[test]
fn reregistration_is_idempotent_on_node_ids() {
    let mut store = InMemoryGraphStore::default();

    let mut first_transport = FakeTransport::everything_server();
    let first = connect_and_register(
        &mut first_transport,
        &mut store,
        "acme",
        "everything",
        "Everything",
        None,
    )
    .expect("first register");

    let mut second_transport = FakeTransport::everything_server();
    let second = connect_and_register(
        &mut second_transport,
        &mut store,
        "acme",
        "everything",
        "Everything",
        None,
    )
    .expect("second register");

    assert_eq!(
        first.registration.affordance_node_ids,
        second.registration.affordance_node_ids
    );
    // Re-registering the same server does not multiply nodes.
    let nodes = affordance_nodes(&store).expect("affordance_nodes");
    assert_eq!(nodes.len(), 2);
}

#[test]
fn missing_tools_array_surfaces_a_protocol_error() {
    struct BadTransport;
    impl McpTransport for BadTransport {
        fn request(&mut self, method: &str, _params: Value) -> ConnectorResult<Value> {
            match method {
                "initialize" => Ok(json!({ "serverInfo": { "name": "bad" } })),
                "tools/list" => Ok(json!({ "wrong": [] })),
                other => Err(ConnectorError::Protocol(format!("unexpected {other}"))),
            }
        }
        fn notify(&mut self, _method: &str, _params: Value) -> ConnectorResult<()> {
            Ok(())
        }
    }

    let mut store = InMemoryGraphStore::default();
    let mut transport = BadTransport;
    let err =
        connect_and_register(&mut transport, &mut store, "acme", "bad", "Bad", None).unwrap_err();
    assert!(matches!(err, ConnectorError::Protocol(_)));
}

#[test]
fn http_bridge_completes_json_handshake_and_registers_tools() {
    let responses = vec![
        response(
            "200 OK",
            &[
                ("Content-Type", "application/json"),
                ("Mcp-Session-Id", "sess-bridge"),
            ],
            r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","serverInfo":{"name":"remote","version":"1.0.0"}}}"#,
        ),
        response("202 Accepted", &[], ""),
        response(
            "200 OK",
            &[("Content-Type", "application/json")],
            r#"{"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"echo","description":"Echo input","inputSchema":{"type":"object"}},{"name":"add","description":"Add two numbers","inputSchema":{"type":"object"}}]}}"#,
        ),
    ];
    let (url, handle) = start_stub(responses);
    let target = ConnectionTarget::Http {
        url,
        headers: std::collections::BTreeMap::new(),
        auth: None,
    };
    let mut transport = connect_http(&target).expect("http transport");
    let mut store = InMemoryGraphStore::default();
    let result = connect_and_register_with_target(
        &mut transport,
        Some(&target),
        &mut store,
        "acme",
        "remote",
        "Remote MCP",
        Some("operator"),
    )
    .expect("connect + register over http");
    assert_eq!(result.server_info.server_name, "remote");
    assert_eq!(result.registration.affordance_node_ids.len(), 2);
    assert_eq!(affordance_nodes(&store).expect("affordance nodes").len(), 2);
    let requests = handle.join().expect("server");
    assert_eq!(requests.len(), 3);
    assert!(requests[0].header("Mcp-Session-Id").is_none());
    assert_eq!(requests[1].header("Mcp-Session-Id"), Some("sess-bridge"));
    assert_eq!(requests[2].header("Mcp-Session-Id"), Some("sess-bridge"));
    assert!(requests[1].body.contains("notifications/initialized"));
}
