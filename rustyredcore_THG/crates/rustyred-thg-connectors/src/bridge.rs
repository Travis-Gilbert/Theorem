//! The connect-and-register bridge: drive an MCP transport through the handshake,
//! list the server's tools, and register them as affordance nodes via
//! `rustyred_thg_affordances::register_connector`. This is the seam the
//! affordances implementation-plan named ("Live MCP tools/list ingestion").

use rustyred_thg_affordances::{
    register_connector, AffordanceGraphStore, ConnectorRegisterResult,
};
use serde_json::json;

use crate::protocol::{
    connector_manifest, initialize_params, parse_initialize, parse_tools_list, tools_list_params,
    InitializeInfo,
};
use crate::transport::McpTransport;
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
    let initialize_result = transport.request("initialize", initialize_params())?;
    let server_info = parse_initialize(&initialize_result);
    transport.notify("notifications/initialized", json!({}))?;

    let tools_result = transport.request("tools/list", tools_list_params())?;
    let descriptors = parse_tools_list(&tools_result)?;
    let manifest = connector_manifest(tenant_id, server_id, label, &descriptors);

    let registration = register_connector(store, manifest, actor)
        .map_err(|e| ConnectorError::Registration(format!("{e:?}")))?;

    Ok(ConnectAndRegisterResult {
        server_info,
        registration,
    })
}
