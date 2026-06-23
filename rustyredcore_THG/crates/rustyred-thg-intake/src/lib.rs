//! CommonPlace source intake: the front of the object lifecycle.
//!
//! An object enters from a source, gets organized, and gets acted on. This crate
//! builds the *enter* stage that the organize ([`commonplace::organize`]) and act
//! stages depend on: connecting external sources, pulling their data in on a
//! scoped basis, routing what arrives into the existing classifier, and wiring
//! the tier-two residue to delegation.
//!
//! - [`SourceSpoke`] is the ingestion contract: map a source's native records
//!   onto [`IngestInput`](commonplace::IngestInput), the universal capture
//!   target. Curated spokes ([`sources`]) hand-write the mapping; [`MappedSpoke`]
//!   derives it from a contract descriptor.
//! - [`sync_source`] is the driver: fetch the scoped delta, map, ingest through
//!   the batch path, dedupe by source ref, advance the cursor.
//! - [`act`] is the volume-absorber seam: a `NeedsYou` item's suggestion fires a
//!   federated-MCP affordance and the result lands on the object.
//!
//! Sync and tokio-free, generic over the [`GraphStore`](rustyred_thg_core::GraphStore)
//! the [`Commonplace`](commonplace::Commonplace) store wraps - it mirrors the
//! connectors-versus-mcp split and keeps `commonplace` a pure data layer.

pub mod act;
pub mod driver;
pub mod mapped;
pub mod mcp;
pub mod rest;
pub mod sources;
pub mod spoke;
pub mod transport;

pub use act::{
    absorb_residue, accept_suggestion, AbsorbReport, ActSeam, AgentSuggestion, ConnectorActSeam,
    SuggestionAction,
};
pub use driver::{sync_source, SyncReport};
pub use rest::{
    ureq_fetch, GSuiteDriveTransport, GmailHttpTransport, HttpFetch, HttpRequest,
    LinearHttpTransport, NotionHttpTransport, OutlookHttpTransport,
};
pub use mapped::{
    FieldKind, IngestFieldMap, MappedSpoke, McpResourceDescriptor, SchemaDescriptor, SourceContract,
};
pub use mcp::McpRecordTransport;
pub use sources::{GSuiteSpoke, GmailSpoke, LinearSpoke, NotionSpoke, OutlookSpoke};
pub use spoke::{
    SourceCursor, SourceError, SourcePage, SourceRecord, SourceResult, SourceScope, SourceSpoke,
};
pub use transport::{FixtureTransport, RecordTransport};

// The MCP server reach type, re-exported so callers build an `McpRecordTransport`
// (ingestion) or persist an act-side affordance target without depending on the
// connectors crate directly.
pub use rustyred_thg_connectors::ConnectionTarget;
