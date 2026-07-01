//! Data domain: typed GraphQL access to the RustyRed Data API envelope.
//! This wraps the same `data_api_payload` path that backs `query_data`.

use async_graphql::{Object, Result as GqlResult};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

fn args_from_input(input: Option<Json>) -> Value {
    input
        .map(|json| json.0)
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn insert_string(args: &mut Value, key: &str, value: Option<String>) {
    if let Some(value) = value {
        args[key] = json!(value);
    }
}

fn insert_i32(args: &mut Value, key: &str, value: Option<i32>) {
    if let Some(value) = value {
        args[key] = json!(value);
    }
}

fn insert_bool(args: &mut Value, key: &str, value: Option<bool>) {
    if let Some(value) = value {
        args[key] = json!(value);
    }
}

#[derive(Default)]
pub struct DataQuery;

#[Object]
impl DataQuery {
    /// Describe the records collections and envelope shape.
    async fn data_schema(&self) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.data("schema", json!({})).map_err(map_err)?)))
    }

    /// List records by collection/label with cursor paging.
    async fn data_records(
        &self,
        collection: Option<String>,
        label: Option<String>,
        limit: Option<i32>,
        cursor: Option<String>,
        hydrate_links: Option<bool>,
    ) -> GqlResult<Json> {
        let mut args = json!({});
        insert_string(&mut args, "collection", collection);
        insert_string(&mut args, "label", label);
        insert_i32(&mut args, "limit", limit);
        insert_string(&mut args, "cursor", cursor);
        insert_bool(&mut args, "hydrate_links", hydrate_links);
        with_invoker(|inv| Ok(Json(inv.data("records", args).map_err(map_err)?)))
    }

    /// Read one record by id.
    async fn data_record(&self, id: String, hydrate_links: Option<bool>) -> GqlResult<Json> {
        let mut args = json!({ "id": id });
        insert_bool(&mut args, "hydrate_links", hydrate_links);
        with_invoker(|inv| Ok(Json(inv.data("record", args).map_err(map_err)?)))
    }

    /// Read graph links for one record id.
    async fn data_links(
        &self,
        id: String,
        direction: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "id": id });
        insert_string(&mut args, "direction", direction);
        insert_i32(&mut args, "link_limit", limit);
        with_invoker(|inv| Ok(Json(inv.data("links", args).map_err(map_err)?)))
    }

    /// Run a flexible Data API query envelope.
    async fn data_query(
        &self,
        input: Option<Json>,
        collection: Option<String>,
        label: Option<String>,
        limit: Option<i32>,
        cursor: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = args_from_input(input);
        insert_string(&mut args, "collection", collection);
        insert_string(&mut args, "label", label);
        insert_i32(&mut args, "limit", limit);
        insert_string(&mut args, "cursor", cursor);
        with_invoker(|inv| Ok(Json(inv.data("query", args).map_err(map_err)?)))
    }

    /// Memory-oriented retrieval through the Data API envelope.
    async fn data_retrieve(
        &self,
        query: String,
        collection: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "query": query });
        insert_string(&mut args, "collection", collection);
        insert_i32(&mut args, "limit", limit);
        with_invoker(|inv| Ok(Json(inv.data("retrieve", args).map_err(map_err)?)))
    }

    /// List saved Data Views.
    async fn data_views(&self, limit: Option<i32>) -> GqlResult<Json> {
        let mut args = json!({});
        insert_i32(&mut args, "limit", limit);
        with_invoker(|inv| Ok(Json(inv.data("views", args).map_err(map_err)?)))
    }

    /// Fetch a saved Data View, optionally re-running the saved query.
    async fn data_view(&self, id: String, run: Option<bool>) -> GqlResult<Json> {
        let mut args = json!({ "id": id });
        insert_bool(&mut args, "run", run);
        with_invoker(|inv| Ok(Json(inv.data("view", args).map_err(map_err)?)))
    }
}

#[derive(Default)]
pub struct DataMutation;

#[Object]
impl DataMutation {
    /// Persist a saved Data View row as a tenant-scoped graph record.
    async fn upsert_data_view(
        &self,
        id: String,
        query: Json,
        title: Option<String>,
        description: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "id": id, "query": query.0 });
        insert_string(&mut args, "title", title);
        insert_string(&mut args, "description", description);
        with_invoker(|inv| Ok(Json(inv.data("upsert_view", args).map_err(map_err)?)))
    }
}
