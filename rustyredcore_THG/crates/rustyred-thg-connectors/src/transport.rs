//! MCP transport: a synchronous `McpTransport` trait (request/notify with
//! JSON-RPC id correlation), a `StdioTransport` over newline-delimited JSON, and
//! a blocking HTTP/SSE transport for remote MCP servers.
//! The framing is generic over `BufRead + Write` so it tests over in-memory
//! buffers; only the constructors touch the OS or network client.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{ConnectorError, ConnectorResult};

pub const JSONRPC_VERSION: &str = "2.0";

/// How to reach an external MCP server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum ConnectionTarget {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
        #[serde(default)]
        auth: Option<ConnectorAuth>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ConnectorAuth {
    Bearer { token: String },
}

/// A synchronous MCP transport. `request` sends a JSON-RPC request and returns
/// the `result` (mapping a JSON-RPC `error` to `ConnectorError::Rpc`); `notify`
/// sends a notification (no id, no response expected).
pub trait McpTransport {
    fn request(&mut self, method: &str, params: Value) -> ConnectorResult<Value>;
    fn notify(&mut self, method: &str, params: Value) -> ConnectorResult<()>;
}

fn result_from_response(response: &Value) -> ConnectorResult<Value> {
    if let Some(error) = response.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error")
            .to_string();
        return Err(ConnectorError::Rpc { code, message });
    }
    Ok(response.get("result").cloned().unwrap_or(Value::Null))
}

/// Newline-delimited JSON-RPC over any reader/writer pair. Generic so tests drive
/// it over `Cursor`; `spawn_stdio` wires it to a child process's stdout/stdin.
pub struct StdioTransport<R: BufRead, W: Write> {
    reader: R,
    writer: W,
    next_id: i64,
    /// Held to keep the spawned server process alive for the transport's lifetime
    /// (and reaped on drop). `None` for in-memory (test) transports.
    child: Option<Child>,
}

impl<R: BufRead, W: Write> StdioTransport<R, W> {
    pub fn new(reader: R, writer: W) -> Self {
        Self {
            reader,
            writer,
            next_id: 1,
            child: None,
        }
    }

    /// Borrow what has been written so far (used by tests to assert framing).
    pub fn writer(&self) -> &W {
        &self.writer
    }

    fn write_message(&mut self, message: &Value) -> ConnectorResult<()> {
        let line =
            serde_json::to_string(message).map_err(|e| ConnectorError::Protocol(e.to_string()))?;
        self.writer
            .write_all(line.as_bytes())
            .and_then(|_| self.writer.write_all(b"\n"))
            .and_then(|_| self.writer.flush())
            .map_err(|e| ConnectorError::Transport(e.to_string()))
    }

    /// Read newline-framed JSON values until one is a response with `id == want`.
    /// Server-initiated notifications (no id) and responses to other ids are
    /// skipped, so an interleaved log notification does not derail the handshake.
    fn read_response(&mut self, want: i64) -> ConnectorResult<Value> {
        let mut line = String::new();
        loop {
            line.clear();
            let read = self
                .reader
                .read_line(&mut line)
                .map_err(|e| ConnectorError::Transport(e.to_string()))?;
            if read == 0 {
                return Err(ConnectorError::Transport(
                    "stream closed before a matching response".into(),
                ));
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed)
                .map_err(|e| ConnectorError::Protocol(e.to_string()))?;
            match value.get("id").and_then(Value::as_i64) {
                Some(id) if id == want => return Ok(value),
                _ => continue,
            }
        }
    }
}

impl<R: BufRead, W: Write> McpTransport for StdioTransport<R, W> {
    fn request(&mut self, method: &str, params: Value) -> ConnectorResult<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_message(&request)?;
        let response = self.read_response(id)?;
        result_from_response(&response)
    }

    fn notify(&mut self, method: &str, params: Value) -> ConnectorResult<()> {
        let notification = json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": params,
        });
        self.write_message(&notification)
    }
}

impl<R: BufRead, W: Write> Drop for StdioTransport<R, W> {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Spawn an MCP server subprocess and wire its stdio into a transport. This is
/// the only function that touches the OS; the framing and correlation above are
/// tested over in-memory buffers. `stderr` is inherited so the server's logs are
/// visible during development.
pub fn spawn_stdio(
    target: &ConnectionTarget,
) -> ConnectorResult<StdioTransport<BufReader<ChildStdout>, ChildStdin>> {
    let ConnectionTarget::Stdio { command, args, env } = target else {
        return Err(ConnectorError::Transport(
            "spawn_stdio called with a non-stdio connection target".to_string(),
        ));
    };
    let mut cmd = Command::new(command);
    cmd.args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| ConnectorError::Transport(format!("spawn {command}: {e}")))?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| ConnectorError::Transport("child stdin unavailable".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ConnectorError::Transport("child stdout unavailable".into()))?;
    let mut transport = StdioTransport::new(BufReader::new(stdout), stdin);
    transport.child = Some(child);
    Ok(transport)
}

pub struct HttpTransport {
    endpoint: String,
    headers: BTreeMap<String, String>,
    session_id: Option<String>,
    next_id: i64,
    agent: ureq::Agent,
}

struct HttpResponsePayload {
    mime: String,
    body: String,
}

impl HttpTransport {
    pub fn new(endpoint: String, headers: BTreeMap<String, String>) -> Self {
        Self {
            endpoint,
            headers,
            session_id: None,
            next_id: 1,
            agent: ureq::agent(),
        }
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    fn post_message(&mut self, message: &Value) -> ConnectorResult<HttpResponsePayload> {
        let body =
            serde_json::to_string(message).map_err(|e| ConnectorError::Protocol(e.to_string()))?;
        let mut request = self
            .agent
            .post(&self.endpoint)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream");
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }
        if let Some(session_id) = &self.session_id {
            request = request.header("Mcp-Session-Id", session_id);
        }
        let mut response = request.send(body.as_bytes()).map_err(|e| match e {
            ureq::Error::StatusCode(code) => {
                ConnectorError::Transport(format!("http status {code}"))
            }
            other => ConnectorError::Transport(other.to_string()),
        })?;
        if let Some(session_id) = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|value| value.to_str().ok())
        {
            self.session_id = Some(session_id.to_string());
        }
        let mime = response.body().mime_type().unwrap_or("").to_string();
        let body = response
            .body_mut()
            .read_to_string()
            .map_err(|e| ConnectorError::Transport(e.to_string()))?;
        Ok(HttpResponsePayload { mime, body })
    }
}

impl McpTransport for HttpTransport {
    fn request(&mut self, method: &str, params: Value) -> ConnectorResult<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        });
        let response = self.post_message(&request)?;
        let message = if response.mime.eq_ignore_ascii_case("text/event-stream") {
            read_sse_response(BufReader::new(response.body.as_bytes()), id)?
        } else {
            serde_json::from_str(&response.body)
                .map_err(|e| ConnectorError::Protocol(e.to_string()))?
        };
        match message.get("id").and_then(Value::as_i64) {
            Some(response_id) if response_id == id => result_from_response(&message),
            _ => Err(ConnectorError::Transport(
                "http response did not contain the matching JSON-RPC id".to_string(),
            )),
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> ConnectorResult<()> {
        let notification = json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": params,
        });
        let _ = self.post_message(&notification)?;
        Ok(())
    }
}

pub fn read_sse_response<R: BufRead>(mut reader: R, want: i64) -> ConnectorResult<Value> {
    let mut line = String::new();
    let mut data = String::new();
    loop {
        line.clear();
        let read = reader
            .read_line(&mut line)
            .map_err(|e| ConnectorError::Transport(e.to_string()))?;
        if read == 0 {
            if !data.trim().is_empty() {
                if let Some(message) = parse_sse_event(&data, want)? {
                    return Ok(message);
                }
            }
            return Err(ConnectorError::Transport(
                "event stream closed before a matching response".to_string(),
            ));
        }

        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            if !data.trim().is_empty() {
                if let Some(message) = parse_sse_event(&data, want)? {
                    return Ok(message);
                }
                data.clear();
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            if !data.is_empty() {
                data.push('\n');
            }
            data.push_str(rest.strip_prefix(' ').unwrap_or(rest));
        }
    }
}

fn parse_sse_event(data: &str, want: i64) -> ConnectorResult<Option<Value>> {
    let value: Value =
        serde_json::from_str(data).map_err(|e| ConnectorError::Protocol(e.to_string()))?;
    match value.get("id").and_then(Value::as_i64) {
        Some(id) if id == want => Ok(Some(value)),
        _ => Ok(None),
    }
}

pub fn connect_http(target: &ConnectionTarget) -> ConnectorResult<HttpTransport> {
    let ConnectionTarget::Http { url, headers, auth } = target else {
        return Err(ConnectorError::Transport(
            "connect_http called with a non-http connection target".to_string(),
        ));
    };
    let mut resolved_headers = headers.clone();
    if let Some(ConnectorAuth::Bearer { token }) = auth {
        resolved_headers.insert("Authorization".to_string(), format!("Bearer {token}"));
    }
    Ok(HttpTransport::new(url.clone(), resolved_headers))
}

pub enum ConnectedTransport {
    Stdio(StdioTransport<BufReader<ChildStdout>, ChildStdin>),
    Http(HttpTransport),
}

impl McpTransport for ConnectedTransport {
    fn request(&mut self, method: &str, params: Value) -> ConnectorResult<Value> {
        match self {
            ConnectedTransport::Stdio(transport) => transport.request(method, params),
            ConnectedTransport::Http(transport) => transport.request(method, params),
        }
    }

    fn notify(&mut self, method: &str, params: Value) -> ConnectorResult<()> {
        match self {
            ConnectedTransport::Stdio(transport) => transport.notify(method, params),
            ConnectedTransport::Http(transport) => transport.notify(method, params),
        }
    }
}

pub fn connect_transport(target: &ConnectionTarget) -> ConnectorResult<ConnectedTransport> {
    match target {
        ConnectionTarget::Stdio { .. } => spawn_stdio(target).map(ConnectedTransport::Stdio),
        ConnectionTarget::Http { .. } => connect_http(target).map(ConnectedTransport::Http),
    }
}
