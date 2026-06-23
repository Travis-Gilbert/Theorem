//! SPEC-2 deliverable 1: the one projection from a graph node (or a mutation
//! event) to the Item shape.
//!
//! The invariant the projection enforces: a harness / Theseus / dispatch node
//! read as an Item, and the same node delivered as an Item delta on the live
//! changefeed, have IDENTICAL shape, because both go through [`project_node`]
//! here. The read resolver ([`super::items`]) converts a [`ProjectedItem`] to the
//! GraphQL `Item`; the async changefeed handler on `rustyred-thg-server`
//! serializes an [`ItemDelta`] to SSE JSON. Neither reimplements the shape, so a
//! shape change in one is a shape change in both (SPEC-2 acceptance 4).
//!
//! The projection NEVER mutates the node it reads. It presents native harness /
//! Theseus / commonplace nodes as Items on read and as Item deltas on the feed;
//! the single source of truth stays the native node.

use serde::Serialize;
use serde_json::{Map, Value};

use rustyred_thg_core::{MutationEvent, MutationKind, NodeRecord};

/// Node labels the projection recognizes. These strings are the projection's
/// match keys, bound to the writers by reading them in the codebase:
///
/// - `TaskNode`           <- `multihead_task_payload` (work-graph task node);
///   `theorem-harness-runtime/src/work_graph_store.rs` `TASK_NODE_LABEL`.
/// - `CoordinationRecord` <- `write_record_payload` (coordination record node);
///   `rustyred-thg-mcp/src/lib.rs` `coordination_record_node`, the specific label
///   of the `["HarnessCoordination", "CoordinationRecord"]` pair.
/// - `MemoryDocument`     <- `remember` / `encode` (memory doc node);
///   `theorem-harness-runtime/src/memory.rs`, the specific label of the
///   `["HarnessMemory", "MemoryAtom", "MemoryDocument"]` triple.
/// - `Job`                <- `job_submit` (dispatch job node; deliverable 5,
///   reconciled: `job_submit` already writes a `Job` graph node, so jobs ride the
///   graph changefeed for free); `theorem-harness-runtime/src/job_queue.rs`
///   `JOB_LABEL`.
/// - `Item`               <- the commonplace store (real user Items);
///   `commonplace/src/store.rs` `ITEM_LABEL`.
pub const TASK_NODE_LABEL: &str = "TaskNode";
pub const COORDINATION_RECORD_LABEL: &str = "CoordinationRecord";
pub const MEMORY_DOCUMENT_LABEL: &str = "MemoryDocument";
pub const JOB_LABEL: &str = "Job";
pub const ITEM_LABEL: &str = "Item";

/// Every label the Item domain projects, for the changefeed hook matcher and the
/// `items` enumeration. `Item` is first so real user Items lead the union.
pub const PROJECTED_LABELS: [&str; 5] = [
    ITEM_LABEL,
    TASK_NODE_LABEL,
    COORDINATION_RECORD_LABEL,
    MEMORY_DOCUMENT_LABEL,
    JOB_LABEL,
];

/// The projected Item: the shared shape, deliberately free of any async-graphql
/// dependency so the async server's changefeed handler can build it without
/// pulling in the GraphQL types. [`super::items::Item`] is the thin GraphQL
/// wrapper over this.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProjectedItem {
    pub id: String,
    /// The item kind token. For a real `Item` node, its own stored `kind`
    /// (`note` / `file` / ...); for a projected harness or dispatch node, the
    /// fixed origin kind (`task` / `coordination` / `memory` / `job`).
    pub kind: String,
    pub title: String,
    /// The origin tag (e.g. `harness:task`, `dispatch:job`, `commonplace`).
    pub source: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// The node's native properties, preserved verbatim under the Item's `extra`.
    pub extra: Value,
}

/// The change a changefeed delta carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemChange {
    Upserted,
    Deleted,
}

/// One changefeed delta: an Item upsert (with the projected item) or a delete
/// (id only, since the node is gone). Serializes to the SSE JSON payload the
/// CommonPlace Auto-Organizer applies.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ItemDelta {
    pub change: ItemChange,
    pub id: String,
    /// The tenant the mutation landed in, carried from the event so the
    /// process-global changefeed bus can be filtered per tenant by the SSE
    /// endpoint (the bus is shared across tenants; the delta is self-describing).
    pub tenant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<ProjectedItem>,
}

/// Map a recognized harness/dispatch label set to `(kind, source)`. Returns
/// `None` for an `Item` node (its kind comes from its own `kind` property) and
/// for nodes the Item domain does not project.
fn harness_origin(labels: &[String]) -> Option<(&'static str, &'static str)> {
    // A node may carry several labels; match the specific projected one.
    for label in labels {
        match label.as_str() {
            TASK_NODE_LABEL => return Some(("task", "harness:task")),
            COORDINATION_RECORD_LABEL => return Some(("coordination", "harness:coordination")),
            MEMORY_DOCUMENT_LABEL => return Some(("memory", "harness:memory")),
            JOB_LABEL => return Some(("job", "dispatch:job")),
            _ => {}
        }
    }
    None
}

fn is_item_node(labels: &[String]) -> bool {
    labels.iter().any(|l| l == ITEM_LABEL)
}

/// True if a node bearing these labels projects to an Item. Used by the
/// changefeed to drop events for non-Item nodes and by tests.
pub fn is_projectable(labels: &[String]) -> bool {
    is_item_node(labels) || harness_origin(labels).is_some()
}

fn prop_str(props: &Map<String, Value>, key: &str) -> Option<String> {
    props.get(key).and_then(Value::as_str).map(str::to_string)
}

/// First non-empty string among `keys`.
fn first_str(props: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|k| prop_str(props, k).filter(|s| !s.trim().is_empty()))
}

/// First epoch-ms timestamp among `keys`. Accepts a numeric value (ms) or a
/// numeric string; a non-numeric string (e.g. an ISO date) is skipped rather
/// than mis-parsed, because every writer the projection binds stores ms.
fn first_ms(props: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|k| match props.get(*k) {
        Some(Value::Number(n)) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Some(Value::String(s)) => s.trim().parse::<i64>().ok(),
        _ => None,
    })
}

/// Title is `title`, falling back to a TaskNode's `goal`, then other common
/// name carriers; finally the node id so an Item always has a non-empty title.
const TITLE_KEYS: &[&str] = &["title", "goal", "name", "summary", "label"];
const CREATED_KEYS: &[&str] = &["created_at_ms", "created_at", "createdAt"];
const UPDATED_KEYS: &[&str] = &[
    "updated_at_ms",
    "updated_at",
    "updatedAt",
    "committed_at_ms",
];

/// Project a graph node to a [`ProjectedItem`], or `None` if the node is not one
/// the Item domain recognizes. The node is read, never mutated.
pub fn project_node(node: &NodeRecord) -> Option<ProjectedItem> {
    let empty = Map::new();
    let props = node.properties.as_object().unwrap_or(&empty);

    let (kind, source) = if let Some((kind, source)) = harness_origin(&node.labels) {
        (kind.to_string(), source.to_string())
    } else if is_item_node(&node.labels) {
        // A real commonplace Item: its own stored kind, and its own source if it
        // carries one, else the `commonplace` origin.
        let kind = prop_str(props, "kind").unwrap_or_else(|| "note".to_string());
        let source = prop_str(props, "source").unwrap_or_else(|| "commonplace".to_string());
        (kind, source)
    } else {
        return None;
    };

    let title = first_str(props, TITLE_KEYS).unwrap_or_else(|| node.id.clone());
    let created_at_ms = first_ms(props, CREATED_KEYS).unwrap_or(0);
    let updated_at_ms = first_ms(props, UPDATED_KEYS).unwrap_or(created_at_ms);

    Some(ProjectedItem {
        id: node.id.clone(),
        kind,
        title,
        source,
        created_at_ms,
        updated_at_ms,
        extra: node.properties.clone(),
    })
}

/// Project a mutation event to an Item delta for the changefeed. This is the
/// spec's "second form" that maps a [`MutationEvent`] through the same
/// projection: on an upsert the caller supplies the freshly re-read node (the
/// changefeed hook re-reads it through its `&mut RedCoreGraphStore`) and the
/// delta carries [`project_node`]'s output; on a delete the node is gone, so the
/// delta is a tombstone (id only).
///
/// Returns `None` when the event is for a node the Item domain does not project
/// or for an edge mutation, so the changefeed carries only Item-relevant deltas.
pub fn project_event(event: &MutationEvent, node: Option<&NodeRecord>) -> Option<ItemDelta> {
    if !is_projectable(&event.labels) {
        return None;
    }
    match event.kind {
        MutationKind::NodeDeleted => Some(ItemDelta {
            change: ItemChange::Deleted,
            id: event.id.clone(),
            tenant: event.tenant.clone(),
            item: None,
        }),
        MutationKind::NodeUpserted => Some(ItemDelta {
            change: ItemChange::Upserted,
            id: event.id.clone(),
            tenant: event.tenant.clone(),
            // If the node could not be re-read (raced deletion), the delta still
            // carries the id and change so the consumer can reconcile.
            item: node.and_then(project_node),
        }),
        // Edge mutations are not Item changes (the Item domain is node-shaped);
        // unreachable in practice because edge events carry `[edge_type]` labels,
        // which `is_projectable` already rejects above.
        MutationKind::EdgeUpserted | MutationKind::EdgeDeleted => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, labels: &[&str], props: Value) -> NodeRecord {
        NodeRecord::new(id, labels.iter().map(|s| s.to_string()), props)
    }

    #[test]
    fn projects_work_graph_task_node() {
        let n = node(
            "run-1::task-7",
            &[TASK_NODE_LABEL],
            json!({ "goal": "Ship SPEC-2", "created_at_ms": 1000, "updated_at_ms": 2000 }),
        );
        let p = project_node(&n).expect("task node projects");
        assert_eq!(p.kind, "task");
        assert_eq!(p.source, "harness:task");
        // Title falls back to `goal` for a task node.
        assert_eq!(p.title, "Ship SPEC-2");
        assert_eq!(p.created_at_ms, 1000);
        assert_eq!(p.updated_at_ms, 2000);
        // Native fields preserved under extra.
        assert_eq!(p.extra["goal"], json!("Ship SPEC-2"));
    }

    #[test]
    fn projects_coordination_record_node() {
        let n = node(
            "rec-1",
            &["HarnessCoordination", COORDINATION_RECORD_LABEL],
            json!({ "title": "Lane split", "created_at": 5 }),
        );
        let p = project_node(&n).expect("coordination record projects");
        assert_eq!(p.kind, "coordination");
        assert_eq!(p.source, "harness:coordination");
        assert_eq!(p.title, "Lane split");
        assert_eq!(p.created_at_ms, 5);
    }

    #[test]
    fn projects_memory_document_node() {
        let n = node(
            "doc-1",
            &["HarnessMemory", "MemoryAtom", MEMORY_DOCUMENT_LABEL],
            json!({ "title": "A lesson", "created_at": 10, "updated_at": 20 }),
        );
        let p = project_node(&n).expect("memory doc projects");
        assert_eq!(p.kind, "memory");
        assert_eq!(p.source, "harness:memory");
        assert_eq!(p.updated_at_ms, 20);
    }

    #[test]
    fn projects_dispatch_job_node() {
        let n = node(
            "job-1",
            &[JOB_LABEL],
            json!({ "title": "Build it", "created_at_ms": 7 }),
        );
        let p = project_node(&n).expect("job node projects");
        assert_eq!(p.kind, "job");
        assert_eq!(p.source, "dispatch:job");
    }

    #[test]
    fn projects_real_commonplace_item_with_its_own_kind() {
        // A real Item written by the commonplace store: it carries its own kind
        // and is indistinguishable in shape from a projected harness Item
        // (SPEC-2 acceptance 3).
        let n = node(
            "item-1",
            &[ITEM_LABEL],
            json!({ "kind": "note", "title": "My note", "created_at_ms": 100, "updated_at_ms": 100 }),
        );
        let p = project_node(&n).expect("item projects");
        assert_eq!(p.kind, "note");
        assert_eq!(p.source, "commonplace");
        assert_eq!(p.title, "My note");
    }

    #[test]
    fn unrecognized_node_does_not_project() {
        let n = node("x-1", &["CodeSymbol"], json!({ "signature": "fn f()" }));
        assert!(project_node(&n).is_none());
        assert!(!is_projectable(&n.labels));
    }

    #[test]
    fn title_falls_back_to_id_when_absent() {
        let n = node(
            "naked-task",
            &[TASK_NODE_LABEL],
            json!({ "created_at_ms": 1 }),
        );
        let p = project_node(&n).unwrap();
        assert_eq!(p.title, "naked-task");
    }

    #[test]
    fn event_upsert_carries_same_shape_as_node_read() {
        // Acceptance 4: the changefeed delta's item is exactly project_node's
        // output, so the live shape and the queried shape cannot diverge.
        let n = node(
            "run-1::task-7",
            &[TASK_NODE_LABEL],
            json!({ "goal": "g", "created_at_ms": 1, "updated_at_ms": 2 }),
        );
        let event = MutationEvent::new(
            MutationKind::NodeUpserted,
            "tenant-a",
            "run-1::task-7",
            vec![TASK_NODE_LABEL.to_string()],
            vec!["goal".to_string()],
            2,
            0,
        );
        let delta = project_event(&event, Some(&n)).expect("upsert projects");
        assert_eq!(delta.change, ItemChange::Upserted);
        assert_eq!(delta.id, "run-1::task-7");
        assert_eq!(delta.tenant, "tenant-a");
        assert_eq!(delta.item.as_ref(), project_node(&n).as_ref());
    }

    #[test]
    fn event_delete_is_a_tombstone() {
        let event = MutationEvent::new(
            MutationKind::NodeDeleted,
            "tenant-a",
            "run-1::task-7",
            vec![TASK_NODE_LABEL.to_string()],
            vec![],
            3,
            0,
        );
        let delta = project_event(&event, None).expect("delete projects");
        assert_eq!(delta.change, ItemChange::Deleted);
        assert_eq!(delta.id, "run-1::task-7");
        assert!(delta.item.is_none());
    }

    #[test]
    fn event_for_unprojectable_node_is_dropped() {
        let event = MutationEvent::new(
            MutationKind::NodeUpserted,
            "tenant-a",
            "sym-1",
            vec!["CodeSymbol".to_string()],
            vec![],
            1,
            0,
        );
        assert!(project_event(&event, None).is_none());
    }

    #[test]
    fn edge_event_is_dropped() {
        let event = MutationEvent::new(
            MutationKind::EdgeUpserted,
            "tenant-a",
            "e-1",
            vec!["IN_COLLECTION".to_string()],
            vec![],
            1,
            0,
        );
        assert!(project_event(&event, None).is_none());
    }
}
