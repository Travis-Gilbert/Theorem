//! Auto-structuring ingest (plan unit F2).
//!
//! F2 is the no-button path: detect the input kind, embed it, classify it into a
//! collection, file it, link it to similar items, resolve near-duplicate
//! entities, and write the resulting [`Item`](crate::Item). The default embedder
//! is deterministic and local so the acceptance suite is repeatable; production
//! text/SigLIP/RunPod embedders can implement [`Embedder`] behind the same seam.

use std::collections::{BTreeSet, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, InMemoryGraphStore, NodeQuery,
    NodeRecord, RedCoreGraphStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::blob::BlobStore;
use crate::collection::{Collection, CollectionKind};
use crate::content_core::{
    content_core_extract_with_config, ContentCoreExtractionConfig, ContentCoreExtractionError,
    ExtractedDoc,
};
use crate::item::{Item, ItemBody, ItemKind, Residency, SourceRef};
use crate::store::{Commonplace, COLLECTION_LABEL, ITEM_LABEL};

/// Default bounded source-prior boost (B1): added to a candidate collection's
/// cosine when that collection already holds an item from the item's source. Set
/// well below a typical content-match gap so a strong content signal still wins.
pub const DEFAULT_SOURCE_PRIOR_BOOST: f32 = 0.05;

/// Top-level item node property used by the engine's vector designation.
pub const ITEM_EMBEDDING_PROPERTY: &str = "embedding";
/// Top-level collection node property containing the label/cluster embedding.
pub const COLLECTION_EMBEDDING_PROPERTY: &str = "label_embedding";
/// First-class entity node label produced by F2 field extraction.
pub const ENTITY_LABEL: &str = "Entity";
/// Edge from an item to a resolved entity.
pub const MENTIONS_ENTITY_EDGE: &str = "MENTIONS_ENTITY";
const ENTITY_EMBEDDING_PROPERTY: &str = "entity_embedding";

/// A graph store that can expose the engine vector index for CommonPlace items.
///
/// This trait is local to the crate, so the F2 pipeline can support both the
/// in-memory test engine and durable RedCore without changing the core
/// [`GraphStore`] trait.
pub trait EmbeddingGraphStore: GraphStore {
    fn designate_commonplace_item_embedding(&mut self, dimension: usize) -> GraphStoreResult<()>;
    fn search_commonplace_item_embedding(
        &self,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>>;
}

impl EmbeddingGraphStore for InMemoryGraphStore {
    fn designate_commonplace_item_embedding(&mut self, dimension: usize) -> GraphStoreResult<()> {
        self.designate_vector_property(ITEM_LABEL, ITEM_EMBEDDING_PROPERTY, dimension)
    }

    fn search_commonplace_item_embedding(
        &self,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.vector_search(Some(ITEM_LABEL), ITEM_EMBEDDING_PROPERTY, query, k)
    }
}

impl EmbeddingGraphStore for RedCoreGraphStore {
    fn designate_commonplace_item_embedding(&mut self, dimension: usize) -> GraphStoreResult<()> {
        self.designate_vector_property(ITEM_LABEL, ITEM_EMBEDDING_PROPERTY, dimension)
    }

    fn search_commonplace_item_embedding(
        &self,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.vector_search(Some(ITEM_LABEL), ITEM_EMBEDDING_PROPERTY, query, k)
    }
}

/// Input body accepted by the F2 pipeline.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum IngestBody {
    Text {
        text: String,
        kind: ItemKind,
    },
    Link {
        url: String,
        text: String,
    },
    Binary {
        bytes: Vec<u8>,
        mime: Option<String>,
        kind: ItemKind,
    },
}

/// Task scalars carried through the universal capture contract (Layer C). When
/// present on an [`IngestInput`] of kind [`ItemKind::Task`], they are written onto
/// the resulting item's indexed `status`/`priority`/`due_at_ms` scalars, so a
/// source-mapped task (e.g. a Linear issue) lands fully shaped without a bespoke
/// write path.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TaskFields {
    pub status: Option<String>,
    pub priority: Option<String>,
    pub due_at_ms: Option<i64>,
}

impl TaskFields {
    fn apply(&self, mut item: Item) -> Item {
        item.status = self.status.clone();
        item.priority = self.priority.clone();
        item.due_at_ms = self.due_at_ms;
        item
    }
}

/// A no-button capture request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngestInput {
    pub title: String,
    pub body: IngestBody,
    pub source: Option<String>,
    /// The source record this capture came from (A3), for idempotent re-fetch.
    #[serde(default)]
    pub source_ref: Option<SourceRef>,
    pub residency: Residency,
    pub tags: Vec<String>,
    /// Task scalars (Layer C); meaningful only for `Task`-kind captures.
    #[serde(default)]
    pub task: Option<TaskFields>,
}

impl IngestInput {
    pub fn note(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self::text(title, text, ItemKind::Note)
    }

    pub fn document(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self::text(title, text, ItemKind::Doc)
    }

    pub fn text(title: impl Into<String>, text: impl Into<String>, kind: ItemKind) -> Self {
        Self {
            title: title.into(),
            body: IngestBody::Text {
                text: text.into(),
                kind,
            },
            source: None,
            source_ref: None,
            residency: Residency::Local,
            tags: Vec::new(),
            task: None,
        }
    }

    pub fn image(title: impl Into<String>, bytes: Vec<u8>, mime: Option<String>) -> Self {
        Self {
            title: title.into(),
            body: IngestBody::Binary {
                bytes,
                mime,
                kind: ItemKind::Image,
            },
            source: None,
            source_ref: None,
            residency: Residency::Local,
            tags: Vec::new(),
            task: None,
        }
    }

    pub fn link(title: impl Into<String>, url: impl Into<String>, text: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            title: title.into(),
            body: IngestBody::Link {
                url: url.clone(),
                text: text.into(),
            },
            source: Some(url),
            source_ref: None,
            residency: Residency::Local,
            tags: Vec::new(),
            task: None,
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Stamp the source record this capture came from (A3): sets `source` to the
    /// ref's source and records the full ref for idempotent re-fetch.
    pub fn with_source_ref(mut self, source_ref: SourceRef) -> Self {
        self.source = Some(source_ref.source.clone());
        self.source_ref = Some(source_ref);
        self
    }

    /// Attach task scalars (Layer C). Forces the capture to `Task` kind so the
    /// scalars land on a task item.
    pub fn as_task(mut self, task: TaskFields) -> Self {
        if let IngestBody::Text { kind, .. } = &mut self.body {
            *kind = ItemKind::Task;
        }
        self.task = Some(task);
        self
    }

    pub fn with_residency(mut self, residency: Residency) -> Self {
        self.residency = residency;
        self
    }

    pub fn with_tags<I, T>(mut self, tags: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.tags = tags.into_iter().map(Into::into).collect();
        self
    }

    fn item_kind(&self) -> ItemKind {
        // Task scalars force `Task` kind for any body shape (e.g. a link-bodied
        // capture turned into a task), so it appears in task queries.
        if self.task.is_some() {
            return ItemKind::Task;
        }
        match &self.body {
            IngestBody::Text { kind, .. } | IngestBody::Binary { kind, .. } => kind.clone(),
            IngestBody::Link { .. } => ItemKind::Link,
        }
    }

    fn searchable_text(&self) -> String {
        match &self.body {
            IngestBody::Text { text, .. } => format!("{}\n{}", self.title, text),
            IngestBody::Link { url, text } => format!("{}\n{}\n{}", self.title, url, text),
            IngestBody::Binary { mime, .. } => {
                format!("{}\n{}", self.title, mime.clone().unwrap_or_default())
            }
        }
    }
}

/// Embedding seam for text and image/document bytes.
pub trait Embedder {
    fn dimension(&self) -> usize;
    fn embed_text(&self, text: &str) -> GraphStoreResult<Vec<f32>>;
    fn embed_image(&self, bytes: &[u8], mime: Option<&str>) -> GraphStoreResult<Vec<f32>>;
}

/// Deterministic local embedder for tests and offline-first runs.
#[derive(Clone, Copy, Debug)]
pub struct DeterministicEmbedder {
    dimension: usize,
}

impl Default for DeterministicEmbedder {
    fn default() -> Self {
        Self { dimension: 16 }
    }
}

impl DeterministicEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(1),
        }
    }

    fn embed_tokens<'a>(&self, tokens: impl IntoIterator<Item = &'a str>) -> Vec<f32> {
        let mut vector = vec![0.0; self.dimension];
        for token in tokens {
            if token.is_empty() {
                continue;
            }
            let digest = Sha256::digest(token.as_bytes());
            let idx = usize::from(digest[0]) % self.dimension;
            let weight = 1.0 + (token.len().min(16) as f32 / 16.0);
            vector[idx] += weight;
        }
        normalize(vector)
    }
}

impl Embedder for DeterministicEmbedder {
    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed_text(&self, text: &str) -> GraphStoreResult<Vec<f32>> {
        let tokens = tokenize(text);
        Ok(self.embed_tokens(tokens.iter().map(String::as_str)))
    }

    fn embed_image(&self, bytes: &[u8], mime: Option<&str>) -> GraphStoreResult<Vec<f32>> {
        let mut vector = vec![0.0; self.dimension];
        for chunk in bytes.chunks(8) {
            let digest = Sha256::digest(chunk);
            let idx = usize::from(digest[0]) % self.dimension;
            vector[idx] += 1.0 + (chunk.len() as f32 / 8.0);
        }
        if let Some(mime) = mime {
            for token in tokenize(mime) {
                let digest = Sha256::digest(token.as_bytes());
                let idx = usize::from(digest[0]) % self.dimension;
                vector[idx] += 0.5;
            }
        }
        Ok(normalize(vector))
    }
}

/// Similarity edge written by ingest.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarityLink {
    pub item_id: String,
    pub score: f32,
}

/// Entity mention resolved to a canonical entity node.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ResolvedEntity {
    pub mention: String,
    pub entity_id: String,
    pub canonical: String,
    pub score: f32,
}

/// One ranked candidate collection for an item, by cosine to the collection's
/// label embedding (the same signal `best_collection` gates on).
#[derive(Clone, Debug, PartialEq)]
pub struct ClassificationRank {
    pub collection_id: String,
    pub collection_name: String,
    pub score: f32, // cosine 0..1
}

/// Engine-sourced classification of an item: every collection ranked by cosine
/// to its label embedding, best first. The caller applies its own threshold.
#[derive(Clone, Debug, Default)]
pub struct Classification {
    /// Ranked candidate collections, best first. Empty if the item has no
    /// embedding or there are no collections with label embeddings.
    pub ranked: Vec<ClassificationRank>,
}

impl Classification {
    /// The best candidate, if any.
    pub fn best(&self) -> Option<&ClassificationRank> {
        self.ranked.first()
    }

    /// The best candidate's score, or 0.0 when there is no candidate.
    pub fn confidence(&self) -> f32 {
        self.ranked.first().map(|rank| rank.score).unwrap_or(0.0)
    }
}

/// The observable F2 receipt.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngestReceipt {
    pub item: Item,
    pub collection: Collection,
    pub folder_path: String,
    pub embedding: Vec<f32>,
    pub similar_items: Vec<SimilarityLink>,
    pub entities: Vec<ResolvedEntity>,
    pub extraction: Option<IngestExtractionReceipt>,
}

/// Observable content-core extraction status for one ingest item.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngestExtractionReceipt {
    pub status: String,
    pub source: Option<String>,
    pub reason: Option<String>,
    pub extracted_text_len: usize,
    pub detected_type: Option<String>,
    pub engine: Option<String>,
    pub metadata: Value,
}

/// Auto-structuring ingest pipeline.
#[derive(Clone, Debug)]
pub struct IngestPipeline<E = DeterministicEmbedder> {
    embedder: E,
    collection_threshold: f32,
    similarity_threshold: f32,
    entity_threshold: f32,
    content_core_config: ContentCoreExtractionConfig,
}

impl Default for IngestPipeline<DeterministicEmbedder> {
    fn default() -> Self {
        Self {
            embedder: DeterministicEmbedder::default(),
            collection_threshold: 0.58,
            similarity_threshold: 0.62,
            entity_threshold: 0.86,
            content_core_config: ContentCoreExtractionConfig::from_env(),
        }
    }
}

impl<E> IngestPipeline<E>
where
    E: Embedder,
{
    pub fn new(embedder: E) -> Self {
        Self {
            embedder,
            collection_threshold: 0.58,
            similarity_threshold: 0.62,
            entity_threshold: 0.86,
            content_core_config: ContentCoreExtractionConfig::from_env(),
        }
    }

    pub fn with_collection_threshold(mut self, threshold: f32) -> Self {
        self.collection_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    pub fn with_similarity_threshold(mut self, threshold: f32) -> Self {
        self.similarity_threshold = threshold.clamp(0.0, 1.0);
        self
    }

    pub fn with_content_core_config(mut self, config: ContentCoreExtractionConfig) -> Self {
        self.content_core_config = config;
        self
    }

    pub fn without_content_core(mut self) -> Self {
        self.content_core_config.enabled = false;
        self
    }

    pub fn ingest<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        input: IngestInput,
    ) -> GraphStoreResult<IngestReceipt>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        commonplace
            .store_mut()
            .designate_commonplace_item_embedding(self.embedder.dimension())?;
        let mut prior_items = commonplace.all_items()?;
        let (input, extraction) = self.prepare_content_core_input(input);
        self.ingest_one(commonplace, input, &mut prior_items, None, extraction)
    }

    /// Ingest a batch, amortizing the prior-items snapshot and the vector-index
    /// designation across the whole batch (A4): one snapshot and one designation,
    /// not N of each. Similarity for each input is computed against the snapshot
    /// plus the items earlier in the same batch, so the resulting graph is
    /// identical to ingesting the same inputs one at a time through `ingest`.
    pub fn ingest_batch<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        inputs: Vec<IngestInput>,
    ) -> GraphStoreResult<Vec<IngestReceipt>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        commonplace
            .store_mut()
            .designate_commonplace_item_embedding(self.embedder.dimension())?;
        let mut prior_items = commonplace.all_items()?;
        let mut receipts = Vec::with_capacity(inputs.len());
        for input in inputs {
            let (input, extraction) = self.prepare_content_core_input(input);
            receipts.push(self.ingest_one(
                commonplace,
                input,
                &mut prior_items,
                None,
                extraction,
            )?);
        }
        Ok(receipts)
    }

    /// Ingest one input hard-routed into a named collection (B1): bypass the
    /// cosine collection choice and file into `collection_name` (create-or-get),
    /// regardless of content. Still embeds, links similars, and resolves entities.
    pub fn ingest_routed<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        input: IngestInput,
        collection_name: &str,
    ) -> GraphStoreResult<IngestReceipt>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        commonplace
            .store_mut()
            .designate_commonplace_item_embedding(self.embedder.dimension())?;
        let forced =
            commonplace.get_or_create_collection(collection_name, CollectionKind::Manual)?;
        let mut prior_items = commonplace.all_items()?;
        let (input, extraction) = self.prepare_content_core_input(input);
        self.ingest_one(
            commonplace,
            input,
            &mut prior_items,
            Some(forced),
            extraction,
        )
    }

    /// The per-item ingest core shared by `ingest`, `ingest_batch`, and
    /// `ingest_routed`. `prior_items` is the running set of already-stored items
    /// (snapshot plus earlier batch items); the just-written item is appended so
    /// the next call sees it as a similarity candidate. `forced` bypasses cosine
    /// collection choice (the routed path).
    fn ingest_one<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        input: IngestInput,
        prior_items: &mut Vec<Item>,
        forced: Option<Collection>,
        extraction: Option<IngestExtractionReceipt>,
    ) -> GraphStoreResult<IngestReceipt>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let embedding = self.embed_input(&input)?;
        let searchable_text = input.searchable_text();

        // A3 idempotency: a re-fetched source record updates the same item in
        // place (reusing its id upserts every outgoing edge), never a duplicate.
        let existing_item = match &input.source_ref {
            Some(source_ref) => {
                commonplace.item_by_source_ref(&source_ref.source, &source_ref.external_id)?
            }
            None => None,
        };

        let collection = match (forced, &existing_item) {
            (Some(collection), _) => collection,
            // Sticky on update: keep the item's current collection. No durable
            // edge-delete is available, so re-classifying to a different
            // collection would leave the item a member of both; staying put
            // keeps membership single and correct. (A genuine re-file is an
            // explicit `ingest_routed`/move, not a side effect of re-sync.)
            (None, Some(existing)) => match existing.collections.first() {
                Some(collection_id) => match commonplace.get_collection(collection_id)? {
                    Some(collection) => collection,
                    None => self.choose_or_create_collection(commonplace, &input, &embedding)?,
                },
                None => self.choose_or_create_collection(commonplace, &input, &embedding)?,
            },
            (None, None) => self.choose_or_create_collection(commonplace, &input, &embedding)?,
        };
        let folder_path = folder_path_for(&collection.name, &input.title);

        let mut item = Item::new(input.item_kind(), input.title.clone())
            .with_residency(input.residency)
            .with_tags(input.tags.clone())
            .with_collections([collection.id.clone()])
            .with_classification(collection.name.clone())
            .with_embedding_ref("commonplace:embedding:inline:v1")
            .with_extra(ITEM_EMBEDDING_PROPERTY, json!(embedding))
            .with_extra("folder_path", json!(folder_path.clone()))
            .with_extra("auto_structured", json!(true));

        if let Some(existing) = &existing_item {
            // Reuse the id and preserve the original creation time (put_item only
            // sets created_at_ms when it is 0, so carrying it forward stops a
            // re-fetch from resetting it).
            item = item.with_id(existing.id.clone());
            item.created_at_ms = existing.created_at_ms;
        }
        if let Some(source) = input.source.clone() {
            item = item.with_source(source);
        }
        if let Some(source_ref) = input.source_ref.clone() {
            item = item.with_source_ref(source_ref);
        }
        if let Some(task) = &input.task {
            item = task.apply(item);
        }
        if let Some(extraction) = &extraction {
            item = item.with_extra("content_core_extraction", json!(extraction));
        }
        item = match &input.body {
            IngestBody::Text { text, .. } => item.with_text(text.clone()),
            IngestBody::Link { text, .. } => item.with_text(text.clone()),
            IngestBody::Binary { bytes, mime, .. } => {
                let content_hash = commonplace.blobs().put(bytes)?;
                item.with_body(ItemBody::Blob {
                    content_hash,
                    byte_len: bytes.len() as u64,
                    mime: mime.clone(),
                })
            }
        };

        let item = commonplace.put_item(item)?;
        let similar_items =
            self.write_similarity_edges(commonplace, &item.id, prior_items.as_slice(), &embedding)?;
        let entities = self.resolve_entities(commonplace, &item.id, &searchable_text)?;
        prior_items.push(item.clone());

        Ok(IngestReceipt {
            item,
            collection,
            folder_path,
            embedding,
            similar_items,
            entities,
            extraction,
        })
    }

    fn prepare_content_core_input(
        &self,
        input: IngestInput,
    ) -> (IngestInput, Option<IngestExtractionReceipt>) {
        prepare_content_core_input_with(input, &self.content_core_config, |source, config| {
            content_core_extract_with_config(source, config)
        })
    }

    pub fn search<S, B>(
        &self,
        commonplace: &Commonplace<S, B>,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let embedding = self.embedder.embed_text(query)?;
        self.search_embedding(commonplace, &embedding, k)
    }

    pub fn search_embedding<S, B>(
        &self,
        commonplace: &Commonplace<S, B>,
        embedding: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        commonplace
            .store()
            .search_commonplace_item_embedding(embedding, k)
    }

    fn embed_input(&self, input: &IngestInput) -> GraphStoreResult<Vec<f32>> {
        match &input.body {
            IngestBody::Binary { bytes, mime, .. } if input.item_kind() == ItemKind::Image => {
                self.embedder.embed_image(bytes, mime.as_deref())
            }
            _ => self.embedder.embed_text(&input.searchable_text()),
        }
    }

    fn choose_or_create_collection<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        input: &IngestInput,
        embedding: &[f32],
    ) -> GraphStoreResult<Collection>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        if let Some(existing) = self.best_collection(commonplace, embedding)? {
            return Ok(existing);
        }

        let name = infer_collection_name(input);
        let collection = commonplace.create_collection(name, CollectionKind::Auto)?;
        let mut node = commonplace
            .store()
            .get_node(&collection.id)
            .cloned()
            .ok_or_else(|| GraphStoreError::new("collection_missing", "new collection missing"))?;
        if let Some(properties) = node.properties.as_object_mut() {
            properties.insert(COLLECTION_EMBEDDING_PROPERTY.to_string(), json!(embedding));
            properties.insert(
                "seed_terms".to_string(),
                json!(tokenize(&input.searchable_text())),
            );
        }
        commonplace.store_mut().upsert_node(node)?;
        Ok(collection)
    }

    fn best_collection<S, B>(
        &self,
        commonplace: &Commonplace<S, B>,
        embedding: &[f32],
    ) -> GraphStoreResult<Option<Collection>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let mut best: Option<(Collection, f32)> = None;
        for node in commonplace
            .store()
            .query_nodes(NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX))
        {
            let Some(candidate) = float_array(&node.properties, COLLECTION_EMBEDDING_PROPERTY)
            else {
                continue;
            };
            if candidate.len() != embedding.len() {
                continue;
            }
            let score = cosine(embedding, &candidate);
            if score >= self.collection_threshold
                && best
                    .as_ref()
                    .map(|(_, best_score)| score > *best_score)
                    .unwrap_or(true)
            {
                if let Some(collection) = commonplace.get_collection(&node.id)? {
                    best = Some((collection, score));
                }
            }
        }
        Ok(best.map(|(collection, _)| collection))
    }

    /// Classify an already-stored item against the live collections: read the
    /// item's stored embedding and rank every collection's label embedding by
    /// cosine, best first. Unlike [`best_collection`](Self::best_collection),
    /// this is NOT gated by `collection_threshold` (the caller applies its own
    /// ceiling); returns an empty [`Classification`] when the item carries no
    /// embedding or no collection has a label embedding.
    pub fn classify_item<S, B>(
        &self,
        commonplace: &Commonplace<S, B>,
        item: &Item,
    ) -> GraphStoreResult<Classification>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        classify_item_ranking(commonplace, item)
    }

    /// Classify like [`classify_item`](Self::classify_item) but add a small,
    /// bounded source prior (B1): a candidate collection that already holds an
    /// item from the same `source` as `item` gets `boost` added to its cosine,
    /// then the list is re-sorted. The prior is a separate additive term, not a
    /// replacement, so a strong content match still wins and `classify_item`
    /// itself stays source-agnostic. `boost` is clamped to a bounded range.
    ///
    /// `boost` ceiling: pass [`DEFAULT_SOURCE_PRIOR_BOOST`] for the default dial.
    // ponytail: O(members) scan per candidate collection; fine at personal-DB
    // scale. If a tenant grows huge, precompute a (collection, source) presence
    // index instead.
    pub fn classify_item_with_source_prior<S, B>(
        &self,
        commonplace: &Commonplace<S, B>,
        item: &Item,
        boost: f32,
    ) -> GraphStoreResult<Classification>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let mut classification = self.classify_item(commonplace, item)?;
        let Some(source) = item.source.as_deref() else {
            return Ok(classification);
        };
        let boost = boost.clamp(0.0, 0.25);
        if boost > 0.0 {
            for rank in &mut classification.ranked {
                if collection_holds_source(commonplace, &rank.collection_id, source)? {
                    rank.score = (rank.score + boost).min(1.0);
                }
            }
            classification.ranked.sort_by(|left, right| {
                right
                    .score
                    .partial_cmp(&left.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| left.collection_id.cmp(&right.collection_id))
            });
        }
        Ok(classification)
    }

    fn write_similarity_edges<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        item_id: &str,
        prior_items: &[Item],
        embedding: &[f32],
    ) -> GraphStoreResult<Vec<SimilarityLink>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let mut links = Vec::new();
        for prior in prior_items {
            // Never link an item to itself (possible on an idempotent re-ingest,
            // where the item appears in its own prior snapshot under the same id).
            if prior.id == item_id {
                continue;
            }
            let Some(candidate) = prior
                .extra
                .get(ITEM_EMBEDDING_PROPERTY)
                .and_then(value_to_f32_vec)
            else {
                continue;
            };
            if candidate.len() != embedding.len() {
                continue;
            }
            let score = cosine(embedding, &candidate);
            if score >= self.similarity_threshold {
                commonplace.add_similarity(item_id, &prior.id, score)?;
                links.push(SimilarityLink {
                    item_id: prior.id.clone(),
                    score,
                });
            }
        }
        links.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(links)
    }

    fn resolve_entities<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        item_id: &str,
        text: &str,
    ) -> GraphStoreResult<Vec<ResolvedEntity>>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let mut resolved = Vec::new();
        let mut seen = HashSet::new();
        for mention in extract_entity_mentions(text) {
            if !seen.insert(canonical_entity(&mention)) {
                continue;
            }
            let entity = self.resolve_entity(commonplace, &mention)?;
            let edge = EdgeRecord::new(
                format!("mentions:{item_id}:{}", entity.entity_id),
                item_id,
                MENTIONS_ENTITY_EDGE,
                &entity.entity_id,
                json!({
                    "mention": entity.mention,
                    "canonical": entity.canonical,
                    "score": entity.score,
                }),
            )
            .with_confidence(entity.score as f64);
            commonplace.store_mut().upsert_edge(edge)?;
            resolved.push(entity);
        }
        Ok(resolved)
    }

    fn resolve_entity<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        mention: &str,
    ) -> GraphStoreResult<ResolvedEntity>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let canonical = canonical_entity(mention);
        let embedding = self.embedder.embed_text(&canonical)?;
        let mut best: Option<(String, String, f32)> = None;
        for node in commonplace
            .store()
            .query_nodes(NodeQuery::label(ENTITY_LABEL).with_limit(usize::MAX))
        {
            let existing_canonical = node
                .properties
                .get("canonical")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let score = if existing_canonical == canonical {
                1.0
            } else {
                node.properties
                    .get(ENTITY_EMBEDDING_PROPERTY)
                    .and_then(value_to_f32_vec)
                    .filter(|candidate| candidate.len() == embedding.len())
                    .map(|candidate| cosine(&embedding, &candidate))
                    .unwrap_or(0.0)
            };
            if score >= self.entity_threshold
                && best
                    .as_ref()
                    .map(|(_, _, best_score)| score > *best_score)
                    .unwrap_or(true)
            {
                best = Some((node.id, existing_canonical, score));
            }
        }

        if let Some((entity_id, canonical, score)) = best {
            return Ok(ResolvedEntity {
                mention: mention.to_string(),
                entity_id,
                canonical,
                score,
            });
        }

        let entity_id = format!("entity:{}", slug(&canonical));
        let record = NodeRecord::new(
            entity_id.clone(),
            [ENTITY_LABEL],
            json!({
                "name": mention.trim(),
                "canonical": canonical,
                ENTITY_EMBEDDING_PROPERTY: embedding,
            }),
        );
        commonplace.store_mut().upsert_node(record)?;
        Ok(ResolvedEntity {
            mention: mention.to_string(),
            entity_id,
            canonical,
            score: 1.0,
        })
    }
}

impl<S, B> Commonplace<S, B>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    pub fn ingest(&mut self, input: IngestInput) -> GraphStoreResult<IngestReceipt> {
        IngestPipeline::default().ingest(self, input)
    }
}

fn prepare_content_core_input_with<F>(
    input: IngestInput,
    config: &ContentCoreExtractionConfig,
    mut extract: F,
) -> (IngestInput, Option<IngestExtractionReceipt>)
where
    F: FnMut(
        &str,
        &ContentCoreExtractionConfig,
    ) -> Result<ExtractedDoc, ContentCoreExtractionError>,
{
    let route = content_core_route(&input);
    match route {
        ContentCoreRoute::None => (input, None),
        ContentCoreRoute::Url { url } => {
            if !config.enabled {
                return (
                    input,
                    Some(skipped_extraction(
                        Some(url),
                        ContentCoreExtractionError::Disabled.reason(),
                    )),
                );
            }
            match extract(&url, config) {
                Ok(doc) => apply_url_extraction(input, url, doc),
                Err(error) => (input, Some(skipped_extraction(Some(url), error.reason()))),
            }
        }
        ContentCoreRoute::Binary { mime, extension } => {
            let receipt_source = stable_extraction_source(&input);
            if !config.enabled {
                return (
                    input,
                    Some(skipped_extraction(
                        Some(receipt_source),
                        ContentCoreExtractionError::Disabled.reason(),
                    )),
                );
            }
            let IngestBody::Binary { bytes, .. } = &input.body else {
                return (input, None);
            };
            let temp = match TempExtractionFile::new(bytes, extension.as_deref()) {
                Ok(temp) => temp,
                Err(error) => {
                    return (
                        input,
                        Some(skipped_extraction(
                            Some(receipt_source),
                            format!("could not stage content-core input: {error}"),
                        )),
                    );
                }
            };
            let source = temp.path_string();
            match extract(&source, config) {
                Ok(doc) => apply_binary_extraction(input, receipt_source, mime, doc),
                Err(error) => (
                    input,
                    Some(skipped_extraction(Some(receipt_source), error.reason())),
                ),
            }
        }
    }
}

fn apply_url_extraction(
    mut input: IngestInput,
    url: String,
    doc: ExtractedDoc,
) -> (IngestInput, Option<IngestExtractionReceipt>) {
    if doc.text.trim().is_empty() {
        return (
            input,
            Some(skipped_extraction(
                Some(url),
                "content-core returned empty extracted text",
            )),
        );
    }
    input.body = IngestBody::Link {
        url: url.clone(),
        text: doc.text.clone(),
    };
    input.source = Some(url.clone());
    let receipt = extracted_receipt(Some(url), &doc);
    (input, Some(receipt))
}

fn apply_binary_extraction(
    mut input: IngestInput,
    receipt_source: String,
    mime: Option<String>,
    doc: ExtractedDoc,
) -> (IngestInput, Option<IngestExtractionReceipt>) {
    if doc.text.trim().is_empty() {
        return (
            input,
            Some(skipped_extraction(
                Some(receipt_source),
                "content-core returned empty extracted text",
            )),
        );
    }
    let kind = input.item_kind();
    input.body = IngestBody::Text {
        text: doc.text.clone(),
        kind,
    };
    let mut receipt = extracted_receipt(Some(receipt_source), &doc);
    if receipt.detected_type.is_none() {
        receipt.detected_type = mime;
    }
    (input, Some(receipt))
}

fn stable_extraction_source(input: &IngestInput) -> String {
    input
        .source
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            input
                .source_ref
                .as_ref()
                .map(|source_ref| source_ref.source.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .or_else(|| {
            let title = input.title.trim();
            (!title.is_empty()).then(|| title.to_string())
        })
        .unwrap_or_else(|| "binary ingest item".to_string())
}

fn extracted_receipt(source: Option<String>, doc: &ExtractedDoc) -> IngestExtractionReceipt {
    IngestExtractionReceipt {
        status: "extracted".to_string(),
        source,
        reason: None,
        extracted_text_len: doc.text.len(),
        detected_type: doc.detected_type.clone(),
        engine: doc.engine.clone(),
        metadata: doc.metadata.clone(),
    }
}

fn skipped_extraction(
    source: Option<String>,
    reason: impl Into<String>,
) -> IngestExtractionReceipt {
    IngestExtractionReceipt {
        status: "skipped".to_string(),
        source,
        reason: Some(reason.into()),
        extracted_text_len: 0,
        detected_type: None,
        engine: None,
        metadata: json!({}),
    }
}

enum ContentCoreRoute {
    None,
    Url {
        url: String,
    },
    Binary {
        mime: Option<String>,
        extension: Option<String>,
    },
}

fn content_core_route(input: &IngestInput) -> ContentCoreRoute {
    match &input.body {
        IngestBody::Text { .. } => ContentCoreRoute::None,
        IngestBody::Link { url, .. } if is_http_url(url) => {
            ContentCoreRoute::Url { url: url.clone() }
        }
        IngestBody::Link { .. } => ContentCoreRoute::None,
        IngestBody::Binary { mime, kind, .. } => {
            if *kind == ItemKind::Image || is_image_mime(mime.as_deref()) {
                return ContentCoreRoute::None;
            }
            let extension = extension_from_title(&input.title)
                .or_else(|| extension_for_mime(mime.as_deref()).map(str::to_string));
            if binary_supported_by_content_core(mime.as_deref(), extension.as_deref()) {
                ContentCoreRoute::Binary {
                    mime: mime.clone(),
                    extension,
                }
            } else {
                ContentCoreRoute::None
            }
        }
    }
}

fn binary_supported_by_content_core(mime: Option<&str>, extension: Option<&str>) -> bool {
    if let Some(mime) = mime.map(|value| value.trim().to_ascii_lowercase()) {
        if mime.starts_with("audio/") || mime.starts_with("video/") {
            return true;
        }
        if matches!(
            mime.as_str(),
            "application/pdf"
                | "application/epub+zip"
                | "application/msword"
                | "application/vnd.ms-excel"
                | "application/vnd.ms-powerpoint"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
        ) {
            return true;
        }
        if mime == "text/plain" || mime == "text/markdown" || mime == "text/x-markdown" {
            return false;
        }
    }
    matches!(
        extension.map(|value| value.trim().to_ascii_lowercase()),
        Some(ext)
            if matches!(
                ext.as_str(),
                "pdf"
                    | "doc"
                    | "docx"
                    | "ppt"
                    | "pptx"
                    | "xls"
                    | "xlsx"
                    | "epub"
                    | "mp3"
                    | "wav"
                    | "m4a"
                    | "flac"
                    | "ogg"
                    | "mp4"
                    | "avi"
                    | "mov"
                    | "mkv"
            )
    )
}

fn is_http_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn is_image_mime(mime: Option<&str>) -> bool {
    mime.map(|value| value.trim().to_ascii_lowercase().starts_with("image/"))
        .unwrap_or(false)
}

fn extension_from_title(title: &str) -> Option<String> {
    Path::new(title)
        .extension()
        .and_then(|value| value.to_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_start_matches('.').to_ascii_lowercase())
}

fn extension_for_mime(mime: Option<&str>) -> Option<&'static str> {
    match mime
        .map(|value| value.trim().to_ascii_lowercase())?
        .as_str()
    {
        "application/pdf" => Some("pdf"),
        "application/epub+zip" => Some("epub"),
        "application/msword" => Some("doc"),
        "application/vnd.ms-excel" => Some("xls"),
        "application/vnd.ms-powerpoint" => Some("ppt"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => Some("docx"),
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => Some("pptx"),
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some("xlsx"),
        "audio/mpeg" => Some("mp3"),
        "audio/wav" | "audio/x-wav" => Some("wav"),
        "audio/mp4" | "audio/x-m4a" => Some("m4a"),
        "audio/flac" => Some("flac"),
        "audio/ogg" => Some("ogg"),
        "video/mp4" => Some("mp4"),
        "video/x-msvideo" => Some("avi"),
        "video/quicktime" => Some("mov"),
        "video/x-matroska" => Some("mkv"),
        _ => None,
    }
}

struct TempExtractionFile {
    path: PathBuf,
}

impl TempExtractionFile {
    fn new(bytes: &[u8], extension: Option<&str>) -> std::io::Result<Self> {
        let suffix = extension
            .map(|value| value.trim().trim_start_matches('.'))
            .filter(|value| !value.is_empty())
            .unwrap_or("bin");
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        for attempt in 0..16 {
            let mut path = std::env::temp_dir();
            path.push(format!(
                "commonplace-content-core-{}-{seed}-{attempt}.{suffix}",
                std::process::id()
            ));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    file.write_all(bytes)?;
                    return Ok(Self { path });
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate unique content-core temp file",
        ))
    }

    fn path_string(&self) -> String {
        self.path.to_string_lossy().to_string()
    }
}

impl Drop for TempExtractionFile {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn infer_collection_name(input: &IngestInput) -> String {
    if let Some(tag) = input.tags.iter().find(|tag| !tag.trim().is_empty()) {
        return title_case(tag);
    }
    if input.item_kind() == ItemKind::Image {
        return "Images".to_string();
    }
    let tokens = tokenize(&input.searchable_text());
    if tokens.iter().any(|token| {
        matches!(
            token.as_str(),
            "contract" | "client" | "matter" | "case" | "plaintiff" | "defendant" | "court"
        )
    }) {
        return "Legal".to_string();
    }
    tokens
        .into_iter()
        .find(|token| token.len() > 3)
        .map(|token| title_case(&token))
        .unwrap_or_else(|| "Inbox".to_string())
}

fn folder_path_for(collection_name: &str, title: &str) -> String {
    format!("collections/{}/{}", slug(collection_name), slug(title))
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|raw| {
            let token = normalize_token(raw);
            (!token.is_empty()).then_some(token)
        })
        .collect()
}

fn normalize_token(raw: &str) -> String {
    let lower = raw.trim().to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "corporation" => "corp",
        "incorporated" => "inc",
        "companies" => "company",
        other => other,
    };
    if mapped.len() > 4 && mapped.ends_with('s') {
        mapped.trim_end_matches('s').to_string()
    } else {
        mapped.to_string()
    }
}

fn title_case(value: &str) -> String {
    let words = value
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.trim().is_empty())
        .map(|word| word.trim().to_ascii_lowercase())
        .collect::<Vec<_>>();
    if words.is_empty() {
        return "Inbox".to_string();
    }
    words
        .into_iter()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn slug(value: &str) -> String {
    let mut output = String::new();
    let mut last_dash = false;
    for ch in value.trim().to_ascii_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            output.push(ch);
            last_dash = false;
        } else if !last_dash {
            output.push('-');
            last_dash = true;
        }
    }
    let trimmed = output.trim_matches('-');
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right.iter())
        .map(|(a, b)| a * b)
        .sum::<f32>();
    let left_norm = left.iter().map(|value| value * value).sum::<f32>().sqrt();
    let right_norm = right.iter().map(|value| value * value).sum::<f32>().sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn float_array(properties: &Value, key: &str) -> Option<Vec<f32>> {
    properties.get(key).and_then(value_to_f32_vec)
}

/// Rank every collection by cosine to an item's stored embedding, best first
/// (the body of [`IngestPipeline::classify_item`], lifted to a free function so
/// the two-tier [`crate::organize::decide`] can classify with no embedder in
/// play). Ungated: the caller applies its own ceiling.
pub fn classify_item_ranking<S, B>(
    commonplace: &Commonplace<S, B>,
    item: &Item,
) -> GraphStoreResult<Classification>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let Some(embedding) = item
        .extra
        .get(ITEM_EMBEDDING_PROPERTY)
        .and_then(value_to_f32_vec)
    else {
        return Ok(Classification::default());
    };

    let mut ranked: Vec<ClassificationRank> = Vec::new();
    for node in commonplace
        .store()
        .query_nodes(NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX))
    {
        let Some(candidate) = float_array(&node.properties, COLLECTION_EMBEDDING_PROPERTY) else {
            continue;
        };
        if candidate.len() != embedding.len() {
            continue;
        }
        let score = cosine(&embedding, &candidate);
        let collection_name = commonplace
            .get_collection(&node.id)?
            .map(|collection| collection.name)
            .unwrap_or_default();
        ranked.push(ClassificationRank {
            collection_id: node.id.clone(),
            collection_name,
            score,
        });
    }
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.collection_id.cmp(&right.collection_id))
    });

    Ok(Classification { ranked })
}

/// Whether a collection already holds an item from `source` (B1 source prior).
fn collection_holds_source<S, B>(
    commonplace: &Commonplace<S, B>,
    collection_id: &str,
    source: &str,
) -> GraphStoreResult<bool>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    Ok(commonplace
        .collection_items(collection_id)?
        .iter()
        .any(|item| item.source.as_deref() == Some(source)))
}

fn value_to_f32_vec(value: &Value) -> Option<Vec<f32>> {
    value
        .as_array()?
        .iter()
        .map(|entry| entry.as_f64().map(|value| value as f32))
        .collect()
}

fn extract_entity_mentions(text: &str) -> Vec<String> {
    let mut mentions = BTreeSet::new();
    for segment in text.split(['\n', ';']) {
        let Some((key, value)) = segment.split_once(':') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if !matches!(
            key.as_str(),
            "client" | "company" | "entity" | "matter" | "organization" | "org"
        ) {
            continue;
        }
        let mention = value
            .split(['.', ',', '(', ')'])
            .next()
            .unwrap_or_default()
            .trim();
        if mention.len() >= 2 {
            mentions.insert(mention.to_string());
        }
    }
    mentions.into_iter().collect()
}

fn canonical_entity(value: &str) -> String {
    tokenize(value)
        .into_iter()
        .filter(|token| !matches!(token.as_str(), "the" | "a" | "an"))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;
    use crate::content_core::{ContentCoreCommand, ContentCoreExtractionConfig};

    fn test_config() -> ContentCoreExtractionConfig {
        ContentCoreExtractionConfig {
            enabled: true,
            timeout: Duration::from_secs(1),
            commands: vec![ContentCoreCommand::new("content-core", Vec::new())],
            env: Default::default(),
        }
    }

    fn doc(text: &str, detected_type: &str, engine: &str) -> ExtractedDoc {
        ExtractedDoc {
            text: text.to_string(),
            title: None,
            source_type: Some("file".to_string()),
            detected_type: Some(detected_type.to_string()),
            engine: Some(engine.to_string()),
            metadata: json!({ "pages": 2 }),
        }
    }

    #[test]
    fn content_core_url_replaces_link_text_and_records_engine() {
        let input = IngestInput::link("Example", "https://example.com/report", "placeholder");
        let (input, receipt) =
            prepare_content_core_input_with(input, &test_config(), |source, _| {
                assert_eq!(source, "https://example.com/report");
                Ok(ExtractedDoc {
                    text: "Client: Acme Corp. Extracted article text.".to_string(),
                    title: None,
                    source_type: Some("url".to_string()),
                    detected_type: Some("article".to_string()),
                    engine: Some("firecrawl".to_string()),
                    metadata: json!({ "engine": "firecrawl" }),
                })
            });

        assert!(matches!(
            input.body,
            IngestBody::Link { ref text, .. } if text.contains("Extracted article")
        ));
        let receipt = receipt.expect("extraction receipt");
        assert_eq!(receipt.status, "extracted");
        assert_eq!(receipt.detected_type.as_deref(), Some("article"));
        assert_eq!(receipt.engine.as_deref(), Some("firecrawl"));
    }

    #[test]
    fn content_core_pdf_binary_becomes_text_for_organizer() {
        let input = IngestInput {
            title: "contract.pdf".to_string(),
            body: IngestBody::Binary {
                bytes: b"%PDF-1.4 fake".to_vec(),
                mime: Some("application/pdf".to_string()),
                kind: ItemKind::Doc,
            },
            source: None,
            source_ref: None,
            residency: Residency::Local,
            tags: Vec::new(),
            task: None,
        };
        let (input, receipt) =
            prepare_content_core_input_with(input, &test_config(), |source, _| {
                assert!(
                    source.ends_with(".pdf"),
                    "temp source should preserve pdf extension"
                );
                assert!(
                    Path::new(source).exists(),
                    "temp source exists during extraction"
                );
                Ok(doc(
                    "Client: Acme Corp. Contract text from PDF.",
                    "application/pdf",
                    "docling",
                ))
            });

        assert!(matches!(
            input.body,
            IngestBody::Text { ref text, kind: ItemKind::Doc } if text.contains("Contract text")
        ));
        let receipt = receipt.expect("extraction receipt");
        assert_eq!(receipt.status, "extracted");
        assert_eq!(receipt.source.as_deref(), Some("contract.pdf"));
        assert_eq!(receipt.detected_type.as_deref(), Some("application/pdf"));
        assert_eq!(receipt.engine.as_deref(), Some("docling"));
    }

    #[test]
    fn content_core_absent_keeps_supported_binary_and_records_reason() {
        let input = IngestInput {
            title: "meeting.mp3".to_string(),
            body: IngestBody::Binary {
                bytes: b"audio".to_vec(),
                mime: Some("audio/mpeg".to_string()),
                kind: ItemKind::File,
            },
            source: None,
            source_ref: None,
            residency: Residency::Local,
            tags: Vec::new(),
            task: None,
        };
        let (input, receipt) =
            prepare_content_core_input_with(input, &test_config(), |_source, _| {
                Err(ContentCoreExtractionError::Unavailable(
                    "spawn content-core".to_string(),
                ))
            });

        assert!(matches!(input.body, IngestBody::Binary { .. }));
        let receipt = receipt.expect("skip receipt");
        assert_eq!(receipt.source.as_deref(), Some("meeting.mp3"));
        let reason = receipt.reason.expect("skip reason");
        assert!(reason.contains("content-core unavailable"));
    }

    #[test]
    fn native_text_markdown_and_images_do_not_route_to_content_core() {
        let text = IngestInput::document("notes.md", "# Plain markdown");
        let (text, text_receipt) =
            prepare_content_core_input_with(text, &test_config(), |_source, _| {
                panic!("plain text must not call content-core")
            });
        assert!(matches!(text.body, IngestBody::Text { .. }));
        assert!(text_receipt.is_none());

        let image = IngestInput::image(
            "screenshot.png",
            b"image".to_vec(),
            Some("image/png".to_string()),
        );
        let (image, image_receipt) =
            prepare_content_core_input_with(image, &test_config(), |_source, _| {
                panic!("images must stay on the vision spine")
            });
        assert_eq!(image.item_kind(), ItemKind::Image);
        assert!(image_receipt.is_none());
    }
}
