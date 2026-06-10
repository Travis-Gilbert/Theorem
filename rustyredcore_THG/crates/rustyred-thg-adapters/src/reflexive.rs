//! Reflexive RustyRed learning contracts.
//!
//! Learned organs in this module obey one invariant: they rank or steer within
//! a bounded, enumerated space. They do not author free-form graph mutations.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    now_ms, stable_hash, EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, NodeRecord,
    ThgError, ThgResult,
};

use crate::pairformer::{
    run_pairformer, PairformerConfig, PairformerEdgeInput, PairformerInput, PairformerLinkScore,
};
use crate::types::{
    edge_with_adapter_provenance, normalize_tenant_id, thg_error_from_store, AdapterGraphStore,
    THG_ADAPTER_SOURCE,
};

pub const REPRESENTATION_SIDECAR_LABEL: &str = "RepresentationSidecar";
pub const REFLEXIVE_DENSIFICATION_RUN_LABEL: &str = "ReflexiveDensificationRun";
pub const REFLEXIVE_EDGE_CANDIDATE_LABEL: &str = "ReflexiveEdgeCandidate";

pub const REPRESENTS_NODE: &str = "REPRESENTS_NODE";
pub const REFLEXIVE_CANDIDATE_OF: &str = "REFLEXIVE_CANDIDATE_OF";
pub const REFLEXIVE_CANDIDATE_SOURCE: &str = "REFLEXIVE_CANDIDATE_SOURCE";
pub const REFLEXIVE_CANDIDATE_TARGET: &str = "REFLEXIVE_CANDIDATE_TARGET";

pub const DEFAULT_DENSIFICATION_MAX_NODES: usize = 128;
pub const DEFAULT_DENSIFICATION_MAX_DEPTH: usize = 2;
pub const DEFAULT_DENSIFICATION_MAX_CANDIDATES: usize = 64;
pub const DEFAULT_DENSIFICATION_CONFIDENCE_CEILING: f32 = 0.74;
pub const DEFAULT_SCATTER_BURN_NATIVE_MAX_ELEMENTS: usize = 262_144;
pub const DEFAULT_FIXED_POINT_SCALE: i64 = 1_000_000;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScatterAggregationPath {
    /// Burn `select` for row gather plus tensor `scatter` sum update.
    BurnScatterAdd,
    /// Integer atomic-compatible accumulation, then rescale to floats.
    FixedPointAtomicCompatible,
    /// Native-only fast path. Valid only when the runtime advertises float atomics.
    NativeFloatAtomicFastPath,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScatterAggregationRequest {
    pub num_edges: usize,
    pub feature_dim: usize,
    pub deterministic_required: bool,
    pub browser_webgpu_target: bool,
    pub float_atomic_add_available: bool,
    pub burn_native_max_elements: usize,
}

impl ScatterAggregationRequest {
    pub fn normalized(mut self) -> Self {
        if self.burn_native_max_elements == 0 {
            self.burn_native_max_elements = DEFAULT_SCATTER_BURN_NATIVE_MAX_ELEMENTS;
        }
        self
    }
}

pub fn choose_scatter_aggregation_path(
    request: ScatterAggregationRequest,
) -> ScatterAggregationPath {
    let request = request.normalized();
    let elements = request.num_edges.saturating_mul(request.feature_dim);
    if elements <= request.burn_native_max_elements {
        return ScatterAggregationPath::BurnScatterAdd;
    }
    if request.deterministic_required
        || request.browser_webgpu_target
        || !request.float_atomic_add_available
    {
        return ScatterAggregationPath::FixedPointAtomicCompatible;
    }
    ScatterAggregationPath::NativeFloatAtomicFastPath
}

/// Deterministic fixed-point scatter aggregation for small fixtures and for
/// backends where float atomics are unavailable. It mirrors the CubeCL kernel's
/// "one edge contributes once to degree" contract.
pub fn aggregate_messages_fixed_point(
    messages: &[Vec<f32>],
    edge_dst: &[usize],
    num_nodes: usize,
    scale: i64,
    mean_aggregate: bool,
) -> ThgResult<Vec<Vec<f32>>> {
    let scale = scale.max(1);
    if messages.len() != edge_dst.len() {
        return Err(ThgError::new(
            "scatter_shape_mismatch",
            "messages and edge_dst must have the same length",
        ));
    }
    let feature_dim = messages.first().map(Vec::len).unwrap_or(0);
    if messages.iter().any(|message| message.len() != feature_dim) {
        return Err(ThgError::new(
            "scatter_shape_mismatch",
            "all message rows must have the same feature dimension",
        ));
    }
    if edge_dst.iter().any(|dst| *dst >= num_nodes) {
        return Err(ThgError::new(
            "scatter_index_out_of_bounds",
            "edge_dst contains a destination outside num_nodes",
        ));
    }

    let mut sums = vec![vec![0_i64; feature_dim]; num_nodes];
    let mut degrees = vec![0_i64; num_nodes];
    for (message, dst) in messages.iter().zip(edge_dst) {
        degrees[*dst] += 1;
        for (slot, value) in sums[*dst].iter_mut().zip(message) {
            *slot += (*value as f64 * scale as f64).round() as i64;
        }
    }

    let mut out = vec![vec![0.0_f32; feature_dim]; num_nodes];
    for node_idx in 0..num_nodes {
        let divisor = if mean_aggregate {
            degrees[node_idx].max(1) as f32
        } else {
            1.0
        };
        for dim_idx in 0..feature_dim {
            out[node_idx][dim_idx] = sums[node_idx][dim_idx] as f32 / scale as f32 / divisor;
        }
    }
    Ok(out)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepresentationSidecarInput {
    pub tenant_id: String,
    pub representation_id: String,
    pub target_kind: RepresentationTargetKind,
    pub target_id: String,
    pub model_id: String,
    pub embedding: Vec<f32>,
    pub adapter_ids: Vec<String>,
    pub graph_version: u64,
    pub metadata: Value,
    pub manifest_version: u32,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RepresentationTargetKind {
    Node,
    Edge,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RepresentationSidecarWriteback {
    pub representation_node_id: String,
    pub transaction: rustyred_thg_core::GraphTransaction,
}

pub fn upsert_representation_sidecar<S: AdapterGraphStore>(
    store: &mut S,
    input: RepresentationSidecarInput,
    actor: Option<&str>,
) -> ThgResult<RepresentationSidecarWriteback> {
    let input = normalize_representation_input(input)?;
    match input.target_kind {
        RepresentationTargetKind::Node => ensure_node_exists(store, &input.target_id)?,
        RepresentationTargetKind::Edge => ensure_edge_exists(store, &input.target_id)?,
    }

    let node_id = representation_sidecar_node_id(&input.tenant_id, &input.representation_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        &node_id,
        [REPRESENTATION_SIDECAR_LABEL],
        json!({
            "tenant_id": input.tenant_id,
            "representation_id": input.representation_id,
            "target_kind": input.target_kind,
            "target_id": input.target_id,
            "model_id": input.model_id,
            "embedding": input.embedding,
            "adapter_ids": input.adapter_ids,
            "graph_version": input.graph_version,
            "metadata": input.metadata,
            "manifest_version": input.manifest_version,
            "source": THG_ADAPTER_SOURCE,
        }),
    ))];

    if input.target_kind == RepresentationTargetKind::Node {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&node_id, REPRESENTS_NODE, &input.target_id),
            &node_id,
            REPRESENTS_NODE,
            &input.target_id,
            json!({
                "tenant_id": input.tenant_id,
                "target_kind": "node",
            }),
            actor,
        )));
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(RepresentationSidecarWriteback {
        representation_node_id: node_id,
        transaction,
    })
}

pub fn representation_sidecar_node_id(tenant_id: &str, representation_id: &str) -> String {
    format!(
        "representation_sidecar:{}:{}",
        normalize_tenant_id(tenant_id),
        representation_id.trim()
    )
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DensificationRequest {
    pub tenant_id: String,
    pub seed_node_ids: Vec<String>,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub min_path_confidence: f32,
    pub confidence_threshold: f32,
    pub confidence_ceiling: f32,
    pub max_candidates: usize,
    pub admission_tier: String,
    pub model_id: String,
    pub allowed_edge_types: Vec<String>,
}

impl DensificationRequest {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = normalize_tenant_id(&self.tenant_id);
        self.seed_node_ids = self
            .seed_node_ids
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        if self.max_nodes == 0 {
            self.max_nodes = DEFAULT_DENSIFICATION_MAX_NODES;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_DENSIFICATION_MAX_DEPTH;
        }
        if self.max_candidates == 0 {
            self.max_candidates = DEFAULT_DENSIFICATION_MAX_CANDIDATES;
        }
        if self.confidence_ceiling <= 0.0 {
            self.confidence_ceiling = DEFAULT_DENSIFICATION_CONFIDENCE_CEILING;
        }
        self.min_path_confidence = self.min_path_confidence.clamp(0.0, 1.0);
        self.confidence_threshold = self.confidence_threshold.clamp(0.0, 1.0);
        self.confidence_ceiling = self.confidence_ceiling.clamp(0.0, 1.0);
        self.admission_tier = self.admission_tier.trim().to_string();
        if self.admission_tier.is_empty() {
            self.admission_tier = "advisory_inferred".to_string();
        }
        self.model_id = self.model_id.trim().to_string();
        if self.model_id.is_empty() {
            self.model_id = "reflexive-composition/heuristic-v1".to_string();
        }
        self.allowed_edge_types = self
            .allowed_edge_types
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InferredEdgeCandidate {
    pub candidate_id: String,
    pub tenant_id: String,
    pub source_id: String,
    pub target_id: String,
    pub proposed_edge_type: String,
    pub confidence: f32,
    pub confidence_ceiling: f32,
    pub admission_tier: String,
    pub model_id: String,
    pub support_path_edge_ids: Vec<String>,
    pub support_path_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DensificationResult {
    pub tenant_id: String,
    pub considered_node_ids: Vec<String>,
    pub bounded: bool,
    pub candidates: Vec<InferredEdgeCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DensificationQuarantineResult {
    pub run_node_id: String,
    pub candidate_node_ids: Vec<String>,
    pub transaction: rustyred_thg_core::GraphTransaction,
}

pub fn rank_densification_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
) -> ThgResult<DensificationResult> {
    let request = request.normalized();
    if request.seed_node_ids.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            bounded: false,
            candidates: Vec::new(),
        });
    }

    let node_ids = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let allowed_edge_types = request
        .allowed_edge_types
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let edge_refs = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && edge.effective_confidence() as f32 >= request.min_path_confidence
                && (allowed_edge_types.is_empty() || allowed_edge_types.contains(&edge.edge_type))
                && node_ids.contains(&edge.from_id)
                && node_ids.contains(&edge.to_id)
        })
        .collect::<Vec<_>>();

    let (considered, bounded) = bounded_neighborhood(&request, &edge_refs);
    let existing_direct_pairs = edge_refs
        .iter()
        .map(|edge| (edge.from_id.clone(), edge.to_id.clone()))
        .collect::<BTreeSet<_>>();

    let mut by_key: BTreeMap<(String, String, String), InferredEdgeCandidate> = BTreeMap::new();
    for first in &edge_refs {
        if !considered.contains(&first.from_id) || !considered.contains(&first.to_id) {
            continue;
        }
        for second in edge_refs.iter().filter(|edge| edge.from_id == first.to_id) {
            if !considered.contains(&second.to_id) {
                continue;
            }
            if first.from_id == second.to_id {
                continue;
            }
            let proposed_edge_type =
                normalized_inferred_edge_type(&first.edge_type, &second.edge_type);
            if existing_direct_pairs.contains(&(first.from_id.clone(), second.to_id.clone())) {
                continue;
            }
            let raw_confidence =
                ((first.effective_confidence() * second.effective_confidence()).sqrt()) as f32;
            let confidence = raw_confidence.min(request.confidence_ceiling);
            if confidence < request.confidence_threshold {
                continue;
            }
            let candidate = make_candidate(&request, first, second, proposed_edge_type, confidence);
            let key = (
                candidate.source_id.clone(),
                candidate.target_id.clone(),
                candidate.proposed_edge_type.clone(),
            );
            match by_key.get(&key) {
                Some(prior) if prior.confidence >= candidate.confidence => {}
                _ => {
                    by_key.insert(key, candidate);
                }
            }
        }
    }

    let mut candidates = by_key.into_values().collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    candidates.truncate(request.max_candidates);
    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded,
        candidates,
    })
}

pub fn rank_pairformer_densification_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
    config: PairformerConfig,
) -> ThgResult<DensificationResult> {
    let request = request.normalized();
    if request.seed_node_ids.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            bounded: false,
            candidates: Vec::new(),
        });
    }

    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let allowed_edge_types = request
        .allowed_edge_types
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let edge_refs = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && edge.effective_confidence() as f32 >= request.min_path_confidence
                && (allowed_edge_types.is_empty() || allowed_edge_types.contains(&edge.edge_type))
                && nodes_by_id.contains_key(&edge.from_id)
                && nodes_by_id.contains_key(&edge.to_id)
        })
        .collect::<Vec<_>>();

    let (mut considered, mut bounded) = bounded_neighborhood(&request, &edge_refs);
    considered.retain(|node_id| nodes_by_id.contains_key(node_id));
    let config = config.normalized();
    if considered.len() > config.max_nodes {
        bounded = true;
        considered = considered.into_iter().take(config.max_nodes).collect();
    }
    if considered.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            bounded,
            candidates: Vec::new(),
        });
    }

    let existing_direct_pairs = edge_refs
        .iter()
        .map(|edge| (edge.from_id.clone(), edge.to_id.clone()))
        .collect::<BTreeSet<_>>();
    let input = pairformer_input_from_graph(&considered, &nodes_by_id, &edge_refs);
    let output = run_pairformer(&input, config)?;

    let mut candidates = output
        .link_scores
        .iter()
        .filter_map(|score| pairformer_score_to_candidate(&request, score, &existing_direct_pairs))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    candidates.truncate(request.max_candidates);

    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded,
        candidates,
    })
}

pub fn quarantine_densification_candidates<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    run_id: &str,
    candidates: &[InferredEdgeCandidate],
    actor: Option<&str>,
) -> ThgResult<DensificationQuarantineResult> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err(ThgError::new(
            "invalid_densification_run",
            "run_id is required",
        ));
    }
    let run_node_id = densification_run_node_id(&tenant_id, run_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        &run_node_id,
        [REFLEXIVE_DENSIFICATION_RUN_LABEL],
        json!({
            "tenant_id": tenant_id,
            "run_id": run_id,
            "candidate_count": candidates.len(),
            "created_at_ms": now_ms(),
            "source": THG_ADAPTER_SOURCE,
        }),
    ))];
    let mut candidate_node_ids = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        ensure_node_exists(store, &candidate.source_id)?;
        ensure_node_exists(store, &candidate.target_id)?;
        let candidate_node_id =
            densification_candidate_node_id(&tenant_id, &candidate.candidate_id);
        candidate_node_ids.push(candidate_node_id.clone());
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            &candidate_node_id,
            [REFLEXIVE_EDGE_CANDIDATE_LABEL],
            json!({
                "tenant_id": tenant_id,
                "candidate_id": candidate.candidate_id,
                "source_id": candidate.source_id,
                "target_id": candidate.target_id,
                "proposed_edge_type": candidate.proposed_edge_type,
                "confidence": candidate.confidence,
                "confidence_ceiling": candidate.confidence_ceiling,
                "admission_tier": candidate.admission_tier,
                "model_id": candidate.model_id,
                "support_path_edge_ids": candidate.support_path_edge_ids,
                "support_path_node_ids": candidate.support_path_node_ids,
                "quarantined": true,
                "source": THG_ADAPTER_SOURCE,
            }),
        )));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(&run_node_id, REFLEXIVE_CANDIDATE_OF, &candidate_node_id),
            &run_node_id,
            REFLEXIVE_CANDIDATE_OF,
            &candidate_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(
                &candidate_node_id,
                REFLEXIVE_CANDIDATE_SOURCE,
                &candidate.source_id,
            ),
            &candidate_node_id,
            REFLEXIVE_CANDIDATE_SOURCE,
            &candidate.source_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            edge_id(
                &candidate_node_id,
                REFLEXIVE_CANDIDATE_TARGET,
                &candidate.target_id,
            ),
            &candidate_node_id,
            REFLEXIVE_CANDIDATE_TARGET,
            &candidate.target_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(DensificationQuarantineResult {
        run_node_id,
        candidate_node_ids,
        transaction,
    })
}

pub fn densification_run_node_id(tenant_id: &str, run_id: &str) -> String {
    format!(
        "reflexive_densification_run:{}:{}",
        normalize_tenant_id(tenant_id),
        run_id.trim()
    )
}

pub fn densification_candidate_node_id(tenant_id: &str, candidate_id: &str) -> String {
    format!(
        "reflexive_edge_candidate:{}:{}",
        normalize_tenant_id(tenant_id),
        candidate_id.trim()
    )
}

fn normalize_representation_input(
    mut input: RepresentationSidecarInput,
) -> ThgResult<RepresentationSidecarInput> {
    input.tenant_id = normalize_tenant_id(&input.tenant_id);
    input.representation_id = input.representation_id.trim().to_string();
    input.target_id = input.target_id.trim().to_string();
    input.model_id = input.model_id.trim().to_string();
    input.adapter_ids = input
        .adapter_ids
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if input.manifest_version == 0 {
        input.manifest_version = 1;
    }
    if input.representation_id.is_empty() {
        return Err(ThgError::new(
            "invalid_representation_sidecar",
            "representation_id is required",
        ));
    }
    if input.target_id.is_empty() {
        return Err(ThgError::new(
            "invalid_representation_sidecar",
            "target_id is required",
        ));
    }
    if input.model_id.is_empty() {
        return Err(ThgError::new(
            "invalid_representation_sidecar",
            "model_id is required",
        ));
    }
    if input.embedding.is_empty() || input.embedding.iter().any(|value| !value.is_finite()) {
        return Err(ThgError::new(
            "invalid_representation_sidecar",
            "embedding must contain finite values",
        ));
    }
    Ok(input)
}

fn ensure_node_exists<S: AdapterGraphStore>(store: &S, node_id: &str) -> ThgResult<()> {
    if store
        .get_node(node_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ThgError::new(
            "missing_graph_endpoint",
            format!("node {node_id} does not exist"),
        ))
    }
}

fn ensure_edge_exists<S: AdapterGraphStore>(store: &S, edge_id: &str) -> ThgResult<()> {
    if store
        .get_edge(edge_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        Ok(())
    } else {
        Err(ThgError::new(
            "missing_graph_endpoint",
            format!("edge {edge_id} does not exist"),
        ))
    }
}

fn bounded_neighborhood(
    request: &DensificationRequest,
    edges: &[&EdgeRecord],
) -> (BTreeSet<String>, bool) {
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for edge in edges {
        adjacency
            .entry(edge.from_id.clone())
            .or_default()
            .insert(edge.to_id.clone());
    }

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in &request.seed_node_ids {
        if visited.insert(seed.clone()) {
            queue.push_back((seed.clone(), 0usize));
        }
    }

    let mut bounded = false;
    while let Some((node_id, depth)) = queue.pop_front() {
        if depth >= request.max_depth {
            continue;
        }
        let Some(neighbors) = adjacency.get(&node_id) else {
            continue;
        };
        for neighbor in neighbors {
            if visited.contains(neighbor) {
                continue;
            }
            if visited.len() >= request.max_nodes {
                bounded = true;
                continue;
            }
            visited.insert(neighbor.clone());
            queue.push_back((neighbor.clone(), depth + 1));
        }
    }
    (visited, bounded)
}

fn make_candidate(
    request: &DensificationRequest,
    first: &EdgeRecord,
    second: &EdgeRecord,
    proposed_edge_type: String,
    confidence: f32,
) -> InferredEdgeCandidate {
    let support_path_edge_ids = vec![first.id.clone(), second.id.clone()];
    let support_path_node_ids = vec![
        first.from_id.clone(),
        first.to_id.clone(),
        second.to_id.clone(),
    ];
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "source_id": first.from_id,
        "target_id": second.to_id,
        "proposed_edge_type": proposed_edge_type,
        "support_path_edge_ids": support_path_edge_ids,
        "model_id": request.model_id,
    }));
    InferredEdgeCandidate {
        candidate_id,
        tenant_id: request.tenant_id.clone(),
        source_id: first.from_id.clone(),
        target_id: second.to_id.clone(),
        proposed_edge_type,
        confidence,
        confidence_ceiling: request.confidence_ceiling,
        admission_tier: request.admission_tier.clone(),
        model_id: request.model_id.clone(),
        support_path_edge_ids,
        support_path_node_ids,
    }
}

fn pairformer_input_from_graph(
    considered: &BTreeSet<String>,
    nodes_by_id: &BTreeMap<String, &NodeRecord>,
    edges: &[&EdgeRecord],
) -> PairformerInput {
    let nodes = considered
        .iter()
        .filter_map(|node_id| {
            nodes_by_id
                .get(node_id)
                .map(|node| crate::pairformer::PairformerNodeInput {
                    node_id: node.id.clone(),
                    features: node_features(node),
                })
        })
        .collect::<Vec<_>>();
    let edges = edges
        .iter()
        .filter(|edge| considered.contains(&edge.from_id) && considered.contains(&edge.to_id))
        .map(|edge| PairformerEdgeInput {
            edge_id: edge.id.clone(),
            source_id: edge.from_id.clone(),
            target_id: edge.to_id.clone(),
            edge_type: edge.edge_type.clone(),
            features: edge_features(edge),
            confidence: edge.effective_confidence() as f32,
        })
        .collect::<Vec<_>>();
    PairformerInput { nodes, edges }
}

fn pairformer_score_to_candidate(
    request: &DensificationRequest,
    score: &PairformerLinkScore,
    existing_direct_pairs: &BTreeSet<(String, String)>,
) -> Option<InferredEdgeCandidate> {
    if existing_direct_pairs.contains(&(score.source_id.clone(), score.target_id.clone())) {
        return None;
    }
    let support_path = score.support_path.as_ref()?;
    let raw_confidence = (score.score * support_path.confidence).clamp(0.0, 1.0);
    let confidence = raw_confidence.min(request.confidence_ceiling);
    if confidence < request.confidence_threshold {
        return None;
    }
    let proposed_edge_type = support_path.relation_hint.clone();
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "source_id": score.source_id,
        "target_id": score.target_id,
        "proposed_edge_type": proposed_edge_type,
        "support_path_edge_ids": support_path.edge_ids,
        "model_id": request.model_id,
        "pairformer_score": score.score,
    }));
    Some(InferredEdgeCandidate {
        candidate_id,
        tenant_id: request.tenant_id.clone(),
        source_id: score.source_id.clone(),
        target_id: score.target_id.clone(),
        proposed_edge_type,
        confidence,
        confidence_ceiling: request.confidence_ceiling,
        admission_tier: request.admission_tier.clone(),
        model_id: request.model_id.clone(),
        support_path_edge_ids: support_path.edge_ids.clone(),
        support_path_node_ids: support_path.node_ids.clone(),
    })
}

fn node_features(node: &NodeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&node.properties, "embedding");
    features.extend(numeric_array_property(&node.properties, "features"));
    features.push(node.labels.len() as f32);
    features.push(node.version as f32 / 1024.0);
    features
}

fn edge_features(edge: &EdgeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&edge.properties, "embedding");
    features.extend(numeric_array_property(&edge.properties, "features"));
    features.push(edge.effective_confidence() as f32);
    features.push((edge.edge_type.len() as f32).ln_1p());
    features
}

fn numeric_array_property(properties: &Value, key: &str) -> Vec<f32> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_f64()
                        .map(|value| value as f32)
                        .or_else(|| item.as_str().and_then(|raw| raw.parse::<f32>().ok()))
                })
                .filter(|value| value.is_finite())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn normalized_inferred_edge_type(left: &str, right: &str) -> String {
    if left == right {
        format!("INFERRED_{left}")
    } else {
        format!("INFERRED_{left}_THEN_{right}")
    }
}

fn edge_id(from_id: &str, edge_type: &str, to_id: &str) -> String {
    format!("edge:{from_id}:{edge_type}:{to_id}")
}
