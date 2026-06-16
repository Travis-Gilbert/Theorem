//! Module 1 - ingest. Pulls the open job sources into `JobRecord`s.
//!
//! Layout: `text` (pure primitives), `hn` (Hacker News), `ats` (Greenhouse /
//! Lever / Ashby). This module owns the shared HTTP client, the slugs.toml
//! loader, and `fetch_all`, which sequences HN + every ATS board.

pub mod ats;
pub mod hn;
pub mod text;

use std::time::Duration;

use reqwest::blocking::Client;
use serde::Deserialize;

use crate::error::{JobIntelError, Result};
use crate::model::JobRecord;

/// One company entry from slugs.toml. `slug` is the ATS board slug; `domain`
/// (optional) is the web domain used for Hunter.io contact lookup.
#[derive(Debug, Clone, Deserialize)]
pub struct CompanySlug {
    pub slug: String,
    #[serde(default)]
    pub domain: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SlugsFile {
    #[serde(default)]
    company: Vec<CompanySlug>,
}

/// Load the company seed list from a TOML file.
pub fn load_slugs(path: &str) -> Result<Vec<CompanySlug>> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| JobIntelError::Ingest(format!("could not read slugs file '{path}': {e}")))?;
    let parsed: SlugsFile = toml::from_str(&raw)
        .map_err(|e| JobIntelError::Ingest(format!("slugs.toml parse error: {e}")))?;
    Ok(parsed.company)
}

/// Build the outbound HTTP client used for all source fetches (separate from
/// the RustyRed client). A 25s timeout keeps a slow board from stalling the run.
pub fn http_client() -> Result<Client> {
    Ok(Client::builder()
        .user_agent("jobintel/0.1 (+https://github.com/Travis-Gilbert)")
        .timeout(Duration::from_secs(25))
        .build()?)
}

/// Fetch every source: the latest HN "Who is Hiring" thread (unless `skip_hn`)
/// plus every ATS board in `slugs`. Per-source failures are logged, not fatal,
/// so one dead board never sinks the whole run.
pub fn fetch_all(client: &Client, slugs: &[CompanySlug], skip_hn: bool) -> Result<Vec<JobRecord>> {
    let mut records = Vec::new();

    if !skip_hn {
        match hn::discover_thread(client).and_then(|id| {
            eprintln!("  HN: latest 'Who is hiring' thread = {id}");
            hn::fetch_hn(client, id)
        }) {
            Ok(hn_records) => {
                eprintln!("  HN: {} top-level posts", hn_records.len());
                records.extend(hn_records);
            }
            Err(err) => eprintln!("  HN ingest failed (continuing): {err}"),
        }
    }

    if !slugs.is_empty() {
        eprintln!("  ATS: probing {} company boards...", slugs.len());
        for company in slugs {
            let found = ats::probe_ats(client, &company.slug, company.domain.as_deref());
            if !found.is_empty() {
                eprintln!("  ATS: {} -> {} roles", company.slug, found.len());
            }
            records.extend(found);
        }
    }

    Ok(records)
}
