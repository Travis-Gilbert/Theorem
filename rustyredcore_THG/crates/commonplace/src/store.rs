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
// Entity label + mention edge are owned by `ingest` (`ENTITY_LABEL` /
// `MENTIONS_ENTITY_EDGE`, value "MENTIONS_ENTITY"); the old store-local
// `MENTIONS`/`Entity` duplicates were dead and are removed so entity traversals
// are not split across two edge types (spec cleanup note).

/// Edge: a subtask points at its parent task (Task -> Task). Parent rollup is a
/// reverse traversal (Layer C).
pub const SUBTASK_OF_EDGE: &str = "SUBTASK_OF";
/// Edge: a task depends on another (Task -> Task), so "what blocks this" is a
/// forward traversal (Layer C).
pub const DEPENDS_ON_EDGE: &str = "DEPENDS_ON";
/// Edge: a task is about an item or entity it concerns (Task -> Item|Entity), so
/// "what is this task about" walks for free (Layer C, the load-bearing one).
pub const ABOUT_EDGE: &str = "ABOUT";
/// Edge: a delegated task points at the agent-run node working it (Task -> Run).
pub const WORKED_BY_EDGE: &str = "WORKED_BY";
/// Property holding a collection's label embedding (centroid / name embedding).
pub const LABEL_EMBEDDING_PROPERTY: &str = "label_embedding";
/// Property holding an item's or entity's embedding vector.
pub const EMBEDDING_PROPERTY: &str = "embedding";
/// Derived property holding `"{source}:{external_id}"` for exact-match lookup (A3).
pub const SOURCE_REF_KEY_PROPERTY: &str = "source_ref_key";

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

    /// The item that came from this exact source record, if one exists (A3). A
    /// single exact-match property filter on the derived `source_ref_key`, so a
    /// re-fetched record updates in place instead of minting a duplicate.
    pub fn item_by_source_ref(
        &self,
        source: &str,
        external_id: &str,
    ) -> GraphStoreResult<Option<Item>> {
        // Reuse SourceRef::key so the lookup key matches what put_item wrote.
        let key = crate::item::SourceRef::new(source, external_id).key();
        let nodes = self.store.query_nodes(
            NodeQuery::label(ITEM_LABEL)
                .with_property(SOURCE_REF_KEY_PROPERTY, json!(key))
                .with_limit(1),
        );
        match nodes.first() {
            Some(node) => self.hydrate_item(node).map(Some),
            None => Ok(None),
        }
    }

    // ---- Collections -------------------------------------------------------

    /// Find a collection by exact name, or `Ok(None)`. Used by source routing to
    /// resolve a rule's target collection (create-or-get).
    pub fn collection_by_name(&self, name: &str) -> GraphStoreResult<Option<Collection>> {
        for node in self
            .store
            .query_nodes(NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX))
        {
            if prop_str(&node.properties, "name").as_deref() == Some(name) {
                return self.get_collection(&node.id);
            }
        }
        Ok(None)
    }

    /// Get a collection by name, creating it (as `kind`) if absent. Idempotent on
    /// name.
    pub fn get_or_create_collection(
        &mut self,
        name: &str,
        kind: CollectionKind,
    ) -> GraphStoreResult<Collection> {
        if let Some(existing) = self.collection_by_name(name)? {
            return Ok(existing);
        }
        self.create_collection(name, kind)
    }

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

    // ---- Tasks (Layer C) ---------------------------------------------------

    /// Mark `child` a subtask of `parent` (edge child -SUBTASK_OF-> parent).
    pub fn add_subtask(&mut self, parent_id: &str, child_id: &str) -> GraphStoreResult<()> {
        self.upsert_link(SUBTASK_OF_EDGE, child_id, parent_id)
    }

    /// Record that `task` depends on `depends_on` (edge task -DEPENDS_ON-> dep).
    pub fn add_dependency(&mut self, task_id: &str, depends_on_id: &str) -> GraphStoreResult<()> {
        self.upsert_link(DEPENDS_ON_EDGE, task_id, depends_on_id)
    }

    /// Link a task to an item or entity it concerns (edge task -ABOUT-> target).
    pub fn link_about(&mut self, task_id: &str, target_id: &str) -> GraphStoreResult<()> {
        self.upsert_link(ABOUT_EDGE, task_id, target_id)
    }

    /// Bind a delegated task to the agent-run node working it
    /// (edge task -WORKED_BY-> run).
    pub fn link_worked_by(&mut self, task_id: &str, run_id: &str) -> GraphStoreResult<()> {
        self.upsert_link(WORKED_BY_EDGE, task_id, run_id)
    }

    /// The subtasks of a parent task (reverse `SUBTASK_OF` traversal).
    pub fn subtasks(&self, parent_id: &str) -> GraphStoreResult<Vec<Item>> {
        self.items_for(NeighborQuery::in_(parent_id).with_edge_type(SUBTASK_OF_EDGE))
    }

    /// The tasks/items this task depends on (forward `DEPENDS_ON` traversal).
    pub fn task_dependencies(&self, task_id: &str) -> GraphStoreResult<Vec<Item>> {
        self.items_for(NeighborQuery::out(task_id).with_edge_type(DEPENDS_ON_EDGE))
    }

    /// The node ids a task is about (forward `ABOUT` traversal). Returns ids
    /// rather than items because an `ABOUT` target may be an `Entity`, not an
    /// `Item`.
    pub fn task_about(&self, task_id: &str) -> GraphStoreResult<Vec<String>> {
        Ok(self
            .store
            .neighbors(NeighborQuery::out(task_id).with_edge_type(ABOUT_EDGE))
            .into_iter()
            .map(|hit| hit.node_id)
            .collect())
    }

    /// Open tasks: `Task` items whose `status` is not a terminal one
    /// (done/closed/cancelled/complete). A missing status counts as open.
    pub fn open_tasks(&self) -> GraphStoreResult<Vec<Item>> {
        Ok(self
            .items_by_kind(&ItemKind::Task)?
            .into_iter()
            .filter(|item| !is_terminal_status(item.status.as_deref()))
            .collect())
    }

    /// Tasks whose `due_at_ms` falls in `[from_ms, to_ms]` (inclusive). The
    /// "due today" range query.
    pub fn tasks_due_between(&self, from_ms: i64, to_ms: i64) -> GraphStoreResult<Vec<Item>> {
        Ok(self
            .items_by_kind(&ItemKind::Task)?
            .into_iter()
            .filter(|item| matches!(item.due_at_ms, Some(due) if due >= from_ms && due <= to_ms))
            .collect())
    }

    /// Subtask progress rollup for a parent task: `(done, total)` over its
    /// `SUBTASK_OF` children.
    pub fn subtask_progress(&self, parent_id: &str) -> GraphStoreResult<(usize, usize)> {
        let children = self.subtasks(parent_id)?;
        let total = children.len();
        let done = children
            .iter()
            .filter(|child| is_terminal_status(child.status.as_deref()))
            .count();
        Ok((done, total))
    }

    // ---- internals ---------------------------------------------------------

    fn upsert_link(&mut self, edge_type: &str, from: &str, to: &str) -> GraphStoreResult<()> {
        let edge = EdgeRecord::new(
            format!("{}:{from}:{to}", edge_type.to_ascii_lowercase()),
            from,
            edge_type,
            to,
            json!({}),
        );
        self.store.upsert_edge(edge)?;
        Ok(())
    }

    fn items_for(&self, query: NeighborQuery) -> GraphStoreResult<Vec<Item>> {
        let ids: Vec<String> = self
            .store
            .neighbors(query)
            .into_iter()
            .map(|hit| hit.node_id)
            .collect();
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            if let Some(item) = self.get_item(&id)? {
                items.push(item);
            }
        }
        Ok(items)
    }

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
        // Derived single-string key for an O(1) exact-match source-ref lookup (A3).
        if let Some(key) = item.source_ref_key() {
            object.insert(SOURCE_REF_KEY_PROPERTY.to_string(), json!(key));
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

/// Whether a task `status` is terminal (the task is finished). A missing status
/// is treated as open. Case-insensitive over the common done/closed vocabulary.
fn is_terminal_status(status: Option<&str>) -> bool {
    match status {
        Some(value) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "done" | "closed" | "complete" | "completed" | "cancelled" | "canceled"
        ),
        None => false,
    }
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
