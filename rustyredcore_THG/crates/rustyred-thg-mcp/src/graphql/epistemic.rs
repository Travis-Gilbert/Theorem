//! Epistemic domain (A4): the typed surface over the shadow epistemic graph.
//! Wraps the flat epistemic tools -- `epistemicNeighbors` (the shadow-neighbor
//! query, the A4 acceptance field), `epistemicFrontier` (dirty frontier),
//! `compileSubgraph`, and `shadowPpr` (PPR over the shadow layer) as reads, and
//! `enrichApply` as a mutation. Every resolver lowers to the matching
//! `epistemic_*_payload` handler through the scoped invoker; no epistemic logic
//! is reimplemented here.
//!
//! Note: `hippoRetrieve` (HippoRAG candidate retrieval) lives on the async
//! product server, not this in-process MCP slice -- there is no payload to wrap
//! in this crate, so it is intentionally absent here rather than reimplemented.

use async_graphql::{Object, Result as GqlResult, ID};
use serde_json::json;

use super::scalars::Json;
use super::{map_err, with_invoker};

#[derive(Default)]
pub struct EpistemicQuery;

#[Object]
impl EpistemicQuery {
    /// Epistemic neighbors of a node over the shadow graph (wraps
    /// `rustyred_thg_epistemic_neighbors`). Returns the same `{node_id, results,
    /// stats}` payload, each result an `{edge, node}` shadow pair.
    async fn epistemic_neighbors(
        &self,
        node_id: ID,
        epistemic_types: Option<Vec<String>>,
        min_confidence: Option<f64>,
        max_depth: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "node_id": node_id.to_string() });
        let obj = args.as_object_mut().expect("json object");
        if let Some(epistemic_types) = epistemic_types {
            obj.insert("epistemic_types".to_string(), json!(epistemic_types));
        }
        if let Some(min_confidence) = min_confidence {
            obj.insert("min_confidence".to_string(), json!(min_confidence));
        }
        if let Some(max_depth) = max_depth {
            obj.insert("max_depth".to_string(), json!(max_depth));
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.epistemic_neighbors(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// The epistemic dirty frontier: shadow nodes needing (re)enrichment.
    async fn epistemic_frontier(
        &self,
        content_ids: Option<Vec<String>>,
        k_hops: Option<i32>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({});
        let obj = args.as_object_mut().expect("json object");
        if let Some(content_ids) = content_ids {
            obj.insert("content_ids".to_string(), json!(content_ids));
        }
        if let Some(k_hops) = k_hops {
            obj.insert("k_hops".to_string(), json!(k_hops));
        }
        if let Some(limit) = limit {
            obj.insert("limit".to_string(), json!(limit));
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.epistemic_dirty_frontier(args.clone())
                    .map_err(map_err)?,
            ))
        })
    }

    /// Compile the epistemic shadow subgraph for the given content nodes.
    async fn compile_subgraph(&self, content_ids: Vec<String>) -> GqlResult<Json> {
        let args = json!({ "content_ids": content_ids });
        with_invoker(|inv| {
            Ok(Json(
                inv.epistemic_compile_subgraph(args.clone())
                    .map_err(map_err)?,
            ))
        })
    }

    /// Personalized PageRank over the epistemic shadow layer.
    async fn shadow_ppr(
        &self,
        seeds: Json,
        top_k: Option<i32>,
        alpha: Option<f64>,
        epsilon: Option<f64>,
        max_pushes: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "seeds": seeds.0 });
        let obj = args.as_object_mut().expect("json object");
        if let Some(top_k) = top_k {
            obj.insert("top_k".to_string(), json!(top_k));
        }
        if let Some(alpha) = alpha {
            obj.insert("alpha".to_string(), json!(alpha));
        }
        if let Some(epsilon) = epsilon {
            obj.insert("epsilon".to_string(), json!(epsilon));
        }
        if let Some(max_pushes) = max_pushes {
            obj.insert("max_pushes".to_string(), json!(max_pushes));
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.epistemic_shadow_ppr(args.clone()).map_err(map_err)?,
            ))
        })
    }
}

#[derive(Default)]
pub struct EpistemicMutation;

#[Object]
impl EpistemicMutation {
    /// Apply epistemic enrichment annotations to the shadow graph (wraps
    /// `epistemic_enrich_apply`). `annotations` is the free-form enrichment
    /// payload the enrichment engine produced.
    #[allow(clippy::too_many_arguments)]
    async fn enrich_apply(
        &self,
        annotations: Json,
        content_ids: Option<Vec<String>>,
        mode: Option<String>,
        engine: Option<String>,
        engine_version: Option<String>,
        computed_at_ms: Option<i64>,
        density_floor: Option<f64>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "annotations": annotations.0 });
        let obj = args.as_object_mut().expect("json object");
        if let Some(content_ids) = content_ids {
            obj.insert("content_ids".to_string(), json!(content_ids));
        }
        if let Some(mode) = mode {
            obj.insert("mode".to_string(), json!(mode));
        }
        if let Some(engine) = engine {
            obj.insert("engine".to_string(), json!(engine));
        }
        if let Some(engine_version) = engine_version {
            obj.insert("engine_version".to_string(), json!(engine_version));
        }
        if let Some(computed_at_ms) = computed_at_ms {
            obj.insert("computed_at".to_string(), json!(computed_at_ms));
        }
        if let Some(density_floor) = density_floor {
            obj.insert("density_floor".to_string(), json!(density_floor));
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.epistemic_enrich_apply(args.clone()).map_err(map_err)?,
            ))
        })
    }
}
