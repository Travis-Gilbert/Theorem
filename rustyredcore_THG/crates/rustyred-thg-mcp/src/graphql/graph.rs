//! Graph domain (A3): the typed multi-model query surface over the flat graph
//! tools. `graphAlgorithm` folds the eight algorithm tools into one enum field;
//! the remaining fields wrap `graphNode`, `neighbors`, `graphSchema`, the vector
//! / full-text / spatial searches, and the symbolic reads, with `designate*` and
//! `bulk*` as mutations. Every resolver lowers to the matching `*_payload`
//! handler through the scoped invoker; no graph logic is reimplemented here.

use std::collections::BTreeMap;

use async_graphql::{Enum, InputObject, Object, Result as GqlResult, SimpleObject, ID};
use rustyred_thg_core::{
    apply_cascade, state::stable_hash, NodeRecord, QueryContext, RankCandidate, RankingRule,
    ThgError,
};
use rustyred_thg_ml::{
    project_multivector_tiers, quantize_sign_bits, rank_binary_hamming_maxsim,
    rerank_exact_maxsim_bounded, BinaryMultiVectorSet, MaxSimAggregation, MaxSimScorer,
    MultiVectorEmbeddingSet, MultiVectorManifest, MultiVectorScore,
};
use serde_json::{json, Map, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

const DEFAULT_EPISTEMIC_TYPES: &[&str] =
    &["supports", "contradicts", "tension", "derives", "cites"];
const HAS_EPISTEMIC_SHADOW_EDGE: &str = "HasEpistemicShadow";
const MULTIVECTOR_MANIFEST_LABEL: &str = "MultiVectorManifest";
const MULTIVECTOR_EXACT_LABEL: &str = "ColdMultiVectorArtifact";
const MULTIVECTOR_BINARY_LABEL: &str = "HotMultiVectorProjection";
const HAS_MULTIVECTOR_EDGE: &str = "HAS_MULTIVECTOR";
const HAS_EXACT_MULTIVECTOR_EDGE: &str = "HAS_EXACT_MULTIVECTOR";
const HAS_BINARY_MULTIVECTOR_EDGE: &str = "HAS_BINARY_MULTIVECTOR";

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
    pub epistemic_dirty_nodes_marked: i32,
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum MultiVectorAggregation {
    Sum,
    Mean,
}

impl From<MultiVectorAggregation> for MaxSimAggregation {
    fn from(value: MultiVectorAggregation) -> Self {
        match value {
            MultiVectorAggregation::Sum => MaxSimAggregation::Sum,
            MultiVectorAggregation::Mean => MaxSimAggregation::Mean,
        }
    }
}

#[derive(Enum, Copy, Clone, Eq, PartialEq, Debug)]
pub enum MultiVectorRankRule {
    Vector,
    GraphProximity,
    SourceReliability,
    Recency,
    EpistemicStatus,
}

impl From<MultiVectorRankRule> for RankingRule {
    fn from(value: MultiVectorRankRule) -> Self {
        match value {
            MultiVectorRankRule::Vector => RankingRule::Vector,
            MultiVectorRankRule::GraphProximity => RankingRule::GraphProximity,
            MultiVectorRankRule::SourceReliability => RankingRule::SourceReliability,
            MultiVectorRankRule::Recency => RankingRule::Recency,
            MultiVectorRankRule::EpistemicStatus => RankingRule::EpistemicStatus,
        }
    }
}

#[derive(InputObject)]
pub struct MultiVectorExactInput {
    pub embedding_set_id: String,
    pub content_id: String,
    pub model_id: String,
    pub model_version: String,
    pub vectors: Vec<Vec<f64>>,
    pub exact_object_ref: Option<String>,
    pub binary_projection_ref: Option<String>,
}

#[derive(Clone, SimpleObject)]
pub struct MultiVectorManifestView {
    pub embedding_set_id: String,
    pub content_id: String,
    pub model_id: String,
    pub model_version: String,
    pub dim: i32,
    pub vector_count: i32,
    pub exact_object_ref: Option<String>,
    pub binary_projection_ref: Option<String>,
    pub exact_bytes: i32,
    pub binary_projection_bytes: i32,
    pub exact_to_binary_byte_ratio: f64,
}

#[derive(SimpleObject)]
pub struct MultiVectorUpsertResult {
    pub manifest: MultiVectorManifestView,
    pub exact_artifact_id: String,
    pub binary_projection_id: String,
    pub binary_projection: Json,
}

#[derive(SimpleObject)]
pub struct MultiVectorSearchHit {
    pub content_id: String,
    pub embedding_set_id: String,
    pub score: f64,
    pub vector_score: f64,
    pub scorer: String,
    pub ranker: String,
    pub components: Json,
    pub vector_count: i32,
}

#[derive(Clone, serde::Serialize, SimpleObject)]
pub struct EpistemicProjectionReceipt {
    pub source: String,
    pub stale: bool,
    pub graph_version: Option<i64>,
    pub projection_version: Option<String>,
    pub computed_at_ms: Option<i64>,
    pub as_of_ms: Option<i64>,
}

#[derive(SimpleObject)]
pub struct EpistemicStandingView {
    pub source: String,
    pub stale: bool,
    pub scores: Json,
    pub support_in_degree: i32,
    pub attack_in_degree: i32,
    pub receipt: EpistemicProjectionReceipt,
}

#[derive(SimpleObject)]
pub struct EpistemicRelationshipView {
    pub relationship_id: String,
    pub source_id: String,
    pub target_id: String,
    pub relation_type: String,
    pub direction: String,
    pub confidence: Option<f64>,
    pub source_kind: Option<String>,
    pub evidence_ref: Option<String>,
    pub assertion_id: Option<String>,
    pub valid_from_ms: Option<i64>,
    pub valid_to_ms: Option<i64>,
    pub recorded_at_ms: Option<i64>,
    pub superseded_at_ms: Option<i64>,
    pub receipt: EpistemicProjectionReceipt,
    pub raw: Json,
}

#[derive(SimpleObject)]
pub struct EpistemicEvidenceView {
    pub evidence_ref: String,
    pub source: String,
    pub found: bool,
    pub body: Option<Json>,
}

#[derive(InputObject)]
pub struct EpistemicRelationshipPromotionInput {
    pub relationship_id: String,
    pub source_id: String,
    pub target_id: String,
    pub assertion_id: Option<String>,
    pub reason: Option<String>,
}

#[derive(SimpleObject)]
pub struct EpistemicRelationshipPromotionResult {
    pub assertion_id: String,
    pub assertion: Json,
    pub source_edge_id: String,
    pub target_edge_id: String,
    pub superseded_relationship: EpistemicRelationshipView,
}

pub struct ContentNode {
    id: String,
    raw: Json,
}

#[Object]
impl ContentNode {
    async fn id(&self) -> &str {
        &self.id
    }

    async fn raw(&self) -> Json {
        self.raw.clone()
    }

    async fn epistemic(&self) -> EpistemicFacet {
        EpistemicFacet {
            node_id: self.id.clone(),
        }
    }
}

pub struct EpistemicFacet {
    node_id: String,
}

#[Object]
impl EpistemicFacet {
    async fn standing(&self, top_k: Option<i32>, as_of_ms: Option<i64>) -> GqlResult<Json> {
        let standing = standing_view_for_node(&self.node_id, top_k, as_of_ms)?;
        Ok(Json(json!({
            "source": standing.source.clone(),
            "scores": standing.scores.0.clone(),
            "support_in_degree": standing.support_in_degree,
            "attack_in_degree": standing.attack_in_degree,
            "stale": standing.stale,
            "receipt": standing.receipt.clone(),
        })))
    }

    async fn standing_view(
        &self,
        top_k: Option<i32>,
        as_of_ms: Option<i64>,
    ) -> GqlResult<EpistemicStandingView> {
        standing_view_for_node(&self.node_id, top_k, as_of_ms)
    }

    async fn relationships(
        &self,
        epistemic_types: Option<Vec<String>>,
        min_confidence: Option<f64>,
        max_depth: Option<i32>,
        as_of_ms: Option<i64>,
    ) -> GqlResult<Json> {
        Ok(Json(Value::Array(relationship_hits_for_node(
            &self.node_id,
            epistemic_types,
            min_confidence,
            max_depth,
            as_of_ms,
        )?)))
    }

    async fn relationship_views(
        &self,
        epistemic_types: Option<Vec<String>>,
        min_confidence: Option<f64>,
        max_depth: Option<i32>,
        as_of_ms: Option<i64>,
    ) -> GqlResult<Vec<EpistemicRelationshipView>> {
        relationship_hits_for_node(
            &self.node_id,
            epistemic_types,
            min_confidence,
            max_depth,
            as_of_ms,
        )?
        .iter()
        .map(|hit| relationship_view_from_hit(&self.node_id, hit))
        .collect()
    }

    async fn evidence(&self, evidence_ref: String) -> GqlResult<EpistemicEvidenceView> {
        hydrate_evidence_ref(evidence_ref)
    }
}

fn standing_view_for_node(
    node_id: &str,
    top_k: Option<i32>,
    as_of_ms: Option<i64>,
) -> GqlResult<EpistemicStandingView> {
    with_invoker(|inv| {
        let mut seeds = serde_json::Map::new();
        seeds.insert(node_id.to_string(), json!(1.0));
        let args = json!({
            "seeds": Value::Object(seeds),
            "top_k": top_k.unwrap_or(8).max(1),
        });
        let shadow = if as_of_ms.is_none() {
            inv.epistemic_shadow_ppr(args.clone()).map_err(map_err)?
        } else {
            json!({ "scores": [] })
        };
        let relationship_args = epistemic_neighbor_args(node_id, None, None, Some(1));
        let relationships = inv
            .epistemic_neighbors(relationship_args.clone())
            .map_err(map_err)?;
        let direct = relationships
            .get("results")
            .and_then(Value::as_array)
            .cloned()
            .map(|items| filter_relationship_hits_as_of(items, as_of_ms))
            .unwrap_or_default();
        let (support_in_degree, attack_in_degree) = epistemic_degree_counts(&direct);
        let scores = shadow.get("scores").cloned().unwrap_or_else(|| json!([]));
        let has_scores = scores
            .as_array()
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);
        let shadow_node = if has_scores {
            first_shadow_node(&shadow)
                .and_then(|shadow_node_id| inv.get_doc(&shadow_node_id).ok().flatten())
        } else {
            None
        };
        let computed_at_ms = shadow_node.as_ref().and_then(|node| {
            value_at_path(node, &["properties", "computed_at"]).and_then(value_i64)
        });
        let projection_version = shadow_node.as_ref().and_then(|node| {
            value_at_path(node, &["properties", "engine_version"])
                .or_else(|| value_at_path(node, &["properties", "projection_version"]))
                .and_then(Value::as_str)
                .map(str::to_string)
        });
        let source = if has_scores {
            "shadow_ppr"
        } else if as_of_ms.is_some() {
            "degree_fallback_as_of"
        } else {
            "degree_fallback"
        }
        .to_string();
        let stale = !has_scores;
        Ok(EpistemicStandingView {
            source: source.clone(),
            stale,
            scores: Json(scores),
            support_in_degree: support_in_degree as i32,
            attack_in_degree: attack_in_degree as i32,
            receipt: EpistemicProjectionReceipt {
                source,
                stale,
                graph_version: None,
                projection_version,
                computed_at_ms,
                as_of_ms,
            },
        })
    })
}

fn relationship_hits_for_node(
    node_id: &str,
    epistemic_types: Option<Vec<String>>,
    min_confidence: Option<f64>,
    max_depth: Option<i32>,
    as_of_ms: Option<i64>,
) -> GqlResult<Vec<Value>> {
    let fallback_types = epistemic_types.clone();
    let explicit_empty_filter = epistemic_types
        .as_ref()
        .map(|values| values.is_empty())
        .unwrap_or(false);
    let args = epistemic_neighbor_args(node_id, epistemic_types, min_confidence, max_depth);
    let payload = with_invoker(|inv| inv.epistemic_neighbors(args.clone()).map_err(map_err))?;
    let mut results = payload
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if results.is_empty() && !explicit_empty_filter {
        results = legacy_shadow_relationships(node_id, fallback_types, min_confidence, max_depth)?;
    }
    results = filter_relationship_hits_as_of(results, as_of_ms);
    Ok(results)
}

fn filter_relationship_hits_as_of(results: Vec<Value>, as_of_ms: Option<i64>) -> Vec<Value> {
    let Some(as_of_ms) = as_of_ms else {
        return results;
    };
    results
        .into_iter()
        .filter(|hit| {
            hit.get("edge")
                .map(|edge| relationship_active_at(edge, as_of_ms))
                .unwrap_or(true)
        })
        .collect()
}

fn relationship_active_at(edge: &Value, as_of_ms: i64) -> bool {
    let valid_from = int_prop(edge, &["properties", "valid_from_ms"])
        .or_else(|| int_prop(edge, &["properties", "validFromMs"]));
    if valid_from.is_some_and(|valid_from| valid_from > as_of_ms) {
        return false;
    }
    let valid_to = int_prop(edge, &["properties", "valid_to_ms"])
        .or_else(|| int_prop(edge, &["properties", "validToMs"]));
    if valid_to.is_some_and(|valid_to| valid_to <= as_of_ms) {
        return false;
    }
    let recorded_at = int_prop(edge, &["properties", "recorded_at_ms"])
        .or_else(|| int_prop(edge, &["properties", "recordedAtMs"]));
    if recorded_at.is_some_and(|recorded_at| recorded_at > as_of_ms) {
        return false;
    }
    let superseded_at = int_prop(edge, &["properties", "superseded_at_ms"])
        .or_else(|| int_prop(edge, &["properties", "supersededAtMs"]));
    superseded_at.is_none_or(|superseded_at| superseded_at > as_of_ms)
}

fn hydrate_evidence_ref(evidence_ref: String) -> GqlResult<EpistemicEvidenceView> {
    with_invoker(|inv| {
        if let Some(content_hash) = cold_document_hash(&evidence_ref) {
            let body = inv
                .get_cold_document_bytes(content_hash)
                .map_err(map_err)?
                .map(|bytes| Json(evidence_body_from_bytes(&bytes)));
            return Ok(EpistemicEvidenceView {
                evidence_ref,
                source: "cold_document".to_string(),
                found: body.is_some(),
                body,
            });
        }
        if let Some(content_hash) = evidence_ref.strip_prefix("sha256:") {
            let cold_hash = format!("sha256:{content_hash}");
            let body = inv
                .get_cold_document_bytes(&cold_hash)
                .map_err(map_err)?
                .map(|bytes| Json(evidence_body_from_bytes(&bytes)));
            return Ok(EpistemicEvidenceView {
                evidence_ref,
                source: "cold_document_hash".to_string(),
                found: body.is_some(),
                body,
            });
        }
        if let Some(node_id) = evidence_ref.strip_prefix("graph://") {
            let body = inv.get_doc(node_id).map_err(map_err)?.map(Json);
            return Ok(EpistemicEvidenceView {
                evidence_ref,
                source: "graph_node".to_string(),
                found: body.is_some(),
                body,
            });
        }
        let body = inv.get_doc(&evidence_ref).map_err(map_err)?.map(Json);
        Ok(EpistemicEvidenceView {
            evidence_ref,
            source: "graph_node".to_string(),
            found: body.is_some(),
            body,
        })
    })
}

fn evidence_body_from_bytes(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).to_string()))
}

fn epistemic_neighbor_args(
    node_id: &str,
    epistemic_types: Option<Vec<String>>,
    min_confidence: Option<f64>,
    max_depth: Option<i32>,
) -> Value {
    let mut args = json!({ "node_id": node_id });
    let obj = args.as_object_mut().expect("json object");
    let epistemic_types = epistemic_types.unwrap_or_else(|| {
        DEFAULT_EPISTEMIC_TYPES
            .iter()
            .map(|value| (*value).to_string())
            .collect()
    });
    obj.insert("epistemic_types".to_string(), json!(epistemic_types));
    if let Some(min_confidence) = min_confidence {
        obj.insert("min_confidence".to_string(), json!(min_confidence));
    }
    if let Some(max_depth) = max_depth {
        obj.insert("max_depth".to_string(), json!(max_depth.max(1)));
    }
    args
}

fn legacy_shadow_relationships(
    node_id: &str,
    epistemic_types: Option<Vec<String>>,
    min_confidence: Option<f64>,
    max_depth: Option<i32>,
) -> GqlResult<Vec<Value>> {
    let bridge_args = json!({ "node_id": node_id, "max_depth": 1 });
    let bridge_payload = with_invoker(|inv| {
        inv.epistemic_neighbors(bridge_args.clone())
            .map_err(map_err)
    })?;
    let shadow_ids = bridge_payload
        .get("results")
        .and_then(Value::as_array)
        .map(|hits| {
            hits.iter()
                .filter(|hit| {
                    hit.get("edge")
                        .and_then(|edge| edge.get("type"))
                        .and_then(Value::as_str)
                        == Some(HAS_EPISTEMIC_SHADOW_EDGE)
                })
                .filter_map(|hit| {
                    hit.get("node")
                        .and_then(|node| node.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let mut results = Vec::new();
    for shadow_id in shadow_ids {
        let args = epistemic_neighbor_args(
            &shadow_id,
            epistemic_types.clone(),
            min_confidence,
            max_depth,
        );
        let payload = with_invoker(|inv| inv.epistemic_neighbors(args.clone()).map_err(map_err))?;
        if let Some(hits) = payload.get("results").and_then(Value::as_array) {
            results.extend(hits.iter().cloned());
        }
    }
    Ok(results)
}

fn epistemic_degree_counts(results: &[Value]) -> (usize, usize) {
    let mut support = 0usize;
    let mut attack = 0usize;
    for hit in results {
        match hit
            .get("edge")
            .and_then(|edge| edge.get("epistemic_type"))
            .and_then(Value::as_str)
        {
            Some("supports" | "derives" | "cites") => support += 1,
            Some("contradicts" | "tension") => attack += 1,
            _ => {}
        }
    }
    (support, attack)
}

fn first_shadow_node(payload: &Value) -> Option<String> {
    payload
        .get("scores")
        .and_then(Value::as_array)
        .and_then(|scores| scores.first())
        .and_then(|score| score.get("shadow_node_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn relationship_view_from_hit(
    anchor_node_id: &str,
    hit: &Value,
) -> GqlResult<EpistemicRelationshipView> {
    let edge = hit
        .get("edge")
        .ok_or_else(|| async_graphql::Error::new("relationship hit missing edge"))?;
    let node = hit.get("node").unwrap_or(&Value::Null);
    let relationship_id = value_at_path(edge, &["id"])
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let source_id = value_at_path(edge, &["from_id"])
        .or_else(|| value_at_path(edge, &["fromId"]))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let edge_target_id = value_at_path(edge, &["to_id"])
        .or_else(|| value_at_path(edge, &["toId"]))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let node_id = value_at_path(node, &["id"])
        .and_then(Value::as_str)
        .unwrap_or(edge_target_id.as_str())
        .to_string();
    let target_id = value_at_path(node, &["properties", "content_node_id"])
        .and_then(Value::as_str)
        .unwrap_or(node_id.as_str())
        .to_string();
    let direction = if source_id == anchor_node_id {
        "out"
    } else if edge_target_id == anchor_node_id {
        "in"
    } else {
        "projection"
    }
    .to_string();
    let relation_type = value_at_path(edge, &["epistemic_type"])
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            value_at_path(edge, &["type"])
                .and_then(Value::as_str)
                .map(normalize_relationship_type)
                .unwrap_or_default()
        });
    let confidence = value_at_path(edge, &["confidence"])
        .and_then(Value::as_f64)
        .or_else(|| value_at_path(edge, &["properties", "confidence"]).and_then(Value::as_f64));
    let source_kind = string_prop(edge, &["properties", "source_kind"])
        .or_else(|| string_prop(edge, &["properties", "sourceKind"]));
    let evidence_ref = string_prop(edge, &["properties", "evidence_ref"])
        .or_else(|| string_prop(edge, &["properties", "evidenceRef"]))
        .or_else(|| string_prop(edge, &["properties", "evidence"]));
    let assertion_id = string_prop(edge, &["properties", "assertion_id"])
        .or_else(|| string_prop(edge, &["properties", "assertionId"]))
        .or_else(|| string_prop(edge, &["properties", "promoted_to"]))
        .or_else(|| string_prop(edge, &["properties", "canonical_assertion_id"]));
    let projection_source = if value_array_contains(node, &["labels"], "EpistemicShadow") {
        "shadow_projection"
    } else {
        "canonical_edge"
    }
    .to_string();
    let stale = value_at_path(edge, &["properties", "stale"])
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(EpistemicRelationshipView {
        relationship_id,
        source_id,
        target_id,
        relation_type,
        direction,
        confidence,
        source_kind,
        evidence_ref,
        assertion_id,
        valid_from_ms: int_prop(edge, &["properties", "valid_from_ms"])
            .or_else(|| int_prop(edge, &["properties", "validFromMs"])),
        valid_to_ms: int_prop(edge, &["properties", "valid_to_ms"])
            .or_else(|| int_prop(edge, &["properties", "validToMs"])),
        recorded_at_ms: int_prop(edge, &["properties", "recorded_at_ms"])
            .or_else(|| int_prop(edge, &["properties", "recordedAtMs"])),
        superseded_at_ms: int_prop(edge, &["properties", "superseded_at_ms"])
            .or_else(|| int_prop(edge, &["properties", "supersededAtMs"])),
        receipt: EpistemicProjectionReceipt {
            source: projection_source,
            stale,
            graph_version: int_prop(edge, &["properties", "graph_version"])
                .or_else(|| int_prop(edge, &["properties", "graphVersion"])),
            projection_version: string_prop(edge, &["properties", "projection_version"])
                .or_else(|| string_prop(edge, &["properties", "projectionVersion"]))
                .or_else(|| string_prop(edge, &["properties", "engine_version"]))
                .or_else(|| string_prop(edge, &["properties", "engineVersion"])),
            computed_at_ms: int_prop(edge, &["properties", "computed_at"])
                .or_else(|| int_prop(edge, &["properties", "computedAt"]))
                .or_else(|| int_prop(edge, &["properties", "computed_at_ms"]))
                .or_else(|| int_prop(edge, &["properties", "computedAtMs"])),
            as_of_ms: None,
        },
        raw: Json(hit.clone()),
    })
}

fn normalize_relationship_type(value: &str) -> String {
    match value {
        "Supports" => "supports",
        "Undercuts" => "contradicts",
        "SameEClass" => "same_eclass",
        other => other,
    }
    .to_ascii_lowercase()
}

fn value_at_path<'a>(value: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    Some(current)
}

fn string_prop(value: &Value, path: &[&str]) -> Option<String> {
    value_at_path(value, path)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn int_prop(value: &Value, path: &[&str]) -> Option<i64> {
    value_at_path(value, path).and_then(value_i64)
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn value_array_contains(value: &Value, path: &[&str], expected: &str) -> bool {
    value_at_path(value, path)
        .and_then(Value::as_array)
        .map(|values| values.iter().any(|value| value.as_str() == Some(expected)))
        .unwrap_or(false)
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
    // Bulk calls are already bounded well below i32::MAX by request-size and
    // practical memory limits; GraphQL exposes signed 32-bit integers.
    BulkResult {
        ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
        inserted: value.get("inserted").and_then(Value::as_i64).unwrap_or(0) as i32,
        failed: value.get("failed").and_then(Value::as_i64).unwrap_or(0) as i32,
        errors: Json(value.get("errors").cloned().unwrap_or(Value::Null)),
        epistemic_dirty_nodes_marked: value
            .get("epistemic_dirty_nodes_marked")
            .and_then(Value::as_i64)
            .unwrap_or(0) as i32,
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
            // Support both backend spellings while flat tools migrate from
            // snake_case to GraphQL-style camelCase.
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

    /// Fetch a content node plus the epistemic facet. This wraps existing graph
    /// and epistemic payloads; the facet is the caller-facing seam.
    async fn content_node(&self, id: ID) -> GqlResult<Option<ContentNode>> {
        let id = id.to_string();
        with_invoker(|inv| {
            Ok(inv.get_doc(&id).map_err(map_err)?.map(|raw| ContentNode {
                id: id.clone(),
                raw: Json(raw),
            }))
        })
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

    /// ColPali-style multi-vector retrieval: hot binary/Hamming candidate search
    /// with bounded exact MaxSim rerank from the cold artifact node.
    #[allow(clippy::too_many_arguments)]
    async fn multi_vector_search(
        &self,
        query_vectors: Vec<Vec<f64>>,
        model_id: Option<String>,
        content_ids: Option<Vec<String>>,
        aggregation: Option<MultiVectorAggregation>,
        limit: Option<i32>,
        candidate_limit: Option<i32>,
        exact_rerank_limit: Option<i32>,
        catalog_limit: Option<i32>,
        graph_seed_ids: Option<Vec<String>>,
        rank_rules: Option<Vec<MultiVectorRankRule>>,
        include_epistemic: Option<bool>,
        as_of_ms: Option<i64>,
    ) -> GqlResult<Vec<MultiVectorSearchHit>> {
        let aggregation = aggregation.unwrap_or(MultiVectorAggregation::Mean).into();
        let limit = limit.unwrap_or(10).clamp(1, 256) as usize;
        let candidate_limit = candidate_limit.unwrap_or(limit as i32 * 4).clamp(1, 4096) as usize;
        let exact_rerank_limit = exact_rerank_limit.unwrap_or(0).clamp(0, 4096) as usize;
        let catalog_limit = catalog_limit.unwrap_or(4096).clamp(1, 50_000) as usize;
        let query_set = MultiVectorEmbeddingSet {
            embedding_set_id: "query:adhoc".to_string(),
            content_id: "query".to_string(),
            model_id: model_id.clone().unwrap_or_else(|| "adhoc".to_string()),
            model_version: "adhoc".to_string(),
            vectors: f64_matrix_to_f32(query_vectors)?,
        };
        query_set.dim().map_err(map_thg_err)?;
        let query_binary = quantize_sign_bits(&query_set).map_err(map_thg_err)?;
        let content_filter = content_ids.map(|ids| ids.into_iter().collect::<Vec<_>>());

        with_invoker(|inv| {
            let nodes = inv
                .items_nodes(&[MULTIVECTOR_BINARY_LABEL], catalog_limit)
                .map_err(map_err)?;
            let mut exact_by_embedding = BTreeMap::new();
            let mut binary_sets = Vec::new();
            for node in nodes {
                let Some(binary) = binary_set_from_node(&node)? else {
                    continue;
                };
                if binary.dim != query_binary.dim {
                    continue;
                }
                if let Some(model_id) = model_id.as_deref() {
                    if binary.model_id != model_id {
                        continue;
                    }
                }
                if let Some(content_filter) = content_filter.as_ref() {
                    if !content_filter.contains(&binary.content_id) {
                        continue;
                    }
                }
                if let Some(exact_id) = str_prop(&node, "exact_artifact_id") {
                    exact_by_embedding.insert(binary.embedding_set_id.clone(), exact_id);
                }
                binary_sets.push(binary);
            }
            if binary_sets.is_empty() {
                return Ok(Vec::new());
            }
            let binary_ranked = rank_binary_hamming_maxsim(
                &query_binary,
                &binary_sets,
                aggregation,
                candidate_limit,
            )
            .map_err(map_thg_err)?;

            let ranked = if exact_rerank_limit > 0 {
                rerank_exact_maxsim_bounded(
                    &query_set.vectors,
                    &binary_ranked,
                    exact_rerank_limit,
                    limit,
                    aggregation,
                    |candidate| {
                        let exact_id = exact_by_embedding
                            .get(&candidate.embedding_set_id)
                            .ok_or_else(|| {
                                ThgError::new(
                                    "multivector_exact_artifact_missing",
                                    format!(
                                        "no exact artifact pointer for {}",
                                        candidate.embedding_set_id
                                    ),
                                )
                            })?;
                        let node = inv
                            .item_node(exact_id)
                            .map_err(|error| {
                                ThgError::new(
                                    "multivector_exact_artifact_read",
                                    format!("{error:?}"),
                                )
                            })?
                            .ok_or_else(|| {
                                ThgError::new(
                                    "multivector_exact_artifact_missing",
                                    format!("exact artifact node {exact_id} is absent"),
                                )
                            })?;
                        if let Some(exact) = exact_set_from_node(&node)? {
                            return Ok(exact);
                        }
                        let exact_ref = str_prop(&node, "exact_object_ref").ok_or_else(|| {
                            ThgError::new(
                                "multivector_exact_artifact_invalid",
                                format!("exact artifact node {exact_id} has no vectors or ref"),
                            )
                        })?;
                        let Some(cold_hash) = cold_document_hash(&exact_ref) else {
                            return Err(ThgError::new(
                                "multivector_exact_artifact_invalid",
                                format!("unsupported exact vector ref {exact_ref}"),
                            ));
                        };
                        let bytes = inv
                            .get_cold_document_bytes(cold_hash)
                            .map_err(|error| {
                                ThgError::new(
                                    "multivector_exact_artifact_read",
                                    format!("{error:?}"),
                                )
                            })?
                            .ok_or_else(|| {
                                ThgError::new(
                                    "multivector_exact_artifact_missing",
                                    format!("cold exact vector object {cold_hash} is absent"),
                                )
                            })?;
                        exact_set_from_cold_bytes(&bytes)?.ok_or_else(|| {
                            ThgError::new(
                                "multivector_exact_artifact_invalid",
                                format!("cold exact vector object {cold_hash} is invalid"),
                            )
                        })
                    },
                )
                .map_err(map_thg_err)?
            } else {
                binary_ranked.into_iter().take(limit).collect()
            };

            let graph_seed_ids = graph_seed_ids.unwrap_or_default();
            let rank_rules = rank_rules.unwrap_or_default();
            let graph_aware = !graph_seed_ids.is_empty()
                || !rank_rules.is_empty()
                || include_epistemic.unwrap_or(false)
                || as_of_ms.is_some();
            if graph_aware {
                graph_aware_search_hits(
                    inv,
                    ranked,
                    graph_seed_ids,
                    rank_rules,
                    include_epistemic.unwrap_or(true),
                    as_of_ms,
                    limit,
                    catalog_limit,
                )
            } else {
                Ok(ranked.into_iter().map(search_hit_from_score).collect())
            }
        })
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

    /// Promote an epistemic relationship edge into a reified assertion node when
    /// the relationship needs identity, evidence, or its own lifecycle.
    async fn promote_epistemic_relationship(
        &self,
        input: EpistemicRelationshipPromotionInput,
    ) -> GqlResult<EpistemicRelationshipPromotionResult> {
        promote_epistemic_relationship(input)
    }

    /// Store exact multi-vectors off the content node, create the hot binary
    /// projection, and connect both through a manifest node.
    async fn upsert_multi_vector(
        &self,
        input: MultiVectorExactInput,
    ) -> GqlResult<MultiVectorUpsertResult> {
        let exact = MultiVectorEmbeddingSet {
            embedding_set_id: input.embedding_set_id,
            content_id: input.content_id,
            model_id: input.model_id,
            model_version: input.model_version,
            vectors: f64_matrix_to_f32(input.vectors)?,
        };
        exact.dim().map_err(map_thg_err)?;
        let exact_artifact_id = format!("multivector:exact:{}", exact.embedding_set_id);
        let binary_projection_id = format!("multivector:binary:{}", exact.embedding_set_id);
        let explicit_exact_object_ref = input.exact_object_ref;
        let binary_projection_ref = input.binary_projection_ref;

        with_invoker(|inv| {
            let cold_exact_hash = if explicit_exact_object_ref.is_none() {
                let exact_bytes = exact_payload_bytes(&exact)?;
                inv.put_cold_document_bytes(&exact_bytes).map_err(map_err)?
            } else {
                None
            };
            let exact_object_ref = explicit_exact_object_ref
                .clone()
                .or_else(|| cold_exact_hash.as_deref().map(cold_document_ref))
                .or_else(|| Some(format!("graph://{exact_artifact_id}")));
            let binary_projection_ref = binary_projection_ref
                .clone()
                .or_else(|| Some(format!("graph://{binary_projection_id}")));
            let bundle = project_multivector_tiers(&exact, exact_object_ref, binary_projection_ref)
                .map_err(map_thg_err)?;
            let manifest = bundle.manifest;
            let binary = bundle.binary_projection;
            let manifest_node =
                manifest_node_json(&manifest, &exact_artifact_id, &binary_projection_id);
            let exact_node = exact_node_json(
                &exact,
                &exact_artifact_id,
                manifest.exact_object_ref.as_deref(),
                cold_exact_hash.as_deref(),
            );
            let binary_node = binary_node_json(&binary, &exact_artifact_id, &binary_projection_id);
            let mut edges = vec![
                json!({
                    "id": format!("{}:{}:{}", HAS_EXACT_MULTIVECTOR_EDGE, exact.embedding_set_id, exact_artifact_id),
                    "from_id": exact.embedding_set_id,
                    "to_id": exact_artifact_id,
                    "type": HAS_EXACT_MULTIVECTOR_EDGE,
                }),
                json!({
                    "id": format!("{}:{}:{}", HAS_BINARY_MULTIVECTOR_EDGE, exact.embedding_set_id, binary_projection_id),
                    "from_id": exact.embedding_set_id,
                    "to_id": binary_projection_id,
                    "type": HAS_BINARY_MULTIVECTOR_EDGE,
                }),
            ];
            if inv.get_doc(&exact.content_id).map_err(map_err)?.is_some() {
                edges.push(json!({
                    "id": format!("{}:{}:{}", HAS_MULTIVECTOR_EDGE, exact.content_id, exact.embedding_set_id),
                    "from_id": exact.content_id,
                    "to_id": exact.embedding_set_id,
                    "type": HAS_MULTIVECTOR_EDGE,
                }));
            }
            let node_payload = inv
                .bulk_nodes(json!({ "nodes": [manifest_node, exact_node, binary_node] }))
                .map_err(map_err)?;
            ensure_bulk_ok(&node_payload, "upsertMultiVector nodes")?;
            let edge_payload = inv.bulk_edges(json!({ "edges": edges })).map_err(map_err)?;
            ensure_bulk_ok(&edge_payload, "upsertMultiVector edges")?;
            Ok(MultiVectorUpsertResult {
                manifest: manifest_view(&manifest),
                exact_artifact_id,
                binary_projection_id,
                binary_projection: Json(binary_projection_json(&binary)),
            })
        })
    }
}

fn f64_matrix_to_f32(vectors: Vec<Vec<f64>>) -> GqlResult<Vec<Vec<f32>>> {
    Ok(vectors
        .into_iter()
        .map(|row| row.into_iter().map(|value| value as f32).collect())
        .collect())
}

fn map_thg_err(err: ThgError) -> async_graphql::Error {
    async_graphql::Error::new(format!("{}: {}", err.code, err.message))
}

fn manifest_view(manifest: &MultiVectorManifest) -> MultiVectorManifestView {
    MultiVectorManifestView {
        embedding_set_id: manifest.embedding_set_id.clone(),
        content_id: manifest.content_id.clone(),
        model_id: manifest.model_id.clone(),
        model_version: manifest.model_version.clone(),
        dim: manifest.dim as i32,
        vector_count: manifest.vector_count as i32,
        exact_object_ref: manifest.exact_object_ref.clone(),
        binary_projection_ref: manifest.binary_projection_ref.clone(),
        exact_bytes: manifest.exact_bytes as i32,
        binary_projection_bytes: manifest.binary_projection_bytes as i32,
        exact_to_binary_byte_ratio: manifest.exact_to_binary_byte_ratio() as f64,
    }
}

fn manifest_node_json(
    manifest: &MultiVectorManifest,
    exact_artifact_id: &str,
    binary_projection_id: &str,
) -> Value {
    json!({
        "id": manifest.embedding_set_id.clone(),
        "labels": [MULTIVECTOR_MANIFEST_LABEL],
        "properties": {
            "content_id": manifest.content_id.clone(),
            "model_id": manifest.model_id.clone(),
            "model_version": manifest.model_version.clone(),
            "dim": manifest.dim,
            "vector_count": manifest.vector_count,
            "exact_object_ref": manifest.exact_object_ref.clone(),
            "binary_projection_ref": manifest.binary_projection_ref.clone(),
            "exact_artifact_id": exact_artifact_id,
            "binary_projection_id": binary_projection_id,
            "exact_bytes": manifest.exact_bytes,
            "binary_projection_bytes": manifest.binary_projection_bytes,
        }
    })
}

fn exact_node_json(
    exact: &MultiVectorEmbeddingSet,
    exact_artifact_id: &str,
    exact_object_ref: Option<&str>,
    cold_exact_hash: Option<&str>,
) -> Value {
    let mut properties = exact_payload_json(exact);
    let props = properties.as_object_mut().expect("exact payload object");
    if let Some(exact_object_ref) = exact_object_ref {
        props.insert(
            "exact_object_ref".to_string(),
            json!(exact_object_ref.to_string()),
        );
    }
    if let Some(cold_exact_hash) = cold_exact_hash {
        props.insert("cold_object_hash".to_string(), json!(cold_exact_hash));
        props.remove("vectors");
    }
    json!({
        "id": exact_artifact_id,
        "labels": [MULTIVECTOR_EXACT_LABEL],
        "properties": properties,
    })
}

fn exact_payload_json(exact: &MultiVectorEmbeddingSet) -> Value {
    json!({
        "embedding_set_id": exact.embedding_set_id.clone(),
        "content_id": exact.content_id.clone(),
        "model_id": exact.model_id.clone(),
        "model_version": exact.model_version.clone(),
        "vectors": exact.vectors.clone(),
    })
}

fn exact_payload_bytes(exact: &MultiVectorEmbeddingSet) -> GqlResult<Vec<u8>> {
    serde_json::to_vec(&exact_payload_json(exact)).map_err(|error| {
        async_graphql::Error::new(format!(
            "failed to encode exact multivector payload: {error}"
        ))
    })
}

fn cold_document_ref(content_hash: &str) -> String {
    format!("cold://document/{content_hash}")
}

fn cold_document_hash(object_ref: &str) -> Option<&str> {
    object_ref.strip_prefix("cold://document/")
}

fn binary_node_json(
    binary: &BinaryMultiVectorSet,
    exact_artifact_id: &str,
    binary_projection_id: &str,
) -> Value {
    let mut payload = binary_projection_json(binary);
    if let Value::Object(map) = &mut payload {
        map.insert("exact_artifact_id".to_string(), json!(exact_artifact_id));
    }
    json!({
        "id": binary_projection_id,
        "labels": [MULTIVECTOR_BINARY_LABEL],
        "properties": payload,
    })
}

fn binary_projection_json(binary: &BinaryMultiVectorSet) -> Value {
    json!({
        "embedding_set_id": binary.embedding_set_id.clone(),
        "content_id": binary.content_id.clone(),
        "model_id": binary.model_id.clone(),
        "model_version": binary.model_version.clone(),
        "dim": binary.dim,
        "vector_count": binary.vector_count,
        "words_per_vector": binary.words_per_vector,
        "words_hex": binary.words.iter().map(|word| format!("{word:016x}")).collect::<Vec<_>>(),
    })
}

fn search_hit_from_score(score: MultiVectorScore) -> MultiVectorSearchHit {
    let vector_score = score.score as f64;
    MultiVectorSearchHit {
        content_id: score.content_id,
        embedding_set_id: score.embedding_set_id,
        score: vector_score,
        vector_score,
        scorer: match score.scorer {
            MaxSimScorer::ExactFloat => "exact_float",
            MaxSimScorer::BinaryHamming => "binary_hamming",
        }
        .to_string(),
        ranker: "vector_only".to_string(),
        components: Json(json!({
            "vector_score": vector_score,
            "vector_score01": vector_score01(score.score),
            "rank_rules": ["vector"],
        })),
        vector_count: score.vector_count as i32,
    }
}

#[allow(clippy::too_many_arguments)]
fn graph_aware_search_hits(
    inv: &dyn super::GraphqlInvoker,
    scores: Vec<MultiVectorScore>,
    graph_seed_ids: Vec<String>,
    rank_rules: Vec<MultiVectorRankRule>,
    include_epistemic: bool,
    as_of_ms: Option<i64>,
    limit: usize,
    catalog_limit: usize,
) -> GqlResult<Vec<MultiVectorSearchHit>> {
    let graph_scores = if graph_seed_ids.is_empty() {
        BTreeMap::new()
    } else {
        graph_ppr_scores(inv, &graph_seed_ids, catalog_limit)?
    };
    let rules = if rank_rules.is_empty() {
        vec![
            RankingRule::Vector,
            RankingRule::GraphProximity,
            RankingRule::SourceReliability,
            RankingRule::EpistemicStatus,
            RankingRule::Recency,
        ]
    } else {
        rank_rules.into_iter().map(Into::into).collect::<Vec<_>>()
    };
    let query = QueryContext {
        as_of_ms,
        recency_reference_ms: as_of_ms,
        ..Default::default()
    };

    let mut components_by_embedding = BTreeMap::new();
    let candidates = scores
        .iter()
        .map(|score| {
            let mut candidate = RankCandidate::new(score.embedding_set_id.clone());
            let vector_score01 = vector_score01(score.score);
            candidate.vector_score = Some(vector_score01);
            if let Some(graph_score) = graph_scores.get(&score.content_id).copied() {
                candidate.graph_hops = Some(graph_score_to_hops(graph_score));
            }
            if include_epistemic || as_of_ms.is_some() {
                apply_epistemic_rank_signals(inv, &score.content_id, &mut candidate, as_of_ms)?;
            }
            components_by_embedding.insert(
                score.embedding_set_id.clone(),
                json!({
                    "vector_score": score.score,
                    "vector_score01": vector_score01,
                    "graph_score": graph_scores.get(&score.content_id).copied(),
                    "graph_hops": candidate.graph_hops,
                    "source_reliability": candidate.source_reliability,
                    "acceptance_status": candidate.acceptance_status,
                    "epistemic_weight": candidate.epistemic_weight,
                    "valid_from_ms": candidate.valid_from_ms,
                    "superseded": candidate.superseded,
                }),
            );
            Ok(candidate)
        })
        .collect::<GqlResult<Vec<_>>>()?;
    let outcome = apply_cascade(candidates, &rules, &query);
    let by_embedding = scores
        .into_iter()
        .map(|score| (score.embedding_set_id.clone(), score))
        .collect::<BTreeMap<_, _>>();
    let n = outcome.ranked.len();
    let rule_names = outcome.rule_order.clone();
    let mut out = Vec::new();
    for (idx, ranked) in outcome.ranked.into_iter().enumerate() {
        let Some(score) = by_embedding.get(&ranked.row_id) else {
            continue;
        };
        let mut components = components_by_embedding
            .remove(&ranked.row_id)
            .unwrap_or_else(|| json!({}));
        if let Some(map) = components.as_object_mut() {
            map.insert("rank_rules".to_string(), json!(rule_names.clone()));
            map.insert("rank_buckets".to_string(), json!(ranked.buckets));
            map.insert("as_of_ms".to_string(), json!(as_of_ms));
        }
        out.push(MultiVectorSearchHit {
            content_id: score.content_id.clone(),
            embedding_set_id: score.embedding_set_id.clone(),
            score: (n - idx) as f64,
            vector_score: score.score as f64,
            scorer: match score.scorer {
                MaxSimScorer::ExactFloat => "exact_float",
                MaxSimScorer::BinaryHamming => "binary_hamming",
            }
            .to_string(),
            ranker: "graph_aware_cascade".to_string(),
            components: Json(components),
            vector_count: score.vector_count as i32,
        });
        if out.len() >= limit {
            break;
        }
    }
    Ok(out)
}

fn graph_ppr_scores(
    inv: &dyn super::GraphqlInvoker,
    seed_ids: &[String],
    top_k: usize,
) -> GqlResult<BTreeMap<String, f64>> {
    let seeds = seed_ids
        .iter()
        .filter(|seed| !seed.trim().is_empty())
        .map(|seed| (seed.clone(), json!(1.0)))
        .collect::<Map<_, _>>();
    if seeds.is_empty() {
        return Ok(BTreeMap::new());
    }
    let payload = inv
        .algorithm(
            "PPR",
            false,
            json!({
                "seeds": Value::Object(seeds),
                "top_k": top_k.max(1),
            }),
        )
        .map_err(map_err)?;
    Ok(payload
        .get("scores")
        .and_then(Value::as_array)
        .map(|scores| {
            scores
                .iter()
                .filter_map(|score| {
                    Some((
                        score.get("node_id")?.as_str()?.to_string(),
                        score.get("score")?.as_f64()?,
                    ))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default())
}

fn apply_epistemic_rank_signals(
    inv: &dyn super::GraphqlInvoker,
    content_id: &str,
    candidate: &mut RankCandidate,
    as_of_ms: Option<i64>,
) -> GqlResult<()> {
    if let Some(node) = inv.item_node(content_id).map_err(map_err)? {
        candidate.acceptance_status = node_prop_str(&node, "acceptance_status").unwrap_or_default();
        candidate.epistemic_weight = node_prop_f64(&node, "epistemic_weight")
            .map(|value| value as f32)
            .unwrap_or(candidate.epistemic_weight);
        candidate.source_reliability = node_prop_f64(&node, "source_reliability")
            .or_else(|| node_prop_f64(&node, "justification_prior"))
            .map(|value| value as f32);
        candidate.valid_from_ms = node_prop_i64(&node, "valid_from_ms");
        candidate.superseded = node_prop_bool(&node, "superseded").unwrap_or(false);
    }
    if as_of_ms.is_some() {
        return Ok(());
    }
    let shadow = first_shadow_for_content(inv, content_id)?;
    let Some(shadow) = shadow else {
        return Ok(());
    };
    if candidate.acceptance_status.is_empty() {
        if let Some(status) = node_prop_str(&shadow, "grounded_extension_status") {
            candidate.acceptance_status = match status.as_str() {
                "in" => "supported",
                "out" => "undercut",
                "undecided" => "provisional",
                other => other,
            }
            .to_string();
        }
    }
    if let Some(mean) = shadow_source_reliability_mean(&shadow) {
        candidate.source_reliability = Some(mean as f32);
    }
    if (candidate.epistemic_weight - 1.0).abs() < f32::EPSILON {
        candidate.epistemic_weight = match candidate.acceptance_status.as_str() {
            "supported" | "grounded" => 1.2,
            "undercut" | "attacked" => 0.8,
            _ => 1.0,
        };
    }
    Ok(())
}

fn first_shadow_for_content(
    inv: &dyn super::GraphqlInvoker,
    content_id: &str,
) -> GqlResult<Option<NodeRecord>> {
    let mut seeds = Map::new();
    seeds.insert(content_id.to_string(), json!(1.0));
    let payload = inv
        .epistemic_shadow_ppr(json!({ "seeds": Value::Object(seeds), "top_k": 1 }))
        .map_err(map_err)?;
    let Some(shadow_id) = first_shadow_node(&payload) else {
        return Ok(None);
    };
    inv.item_node(&shadow_id).map_err(map_err)
}

fn graph_score_to_hops(score: f64) -> u32 {
    if score <= 0.0 {
        return u32::MAX;
    }
    ((1.0 / score) - 1.0).round().max(0.0) as u32
}

fn vector_score01(score: f32) -> f32 {
    ((score + 1.0) / 2.0).clamp(0.0, 1.0)
}

fn node_prop_str(node: &NodeRecord, key: &str) -> Option<String> {
    node.properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn node_prop_f64(node: &NodeRecord, key: &str) -> Option<f64> {
    node.properties.get(key).and_then(Value::as_f64)
}

fn node_prop_i64(node: &NodeRecord, key: &str) -> Option<i64> {
    node.properties.get(key).and_then(value_i64)
}

fn node_prop_bool(node: &NodeRecord, key: &str) -> Option<bool> {
    node.properties.get(key).and_then(Value::as_bool)
}

fn shadow_source_reliability_mean(node: &NodeRecord) -> Option<f64> {
    node.properties
        .get("source_reliability")
        .and_then(|value| value.get("mean"))
        .and_then(Value::as_f64)
}

fn binary_set_from_node(node: &NodeRecord) -> GqlResult<Option<BinaryMultiVectorSet>> {
    let props = node.properties.as_object();
    let Some(props) = props else {
        return Ok(None);
    };
    let Some(words_hex) = props.get("words_hex").and_then(Value::as_array) else {
        return Ok(None);
    };
    let words = words_hex
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| async_graphql::Error::new("words_hex values must be strings"))
                .and_then(|raw| {
                    u64::from_str_radix(raw, 16).map_err(|error| {
                        async_graphql::Error::new(format!("invalid words_hex value: {error}"))
                    })
                })
        })
        .collect::<GqlResult<Vec<_>>>()?;
    Ok(Some(BinaryMultiVectorSet {
        embedding_set_id: prop_str(props, "embedding_set_id").unwrap_or_else(|| node.id.clone()),
        content_id: prop_str(props, "content_id").unwrap_or_default(),
        model_id: prop_str(props, "model_id").unwrap_or_default(),
        model_version: prop_str(props, "model_version").unwrap_or_default(),
        dim: prop_usize(props, "dim").unwrap_or_default(),
        vector_count: prop_usize(props, "vector_count").unwrap_or_default(),
        words_per_vector: prop_usize(props, "words_per_vector").unwrap_or_default(),
        words,
    }))
}

fn exact_set_from_node(node: &NodeRecord) -> Result<Option<MultiVectorEmbeddingSet>, ThgError> {
    let Some(props) = node.properties.as_object() else {
        return Ok(None);
    };
    exact_set_from_props(&node.id, props)
}

fn exact_set_from_cold_bytes(bytes: &[u8]) -> Result<Option<MultiVectorEmbeddingSet>, ThgError> {
    let payload: Value = serde_json::from_slice(bytes).map_err(|error| {
        ThgError::new(
            "multivector_exact_artifact_invalid",
            format!("cold exact vector payload is not JSON: {error}"),
        )
    })?;
    let Some(props) = payload.as_object() else {
        return Err(ThgError::new(
            "multivector_exact_artifact_invalid",
            "cold exact vector payload must be an object",
        ));
    };
    exact_set_from_props("cold:exact", props)
}

fn exact_set_from_props(
    fallback_id: &str,
    props: &Map<String, Value>,
) -> Result<Option<MultiVectorEmbeddingSet>, ThgError> {
    let Some(vectors) = props.get("vectors").and_then(Value::as_array) else {
        return Ok(None);
    };
    let vectors = vectors
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or_else(|| {
                    ThgError::new(
                        "multivector_exact_artifact_invalid",
                        "exact vectors must be an array of arrays",
                    )
                })
                .and_then(|row| {
                    row.iter()
                        .map(|value| {
                            value.as_f64().map(|value| value as f32).ok_or_else(|| {
                                ThgError::new(
                                    "multivector_exact_artifact_invalid",
                                    "exact vector values must be numbers",
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let set = MultiVectorEmbeddingSet {
        embedding_set_id: prop_str(props, "embedding_set_id").unwrap_or_else(|| fallback_id.into()),
        content_id: prop_str(props, "content_id").unwrap_or_default(),
        model_id: prop_str(props, "model_id").unwrap_or_default(),
        model_version: prop_str(props, "model_version").unwrap_or_default(),
        vectors,
    };
    set.dim()?;
    Ok(Some(set))
}

fn str_prop(node: &NodeRecord, key: &str) -> Option<String> {
    node.properties
        .as_object()
        .and_then(|props| prop_str(props, key))
}

fn prop_str(props: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    props
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|value| !value.is_empty())
}

fn prop_usize(props: &serde_json::Map<String, Value>, key: &str) -> Option<usize> {
    props
        .get(key)
        .and_then(Value::as_u64)
        .map(|value| value as usize)
}

fn ensure_bulk_ok(payload: &Value, operation: &str) -> GqlResult<()> {
    if payload.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        Ok(())
    } else {
        Err(async_graphql::Error::new(format!(
            "{operation} failed: {}",
            payload
        )))
    }
}

fn promote_epistemic_relationship(
    input: EpistemicRelationshipPromotionInput,
) -> GqlResult<EpistemicRelationshipPromotionResult> {
    let hits = relationship_hits_for_node(&input.source_id, None, None, Some(1), None)?;
    let hit = hits
        .iter()
        .find(|hit| {
            value_at_path(hit, &["edge", "id"]).and_then(Value::as_str)
                == Some(input.relationship_id.as_str())
        })
        .ok_or_else(|| {
            async_graphql::Error::new(format!(
                "relationship {} was not reachable from {}",
                input.relationship_id, input.source_id
            ))
        })?;
    let view = relationship_view_from_hit(&input.source_id, hit)?;
    let assertion_id = input.assertion_id.unwrap_or_else(|| {
        format!(
            "epistemic:assertion:{}",
            stable_hash(json!([
                input.relationship_id,
                input.source_id,
                input.target_id
            ]))
        )
    });
    let promoted_at_ms = unix_ms();
    let source_edge_id = format!("epistemic:assertion_source:{assertion_id}");
    let target_edge_id = format!("epistemic:assertion_target:{assertion_id}");
    let assertion = json!({
        "id": assertion_id,
        "labels": ["EpistemicAssertion"],
        "properties": {
            "relationship_id": view.relationship_id,
            "source_id": input.source_id,
            "target_id": input.target_id,
            "relation_type": view.relation_type,
            "confidence": view.confidence,
            "source_kind": view.source_kind,
            "evidence_ref": view.evidence_ref,
            "valid_from_ms": view.valid_from_ms,
            "valid_to_ms": view.valid_to_ms,
            "recorded_at_ms": view.recorded_at_ms.unwrap_or(promoted_at_ms),
            "promoted_at_ms": promoted_at_ms,
            "promotion_reason": input.reason.unwrap_or_else(|| "relationship_needed_identity".to_string()),
        }
    });
    let superseded_edge = promoted_relationship_edge_payload(hit, &assertion_id, promoted_at_ms)?;
    let source_edge = json!({
        "id": source_edge_id,
        "from_id": input.source_id,
        "to_id": assertion_id,
        "type": "ASSERTS_RELATION_SOURCE",
        "properties": {
            "relationship_id": view.relationship_id,
            "promoted_at_ms": promoted_at_ms,
        }
    });
    let target_edge = json!({
        "id": target_edge_id,
        "from_id": assertion_id,
        "to_id": input.target_id,
        "type": "ASSERTS_RELATION_TARGET",
        "properties": {
            "relationship_id": view.relationship_id,
            "relation_type": view.relation_type,
            "promoted_at_ms": promoted_at_ms,
        }
    });
    with_invoker(|inv| {
        let node_payload = inv
            .bulk_nodes(json!({ "nodes": [assertion.clone()] }))
            .map_err(map_err)?;
        ensure_bulk_ok(&node_payload, "promoteEpistemicRelationship node")?;
        let edge_payload = inv
            .bulk_edges(json!({ "edges": [superseded_edge, source_edge, target_edge] }))
            .map_err(map_err)?;
        ensure_bulk_ok(&edge_payload, "promoteEpistemicRelationship edges")?;
        Ok(EpistemicRelationshipPromotionResult {
            assertion_id,
            assertion: Json(assertion),
            source_edge_id,
            target_edge_id,
            superseded_relationship: view,
        })
    })
}

fn promoted_relationship_edge_payload(
    hit: &Value,
    assertion_id: &str,
    promoted_at_ms: i64,
) -> GqlResult<Value> {
    let edge = hit
        .get("edge")
        .ok_or_else(|| async_graphql::Error::new("relationship hit missing edge"))?;
    let relationship_id = value_at_path(edge, &["id"])
        .and_then(Value::as_str)
        .ok_or_else(|| async_graphql::Error::new("relationship edge missing id"))?;
    let from_id = value_at_path(edge, &["from_id"])
        .or_else(|| value_at_path(edge, &["fromId"]))
        .and_then(Value::as_str)
        .ok_or_else(|| async_graphql::Error::new("relationship edge missing from_id"))?;
    let to_id = value_at_path(edge, &["to_id"])
        .or_else(|| value_at_path(edge, &["toId"]))
        .and_then(Value::as_str)
        .ok_or_else(|| async_graphql::Error::new("relationship edge missing to_id"))?;
    let edge_type = value_at_path(edge, &["type"])
        .and_then(Value::as_str)
        .ok_or_else(|| async_graphql::Error::new("relationship edge missing type"))?;
    let mut properties = value_at_path(edge, &["properties"])
        .cloned()
        .unwrap_or_else(|| json!({}));
    let props = ensure_json_object(&mut properties);
    props.insert("promoted_to".to_string(), json!(assertion_id));
    props.insert("canonical_assertion_id".to_string(), json!(assertion_id));
    props.insert("superseded_at_ms".to_string(), json!(promoted_at_ms));
    Ok(json!({
        "id": relationship_id,
        "from_id": from_id,
        "to_id": to_id,
        "type": edge_type,
        "confidence": value_at_path(edge, &["confidence"]).cloned().unwrap_or(Value::Null),
        "epistemic_type": value_at_path(edge, &["epistemic_type"]).cloned().unwrap_or(Value::Null),
        "properties": properties,
    }))
}

fn ensure_json_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("json object")
}

fn unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{
        EdgeRecord, EpistemicType, GraphStore, InMemoryGraphStore, NodeRecord, RedCoreDurability,
        RedCoreGraphStore, RedCoreOptions,
    };
    use serde_json::{json, Value};

    use crate::graphql::{execute_graphql, OpKind};
    use crate::SharedStore;

    fn run(
        store: SharedStore<InMemoryGraphStore>,
        query: &str,
        variables: Value,
        op: OpKind,
    ) -> Value {
        execute_graphql(
            "tenant-a",
            store,
            &json!({ "query": query, "variables": variables }),
            op,
            false,
        )
        .expect("graphql runs")
    }

    fn run_redcore(
        store: SharedStore<RedCoreGraphStore>,
        query: &str,
        variables: Value,
        op: OpKind,
    ) -> Value {
        execute_graphql(
            "tenant-a",
            store,
            &json!({ "query": query, "variables": variables }),
            op,
            false,
        )
        .expect("graphql runs")
    }

    fn upsert_doc(store: SharedStore<InMemoryGraphStore>, id: &str, vectors: Value) -> Value {
        run(
            store,
            "mutation($input: MultiVectorExactInput!){
                upsertMultiVector(input:$input){
                    manifest{
                        embeddingSetId
                        contentId
                        dim
                        vectorCount
                        exactObjectRef
                        binaryProjectionRef
                        exactToBinaryByteRatio
                    }
                    exactArtifactId
                    binaryProjectionId
                }
            }",
            json!({
                "input": {
                    "embeddingSetId": format!("mv:{id}"),
                    "contentId": id,
                    "modelId": "colpali-fixture",
                    "modelVersion": "test-v1",
                    "vectors": vectors,
                }
            }),
            OpKind::Mutate,
        )
    }

    fn upsert_doc_redcore(
        store: SharedStore<RedCoreGraphStore>,
        id: &str,
        vectors: Value,
    ) -> Value {
        run_redcore(
            store,
            "mutation($input: MultiVectorExactInput!){
                upsertMultiVector(input:$input){
                    manifest{
                        embeddingSetId
                        contentId
                        exactObjectRef
                        binaryProjectionRef
                    }
                    exactArtifactId
                    binaryProjectionId
                }
            }",
            json!({
                "input": {
                    "embeddingSetId": format!("mv:{id}"),
                    "contentId": id,
                    "modelId": "colpali-fixture",
                    "modelVersion": "test-v1",
                    "vectors": vectors,
                }
            }),
            OpKind::Mutate,
        )
    }

    fn put_content(store: &SharedStore<InMemoryGraphStore>, id: &str, properties: Value) {
        store.with_store(|inner| {
            GraphStore::upsert_node(inner, NodeRecord::new(id, ["Claim"], properties))
                .expect("content node");
        });
    }

    fn put_epistemic_edge(
        store: &SharedStore<InMemoryGraphStore>,
        id: &str,
        from_id: &str,
        edge_type: &str,
        to_id: &str,
        properties: Value,
        epistemic_type: EpistemicType,
        confidence: f64,
    ) {
        store.with_store(|inner| {
            GraphStore::upsert_edge(
                inner,
                EdgeRecord::new(id, from_id, edge_type, to_id, properties)
                    .with_epistemic_type(epistemic_type)
                    .with_confidence(confidence),
            )
            .expect("epistemic edge");
        });
    }

    fn temp_redcore_dir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("theorem-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn multivector_graphql_indexes_hot_binary_and_reranks_cold_exact() {
        let store = SharedStore::new(InMemoryGraphStore::new());

        let indexed_a = upsert_doc(store.clone(), "doc:a", json!([[1.0, 0.0], [0.0, 1.0]]));
        assert_eq!(indexed_a["errors"], Value::Null, "{indexed_a}");
        assert_eq!(
            indexed_a["data"]["upsertMultiVector"]["manifest"]["contentId"],
            "doc:a"
        );
        assert_eq!(
            indexed_a["data"]["upsertMultiVector"]["manifest"]["vectorCount"],
            2
        );
        assert!(
            indexed_a["data"]["upsertMultiVector"]["manifest"]["exactToBinaryByteRatio"]
                .as_f64()
                .unwrap()
                > 1.0
        );

        let indexed_b = upsert_doc(store.clone(), "doc:b", json!([[-1.0, 0.0], [0.0, -1.0]]));
        assert_eq!(indexed_b["errors"], Value::Null, "{indexed_b}");

        let exact_node = store.with_store(|inner| {
            InMemoryGraphStore::get_node(inner, "multivector:exact:mv:doc:a").cloned()
        });
        assert!(
            exact_node.is_some(),
            "exact vectors should live on a separate cold-artifact node"
        );
        assert_eq!(
            exact_node.unwrap().labels,
            vec!["ColdMultiVectorArtifact".to_string()]
        );
        let content_node =
            store.with_store(|inner| InMemoryGraphStore::get_node(inner, "doc:a").cloned());
        assert!(
            content_node.is_none(),
            "upsertMultiVector should not stuff exact vectors onto the content node"
        );

        let hot_only = run(
            store.clone(),
            "query($q:[[Float!]!]!){
                multiVectorSearch(
                    queryVectors:$q,
                    modelId:\"colpali-fixture\",
                    limit:2,
                    candidateLimit:4
                ){
                    contentId
                    embeddingSetId
                    scorer
                    vectorCount
                    score
                }
            }",
            json!({ "q": [[1.0, 0.0], [0.0, 1.0]] }),
            OpKind::Query,
        );
        assert_eq!(hot_only["errors"], Value::Null, "{hot_only}");
        let hot_hits = hot_only["data"]["multiVectorSearch"]
            .as_array()
            .expect("hot hits array");
        assert_eq!(hot_hits.len(), 2, "{hot_only}");
        assert_eq!(hot_hits[0]["contentId"], "doc:a", "{hot_only}");
        assert_eq!(hot_hits[0]["scorer"], "binary_hamming", "{hot_only}");

        let searched = run(
            store.clone(),
            "query($q:[[Float!]!]!){
                multiVectorSearch(
                    queryVectors:$q,
                    modelId:\"colpali-fixture\",
                    limit:2,
                    candidateLimit:4,
                    exactRerankLimit:2
                ){
                    contentId
                    embeddingSetId
                    scorer
                    vectorCount
                    score
                }
            }",
            json!({ "q": [[1.0, 0.0], [0.0, 1.0]] }),
            OpKind::Query,
        );
        assert_eq!(searched["errors"], Value::Null, "{searched}");
        let hits = searched["data"]["multiVectorSearch"]
            .as_array()
            .expect("hits array");
        assert_eq!(hits.len(), 2, "{searched}");
        assert_eq!(hits[0]["contentId"], "doc:a", "{searched}");
        assert_eq!(hits[0]["embeddingSetId"], "mv:doc:a", "{searched}");
        assert_eq!(hits[0]["scorer"], "exact_float", "{searched}");
        assert_eq!(hits[0]["vectorCount"], 2, "{searched}");
    }

    #[test]
    fn multivector_graph_aware_cascade_uses_epistemic_standing() {
        let store = SharedStore::new(InMemoryGraphStore::new());
        put_content(
            &store,
            "doc:trusted",
            json!({
                "acceptance_status": "supported",
                "epistemic_weight": 1.2,
                "source_reliability": 0.9,
            }),
        );
        put_content(
            &store,
            "doc:weak",
            json!({
                "acceptance_status": "undercut",
                "epistemic_weight": 0.8,
                "source_reliability": 0.2,
            }),
        );
        let indexed_trusted = upsert_doc(store.clone(), "doc:trusted", json!([[0.7, 0.3]]));
        assert_eq!(indexed_trusted["errors"], Value::Null, "{indexed_trusted}");
        let indexed_weak = upsert_doc(store.clone(), "doc:weak", json!([[1.0, 0.0]]));
        assert_eq!(indexed_weak["errors"], Value::Null, "{indexed_weak}");

        let searched = run(
            store,
            "query($q:[[Float!]!]!){
                multiVectorSearch(
                    queryVectors:$q,
                    modelId:\"colpali-fixture\",
                    limit:2,
                    exactRerankLimit:2,
                    rankRules:[EPISTEMIC_STATUS,VECTOR],
                    includeEpistemic:true
                ){
                    contentId
                    score
                    vectorScore
                    ranker
                    components
                }
            }",
            json!({ "q": [[1.0, 0.0]] }),
            OpKind::Query,
        );
        assert_eq!(searched["errors"], Value::Null, "{searched}");
        let hits = searched["data"]["multiVectorSearch"]
            .as_array()
            .expect("hits array");
        assert_eq!(hits[0]["contentId"], "doc:trusted", "{searched}");
        assert_eq!(hits[0]["ranker"], "graph_aware_cascade", "{searched}");
        assert!(
            hits[0]["vectorScore"].as_f64().unwrap() < hits[1]["vectorScore"].as_f64().unwrap(),
            "epistemic-first cascade should outrank the stronger raw vector hit: {searched}"
        );
        assert_eq!(
            hits[0]["components"]["acceptance_status"], "supported",
            "{searched}"
        );
    }

    #[test]
    fn epistemic_facet_supports_as_of_evidence_and_promotion() {
        let store = SharedStore::new(InMemoryGraphStore::new());
        put_content(&store, "claim:a", json!({"claim_text": "alpha"}));
        put_content(&store, "claim:b", json!({"claim_text": "beta"}));
        put_content(
            &store,
            "evidence:1",
            json!({"body": "source paragraph", "kind": "evidence"}),
        );
        put_epistemic_edge(
            &store,
            "edge:ab",
            "claim:a",
            "SUPPORTS",
            "claim:b",
            json!({
                "evidence_ref": "graph://evidence:1",
                "valid_from_ms": 1_000,
                "valid_to_ms": 2_000,
                "recorded_at_ms": 900,
            }),
            EpistemicType::Supports,
            0.7,
        );

        let active = run(
            store.clone(),
            "query{
                contentNode(id:\"claim:a\"){
                    epistemic{
                        standingView(asOfMs:1500){ source stale supportInDegree receipt{ asOfMs } }
                        relationshipViews(maxDepth:1, asOfMs:1500){ relationshipId evidenceRef }
                        evidence(evidenceRef:\"graph://evidence:1\"){ found source body }
                    }
                }
            }",
            json!({}),
            OpKind::Query,
        );
        assert_eq!(active["errors"], Value::Null, "{active}");
        let facet = &active["data"]["contentNode"]["epistemic"];
        assert_eq!(facet["standingView"]["source"], "degree_fallback_as_of");
        assert_eq!(facet["standingView"]["receipt"]["asOfMs"], json!(1500));
        assert_eq!(
            facet["relationshipViews"][0]["evidenceRef"],
            "graph://evidence:1"
        );
        assert_eq!(facet["evidence"]["found"], true);
        assert_eq!(facet["evidence"]["source"], "graph_node");

        let inactive = run(
            store.clone(),
            "query{
                contentNode(id:\"claim:a\"){
                    epistemic{
                        relationshipViews(maxDepth:1, asOfMs:2500){ relationshipId }
                    }
                }
            }",
            json!({}),
            OpKind::Query,
        );
        assert_eq!(inactive["errors"], Value::Null, "{inactive}");
        assert!(
            inactive["data"]["contentNode"]["epistemic"]["relationshipViews"]
                .as_array()
                .unwrap()
                .is_empty(),
            "{inactive}"
        );

        let promoted = run(
            store.clone(),
            "mutation{
                promoteEpistemicRelationship(input:{
                    relationshipId:\"edge:ab\",
                    sourceId:\"claim:a\",
                    targetId:\"claim:b\"
                }){
                    assertionId
                    sourceEdgeId
                    targetEdgeId
                    supersededRelationship{ relationshipId }
                }
            }",
            json!({}),
            OpKind::Mutate,
        );
        assert_eq!(promoted["errors"], Value::Null, "{promoted}");
        let assertion_id = promoted["data"]["promoteEpistemicRelationship"]["assertionId"]
            .as_str()
            .expect("assertion id")
            .to_string();
        store.with_store(|inner| {
            let assertion = GraphStore::get_node(inner, &assertion_id).expect("assertion node");
            assert!(assertion.labels.contains(&"EpistemicAssertion".to_string()));
            let edge = GraphStore::get_edge(inner, "edge:ab").expect("promoted edge");
            assert_eq!(
                edge.properties
                    .get("canonical_assertion_id")
                    .and_then(Value::as_str),
                Some(assertion_id.as_str())
            );
        });
    }

    #[test]
    fn multivector_redcore_stores_exact_vectors_in_cold_object_store() {
        let dir = temp_redcore_dir("multivector-redcore-cold");
        {
            let store = SharedStore::new(
                RedCoreGraphStore::open(
                    &dir,
                    RedCoreOptions {
                        durability: RedCoreDurability::None,
                        snapshot_interval_writes: 0,
                        strict_acid: false,
                    },
                )
                .expect("redcore opens"),
            );

            let indexed =
                upsert_doc_redcore(store.clone(), "doc:cold", json!([[1.0, 0.0], [0.0, 1.0]]));
            assert_eq!(indexed["errors"], Value::Null, "{indexed}");
            let exact_ref = indexed["data"]["upsertMultiVector"]["manifest"]["exactObjectRef"]
                .as_str()
                .expect("exact object ref");
            assert!(
                exact_ref.starts_with("cold://document/sha256:"),
                "redcore should store exact vectors in the cold object store: {indexed}"
            );

            let exact_node = store.with_store(|inner| {
                RedCoreGraphStore::get_node(inner, "multivector:exact:mv:doc:cold")
                    .expect("read exact node")
                    .expect("exact node")
            });
            let props = exact_node
                .properties
                .as_object()
                .expect("properties object");
            assert_eq!(
                props.get("exact_object_ref").and_then(Value::as_str),
                Some(exact_ref)
            );
            assert!(
                props.get("vectors").is_none(),
                "cold-backed exact artifact nodes must not keep the full vector array hot"
            );

            let searched = run_redcore(
                store.clone(),
                "query($q:[[Float!]!]!){
                    multiVectorSearch(
                        queryVectors:$q,
                        modelId:\"colpali-fixture\",
                        limit:1,
                        exactRerankLimit:1
                    ){
                        contentId
                        embeddingSetId
                        scorer
                    }
                }",
                json!({ "q": [[1.0, 0.0], [0.0, 1.0]] }),
                OpKind::Query,
            );
            assert_eq!(searched["errors"], Value::Null, "{searched}");
            assert_eq!(
                searched["data"]["multiVectorSearch"][0]["contentId"], "doc:cold",
                "{searched}"
            );
            assert_eq!(
                searched["data"]["multiVectorSearch"][0]["scorer"], "exact_float",
                "{searched}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
