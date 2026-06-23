//! Five curated first-class spokes (Layer A6): GSuite, Gmail, Outlook, Notion,
//! Linear. GitHub is the existing ingestion exemplar (the gateway's named
//! example), so it is the pattern these mirror, not rebuilt.
//!
//! Each spoke wraps a [`RecordTransport`] (a fixture in tests, HTTP/MCP in
//! production) and supplies the only source-specific shaping: the
//! record-to-[`IngestInput`] mapping and the container a record belongs to. The
//! four things the spec names per source - auth, scope, mapping, cursor - are in
//! the doc comments; auth/cursor are documented seams (the credential lives in
//! the catalog F3 row; the cursor token is the source's incremental marker the
//! driver persists), not live clients.

use commonplace::{IngestInput, ItemKind, TaskFields};
use serde_json::Value;

use crate::mapped::{field_i64, field_str};
use crate::spoke::{SourceCursor, SourcePage, SourceRecord, SourceResult, SourceScope, SourceSpoke};
use crate::transport::RecordTransport;

/// First string in a JSON array field (e.g. Gmail `labelIds[0]`).
fn first_in_array(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)?
        .as_array()?
        .first()?
        .as_str()
        .map(str::to_string)
}

macro_rules! delegate_fetch {
    () => {
        fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
            self.transport.fetch(scope, cursor)
        }
    };
}

/// Gmail. Auth: OAuth user token (catalog F3 row). Scope: `containers` are Gmail
/// labels, `filters` are search operators ("is:unread"). Cursor: Gmail historyId.
pub struct GmailSpoke {
    transport: Box<dyn RecordTransport>,
}

impl GmailSpoke {
    pub fn new(transport: Box<dyn RecordTransport>) -> Self {
        Self { transport }
    }
}

impl SourceSpoke for GmailSpoke {
    fn source_id(&self) -> &str {
        "gmail"
    }
    delegate_fetch!();

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        // Mapping: subject -> title, body inline; a long message is a Doc, a
        // short one a Note. An attachment record would map to a File item.
        let subject = field_str(&record.raw, "subject").unwrap_or_else(|| "(no subject)".into());
        let body = field_str(&record.raw, "body")
            .or_else(|| field_str(&record.raw, "snippet"))
            .unwrap_or_default();
        let kind = if body.len() > 800 {
            ItemKind::Doc
        } else {
            ItemKind::Note
        };
        Ok(IngestInput::text(subject, body, kind))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        first_in_array(&record.raw, "labelIds")
    }
}

/// GSuite (Drive). Auth: OAuth user token. Scope: `containers` are Drive folder
/// ids. Cursor: Drive changes pageToken. Mapping: name -> title; a Google Doc
/// becomes a Doc, anything else a File.
pub struct GSuiteSpoke {
    transport: Box<dyn RecordTransport>,
}

impl GSuiteSpoke {
    pub fn new(transport: Box<dyn RecordTransport>) -> Self {
        Self { transport }
    }
}

impl SourceSpoke for GSuiteSpoke {
    fn source_id(&self) -> &str {
        "gsuite"
    }
    delegate_fetch!();

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        let name = field_str(&record.raw, "name").unwrap_or_else(|| "(untitled)".into());
        let mime = field_str(&record.raw, "mimeType").unwrap_or_default();
        let body = field_str(&record.raw, "text").unwrap_or_default();
        let kind = if mime.contains("document") || mime.contains("text") {
            ItemKind::Doc
        } else {
            ItemKind::File
        };
        Ok(IngestInput::text(name, body, kind))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        first_in_array(&record.raw, "parents").or_else(|| field_str(&record.raw, "folderId"))
    }
}

/// Outlook. Auth: Microsoft Graph token. Scope: `containers` are mail folders.
/// Cursor: Microsoft Graph delta link. Mapping: subject -> title, bodyPreview
/// inline.
pub struct OutlookSpoke {
    transport: Box<dyn RecordTransport>,
}

impl OutlookSpoke {
    pub fn new(transport: Box<dyn RecordTransport>) -> Self {
        Self { transport }
    }
}

impl SourceSpoke for OutlookSpoke {
    fn source_id(&self) -> &str {
        "outlook"
    }
    delegate_fetch!();

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        let subject = field_str(&record.raw, "subject").unwrap_or_else(|| "(no subject)".into());
        let body = field_str(&record.raw, "bodyPreview")
            .or_else(|| field_str(&record.raw, "body"))
            .unwrap_or_default();
        let kind = if body.len() > 800 {
            ItemKind::Doc
        } else {
            ItemKind::Note
        };
        Ok(IngestInput::text(subject, body, kind))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        field_str(&record.raw, "parentFolderId")
    }
}

/// Notion. Auth: integration token. Scope: `containers` are database ids.
/// Cursor: Notion last_edited_time. Mapping: title -> title, page text inline;
/// a page is a Doc.
pub struct NotionSpoke {
    transport: Box<dyn RecordTransport>,
}

impl NotionSpoke {
    pub fn new(transport: Box<dyn RecordTransport>) -> Self {
        Self { transport }
    }
}

impl SourceSpoke for NotionSpoke {
    fn source_id(&self) -> &str {
        "notion"
    }
    delegate_fetch!();

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        let title = field_str(&record.raw, "title")
            .or_else(|| field_str(&record.raw, "properties.title"))
            .unwrap_or_else(|| "(untitled)".into());
        let body = field_str(&record.raw, "text").unwrap_or_default();
        Ok(IngestInput::text(title, body, ItemKind::Doc))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        field_str(&record.raw, "database_id")
            .or_else(|| field_str(&record.raw, "parent.database_id"))
    }
}

/// Linear. Auth: API key. Scope: `containers` are team ids, `filters` are states
/// ("state:open"). Cursor: Linear updatedAt. Mapping: a Linear issue becomes a
/// Task (Layer C), with state/priority/dueDate riding the task scalars.
pub struct LinearSpoke {
    transport: Box<dyn RecordTransport>,
}

impl LinearSpoke {
    pub fn new(transport: Box<dyn RecordTransport>) -> Self {
        Self { transport }
    }
}

impl SourceSpoke for LinearSpoke {
    fn source_id(&self) -> &str {
        "linear"
    }
    delegate_fetch!();

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        let title = field_str(&record.raw, "title").unwrap_or_else(|| "(untitled issue)".into());
        let description = field_str(&record.raw, "description").unwrap_or_default();
        // A Linear issue is a Task; its state/priority/due ride the scalars.
        let task = TaskFields {
            status: field_str(&record.raw, "state"),
            priority: field_str(&record.raw, "priority"),
            // Fixtures carry an explicit epoch-ms `dueAtMs`; a live ISO `dueDate`
            // would be parsed here (a documented follow-up).
            due_at_ms: field_i64(&record.raw, "dueAtMs"),
        };
        Ok(IngestInput::note(title, description).as_task(task))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        field_str(&record.raw, "teamId").or_else(|| field_str(&record.raw, "team.id"))
    }
}
