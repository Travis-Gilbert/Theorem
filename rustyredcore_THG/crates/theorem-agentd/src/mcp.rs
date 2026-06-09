use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use crate::config::McpServerConfig;
use crate::tools::ToolCatalog;
use crate::{AgentdError, AgentdResult};

pub struct McpRouter {
    clients: BTreeMap<String, McpClient>,
}

impl McpRouter {
    pub fn from_configs(configs: Vec<McpServerConfig>) -> AgentdResult<Self> {
        let mut clients = BTreeMap::new();
        for config in configs {
            let name = config.name.clone();
            clients.insert(name, McpClient::new(config)?);
        }
        Ok(Self { clients })
    }

    pub fn call_tool(
        &self,
        catalog: &ToolCatalog,
        name: &str,
        arguments: Value,
    ) -> AgentdResult<Value> {
        catalog.validate_call(name, &arguments)?;
        let definition = catalog
            .get(name)
            .ok_or_else(|| AgentdError::Tool(format!("unknown tool '{name}'")))?;
        let client = self.clients.get(&definition.server).ok_or_else(|| {
            AgentdError::Mcp(format!(
                "tool {name} routes to missing MCP server '{}'",
                definition.server
            ))
        })?;
        client.call_tool(name, arguments)
    }

    pub fn best_effort_call(&self, name: &str, arguments: Value) -> Value {
        let Some(client) = self.clients.get("harness") else {
            return json!({"unavailable": "harness MCP server is not configured"});
        };
        match client.call_tool(name, arguments) {
            Ok(value) => value,
            Err(error) => json!({"unavailable": error.to_string()}),
        }
    }
}

pub struct McpClient {
    http: reqwest::blocking::Client,
    url: String,
    token: Option<String>,
    tenant_slug: String,
    next_id: AtomicU64,
    /// When true, route calls through the MCP streamable-HTTP session protocol
    /// with SSE response parsing (see `call_tool_session`).
    session: bool,
    /// Cached MCP session id from the `initialize` response header, established
    /// lazily on the first call to a session server.
    session_id: Mutex<Option<String>>,
}

impl McpClient {
    pub fn new(config: McpServerConfig) -> AgentdResult<Self> {
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(120))
            .build()
            .map_err(AgentdError::from)?;
        let token = config
            .token_env
            .as_deref()
            .and_then(|env| std::env::var(env).ok())
            .filter(|token| !token.trim().is_empty());
        Ok(Self {
            http,
            url: config.url,
            token,
            tenant_slug: config.tenant_slug,
            next_id: AtomicU64::new(1),
            session: config.session,
            session_id: Mutex::new(None),
        })
    }

    pub fn call_tool(&self, name: &str, mut arguments: Value) -> AgentdResult<Value> {
        if self.session {
            return self.call_tool_session(name, arguments);
        }
        if let Value::Object(map) = &mut arguments {
            map.entry("tenant_slug".to_string())
                .or_insert_with(|| json!(self.tenant_slug));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        });
        let mut request = self.http.post(&self.url).json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        let value: Value = response.json()?;
        parse_tool_response(&value)
    }

    /// Call a tool on a session-mode MCP server: establish the MCP session if
    /// needed, POST tools/call with the `Mcp-Session-Id` header, and parse the
    /// SSE (or JSON) response into the standard MCP `result`. No tenant_slug is
    /// injected here (that is a harness-only convention).
    fn call_tool_session(&self, name: &str, arguments: Value) -> AgentdResult<Value> {
        let session_id = self.ensure_session()?;
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": name,
                "arguments": arguments
            }
        });
        let mut request = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", session_id.as_str())
            .json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        let envelope = read_mcp_response(response)?;
        mcp_result(&envelope)
    }

    /// Establish (once) and return the MCP session id for this server: POST
    /// `initialize`, read the `mcp-session-id` response header, then POST the
    /// `notifications/initialized` acknowledgement. Cached for reuse.
    fn ensure_session(&self) -> AgentdResult<String> {
        if let Some(existing) = self.session_id.lock().unwrap().clone() {
            return Ok(existing);
        }
        let init_id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let init_body = json!({
            "jsonrpc": "2.0",
            "id": init_id,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": {"name": "theorem-agentd", "version": env!("CARGO_PKG_VERSION")}
            }
        });
        let mut request = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .json(&init_body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|value| value.to_str().ok())
            .map(|value| value.to_string())
            .ok_or_else(|| {
                AgentdError::Mcp("initialize response missing mcp-session-id header".to_string())
            })?;

        let mut ack = self
            .http
            .post(&self.url)
            .header("Accept", "application/json, text/event-stream")
            .header("Mcp-Session-Id", session_id.as_str())
            .json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
        if let Some(token) = &self.token {
            ack = ack.bearer_auth(token);
        }
        ack.send()?.error_for_status()?;

        *self.session_id.lock().unwrap() = Some(session_id.clone());
        Ok(session_id)
    }
}

pub fn parse_tool_response(value: &Value) -> AgentdResult<Value> {
    if let Some(error) = value.get("error") {
        return Err(AgentdError::Mcp(format!("jsonrpc error: {error}")));
    }
    let text = value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(|content| content.get(0))
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| AgentdError::Mcp("response missing result.content[0].text".to_string()))?;
    // Some harness tool outputs embed raw control characters (e.g. unescaped
    // newlines inside record bodies), which strict JSON rejects. Fall back to
    // handing back the raw text so the turn stays usable instead of erroring.
    let payload: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(_) => return Ok(json!({ "text": text })),
    };
    if let Some(error) = payload.get("error") {
        return Err(AgentdError::Mcp(format!("tool error: {error}")));
    }
    // Harness tools may wrap the backend value in {result: ...} or return it
    // directly (e.g. coordinate returns a receipt with no `result` key). Unwrap
    // when wrapped, otherwise hand back the payload as-is.
    Ok(payload.get("result").cloned().unwrap_or(payload))
}

/// Read an MCP HTTP response body as the JSON-RPC envelope, handling both
/// `application/json` and `text/event-stream` (SSE) content types.
pub fn read_mcp_response(response: reqwest::blocking::Response) -> AgentdResult<Value> {
    let is_sse = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|content_type| content_type.contains("text/event-stream"))
        .unwrap_or(false);
    let body = response.text()?;
    if is_sse {
        parse_sse_envelope(&body)
    } else {
        serde_json::from_str(&body).map_err(AgentdError::from)
    }
}

/// Extract the JSON-RPC envelope from an SSE stream. Servers emit one
/// `data: <json>` payload per event; prefer the event carrying `result` or
/// `error` (the response) over interim notifications.
pub fn parse_sse_envelope(body: &str) -> AgentdResult<Value> {
    let mut chosen: Option<Value> = None;
    for line in body.lines() {
        let Some(rest) = line.strip_prefix("data:") else {
            continue;
        };
        let chunk = rest.trim();
        if chunk.is_empty() {
            continue;
        }
        if let Ok(value) = serde_json::from_str::<Value>(chunk) {
            let is_response = value.get("result").is_some() || value.get("error").is_some();
            if is_response || chosen.is_none() {
                chosen = Some(value);
            }
        }
    }
    chosen.ok_or_else(|| AgentdError::Mcp("SSE response carried no JSON data line".to_string()))
}

/// Interpret a standard MCP JSON-RPC envelope: surface JSON-RPC errors and
/// tool-level `isError` results, otherwise return the `result` object.
pub fn mcp_result(envelope: &Value) -> AgentdResult<Value> {
    if let Some(error) = envelope.get("error") {
        return Err(AgentdError::Mcp(format!("jsonrpc error: {error}")));
    }
    let result = envelope
        .get("result")
        .cloned()
        .ok_or_else(|| AgentdError::Mcp("response missing result envelope".to_string()))?;
    if result.get("isError").and_then(Value::as_bool) == Some(true) {
        let text = result
            .get("content")
            .and_then(|content| content.get(0))
            .and_then(|entry| entry.get("text"))
            .and_then(Value::as_str)
            .unwrap_or("tool reported isError");
        return Err(AgentdError::Mcp(format!("tool error: {text}")));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mcp_tool_response() {
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "content": [{
                    "type": "text",
                    "text": json!({"tenant":"default","result":{"ok":true}}).to_string()
                }]
            }
        });
        let payload = parse_tool_response(&envelope).unwrap();
        assert_eq!(payload, json!({"ok": true}));
    }

    #[test]
    fn parses_sse_envelope_prefers_the_response_event() {
        let body = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/message\",\"params\":{}}\n\nevent: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"ok\"}]}}\n\n";
        let envelope = parse_sse_envelope(body).unwrap();
        let result = mcp_result(&envelope).unwrap();
        assert_eq!(result["content"][0]["text"], "ok");
    }

    #[test]
    fn mcp_result_surfaces_tool_is_error() {
        let envelope = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {"isError": true, "content": [{"type": "text", "text": "missing arg"}]}
        });
        let error = mcp_result(&envelope).unwrap_err();
        assert!(error.to_string().contains("missing arg"));
    }

    // Live network proof of the MCP streamable-HTTP session transport against the
    // real TickTick MCP. Run with: `cargo test -p theorem-agentd -- --ignored`.
    #[test]
    #[ignore = "live network: hits the real TickTick MCP at ticktick-mcp-production"]
    fn live_ticktick_session_round_trip() {
        let config = McpServerConfig {
            name: "ticktick".to_string(),
            url: "https://ticktick-mcp-production-8a84.up.railway.app/mcp".to_string(),
            token_env: None,
            tenant_slug: "default".to_string(),
            session: true,
        };
        let client = McpClient::new(config).unwrap();
        let outcome = client.call_tool("ticktick_list_projects", json!({"params": {}}));
        let summary = match &outcome {
            Ok(value) => format!("ok; content_present={}", value.get("content").is_some()),
            Err(error) => error.to_string(),
        };
        println!("live ticktick round trip: {summary}");
        // The transport (initialize -> session -> initialized -> tools/call over
        // SSE) must complete: an OK result or a tool-level error are both fine,
        // but never a session/transport failure.
        assert!(
            !summary.contains("Missing session ID") && !summary.starts_with("http error"),
            "MCP session transport did not complete: {summary}"
        );
    }
}
