//! CommonPlace interoperability API (plan unit F3).
//!
//! The universal connection point: a typed consumer GraphQL profile over the
//! CommonPlace object model, gated by per-instance API keys, so any front end or
//! self-hosted instance talks to the database with one API. Anything that speaks
//! this schema plus a valid key is a client.
//!
//! The schema is store-agnostic in shape; this crate wires it to an in-memory
//! instance store ([`in_memory_store`]) that is fully testable in-process. A
//! durable self-hosted instance swaps the [`schema::ApiStore`] backing to
//! `RedCoreGraphStore` + `DiskObjectStore` behind the identical schema (named
//! follow-up; F1's restart test already proves that backing is durable).

pub mod auth;
pub mod briefing;
pub mod discover;
pub mod mcp;
pub mod portability;
pub mod retrieve;
pub mod schema;

pub use auth::{ApiKeyRegistry, ApiKeyToken, Principal};
pub use briefing::{briefing, Briefing, BriefingConfig, ConnectedItem};
pub use discover::{discover, CandidateLink, DiscoverConfig};
pub use portability::{
    export, export_json, export_markdown, import, ExportDocument, ImportSummary, EXPORT_VERSION,
};
pub use retrieve::{ask, AnswerKind, AnswerModel, AskConfig, AskResult, NoModel, RetrievedItem};
pub use schema::{
    build_schema, build_schema_with_model, AnswerKindGql, ApiStore, AskResultGql, BriefingGql,
    CandidateLinkGql, CollectionGql, ConnectedItemGql, ConsumerSchema, ExportFormat,
    ImportResultGql, IngestInputGql, ItemGql, Mutation, ProvenanceGql, Query, SearchHitGql,
    SharedStore,
};

use std::sync::{Arc, Mutex};

use commonplace::{Commonplace, InMemoryBlobStore};
use rustyred_thg_core::InMemoryGraphStore;

/// A fresh in-memory CommonPlace instance store (one instance / one dataset).
pub fn in_memory_store() -> SharedStore {
    Arc::new(Mutex::new(Commonplace::new(
        InMemoryGraphStore::new(),
        InMemoryBlobStore::new(),
    )))
}
