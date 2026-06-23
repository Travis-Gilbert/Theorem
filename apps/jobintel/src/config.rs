//! Runtime configuration, resolved from environment variables.
//!
//! Spec contract (jobintel-build-spec.md, "Shape and boundary"):
//!   RUSTYRED_URL, RUSTYRED_TENANT, RUSTYRED_TOKEN, HUNTER_API_KEY (optional).
//!
//! Two extras beyond the spec, both with safe defaults so the one-command demo
//! still works untouched:
//!   JOBINTEL_EMBED_URL  - endpoint for the HttpEmbedder (Theseus SBERT swap).
//!   JOBINTEL_EMBED_DIM  - embedding dimension D (default 384, matching bge-small).
//!
//! 0.2 (outreach) adds the Gmail + cadence config (spec "Shape and boundary"):
//!   GMAIL_TOKEN         - operator's Gmail OAuth access token (Bearer), or a
//!                         path to a file containing it. Absent => draft/sync/
//!                         followups refuse with a clear error (queue/stats still
//!                         work: they only read RustyRed).
//!   DAILY_DRAFT_CAP     - max drafts per `outreach draft` run (default 8).
//!   FOLLOWUP_DAYS       - comma list of days-after-send for nudges (default "4,9").
//!   JOBINTEL_GMAIL_API  - Gmail API base URL (default https://gmail.googleapis.com);
//!                         overridable so the integration test points it at a mock.

use crate::error::{JobIntelError, Result};

/// Default embedding dimension. Matches the spec's bge-small-en-v1.5 choice so
/// the HNSW index designated at this D is interchangeable across embedders.
pub const DEFAULT_EMBED_DIM: usize = 384;

/// Spec default: at most 8 drafts per `outreach draft` run. The daily cap is the
/// safety rail (sender-reputation guardrail), not a throttle to remove.
pub const DEFAULT_DAILY_DRAFT_CAP: usize = 8;

/// Spec default follow-up schedule: nudge at day 4 and day 9 after send, then stop.
pub const DEFAULT_FOLLOWUP_DAYS: &[u32] = &[4, 9];

/// Default Gmail API base. Overridable via `JOBINTEL_GMAIL_API` so tests can swap
/// in a local mock the same way the RustyRed client is base-URL-swappable.
pub const DEFAULT_GMAIL_API: &str = "https://gmail.googleapis.com";

#[derive(Debug, Clone)]
pub struct Config {
    /// Base URL of the running RustyRed server, e.g. `http://localhost:8080`.
    pub rustyred_url: String,
    /// Tenant slug the graph writes/reads are scoped to.
    pub tenant: String,
    /// Bearer token presented as `Authorization: Bearer <token>`.
    pub token: String,
    /// Hunter.io key for ATS contact discovery. Absent => contacts left empty.
    pub hunter_api_key: Option<String>,
    /// Optional remote embedding endpoint for the HttpEmbedder.
    pub embed_url: Option<String>,
    /// Embedding dimension D used for vector designation + queries.
    pub embed_dim: usize,
    /// Operator's Gmail OAuth access token (Bearer). None => outreach send-side
    /// verbs (draft/sync/followups) refuse with a clear error.
    pub gmail_token: Option<String>,
    /// Gmail API base URL (default `https://gmail.googleapis.com`).
    pub gmail_api_base: String,
    /// Max drafts per `outreach draft` run (the daily safety rail).
    pub daily_draft_cap: usize,
    /// Days-after-send for follow-up nudges, ascending (default [4, 9]).
    pub followup_days: Vec<u32>,
}

impl Config {
    /// Resolve from the process environment. `RUSTYRED_URL` and `RUSTYRED_TENANT`
    /// are required; `RUSTYRED_TOKEN` defaults to empty (dev servers run with
    /// `require_auth = false`, in which case any/empty token is accepted).
    pub fn from_env() -> Result<Self> {
        let rustyred_url = std::env::var("RUSTYRED_URL")
            .map_err(|_| JobIntelError::Config("RUSTYRED_URL is required".into()))?
            .trim_end_matches('/')
            .to_string();
        let tenant = std::env::var("RUSTYRED_TENANT")
            .map_err(|_| JobIntelError::Config("RUSTYRED_TENANT is required".into()))?;
        let token = std::env::var("RUSTYRED_TOKEN").unwrap_or_default();
        let hunter_api_key = non_empty(std::env::var("HUNTER_API_KEY").ok());
        let embed_url = non_empty(std::env::var("JOBINTEL_EMBED_URL").ok());
        let embed_dim = std::env::var("JOBINTEL_EMBED_DIM")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .filter(|dim| *dim > 0)
            .unwrap_or(DEFAULT_EMBED_DIM);

        let gmail_token = resolve_gmail_token();
        let gmail_api_base = non_empty(std::env::var("JOBINTEL_GMAIL_API").ok())
            .unwrap_or_else(|| DEFAULT_GMAIL_API.to_string())
            .trim_end_matches('/')
            .to_string();
        let daily_draft_cap = std::env::var("DAILY_DRAFT_CAP")
            .ok()
            .and_then(|raw| raw.trim().parse::<usize>().ok())
            .filter(|cap| *cap > 0)
            .unwrap_or(DEFAULT_DAILY_DRAFT_CAP);
        let followup_days = std::env::var("FOLLOWUP_DAYS")
            .ok()
            .map(|raw| parse_followup_days(&raw))
            .filter(|days| !days.is_empty())
            .unwrap_or_else(|| DEFAULT_FOLLOWUP_DAYS.to_vec());

        Ok(Self {
            rustyred_url,
            tenant,
            token,
            hunter_api_key,
            embed_url,
            embed_dim,
            gmail_token,
            gmail_api_base,
            daily_draft_cap,
            followup_days,
        })
    }
}

/// `GMAIL_TOKEN` may be the raw token or a path to a file holding it (spec:
/// "GMAIL_TOKEN (or a path to the operator's Gmail OAuth)"). If the value names
/// an existing file, read + trim its contents; otherwise treat it as the token.
fn resolve_gmail_token() -> Option<String> {
    let raw = non_empty(std::env::var("GMAIL_TOKEN").ok())?;
    if std::path::Path::new(&raw).is_file() {
        std::fs::read_to_string(&raw)
            .ok()
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty())
    } else {
        Some(raw)
    }
}

/// Parse a "4,9"-style schedule into ascending, de-duplicated, positive days.
fn parse_followup_days(raw: &str) -> Vec<u32> {
    let mut days: Vec<u32> = raw
        .split(',')
        .filter_map(|t| t.trim().parse::<u32>().ok())
        .filter(|d| *d > 0)
        .collect();
    days.sort_unstable();
    days.dedup();
    days
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_followup_days_sorts_dedups_and_drops_junk() {
        assert_eq!(parse_followup_days("4,9"), vec![4, 9]);
        assert_eq!(parse_followup_days("9, 4, 4"), vec![4, 9]);
        assert_eq!(parse_followup_days(" 7 , x , 0 , 2 "), vec![2, 7]);
        assert!(parse_followup_days("nonsense").is_empty());
    }
}
