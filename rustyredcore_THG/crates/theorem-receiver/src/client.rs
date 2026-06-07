//! The outbound MCP-over-HTTP client.
//!
//! The receiver speaks JSON-RPC `tools/call` to the cloud harness endpoint. Every
//! call carries the bearer token and tenant_slug. This is the ONLY network seam,
//! and it is outbound only: no inbound port, no tunnel.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{json, Value};
use theorem_harness_core::Job;

use crate::{ReceiverError, ReceiverResult};

/// An outbound client for the cloud harness MCP endpoint.
pub struct HarnessClient {
    http: reqwest::blocking::Client,
    url: String,
    token: String,
    tenant_slug: String,
    next_id: AtomicU64,
}

impl HarnessClient {
    /// Build a client. The token is the harness bearer (read from the env by the
    /// caller); it is never persisted.
    pub fn new(
        url: impl Into<String>,
        token: impl Into<String>,
        tenant_slug: impl Into<String>,
    ) -> ReceiverResult<Self> {
        let http = reqwest::blocking::Client::builder()
            .build()
            .map_err(ReceiverError::from)?;
        Ok(Self {
            http,
            url: url.into(),
            token: token.into(),
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
        let response = self
            .http
            .post(&self.url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()?
            .error_for_status()?;
        let value: Value = response.json()?;
        parse_tool_response(&value)
    }

    /// Claim the highest-priority matching job, or `None` if nothing is queued.
    pub fn job_claim(
        &self,
        receiver_id: &str,
        lanes: &[String],
        repos: &[String],
    ) -> ReceiverResult<Option<Job>> {
        let payload = self.call_tool(
            "job_claim",
            json!({ "receiver_id": receiver_id, "lanes": lanes, "repos": repos }),
        )?;
        parse_claim(&payload)
    }

    /// Close a job Done/Failed with a fitness receipt.
    pub fn job_complete(
        &self,
        job_id: &str,
        outcome: &str,
        pr_ref: Option<String>,
        session_ref: Option<String>,
        receipts: Value,
    ) -> ReceiverResult<Value> {
        self.call_tool(
            "job_complete",
            json!({
                "job_id": job_id,
                "outcome": outcome,
                "pr_ref": pr_ref,
                "session_ref": session_ref,
                "receipts": receipts,
            }),
        )
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
    let payload: Value = serde_json::from_str(text)?;

    // A tool-level error (e.g. read-only mode) comes back as a result whose text
    // is `{error, message}` with no `result` envelope.
    if payload.get("result").is_none() {
        if let Some(message) = payload.get("message").and_then(Value::as_str) {
            return Err(ReceiverError::Protocol(format!("tool error: {message}")));
        }
        if payload.get("error").is_some() {
            return Err(ReceiverError::Protocol(format!("tool error: {payload}")));
        }
    }

    payload
        .get("result")
        .cloned()
        .ok_or_else(|| ReceiverError::Protocol("tool payload missing 'result' envelope".to_string()))
}

/// Interpret a `job_claim` backend payload.
fn parse_claim(payload: &Value) -> ReceiverResult<Option<Job>> {
    match payload.get("claimed").and_then(Value::as_bool) {
        Some(true) => {
            let job_value = payload
                .get("job")
                .ok_or_else(|| ReceiverError::Protocol("claimed payload missing 'job'".to_string()))?;
            let job: Job = serde_json::from_value(job_value.clone())?;
            Ok(Some(job))
        }
        _ => Ok(None),
    }
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
    fn parses_a_claimed_job() {
        let job = json!({
            "job_id": "job-001",
            "kind": "App",
            "title": "Dia",
            "spec_ref": "docs/plans/theorem-desktop/HANDOFF.md",
            "repo": "Travis-Gilbert/theorem",
            "branch": "job/job-001",
            "priority": "P0",
            "target_head": "Either",
            "status": "Claimed",
            "submitted_by": "claude.ai",
            "submitted_at": "1.0Z",
            "claimed_by": "receiver-a",
            "claimed_at": "2.0Z",
            "closed_at": null,
            "session_ref": null,
            "pr_ref": null,
            "idempotency_key": "sha256:abc",
            "notes": null
        });
        let envelope = wrap(json!({ "tenant": "default", "result": { "claimed": true, "job_id": "job-001", "job": job } }));
        let payload = parse_tool_response(&envelope).unwrap();
        let claimed = parse_claim(&payload).unwrap().unwrap();
        assert_eq!(claimed.job_id, "job-001");
        assert_eq!(claimed.branch_ref(), "job/job-001");
    }

    #[test]
    fn parses_empty_claim() {
        let envelope = wrap(json!({ "tenant": "default", "result": { "claimed": false } }));
        let payload = parse_tool_response(&envelope).unwrap();
        assert!(parse_claim(&payload).unwrap().is_none());
    }

    #[test]
    fn surfaces_jsonrpc_error() {
        let value = json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "bad" } });
        assert!(parse_tool_response(&value).is_err());
    }

    #[test]
    fn surfaces_read_only_tool_error() {
        let envelope = wrap(json!({ "error": "mcp_read_only", "message": "job_claim is unavailable while read-only mode is active." }));
        let result = parse_tool_response(&envelope);
        assert!(result.is_err());
        assert!(format!("{}", result.unwrap_err()).contains("read-only"));
    }
}
