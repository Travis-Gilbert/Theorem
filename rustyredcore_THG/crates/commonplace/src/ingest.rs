//! Auto-structuring ingest (plan unit F2).
//!
//! F2 is the no-button path: detect the input kind, embed it, classify it into a
//! collection, file it, link it to similar items, resolve near-duplicate
//! entities, and write the resulting [`Item`](crate::Item). The default embedder
//! is deterministic and local so the acceptance suite is repeatable; production
//! text/SigLIP/RunPod embedders can implement [`Embedder`] behind the same seam.

use std::collections::{BTreeSet, HashSet};

use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, InMemoryGraphStore, NodeQuery,
    NodeRecord, RedCoreGraphStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::blob::BlobStore;
use crate::collection::{Collection, CollectionKind};
use crate::item::{Item, ItemBody, ItemKind, Residency};
use crate::store::{Commonplace, COLLECTION_LABEL, ITEM_LABEL};

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

/// A no-button capture request.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IngestInput {
    pub title: String,
    pub body: IngestBody,
    pub source: Option<String>,
    pub residency: Residency,
    pub tags: Vec<String>,
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
            residency: Residency::Local,
            tags: Vec::new(),
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
            residency: Residency::Local,
            tags: Vec::new(),
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
            residency: Residency::Local,
            tags: Vec::new(),
        }
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
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
#[derive(Clone, Debug)]
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
}

/// Auto-structuring ingest pipeline.
#[derive(Clone, Debug)]
pub struct IngestPipeline<E = DeterministicEmbedder> {
    embedder: E,
    collection_threshold: f32,
    similarity_threshold: f32,
    entity_threshold: f32,
}

impl Default for IngestPipeline<DeterministicEmbedder> {
    fn default() -> Self {
        Self {
            embedder: DeterministicEmbedder::default(),
            collection_threshold: 0.58,
            similarity_threshold: 0.62,
            entity_threshold: 0.86,
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

    pub fn ingest<S, B>(
        &self,
        commonplace: &mut Commonplace<S, B>,
        input: IngestInput,
    ) -> GraphStoreResult<IngestReceipt>
    where
        S: EmbeddingGraphStore,
        B: BlobStore,
    {
        let embedding = self.embed_input(&input)?;
        commonplace
            .store_mut()
            .designate_commonplace_item_embedding(self.embedder.dimension())?;

        let prior_items = commonplace.all_items()?;
        let searchable_text = input.searchable_text();
        let collection = self.choose_or_create_collection(commonplace, &input, &embedding)?;
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

        if let Some(source) = input.source.clone() {
            item = item.with_source(source);
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
            self.write_similarity_edges(commonplace, &item.id, &prior_items, &embedding)?;
        let entities = self.resolve_entities(commonplace, &item.id, &searchable_text)?;

        Ok(IngestReceipt {
            item,
            collection,
            folder_path,
            embedding,
            similar_items,
            entities,
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
            let Some(candidate) = float_array(&node.properties, COLLECTION_EMBEDDING_PROPERTY)
            else {
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
