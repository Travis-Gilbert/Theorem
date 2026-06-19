//! The CommonPlace consumer object model (plan unit F1).
//!
//! [`Item`] is the universal unit: anything stored in CommonPlace is an `Item`.
//! It is deliberately generic (any [`ItemKind`]) yet structured enough to
//! organize and query. A `File` is an `Item` whose [`ItemBody`] is a
//! content-addressed blob (see [`crate::blob`]); a note's body is inline text.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The kind of an `Item`. Open-ended so the store holds anything: the ingest
/// pipeline (F2) may coin new kinds via [`ItemKind::Other`]. Serializes to a
/// canonical lowercase string so a node property filter (`kind = "note"`) works.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub enum ItemKind {
    File,
    Note,
    Link,
    Image,
    Doc,
    Other(String),
}

impl ItemKind {
    /// The canonical lowercase token stored on the graph node.
    pub fn as_str(&self) -> &str {
        match self {
            ItemKind::File => "file",
            ItemKind::Note => "note",
            ItemKind::Link => "link",
            ItemKind::Image => "image",
            ItemKind::Doc => "doc",
            ItemKind::Other(other) => other.as_str(),
        }
    }
}

impl From<ItemKind> for String {
    fn from(kind: ItemKind) -> Self {
        kind.as_str().to_string()
    }
}

impl From<String> for ItemKind {
    fn from(value: String) -> Self {
        match value.as_str() {
            "file" => ItemKind::File,
            "note" => ItemKind::Note,
            "link" => ItemKind::Link,
            "image" => ItemKind::Image,
            "doc" => ItemKind::Doc,
            _ => ItemKind::Other(value),
        }
    }
}

/// Where an `Item` lives. Residency is the hook the sync layer (S1) reads:
/// `Local` never leaves the device, `Synced` replicates to the hosted instance,
/// `Hosted` lives in the cloud.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(into = "String", from = "String")]
pub enum Residency {
    #[default]
    Local,
    Synced,
    Hosted,
}

impl Residency {
    pub fn as_str(&self) -> &'static str {
        match self {
            Residency::Local => "local",
            Residency::Synced => "synced",
            Residency::Hosted => "hosted",
        }
    }
}

impl From<Residency> for String {
    fn from(residency: Residency) -> Self {
        residency.as_str().to_string()
    }
}

impl From<String> for Residency {
    fn from(value: String) -> Self {
        match value.as_str() {
            "synced" => Residency::Synced,
            "hosted" => Residency::Hosted,
            _ => Residency::Local,
        }
    }
}

/// The body of an `Item`. A note is `Inline`; a file is a `Blob` reference into
/// the content-addressed blob store. `Empty` covers items that are pure
/// metadata (e.g. a bare link whose content has not been fetched yet).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "body_kind", rename_all = "snake_case")]
pub enum ItemBody {
    #[default]
    Empty,
    Inline {
        text: String,
    },
    Blob {
        content_hash: String,
        byte_len: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime: Option<String>,
    },
}

/// The universal personal-database unit.
///
/// `tags` is the caller-facing label list (stored as a node property and
/// projected to `Tag` nodes + `HAS_TAG` edges for graph algorithms). `collections`
/// is edge-canonical: it is reconstructed from `IN_COLLECTION` edges on read, so
/// the field on a freshly-built `Item` is an input request, while the field on a
/// loaded `Item` reflects actual membership.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    #[serde(default)]
    pub id: String,
    pub kind: ItemKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub body: ItemBody,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub residency: Residency,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collections: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding_ref: Option<String>,
    /// The raw embedding vector, stored as a top-level node property so the
    /// substrate's vector index (designated on `(Item, embedding)`) picks it up
    /// automatically on write. Written by the F2 ingest pipeline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    #[serde(default)]
    pub created_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub extra: Map<String, Value>,
}

impl Item {
    /// A new, unpersisted item. `id`, `created_at_ms`, and `updated_at_ms` are
    /// assigned by [`crate::Commonplace::put_item`] when the item is written.
    pub fn new(kind: ItemKind, title: impl Into<String>) -> Self {
        Self {
            id: String::new(),
            kind,
            title: title.into(),
            body: ItemBody::Empty,
            source: None,
            residency: Residency::Local,
            tags: Vec::new(),
            collections: Vec::new(),
            embedding_ref: None,
            embedding: None,
            classification: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            extra: Map::new(),
        }
    }

    /// Convenience: a `Note` item with inline text.
    pub fn note(title: impl Into<String>, text: impl Into<String>) -> Self {
        Self::new(ItemKind::Note, title).with_text(text)
    }

    pub fn with_id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn with_body(mut self, body: ItemBody) -> Self {
        self.body = body;
        self
    }

    pub fn with_text(mut self, text: impl Into<String>) -> Self {
        self.body = ItemBody::Inline { text: text.into() };
        self
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

    pub fn with_collections<I, T>(mut self, collections: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<String>,
    {
        self.collections = collections.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_classification(mut self, classification: impl Into<String>) -> Self {
        self.classification = Some(classification.into());
        self
    }

    pub fn with_embedding_ref(mut self, embedding_ref: impl Into<String>) -> Self {
        self.embedding_ref = Some(embedding_ref.into());
        self
    }

    pub fn with_embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: Value) -> Self {
        self.extra.insert(key.into(), value);
        self
    }

    /// The best plain-text representation of the item for embedding/search:
    /// title plus inline body when present.
    pub fn text_for_embedding(&self) -> String {
        match &self.body {
            ItemBody::Inline { text } => format!("{}\n{}", self.title, text),
            _ => self.title.clone(),
        }
    }
}
