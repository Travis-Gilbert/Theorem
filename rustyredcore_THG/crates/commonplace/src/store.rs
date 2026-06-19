//! [`Commonplace`]: the store facade over a [`GraphStore`] plus a [`BlobStore`]
//! (plan unit F1).
//!
//! It maps the consumer object model onto graph records:
//! - an `Item` is an `Item`-labelled node;
//! - a `Collection` is a `Collection`-labelled node;
//! - membership is an `IN_COLLECTION` edge (Item -> Collection), so a collection
//!   enumerates its members by reverse traversal;
//! - each tag is a `Tag`-labelled node joined by a `HAS_TAG` edge (Item -> Tag),
//!   and also kept on the item as a label array for cheap reads;
//! - a `File` item's bytes live in the blob store, addressed by the item body's
//!   content hash.
//!
//! Residency changes (per-item field) and the `SIMILAR_TO` edge (written by F2)
//! ride the same model.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{
    now_ms, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NeighborQuery, NodeQuery,
    NodeRecord,
};
use serde_json::{json, Value};

use crate::blob::BlobStore;
use crate::collection::{Collection, CollectionKind};
use crate::item::{Item, ItemBody, ItemKind};
use crate::tag::{tag_id, Tag};

/// Node label for items.
pub const ITEM_LABEL: &str = "Item";
/// Node label for collections.
pub const COLLECTION_LABEL: &str = "Collection";
/// Node label for tags.
pub const TAG_LABEL: &str = "Tag";
/// Edge type for collection membership (Item -> Collection).
pub const IN_COLLECTION_EDGE: &str = "IN_COLLECTION";
/// Edge type for tag attachment (Item -> Tag).
pub const HAS_TAG_EDGE: &str = "HAS_TAG";
/// Edge type for item-to-item similarity (written by the F2 ingest pipeline).
pub const SIMILAR_TO_EDGE: &str = "SIMILAR_TO";
/// Node label for resolved entities (people, orgs, etc.; F2 entity resolution).
pub const ENTITY_LABEL: &str = "Entity";
/// Edge type linking an item to an entity it mentions (Item -> Entity).
pub const MENTIONS_EDGE: &str = "MENTIONS";
/// Property holding a collection's label embedding (centroid / name embedding).
pub const LABEL_EMBEDDING_PROPERTY: &str = "label_embedding";
/// Property holding an item's or entity's embedding vector.
pub const EMBEDDING_PROPERTY: &str = "embedding";

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The CommonPlace store: a graph store for structure plus a blob store for
/// file bodies. Generic over both so it runs in-memory, on disk, or remotely.
pub struct Commonplace<S, B> {
    store: S,
    blobs: B,
}

impl<S, B> Commonplace<S, B>
where
    S: GraphStore,
    B: BlobStore,
{
    /// Build a store over an existing graph store and blob store.
    pub fn new(store: S, blobs: B) -> Self {
        Self { store, blobs }
    }

    /// Borrow the underlying graph store (for graph algorithms / retrieval).
    pub fn store(&self) -> &S {
        &self.store
    }

    /// Mutably borrow the underlying graph store.
    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    /// Borrow the blob store.
    pub fn blobs(&self) -> &B {
        &self.blobs
    }

    /// Decompose into the two backing stores.
    pub fn into_parts(self) -> (S, B) {
        (self.store, self.blobs)
    }

    // ---- Items -------------------------------------------------------------

    /// Write an item (create or replace by id) and return the stored item with
    /// assigned id/timestamps and reconstructed collection membership.
    pub fn put_item(&mut self, mut item: Item) -> GraphStoreResult<Item> {
        let now = now_ms();
        if item.id.trim().is_empty() {
            item.id = new_id("item");
        }
        if item.created_at_ms == 0 {
            item.created_at_ms = now;
        }
        item.updated_at_ms = now;
        item.tags = dedup_nonempty(item.tags);
        let requested_collections = dedup_nonempty(std::mem::take(&mut item.collections));

        let record = NodeRecord::new(item.id.clone(), [ITEM_LABEL], item_props(&item)?);
        self.store.upsert_node(record)?;

        for tag in &item.tags {
            self.write_tag_projection(&item.id, tag)?;
        }
        for collection_id in &requested_collections {
            self.add_to_collection(&item.id, collection_id)?;
        }

        item.collections = self.read_member_collections(&item.id);
        Ok(item)
    }

    /// Write a `File` item: store the bytes in the blob store, then persist an
    /// item whose body is the resulting content-addressed blob reference.
    pub fn put_file(
        &mut self,
        title: impl Into<String>,
        bytes: &[u8],
        mime: Option<String>,
    ) -> GraphStoreResult<Item> {
        let content_hash = self.blobs.put(bytes)?;
        let item = Item::new(ItemKind::File, title).with_body(ItemBody::Blob {
            content_hash,
            byte_len: bytes.len() as u64,
            mime,
        });
        self.put_item(item)
    }

    /// Read an item by id, with its collection membership reconstructed from
    /// edges. Returns `Ok(None)` if the id is absent or not an item node.
    pub fn get_item(&self, id: &str) -> GraphStoreResult<Option<Item>> {
        let node = match self.store.get_node(id) {
            Some(node) if node.labels.iter().any(|label| label == ITEM_LABEL) => node.clone(),
            _ => return Ok(None),
        };
        self.hydrate_item(&node).map(Some)
    }

    /// Resolve a `File` item's bytes from the blob store. Returns `Ok(None)` for
    /// non-file items or a missing blob.
    pub fn read_blob(&self, item: &Item) -> GraphStoreResult<Option<Vec<u8>>> {
        match &item.body {
            ItemBody::Blob { content_hash, .. } => self.blobs.get(content_hash),
            _ => Ok(None),
        }
    }

    /// All items of a given kind.
    pub fn items_by_kind(&self, kind: &ItemKind) -> GraphStoreResult<Vec<Item>> {
        let nodes = self.store.query_nodes(
            NodeQuery::label(ITEM_LABEL)
                .with_property("kind", json!(kind.as_str()))
                .with_limit(usize::MAX),
        );
        nodes.iter().map(|node| self.hydrate_item(node)).collect()
    }

    /// Every item in the store.
    pub fn all_items(&self) -> GraphStoreResult<Vec<Item>> {
        let nodes = self
            .store
            .query_nodes(NodeQuery::label(ITEM_LABEL).with_limit(usize::MAX));
        nodes.iter().map(|node| self.hydrate_item(node)).collect()
    }

    // ---- Collections -------------------------------------------------------

    /// Create a new collection and return it with its assigned id.
    pub fn create_collection(
        &mut self,
        name: impl Into<String>,
        kind: CollectionKind,
    ) -> GraphStoreResult<Collection> {
        let collection = Collection {
            id: new_id("coll"),
            name: name.into(),
            kind,
            created_at_ms: now_ms(),
        };
        let props = json!({
            "name": collection.name,
            "kind": collection.kind.as_str(),
            "created_at_ms": collection.created_at_ms,
        });
        let record = NodeRecord::new(collection.id.clone(), [COLLECTION_LABEL], props);
        self.store.upsert_node(record)?;
        Ok(collection)
    }

    /// Read a collection by id, or `Ok(None)` if absent / not a collection.
    pub fn get_collection(&self, id: &str) -> GraphStoreResult<Option<Collection>> {
        let node = match self.store.get_node(id) {
            Some(node) if node.labels.iter().any(|label| label == COLLECTION_LABEL) => node,
            _ => return Ok(None),
        };
        Ok(Some(Collection {
            id: node.id.clone(),
            name: prop_str(&node.properties, "name").unwrap_or_default(),
            kind: prop_str(&node.properties, "kind")
                .map(CollectionKind::from)
                .unwrap_or_default(),
            created_at_ms: node
                .properties
                .get("created_at_ms")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        }))
    }

    /// Add an item to a collection (idempotent).
    pub fn add_to_collection(
        &mut self,
        item_id: &str,
        collection_id: &str,
    ) -> GraphStoreResult<()> {
        let edge = EdgeRecord::new(
            format!("incol:{item_id}:{collection_id}"),
            item_id,
            IN_COLLECTION_EDGE,
            collection_id,
            json!({}),
        );
        self.store.upsert_edge(edge)?;
        Ok(())
    }

    /// Link two items as semantically similar. The F2 ingest pipeline writes
    /// these edges after embedding a new item; callers may also use it when
    /// importing known related material.
    pub fn add_similarity(
        &mut self,
        from_item_id: &str,
        to_item_id: &str,
        score: f32,
    ) -> GraphStoreResult<()> {
        let edge = EdgeRecord::new(
            format!("similar:{from_item_id}:{to_item_id}"),
            from_item_id,
            SIMILAR_TO_EDGE,
            to_item_id,
            json!({
                "score": score,
                "method": "commonplace_embedding_v1",
            }),
        )
        .with_confidence(score as f64);
        self.store.upsert_edge(edge)?;
        Ok(())
    }

    /// All items that belong to a collection.
    pub fn collection_items(&self, collection_id: &str) -> GraphStoreResult<Vec<Item>> {
        let item_ids: Vec<String> = self
            .store
            .neighbors(NeighborQuery::in_(collection_id).with_edge_type(IN_COLLECTION_EDGE))
            .into_iter()
            .map(|hit| hit.node_id)
            .collect();
        let mut items = Vec::with_capacity(item_ids.len());
        for id in item_ids {
            if let Some(item) = self.get_item(&id)? {
                items.push(item);
            }
        }
        Ok(items)
    }

    // ---- Tags --------------------------------------------------------------

    /// Attach a tag (create-or-get by stable slug) to an item.
    pub fn tag_item(&mut self, item_id: &str, name: &str) -> GraphStoreResult<Tag> {
        self.write_tag_projection(item_id, name)
    }

    /// All tags attached to an item, as graph nodes.
    pub fn item_tags(&self, item_id: &str) -> GraphStoreResult<Vec<Tag>> {
        let tag_ids: Vec<String> = self
            .store
            .neighbors(NeighborQuery::out(item_id).with_edge_type(HAS_TAG_EDGE))
            .into_iter()
            .map(|hit| hit.node_id)
            .collect();
        let mut tags = Vec::with_capacity(tag_ids.len());
        for id in tag_ids {
            if let Some(node) = self.store.get_node(&id) {
                tags.push(Tag {
                    id: node.id.clone(),
                    name: prop_str(&node.properties, "name").unwrap_or_default(),
                });
            }
        }
        Ok(tags)
    }

    // ---- internals ---------------------------------------------------------

    fn write_tag_projection(&mut self, item_id: &str, name: &str) -> GraphStoreResult<Tag> {
        let id = tag_id(name);
        let clean = name.trim().to_string();
        let record = NodeRecord::new(id.clone(), [TAG_LABEL], json!({ "name": clean }));
        self.store.upsert_node(record)?;
        let edge = EdgeRecord::new(
            format!("hastag:{item_id}:{id}"),
            item_id,
            HAS_TAG_EDGE,
            &id,
            json!({}),
        );
        self.store.upsert_edge(edge)?;
        Ok(Tag { id, name: clean })
    }

    pub(crate) fn read_member_collections(&self, item_id: &str) -> Vec<String> {
        self.store
            .neighbors(NeighborQuery::out(item_id).with_edge_type(IN_COLLECTION_EDGE))
            .into_iter()
            .map(|hit| hit.node_id)
            .collect()
    }

    pub(crate) fn hydrate_item(&self, node: &NodeRecord) -> GraphStoreResult<Item> {
        let mut props = node.properties.clone();
        if let Some(object) = props.as_object_mut() {
            object.insert("id".to_string(), json!(node.id));
            // collections are edge-canonical; never trust a stored copy.
            object.remove("collections");
        }
        let mut item: Item = serde_json::from_value(props).map_err(serde_err)?;
        item.id = node.id.clone();
        item.collections = self.read_member_collections(&node.id);
        Ok(item)
    }
}

fn item_props(item: &Item) -> GraphStoreResult<Value> {
    let mut value = serde_json::to_value(item).map_err(serde_err)?;
    if let Some(object) = value.as_object_mut() {
        object.remove("id"); // node.id is the single source of truth
        object.remove("collections"); // edge-canonical (IN_COLLECTION)
        if let Some(embedding) = item.extra.get(crate::ingest::ITEM_EMBEDDING_PROPERTY) {
            object.insert(
                crate::ingest::ITEM_EMBEDDING_PROPERTY.to_string(),
                embedding.clone(),
            );
        }
    }
    Ok(value)
}

fn prop_str(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn dedup_nonempty(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .filter(|value| seen.insert(value.clone()))
        .collect()
}

pub(crate) fn new_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}:{nanos:x}-{counter:x}")
}

fn serde_err(error: serde_json::Error) -> GraphStoreError {
    GraphStoreError::new("commonplace_serde", error.to_string())
}
