use std::collections::HashMap;

use serde_json::{json, Value};

use rustyred_thg_affordances::affordance_nodes;
use rustyred_thg_core::InMemoryGraphStore;

use crate::bridge::connect_and_register;
use crate::transport::McpTransport;
use crate::{ConnectorError, ConnectorResult};

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
