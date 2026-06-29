//! Invoke bridge: call a selected affordance's tool on its owning MCP server and
//! record the real outcome, closing the loop register -> select -> invoke -> learn.
//!
//! Dry-run by default. `InvokePolicy::DryRun` (the default) never sends a
//! `tools/call`: it resolves the planned call and reports it, recording nothing,
//! so the loop can be exercised with zero side effects. Live firing is opt-in via
//! `InvokePolicy::FireAllowlist`, which fires only affordances the operator names
//! explicitly (human-in-the-loop).
//!
//! A writeback-policy-keyed auto-fire gate is deliberately NOT offered yet: tools
//! registered from a live `tools/list` default their `writeback_policy` to
//! "read-only" (the MCP catalog carries no side-effect annotation that the
//! registration path extracts), so keying auto-fire off it would fire
//! side-effecting tools. Extracting MCP `readOnlyHint`/`destructiveHint` is the
//! follow-up that would make a writeback-keyed gate safe.

// `connector_connection_target` is imported via its module path rather than the
// crate-root re-export, so this crate does not depend on a `lib.rs` re-export that
// is being co-edited by concurrent (charter) work; `registry` is a committed
// `pub mod`, so the path is stable.
use rustyred_plugin::PluginHost;
use rustyred_thg_affordances::registry::connector_connection_target;
use rustyred_thg_affordances::{
    affordance_node_id, record_invocation, Affordance, AffordanceGraphStore,
    InvocationRecordRequest, InvocationRecordResult,
};
use serde_json::{json, Value};

use crate::protocol::{
    initialize_params, parse_tool_call_result, tools_call_params, ToolCallOutcome,
};
use crate::transport::{connect_transport, ConnectionTarget, McpTransport};
use crate::{ConnectorError, ConnectorResult};

/// When a selected tool may actually fire. The default never fires.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum InvokePolicy {
    /// Never send `tools/call`. Resolve and report the planned call; record
    /// nothing. The safe default: the loop runs with zero side effects.
    #[default]
    DryRun,
    /// Fire only affordances whose id is on this explicit allowlist; everything
    /// else falls back to dry-run. The operator names exactly which tools may
    /// execute, so nothing side-effecting fires by surprise.
    FireAllowlist(Vec<String>),
}

/// The inputs to an invoke attempt, bundled (mirrors `SelectionRequest` and
/// `InvocationRecordRequest`): which affordance to invoke for which task, the call
/// arguments, and the candidate set considered at selection time (recorded with
/// the outcome so the training corpus carries the alternatives, not just the pick).
#[derive(Clone, Debug)]
pub struct InvokeRequest {
    pub tenant_id: String,
    pub task_type: String,
    pub affordance_id: String,
    pub arguments: Value,
    pub candidate_affordance_ids: Vec<String>,
}

/// A selected affordance resolved to a concrete call: which tool, on which server,
/// reachable how. Produced by store reads only; no contact with the server.
#[derive(Clone, Debug)]
pub struct PlannedInvocation {
    pub affordance_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub writeback_policy: String,
    pub connection_target: ConnectionTarget,
    pub arguments: Value,
}

impl InvokePolicy {
    fn may_fire(&self, planned: &PlannedInvocation) -> bool {
        match self {
            InvokePolicy::DryRun => false,
            InvokePolicy::FireAllowlist(ids) => ids.iter().any(|id| id == &planned.affordance_id),
        }
    }
}

/// What happened on an invoke attempt. `fired` is false for any dry-run (the
/// policy withheld firing); `outcome` and `recorded` are present only when the
/// tool actually fired and its result was recorded.
#[derive(Debug)]
pub struct InvokeReport {
    pub planned: PlannedInvocation,
    pub fired: bool,
    pub dry_run_reason: Option<String>,
    pub outcome: Option<ToolCallOutcome>,
    pub recorded: Option<InvocationRecordResult>,
}

/// Resolve a selected affordance into a concrete planned call. Reads the
/// `Affordance` node for its tool/server/writeback policy and the owning
/// `Connector` node for the persisted reach. Errors if the affordance is not
/// registered or has no persisted connection target (you cannot invoke what you
/// cannot reach: re-register the server with a target first).
pub fn plan_invocation<S: AffordanceGraphStore>(
    store: &S,
    tenant_id: &str,
    affordance_id: &str,
    arguments: Value,
) -> ConnectorResult<PlannedInvocation> {
    let node_id = affordance_node_id(tenant_id, affordance_id);
    let node = store
        .get_node(&node_id)
        .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?
        .ok_or_else(|| {
            ConnectorError::Registration(format!("affordance {affordance_id} is not registered"))
        })?;
    let affordance = Affordance::from_node_record(&node)
        .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?;
    let target_value = connector_connection_target(store, tenant_id, &affordance.server_id)
        .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?
        .ok_or_else(|| {
            ConnectorError::Transport(format!(
                "no connection target persisted for server {}; re-register it with a target before invoking",
                affordance.server_id
            ))
        })?;
    let connection_target: ConnectionTarget = serde_json::from_value(target_value)
        .map_err(|e| ConnectorError::Protocol(format!("connection_target decode: {e}")))?;
    Ok(PlannedInvocation {
        affordance_id: affordance.affordance_id,
        server_id: affordance.server_id,
        tool_name: affordance.tool_name,
        writeback_policy: affordance.writeback_policy,
        connection_target,
        arguments,
    })
}

/// Fire a planned call over an already-handshaken transport, parse the outcome,
/// and record it as an invocation (feeding fitness + the training corpus). This is
/// the learning half of the loop; it is tested over a fake transport so no process
/// spawns and no real tool executes.
pub fn fire_over_transport<T: McpTransport, S: AffordanceGraphStore>(
    transport: &mut T,
    store: &mut S,
    tenant_id: &str,
    task_type: &str,
    planned: &PlannedInvocation,
    candidate_affordance_ids: Vec<String>,
    actor: Option<&str>,
) -> ConnectorResult<(ToolCallOutcome, InvocationRecordResult)> {
    let result = transport.request(
        "tools/call",
        tools_call_params(&planned.tool_name, planned.arguments.clone()),
    )?;
    let outcome = parse_tool_call_result(&result);
    let outcome_value = if outcome.is_error { 0.0 } else { 1.0 };
    let recorded = record_invocation(
        store,
        InvocationRecordRequest {
            tenant_id: tenant_id.to_string(),
            task_type: task_type.to_string(),
            candidate_affordance_ids,
            selected_affordance_id: planned.affordance_id.clone(),
            outcome_value,
            outcome_weight: 1.0,
            outcome_label: if outcome.is_error {
                "tool_error".to_string()
            } else {
                "tool_ok".to_string()
            },
            previous_affordance_id: None,
            query_text: String::new(),
            recorded_at_ms: None,
        },
        actor,
    )
    .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?;
    Ok((outcome, recorded))
}

/// The full invoke bridge: plan the call, apply the gate, and (only if the policy
/// permits) spawn the server, handshake, fire, and record. With the default
/// `InvokePolicy::DryRun` nothing is spawned or sent and the report carries the
/// planned call only. The OS-touching spawn lives here; the fire + record logic is
/// `fire_over_transport`, exercised over a fake transport in tests.
pub fn invoke_affordance<S: AffordanceGraphStore>(
    store: &mut S,
    req: InvokeRequest,
    policy: &InvokePolicy,
    actor: Option<&str>,
) -> ConnectorResult<InvokeReport> {
    let planned = plan_invocation(
        store,
        &req.tenant_id,
        &req.affordance_id,
        req.arguments.clone(),
    )?;
    if !policy.may_fire(&planned) {
        return Ok(InvokeReport {
            planned,
            fired: false,
            dry_run_reason: Some(
                "invoke policy withheld live firing (dry-run); no tools/call sent".to_string(),
            ),
            outcome: None,
            recorded: None,
        });
    }
    if planned.connection_target.wasm_plugin_spec().is_some() {
        let (outcome, recorded) = fire_rustyred_plugin(
            store,
            &req.tenant_id,
            &req.task_type,
            &planned,
            req.candidate_affordance_ids,
            actor,
        )?;
        return Ok(InvokeReport {
            planned,
            fired: true,
            dry_run_reason: None,
            outcome: Some(outcome),
            recorded: Some(recorded),
        });
    }
    let mut transport = connect_transport(&planned.connection_target)?;
    transport.request("initialize", initialize_params())?;
    transport.notify("notifications/initialized", json!({}))?;
    let (outcome, recorded) = fire_over_transport(
        &mut transport,
        store,
        &req.tenant_id,
        &req.task_type,
        &planned,
        req.candidate_affordance_ids,
        actor,
    )?;
    Ok(InvokeReport {
        planned,
        fired: true,
        dry_run_reason: None,
        outcome: Some(outcome),
        recorded: Some(recorded),
    })
}

fn fire_rustyred_plugin<S: AffordanceGraphStore>(
    store: &mut S,
    tenant_id: &str,
    task_type: &str,
    planned: &PlannedInvocation,
    candidate_affordance_ids: Vec<String>,
    actor: Option<&str>,
) -> ConnectorResult<(ToolCallOutcome, InvocationRecordResult)> {
    let spec = planned
        .connection_target
        .wasm_plugin_spec()
        .ok_or_else(|| {
            ConnectorError::Transport(
                "planned invocation is not a rustyred_plugin target".to_string(),
            )
        })?;
    let input = serde_json::to_string(&planned.arguments)
        .map_err(|error| ConnectorError::Protocol(error.to_string()))?;
    let mut plugin = PluginHost::new()
        .load(spec)
        .map_err(|error| ConnectorError::Transport(error.to_string()))?;
    let outcome = match plugin.invoke(&planned.tool_name, &input) {
        Ok(text) => ToolCallOutcome {
            is_error: false,
            text,
        },
        Err(error) => ToolCallOutcome {
            is_error: true,
            text: error.to_string(),
        },
    };
    let recorded = record_invocation(
        store,
        InvocationRecordRequest {
            tenant_id: tenant_id.to_string(),
            task_type: task_type.to_string(),
            candidate_affordance_ids,
            selected_affordance_id: planned.affordance_id.clone(),
            outcome_value: if outcome.is_error { 0.0 } else { 1.0 },
            outcome_weight: 1.0,
            outcome_label: if outcome.is_error {
                "plugin_error".to_string()
            } else {
                "plugin_ok".to_string()
            },
            previous_affordance_id: None,
            query_text: String::new(),
            recorded_at_ms: None,
        },
        actor,
    )
    .map_err(|error| ConnectorError::Registration(format!("{error:?}")))?;
    Ok((outcome, recorded))
}
