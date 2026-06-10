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
pub const CONTEXT_PACK_LABEL: &str = "ContextPack";
pub const CONTEXT_ATOM_LABEL: &str = "ContextAtom";

pub const EMBEDDING_SITUATION_SBERT_384: &str = "embedding_situation_sbert_384";
pub const EMBEDDING_TRAINING_SBERT_384: &str = "embedding_training_sbert_384";
pub const EMBEDDING_USER_SBERT_384: &str = "embedding_user_sbert_384";
pub const EMBEDDING_CODE_UNIXCODER_768: &str = "embedding_code_unixcoder_768";
pub const EMBEDDING_CODEGRAPHBERT_768: &str = "embedding_codegraphbert_768";

pub const MATCHED_SIMILAR_SITUATION: &str = "MATCHED_SIMILAR_SITUATION";
pub const ESCALATED_TO_SEARCH: &str = "ESCALATED_TO_SEARCH";
pub const CONTEXT_ATOM_SELECTED: &str = "CONTEXT_ATOM_SELECTED";
pub const CONTEXT_USE_RECEIPT_LABEL: &str = "ContextUseReceipt";
pub const CONTEXT_PACK_OUTCOME: &str = "CONTEXT_PACK_OUTCOME";

pub const DEFAULT_CONTEXT_TOKEN_BUDGET: usize = 4_096;
pub const DEFAULT_CONTEXT_MAX_ATOMS: usize = 32;
pub const DEFAULT_CONTEXT_TOKEN_COST: usize = 128;

const CONTEXT_RECENCY_HALF_LIFE_MS: f32 = 7.0 * 24.0 * 60.0 * 60.0 * 1_000.0;

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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextAtomCandidate {
    pub node_id: String,
    pub label: String,
    pub summary: Option<String>,
    pub similarity: f32,
    pub token_cost: usize,
    pub age_ms: Option<u64>,
    pub use_count: u32,
    pub success_count: u32,
    pub failure_count: u32,
    pub pinned: bool,
    pub required: bool,
    pub graph_degree: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextScoringPolicy {
    pub token_budget: usize,
    pub max_atoms: usize,
    pub min_score: f32,
    pub similarity_weight: f32,
    pub receipt_weight: f32,
    pub recency_weight: f32,
    pub graph_weight: f32,
    pub pin_bonus: f32,
    pub required_bonus: f32,
    pub failure_penalty: f32,
}

impl Default for ContextScoringPolicy {
    fn default() -> Self {
        Self {
            token_budget: DEFAULT_CONTEXT_TOKEN_BUDGET,
            max_atoms: DEFAULT_CONTEXT_MAX_ATOMS,
            min_score: 0.20,
            similarity_weight: 0.58,
            receipt_weight: 0.20,
            recency_weight: 0.12,
            graph_weight: 0.10,
            pin_bonus: 0.15,
            required_bonus: 0.35,
            failure_penalty: 0.25,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RankedContextAtom {
    pub node_id: String,
    pub label: String,
    pub summary: Option<String>,
    pub score: f32,
    pub token_cost: usize,
    pub rank: usize,
    pub selected: bool,
    pub reasons: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextScoringResult {
    pub context_pack_id: String,
    pub tenant_id: String,
    pub token_budget: usize,
    pub used_tokens: usize,
    pub ranked_atoms: Vec<RankedContextAtom>,
    pub selected_node_ids: Vec<String>,
    pub bounded: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ContextScoringReceipt {
    pub context_pack_node_id: String,
    pub selected_edge_ids: Vec<String>,
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

pub fn context_candidates_from_similar_situation(
    result: &SimilarSituationSearchResult,
    default_token_cost: usize,
) -> Vec<ContextAtomCandidate> {
    let token_cost = default_token_cost.max(1);
    result
        .hits
        .iter()
        .map(|hit| ContextAtomCandidate {
            node_id: hit.node_id.clone(),
            label: hit.label.clone(),
            summary: hit.summary.clone(),
            similarity: hit.similarity,
            token_cost,
            age_ms: None,
            use_count: 0,
            success_count: 0,
            failure_count: 0,
            pinned: false,
            required: false,
            graph_degree: 0,
        })
        .collect()
}

pub fn score_context_atoms(
    tenant_id: &str,
    candidates: Vec<ContextAtomCandidate>,
    policy: ContextScoringPolicy,
) -> ThgResult<ContextScoringResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let policy = normalize_context_scoring_policy(policy);
    let mut candidates_by_id = HashMap::new();
    for candidate in candidates {
        let candidate = normalize_context_atom_candidate(candidate);
        if candidate.node_id.is_empty() {
            continue;
        }
        candidates_by_id
            .entry(candidate.node_id.clone())
            .and_modify(|existing| merge_context_atom_candidate(existing, &candidate))
            .or_insert(candidate);
    }

    let mut ranked_atoms = candidates_by_id
        .into_values()
        .map(|candidate| score_context_atom_candidate(&candidate, &policy))
        .collect::<Vec<_>>();
    ranked_atoms.sort_by(compare_ranked_context_atoms);

    let mut used_tokens = 0usize;
    let mut selected_node_ids = Vec::new();
    for (idx, atom) in ranked_atoms.iter_mut().enumerate() {
        atom.rank = idx + 1;
        let required = atom.reasons.iter().any(|reason| reason == "required");
        let score_ok = atom.score >= policy.min_score || required;
        let max_atoms_ok = selected_node_ids.len() < policy.max_atoms;
        let budget_ok = used_tokens.saturating_add(atom.token_cost) <= policy.token_budget;

        if score_ok && max_atoms_ok && budget_ok {
            atom.selected = true;
            used_tokens += atom.token_cost;
            selected_node_ids.push(atom.node_id.clone());
            atom.reasons.push("selected_under_budget".to_string());
        } else {
            atom.selected = false;
            if !score_ok {
                atom.reasons.push("below_min_score".to_string());
            }
            if !max_atoms_ok {
                atom.reasons.push("max_atoms_reached".to_string());
            }
            if !budget_ok {
                if required {
                    atom.reasons.push("required_atom_over_budget".to_string());
                } else {
                    atom.reasons.push("token_budget_exceeded".to_string());
                }
            }
        }
    }

    let bounded = ranked_atoms.iter().any(|atom| !atom.selected);
    let context_pack_id = context_pack_node_id(&tenant_id, &selected_node_ids, used_tokens);
    Ok(ContextScoringResult {
        context_pack_id,
        tenant_id,
        token_budget: policy.token_budget,
        used_tokens,
        ranked_atoms,
        selected_node_ids,
        bounded,
    })
}

pub fn record_context_scoring_result<S: AdapterGraphStore>(
    store: &mut S,
    result: &ContextScoringResult,
    actor: Option<&str>,
) -> ThgResult<ContextScoringReceipt> {
    let context_pack_node_id = result.context_pack_id.clone();
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
            &context_pack_node_id,
            [CONTEXT_PACK_LABEL],
            json!({
                "tenant_id": result.tenant_id,
                "context_pack_id": result.context_pack_id,
                "token_budget": result.token_budget,
                "used_tokens": result.used_tokens,
                "selected_node_ids": result.selected_node_ids,
                "selected_count": result.selected_node_ids.len(),
                "ranked_count": result.ranked_atoms.len(),
                "bounded": result.bounded,
                "recorded_at_ms": now_ms(),
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
    ];

    let mut selected_edge_ids = Vec::new();
    for atom in result.ranked_atoms.iter().filter(|atom| atom.selected) {
        let edge_id = format!(
            "edge:{}:{}:{}",
            context_pack_node_id, CONTEXT_ATOM_SELECTED, atom.node_id
        );
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id.clone(),
            &context_pack_node_id,
            CONTEXT_ATOM_SELECTED,
            &atom.node_id,
            json!({
                "tenant_id": result.tenant_id,
                "rank": atom.rank,
                "label": atom.label,
                "score": atom.score,
                "token_cost": atom.token_cost,
                "reasons": atom.reasons,
            }),
            actor,
        )));
        selected_edge_ids.push(edge_id);
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(ContextScoringReceipt {
        context_pack_node_id,
        selected_edge_ids,
        transaction,
    })
}

/// Record the outcome of using a context pack: the use-receipt that the
/// plan names as the label source for the memory scorer. One receipt node
/// per (pack, outcome occurrence), linked from the pack.
pub fn record_context_use_outcome<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    context_pack_node_id: &str,
    outcome_id: &str,
    success: bool,
    note: Option<&str>,
    actor: Option<&str>,
) -> ThgResult<String> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let context_pack_node_id = context_pack_node_id.trim();
    let outcome_id = outcome_id.trim();
    if context_pack_node_id.is_empty() || outcome_id.is_empty() {
        return Err(ThgError::new(
            "invalid_context_use_outcome",
            "context_pack_node_id and outcome_id are required",
        ));
    }
    if store
        .get_node(context_pack_node_id)
        .map_err(thg_error_from_store)?
        .is_none()
    {
        return Err(ThgError::new(
            "missing_graph_endpoint",
            format!("context pack {context_pack_node_id} does not exist"),
        ));
    }
    let receipt_node_id = format!(
        "context_use_receipt:{}:{}:{}",
        tenant_id,
        slug_segment(context_pack_node_id),
        slug_segment(outcome_id)
    );
    let mutations = vec![
        GraphMutation::NodeUpsert(NodeRecord::new(
            &receipt_node_id,
            [CONTEXT_USE_RECEIPT_LABEL],
            json!({
                "tenant_id": tenant_id,
                "context_pack_node_id": context_pack_node_id,
                "outcome_id": outcome_id,
                "success": success,
                "note": note.map(str::trim).filter(|text| !text.is_empty()),
                "recorded_at_ms": now_ms(),
                "source": THG_ADAPTER_SOURCE,
            }),
        )),
        GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            format!(
                "edge:{}:{}:{}",
                context_pack_node_id, CONTEXT_PACK_OUTCOME, receipt_node_id
            ),
            context_pack_node_id,
            CONTEXT_PACK_OUTCOME,
            &receipt_node_id,
            json!({
                "tenant_id": tenant_id,
                "success": success,
            }),
            actor,
        )),
    ];
    store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(receipt_node_id)
}

/// Close the read-back loop: feed prior selection edges and use-receipts
/// back into candidate features before scoring, so the scorer's
/// receipt/recency/degree terms run on graph truth instead of zero-fill.
/// Read-only and bounded by the candidate list it is given.
pub fn enrich_context_candidates_from_store<S: crate::reflexive_executor::ReflexiveReadStore>(
    store: &S,
    tenant_id: &str,
    candidates: Vec<ContextAtomCandidate>,
) -> ThgResult<Vec<ContextAtomCandidate>> {
    use rustyred_thg_core::{Direction, NeighborQuery};

    let tenant_id = normalize_tenant_id(tenant_id);
    let now = u64::try_from(now_ms()).unwrap_or(0);
    let mut enriched = Vec::with_capacity(candidates.len());
    for mut candidate in candidates {
        let node_id = candidate.node_id.trim().to_string();
        if node_id.is_empty() {
            enriched.push(candidate);
            continue;
        }

        // Graph degree: out plus in neighbors of the atom itself.
        let out_degree = store
            .read_neighbors(NeighborQuery {
                node_id: node_id.clone(),
                direction: Direction::Out,
                edge_type: None,
                include_expired: false,
            })
            .map_err(thg_error_from_store)?
            .len();
        let in_hits = store
            .read_neighbors(NeighborQuery {
                node_id: node_id.clone(),
                direction: Direction::In,
                edge_type: None,
                include_expired: false,
            })
            .map_err(thg_error_from_store)?;
        candidate.graph_degree = candidate.graph_degree.max(out_degree + in_hits.len());

        // Selection history: packs that selected this atom, and the
        // outcomes recorded against those packs.
        let mut use_count = 0u32;
        let mut success_count = 0u32;
        let mut failure_count = 0u32;
        for hit in in_hits
            .iter()
            .filter(|hit| hit.edge_type == CONTEXT_ATOM_SELECTED)
        {
            let Some(pack) = store.read_node(&hit.node_id).map_err(thg_error_from_store)? else {
                continue;
            };
            if property_str(&pack.properties, "tenant_id") != Some(tenant_id.as_str()) {
                continue;
            }
            use_count = use_count.saturating_add(1);
            let outcomes = store
                .read_neighbors(NeighborQuery {
                    node_id: pack.id.clone(),
                    direction: Direction::Out,
                    edge_type: Some(CONTEXT_PACK_OUTCOME.to_string()),
                    include_expired: false,
                })
                .map_err(thg_error_from_store)?;
            for outcome_hit in outcomes {
                let Some(receipt) = store
                    .read_node(&outcome_hit.node_id)
                    .map_err(thg_error_from_store)?
                else {
                    continue;
                };
                match receipt.properties.get("success").and_then(Value::as_bool) {
                    Some(true) => success_count = success_count.saturating_add(1),
                    Some(false) => failure_count = failure_count.saturating_add(1),
                    None => {}
                }
            }
        }
        candidate.use_count = candidate.use_count.saturating_add(use_count);
        candidate.success_count = candidate.success_count.saturating_add(success_count);
        candidate.failure_count = candidate.failure_count.saturating_add(failure_count);

        // Age and pin state from the atom node itself.
        if let Some(atom) = store.read_node(&node_id).map_err(thg_error_from_store)? {
            if candidate.age_ms.is_none() {
                let recorded = atom
                    .properties
                    .get("recorded_at_ms")
                    .or_else(|| atom.properties.get("created_at_ms"))
                    .and_then(Value::as_u64);
                if let Some(recorded_at) = recorded {
                    candidate.age_ms = Some(now.saturating_sub(recorded_at));
                }
            }
            if let Some(pinned) = atom.properties.get("pinned").and_then(Value::as_bool) {
                candidate.pinned |= pinned;
            }
        }
        enriched.push(candidate);
    }
    Ok(enriched)
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

fn context_pack_node_id(
    tenant_id: &str,
    selected_node_ids: &[String],
    used_tokens: usize,
) -> String {
    let digest = stable_hash(json!({
        "tenant_id": tenant_id,
        "selected_node_ids": selected_node_ids,
        "used_tokens": used_tokens,
    }));
    format!(
        "context_pack:{}:{}",
        normalize_tenant_id(tenant_id),
        slug_segment(&digest)
    )
}

fn normalize_context_scoring_policy(policy: ContextScoringPolicy) -> ContextScoringPolicy {
    let defaults = ContextScoringPolicy::default();
    ContextScoringPolicy {
        token_budget: policy.token_budget,
        max_atoms: policy.max_atoms,
        min_score: finite_or(policy.min_score, defaults.min_score).clamp(0.0, 1.0),
        similarity_weight: finite_or(policy.similarity_weight, defaults.similarity_weight),
        receipt_weight: finite_or(policy.receipt_weight, defaults.receipt_weight),
        recency_weight: finite_or(policy.recency_weight, defaults.recency_weight),
        graph_weight: finite_or(policy.graph_weight, defaults.graph_weight),
        pin_bonus: finite_or(policy.pin_bonus, defaults.pin_bonus),
        required_bonus: finite_or(policy.required_bonus, defaults.required_bonus),
        failure_penalty: finite_or(policy.failure_penalty, defaults.failure_penalty),
    }
}

fn normalize_context_atom_candidate(mut candidate: ContextAtomCandidate) -> ContextAtomCandidate {
    candidate.node_id = candidate.node_id.trim().to_string();
    candidate.label = candidate.label.trim().to_string();
    if candidate.label.is_empty() {
        candidate.label = CONTEXT_ATOM_LABEL.to_string();
    }
    candidate.summary = candidate
        .summary
        .map(|summary| summary.trim().to_string())
        .filter(|summary| !summary.is_empty());
    candidate.similarity = finite_or(candidate.similarity, 0.0).clamp(0.0, 1.0);
    candidate.token_cost = candidate.token_cost.max(1);
    candidate
}

fn merge_context_atom_candidate(
    existing: &mut ContextAtomCandidate,
    incoming: &ContextAtomCandidate,
) {
    if existing.summary.is_none() {
        existing.summary = incoming.summary.clone();
    }
    existing.similarity = existing.similarity.max(incoming.similarity);
    existing.token_cost = existing.token_cost.min(incoming.token_cost);
    existing.age_ms = match (existing.age_ms, incoming.age_ms) {
        (Some(existing_age), Some(incoming_age)) => Some(existing_age.min(incoming_age)),
        (None, Some(incoming_age)) => Some(incoming_age),
        (existing_age, None) => existing_age,
    };
    existing.use_count = existing.use_count.saturating_add(incoming.use_count);
    existing.success_count = existing
        .success_count
        .saturating_add(incoming.success_count);
    existing.failure_count = existing
        .failure_count
        .saturating_add(incoming.failure_count);
    existing.pinned |= incoming.pinned;
    existing.required |= incoming.required;
    existing.graph_degree = existing.graph_degree.max(incoming.graph_degree);
}

fn score_context_atom_candidate(
    candidate: &ContextAtomCandidate,
    policy: &ContextScoringPolicy,
) -> RankedContextAtom {
    let receipt_total = candidate
        .success_count
        .saturating_add(candidate.failure_count);
    let receipt_score = if receipt_total == 0 {
        0.5
    } else {
        candidate.success_count as f32 / receipt_total as f32
    };
    let failure_rate = if receipt_total == 0 {
        0.0
    } else {
        candidate.failure_count as f32 / receipt_total as f32
    };
    let recency_score = candidate
        .age_ms
        .map(recency_score_from_age_ms)
        .unwrap_or(0.5);
    let graph_score = graph_degree_score(candidate.graph_degree);
    let mut score = candidate.similarity * policy.similarity_weight
        + receipt_score * policy.receipt_weight
        + recency_score * policy.recency_weight
        + graph_score * policy.graph_weight
        - failure_rate * policy.failure_penalty;
    if candidate.pinned {
        score += policy.pin_bonus;
    }
    if candidate.required {
        score += policy.required_bonus;
    }
    score = finite_or(score, 0.0).max(0.0);

    RankedContextAtom {
        node_id: candidate.node_id.clone(),
        label: candidate.label.clone(),
        summary: candidate.summary.clone(),
        score,
        token_cost: candidate.token_cost,
        rank: 0,
        selected: false,
        reasons: context_atom_reasons(
            candidate,
            receipt_total,
            receipt_score,
            failure_rate,
            recency_score,
            graph_score,
        ),
    }
}

fn context_atom_reasons(
    candidate: &ContextAtomCandidate,
    receipt_total: u32,
    receipt_score: f32,
    failure_rate: f32,
    recency_score: f32,
    graph_score: f32,
) -> Vec<String> {
    let mut reasons = Vec::new();
    if candidate.required {
        reasons.push("required".to_string());
    }
    if candidate.pinned {
        reasons.push("pinned".to_string());
    }
    if candidate.similarity >= 0.80 {
        reasons.push("similarity_signal".to_string());
    }
    if receipt_total == 0 {
        reasons.push("receipt_unknown".to_string());
    } else if receipt_score >= 0.75 {
        reasons.push("receipt_success".to_string());
    }
    if candidate.use_count > 0 {
        reasons.push("used_before".to_string());
    }
    if failure_rate > 0.0 {
        reasons.push("failure_penalty".to_string());
    }
    if recency_score >= 0.66 {
        reasons.push("recent".to_string());
    }
    if graph_score >= 0.25 {
        reasons.push("graph_central".to_string());
    }
    reasons
}

fn compare_ranked_context_atoms(
    a: &RankedContextAtom,
    b: &RankedContextAtom,
) -> std::cmp::Ordering {
    let a_required = a.reasons.iter().any(|reason| reason == "required");
    let b_required = b.reasons.iter().any(|reason| reason == "required");
    b_required
        .cmp(&a_required)
        .then_with(|| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| a.token_cost.cmp(&b.token_cost))
        .then_with(|| a.node_id.cmp(&b.node_id))
}

fn recency_score_from_age_ms(age_ms: u64) -> f32 {
    let age = age_ms as f32;
    (CONTEXT_RECENCY_HALF_LIFE_MS / (CONTEXT_RECENCY_HALF_LIFE_MS + age)).clamp(0.0, 1.0)
}

fn graph_degree_score(graph_degree: usize) -> f32 {
    (((graph_degree as f32) + 1.0).ln() / 65.0_f32.ln()).clamp(0.0, 1.0)
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
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
