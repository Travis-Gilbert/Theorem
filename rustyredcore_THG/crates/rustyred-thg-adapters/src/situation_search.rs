//! Similar-situation retrieval over training and harness graph records.
//!
//! Reasoning traces, postmortems, user-model records, training artifacts, and
//! code records stay as graph records. This module registers vector views over
//! those records and returns the policy decision that tells the caller whether
//! local memory is strong enough or RustyWeb/code search should be consulted.

use std::collections::HashMap;

use rustyred_thg_core::{
    now_ms, stable_hash, GraphMutation, GraphMutationBatch, GraphStoreResult, GraphTransaction,
    InMemoryGraphStore, NodeRecord, RedCoreGraphStore, ThgError, ThgResult, VectorDesignation,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::training_substrate::{
    ARTIFACT_LABEL, GNN_EXPORT_LABEL, OBJECT_LABEL, PARAPHRASE_PAIR_LABEL, POSTMORTEM_LABEL,
    REASONING_TRACE_LABEL, TRACE_STEP_LABEL, TRAINING_PACK_LABEL,
};
use crate::types::{
    edge_with_adapter_provenance, normalize_tenant_id, tenant_node_id, thg_error_from_store,
    AdapterGraphStore, THG_ADAPTER_SOURCE,
};

pub const HARNESS_RUN_LABEL: &str = "HarnessRun";
pub const HARNESS_EVENT_LABEL: &str = "HarnessEvent";
pub const USER_MODEL_LABEL: &str = "UserModel";
pub const USER_PREFERENCE_LABEL: &str = "UserPreference";
pub const CODE_OBJECT_LABEL: &str = "CodeObject";
pub const CODE_SYMBOL_LABEL: &str = "CodeSymbol";
pub const CODE_FILE_LABEL: &str = "CodeFile";
pub const SIMILAR_SITUATION_SEARCH_LABEL: &str = "SimilarSituationSearch";
pub const SEARCH_ESCALATION_PLAN_LABEL: &str = "SearchEscalationPlan";

pub const EMBEDDING_SITUATION_SBERT_384: &str = "embedding_situation_sbert_384";
pub const EMBEDDING_TRAINING_SBERT_384: &str = "embedding_training_sbert_384";
pub const EMBEDDING_USER_SBERT_384: &str = "embedding_user_sbert_384";
pub const EMBEDDING_CODE_UNIXCODER_768: &str = "embedding_code_unixcoder_768";
pub const EMBEDDING_CODEGRAPHBERT_768: &str = "embedding_codegraphbert_768";

pub const MATCHED_SIMILAR_SITUATION: &str = "MATCHED_SIMILAR_SITUATION";
pub const ESCALATED_TO_SEARCH: &str = "ESCALATED_TO_SEARCH";

pub trait SituationSearchGraphStore: AdapterGraphStore {
    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()>;

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>>;
}

impl SituationSearchGraphStore for InMemoryGraphStore {
    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        InMemoryGraphStore::designate_vector_property(self, label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        InMemoryGraphStore::vector_search(self, label, property_name, query, k)
    }
}

impl SituationSearchGraphStore for RedCoreGraphStore {
    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        RedCoreGraphStore::designate_vector_property(self, label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        RedCoreGraphStore::vector_search(self, label, property_name, query, k)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SimilarSituationSearchMode {
    RecallOnly,
    RepoAssisted,
    WebAssisted,
    #[default]
    Auto,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationSearchPolicy {
    pub min_local_similarity_without_external: f32,
    pub min_local_hits_without_external: usize,
    pub graph_maturity_nodes_for_local_only: usize,
    pub always_search_web_until_graph_matures: bool,
}

impl Default for SimilarSituationSearchPolicy {
    fn default() -> Self {
        Self {
            min_local_similarity_without_external: 0.90,
            min_local_hits_without_external: 3,
            graph_maturity_nodes_for_local_only: 250_000,
            always_search_web_until_graph_matures: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationSearchRequest {
    pub tenant_id: String,
    pub query_text: Option<String>,
    pub query_embedding: Vec<f32>,
    pub embedding_property: String,
    pub target_labels: Vec<String>,
    pub top_k: usize,
    pub mode: SimilarSituationSearchMode,
}

impl SimilarSituationSearchRequest {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = normalize_tenant_id(&self.tenant_id);
        self.query_text = self
            .query_text
            .map(|text| text.trim().to_string())
            .filter(|text| !text.is_empty());
        self.embedding_property = self.embedding_property.trim().to_string();
        if self.embedding_property.is_empty() {
            self.embedding_property = EMBEDDING_SITUATION_SBERT_384.to_string();
        }
        self.target_labels = self
            .target_labels
            .into_iter()
            .map(|label| label.trim().to_string())
            .filter(|label| !label.is_empty())
            .collect();
        if self.target_labels.is_empty() {
            self.target_labels = default_situation_target_labels();
        }
        self.top_k = self.top_k.max(1);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationHit {
    pub node_id: String,
    pub label: String,
    pub distance: f32,
    pub similarity: f32,
    pub rank: usize,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationDecision {
    pub local_memory_sufficient: bool,
    pub should_search_codebase: bool,
    pub should_search_open_web: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationSearchResult {
    pub search_id: String,
    pub tenant_id: String,
    pub mode: SimilarSituationSearchMode,
    pub embedding_property: String,
    pub query_vector_hash: String,
    pub graph_nodes_total: usize,
    pub hits: Vec<SimilarSituationHit>,
    pub best_similarity: Option<f32>,
    pub decision: SimilarSituationDecision,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct SimilarSituationSearchReceipt {
    pub search_node_id: String,
    pub escalation_plan_node_ids: Vec<String>,
    pub transaction: GraphTransaction,
}

pub fn default_situation_target_labels() -> Vec<String> {
    [
        POSTMORTEM_LABEL,
        REASONING_TRACE_LABEL,
        TRACE_STEP_LABEL,
        HARNESS_RUN_LABEL,
        HARNESS_EVENT_LABEL,
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

pub fn semantic_vector_designations() -> Vec<VectorDesignation> {
    [
        (POSTMORTEM_LABEL, EMBEDDING_SITUATION_SBERT_384, 384usize),
        (REASONING_TRACE_LABEL, EMBEDDING_SITUATION_SBERT_384, 384),
        (TRACE_STEP_LABEL, EMBEDDING_SITUATION_SBERT_384, 384),
        (HARNESS_RUN_LABEL, EMBEDDING_SITUATION_SBERT_384, 384),
        (HARNESS_EVENT_LABEL, EMBEDDING_SITUATION_SBERT_384, 384),
        (TRAINING_PACK_LABEL, EMBEDDING_TRAINING_SBERT_384, 384),
        (ARTIFACT_LABEL, EMBEDDING_TRAINING_SBERT_384, 384),
        (GNN_EXPORT_LABEL, EMBEDDING_TRAINING_SBERT_384, 384),
        (PARAPHRASE_PAIR_LABEL, EMBEDDING_TRAINING_SBERT_384, 384),
        (OBJECT_LABEL, EMBEDDING_TRAINING_SBERT_384, 384),
        (USER_MODEL_LABEL, EMBEDDING_USER_SBERT_384, 384),
        (USER_PREFERENCE_LABEL, EMBEDDING_USER_SBERT_384, 384),
        (CODE_OBJECT_LABEL, EMBEDDING_CODE_UNIXCODER_768, 768),
        (CODE_SYMBOL_LABEL, EMBEDDING_CODE_UNIXCODER_768, 768),
        (CODE_FILE_LABEL, EMBEDDING_CODE_UNIXCODER_768, 768),
        (CODE_OBJECT_LABEL, EMBEDDING_CODEGRAPHBERT_768, 768),
        (CODE_SYMBOL_LABEL, EMBEDDING_CODEGRAPHBERT_768, 768),
        (CODE_FILE_LABEL, EMBEDDING_CODEGRAPHBERT_768, 768),
    ]
    .into_iter()
    .map(|(label, property, dimension)| VectorDesignation {
        label: label.to_string(),
        property: property.to_string(),
        dimension,
    })
    .collect()
}

pub fn register_semantic_vector_designations<S: SituationSearchGraphStore>(
    store: &mut S,
) -> ThgResult<Vec<VectorDesignation>> {
    let designations = semantic_vector_designations();
    for designation in &designations {
        store
            .designate_vector_property(
                &designation.label,
                &designation.property,
                designation.dimension,
            )
            .map_err(thg_error_from_store)?;
    }
    Ok(designations)
}

pub fn similar_situation_search<S: SituationSearchGraphStore>(
    store: &S,
    request: SimilarSituationSearchRequest,
    policy: SimilarSituationSearchPolicy,
) -> ThgResult<SimilarSituationSearchResult> {
    let request = request.normalized();
    if request.query_embedding.is_empty() {
        return Err(ThgError::new(
            "invalid_similar_situation_search",
            "query_embedding is required",
        ));
    }

    let mut hits_by_id: HashMap<String, SimilarSituationHit> = HashMap::new();
    for label in &request.target_labels {
        let vector_hits = store
            .vector_search(
                Some(label.as_str()),
                &request.embedding_property,
                &request.query_embedding,
                request.top_k,
            )
            .map_err(thg_error_from_store)?;
        for (node_id, distance) in vector_hits {
            let Some(node) = store.get_node(&node_id).map_err(thg_error_from_store)? else {
                continue;
            };
            if property_str(&node.properties, "tenant_id") != Some(request.tenant_id.as_str()) {
                continue;
            }
            let similarity = similarity_from_distance(distance);
            let hit = SimilarSituationHit {
                node_id: node_id.clone(),
                label: label.clone(),
                distance,
                similarity,
                rank: 0,
                summary: situation_summary(&node),
            };
            match hits_by_id.get(&node_id) {
                Some(existing) if existing.similarity >= similarity => {}
                _ => {
                    hits_by_id.insert(node_id, hit);
                }
            }
        }
    }

    let mut hits = hits_by_id.into_values().collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        b.similarity
            .partial_cmp(&a.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                a.distance
                    .partial_cmp(&b.distance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| a.node_id.cmp(&b.node_id))
    });
    hits.truncate(request.top_k);
    for (idx, hit) in hits.iter_mut().enumerate() {
        hit.rank = idx + 1;
    }

    let best_similarity = hits.first().map(|hit| hit.similarity);
    let graph_nodes_total = store.stats().map_err(thg_error_from_store)?.nodes_total;
    let decision = decide_search_escalation(
        &request.mode,
        &policy,
        hits.len(),
        best_similarity,
        graph_nodes_total,
    );
    let query_vector_hash = stable_hash(json!({
        "embedding_property": request.embedding_property,
        "query_embedding": request.query_embedding,
    }));
    let search_id = similar_situation_search_id(
        &request.tenant_id,
        request.query_text.as_deref(),
        &request.embedding_property,
        &query_vector_hash,
    );

    Ok(SimilarSituationSearchResult {
        search_id,
        tenant_id: request.tenant_id,
        mode: request.mode,
        embedding_property: request.embedding_property,
        query_vector_hash,
        graph_nodes_total,
        hits,
        best_similarity,
        decision,
    })
}

pub fn record_similar_situation_search<S: SituationSearchGraphStore>(
    store: &mut S,
    result: &SimilarSituationSearchResult,
    query_text: Option<&str>,
    actor: Option<&str>,
) -> ThgResult<SimilarSituationSearchReceipt> {
    let search_node_id = result.search_id.clone();
    let mut mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            tenant_node_id(&result.tenant_id),
            ["Tenant"],
            json!({
                "tenant_id": result.tenant_id,
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
        GraphMutation::NodeUpsert(NodeRecord::new(
            &search_node_id,
            [SIMILAR_SITUATION_SEARCH_LABEL],
            json!({
                "tenant_id": result.tenant_id,
                "search_id": result.search_id,
                "query_text": query_text.map(str::trim).filter(|text| !text.is_empty()),
                "query_vector_hash": result.query_vector_hash,
                "embedding_property": result.embedding_property,
                "mode": result.mode,
                "graph_nodes_total": result.graph_nodes_total,
                "local_hit_count": result.hits.len(),
                "best_similarity": result.best_similarity,
                "local_memory_sufficient": result.decision.local_memory_sufficient,
                "should_search_codebase": result.decision.should_search_codebase,
                "should_search_open_web": result.decision.should_search_open_web,
                "decision_reasons": result.decision.reasons,
                "recorded_at_ms": now_ms(),
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
    ];

    for hit in &result.hits {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            format!(
                "edge:{}:{}:{}",
                search_node_id, MATCHED_SIMILAR_SITUATION, hit.node_id
            ),
            &search_node_id,
            MATCHED_SIMILAR_SITUATION,
            &hit.node_id,
            json!({
                "tenant_id": result.tenant_id,
                "rank": hit.rank,
                "label": hit.label,
                "distance": hit.distance,
                "similarity": hit.similarity,
            }),
            actor,
        )));
    }

    let mut escalation_plan_node_ids = Vec::new();
    if result.decision.should_search_codebase {
        push_escalation_plan(
            &mut mutations,
            &mut escalation_plan_node_ids,
            result,
            "codebase",
            "code_search",
            actor,
        );
    }
    if result.decision.should_search_open_web {
        push_escalation_plan(
            &mut mutations,
            &mut escalation_plan_node_ids,
            result,
            "open_web",
            "rustyweb_open_web",
            actor,
        );
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(SimilarSituationSearchReceipt {
        search_node_id,
        escalation_plan_node_ids,
        transaction,
    })
}

fn decide_search_escalation(
    mode: &SimilarSituationSearchMode,
    policy: &SimilarSituationSearchPolicy,
    local_hits: usize,
    best_similarity: Option<f32>,
    graph_nodes_total: usize,
) -> SimilarSituationDecision {
    let mut reasons = Vec::new();
    let best = best_similarity.unwrap_or(0.0);
    let local_signal_strong = local_hits >= policy.min_local_hits_without_external
        && best >= policy.min_local_similarity_without_external;
    let graph_mature = graph_nodes_total >= policy.graph_maturity_nodes_for_local_only;
    if local_hits < policy.min_local_hits_without_external {
        reasons.push("few_local_hits".to_string());
    }
    if best < policy.min_local_similarity_without_external {
        reasons.push("low_local_similarity".to_string());
    }
    if !graph_mature {
        reasons.push("graph_below_maturity_threshold".to_string());
    }

    match mode {
        SimilarSituationSearchMode::RecallOnly => SimilarSituationDecision {
            local_memory_sufficient: local_signal_strong,
            should_search_codebase: false,
            should_search_open_web: false,
            reasons: vec!["mode_recall_only".to_string()],
        },
        SimilarSituationSearchMode::RepoAssisted => SimilarSituationDecision {
            local_memory_sufficient: local_signal_strong && graph_mature,
            should_search_codebase: !local_signal_strong,
            should_search_open_web: false,
            reasons: if reasons.is_empty() {
                vec!["mode_repo_assisted".to_string()]
            } else {
                reasons
            },
        },
        SimilarSituationSearchMode::WebAssisted => SimilarSituationDecision {
            local_memory_sufficient: false,
            should_search_codebase: !local_signal_strong,
            should_search_open_web: true,
            reasons: vec!["mode_web_assisted".to_string()],
        },
        SimilarSituationSearchMode::Auto => {
            let should_search_open_web = !local_signal_strong
                || (policy.always_search_web_until_graph_matures && !graph_mature);
            SimilarSituationDecision {
                local_memory_sufficient: local_signal_strong && !should_search_open_web,
                should_search_codebase: !local_signal_strong,
                should_search_open_web,
                reasons: if reasons.is_empty() {
                    vec!["local_memory_sufficient".to_string()]
                } else {
                    reasons
                },
            }
        }
    }
}

fn push_escalation_plan(
    mutations: &mut Vec<GraphMutation>,
    escalation_plan_node_ids: &mut Vec<String>,
    result: &SimilarSituationSearchResult,
    channel: &str,
    tool_hint: &str,
    actor: Option<&str>,
) {
    let plan_node_id =
        search_escalation_plan_node_id(&result.tenant_id, &result.search_id, channel);
    mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
        &plan_node_id,
        [SEARCH_ESCALATION_PLAN_LABEL],
        json!({
            "tenant_id": result.tenant_id,
            "search_id": result.search_id,
            "channel": channel,
            "tool_hint": tool_hint,
            "decision_reasons": result.decision.reasons,
            "source": THG_ADAPTER_SOURCE,
        }),
    )));
    mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
        format!(
            "edge:{}:{}:{}",
            result.search_id, ESCALATED_TO_SEARCH, plan_node_id
        ),
        &result.search_id,
        ESCALATED_TO_SEARCH,
        &plan_node_id,
        json!({
            "tenant_id": result.tenant_id,
            "channel": channel,
            "tool_hint": tool_hint,
        }),
        actor,
    )));
    escalation_plan_node_ids.push(plan_node_id);
}

fn similar_situation_search_id(
    tenant_id: &str,
    query_text: Option<&str>,
    embedding_property: &str,
    query_vector_hash: &str,
) -> String {
    let digest = stable_hash(json!({
        "tenant_id": tenant_id,
        "query_text": query_text,
        "embedding_property": embedding_property,
        "query_vector_hash": query_vector_hash,
    }));
    format!(
        "similar_situation_search:{tenant_id}:{}",
        slug_segment(&digest)
    )
}

fn search_escalation_plan_node_id(tenant_id: &str, search_id: &str, channel: &str) -> String {
    format!(
        "search_escalation_plan:{}:{}:{}",
        normalize_tenant_id(tenant_id),
        slug_segment(search_id),
        slug_segment(channel)
    )
}

fn situation_summary(node: &NodeRecord) -> Option<String> {
    [
        "summary",
        "title",
        "failure_mode",
        "repair_pattern",
        "task_family",
        "event_type",
        "type",
    ]
    .into_iter()
    .find_map(|key| property_str(&node.properties, key).map(str::to_string))
}

fn property_str<'a>(properties: &'a Value, key: &str) -> Option<&'a str> {
    properties.get(key).and_then(Value::as_str)
}

fn similarity_from_distance(distance: f32) -> f32 {
    (1.0 - distance).clamp(0.0, 1.0)
}

fn slug_segment(value: &str) -> String {
    let mut slug = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug.trim_matches('-').to_string()
}
