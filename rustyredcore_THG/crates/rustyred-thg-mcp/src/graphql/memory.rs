//! Memory domain (first slice). One typed schema over the twelve flat memory
//! tools: `recall`/`observe` -> `memory`, `relate` -> `MemoryDoc.related`, the
//! links array -> `MemoryDoc.links`, and remember/encode/revise/forget/handoff
//! as mutations. Resolvers WRAP the existing payload handlers via the scoped
//! invoker; nothing here re-implements memory logic.

use async_graphql::{ComplexObject, InputObject, Object, Result as GqlResult, SimpleObject, ID};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

/// A memory document, modeling the live shape returned by the memory tools.
#[derive(Clone, SimpleObject)]
#[graphql(complex)]
pub struct MemoryDoc {
    pub id: ID,
    pub kind: String,
    pub title: Option<String>,
    pub gist: Option<String>,
    pub summary: Option<String>,
    pub content_preview: Option<String>,
    pub content: String,
    pub served_tier: Option<String>,
    pub tags: Vec<String>,
    pub status: String,
    pub fitness: Option<f64>,
    /// The recorded feedback outcome, present only when the doc was written
    /// through encode (rememberMemory with an `outcome`).
    pub outcome: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[ComplexObject]
impl MemoryDoc {
    /// The doc's links, each resolved to its MemoryDoc. In this substrate a link
    /// is a `MEMORY_RELATES` edge, so the links array is the doc's one-hop
    /// `MEMORY_RELATES` neighborhood (turns the recall-then-fetch chain into one
    /// nested selection). `related` is the general, caller-parameterized traversal.
    async fn links(&self) -> GqlResult<Vec<MemoryDoc>> {
        let args = json!({
            "seed_id": self.id.to_string(),
            "edge_types": ["MEMORY_RELATES"],
            "max_hops": 1
        });
        with_invoker(|inv| {
            let value = inv.relate(args.clone()).map_err(map_err)?;
            Ok(items(&value, &["relations", "results"])
                .into_iter()
                .map(MemoryDoc::from_value)
                .collect())
        })
    }

    /// Neighbors via the `relate` traversal (recall-then-relate, one round trip).
    async fn related(
        &self,
        edge_types: Option<Vec<String>>,
        #[graphql(default = 1)] max_hops: i32,
    ) -> GqlResult<Vec<MemoryDoc>> {
        let mut args = json!({ "seed_id": self.id.to_string(), "max_hops": max_hops });
        if let Some(edge_types) = edge_types {
            args["edge_types"] = json!(edge_types);
        }
        with_invoker(|inv| {
            let value = inv.relate(args.clone()).map_err(map_err)?;
            Ok(items(&value, &["relations", "results"])
                .into_iter()
                .map(MemoryDoc::from_value)
                .collect())
        })
    }
}

impl MemoryDoc {
    /// Build a MemoryDoc from any of the live memory shapes (recall item,
    /// relation item, write-result document, or a raw node record). Reads
    /// defensively across the item itself and its nested document/node/properties.
    pub fn from_value(value: &Value) -> MemoryDoc {
        MemoryDoc {
            id: ID(
                str_field(value, &["id", "doc_id", "node_id", "docId", "nodeId"])
                    .unwrap_or_default(),
            ),
            kind: str_field(value, &["kind", "memory_node_type", "memoryNodeType"])
                .or_else(|| first_label(value))
                .unwrap_or_else(|| "memory".to_string()),
            title: str_field(value, &["title"]),
            gist: str_field(value, &["gist"]),
            summary: str_field(value, &["summary"]),
            content_preview: str_field(value, &["content_preview", "contentPreview"]),
            content: str_field(value, &["content"]).unwrap_or_default(),
            served_tier: str_field(value, &["served_tier", "servedTier"]),
            tags: arr_strings(value, &["tags"]),
            status: str_field(value, &["status"]).unwrap_or_else(|| "active".to_string()),
            fitness: f64_field(value, &["fitness", "score"]),
            outcome: outcome_field(value),
            created_at: str_field(value, &["created_at", "createdAt"]).unwrap_or_default(),
            updated_at: str_field(value, &["updated_at", "updatedAt"]).unwrap_or_default(),
        }
    }

    /// Build a MemoryDoc from a write-tool result. Different handlers wrap the
    /// saved doc under different keys: remember -> document/node, encode -> memory,
    /// self_revise -> revised, handoff -> handoff, self_archive -> archived.
    pub fn from_write_result(value: &Value) -> Option<MemoryDoc> {
        for key in [
            "document", "node", "memory", "revised", "handoff", "archived", "saved",
        ] {
            if let Some(inner) = value.get(key) {
                if !inner.is_null() {
                    return Some(MemoryDoc::from_value(inner));
                }
            }
        }
        // Some handlers return the doc fields at the top level.
        if value.get("id").is_some() || value.get("doc_id").is_some() {
            return Some(MemoryDoc::from_value(value));
        }
        None
    }
}

/// Input for remember/encode/revise. `outcome` present => encode (feedback)
/// semantics; absent => plain remember.
#[derive(InputObject)]
pub struct MemoryInput {
    pub content: String,
    pub kind: String,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub gist: Option<String>,
    pub tags: Option<Vec<String>>,
    pub links: Option<Vec<ID>>,
    pub outcome: Option<String>,
    pub signal: Option<String>,
}

impl MemoryInput {
    /// Lower to the handler argument object. Returns (args, is_encode).
    fn into_args(self) -> (Value, bool) {
        let is_encode = self.outcome.is_some();
        let mut args = json!({ "content": self.content, "kind": self.kind });
        let obj = args.as_object_mut().expect("json object");
        if let Some(title) = self.title {
            obj.insert("title".to_string(), json!(title));
        }
        if let Some(summary) = self.summary {
            obj.insert("summary".to_string(), json!(summary));
        }
        if let Some(gist) = self.gist {
            obj.insert("gist".to_string(), json!(gist));
        }
        if let Some(tags) = self.tags {
            obj.insert("tags".to_string(), json!(tags));
        }
        if let Some(links) = self.links {
            let ids: Vec<String> = links.into_iter().map(|id| id.to_string()).collect();
            obj.insert("links".to_string(), json!(ids));
        }
        if let Some(outcome) = self.outcome {
            obj.insert("outcome".to_string(), json!(outcome));
        }
        if let Some(signal) = self.signal {
            obj.insert("signal".to_string(), json!(signal));
        }
        (args, is_encode)
    }
}

#[derive(SimpleObject)]
pub struct ForgetResult {
    pub id: ID,
    pub deleted: bool,
}

#[derive(Default)]
pub struct MemoryQuery;

#[Object]
impl MemoryQuery {
    /// Recall memory docs (lowers to `recall`). Nested `related`/`links` walk the
    /// graph in the same round trip.
    #[allow(clippy::too_many_arguments)]
    async fn memory(
        &self,
        query: Option<String>,
        kind: Option<String>,
        tags: Option<Vec<String>>,
        #[graphql(default = 10)] limit: i32,
        #[graphql(default = false)] include_low_fitness: bool,
        recency_half_life_seconds: Option<f64>,
        ppr_alpha: Option<f64>,
        ppr_epsilon: Option<f64>,
        seed_limit: Option<i32>,
        query_time: Option<String>,
        #[graphql(default = false)] hydrate: bool,
        hydrate_top_k: Option<i32>,
        content_preview_chars: Option<i32>,
        detail: Option<String>,
        detail_top_k: Option<i32>,
        detail_ids: Option<Vec<ID>>,
    ) -> GqlResult<Vec<MemoryDoc>> {
        let mut args = json!({
            "limit": limit,
            "include_low_fitness": include_low_fitness,
            "record_recall_metadata": false
        });
        let obj = args.as_object_mut().expect("json object");
        if let Some(query) = query {
            obj.insert("query".to_string(), json!(query));
        }
        if let Some(kind) = kind {
            obj.insert("kind".to_string(), json!(kind));
        }
        if let Some(tags) = tags {
            obj.insert("tags".to_string(), json!(tags));
        }
        if let Some(half_life) = recency_half_life_seconds {
            obj.insert("recency_half_life_seconds".to_string(), json!(half_life));
        }
        if let Some(alpha) = ppr_alpha {
            obj.insert("ppr_alpha".to_string(), json!(alpha));
        }
        if let Some(epsilon) = ppr_epsilon {
            obj.insert("ppr_epsilon".to_string(), json!(epsilon));
        }
        if let Some(seed_limit) = seed_limit {
            obj.insert("seed_limit".to_string(), json!(seed_limit));
        }
        if let Some(query_time) = query_time {
            obj.insert("query_time".to_string(), json!(query_time));
        }
        if hydrate {
            obj.insert("hydrate".to_string(), json!(hydrate));
        }
        if let Some(hydrate_top_k) = hydrate_top_k {
            obj.insert("hydrate_top_k".to_string(), json!(hydrate_top_k));
        }
        if let Some(detail) = detail {
            obj.insert("detail".to_string(), json!(detail));
        }
        if let Some(detail_top_k) = detail_top_k {
            obj.insert("detail_top_k".to_string(), json!(detail_top_k));
        }
        if let Some(detail_ids) = detail_ids {
            let ids: Vec<String> = detail_ids.into_iter().map(|id| id.to_string()).collect();
            obj.insert("detail_ids".to_string(), json!(ids));
        }
        if let Some(content_preview_chars) = content_preview_chars {
            obj.insert(
                "content_preview_chars".to_string(),
                json!(content_preview_chars),
            );
        }
        with_invoker(|inv| {
            let value = inv.recall(args.clone()).map_err(map_err)?;
            Ok(items(&value, &["results"])
                .into_iter()
                .map(MemoryDoc::from_value)
                .collect())
        })
    }

    /// Fetch a single memory doc by id.
    async fn memory_doc(&self, id: ID) -> GqlResult<Option<MemoryDoc>> {
        let id = id.to_string();
        with_invoker(|inv| {
            Ok(inv
                .get_doc(&id)
                .map_err(map_err)?
                .map(|value| MemoryDoc::from_value(&value)))
        })
    }

    /// Recall from the archive (lowers to `self_recall_archive`).
    async fn memory_archive(
        &self,
        query: Option<String>,
        #[graphql(default = 10)] limit: i32,
    ) -> GqlResult<Vec<MemoryDoc>> {
        let mut args = json!({ "limit": limit });
        if let Some(query) = query {
            args["query"] = json!(query);
        }
        with_invoker(|inv| {
            let value = inv.archive_recall(args.clone()).map_err(map_err)?;
            Ok(items(&value, &["results"])
                .into_iter()
                .map(MemoryDoc::from_value)
                .collect())
        })
    }
}

#[derive(Default)]
pub struct MemoryMutation;

#[Object]
impl MemoryMutation {
    /// Persist a memory doc. `input.outcome` present => encode (feedback); absent
    /// => plain remember.
    async fn remember_memory(&self, input: MemoryInput) -> GqlResult<MemoryDoc> {
        let (args, is_encode) = input.into_args();
        with_invoker(|inv| {
            let value = if is_encode {
                inv.encode(args.clone())
            } else {
                inv.remember(args.clone())
            }
            .map_err(map_err)?;
            MemoryDoc::from_write_result(&value)
                .ok_or_else(|| async_graphql::Error::new("remember returned no document"))
        })
    }

    /// Revise an existing doc (lowers to `self_revise`).
    async fn revise_memory(
        &self,
        id: ID,
        input: MemoryInput,
        reason: Option<String>,
    ) -> GqlResult<MemoryDoc> {
        let (mut args, _) = input.into_args();
        args["doc_id"] = json!(id.to_string());
        if let Some(reason) = reason {
            args["reason"] = json!(reason);
        }
        with_invoker(|inv| {
            let value = inv.revise(args.clone()).map_err(map_err)?;
            MemoryDoc::from_write_result(&value)
                .ok_or_else(|| async_graphql::Error::new("revise returned no document"))
        })
    }

    /// Forget (archive) a doc (lowers to `forget`).
    async fn forget_memory(&self, id: ID, reason: String) -> GqlResult<ForgetResult> {
        let id_string = id.to_string();
        let args = json!({ "id": id_string, "doc_id": id_string, "reason": reason });
        with_invoker(|inv| {
            let value = inv.forget(args.clone()).map_err(map_err)?;
            let deleted = value.get("forgotten_type").is_some()
                || value.get("document").is_some()
                || value
                    .get("forgotten")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            Ok(ForgetResult {
                id: ID(id_string.clone()),
                deleted,
            })
        })
    }

    /// Create a handoff doc to another actor (lowers to `handoff`).
    async fn create_handoff(
        &self,
        to_actor: String,
        payload: Json,
        title: Option<String>,
    ) -> GqlResult<MemoryDoc> {
        let mut args = json!({
            "to_actor": to_actor,
            "payload": payload.0,
        });
        if let Some(title) = title {
            args["title"] = json!(title);
        }
        with_invoker(|inv| {
            let value = inv.handoff(args.clone()).map_err(map_err)?;
            MemoryDoc::from_write_result(&value)
                .ok_or_else(|| async_graphql::Error::new("handoff returned no document"))
        })
    }
}

// ---- defensive field extraction across the live memory Value shapes ----

fn items<'a>(value: &'a Value, keys: &[&str]) -> Vec<&'a Value> {
    for key in keys {
        if let Some(array) = value.get(key).and_then(Value::as_array) {
            return array.iter().collect();
        }
    }
    Vec::new()
}

fn nested(value: &Value) -> Vec<&Value> {
    let mut out = vec![value];
    for nest in ["document", "node", "properties"] {
        if let Some(inner) = value.get(nest) {
            out.push(inner);
            if let Some(props) = inner.get("properties") {
                out.push(props);
            }
        }
    }
    out
}

fn str_field(value: &Value, keys: &[&str]) -> Option<String> {
    for scope in nested(value) {
        for key in keys {
            if let Some(found) = scope.get(key).and_then(Value::as_str) {
                if !found.is_empty() {
                    return Some(found.to_string());
                }
            }
        }
    }
    None
}

fn f64_field(value: &Value, keys: &[&str]) -> Option<f64> {
    for scope in nested(value) {
        for key in keys {
            if let Some(found) = scope.get(key).and_then(Value::as_f64) {
                return Some(found);
            }
        }
    }
    None
}

fn arr_strings(value: &Value, keys: &[&str]) -> Vec<String> {
    for scope in nested(value) {
        for key in keys {
            if let Some(array) = scope.get(key).and_then(Value::as_array) {
                return array
                    .iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect();
            }
        }
    }
    Vec::new()
}

/// The recorded feedback outcome, read from `fitness.outcome` (encode) or a
/// top-level `outcome`. Absent for plain remember (which has no fitness outcome).
fn outcome_field(value: &Value) -> Option<String> {
    for scope in nested(value) {
        if let Some(outcome) = scope
            .get("fitness")
            .and_then(|fitness| fitness.get("outcome"))
            .and_then(Value::as_str)
        {
            if !outcome.is_empty() {
                return Some(outcome.to_string());
            }
        }
    }
    None
}

fn first_label(value: &Value) -> Option<String> {
    for scope in nested(value) {
        if let Some(label) = scope
            .get("labels")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_str)
        {
            return Some(label.to_string());
        }
    }
    None
}
