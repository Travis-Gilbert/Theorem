//! The record transport seam (Layer A): how a spoke actually gets records from
//! the outside world. Curated spokes and [`MappedSpoke`](crate::MappedSpoke)
//! share it; in production it is HTTP or an MCP stdio session, in tests it is a
//! recorded-response fixture. Sync, matching the rest of the substrate.

use crate::spoke::{SourceCursor, SourcePage, SourceRecord, SourceResult, SourceScope};

/// How a spoke gets records. Injecting this keeps live network out of the
/// spokes' mapping logic, so the same spoke is tested against fixtures and run
/// against a real endpoint.
pub trait RecordTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage>;
}

/// An in-memory transport over recorded responses (tests + the universal path).
/// Honors the scope filters by construction: `containers`, `since_ms`, and
/// `max_records`, and paginates with a resumable cursor (the cursor token is the
/// offset into the eligible set).
pub struct FixtureTransport {
    /// Each entry pairs a record with the container it belongs to, so
    /// `scope.containers` can filter without parsing the raw record.
    entries: Vec<(SourceRecord, Option<String>)>,
    page_size: usize,
}

impl FixtureTransport {
    /// Records with no container (all pass any `containers` filter only when the
    /// scope's container list is empty).
    pub fn new(records: Vec<SourceRecord>) -> Self {
        Self {
            entries: records.into_iter().map(|r| (r, None)).collect(),
            page_size: 50,
        }
    }

    /// Records tagged with their container, so a scoped fetch can filter them.
    pub fn with_containers(entries: Vec<(SourceRecord, Option<String>)>) -> Self {
        Self {
            entries,
            page_size: 50,
        }
    }

    /// Force a small page size to exercise pagination.
    pub fn page_size(mut self, page_size: usize) -> Self {
        self.page_size = page_size.max(1);
        self
    }

    fn eligible(&self, scope: &SourceScope) -> Vec<SourceRecord> {
        self.entries
            .iter()
            .filter(|(_, container)| container_admitted(&scope.containers, container.as_deref()))
            .filter(|(record, _)| match scope.since_ms {
                Some(since) => record.fetched_at_ms >= since,
                None => true,
            })
            .map(|(record, _)| record.clone())
            .collect()
    }
}

/// A record is admitted when the scope lists no containers (the spoke's default
/// subset, modeled here as "all the fixture holds") or when its container is in
/// the list.
fn container_admitted(wanted: &[String], container: Option<&str>) -> bool {
    if wanted.is_empty() {
        return true;
    }
    match container {
        Some(have) => wanted.iter().any(|w| w == have),
        None => false,
    }
}

impl RecordTransport for FixtureTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let mut eligible = self.eligible(scope);
        if let Some(max) = scope.max_records {
            eligible.truncate(max as usize);
        }
        let start = cursor.token.parse::<usize>().unwrap_or(0).min(eligible.len());
        let end = (start + self.page_size).min(eligible.len());
        let records = eligible[start..end].to_vec();
        let consumed = end;
        let exhausted = consumed >= eligible.len();
        let updated_at_ms = records
            .last()
            .map(|r| r.fetched_at_ms)
            .unwrap_or(cursor.updated_at_ms);
        Ok(SourcePage {
            records,
            next: SourceCursor {
                token: consumed.to_string(),
                updated_at_ms,
            },
            exhausted,
        })
    }
}
