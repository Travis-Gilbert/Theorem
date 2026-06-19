//! Graph domain (A3): the typed multi-model query surface over the flat graph
//! tools. `graphAlgorithm` folds the eight algorithm tools into one enum field;
//! the remaining fields wrap `graphNode`, `neighbors`, `graphSchema`, the vector
//! / full-text / spatial searches, and the symbolic reads, with `designate*` and
//! `bulk*` as mutations. Every resolver lowers to the matching `*_payload`
//! handler through the scoped invoker; no graph logic is reimplemented here.

use async_graphql::{Enum, Object, Result as GqlResult, SimpleObject, ID};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

/// Which graph algorithm to run. The eight flat tools become this enum x `inline`.
#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum AlgorithmKind {
    Pagerank,
    Ppr,
    Communities,
    Components,
}

impl AlgorithmKind {
    fn as_str(self) -> &'static str {
        match self {
            AlgorithmKind::Pagerank => "PAGERANK",
            AlgorithmKind::Ppr => "PPR",
            AlgorithmKind::Communities => "COMMUNITIES",
            AlgorithmKind::Components => "COMPONENTS",
        }
    }
}

/// The raw algorithm result payload (scores / communities / components), as the
/// underlying tool returns it.
#[derive(SimpleObject)]
pub struct AlgorithmResult {
    pub result: Json,
}

/// A vector / full-text search hit: a node id and its score.
#[derive(SimpleObject)]
pub struct SearchHit {
    pub node_id: String,
    pub score: f64,
}

/// The result of a bulk node/edge upsert (mirrors the flat bulk-tool payload).
#[derive(SimpleObject)]
pub struct BulkResult {
    pub ok: bool,
    pub inserted: i32,
    pub failed: i32,
    pub errors: Json,
}

/// Parse the `results: [{node_id, score}]` shape the search payloads return.
fn search_hits(value: &Value) -> Vec<SearchHit> {
    value
        .get("results")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|hit| SearchHit {
                    node_id: hit
                        .get("node_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                    score: hit.get("score").and_then(Value::as_f64).unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the `node_ids: [String]` shape the spatial payloads return.
fn node_ids(value: &Value) -> Vec<String> {
    value
        .get("node_ids")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// Parse the bulk-upsert payload into the typed result.
fn bulk_result(value: &Value) -> BulkResult {
    BulkResult {
        ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        inserted: value.get("inserted").and_then(Value::as_i64).unwrap_or(0) as i32,
        failed: value.get("failed").and_then(Value::as_i64).unwrap_or(0) as i32,
        errors: Json(value.get("errors").cloned().unwrap_or(Value::Null)),
    }
}

#[derive(Default)]
pub struct GraphQuery;

#[Object]
impl GraphQuery {
    /// Run a graph algorithm. `kind` selects the algorithm (eight flat tools ->
    /// one field); `inline` chooses the adjacency-supplied inline variant.
    #[allow(clippy::too_many_arguments)]
    async fn graph_algorithm(
        &self,
        kind: AlgorithmKind,
        seeds: Option<Json>,
        damping: Option<f64>,
        alpha: Option<f64>,
        epsilon: Option<f64>,
        top_k: Option<i32>,
        #[graphql(default = false)] inline: bool,
    ) -> GqlResult<AlgorithmResult> {
        let mut args = json!({});
        let obj = args.as_object_mut().expect("json object");
        if let Some(seeds) = seeds {
            obj.insert("seeds".to_string(), seeds.0);
        }
        if let Some(damping) = damping {
            obj.insert("damping".to_string(), json!(damping));
        }
        if let Some(alpha) = alpha {
            obj.insert("alpha".to_string(), json!(alpha));
        }
        if let Some(epsilon) = epsilon {
            obj.insert("epsilon".to_string(), json!(epsilon));
        }
        if let Some(top_k) = top_k {
            obj.insert("top_k".to_string(), json!(top_k));
            obj.insert("topK".to_string(), json!(top_k));
        }
        let kind = kind.as_str();
        let result: Value =
            with_invoker(|inv| inv.algorithm(kind, inline, args.clone()).map_err(map_err))?;
        Ok(AlgorithmResult {
            result: Json(result),
        })
    }

    /// Fetch a single node by id (wraps `get_node`). Returns the raw node record.
    async fn graph_node(&self, id: ID) -> GqlResult<Option<Json>> {
        let id = id.to_string();
        with_invoker(|inv| Ok(inv.get_doc(&id).map_err(map_err)?.map(Json)))
    }

    /// One-hop neighbors of a node (wraps `graph_neighbors`). `direction` is
    /// `out` (default) or `in`; `edgeType` filters to a single edge type.
    async fn neighbors(
        &self,
        node_id: ID,
        direction: Option<String>,
        edge_type: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "node_id": node_id.to_string() });
        let obj = args.as_object_mut().expect("json object");
        if let Some(direction) = direction {
            obj.insert("direction".to_string(), json!(direction));
        }
        if let Some(edge_type) = edge_type {
            obj.insert("edge_type".to_string(), json!(edge_type));
        }
        if let Some(limit) = limit {
            obj.insert("limit".to_string(), json!(limit));
        }
        with_invoker(|inv| Ok(Json(inv.neighbors(args.clone()).map_err(map_err)?)))
    }

    /// The graph schema: labels, edge types, property keys, and index status.
    async fn graph_schema(&self) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.graph_schema().map_err(map_err)?)))
    }

    /// Vector (ANN) search over a designated vector property.
    async fn vector_search(
        &self,
        property: String,
        query: Vec<f64>,
        label: Option<String>,
        #[graphql(default = 10)] k: i32,
    ) -> GqlResult<Vec<SearchHit>> {
        let mut args = json!({ "property": property, "query": query, "k": k });
        if let Some(label) = label {
            args["label"] = json!(label);
        }
        with_invoker(|inv| {
            Ok(search_hits(
                &inv.vector_search(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Hybrid vector + graph search (wraps `vector_hybrid`).
    #[allow(clippy::too_many_arguments)]
    async fn vector_hybrid(
        &self,
        property: String,
        query: Vec<f64>,
        graph_seeds: Vec<String>,
        label: Option<String>,
        #[graphql(default = 10)] k: i32,
        max_hops: Option<i32>,
        alpha: Option<f64>,
    ) -> GqlResult<Vec<SearchHit>> {
        let mut args = json!({
            "property": property,
            "query": query,
            "graph_seeds": graph_seeds,
            "k": k,
        });
        let obj = args.as_object_mut().expect("json object");
        if let Some(label) = label {
            obj.insert("label".to_string(), json!(label));
        }
        if let Some(max_hops) = max_hops {
            obj.insert("max_hops".to_string(), json!(max_hops));
        }
        if let Some(alpha) = alpha {
            obj.insert("alpha".to_string(), json!(alpha));
        }
        with_invoker(|inv| {
            Ok(search_hits(
                &inv.vector_hybrid(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Full-text search over a designated full-text property.
    async fn fulltext_search(
        &self,
        property: String,
        query: String,
        label: Option<String>,
        #[graphql(default = 10)] k: i32,
    ) -> GqlResult<Vec<SearchHit>> {
        let mut args = json!({ "property": property, "query": query, "k": k });
        if let Some(label) = label {
            args["label"] = json!(label);
        }
        with_invoker(|inv| {
            Ok(search_hits(
                &inv.fulltext_search(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Spatial radius search (returns matching node ids).
    #[allow(clippy::too_many_arguments)]
    async fn spatial_radius(
        &self,
        label: String,
        lat_property: String,
        lon_property: String,
        lat: f64,
        lon: f64,
        radius_km: f64,
    ) -> GqlResult<Vec<String>> {
        let args = json!({
            "label": label,
            "lat_property": lat_property,
            "lon_property": lon_property,
            "lat": lat,
            "lon": lon,
            "radius_km": radius_km,
        });
        with_invoker(|inv| {
            Ok(node_ids(
                &inv.spatial_radius(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Spatial bounding-box search (returns matching node ids).
    #[allow(clippy::too_many_arguments)]
    async fn spatial_bbox(
        &self,
        label: String,
        lat_property: String,
        lon_property: String,
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    ) -> GqlResult<Vec<String>> {
        let args = json!({
            "label": label,
            "lat_property": lat_property,
            "lon_property": lon_property,
            "min_lat": min_lat,
            "min_lon": min_lon,
            "max_lat": max_lat,
            "max_lon": max_lon,
        });
        with_invoker(|inv| Ok(node_ids(&inv.spatial_bbox(args.clone()).map_err(map_err)?)))
    }

    /// Symbolic Datalog derivation (wraps `symbolic_datalog_derive`). `input` is
    /// the free-form rule/fact payload the engine expects.
    async fn derive_facts(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.derive_facts(input.0.clone()).map_err(map_err)?)))
    }

    /// Probabilistic source-reliability scoring (wraps the symbolic tool).
    async fn source_reliability(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| {
            Ok(Json(
                inv.source_reliability(input.0.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Probabilistic expected-value computation (wraps the symbolic tool).
    async fn expected_value(&self, input: Json) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.expected_value(input.0.clone()).map_err(map_err)?)))
    }
}

#[derive(Default)]
pub struct GraphMutation;

#[Object]
impl GraphMutation {
    /// Designate a vector property for ANN indexing.
    async fn designate_vector(
        &self,
        label: String,
        property: String,
        dimension: i32,
    ) -> GqlResult<Json> {
        let args = json!({ "label": label, "property": property, "dimension": dimension });
        with_invoker(|inv| Ok(Json(inv.designate_vector(args.clone()).map_err(map_err)?)))
    }

    /// Designate a lat/lon property pair for spatial indexing.
    async fn designate_spatial(
        &self,
        label: String,
        lat_property: String,
        lon_property: String,
        resolution: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({
            "label": label,
            "lat_property": lat_property,
            "lon_property": lon_property,
        });
        if let Some(resolution) = resolution {
            args["resolution"] = json!(resolution);
        }
        with_invoker(|inv| Ok(Json(inv.designate_spatial(args.clone()).map_err(map_err)?)))
    }

    /// Designate a property for full-text indexing.
    async fn designate_fulltext(&self, label: String, property: String) -> GqlResult<Json> {
        let args = json!({ "label": label, "property": property });
        with_invoker(|inv| Ok(Json(inv.designate_fulltext(args.clone()).map_err(map_err)?)))
    }

    /// Bulk-upsert nodes. `nodes` is an array of node records (id, labels,
    /// properties), matching the flat `bulk_nodes` tool.
    async fn bulk_nodes(&self, nodes: Json) -> GqlResult<BulkResult> {
        let args = json!({ "nodes": nodes.0 });
        with_invoker(|inv| Ok(bulk_result(&inv.bulk_nodes(args.clone()).map_err(map_err)?)))
    }

    /// Bulk-upsert edges. `edges` is an array of edge records (id, from_id,
    /// to_id, type, properties), matching the flat `bulk_edges` tool.
    async fn bulk_edges(&self, edges: Json) -> GqlResult<BulkResult> {
        let args = json!({ "edges": edges.0 });
        with_invoker(|inv| Ok(bulk_result(&inv.bulk_edges(args.clone()).map_err(map_err)?)))
    }
}
