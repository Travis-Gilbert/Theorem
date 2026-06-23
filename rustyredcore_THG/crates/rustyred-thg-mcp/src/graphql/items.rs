//! Item domain (SPEC-2 deliverable 2): the `ItemQuery` object on the one shared
//! `QueryRoot`.
//!
//! `items` / `itemsByKind` / `item(id)` return real commonplace Items unioned
//! with projected harness and dispatch nodes, every one shaped by the single
//! [`super::projection::project_node`]. Adding this to `QueryRoot` is what makes
//! the UI GraphQL and the MCP GraphQL carry the same Item field: a query issued
//! by the UI and a query issued by an MCP client return the same Items
//! (SPEC-2 acceptance 6).
//!
//! Resolvers wrap reads, matching the house pattern: they reach the connection-
//! tenant backend through the scoped invoker (`items_nodes` / `item_node`, which
//! wrap the backend's `query_nodes` / `get_node`) and run each node through the
//! projection. No Item shape logic lives here -- only the GraphQL wrapper.

use async_graphql::{InputObject, Object, Result as GqlResult, SimpleObject, ID};
use serde_json::{json, Value};

use super::projection::{self, ProjectedItem, PROJECTED_LABELS};
use super::scalars::Json;
use super::{map_err, with_invoker};

/// The Item GraphQL type: the projection's shape, presented to UI and MCP
/// clients alike. `kind` is an open token (`task` / `coordination` / `memory` /
/// `job` for projected harness/dispatch nodes; `note` / `file` / ... for real
/// commonplace Items). `extra` preserves the native node properties so nothing
/// from the source node is lost in projection.
#[derive(SimpleObject)]
pub struct Item {
    pub id: ID,
    pub kind: String,
    pub title: String,
    pub source: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub extra: Json,
}

impl From<ProjectedItem> for Item {
    fn from(p: ProjectedItem) -> Self {
        Self {
            id: ID(p.id),
            kind: p.kind,
            title: p.title,
            source: p.source,
            created_at_ms: p.created_at_ms,
            updated_at_ms: p.updated_at_ms,
            extra: Json(p.extra),
        }
    }
}

/// Default per-label read bound when the caller does not pass `limit`.
const DEFAULT_LIMIT: usize = 200;
/// Hard ceiling so a caller cannot ask the backend to enumerate unboundedly.
const MAX_LIMIT: usize = 10_000;

fn clamp_limit(limit: Option<i32>) -> usize {
    match limit {
        Some(n) if n > 0 => (n as usize).min(MAX_LIMIT),
        _ => DEFAULT_LIMIT,
    }
}

#[derive(Default)]
pub struct ItemQuery;

#[Object]
impl ItemQuery {
    /// All Items: real commonplace Items unioned with projected harness and
    /// dispatch nodes, each shaped by the one projection. `limit` bounds the
    /// per-label read.
    async fn items(&self, limit: Option<i32>) -> GqlResult<Vec<Item>> {
        let limit = clamp_limit(limit);
        let nodes = with_invoker(|inv| inv.items_nodes(&PROJECTED_LABELS, limit).map_err(map_err))?;
        Ok(nodes
            .iter()
            .filter_map(projection::project_node)
            .map(Item::from)
            .collect())
    }

    /// Items of a single kind: the projected kind token (e.g. `task` for a
    /// work-graph node, `note` for a real Item).
    async fn items_by_kind(&self, kind: String, limit: Option<i32>) -> GqlResult<Vec<Item>> {
        let limit = clamp_limit(limit);
        let nodes = with_invoker(|inv| inv.items_nodes(&PROJECTED_LABELS, limit).map_err(map_err))?;
        Ok(nodes
            .iter()
            .filter_map(projection::project_node)
            .filter(|p| p.kind == kind)
            .map(Item::from)
            .collect())
    }

    /// A single Item by id (real or projected), or null when the id is absent or
    /// is not a node the Item domain projects.
    async fn item(&self, id: ID) -> GqlResult<Option<Item>> {
        let node = with_invoker(|inv| inv.item_node(id.as_str()).map_err(map_err))?;
        Ok(node
            .as_ref()
            .and_then(projection::project_node)
            .map(Item::from))
    }
}

/// Input for `putItem`: create-or-update a real CommonPlace Item by id. Its shape
/// matches what SPEC-3's space-type repository sends (`id`/`kind`/`title`/
/// `source`/`extra`), so a space-type instance round-trips through this one write
/// seam. The optional `*_at_ms` fields let a caller pin timestamps; absent, they
/// default to now (and `created_at_ms` is preserved across updates).
#[derive(InputObject)]
pub struct ItemInput {
    pub id: Option<ID>,
    pub kind: String,
    pub title: String,
    pub source: Option<String>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
    pub extra: Option<Json>,
}

impl ItemInput {
    fn into_args(self) -> Value {
        let mut args = json!({ "kind": self.kind, "title": self.title });
        let obj = args.as_object_mut().expect("json object");
        if let Some(id) = self.id {
            obj.insert("id".to_string(), json!(id.to_string()));
        }
        if let Some(source) = self.source {
            obj.insert("source".to_string(), json!(source));
        }
        if let Some(created_at_ms) = self.created_at_ms {
            obj.insert("created_at_ms".to_string(), json!(created_at_ms));
        }
        if let Some(updated_at_ms) = self.updated_at_ms {
            obj.insert("updated_at_ms".to_string(), json!(updated_at_ms));
        }
        if let Some(extra) = self.extra {
            obj.insert("extra".to_string(), extra.0);
        }
        args
    }
}

/// The write half of the Item domain. SPEC-2's projection invariant ("never
/// mutate the harness/Theseus nodes it reads") is preserved: `putItem` writes
/// ONLY real `Item` nodes (the user-owned kind), never a projected harness or
/// dispatch node.
#[derive(Default)]
pub struct ItemMutation;

#[Object]
impl ItemMutation {
    /// Create-or-update a real CommonPlace Item (label `Item`) by id. The written
    /// node projects through the same projection and rides the changefeed, so a
    /// space-type instance appears to the UI exactly like any other Item.
    async fn put_item(&self, input: ItemInput) -> GqlResult<Item> {
        let node = with_invoker(|inv| inv.put_item(input.into_args()).map_err(map_err))?;
        projection::project_node(&node)
            .map(Item::from)
            .ok_or_else(|| async_graphql::Error::new("putItem did not return a projectable Item"))
    }
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{InMemoryGraphStore, NodeRecord};
    use serde_json::{json, Value};

    use crate::graphql::{execute_graphql, introspect_sdl, OpKind};

    /// A store holding one projected harness node (a work-graph TaskNode) and one
    /// real commonplace Item, so the union and the indistinguishability of
    /// harness-origin vs user-origin Items can be exercised end-to-end.
    fn store_with_a_task_and_an_item() -> InMemoryGraphStore {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(NodeRecord::new(
                "run-1::task-7",
                ["TaskNode"],
                json!({ "goal": "Ship SPEC-2", "created_at_ms": 1, "updated_at_ms": 2 }),
            ))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(
                "item-1",
                ["Item"],
                json!({ "kind": "note", "title": "My note", "created_at_ms": 3, "updated_at_ms": 3 }),
            ))
            .unwrap();
        store
    }

    fn run_query(store: InMemoryGraphStore, query: &str) -> Value {
        let args = json!({ "query": query });
        execute_graphql("tenant-a", store, &args, OpKind::Query).expect("graphql query runs")
    }

    // SPEC-2 acceptance 1 + 3: a work-graph task node yields a projected Item
    // through the `items` query carrying kind/title/timestamps/source + native
    // fields under `extra`; and a real Item appears in the SAME shape, so
    // harness-origin and user-origin Items are indistinguishable to the client.
    #[test]
    fn items_query_unions_projected_and_real_items_in_one_shape() {
        let resp = run_query(
            store_with_a_task_and_an_item(),
            "{ items { id kind title source createdAtMs updatedAtMs extra } }",
        );
        assert_eq!(resp["errors"], Value::Null, "no graphql errors: {resp}");
        let items = resp["data"]["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2, "task + item: {items:?}");

        let task = items
            .iter()
            .find(|i| i["id"] == "run-1::task-7")
            .expect("projected task present");
        assert_eq!(task["kind"], "task");
        assert_eq!(task["source"], "harness:task");
        assert_eq!(task["title"], "Ship SPEC-2");
        assert_eq!(task["createdAtMs"], 1);
        assert_eq!(task["updatedAtMs"], 2);
        // Native fields preserved under extra.
        assert_eq!(task["extra"]["goal"], "Ship SPEC-2");

        let item = items
            .iter()
            .find(|i| i["id"] == "item-1")
            .expect("real item present");
        // Same field set as the projected task; only kind/source differ by origin.
        assert_eq!(item["kind"], "note");
        assert_eq!(item["source"], "commonplace");
        assert_eq!(
            item.as_object().unwrap().keys().collect::<Vec<_>>(),
            task.as_object().unwrap().keys().collect::<Vec<_>>(),
            "harness-origin and user-origin Items share one shape"
        );
    }

    #[test]
    fn items_by_kind_filters_to_the_projected_kind() {
        let resp = run_query(
            store_with_a_task_and_an_item(),
            "{ itemsByKind(kind: \"task\") { id kind } }",
        );
        assert_eq!(resp["errors"], Value::Null, "{resp}");
        let items = resp["data"]["itemsByKind"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "run-1::task-7");
        assert_eq!(items[0]["kind"], "task");
    }

    #[test]
    fn item_by_id_returns_the_projected_item() {
        let resp = run_query(
            store_with_a_task_and_an_item(),
            "{ item(id: \"item-1\") { id kind title source } }",
        );
        assert_eq!(resp["errors"], Value::Null, "{resp}");
        let item = &resp["data"]["item"];
        assert_eq!(item["id"], "item-1");
        assert_eq!(item["kind"], "note");
        assert_eq!(item["source"], "commonplace");
    }

    #[test]
    fn item_by_id_is_null_for_an_unprojectable_node() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(NodeRecord::new(
                "sym-1",
                ["CodeSymbol"],
                json!({ "signature": "fn f()" }),
            ))
            .unwrap();
        let resp = run_query(store, "{ item(id: \"sym-1\") { id } }");
        assert_eq!(resp["errors"], Value::Null, "{resp}");
        assert_eq!(resp["data"]["item"], Value::Null);
    }

    // SPEC-2 acceptance 6: ItemQuery is a field on the one QueryRoot, so the SDL
    // that `graphql_introspect` returns shows the Item field both UI and MCP
    // clients query.
    #[test]
    fn item_domain_is_in_the_introspected_sdl() {
        let sdl = introspect_sdl();
        let sdl = sdl.as_str().expect("sdl is a string");
        assert!(sdl.contains("type Item"), "Item type in SDL");
        assert!(sdl.contains("items("), "items field in SDL");
        assert!(sdl.contains("itemsByKind("), "itemsByKind field in SDL");
        assert!(sdl.contains("createdAtMs"), "createdAtMs field in SDL");
        assert!(sdl.contains("putItem"), "putItem mutation in SDL");
        assert!(sdl.contains("input ItemInput"), "ItemInput type in SDL");
    }

    // SPEC-3 write seam: a space_type instance written via putItem reads back
    // through itemsByKind as a real Item carrying its extra (the exact round-trip
    // the space-type repository performs against this backend).
    #[test]
    fn put_item_round_trips_a_space_type_through_items_by_kind() {
        let shared = crate::SharedStore::new(InMemoryGraphStore::new());
        let mutate = json!({
            "query": "mutation($input: ItemInput!){ putItem(input: $input){ id kind title source } }",
            "variables": { "input": {
                "id": "space:notes",
                "kind": "space_type",
                "title": "Notes",
                "source": "commonplace:space-type-registry",
                "extra": { "type_key": "notes", "order": 50, "enabled": true }
            }}
        });
        let wrote = execute_graphql("tenant-a", shared.clone(), &mutate, OpKind::Mutate)
            .expect("mutation runs");
        assert_eq!(wrote["errors"], Value::Null, "{wrote}");
        assert_eq!(wrote["data"]["putItem"]["kind"], "space_type");
        assert_eq!(
            wrote["data"]["putItem"]["source"],
            "commonplace:space-type-registry"
        );

        let read = execute_graphql(
            "tenant-a",
            shared.clone(),
            &json!({ "query": "{ itemsByKind(kind: \"space_type\"){ id title extra } }" }),
            OpKind::Query,
        )
        .expect("query runs");
        assert_eq!(read["errors"], Value::Null, "{read}");
        let items = read["data"]["itemsByKind"].as_array().expect("items array");
        assert_eq!(items.len(), 1, "the written space_type reads back");
        assert_eq!(items[0]["id"], "space:notes");
        assert_eq!(items[0]["title"], "Notes");
        // extra preserves the space-type config under the nested `extra` key the
        // frontend's readItemExtra path reads.
        assert_eq!(items[0]["extra"]["extra"]["type_key"], "notes");
    }
}
