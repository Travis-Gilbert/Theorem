//! A live MCP record transport (Layer A5, the `Mcp` contract variant made real).
//!
//! An MCP server becomes an ingestion *source*: a read-tool's output maps to
//! `Item`s. The spec phrases it "an MCP server's resources or read tools as the
//! record source"; this uses a read-tool via `tools/call`, which
//! `rustyred-thg-connectors` already speaks, so no new MCP protocol is needed.
//! This is the inbound mirror of the C2 act seam's outbound `invoke_affordance`:
//! same server, two roles (records in here, action tools fired there).

use rustyred_thg_connectors::{
    connect_transport, initialize_params, parse_tool_call_result, tools_call_params,
    ConnectionTarget, ConnectorError, ConnectorResult, McpTransport,
};
use serde_json::{json, Value};

use crate::mapped::{field_i64, field_str};
use crate::spoke::{SourceCursor, SourceError, SourcePage, SourceRecord, SourceResult, SourceScope};
use crate::transport::RecordTransport;

type Connector = dyn Fn() -> ConnectorResult<Box<dyn McpTransport>>;

/// A [`RecordTransport`] backed by a live MCP server. Each fetch opens a
/// connection (the connectors stdio/HTTP transport), handshakes, calls one
/// read-tool with the scope + cursor as arguments, and parses the tool's JSON
/// result into a page. The server does the scoping; its read-tool returns either
/// a bare JSON array of records or a `{records, next, exhausted}` page object.
pub struct McpRecordTransport {
    connect: Box<Connector>,
    tool_name: String,
    id_field: String,
}

impl McpRecordTransport {
    /// Connect to a real MCP server (stdio subprocess or HTTP) and pull from
    /// `tool_name`. The credential/reach rides the `ConnectionTarget`, exactly as
    /// the federated-MCP act side persists it on the `Connector` node.
    pub fn connect(target: ConnectionTarget, tool_name: impl Into<String>) -> Self {
        Self {
            connect: Box::new(move || {
                connect_transport(&target).map(|t| Box::new(t) as Box<dyn McpTransport>)
            }),
            tool_name: tool_name.into(),
            id_field: "id".to_string(),
        }
    }

    /// Build over an injected connector, so a fake MCP transport can drive the
    /// full connect -> handshake -> call -> parse path without spawning a process.
    pub fn with_connector<F>(tool_name: impl Into<String>, connect: F) -> Self
    where
        F: Fn() -> ConnectorResult<Box<dyn McpTransport>> + 'static,
    {
        Self {
            connect: Box::new(connect),
            tool_name: tool_name.into(),
            id_field: "id".to_string(),
        }
    }

    /// The field on each record holding its stable external id (default `"id"`).
    pub fn id_field(mut self, field: impl Into<String>) -> Self {
        self.id_field = field.into();
        self
    }

    fn page_from_payload(&self, payload: Value, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let (raw_records, next_token, exhausted) = match payload {
            Value::Array(arr) => (arr, String::new(), true),
            Value::Object(map) => {
                let records = map
                    .get("records")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let next = map.get("next").and_then(Value::as_str).unwrap_or("").to_string();
                let exhausted = map.get("exhausted").and_then(Value::as_bool).unwrap_or(true);
                (records, next, exhausted)
            }
            _ => {
                return Err(SourceError::Mapping(
                    "mcp tool result must be a JSON array or a {records,next,exhausted} object"
                        .into(),
                ))
            }
        };
        let mut records = Vec::with_capacity(raw_records.len());
        for raw in raw_records {
            let external_id = field_str(&raw, &self.id_field).ok_or_else(|| {
                SourceError::Mapping(format!("mcp record missing id field `{}`", self.id_field))
            })?;
            let fetched_at_ms = field_i64(&raw, "updated_at_ms").unwrap_or(0);
            records.push(SourceRecord::new(external_id, raw, fetched_at_ms));
        }
        Ok(SourcePage {
            records,
            next: SourceCursor {
                token: next_token,
                updated_at_ms: cursor.updated_at_ms,
            },
            exhausted,
        })
    }
}

impl RecordTransport for McpRecordTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        // ponytail: one connection per page (a stdio target re-spawns the server
        // per page). Fine for a delta pull; hold an interior-mutable connection if
        // a busy source makes the per-page handshake cost matter.
        let mut transport = (self.connect)().map_err(conn_err)?;
        transport
            .request("initialize", initialize_params())
            .map_err(conn_err)?;
        transport
            .notify("notifications/initialized", json!({}))
            .map_err(conn_err)?;

        let args = json!({
            "containers": scope.containers,
            "since_ms": scope.since_ms,
            "max_records": scope.max_records,
            "filters": scope.filters,
            "cursor": cursor.token,
        });
        let result = transport
            .request("tools/call", tools_call_params(&self.tool_name, args))
            .map_err(conn_err)?;
        let outcome = parse_tool_call_result(&result);
        if outcome.is_error {
            return Err(SourceError::Transport(format!(
                "mcp tool `{}` returned isError: {}",
                self.tool_name, outcome.text
            )));
        }
        let payload: Value = serde_json::from_str(&outcome.text)
            .map_err(|e| SourceError::Mapping(format!("mcp tool result is not JSON: {e}")))?;
        self.page_from_payload(payload, cursor)
    }
}

fn conn_err(error: ConnectorError) -> SourceError {
    SourceError::Transport(error.to_string())
}
