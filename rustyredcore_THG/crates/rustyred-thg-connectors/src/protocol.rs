//! Pure MCP JSON-RPC protocol: request param builders, response parsers, and the
//! ToolDescriptor -> ToolManifest -> ConnectorManifest mapping. No I/O; every
//! function is a value-in / value-out transform, fully unit-testable. The
//! transport layer owns the JSON-RPC envelope (jsonrpc/id/method) and id
//! correlation; this module owns the MCP-specific payload shapes.

use serde_json::{json, Value};

use rustyred_thg_affordances::{ConnectorManifest, ToolManifest};

use crate::{ConnectorError, ConnectorResult};

/// The MCP protocol version this client advertises in `initialize`.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// `initialize` request params (our client identity + capabilities).
pub fn initialize_params() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {},
        "clientInfo": {
            "name": "rustyred-thg-connectors",
            "version": env!("CARGO_PKG_VERSION"),
        }
    })
}

/// `tools/list` request params. Slice 1 reads a single page (no cursor).
pub fn tools_list_params() -> Value {
    json!({})
}

/// `tools/call` request params (used by the deferred invoke slice).
pub fn tools_call_params(name: &str, arguments: Value) -> Value {
    json!({ "name": name, "arguments": arguments })
}

/// Lightweight server identity from an `initialize` result.
#[derive(Clone, Debug, PartialEq)]
pub struct InitializeInfo {
    pub server_name: String,
    pub server_version: String,
    pub protocol_version: String,
}

/// Parse an `initialize` result into the server identity. Tolerant of missing
/// fields (returns empty strings), since `serverInfo` is informational.
pub fn parse_initialize(result: &Value) -> InitializeInfo {
    let server = result.get("serverInfo");
    InitializeInfo {
        server_name: server
            .and_then(|s| s.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        server_version: server
            .and_then(|s| s.get("version"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        protocol_version: result
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    }
}

/// One tool as described by an MCP server's `tools/list` (MCP uses camelCase
/// `inputSchema`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Parse a `tools/list` result (`{ "tools": [ {name, description, inputSchema} ] }`)
/// into descriptors. Tolerant of missing description/inputSchema; skips entries
/// with no non-empty `name` (a tool with no name is unaddressable).
pub fn parse_tools_list(result: &Value) -> ConnectorResult<Vec<ToolDescriptor>> {
    let tools = result
        .get("tools")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ConnectorError::Protocol("tools/list result missing `tools` array".into())
        })?;
    let mut out = Vec::with_capacity(tools.len());
    for tool in tools {
        let name = tool
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() {
            continue;
        }
        let description = tool
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let input_schema = tool.get("inputSchema").cloned().unwrap_or_else(|| json!({}));
        out.push(ToolDescriptor {
            name,
            description,
            input_schema,
        });
    }
    Ok(out)
}

/// Map one descriptor to the affordances crate's `ToolManifest`. `label` defaults
/// to the tool name; `description_embedding` is `None` because RustyRed core has
/// no Rust text embedder (the affordances plan's "Text embedder" seam), so
/// selection degrades to structural PPR until an embedder fills it.
pub fn tool_manifest_from_descriptor(descriptor: &ToolDescriptor) -> ToolManifest {
    ToolManifest {
        name: descriptor.name.clone(),
        label: descriptor.name.clone(),
        description: descriptor.description.clone(),
        input_schema: descriptor.input_schema.clone(),
        permissions: Vec::new(),
        cost: json!({}),
        writeback_policy: String::new(),
        tags: Vec::new(),
        description_embedding: None,
    }
}

/// Assemble a `ConnectorManifest` (the contract boundary into the affordance
/// registry) from a server's tool catalog. No change to the affordances crate:
/// this produces exactly what `register_connector` already consumes.
pub fn connector_manifest(
    tenant_id: &str,
    server_id: &str,
    label: &str,
    descriptors: &[ToolDescriptor],
) -> ConnectorManifest {
    ConnectorManifest {
        tenant_id: tenant_id.to_string(),
        server_id: server_id.to_string(),
        label: label.to_string(),
        tools: descriptors.iter().map(tool_manifest_from_descriptor).collect(),
    }
}

/// Outcome of a `tools/call` (used by the deferred invoke slice). Concatenates
/// the text content blocks; `is_error` reflects the MCP `isError` flag.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallOutcome {
    pub is_error: bool,
    pub text: String,
}

pub fn parse_tool_call_result(result: &Value) -> ToolCallOutcome {
    let is_error = result
        .get("isError")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let text = result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    ToolCallOutcome { is_error, text }
}
