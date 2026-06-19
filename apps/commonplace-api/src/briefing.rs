//! Proactive surfacing: the briefing / graph-weather (plan unit I2).
//!
//! Reads the consumer store along three axes, unprompted: what is new (recency),
//! what connects (`SIMILAR_TO` degree), and what is unresolved (open-tagged or
//! unfiled items). The same graph I1 answers questions against, surfaced without
//! being asked.
//!
//! Scope note (surfaced): "newly connected" ranks by connection count (degree),
//! since the object model carries no per-edge timestamp yet; true recency of
//! linking needs an edge `created_at` (named follow-up). "Unfiled" leans on F2's
//! design (ingest always files into a collection), so an item in no collection
//! is a raw capture not yet organized.

use std::collections::HashSet;

use commonplace::{BlobStore, Commonplace, Item, SIMILAR_TO_EDGE};
use rustyred_thg_core::{GraphStore, GraphStoreResult, NeighborQuery};

/// An item plus how it connects to the rest of the store.
#[derive(Clone, Debug)]
pub struct ConnectedItem {
    pub item: Item,
    pub connections: usize,
    pub related: Vec<Item>,
}

/// The proactive briefing over a store.
#[derive(Clone, Debug)]
pub struct Briefing {
    pub recent: Vec<Item>,
    pub newly_connected: Vec<ConnectedItem>,
    pub open_threads: Vec<Item>,
}

/// Limits for each briefing axis.
#[derive(Clone, Debug)]
pub struct BriefingConfig {
    pub recent_limit: usize,
    pub connected_limit: usize,
    pub open_limit: usize,
    /// How many related neighbors to attach per connected item.
    pub related_limit: usize,
}

impl Default for BriefingConfig {
    fn default() -> Self {
        Self {
            recent_limit: 10,
            connected_limit: 10,
            open_limit: 10,
            related_limit: 3,
        }
    }
}

/// Compute the briefing over the consumer store.
pub fn briefing<S, B>(cp: &Commonplace<S, B>, config: &BriefingConfig) -> GraphStoreResult<Briefing>
where
    S: GraphStore,
    B: BlobStore,
{
    let items = cp.all_items()?;

    // What is new: most recently updated items.
    let mut recent = items.clone();
    recent.sort_by(|a, b| {
        b.updated_at_ms
            .cmp(&a.updated_at_ms)
            .then(b.created_at_ms.cmp(&a.created_at_ms))
            .then(a.id.cmp(&b.id))
    });
    recent.truncate(config.recent_limit);

    // What connects: items by SIMILAR_TO degree, recency as tie-break.
    let mut connected: Vec<ConnectedItem> = Vec::new();
    for item in &items {
        let neighbor_ids = similar_neighbor_ids(cp, &item.id);
        if neighbor_ids.is_empty() {
            continue;
        }
        let mut related = Vec::new();
        for id in neighbor_ids.iter().take(config.related_limit) {
            if let Some(neighbor) = cp.get_item(id)? {
                related.push(neighbor);
            }
        }
        connected.push(ConnectedItem {
            item: item.clone(),
            connections: neighbor_ids.len(),
            related,
        });
    }
    connected.sort_by(|a, b| {
        b.connections
            .cmp(&a.connections)
            .then(b.item.updated_at_ms.cmp(&a.item.updated_at_ms))
            .then(a.item.id.cmp(&b.item.id))
    });
    connected.truncate(config.connected_limit);

    // What is unresolved: open-tagged or unfiled (in no collection) items.
    let mut open_threads: Vec<Item> = items.into_iter().filter(is_open_thread).collect();
    open_threads.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.id.cmp(&b.id)));
    open_threads.truncate(config.open_limit);

    Ok(Briefing {
        recent,
        newly_connected: connected,
        open_threads,
    })
}

fn similar_neighbor_ids<S, B>(cp: &Commonplace<S, B>, item_id: &str) -> Vec<String>
where
    S: GraphStore,
    B: BlobStore,
{
    let mut seen = HashSet::new();
    let mut ids = Vec::new();
    for direction in [
        NeighborQuery::out(item_id).with_edge_type(SIMILAR_TO_EDGE),
        NeighborQuery::in_(item_id).with_edge_type(SIMILAR_TO_EDGE),
    ] {
        for hit in cp.store().neighbors(direction) {
            if hit.node_id != item_id && seen.insert(hit.node_id.clone()) {
                ids.push(hit.node_id);
            }
        }
    }
    ids
}

fn is_open_thread(item: &Item) -> bool {
    let open_tag = item.tags.iter().any(|tag| {
        matches!(
            tag.trim().to_lowercase().as_str(),
            "open" | "todo" | "to-do" | "unresolved" | "followup" | "follow-up"
        )
    });
    open_tag || item.collections.is_empty()
}
