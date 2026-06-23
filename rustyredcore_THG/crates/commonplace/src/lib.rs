//! CommonPlace: the consumer projection over the RustyRedCore-THG substrate.
//!
//! CommonPlace is a portable, auto-organizing personal database. This crate is
//! its data layer (plan unit F1): the universal object model ([`Item`],
//! [`Collection`], [`Tag`]) stored graph-natively so the substrate's existing
//! PPR / HNSW / community machinery runs over personal data directly. Anything
//! a user saves is an `Item`; a `File` is an `Item` whose body is a
//! content-addressed blob (see [`blob`]).
//!
//! Layering (held apart on purpose, per the plan):
//! - The store is graph-native and portable: it speaks the [`rustyred_thg_core`]
//!   [`GraphStore`](rustyred_thg_core::GraphStore) trait, so it runs in-memory
//!   for tests, durably on disk via `RedCoreGraphStore`, or against a server.
//! - The object model is generic enough to store anything yet structured enough
//!   to organize and query.
//!
//! Code-grounded divergence from the plan text (surfaced, not buried): F1 names
//! its home as "a `commonplace` module over `rustyred-thg-catalog`". The catalog
//! is sqlx/Postgres (a tenant/billing/auth catalog that needs a live DB). The
//! object model itself lands graph-native over `GraphStore` instead, because
//! that is what makes Items first-class graph citizens for F2's classification
//! and I1's unified retrieve, and it keeps the acceptance suite DB-free. The
//! catalog stays the home for the tenant/key/billing rows that F3 will read.

pub mod blob;
pub mod collection;
pub mod ingest;
pub mod item;
pub mod organize;
pub mod store;
pub mod tag;

pub use blob::{content_hash, BlobStore, InMemoryBlobStore};
pub use collection::{Collection, CollectionKind};
pub use ingest::{
    classify_item_ranking, Classification, ClassificationRank, DeterministicEmbedder, Embedder,
    EmbeddingGraphStore, IngestBody, IngestInput, IngestPipeline, IngestReceipt, ResolvedEntity,
    SimilarityLink, TaskFields, COLLECTION_EMBEDDING_PROPERTY, DEFAULT_SOURCE_PRIOR_BOOST,
    ENTITY_LABEL, ITEM_EMBEDDING_PROPERTY, MENTIONS_ENTITY_EDGE,
};
pub use item::{Item, ItemBody, ItemKind, Residency, SourceRef};
pub use organize::{
    decide, route, NeedsYouReason, OrganizeDecision, OrganizePolicy, RoutingRule,
};
pub use store::{
    Commonplace, ABOUT_EDGE, COLLECTION_LABEL, DEPENDS_ON_EDGE, HAS_TAG_EDGE, IN_COLLECTION_EDGE,
    ITEM_LABEL, SIMILAR_TO_EDGE, SOURCE_REF_KEY_PROPERTY, SUBTASK_OF_EDGE, TAG_LABEL,
    WORKED_BY_EDGE,
};
pub use tag::{tag_id, Tag};
