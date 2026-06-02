//! Live MCP connector transport.
//!
//! The outbound mirror of `rustyred-thg-mcp` (the inbound adapter that exposes
//! the graph as MCP tools): connect to an *external* MCP server, perform the
//! handshake, list its tools, and feed them through
//! `rustyred_thg_affordances::register_connector` so each tool becomes a
//! learnable `Affordance` graph node. This is the transport half the affordance
//! layer needs to carry real connectors instead of hand-fed manifests.
//!
//! Sync and tokio-free, matching every sibling crate. MCP stdio framing is
//! newline-delimited JSON over a child process's stdin/stdout, so the protocol
//! layer (`protocol`) is pure and the transport (`transport`) is a thin
//! `BufRead + Write` shell. Plan: docs/plans/mcp-learning-layer/connector-transport-plan.md.

pub mod bridge;
pub mod invoke;
pub mod protocol;
pub mod transport;

pub use bridge::{
    connect_and_register, connect_and_register_with_target, connect_target,
    ConnectAndRegisterResult,
};
pub use invoke::{
    fire_over_transport, invoke_affordance, plan_invocation, InvokePolicy, InvokeReport,
    InvokeRequest, PlannedInvocation,
};
pub use protocol::{
    connector_manifest, initialize_params, parse_initialize, parse_tool_call_result,
    parse_tools_list, tool_manifest_from_descriptor, tools_call_params, tools_list_params,
    InitializeInfo, ToolCallOutcome, ToolDescriptor,
};
pub use transport::{spawn_stdio, ConnectionTarget, McpTransport, StdioTransport};

use std::fmt;

/// Errors from the connector transport: protocol decode failures, JSON-RPC error
/// responses from the server, transport I/O, and registration failures.
#[derive(Debug)]
pub enum ConnectorError {
    /// Malformed or unexpected JSON-RPC / MCP payload.
    Protocol(String),
    /// The server returned a JSON-RPC error object.
    Rpc { code: i64, message: String },
    /// Transport-level I/O (process spawn, read/write, stream closed).
    Transport(String),
    /// Registration into the affordance graph failed.
    Registration(String),
}

impl fmt::Display for ConnectorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConnectorError::Protocol(m) => write!(f, "protocol error: {m}"),
            ConnectorError::Rpc { code, message } => write!(f, "rpc error {code}: {message}"),
            ConnectorError::Transport(m) => write!(f, "transport error: {m}"),
            ConnectorError::Registration(m) => write!(f, "registration error: {m}"),
        }
    }
}

impl std::error::Error for ConnectorError {}

pub type ConnectorResult<T> = Result<T, ConnectorError>;

#[cfg(test)]
#[path = "tests/protocol_test.rs"]
mod protocol_test;

#[cfg(test)]
#[path = "tests/transport_test.rs"]
mod transport_test;

#[cfg(test)]
#[path = "tests/bridge_test.rs"]
mod bridge_test;

#[cfg(test)]
#[path = "tests/invoke_test.rs"]
mod invoke_test;
