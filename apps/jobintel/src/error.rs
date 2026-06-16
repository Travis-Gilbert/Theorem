//! Crate error type.
//!
//! A single `thiserror` enum keeps error provenance explicit across the HTTP,
//! parse, and config seams. The CLI boundary (`main.rs`) lifts these into
//! `anyhow::Result` for ergonomic top-level reporting, but library code returns
//! the typed enum so callers can match on the failure mode.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum JobIntelError {
    #[error("config error: {0}")]
    Config(String),

    #[error("http transport error: {0}")]
    Http(#[from] reqwest::Error),

    /// A RustyRed route returned a non-2xx status. Carries the status and the
    /// (truncated) body so a failing tenant route is debuggable from the CLI.
    #[error("rustyred {route} returned HTTP {status}: {body}")]
    Rustyred {
        route: String,
        status: u16,
        body: String,
    },

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ingest error: {0}")]
    Ingest(String),

    #[error("embedding error: {0}")]
    Embed(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, JobIntelError>;
