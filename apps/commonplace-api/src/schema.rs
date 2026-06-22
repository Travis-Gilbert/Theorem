//! The consumer GraphQL profile (plan unit F3).
//!
//! Exposes the CommonPlace object model as a typed schema: queries for items,
//! collections, and similarity search; mutations for ingest and edit. Every
//! resolver authorizes the request's API key before touching the store, so an
//! invalid key is rejected before any data access.
//!
//! The store is fixed to the in-memory backing here so the seam is fully
//! testable in-process; the identical schema runs over the durable
//! `RedCoreGraphStore` + `DiskObjectStore` backing (both impl the traits this
//! needs) by swapping the type alias, which is the named follow-up for the
//! durable self-hosted binary.

use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

use async_graphql::{
    Context, EmptySubscription, Enum, Error, InputObject, Object, Result, Schema, SimpleObject,
};
use commonplace::{
    BlobStore, Collection, Commonplace, EmbeddingGraphStore, InMemoryBlobStore, IngestInput,
    IngestPipeline, Item, ItemBody, ItemKind, Residency, COLLECTION_LABEL,
};
use rustyred_thg_core::{DiskObjectStore, InMemoryGraphStore, NodeQuery, RedCoreGraphStore};

use crate::auth::{ApiKeyRegistry, ApiKeyToken, Principal};
use crate::briefing::{briefing as run_briefing, Briefing, BriefingConfig, ConnectedItem};
use crate::discover::{discover as run_discover, CandidateLink, DiscoverConfig};
use crate::portability::{self, ExportDocument};
use crate::retrieve::{
    answer_from_provenance, retrieve_grounding, AnswerKind, AnswerModel, AskConfig, AskResult,
    NoModel, RetrievedItem,
};

/// The default in-memory store backing (tests + the no-data-dir binary path).
pub type ApiStore = Commonplace<InMemoryGraphStore, InMemoryBlobStore>;
/// A shared, lockable instance store, generic over the backing.
pub type SharedStore<S, B> = Arc<Mutex<Commonplace<S, B>>>;
/// The in-memory shared store.
pub type InMemoryShared = SharedStore<InMemoryGraphStore, InMemoryBlobStore>;
/// The durable shared store (RedCore + disk) for a self-hosted instance.
pub type DurableShared = SharedStore<RedCoreGraphStore, DiskObjectStore>;
/// The consumer schema over the in-memory backing (default / tests).
pub type ConsumerSchema = Schema<
    Query<InMemoryGraphStore, InMemoryBlobStore>,
    Mutation<InMemoryGraphStore, InMemoryBlobStore>,
    EmptySubscription,
>;
/// The consumer schema over the durable RedCore + disk backing.
pub type DurableSchema = Schema<
    Query<RedCoreGraphStore, DiskObjectStore>,
    Mutation<RedCoreGraphStore, DiskObjectStore>,
    EmptySubscription,
>;

/// An item, in the consumer API shape.
#[derive(SimpleObject)]
pub struct ItemGql {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body_text: Option<String>,
    pub blob_hash: Option<String>,
    pub mime: Option<String>,
    pub source: Option<String>,
    pub residency: String,
    pub tags: Vec<String>,
    pub collections: Vec<String>,
    pub classification: Option<String>,
    pub path: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl From<Item> for ItemGql {
    fn from(item: Item) -> Self {
        let (body_text, blob_hash, mime) = match item.body {
            ItemBody::Inline { text } => (Some(text), None, None),
            ItemBody::Blob {
                content_hash, mime, ..
            } => (None, Some(content_hash), mime),
            ItemBody::Empty => (None, None, None),
        };
        let path = item
            .extra
            .get("path")
            .or_else(|| item.extra.get("folder_path"))
            .and_then(|value| value.as_str())
            .map(str::to_string);
        Self {
            id: item.id,
            kind: item.kind.as_str().to_string(),
            title: item.title,
            body_text,
            blob_hash,
            mime,
            source: item.source,
            residency: item.residency.as_str().to_string(),
            tags: item.tags,
            collections: item.collections,
            classification: item.classification,
            path,
            created_at_ms: item.created_at_ms,
            updated_at_ms: item.updated_at_ms,
        }
    }
}

/// A collection, in the consumer API shape.
#[derive(SimpleObject)]
pub struct CollectionGql {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_at_ms: i64,
}

impl From<Collection> for CollectionGql {
    fn from(collection: Collection) -> Self {
        Self {
            id: collection.id,
            name: collection.name,
            kind: collection.kind.as_str().to_string(),
            created_at_ms: collection.created_at_ms,
        }
    }
}

/// A similarity-search hit.
#[derive(SimpleObject)]
pub struct SearchHitGql {
    pub item: ItemGql,
    pub score: f64,
}

/// Input for the auto-structuring ingest mutation.
#[derive(InputObject)]
pub struct IngestInputGql {
    pub title: String,
    pub text: String,
    /// One of file/note/link/image/doc, or any custom kind. Defaults to note.
    pub kind: Option<String>,
    pub tags: Option<Vec<String>>,
    pub source: Option<String>,
    pub residency: Option<String>,
}

/// How an ask answer was produced.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum AnswerKindGql {
    /// Synthesized by a configured generative model.
    Model,
    /// Extracted from the retrieved items (no generative model configured).
    Extractive,
    /// No matching items were found.
    Empty,
}

impl From<AnswerKind> for AnswerKindGql {
    fn from(kind: AnswerKind) -> Self {
        match kind {
            AnswerKind::Model => AnswerKindGql::Model,
            AnswerKind::Extractive => AnswerKindGql::Extractive,
            AnswerKind::Empty => AnswerKindGql::Empty,
        }
    }
}

/// One grounding item behind an answer.
#[derive(SimpleObject)]
pub struct ProvenanceGql {
    pub item: ItemGql,
    pub score: f64,
    /// Which retrieval arms surfaced this item (vector / lexical / graph).
    pub arms: Vec<String>,
}

impl From<RetrievedItem> for ProvenanceGql {
    fn from(hit: RetrievedItem) -> Self {
        Self {
            item: ItemGql::from(hit.item),
            score: hit.score,
            arms: hit.arms,
        }
    }
}

/// An answer grounded in the user's items, each traceable to its source.
#[derive(SimpleObject)]
pub struct AskResultGql {
    pub answer: String,
    pub answer_kind: AnswerKindGql,
    pub provenance: Vec<ProvenanceGql>,
}

impl From<AskResult> for AskResultGql {
    fn from(result: AskResult) -> Self {
        Self {
            answer: result.answer,
            answer_kind: AnswerKindGql::from(result.answer_kind),
            provenance: result
                .provenance
                .into_iter()
                .map(ProvenanceGql::from)
                .collect(),
        }
    }
}

/// An item plus how it connects to the rest of the store.
#[derive(SimpleObject)]
pub struct ConnectedItemGql {
    pub item: ItemGql,
    pub connections: i32,
    pub related: Vec<ItemGql>,
}

impl From<ConnectedItem> for ConnectedItemGql {
    fn from(connected: ConnectedItem) -> Self {
        Self {
            item: ItemGql::from(connected.item),
            connections: connected.connections as i32,
            related: connected.related.into_iter().map(ItemGql::from).collect(),
        }
    }
}

/// Proactive briefing over the store: what is new, what connects, what is open.
#[derive(SimpleObject)]
pub struct BriefingGql {
    pub recent: Vec<ItemGql>,
    pub newly_connected: Vec<ConnectedItemGql>,
    pub open_threads: Vec<ItemGql>,
}

impl From<Briefing> for BriefingGql {
    fn from(briefing: Briefing) -> Self {
        Self {
            recent: briefing.recent.into_iter().map(ItemGql::from).collect(),
            newly_connected: briefing
                .newly_connected
                .into_iter()
                .map(ConnectedItemGql::from)
                .collect(),
            open_threads: briefing
                .open_threads
                .into_iter()
                .map(ItemGql::from)
                .collect(),
        }
    }
}

/// A proposed connection between two not-yet-linked items.
#[derive(SimpleObject)]
pub struct CandidateLinkGql {
    pub a: ItemGql,
    pub b: ItemGql,
    pub similarity: f64,
    pub reason: String,
}

impl From<CandidateLink> for CandidateLinkGql {
    fn from(link: CandidateLink) -> Self {
        Self {
            a: ItemGql::from(link.a),
            b: ItemGql::from(link.b),
            similarity: link.similarity,
            reason: link.reason,
        }
    }
}

/// Serialization format for export.
#[derive(Enum, Copy, Clone, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    /// Lossless JSON (reimports without loss).
    Json,
    /// Human-readable markdown (one-way rendering).
    Markdown,
}

/// What an import wrote.
#[derive(SimpleObject)]
pub struct ImportResultGql {
    pub imported: i32,
    pub collections: i32,
}

fn principal(ctx: &Context<'_>) -> Result<Principal> {
    let token = ctx
        .data_opt::<ApiKeyToken>()
        .ok_or_else(|| Error::new("missing API key: present a key via the x-api-key header"))?;
    let registry = ctx.data::<Arc<ApiKeyRegistry>>()?;
    registry
        .resolve(&token.0)
        .cloned()
        .ok_or_else(|| Error::new("invalid API key"))
}

fn shared<S, B>(ctx: &Context<'_>) -> Result<SharedStore<S, B>>
where
    S: Send + Sync + 'static,
    B: Send + Sync + 'static,
{
    ctx.data::<SharedStore<S, B>>().cloned()
}

fn store_err(error: rustyred_thg_core::GraphStoreError) -> Error {
    Error::new(format!("{error:?}"))
}

/// Consumer read API.
pub struct Query<S, B>(PhantomData<fn() -> (S, B)>);

#[Object(name = "Query")]
impl<S, B> Query<S, B>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    /// One item by id.
    async fn item(&self, ctx: &Context<'_>, id: String) -> Result<Option<ItemGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        Ok(cp.get_item(&id).map_err(store_err)?.map(ItemGql::from))
    }

    /// All items, optionally filtered to a kind.
    async fn items(&self, ctx: &Context<'_>, kind: Option<String>) -> Result<Vec<ItemGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let items = match kind {
            Some(kind) => cp.items_by_kind(&ItemKind::from(kind)).map_err(store_err)?,
            None => cp.all_items().map_err(store_err)?,
        };
        Ok(items.into_iter().map(ItemGql::from).collect())
    }

    /// One collection by id.
    async fn collection(&self, ctx: &Context<'_>, id: String) -> Result<Option<CollectionGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        Ok(cp
            .get_collection(&id)
            .map_err(store_err)?
            .map(CollectionGql::from))
    }

    /// All collections.
    async fn collections(&self, ctx: &Context<'_>) -> Result<Vec<CollectionGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let ids: Vec<String> = cp
            .store()
            .query_nodes(NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX))
            .into_iter()
            .map(|node| node.id)
            .collect();
        let mut collections = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(collection) = cp.get_collection(&id).map_err(store_err)? {
                collections.push(CollectionGql::from(collection));
            }
        }
        Ok(collections)
    }

    /// Items in a collection.
    async fn collection_items(&self, ctx: &Context<'_>, id: String) -> Result<Vec<ItemGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        Ok(cp
            .collection_items(&id)
            .map_err(store_err)?
            .into_iter()
            .map(ItemGql::from)
            .collect())
    }

    /// Similarity search over items.
    async fn search(
        &self,
        ctx: &Context<'_>,
        query: String,
        k: Option<i32>,
    ) -> Result<Vec<SearchHitGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let k = k.unwrap_or(10).max(1) as usize;
        let hits = IngestPipeline::default()
            .search(&cp, &query, k)
            .map_err(store_err)?;
        let mut results = Vec::with_capacity(hits.len());
        for (id, score) in hits {
            if let Some(item) = cp.get_item(&id).map_err(store_err)? {
                results.push(SearchHitGql {
                    item: ItemGql::from(item),
                    score: score as f64,
                });
            }
        }
        Ok(results)
    }

    /// Ask a question over your store: unified graph + vector + lexical retrieval
    /// (reciprocal-rank fusion) with per-item provenance, answered by the
    /// configured model or an honest extractive fallback. Each provenance entry
    /// is the item a part of the answer is grounded in.
    async fn ask(
        &self,
        ctx: &Context<'_>,
        question: String,
        k: Option<i32>,
    ) -> Result<AskResultGql> {
        principal(ctx)?;
        let model = ctx.data::<Arc<dyn AnswerModel>>()?.clone();
        let store = shared::<S, B>(ctx)?;
        let config = AskConfig {
            k: k.unwrap_or(5).max(1) as usize,
            ..AskConfig::default()
        };
        let provenance = {
            let cp = store
                .lock()
                .map_err(|_| Error::new("store lock poisoned"))?;
            retrieve_grounding(&*cp, &question, &config).map_err(store_err)?
        };
        let result = answer_from_provenance(model.as_ref(), &question, provenance);
        Ok(AskResultGql::from(result))
    }

    /// Proactive briefing: recent, newly-connected, and open-thread items
    /// surfaced from the store without being asked.
    async fn briefing(
        &self,
        ctx: &Context<'_>,
        recent_limit: Option<i32>,
        connected_limit: Option<i32>,
        open_limit: Option<i32>,
    ) -> Result<BriefingGql> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let config = BriefingConfig {
            recent_limit: recent_limit.unwrap_or(10).max(1) as usize,
            connected_limit: connected_limit.unwrap_or(10).max(1) as usize,
            open_limit: open_limit.unwrap_or(10).max(1) as usize,
            ..BriefingConfig::default()
        };
        let briefing = run_briefing(&*cp, &config).map_err(store_err)?;
        Ok(BriefingGql::from(briefing))
    }

    /// Discovery: propose ranked candidate links between items that are
    /// semantically similar but not yet connected.
    async fn discover(
        &self,
        ctx: &Context<'_>,
        min_similarity: Option<f64>,
        max_results: Option<i32>,
    ) -> Result<Vec<CandidateLinkGql>> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let config = DiscoverConfig {
            min_similarity: min_similarity.unwrap_or(0.5),
            max_results: max_results.unwrap_or(20).max(1) as usize,
            ..DiscoverConfig::default()
        };
        let links = run_discover(&*cp, &config).map_err(store_err)?;
        Ok(links.into_iter().map(CandidateLinkGql::from).collect())
    }

    /// Export the whole store: lossless JSON (default) or human-readable
    /// markdown. The JSON output reimports via `importItems` without loss.
    async fn export(&self, ctx: &Context<'_>, format: Option<ExportFormat>) -> Result<String> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let output = match format.unwrap_or(ExportFormat::Json) {
            ExportFormat::Json => portability::export_json(&*cp).map_err(store_err)?,
            ExportFormat::Markdown => portability::export_markdown(&*cp).map_err(store_err)?,
        };
        Ok(output)
    }
}

/// Consumer write API.
pub struct Mutation<S, B>(PhantomData<fn() -> (S, B)>);

#[Object(name = "Mutation")]
impl<S, B> Mutation<S, B>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    /// Auto-structuring ingest: embed, classify, file, link, resolve entities.
    async fn ingest(&self, ctx: &Context<'_>, input: IngestInputGql) -> Result<ItemGql> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let kind = input.kind.map(ItemKind::from).unwrap_or(ItemKind::Note);
        let mut request = IngestInput::text(input.title, input.text, kind);
        if let Some(tags) = input.tags {
            request = request.with_tags(tags);
        }
        if let Some(source) = input.source {
            request = request.with_source(source);
        }
        if let Some(residency) = input.residency {
            request = request.with_residency(Residency::from(residency));
        }
        let receipt = IngestPipeline::default()
            .ingest(&mut cp, request)
            .map_err(store_err)?;
        Ok(ItemGql::from(receipt.item))
    }

    /// Create a plain note item (no auto-structuring).
    async fn put_note(
        &self,
        ctx: &Context<'_>,
        title: String,
        text: String,
        tags: Option<Vec<String>>,
    ) -> Result<ItemGql> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let mut item = Item::note(title, text);
        if let Some(tags) = tags {
            item = item.with_tags(tags);
        }
        Ok(ItemGql::from(cp.put_item(item).map_err(store_err)?))
    }

    /// Edit an existing item's title, tags, or residency (in place by id).
    async fn edit_item(
        &self,
        ctx: &Context<'_>,
        id: String,
        title: Option<String>,
        tags: Option<Vec<String>>,
        residency: Option<String>,
    ) -> Result<ItemGql> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let mut item = cp
            .get_item(&id)
            .map_err(store_err)?
            .ok_or_else(|| Error::new("item not found"))?;
        if let Some(title) = title {
            item.title = title;
        }
        if let Some(tags) = tags {
            item.tags = tags;
        }
        if let Some(residency) = residency {
            item.residency = Residency::from(residency);
        }
        Ok(ItemGql::from(cp.put_item(item).map_err(store_err)?))
    }

    /// Create a manual collection.
    async fn create_collection(&self, ctx: &Context<'_>, name: String) -> Result<CollectionGql> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        Ok(CollectionGql::from(
            cp.create_collection(name, commonplace::CollectionKind::Manual)
                .map_err(store_err)?,
        ))
    }

    /// Add an item to a collection.
    async fn add_to_collection(
        &self,
        ctx: &Context<'_>,
        item_id: String,
        collection_id: String,
    ) -> Result<bool> {
        principal(ctx)?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        cp.add_to_collection(&item_id, &collection_id)
            .map_err(store_err)?;
        Ok(true)
    }

    /// Import a JSON export document (from `export`), recreating items and
    /// collections with their original ids so memberships survive.
    async fn import_items(&self, ctx: &Context<'_>, data: String) -> Result<ImportResultGql> {
        principal(ctx)?;
        let document: ExportDocument = serde_json::from_str(&data)
            .map_err(|error| Error::new(format!("invalid export JSON: {error}")))?;
        let store = shared::<S, B>(ctx)?;
        let mut cp = store
            .lock()
            .map_err(|_| Error::new("store lock poisoned"))?;
        let summary = portability::import(&mut cp, &document).map_err(store_err)?;
        Ok(ImportResultGql {
            imported: summary.items as i32,
            collections: summary.collections as i32,
        })
    }
}

/// Build the consumer schema over an instance store and its key registry, with
/// no generative answer model (ask uses the extractive fallback).
pub fn build_schema<S, B>(
    store: SharedStore<S, B>,
    registry: Arc<ApiKeyRegistry>,
) -> Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    build_schema_with_model(store, registry, Arc::new(NoModel))
}

/// Build the schema with a specific answer model (for example local Gemma via
/// an OpenAI-compatible endpoint) for generative answers behind the same
/// retrieval.
pub fn build_schema_with_model<S, B>(
    store: SharedStore<S, B>,
    registry: Arc<ApiKeyRegistry>,
    model: Arc<dyn AnswerModel>,
) -> Schema<Query<S, B>, Mutation<S, B>, EmptySubscription>
where
    S: EmbeddingGraphStore + Send + Sync + 'static,
    B: BlobStore + Send + Sync + 'static,
{
    Schema::build(Query(PhantomData), Mutation(PhantomData), EmptySubscription)
        .data(store)
        .data(registry)
        .data(model)
        .finish()
}
