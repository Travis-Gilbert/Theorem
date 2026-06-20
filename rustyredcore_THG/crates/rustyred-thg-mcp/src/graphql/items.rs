//! Item query surface.
//!
//! Items are a projection over graph-native CommonPlace `Item` nodes plus the
//! Harness/Theseus records the Auto-Organizer needs to see beside them.

use async_graphql::{Object, Result as GqlResult, SimpleObject, ID};
use rustyred_thg_core::NodeRecord;

use super::projection::{project_node_to_item, ProjectedItem};
use super::scalars::Json;
use super::{map_err, with_invoker};

#[derive(Clone, SimpleObject)]
pub struct Item {
    pub id: ID,
    pub kind: String,
    pub title: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub source: Option<String>,
    pub labels: Vec<String>,
    pub extra: Json,
}

impl From<ProjectedItem> for Item {
    fn from(item: ProjectedItem) -> Self {
        Self {
            id: ID(item.id),
            kind: item.kind,
            title: item.title,
            created_at: item.created_at,
            updated_at: item.updated_at,
            source: item.source,
            labels: item.labels,
            extra: Json(item.extra),
        }
    }
}

#[derive(Default)]
pub struct ItemQuery;

#[Object]
impl ItemQuery {
    /// Hydrate projected Items from CommonPlace Item nodes and harness graph
    /// records. The resolver does not mutate the native records.
    async fn items(
        &self,
        kind: Option<String>,
        #[graphql(default = 100)] limit: i32,
    ) -> GqlResult<Vec<Item>> {
        with_invoker(|inv| {
            let nodes = inv.item_projection_nodes(normalize_limit(limit)).map_err(map_err)?;
            Ok(project_items(nodes, kind.as_deref(), limit))
        })
    }

    /// Hydrate Items filtered by the projected kind.
    async fn items_by_kind(
        &self,
        kind: String,
        #[graphql(default = 100)] limit: i32,
    ) -> GqlResult<Vec<Item>> {
        with_invoker(|inv| {
            let nodes = inv.item_projection_nodes(normalize_limit(limit)).map_err(map_err)?;
            Ok(project_items(nodes, Some(kind.as_str()), limit))
        })
    }

    /// Hydrate one Item by native graph id.
    async fn item(&self, id: ID) -> GqlResult<Option<Item>> {
        let id = id.to_string();
        with_invoker(|inv| {
            Ok(inv
                .get_doc(&id)
                .map_err(map_err)?
                .and_then(|value| serde_json::from_value::<NodeRecord>(value).ok())
                .and_then(|node| project_node_to_item(&node))
                .map(Item::from))
        })
    }
}

fn project_items(nodes: Vec<NodeRecord>, kind: Option<&str>, limit: i32) -> Vec<Item> {
    let wanted_kind = kind.map(str::trim).filter(|kind| !kind.is_empty());
    let mut items = nodes
        .into_iter()
        .filter_map(|node| project_node_to_item(&node))
        .filter(|item| {
            wanted_kind
                .map(|wanted| item.kind.eq_ignore_ascii_case(wanted))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    items.sort_by(|a, b| {
        let a_ts = a
            .updated_at
            .as_ref()
            .or(a.created_at.as_ref())
            .map(String::as_str)
            .unwrap_or("");
        let b_ts = b
            .updated_at
            .as_ref()
            .or(b.created_at.as_ref())
            .map(String::as_str)
            .unwrap_or("");
        b_ts.cmp(a_ts).then_with(|| a.id.cmp(&b.id))
    });
    items
        .into_iter()
        .take(normalize_limit(limit))
        .map(Item::from)
        .collect()
}

fn normalize_limit(limit: i32) -> usize {
    if limit <= 0 {
        100
    } else {
        limit as usize
    }
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::json;

    use super::super::{execute_graphql, introspect_sdl, OpKind};
    use crate::SharedStore;

    #[test]
    fn graphql_items_projects_harness_writes() {
        let store = SharedStore::new(InMemoryGraphStore::new());

        let create_task = execute_graphql(
            "tenant-items",
            store.clone(),
            &json!({
                "query": "mutation($run:String!){ createTaskNode(runId:$run, nodeId:\"task:a\", goal:\"Implement Item projection\", kind:\"implementation\", actor:\"codex\"){ ok task } }",
                "variables": { "run": "run-items" }
            }),
            OpKind::Mutate,
        )
        .expect("task mutation succeeds");
        assert_eq!(create_task["data"]["createTaskNode"]["ok"], true);

        let create_record = execute_graphql(
            "tenant-items",
            store.clone(),
            &json!({
                "query": "mutation { writeCoordinationRecord(roomId:\"room:item\", actor:\"codex\", recordType:\"decision\", summary:\"Project coordination records as Items\", title:\"Projection decision\") }"
            }),
            OpKind::Mutate,
        )
        .expect("coordination record mutation succeeds");
        assert!(
            create_record["errors"].as_array().map(Vec::is_empty).unwrap_or(true),
            "coordination record mutation failed: {create_record}"
        );

        let remember = execute_graphql(
            "tenant-items",
            store.clone(),
            &json!({
                "query": "mutation($input:MemoryInput!){ rememberMemory(input:$input){ id title } }",
                "variables": {
                    "input": {
                        "content": "Memory item content",
                        "kind": "note",
                        "title": "Memory item"
                    }
                }
            }),
            OpKind::Mutate,
        )
        .expect("memory mutation succeeds");
        assert!(remember["data"]["rememberMemory"]["id"].as_str().is_some());

        let items = execute_graphql(
            "tenant-items",
            store,
            &json!({
                "query": "{ items(limit:20){ id kind title source extra } itemsByKind(kind:\"task\", limit:5){ kind title } }"
            }),
            OpKind::Query,
        )
        .expect("items query succeeds");
        assert!(
            items["errors"].as_array().map(Vec::is_empty).unwrap_or(true),
            "items query failed: {items}"
        );
        let all = items["data"]["items"].as_array().expect("items array");
        assert!(all.iter().any(|item| {
            item["kind"] == "task" && item["title"] == "Implement Item projection"
        }));
        assert!(all
            .iter()
            .any(|item| item["kind"] == "coordination" && item["title"] == "Projection decision"));
        assert!(all
            .iter()
            .any(|item| item["kind"] == "memory" && item["title"] == "Memory item"));
        assert_eq!(items["data"]["itemsByKind"][0]["kind"], "task");
    }

    #[test]
    fn graphql_sdl_exposes_item_fields() {
        let sdl = introspect_sdl();
        let sdl = sdl.as_str().expect("sdl string");
        assert!(sdl.contains("type Item"));
        assert!(sdl.contains("items("));
        assert!(sdl.contains("itemsByKind"));
    }
}
