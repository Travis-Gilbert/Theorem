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
pub mod organize;
pub mod portability;
pub mod repo_connect;
pub mod retrieve;
pub mod schema;
pub mod serve;

pub use auth::{ApiKeyRegistry, ApiKeyToken, Principal};
pub use briefing::{briefing, Briefing, BriefingConfig, ConnectedItem};
pub use discover::{discover, CandidateLink, DiscoverConfig};
pub use organize::{
    organize, DailyProgress, OrganizeConfig, OrganizeFiled, OrganizeGroup, OrganizeItem,
    OrganizeSnapshot, OrganizedToday, Subtask, Timeframe,
};
pub use portability::{
    export, export_json, export_markdown, import, ExportDocument, ImportSummary, EXPORT_VERSION,
};
pub use repo_connect::{
    connector_from_env, EngineRepositoryConnector, EnvGitCredentialResolver,
    GitCredentialResolverRef, RepositoryConnectInput, RepositoryConnectReceipt,
    RepositoryConnector, RepositoryConnectorRef,
};
pub use retrieve::{
    answer_model_from_env, ask, AnswerKind, AnswerModel, AskConfig, AskResult,
    LocalOpenAiAnswerModel, NoModel, RetrievedItem,
};
pub use schema::{
    build_schema, build_schema_with_model, build_schema_with_model_and_repository_connector,
    AnswerKindGql, ApiStore, AskResultGql, BriefingGql, CandidateLinkGql, CollectionGql,
    ConnectedItemGql, ConsumerSchema, DurableSchema, DurableShared, ExportFormat, ImportResultGql,
    InMemoryShared, IngestInputGql, ItemGql, Mutation, ProvenanceGql, Query,
    RepositoryConnectInputGql, RepositoryConnectReceiptGql, SearchHitGql, SharedStore,
};
pub use serve::{build_router_with_model, run_from_env, serve_loopback, serve_loopback_with_ready};

use std::path::Path;
use std::sync::{Arc, Mutex};

use commonplace::{Commonplace, InMemoryBlobStore};
use rustyred_thg_core::{
    DiskObjectStore, GraphStoreResult, InMemoryGraphStore, RedCoreDurability, RedCoreGraphStore,
    RedCoreOptions,
};

/// A fresh in-memory CommonPlace instance store (one instance / one dataset).
pub fn in_memory_store() -> InMemoryShared {
    Arc::new(Mutex::new(Commonplace::new(
        InMemoryGraphStore::new(),
        InMemoryBlobStore::new(),
    )))
}

/// A durable CommonPlace instance store rooted at `dir`: a `RedCoreGraphStore`
/// over `<dir>/graph` plus a `DiskObjectStore` over `<dir>/blobs`. Items written
/// through it survive a restart (re-opening the same `dir` rehydrates them).
pub fn redcore_store(dir: impl AsRef<Path>) -> GraphStoreResult<DurableShared> {
    let root = dir.as_ref();
    // fsync per commit: a self-hosted instance must survive an abrupt stop, not
    // just a graceful shutdown (the default AofEverysec can lose the last second).
    let options = RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        ..RedCoreOptions::default()
    };
    let store = RedCoreGraphStore::open(root.join("graph"), options)?;
    let blobs = DiskObjectStore::open(root.join("blobs"))?;
    Ok(Arc::new(Mutex::new(Commonplace::new(store, blobs))))
}
