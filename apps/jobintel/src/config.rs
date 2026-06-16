//! Runtime configuration, resolved from environment variables.
//!
//! Spec contract (jobintel-build-spec.md, "Shape and boundary"):
//!   RUSTYRED_URL, RUSTYRED_TENANT, RUSTYRED_TOKEN, HUNTER_API_KEY (optional).
//!
//! Two extras beyond the spec, both with safe defaults so the one-command demo
//! still works untouched:
//!   JOBINTEL_EMBED_URL  - endpoint for the HttpEmbedder (Theseus SBERT swap).
//!   JOBINTEL_EMBED_DIM  - embedding dimension D (default 384, matching bge-small).

use crate::error::{JobIntelError, Result};

/// Default embedding dimension. Matches the spec's bge-small-en-v1.5 choice so
/// the HNSW index designated at this D is interchangeable across embedders.
pub const DEFAULT_EMBED_DIM: usize = 384;

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

        Ok(Self {
            rustyred_url,
            tenant,
            token,
            hunter_api_key,
            embed_url,
            embed_dim,
        })
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}
