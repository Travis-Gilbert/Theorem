use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use rustyred_thg_core::InMemoryGraphStore;

use crate::bridge::{connect_and_register, connect_and_register_with_target};
use crate::invoke::{
    fire_over_transport, invoke_affordance, plan_invocation, InvokePolicy, InvokeRequest,
};
use crate::transport::{ConnectionTarget, McpTransport};
use crate::{ConnectorError, ConnectorResult};

/// A transport with canned per-method responses, including `tools/call`, so the
/// invoke path runs end-to-end with no process spawn and no real tool execution.
struct FakeServer {
    responses: HashMap<String, Value>,
}

impl FakeServer {
    fn with_tool_call(tool_call_result: Value) -> Self {
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
        responses.insert("tools/call".to_string(), tool_call_result);
        Self { responses }
    }
}

impl McpTransport for FakeServer {
    fn request(&mut self, method: &str, _params: Value) -> ConnectorResult<Value> {
        self.responses
            .get(method)
            .cloned()
            .ok_or_else(|| ConnectorError::Protocol(format!("no canned response for {method}")))
    }

    fn notify(&mut self, _method: &str, _params: Value) -> ConnectorResult<()> {
        Ok(())
    }
}

fn stdio_target() -> ConnectionTarget {
    ConnectionTarget::Stdio {
        command: "npx".to_string(),
        args: vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-everything".to_string(),
        ],
        env: BTreeMap::new(),
    }
}

/// Register the fake server's tools WITH a persisted connection target, so the
/// invoke bridge can resolve a reach back to the server.
fn registered_store() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::default();
    let mut transport = FakeServer::with_tool_call(json!({ "content": [], "isError": false }));
    let target = stdio_target();
    connect_and_register_with_target(
        &mut transport,
        Some(&target),
        &mut store,
        "acme",
        "everything",
        "Everything",
        Some("operator"),
    )
    .expect("register with target");
    store
}

#[test]
fn register_with_target_persists_the_reach_for_planning() {
    let store = registered_store();
    let planned = plan_invocation(&store, "acme", "everything.add", json!({ "a": 2, "b": 3 }))
        .expect("plan resolves a persisted target");
    assert_eq!(planned.tool_name, "add");
    assert_eq!(planned.server_id, "everything");
    // The reach round-trips: serialized into the Connector node on register,
    // deserialized back here, byte-identical.
    assert_eq!(planned.connection_target, stdio_target());
}

#[test]
fn dry_run_is_the_default_and_fires_nothing() {
    let mut store = registered_store();
    let report = invoke_affordance(
        &mut store,
        InvokeRequest {
            tenant_id: "acme".to_string(),
            task_type: "math".to_string(),
            affordance_id: "everything.add".to_string(),
            arguments: json!({ "a": 2, "b": 3 }),
            candidate_affordance_ids: vec![
                "everything.add".to_string(),
                "everything.echo".to_string(),
            ],
        },
        &InvokePolicy::default(),
        Some("operator"),
    )
    .expect("dry-run invoke");
    assert!(!report.fired, "the default policy must not fire a live tool");
    assert!(report.outcome.is_none());
    assert!(report.recorded.is_none());
    assert!(report.dry_run_reason.is_some());
    // The plan still resolved: we know exactly what we WOULD have called.
    assert_eq!(report.planned.tool_name, "add");
}

#[test]
fn fire_over_transport_records_a_real_outcome_and_moves_fitness() {
    let mut store = registered_store();
    let planned = plan_invocation(&store, "acme", "everything.add", json!({ "a": 2, "b": 3 }))
        .expect("plan");
    let mut transport = FakeServer::with_tool_call(json!({
        "content": [ { "type": "text", "text": "5" } ],
        "isError": false
    }));
    let (outcome, recorded) = fire_over_transport(
        &mut transport,
        &mut store,
        "acme",
        "math",
        &planned,
        vec!["everything.add".to_string(), "everything.echo".to_string()],
        Some("operator"),
    )
    .expect("fire");
    assert!(!outcome.is_error);
    assert_eq!(outcome.text, "5");
    // A positive tool outcome lifts effective fitness above the 0.5 base: the
    // learning half of the loop fired. This is what makes the next selection rank
    // `add` higher for this task than an untried sibling. The compounding property,
    // now fed by a real (fake-transport) tool result instead of a hand-written one.
    assert!(
        recorded.effective_fitness > 0.5,
        "a positive tool outcome must raise the affordance's fitness, got {}",
        recorded.effective_fitness
    );
}

#[test]
fn allowlist_fires_only_named_affordances() {
    let mut store = registered_store();
    // `everything.add` is invoked, but only `everything.echo` is allowlisted, so
    // `add` must still dry-run: the operator named exactly which tools may execute.
    let report = invoke_affordance(
        &mut store,
        InvokeRequest {
            tenant_id: "acme".to_string(),
            task_type: "math".to_string(),
            affordance_id: "everything.add".to_string(),
            arguments: json!({}),
            candidate_affordance_ids: vec!["everything.add".to_string()],
        },
        &InvokePolicy::FireAllowlist(vec!["everything.echo".to_string()]),
        Some("operator"),
    )
    .expect("invoke");
    assert!(!report.fired, "add is not on the allowlist, so it must dry-run");
    assert!(report.recorded.is_none());
}

#[test]
fn cannot_invoke_without_a_persisted_target() {
    // Register WITHOUT a target (the plain path), then planning must refuse: a tool
    // whose server reach was never persisted cannot be invoked.
    let mut store = InMemoryGraphStore::default();
    let mut transport = FakeServer::with_tool_call(json!({ "content": [], "isError": false }));
    connect_and_register(
        &mut transport,
        &mut store,
        "acme",
        "everything",
        "Everything",
        None,
    )
    .expect("register without target");
    let err = plan_invocation(&store, "acme", "everything.add", json!({})).unwrap_err();
    assert!(matches!(err, ConnectorError::Transport(_)));
}
