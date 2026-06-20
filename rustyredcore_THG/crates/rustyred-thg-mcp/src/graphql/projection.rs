//! Item projection for graph-native Harness and CommonPlace records.
//!
//! This module is intentionally independent from async-graphql. The GraphQL
//! resolver and the async server changefeed both call the same projection
//! functions so Item hydration and Item deltas do not drift.

use rustyred_thg_core::{MutationEvent, MutationKind, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const ITEM_LABEL: &str = "Item";
pub const TASK_NODE_LABEL: &str = "TaskNode";
pub const COORDINATION_RECORD_LABEL: &str = "CoordinationRecord";
pub const MEMORY_DOCUMENT_LABEL: &str = "MemoryDocument";
pub const MEMORY_NODE_LABEL: &str = "MemoryNode";
pub const JOB_LABEL: &str = "Job";

pub const ITEM_SOURCE_LABELS: &[&str] = &[
    ITEM_LABEL,
    TASK_NODE_LABEL,
    COORDINATION_RECORD_LABEL,
    MEMORY_DOCUMENT_LABEL,
    MEMORY_NODE_LABEL,
    JOB_LABEL,
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectedItem {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub labels: Vec<String>,
    pub extra: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ProjectedItemDelta {
    pub tenant: String,
    pub event: String,
    pub committed_at_ms: u64,
    pub changed_props: Vec<String>,
    pub item: ProjectedItem,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ProjectionClass {
    Item,
    Task,
    Coordination,
    Memory,
    Job,
}

pub fn project_node_to_item(node: &NodeRecord) -> Option<ProjectedItem> {
    let class = classify_labels(&node.labels)?;
    let properties = node.properties.clone();
    Some(ProjectedItem {
        id: node.id.clone(),
        kind: item_kind(class, &properties),
        title: item_title(class, &properties),
        created_at: timestamp(&properties, &["created_at", "createdAt", "submitted_at"]),
        updated_at: timestamp(
            &properties,
            &[
                "updated_at",
                "updatedAt",
                "archived_at",
                "started_at",
                "submitted_at",
                "created_at",
                "createdAt",
            ],
        ),
        source: string_field(
            &properties,
            &[
                "source",
                "origin",
                "origin_surface",
                "submitted_by",
                "actor_id",
                "created_by",
            ],
        ),
        labels: node.labels.clone(),
        extra: properties,
    })
}

pub fn project_mutation_event(
    event: &MutationEvent,
    node: Option<&NodeRecord>,
) -> Option<ProjectedItemDelta> {
    let item = match node {
        Some(node) => project_node_to_item(node)?,
        None => project_event_to_item(event)?,
    };
    Some(ProjectedItemDelta {
        tenant: event.tenant.clone(),
        event: event.kind.as_str().to_string(),
        committed_at_ms: event.committed_at_ms,
        changed_props: event.changed_props.clone(),
        item,
    })
}

pub fn is_projected_item_label(labels: &[String]) -> bool {
    classify_labels(labels).is_some()
}

fn project_event_to_item(event: &MutationEvent) -> Option<ProjectedItem> {
    let class = classify_labels(&event.labels)?;
    Some(ProjectedItem {
        id: event.id.clone(),
        kind: item_kind(class, &Value::Null),
        title: None,
        created_at: None,
        updated_at: None,
        source: None,
        labels: event.labels.clone(),
        extra: json!({
            "projection": "mutation_event",
            "mutation_kind": event.kind.as_str(),
            "changed_props": event.changed_props,
            "deleted": matches!(event.kind, MutationKind::NodeDeleted),
        }),
    })
}

fn classify_labels(labels: &[String]) -> Option<ProjectionClass> {
    if has_label(labels, ITEM_LABEL) {
        return Some(ProjectionClass::Item);
    }
    if has_label(labels, TASK_NODE_LABEL) {
        return Some(ProjectionClass::Task);
    }
    if has_label(labels, COORDINATION_RECORD_LABEL) {
        return Some(ProjectionClass::Coordination);
    }
    if has_label(labels, MEMORY_DOCUMENT_LABEL) || has_label(labels, MEMORY_NODE_LABEL) {
        return Some(ProjectionClass::Memory);
    }
    if has_label(labels, JOB_LABEL) {
        return Some(ProjectionClass::Job);
    }
    None
}

fn has_label(labels: &[String], wanted: &str) -> bool {
    labels.iter().any(|label| label == wanted)
}

fn item_kind(class: ProjectionClass, properties: &Value) -> String {
    match class {
        ProjectionClass::Item => string_field(properties, &["kind", "item_kind", "itemKind", "type"])
            .unwrap_or_else(|| "item".to_string()),
        ProjectionClass::Task => "task".to_string(),
        ProjectionClass::Coordination => "coordination".to_string(),
        ProjectionClass::Memory => "memory".to_string(),
        ProjectionClass::Job => "job".to_string(),
    }
}

fn item_title(class: ProjectionClass, properties: &Value) -> Option<String> {
    match class {
        ProjectionClass::Task => string_field(properties, &["title", "goal"]),
        ProjectionClass::Coordination => string_field(properties, &["title", "summary", "body"]),
        ProjectionClass::Memory => string_field(properties, &["title", "summary", "content"]),
        ProjectionClass::Job => string_field(properties, &["title"]),
        ProjectionClass::Item => string_field(properties, &["title", "name", "goal", "summary"]),
    }
}

fn timestamp(properties: &Value, keys: &[&str]) -> Option<String> {
    string_field(properties, keys)
}

fn string_field(value: &Value, keys: &[&str]) -> Option<String> {
    let values = nested_values(value);
    for key in keys {
        for candidate in &values {
            if let Some(raw) = candidate.get(*key).and_then(Value::as_str) {
                let trimmed = raw.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn nested_values(value: &Value) -> Vec<&Value> {
    let mut out = vec![value];
    for key in ["properties", "document", "node", "item"] {
        if let Some(inner) = value.get(key) {
            out.push(inner);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_harness_labels_without_mutating_native_props() {
        let node = NodeRecord::new(
            "work-graph:run-1:node:a",
            ["TaskNode"],
            json!({
                "id": "a",
                "run_id": "run-1",
                "goal": "Implement item projection",
                "created_by": "codex"
            }),
        );

        let item = project_node_to_item(&node).expect("task node projects");
        assert_eq!(item.kind, "task");
        assert_eq!(item.title.as_deref(), Some("Implement item projection"));
        assert_eq!(item.source.as_deref(), Some("codex"));
        assert_eq!(item.extra["goal"], "Implement item projection");
    }
}
