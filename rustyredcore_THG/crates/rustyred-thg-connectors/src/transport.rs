//! MCP transport: a synchronous `McpTransport` trait (request/notify with
//! JSON-RPC id correlation) and a `StdioTransport` over newline-delimited JSON.
//! The framing is generic over `BufRead + Write` so it tests over in-memory
//! buffers; the only line that touches the OS is `spawn_stdio`'s process spawn.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{ConnectorError, ConnectorResult};

pub const JSONRPC_VERSION: &str = "2.0";

/// How to reach an external MCP server. Slice 1 ships stdio (the dominant
/// local-MCP transport: an `npx`/binary server speaking JSON-RPC over its own
/// stdio). HTTP/SSE is a later drop-in behind the same `McpTransport` trait.
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
}

/// A synchronous MCP transport. `request` sends a JSON-RPC request and returns
/// the `result` (mapping a JSON-RPC `error` to `ConnectorError::Rpc`); `notify`
/// sends a notification (no id, no response expected).
pub trait McpTransport {
    fn request(&mut self, method: &str, params: Value) -> ConnectorResult<Value>;
    fn notify(&mut self, method: &str, params: Value) -> ConnectorResult<()>;
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
    let ConnectionTarget::Stdio { command, args, env } = target;
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
