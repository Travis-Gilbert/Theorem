//! The universal-plus-curated contract (Layer A5).
//!
//! [`IngestInput`] is the universal target. Curated spokes ([`crate::sources`])
//! reach it with hand-written mappings; [`MappedSpoke`] reaches it from *data* (a
//! contract descriptor), so any source whose records map onto the item contract
//! becomes a spoke with no bespoke Rust. Because tier-one organizing is
//! source-agnostic (it classifies by content), a `MappedSpoke` source organizes
//! identically to a curated one.
//!
//! Code-grounded divergence (surfaced, not buried): all three [`SourceContract`]
//! variants share one [`IngestFieldMap`] shaping core; they differ in *record
//! discovery* (where the records live and how they are reached), which is the
//! transport's job, not the mapping's. So `FieldMap`, `Schema`, and `Mcp` map a
//! record identically once the transport has produced it.

use commonplace::{IngestInput, ItemKind};
use serde_json::Value;

use crate::spoke::{
    SourceCursor, SourcePage, SourceRecord, SourceResult, SourceScope, SourceSpoke,
};
use crate::transport::RecordTransport;

/// How a record's `kind` is decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FieldKind {
    /// Every record from this source is the same kind.
    Fixed(ItemKind),
    /// Read the kind token from this field path on the record.
    FromField(String),
}

/// Which source fields become which `IngestInput` fields. Field paths are
/// dot-separated into the record's `raw` JSON (e.g. `"fields.title"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IngestFieldMap {
    pub title: String,
    pub body: String,
    pub kind: FieldKind,
    pub tags: Vec<String>,
    /// Optional field holding the record's container (Gmail label, Linear team),
    /// for routing (B1).
    pub container: Option<String>,
}

impl IngestFieldMap {
    /// A text-bodied field map with a fixed kind.
    pub fn text(title: impl Into<String>, body: impl Into<String>, kind: ItemKind) -> Self {
        Self {
            title: title.into(),
            body: body.into(),
            kind: FieldKind::Fixed(kind),
            tags: Vec::new(),
            container: None,
        }
    }

    pub fn with_container(mut self, field: impl Into<String>) -> Self {
        self.container = Some(field.into());
        self
    }

    pub fn with_tags<I, T>(mut self, fields: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.tags = fields.into_iter().map(Into::into).collect();
        self
    }

    fn map(&self, record: &SourceRecord) -> IngestInput {
        let title = field_str(&record.raw, &self.title).unwrap_or_else(|| "(untitled)".into());
        let body = field_str(&record.raw, &self.body).unwrap_or_default();
        let kind = match &self.kind {
            FieldKind::Fixed(kind) => kind.clone(),
            FieldKind::FromField(field) => field_str(&record.raw, field)
                .map(ItemKind::from)
                .unwrap_or(ItemKind::Note),
        };
        let tags: Vec<String> = self
            .tags
            .iter()
            .filter_map(|field| field_str(&record.raw, field))
            .collect();
        IngestInput::text(title, body, kind).with_tags(tags)
    }

    fn container(&self, record: &SourceRecord) -> Option<String> {
        self.container
            .as_ref()
            .and_then(|field| field_str(&record.raw, field))
    }
}

/// Where a `MappedSpoke` gets its mapping. The OpenAPI/GraphQL and MCP variants
/// carry their discovery metadata plus the same field map that does the shaping.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SourceContract {
    /// Field paths from the source record onto `IngestInput` fields.
    FieldMap(IngestFieldMap),
    /// An OpenAPI/GraphQL descriptor: where the records live in the response,
    /// plus the field map for each one.
    Schema(SchemaDescriptor),
    /// An MCP server's resources/read tools as the record source. Discovery
    /// reuses `rustyred-thg-connectors`; the field map shapes each resource.
    Mcp(McpResourceDescriptor),
}

impl SourceContract {
    fn field_map(&self) -> &IngestFieldMap {
        match self {
            SourceContract::FieldMap(map) => map,
            SourceContract::Schema(schema) => &schema.field_map,
            SourceContract::Mcp(mcp) => &mcp.field_map,
        }
    }
}

/// An OpenAPI/GraphQL descriptor: the response path the records sit at (resolved
/// by the transport) plus the field map that shapes each one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaDescriptor {
    pub records_path: String,
    pub field_map: IngestFieldMap,
}

/// An MCP server's resources as the record source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpResourceDescriptor {
    pub server_id: String,
    pub resource: String,
    pub field_map: IngestFieldMap,
}

/// A spoke whose mapping is data, not code (A5). Any source describable by a
/// contract becomes an ingestion spoke without bespoke Rust.
pub struct MappedSpoke {
    source_id: String,
    contract: SourceContract,
    transport: Box<dyn RecordTransport>,
}

impl MappedSpoke {
    pub fn new(
        source_id: impl Into<String>,
        contract: SourceContract,
        transport: Box<dyn RecordTransport>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            contract,
            transport,
        }
    }
}

impl SourceSpoke for MappedSpoke {
    fn source_id(&self) -> &str {
        &self.source_id
    }

    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        self.transport.fetch(scope, cursor)
    }

    fn to_ingest_input(&self, record: &SourceRecord) -> SourceResult<IngestInput> {
        Ok(self.contract.field_map().map(record))
    }

    fn record_container(&self, record: &SourceRecord) -> Option<String> {
        self.contract.field_map().container(record)
    }
}

/// Read a dot-separated field path out of a record's raw JSON as a string.
/// Numbers and booleans render to their literal text so a numeric id maps too.
pub(crate) fn field_str(raw: &Value, path: &str) -> Option<String> {
    let mut node = raw;
    for segment in path.split('.') {
        node = node.get(segment)?;
    }
    match node {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Read a dot-separated field path as an i64 (number, or a numeric string).
pub(crate) fn field_i64(raw: &Value, path: &str) -> Option<i64> {
    let mut node = raw;
    for segment in path.split('.') {
        node = node.get(segment)?;
    }
    node.as_i64().or_else(|| node.as_str().and_then(|s| s.parse().ok()))
}
