//! The outbound MCP-over-HTTP client.
//!
//! The receiver speaks JSON-RPC `tools/call` to the cloud harness endpoint. Every
//! call carries tenant_slug and, when configured, bearer auth. This is the ONLY
//! network seam, and it is outbound only: no inbound port, no tunnel.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use theorem_harness_core::{Job, JobSubmission};

use crate::wake::WakeMessage;
use crate::{ReceiverError, ReceiverResult};

/// An outbound client for the cloud harness MCP endpoint.
pub struct HarnessClient {
    http: reqwest::blocking::Client,
    url: String,
    token: Option<String>,
    tenant_slug: String,
    next_id: AtomicU64,
}

impl HarnessClient {
    /// Build a client. The optional token is the harness bearer (read from the
    /// env by the caller); it is never persisted.
    pub fn new(
        url: impl Into<String>,
        token: Option<String>,
        tenant_slug: impl Into<String>,
    ) -> ReceiverResult<Self> {
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(ReceiverError::from)?;
        Ok(Self {
            http,
            url: url.into(),
            token,
            tenant_slug: tenant_slug.into(),
            next_id: AtomicU64::new(1),
        })
    }

    /// Call an MCP tool and return its unwrapped backend payload (the `result`
    /// field of the `{tenant, result}` envelope).
    pub fn call_tool(&self, name: &str, mut arguments: Value) -> ReceiverResult<Value> {
        if let Value::Object(map) = &mut arguments {
            map.insert("tenant_slug".to_string(), json!(self.tenant_slug));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments },
        });
        let mut request = self.http.post(&self.url).json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        let value: Value = response.json()?;
        parse_tool_response(&value)
    }

    /// Probe the MCP endpoint with `tools/list`.
    pub fn tools_list(&self) -> ReceiverResult<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/list",
        });
        let mut request = self.http.post(&self.url).json(&body);
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        let response = request.send()?.error_for_status()?;
        let value: Value = response.json()?;
        if let Some(error) = value.get("error") {
            return Err(ReceiverError::Protocol(format!("jsonrpc error: {error}")));
        }
        value
            .get("result")
            .cloned()
            .ok_or_else(|| ReceiverError::Protocol("tools/list missing result".to_string()))
    }

    /// Read the board, filtered by repo and derived state.
    pub fn job_list(&self, repo: Option<&str>, state: Option<&str>) -> ReceiverResult<Vec<Job>> {
        let mut arguments = json!({});
        if let Value::Object(map) = &mut arguments {
            if let Some(repo) = repo {
                map.insert("repo".to_string(), json!(repo));
            }
            if let Some(state) = state {
                map.insert("state".to_string(), json!(state));
            }
        }
        let payload = self.call_tool("job_list", arguments)?;
        parse_list(&payload)
    }

    /// Create or upsert a board thread. Dispatch-backed receivers use this as a
    /// best-effort backfill when a row was inserted directly into Postgres.
    pub fn job_submit(
        &self,
        submission: JobSubmission,
        submitted_by: &str,
    ) -> ReceiverResult<Value> {
        let mut arguments = serde_json::to_value(submission)?;
        if let Value::Object(map) = &mut arguments {
            map.insert("submitted_by".to_string(), json!(submitted_by));
        }
        self.call_tool("job_submit", arguments)
    }

    /// Append a receipt. Receiver start and retry-clear writes use this verb too.
    pub fn job_note(
        &self,
        job_id: &str,
        actor: &str,
        text: &str,
        refs: Vec<String>,
        start_session_ref: Option<String>,
        clear_started: bool,
    ) -> ReceiverResult<Value> {
        self.call_tool(
            "job_note",
            json!({
                "job_id": job_id,
                "actor": actor,
                "text": text,
                "refs": refs,
                "start_session_ref": start_session_ref,
                "clear_started": clear_started,
            }),
        )
    }

    /// Archive a completed board thread.
    pub fn job_archive(&self, job_id: &str, reason: &str, actor: &str) -> ReceiverResult<Value> {
        self.call_tool(
            "job_archive",
            json!({
                "job_id": job_id,
                "reason": reason,
                "actor": actor,
            }),
        )
    }

    /// Read recent coordination-room messages for wake planning.
    pub fn read_messages_for_room(
        &self,
        room_id: &str,
        limit: usize,
    ) -> ReceiverResult<Vec<WakeMessage>> {
        let payload = self.call_tool(
            "read_messages_for_room",
            json!({
                "room_id": room_id,
                "limit": limit,
            }),
        )?;
        parse_room_messages(&payload)
    }
}

/// Extract the tool's backend payload from a JSON-RPC `tools/call` response.
fn parse_tool_response(value: &Value) -> ReceiverResult<Value> {
    if let Some(error) = value.get("error") {
        return Err(ReceiverError::Protocol(format!("jsonrpc error: {error}")));
    }
    let text = value
        .get("result")
        .and_then(|result| result.get("content"))
        .and_then(|content| content.get(0))
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ReceiverError::Protocol("response missing result.content[0].text".to_string())
        })?;
    let payload: Value = serde_json::from_str(&escape_raw_control_chars(text))?;

    // A tool-level error (e.g. read-only mode) comes back as `{error, message}`.
    if payload.get("error").is_some() {
        let message = payload
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("tool error");
        return Err(ReceiverError::Protocol(format!("tool error: {message}")));
    }

    // Some tools wrap the backend value in `{result: ...}` (job_list); others
    // return it directly (read_messages_for_room -> {messages, room_id, tenant}).
    Ok(payload.get("result").cloned().unwrap_or(payload))
}

/// Escape raw control characters (unescaped newlines etc.) that appear *inside*
/// JSON string literals. Some harness tool payloads embed raw newlines in record
/// and message bodies, which strict JSON rejects; this keeps the parse resilient
/// without disturbing structural whitespace between tokens.
fn escape_raw_control_chars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut in_string = false;
    let mut escaped = false;
    for ch in input.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => {
                out.push(ch);
                escaped = true;
            }
            '"' => {
                in_string = !in_string;
                out.push(ch);
            }
            c if in_string && (c as u32) < 0x20 => match c {
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                other => out.push_str(&format!("\\u{:04x}", other as u32)),
            },
            c => out.push(c),
        }
    }
    out
}

/// Interpret a `job_list` backend payload.
fn parse_list(payload: &Value) -> ReceiverResult<Vec<Job>> {
    let jobs = payload
        .get("jobs")
        .and_then(Value::as_array)
        .ok_or_else(|| ReceiverError::Protocol("job_list payload missing 'jobs'".to_string()))?;
    jobs.iter()
        .map(|job| serde_json::from_value(job.clone()).map_err(ReceiverError::from))
        .collect()
}

fn parse_room_messages(payload: &Value) -> ReceiverResult<Vec<WakeMessage>> {
    let messages = payload
        .get("messages")
        .and_then(Value::as_array)
        .ok_or_else(|| ReceiverError::Protocol("room payload missing 'messages'".to_string()))?;
    messages
        .iter()
        .cloned()
        .map(serde_json::from_value)
        .collect::<Result<Vec<_>, _>>()
        .map_err(ReceiverError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wrap(payload: Value) -> Value {
        // Mirror how the MCP server frames a tool result.
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "content": [{ "type": "text", "text": payload.to_string() }] }
        })
    }

    #[test]
    fn parses_job_list() {
        let job = json!({
            "job_id": "job-001",
            "title": "Dia",
            "spec_ref": "docs/plans/theorem-desktop/HANDOFF.md",
            "repo": "Travis-Gilbert/theorem",
            "priority": "P0",
            "target_head": "either",
            "submitted_by": "claude.ai",
            "submitted_at": "1.0Z",
            "session_ref": null,
            "started_at": null,
            "archived_at": null,
            "archived_reason": null,
            "idempotency_key": "sha256:abc",
            "receipts": []
        });
        let envelope =
            wrap(json!({ "tenant": "default", "result": { "count": 1, "jobs": [job] } }));
        let payload = parse_tool_response(&envelope).unwrap();
        let jobs = parse_list(&payload).unwrap();
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].job_id, "job-001");
        assert_eq!(jobs[0].derived_state(), "pending");
    }

    #[test]
    fn parses_empty_list() {
        let envelope = wrap(json!({ "tenant": "default", "result": { "count": 0, "jobs": [] } }));
        let payload = parse_tool_response(&envelope).unwrap();
        assert!(parse_list(&payload).unwrap().is_empty());
    }

    #[test]
    fn surfaces_jsonrpc_error() {
        let value =
            json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "bad" } });
        assert!(parse_tool_response(&value).is_err());
    }

    #[test]
    fn surfaces_read_only_tool_error() {
        let envelope = wrap(
            json!({ "error": "mcp_read_only", "message": "job_note is unavailable while read-only mode is active." }),
        );
        let result = parse_tool_response(&envelope);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("read-only"));
    }

    #[test]
    fn parses_room_messages_for_wake_client() {
        let payload = json!({
            "messages": [
                {
                    "tenant_slug": "default",
                    "room_id": "room",
                    "message_id": "msg-1",
                    "actor_id": "travis",
                    "delivery": "wake",
                    "message": "@codex hello",
                    "mentions": ["codex"]
                }
            ]
        });
        let messages = parse_room_messages(&payload).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_id, "msg-1");
        assert!(messages[0].is_wake());
        assert!(messages[0].targets_actor("codex"));
    }
}
