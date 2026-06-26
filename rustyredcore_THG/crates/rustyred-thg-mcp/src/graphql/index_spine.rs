//! Adaptive index-spine inspection over GraphQL.
//!
//! This is the typed wrapper around the flat `rustyred_thg_index_spine` MCP
//! payload. The underlying payload reads graph-native inspection nodes
//! (`IndexManifest`, `QueryReceipt`, `IndexProposal`, context views, maps, and
//! training-export records) with strict limits, so agents can debug recall and
//! index behavior without triggering broad hydration.

use async_graphql::{Enum, Object, Result as GqlResult, SimpleObject, ID};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum IndexSpineSurface {
    IndexManifests,
    QueryReceipts,
    AdvisorProposals,
    ContextViews,
    Maps,
    TrainingRuns,
    TrainingExports,
}

impl IndexSpineSurface {
    fn as_arg(self) -> &'static str {
        match self {
            Self::IndexManifests => "index_manifests",
            Self::QueryReceipts => "query_receipts",
            Self::AdvisorProposals => "advisor_proposals",
            Self::ContextViews => "context_views",
            Self::Maps => "maps",
            Self::TrainingRuns => "training_runs",
            Self::TrainingExports => "training_exports",
        }
    }
}

#[derive(Clone, SimpleObject)]
pub struct IndexSpineRecord {
    pub node_id: ID,
    pub labels: Vec<String>,
    pub version: i64,
    pub properties: Json,
}

impl IndexSpineRecord {
    fn from_value(value: &Value) -> Self {
        Self {
            node_id: ID(value
                .get("node_id")
                .or_else(|| value.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()),
            labels: value
                .get("labels")
                .and_then(Value::as_array)
                .map(|labels| {
                    labels
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default(),
            version: value.get("version").and_then(Value::as_i64).unwrap_or(0),
            properties: Json(
                value
                    .get("properties")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            ),
        }
    }
}

#[derive(SimpleObject)]
pub struct IndexSpineOverview {
    pub ok: bool,
    pub counts: Json,
    pub reliability: Json,
    pub routes: Json,
    pub limit: i32,
}

impl IndexSpineOverview {
    fn from_payload(value: &Value) -> Self {
        Self {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(true),
            counts: Json(value.get("counts").cloned().unwrap_or_else(|| json!({}))),
            reliability: Json(
                value
                    .get("reliability")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            ),
            routes: Json(value.get("routes").cloned().unwrap_or_else(|| json!({}))),
            limit: value.get("limit").and_then(Value::as_i64).unwrap_or(0) as i32,
        }
    }
}

#[derive(SimpleObject)]
pub struct TrainingExportValidation {
    pub ok: bool,
    pub total_records: i32,
    pub status_counts: Json,
    pub blocked_record_ids: Vec<ID>,
    pub records: Vec<IndexSpineRecord>,
}

impl TrainingExportValidation {
    fn from_payload(value: &Value) -> Self {
        Self {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
            total_records: value
                .get("total_records")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32,
            status_counts: Json(
                value
                    .get("status_counts")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            ),
            blocked_record_ids: value
                .get("blocked_record_ids")
                .and_then(Value::as_array)
                .map(|ids| {
                    ids.iter()
                        .filter_map(Value::as_str)
                        .map(|id| ID(id.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            records: records_from_payload(value, "training_exports"),
        }
    }
}

#[derive(Default)]
pub struct IndexSpineQuery;

#[Object]
impl IndexSpineQuery {
    /// Bounded counts for every adaptive index-spine inspection class.
    async fn index_spine_overview(&self) -> GqlResult<IndexSpineOverview> {
        let payload = index_spine_payload(json!({
            "surface": "overview",
            "include_records": false,
        }))?;
        Ok(IndexSpineOverview::from_payload(&payload))
    }

    /// Generic bounded record list for one index-spine class.
    async fn index_spine_records(
        &self,
        surface: IndexSpineSurface,
        limit: Option<i32>,
    ) -> GqlResult<Vec<IndexSpineRecord>> {
        let key = surface.as_arg();
        let payload = index_spine_payload(args_for_surface(key, limit))?;
        Ok(records_from_payload(&payload, key))
    }

    async fn index_manifests(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("index_manifests", limit)
    }

    async fn query_receipts(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("query_receipts", limit)
    }

    async fn advisor_proposals(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("advisor_proposals", limit)
    }

    async fn context_views(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("context_views", limit)
    }

    async fn map_artifacts(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("maps", limit)
    }

    async fn training_runs(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("training_runs", limit)
    }

    async fn training_exports(&self, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
        records("training_exports", limit)
    }

    async fn training_export_validation(
        &self,
        limit: Option<i32>,
    ) -> GqlResult<TrainingExportValidation> {
        let mut args = args_for_surface("export_validation", limit);
        args["include_records"] = json!(true);
        let payload = index_spine_payload(args)?;
        Ok(TrainingExportValidation::from_payload(&payload))
    }
}

fn records(surface: &str, limit: Option<i32>) -> GqlResult<Vec<IndexSpineRecord>> {
    let payload = index_spine_payload(args_for_surface(surface, limit))?;
    Ok(records_from_payload(&payload, surface))
}

fn args_for_surface(surface: &str, limit: Option<i32>) -> Value {
    let mut args = json!({ "surface": surface });
    if let Some(limit) = limit.filter(|limit| *limit > 0) {
        args["limit"] = json!(limit);
    }
    args
}

fn index_spine_payload(args: Value) -> GqlResult<Value> {
    with_invoker(|inv| inv.index_spine(args.clone()).map_err(map_err))
}

fn records_from_payload(value: &Value, key: &str) -> Vec<IndexSpineRecord> {
    value
        .get(key)
        .or_else(|| value.get("records"))
        .and_then(Value::as_array)
        .map(|records| records.iter().map(IndexSpineRecord::from_value).collect())
        .unwrap_or_default()
}
