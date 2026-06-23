//! The connect-and-register bridge: drive an MCP transport through the handshake,
//! list the server's tools, and register them as affordance nodes via
//! `rustyred_thg_affordances::register_connector`. This is the seam the
//! affordances implementation-plan named ("Live MCP tools/list ingestion").

// `register_connector_with_target` is imported via its module path rather than the
// crate-root re-export, so this crate does not depend on a `lib.rs` re-export that
// is being co-edited by concurrent (charter) work; `registry` is a committed
// `pub mod`, so the path is stable.
use rustyred_thg_affordances::registry::register_connector_with_target;
use rustyred_thg_affordances::{AffordanceGraphStore, ConnectorRegisterResult};
use serde_json::{json, to_value};

use crate::protocol::{
    connector_manifest, initialize_params, parse_initialize, parse_tools_list, tools_list_params,
    InitializeInfo,
};
use crate::transport::{connect_transport, ConnectionTarget, McpTransport};
use crate::{ConnectorError, ConnectorResult};

/// Result of connecting to a server and registering its tools.
#[derive(Debug)]
pub struct ConnectAndRegisterResult {
    pub server_info: InitializeInfo,
    pub registration: ConnectorRegisterResult,
}

/// Connect to an MCP server over `transport`, perform the MCP handshake, list its
/// tools, and register each as an `Affordance` node under `(tenant_id, server_id)`.
///
/// Handshake order per the MCP spec: `initialize` request, then the
/// `notifications/initialized` notification, then `tools/list`. Registration uses
/// the existing `register_connector`, so re-running for the same `(tenant, server)`
/// is idempotent and preserves learned fitness/embeddings/outcomes.
pub fn connect_and_register<T: McpTransport, S: AffordanceGraphStore>(
    transport: &mut T,
    store: &mut S,
    tenant_id: &str,
    server_id: &str,
    label: &str,
    actor: Option<&str>,
) -> ConnectorResult<ConnectAndRegisterResult> {
    connect_and_register_with_target(transport, None, store, tenant_id, server_id, label, actor)
}

/// Like `connect_and_register`, but also persist `target` (the reach to this
/// server) on the `Connector` node, so a later selection can re-invoke the
/// server's tools. The invoke bridge (slice 2) requires this: a registered tool
/// whose server reach was never persisted cannot be reached again. `None` leaves
/// any existing persisted target untouched (idempotent re-registration).
pub fn connect_and_register_with_target<T: McpTransport, S: AffordanceGraphStore>(
    transport: &mut T,
    target: Option<&ConnectionTarget>,
    store: &mut S,
    tenant_id: &str,
    server_id: &str,
    label: &str,
    actor: Option<&str>,
) -> ConnectorResult<ConnectAndRegisterResult> {
    let initialize_result = transport.request("initialize", initialize_params())?;
    let server_info = parse_initialize(&initialize_result);
    transport.notify("notifications/initialized", json!({}))?;

    let tools_result = transport.request("tools/list", tools_list_params())?;
    let descriptors = parse_tools_list(&tools_result)?;
    let manifest = connector_manifest(tenant_id, server_id, label, &descriptors);

    let target_value = match target {
        Some(target) => {
            Some(to_value(target).map_err(|e| ConnectorError::Protocol(e.to_string()))?)
        }
        None => None,
    };
    let registration = register_connector_with_target(store, manifest, target_value, actor)
        .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?;

    Ok(ConnectAndRegisterResult {
        server_info,
        registration,
    })
}

/// Connect to a server described by `target` (spawning it over stdio), perform the
/// handshake, register its tools, and persist `target` so they can be invoked
/// later. This is the OS-touching entry that pairs with the invoke bridge: register
/// here, then `invoke::invoke_affordance` reaches the same server again. The
/// in-memory-testable core is `connect_and_register_with_target`.
pub fn connect_target<S: AffordanceGraphStore>(
    target: ConnectionTarget,
    store: &mut S,
    tenant_id: &str,
    server_id: &str,
    label: &str,
    actor: Option<&str>,
) -> ConnectorResult<ConnectAndRegisterResult> {
    let mut transport = connect_transport(&target)?;
    connect_and_register_with_target(
        &mut transport,
        Some(&target),
        store,
        tenant_id,
        server_id,
        label,
        actor,
    )
}
