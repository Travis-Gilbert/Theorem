//! The ingestion-spoke contract (Layer A1/A2).

use std::fmt;

use commonplace::IngestInput;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Errors from a source spoke.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceError {
    /// Credentials missing, expired, or rejected.
    Auth(String),
    /// Network / process / stream failure reaching the source.
    Transport(String),
    /// The source asked us to back off.
    RateLimit(String),
    /// A record could not be shaped onto the universal capture contract.
    Mapping(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Auth(m) => write!(f, "auth error: {m}"),
            SourceError::Transport(m) => write!(f, "transport error: {m}"),
            SourceError::RateLimit(m) => write!(f, "rate limited: {m}"),
            SourceError::Mapping(m) => write!(f, "mapping error: {m}"),
        }
    }
}

impl std::error::Error for SourceError {}

pub type SourceResult<T> = Result<T, SourceError>;

/// A native record pulled from a source, plus its stable id in that source.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SourceRecord {
    /// Stable id in the source (A3 identity is `(source_id, external_id)`).
    pub external_id: String,
    /// The source's native record, opaque to the framework.
    pub raw: Value,
    /// When this record was fetched (epoch ms).
    pub fetched_at_ms: i64,
}

impl SourceRecord {
    pub fn new(external_id: impl Into<String>, raw: Value, fetched_at_ms: i64) -> Self {
        Self {
            external_id: external_id.into(),
            raw,
            fetched_at_ms,
        }
    }
}

/// An opaque per-source incremental cursor. The driver persists it per
/// `(tenant, source)`; where it persists is the catalog (F3) in production and a
/// map in tests.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SourceCursor {
    pub token: String,
    pub updated_at_ms: i64,
}

/// One bounded page of records plus the advanced cursor.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SourcePage {
    pub records: Vec<SourceRecord>,
    pub next: SourceCursor,
    pub exhausted: bool,
}

/// What a spoke is allowed to pull. Scoped by construction (A2): a connected
/// mailbox is a firehose, so a spoke pulls only what its scope admits.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SourceScope {
    /// Named containers in the source (Gmail labels, Notion databases, Outlook
    /// folders, Linear teams). Empty means the spoke's documented default
    /// container subset, never the whole account.
    pub containers: Vec<String>,
    /// Only records touched at or after this instant (epoch ms). Bounds the first
    /// backfill and pairs with the cursor on incremental runs.
    pub since_ms: Option<i64>,
    /// Hard cap on records per sync, so a first connect cannot stall on a decade
    /// of history.
    pub max_records: Option<u32>,
    /// Per-source record-type filter (e.g. Gmail "is:unread", Linear
    /// "state:open"), passed through opaquely to the spoke.
    pub filters: Vec<String>,
}

impl SourceScope {
    /// A scope over the given containers and nothing else.
    pub fn containers<I, T>(containers: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        Self {
            containers: containers.into_iter().map(Into::into).collect(),
            ..Self::default()
        }
    }

    pub fn since(mut self, since_ms: i64) -> Self {
        self.since_ms = Some(since_ms);
        self
    }

    pub fn max_records(mut self, max: u32) -> Self {
        self.max_records = Some(max);
        self
    }
}

/// A source of items: an external service that CommonPlace pulls from.
/// Implementations map the service's native records onto [`IngestInput`], the
/// universal capture contract. They never call back into the agent.
pub trait SourceSpoke {
    /// Stable source identifier, written onto `Item.source` (e.g. "gmail",
    /// "notion", "linear"). Source-agnostic organizing keys off content, so this
    /// string is a routing signal, not a classifier.
    fn source_id(&self) -> &str;

    /// Pull the records the scope admits that changed after the cursor. Returns
    /// the records plus the advanced cursor; pagination lives inside the
    /// implementation, so the driver sees one bounded page at a time.
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage>;

    /// Map one native record onto the universal capture contract. The only
    /// source-specific shaping the rest of the system sees.
    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput>;

    /// The container a record belongs to (Gmail label, Linear team, Notion
    /// database), for explicit routing (B1). Default `None`; curated spokes
    /// override to read it off the record.
    fn record_container(&self, _record: &SourceRecord) -> Option<String> {
        None
    }
}
