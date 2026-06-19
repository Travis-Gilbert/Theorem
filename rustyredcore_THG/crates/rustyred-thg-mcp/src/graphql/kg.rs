//! Harness instant-KG domain (A5, KG half): the typed surface over the harness
//! instant-KG `view` (the flat `harness_kg_*` tools). Each field lowers to the
//! consolidated `instant_kg_payload` through `inv.instant_kg(operation, args)`;
//! no KG logic is reimplemented here. All six fields are reads.

use async_graphql::{Object, Result as GqlResult};
use serde_json::json;

use super::scalars::Json;
use super::{map_err, with_invoker};

#[derive(Default)]
pub struct HarnessKgQuery;

#[Object]
impl HarnessKgQuery {
    /// Instant-KG readiness status plus stats (wraps `harness_kg_status`).
    async fn harness_kg_status(&self) -> GqlResult<Json> {
        with_invoker(|inv| Ok(Json(inv.instant_kg("status", json!({})).map_err(map_err)?)))
    }

    /// Lexical / kind-filtered search over the instant KG (wraps `harness_kg_search`).
    async fn harness_kg_search(
        &self,
        query: String,
        kinds: Option<Vec<String>>,
        top_k: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "query": query });
        let obj = args.as_object_mut().expect("json object");
        if let Some(kinds) = kinds {
            obj.insert("kinds".to_string(), json!(kinds));
        }
        if let Some(top_k) = top_k {
            obj.insert("top_k".to_string(), json!(top_k));
        }
        with_invoker(|inv| Ok(Json(inv.instant_kg("search", args.clone()).map_err(map_err)?)))
    }

    /// Personalized PageRank over the instant KG (wraps `harness_kg_ppr`).
    async fn harness_kg_ppr(
        &self,
        seeds: Json,
        alpha: Option<f64>,
        epsilon: Option<f64>,
        max_pushes: Option<i32>,
        top_k: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "seeds": seeds.0 });
        let obj = args.as_object_mut().expect("json object");
        if let Some(alpha) = alpha {
            obj.insert("alpha".to_string(), json!(alpha));
        }
        if let Some(epsilon) = epsilon {
            obj.insert("epsilon".to_string(), json!(epsilon));
        }
        if let Some(max_pushes) = max_pushes {
            obj.insert("max_pushes".to_string(), json!(max_pushes));
        }
        if let Some(top_k) = top_k {
            obj.insert("top_k".to_string(), json!(top_k));
        }
        with_invoker(|inv| Ok(Json(inv.instant_kg("ppr", args.clone()).map_err(map_err)?)))
    }

    /// Impact traversal from a seed node or symbol name (wraps `harness_kg_impact`).
    async fn harness_kg_impact(
        &self,
        seed: Option<String>,
        symbol_name: Option<String>,
        direction: Option<String>,
        max_depth: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({});
        let obj = args.as_object_mut().expect("json object");
        if let Some(seed) = seed {
            obj.insert("seed".to_string(), json!(seed));
        }
        if let Some(symbol_name) = symbol_name {
            obj.insert("symbol_name".to_string(), json!(symbol_name));
        }
        if let Some(direction) = direction {
            obj.insert("direction".to_string(), json!(direction));
        }
        if let Some(max_depth) = max_depth {
            obj.insert("max_depth".to_string(), json!(max_depth));
        }
        with_invoker(|inv| Ok(Json(inv.instant_kg("impact", args.clone()).map_err(map_err)?)))
    }

    /// Related objects around a seed node (wraps `harness_kg_related_objects`).
    async fn harness_kg_related_objects(
        &self,
        seed: String,
        kinds: Option<Vec<String>>,
        top_k: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "seed": seed });
        let obj = args.as_object_mut().expect("json object");
        if let Some(kinds) = kinds {
            obj.insert("kinds".to_string(), json!(kinds));
        }
        if let Some(top_k) = top_k {
            obj.insert("top_k".to_string(), json!(top_k));
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.instant_kg("related_objects", args.clone())
                    .map_err(map_err)?,
            ))
        })
    }

    /// Explain the edge(s) between two instant-KG nodes (wraps `harness_kg_explain_edge`).
    async fn harness_kg_explain_edge(&self, src: String, dst: String) -> GqlResult<Json> {
        let args = json!({ "src": src, "dst": dst });
        with_invoker(|inv| {
            Ok(Json(
                inv.instant_kg("explain_edge", args.clone()).map_err(map_err)?,
            ))
        })
    }
}
