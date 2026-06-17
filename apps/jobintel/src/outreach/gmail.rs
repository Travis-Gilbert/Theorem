//! The Gmail transport: jobintel's second HTTP seam.
//!
//! This is the only place that talks to `gmail.googleapis.com`. It mirrors the
//! RustyRed client's shape: a base URL (swappable via `JOBINTEL_GMAIL_API` so the
//! integration test points it at a mock), a bearer token, and a handful of typed
//! calls pinned to the real Gmail REST v1 routes:
//!
//!   POST {base}/gmail/v1/users/me/drafts            create a draft (never send)
//!   GET  {base}/gmail/v1/users/me/drafts            list live draft ids
//!   GET  {base}/gmail/v1/users/me/threads/{id}      read a thread's From headers
//!
//! Auth: `Authorization: Bearer <GMAIL_TOKEN>`. jobintel only ever creates drafts
//! and reads threads; it never calls `users.drafts.send` or `users.messages.send`.

use base64::Engine;
use reqwest::blocking::Client;
use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{json, Value};

use crate::error::{JobIntelError, Result};

/// A created Gmail draft: the draft id (so we never double-draft) and the thread
/// id (so follow-ups land in the same conversation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftRef {
    pub draft_id: String,
    pub thread_id: String,
}

/// Thin Gmail REST client. Constructed only when an operation needs Gmail, so the
/// read-only verbs (`queue`, `stats`) never require a token.
pub struct GmailClient {
    http: Client,
    base: String,
    token: String,
}

impl GmailClient {
    /// Build from the operator's token and the (swappable) API base. Returns an
    /// `Outreach` error when no token is configured, so send-side verbs fail with
    /// one clear message instead of a 401 from Gmail.
    pub fn new(token: Option<&str>, base: &str) -> Result<Self> {
        let token = token
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .ok_or_else(|| {
                JobIntelError::Outreach(
                    "GMAIL_TOKEN is required for outreach draft/sync/followups (set it to the operator's Gmail OAuth access token, or a path to a file holding it)".into(),
                )
            })?
            .to_string();
        let http = Client::builder().user_agent("jobintel/0.2").build()?;
        Ok(Self {
            http,
            base: base.trim_end_matches('/').to_string(),
            token,
        })
    }

    fn url(&self, suffix: &str) -> String {
        format!("{}/gmail/v1/users/me/{}", self.base, suffix)
    }

    fn get_json<T: DeserializeOwned>(&self, route: &str, suffix: &str) -> Result<T> {
        let resp = self
            .http
            .get(self.url(suffix))
            .bearer_auth(&self.token)
            .send()?;
        read(route, resp)
    }

    /// Create a Gmail draft for `to` with `subject`/`body`. When `thread_id` is
    /// Some, the draft joins that thread (the spec's threaded follow-up). Returns
    /// the draft + thread ids. jobintel never sends; the operator clicks send.
    pub fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
    ) -> Result<DraftRef> {
        let raw = encode_raw_message(to, subject, body, thread_id);
        let mut message = json!({ "raw": raw });
        if let Some(tid) = thread_id {
            message["threadId"] = json!(tid);
        }
        let resp = self
            .http
            .post(self.url("drafts"))
            .bearer_auth(&self.token)
            .json(&json!({ "message": message }))
            .send()?;
        let value: Value = read("drafts.create", resp)?;
        let draft_id = value
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| missing_field("drafts.create", "id"))?
            .to_string();
        // Gmail echoes the assigned thread id on the draft's message; fall back to
        // the requested thread id (it is unchanged for an in-thread reply).
        let thread = value
            .get("message")
            .and_then(|m| m.get("threadId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| thread_id.map(str::to_string))
            .unwrap_or_default();
        Ok(DraftRef {
            draft_id,
            thread_id: thread,
        })
    }

    /// List the ids of the operator's live drafts. A draft id that was created by
    /// jobintel but is no longer here was sent (or deleted) - the signal `sync`
    /// uses to advance drafted -> sent.
    pub fn list_draft_ids(&self) -> Result<Vec<String>> {
        let value: Value = self.get_json("drafts.list", "drafts")?;
        Ok(value
            .get("drafts")
            .and_then(Value::as_array)
            .map(|drafts| {
                drafts
                    .iter()
                    .filter_map(|d| d.get("id").and_then(Value::as_str).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Return the lowercased sender email addresses seen in `thread_id`. Used by
    /// reply detection (a lead address in the thread means they replied) and by
    /// the sent-detection gate (a non-empty thread means a message exists).
    pub fn thread_senders(&self, thread_id: &str) -> Result<Vec<String>> {
        let suffix = format!(
            "threads/{}?format=metadata&metadataHeaders=From",
            urlencode(thread_id)
        );
        let value: Value = self.get_json("threads.get", &suffix)?;
        Ok(extract_thread_senders(&value))
    }
}

/// Build the base64url-encoded RFC822 message Gmail's `raw` field expects.
fn encode_raw_message(to: &str, subject: &str, body: &str, thread_id: Option<&str>) -> String {
    let mut headers = format!(
        "To: {to}\r\nSubject: {subject}\r\nContent-Type: text/plain; charset=\"UTF-8\"\r\nMIME-Version: 1.0\r\n"
    );
    // For an in-thread follow-up, hint the reply relationship. Gmail threads
    // authoritatively by the request's `threadId`; these headers keep the
    // message well-formed for clients that read them.
    if let Some(tid) = thread_id {
        headers.push_str(&format!("In-Reply-To: {tid}\r\nReferences: {tid}\r\n"));
    }
    let message = format!("{headers}\r\n{body}");
    base64::engine::general_purpose::URL_SAFE.encode(message.as_bytes())
}

/// Pull the `From` addresses out of a `threads.get` (metadata) payload.
fn extract_thread_senders(thread: &Value) -> Vec<String> {
    let mut out = Vec::new();
    let Some(messages) = thread.get("messages").and_then(Value::as_array) else {
        return out;
    };
    for msg in messages {
        let Some(headers) = msg
            .get("payload")
            .and_then(|p| p.get("headers"))
            .and_then(Value::as_array)
        else {
            continue;
        };
        for header in headers {
            let name = header.get("name").and_then(Value::as_str).unwrap_or("");
            if name.eq_ignore_ascii_case("from") {
                if let Some(value) = header.get("value").and_then(Value::as_str) {
                    out.push(extract_email(value));
                }
            }
        }
    }
    out
}

/// Extract a bare lowercased email address from a `From` header value, which is
/// usually `Display Name <addr@host>` but may be a bare address.
pub fn extract_email(from: &str) -> String {
    let trimmed = from.trim();
    if let (Some(start), Some(end)) = (trimmed.find('<'), trimmed.rfind('>')) {
        if start < end {
            return trimmed[start + 1..end].trim().to_lowercase();
        }
    }
    trimmed.to_lowercase()
}

fn read<T: DeserializeOwned>(route: &str, resp: reqwest::blocking::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Err(JobIntelError::Gmail {
            route: route.to_string(),
            status: status.as_u16(),
            body: truncate(&body, 400),
        });
    }
    Ok(resp.json::<T>()?)
}

fn missing_field(route: &str, field: &str) -> JobIntelError {
    JobIntelError::Gmail {
        route: route.to_string(),
        status: StatusCode::OK.as_u16(),
        body: format!("response missing `{field}`"),
    }
}

/// Minimal percent-encoding for a thread id used inside a URL path segment.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_message_is_base64url_and_decodes_to_rfc822() {
        let raw = encode_raw_message("a@b.com", "Hello", "Body line", None);
        // URL-safe alphabet: no '+' or '/'.
        assert!(!raw.contains('+') && !raw.contains('/'));
        let decoded = base64::engine::general_purpose::URL_SAFE
            .decode(raw.as_bytes())
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        assert!(text.contains("To: a@b.com"));
        assert!(text.contains("Subject: Hello"));
        assert!(text.ends_with("\r\nBody line"));
        assert!(!text.contains("In-Reply-To"));
    }

    #[test]
    fn raw_message_threads_a_followup() {
        let raw = encode_raw_message("a@b.com", "Re: Hello", "Just following up", Some("t-123"));
        let text = String::from_utf8(
            base64::engine::general_purpose::URL_SAFE
                .decode(raw.as_bytes())
                .unwrap(),
        )
        .unwrap();
        assert!(text.contains("In-Reply-To: t-123"));
        assert!(text.contains("References: t-123"));
    }

    #[test]
    fn extract_email_handles_display_name_and_bare() {
        assert_eq!(extract_email("Jane Doe <Jane@Acme.com>"), "jane@acme.com");
        assert_eq!(extract_email("  bare@x.io "), "bare@x.io");
        assert_eq!(extract_email("Weird <a@b.com> trailer"), "a@b.com");
    }

    #[test]
    fn extract_thread_senders_reads_from_headers() {
        let thread = json!({
            "messages": [
                { "payload": { "headers": [
                    { "name": "From", "value": "Me <me@self.com>" },
                    { "name": "Subject", "value": "x" }
                ]}},
                { "payload": { "headers": [
                    { "name": "From", "value": "lead@company.com" }
                ]}}
            ]
        });
        let senders = extract_thread_senders(&thread);
        assert_eq!(senders, vec!["me@self.com", "lead@company.com"]);
    }

    #[test]
    fn new_refuses_without_token() {
        assert!(matches!(
            GmailClient::new(None, "https://x"),
            Err(JobIntelError::Outreach(_))
        ));
        assert!(GmailClient::new(Some("tok"), "https://x").is_ok());
    }
}
