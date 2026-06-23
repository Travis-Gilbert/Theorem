//! Reflexive RustyRed learning contracts.
//!
//! Learned organs in this module obey one invariant: they rank or steer within
//! a bounded, enumerated space. They do not author free-form graph mutations.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use rustyred_thg_core::spatial::SpatialIndex;
use rustyred_thg_core::{
    now_ms, stable_hash, EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, NodeRecord,
    SpatialDesignation, ThgError, ThgResult,
};

use crate::hot::{hot_input_from_snapshot, run_hot, HotConfig, HotLinkScore};
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
pub const REFLEXIVE_PROPERTY_CANDIDATE_LABEL: &str = "ReflexivePropertyCandidate";

pub const REPRESENTS_NODE: &str = "REPRESENTS_NODE";
pub const REFLEXIVE_CANDIDATE_OF: &str = "REFLEXIVE_CANDIDATE_OF";
pub const REFLEXIVE_CANDIDATE_SOURCE: &str = "REFLEXIVE_CANDIDATE_SOURCE";
pub const REFLEXIVE_CANDIDATE_TARGET: &str = "REFLEXIVE_CANDIDATE_TARGET";

pub const DEFAULT_DENSIFICATION_MAX_NODES: usize = 128;
pub const DEFAULT_DENSIFICATION_MAX_DEPTH: usize = 2;
pub const DEFAULT_DENSIFICATION_MAX_CANDIDATES: usize = 64;
pub const DEFAULT_DENSIFICATION_CONFIDENCE_CEILING: f32 = 0.74;
pub const DEFAULT_SPATIAL_RADIUS_KM: f64 = 1.0;
pub const DEFAULT_SPATIAL_RESOLUTION: u8 = 9;
pub const DEFAULT_TEMPORAL_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;
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
        self.tenant_id = self.tenant_id.trim().to_string();
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
pub struct InferredPropertyCandidate {
    pub candidate_id: String,
    pub tenant_id: String,
    pub target_node_id: String,
    pub property_key: String,
    pub proposed_value: Value,
    pub confidence: f32,
    pub confidence_ceiling: f32,
    pub admission_tier: String,
    pub model_id: String,
    pub support_edge_ids: Vec<String>,
    pub support_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DensificationResult {
    pub tenant_id: String,
    pub considered_node_ids: Vec<String>,
    pub bounded: bool,
    pub candidates: Vec<InferredEdgeCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PropertyCandidateResult {
    pub tenant_id: String,
    pub considered_node_ids: Vec<String>,
    pub candidates: Vec<InferredPropertyCandidate>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DensificationQuarantineResult {
    pub run_node_id: String,
    pub candidate_node_ids: Vec<String>,
    pub transaction: rustyred_thg_core::GraphTransaction,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PropertyCandidateQuarantineOptions {
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PropertyCandidateQuarantineResult {
    pub run_node_id: String,
    pub candidate_node_ids: Vec<String>,
    pub applied_target_node_ids: Vec<String>,
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

pub fn rank_hot_temporal_densification_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
    config: HotConfig,
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
    let query_pairs = hot_candidate_pairs(
        &considered,
        &nodes_by_id,
        &edge_refs,
        &existing_direct_pairs,
        request.max_candidates.saturating_mul(4).max(request.max_candidates),
    );
    if query_pairs.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: considered.into_iter().collect(),
            bounded,
            candidates: Vec::new(),
        });
    }
    let as_of = edge_refs
        .iter()
        .filter_map(|edge| temporal_edge_timestamp_for_hot(edge))
        .max()
        .unwrap_or_else(now_ms)
        .saturating_add(1);
    let hot_input = hot_input_from_snapshot(snapshot, query_pairs, as_of, config.clone())?;
    let output = run_hot(&hot_input, config)?;

    let mut candidates = output
        .link_scores
        .iter()
        .filter_map(|score| hot_score_to_candidate(&request, score, &existing_direct_pairs))
        .collect::<Vec<_>>();
    sort_edge_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);

    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded,
        candidates,
    })
}

pub fn rank_spatial_candidates(
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

    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let geo_by_id = nodes_by_id
        .iter()
        .filter_map(|(id, node)| geo_point(&node.properties).map(|geo| (id.clone(), geo)))
        .collect::<BTreeMap<_, _>>();
    if geo_by_id.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            bounded: false,
            candidates: Vec::new(),
        });
    }

    let existing_direct = existing_direct_edge_keys(snapshot);
    let mut index = SpatialIndex::for_designation(SpatialDesignation {
        label: "ReflexiveSpatialCandidate".to_string(),
        lat_property: "lat".to_string(),
        lon_property: "lon".to_string(),
        resolution: DEFAULT_SPATIAL_RESOLUTION,
    });
    for (node_id, point) in &geo_by_id {
        index
            .upsert(node_id, point.lat, point.lon)
            .map_err(|err| ThgError::new(err.code(), err.message()))?;
    }

    let mut considered = BTreeSet::new();
    let mut by_key: BTreeMap<(String, String, String), InferredEdgeCandidate> = BTreeMap::new();
    for seed_id in &request.seed_node_ids {
        let Some(seed_point) = geo_by_id.get(seed_id) else {
            continue;
        };
        considered.insert(seed_id.clone());
        let near_ids = index
            .radius_search(seed_point.lat, seed_point.lon, DEFAULT_SPATIAL_RADIUS_KM)
            .map_err(|err| ThgError::new(err.code(), err.message()))?;
        for target_id in near_ids {
            if target_id == *seed_id {
                continue;
            }
            let Some(target_point) = geo_by_id.get(&target_id) else {
                continue;
            };
            considered.insert(target_id.clone());
            let distance_km = haversine_km(
                seed_point.lat,
                seed_point.lon,
                target_point.lat,
                target_point.lon,
            );
            let proposed_edge_type = if distance_km <= 0.01 {
                "CO_LOCATED"
            } else {
                "NEAR"
            };
            if existing_direct.contains(&(
                seed_id.clone(),
                target_id.clone(),
                proposed_edge_type.to_string(),
            )) {
                continue;
            }
            let raw_confidence =
                (1.0 - (distance_km / DEFAULT_SPATIAL_RADIUS_KM)).clamp(0.0, 1.0) as f32;
            let confidence = raw_confidence.min(request.confidence_ceiling);
            if confidence < request.confidence_threshold {
                continue;
            }
            let candidate = make_spatial_candidate(
                &request,
                seed_id,
                &target_id,
                proposed_edge_type,
                distance_km,
                confidence,
            );
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
    sort_edge_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);
    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded: false,
        candidates,
    })
}

pub fn rank_temporal_candidates(
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

    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let temporal_by_id = nodes_by_id
        .iter()
        .filter_map(|(id, node)| {
            temporal_interval(&node.properties).map(|interval| (id.clone(), interval))
        })
        .collect::<BTreeMap<_, _>>();
    if temporal_by_id.is_empty() {
        return Ok(DensificationResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            bounded: false,
            candidates: Vec::new(),
        });
    }

    let existing_direct = existing_direct_edge_keys(snapshot);
    let mut considered = BTreeSet::new();
    let mut by_key: BTreeMap<(String, String, String), InferredEdgeCandidate> = BTreeMap::new();
    for seed_id in &request.seed_node_ids {
        let Some(seed_interval) = temporal_by_id.get(seed_id) else {
            continue;
        };
        considered.insert(seed_id.clone());
        for (other_id, other_interval) in &temporal_by_id {
            if other_id == seed_id {
                continue;
            }
            let Some((source_id, target_id, edge_type, raw_confidence)) =
                temporal_relation(seed_id, seed_interval, other_id, other_interval)
            else {
                continue;
            };
            if existing_direct.contains(&(source_id.clone(), target_id.clone(), edge_type.clone()))
            {
                continue;
            }
            let confidence = raw_confidence.min(request.confidence_ceiling);
            if confidence < request.confidence_threshold {
                continue;
            }
            considered.insert(other_id.clone());
            let candidate = make_temporal_candidate(
                &request,
                &source_id,
                &target_id,
                &edge_type,
                seed_interval,
                other_interval,
                confidence,
            );
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
    sort_edge_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);
    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded: false,
        candidates,
    })
}

pub fn rank_reflexive_organizing_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
    pairformer_config: PairformerConfig,
) -> ThgResult<DensificationResult> {
    let request = request.normalized();
    let graph = rank_densification_candidates(snapshot, request.clone())?;
    let pairformer =
        rank_pairformer_densification_candidates(snapshot, request.clone(), pairformer_config)?;
    let hot =
        rank_hot_temporal_densification_candidates(snapshot, request.clone(), HotConfig::default())?;
    let spatial = rank_spatial_candidates(snapshot, request.clone())?;
    let temporal = rank_temporal_candidates(snapshot, request.clone())?;

    let mut considered = BTreeSet::new();
    let mut bounded = false;
    let mut by_key: BTreeMap<(String, String, String), InferredEdgeCandidate> = BTreeMap::new();
    for result in [graph, pairformer, hot, spatial, temporal] {
        bounded |= result.bounded;
        considered.extend(result.considered_node_ids);
        for candidate in result.candidates {
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
    sort_edge_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);
    Ok(DensificationResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        bounded,
        candidates,
    })
}

pub fn rank_missing_property_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
) -> ThgResult<PropertyCandidateResult> {
    rank_property_candidates_for_keys(snapshot, request, None)
}

pub fn rank_classification_property_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
) -> ThgResult<PropertyCandidateResult> {
    rank_property_candidates_for_keys(
        snapshot,
        request,
        Some(BTreeSet::from(["classification".to_string()])),
    )
}

pub fn rank_property_candidates(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
) -> ThgResult<PropertyCandidateResult> {
    let request = request.normalized();
    let missing = rank_missing_property_candidates(snapshot, request.clone())?;
    let classification = rank_classification_property_candidates(snapshot, request.clone())?;
    let mut considered = BTreeSet::new();
    let mut by_key: BTreeMap<(String, String, String), InferredPropertyCandidate> = BTreeMap::new();
    for result in [missing, classification] {
        considered.extend(result.considered_node_ids);
        for candidate in result.candidates {
            let key = (
                candidate.target_node_id.clone(),
                candidate.property_key.clone(),
                stable_hash(&candidate.proposed_value),
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
    sort_property_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);
    Ok(PropertyCandidateResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
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
    let tenant_id = tenant_id.trim().to_string();
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

pub fn quarantine_property_candidates<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    run_id: &str,
    candidates: &[InferredPropertyCandidate],
    actor: Option<&str>,
) -> ThgResult<PropertyCandidateQuarantineResult> {
    quarantine_property_candidates_with_options(
        store,
        tenant_id,
        run_id,
        candidates,
        actor,
        PropertyCandidateQuarantineOptions::default(),
    )
}

pub fn quarantine_property_candidates_with_options<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    run_id: &str,
    candidates: &[InferredPropertyCandidate],
    actor: Option<&str>,
    options: PropertyCandidateQuarantineOptions,
) -> ThgResult<PropertyCandidateQuarantineResult> {
    let tenant_id = tenant_id.trim().to_string();
    let run_id = run_id.trim();
    if run_id.is_empty() {
        return Err(ThgError::new(
            "invalid_property_candidate_run",
            "run_id is required",
        ));
    }
    let run_node_id = property_candidate_run_node_id(&tenant_id, run_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        &run_node_id,
        [REFLEXIVE_DENSIFICATION_RUN_LABEL],
        json!({
            "tenant_id": tenant_id,
            "run_id": run_id,
            "candidate_count": candidates.len(),
            "candidate_kind": "property",
            "created_at_ms": now_ms(),
            "dry_run": options.dry_run,
            "source": THG_ADAPTER_SOURCE,
        }),
    ))];
    let mut candidate_node_ids = Vec::with_capacity(candidates.len());
    let mut applied_target_node_ids = Vec::new();

    for candidate in candidates {
        let mut target_node = store
            .get_node(&candidate.target_node_id)
            .map_err(thg_error_from_store)?
            .ok_or_else(|| {
                ThgError::new(
                    "missing_graph_endpoint",
                    format!("node {} does not exist", candidate.target_node_id),
                )
            })?;
        let candidate_node_id = property_candidate_node_id(&tenant_id, &candidate.candidate_id);
        candidate_node_ids.push(candidate_node_id.clone());
        let applies = candidate.confidence >= candidate.confidence_ceiling && !options.dry_run;
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            &candidate_node_id,
            [REFLEXIVE_PROPERTY_CANDIDATE_LABEL],
            json!({
                "tenant_id": tenant_id,
                "candidate_id": candidate.candidate_id,
                "target_node_id": candidate.target_node_id,
                "property_key": candidate.property_key,
                "proposed_value": candidate.proposed_value,
                "confidence": candidate.confidence,
                "confidence_ceiling": candidate.confidence_ceiling,
                "admission_tier": candidate.admission_tier,
                "model_id": candidate.model_id,
                "support_edge_ids": candidate.support_edge_ids,
                "support_node_ids": candidate.support_node_ids,
                "quarantined": true,
                "applied": applies,
                "source": THG_ADAPTER_SOURCE,
                "actor": actor.unwrap_or_default(),
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
                REFLEXIVE_CANDIDATE_TARGET,
                &candidate.target_node_id,
            ),
            &candidate_node_id,
            REFLEXIVE_CANDIDATE_TARGET,
            &candidate.target_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
        for support_node_id in &candidate.support_node_ids {
            if support_node_id == &candidate.target_node_id {
                continue;
            }
            ensure_node_exists(store, support_node_id)?;
            mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
                edge_id(
                    &candidate_node_id,
                    REFLEXIVE_CANDIDATE_SOURCE,
                    support_node_id,
                ),
                &candidate_node_id,
                REFLEXIVE_CANDIDATE_SOURCE,
                support_node_id,
                json!({ "tenant_id": tenant_id }),
                actor,
            )));
        }

        if applies {
            let mut properties = target_node
                .properties
                .as_object()
                .cloned()
                .unwrap_or_else(Map::new);
            properties.insert(
                candidate.property_key.clone(),
                candidate.proposed_value.clone(),
            );
            target_node.properties = Value::Object(properties);
            mutations.push(GraphMutation::NodeUpsert(target_node));
            applied_target_node_ids.push(candidate.target_node_id.clone());
        }
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(PropertyCandidateQuarantineResult {
        run_node_id,
        candidate_node_ids,
        applied_target_node_ids,
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

pub fn property_candidate_run_node_id(tenant_id: &str, run_id: &str) -> String {
    format!(
        "reflexive_property_candidate_run:{}:{}",
        normalize_tenant_id(tenant_id),
        run_id.trim()
    )
}

pub fn property_candidate_node_id(tenant_id: &str, candidate_id: &str) -> String {
    format!(
        "reflexive_property_candidate:{}:{}",
        normalize_tenant_id(tenant_id),
        candidate_id.trim()
    )
}

fn normalize_representation_input(
    mut input: RepresentationSidecarInput,
) -> ThgResult<RepresentationSidecarInput> {
    // Tenant stays raw in the input/property; the *_node_id builders apply the
    // single canonical normalization when constructing keys. Normalizing here as
    // well would double-encode under the injective tenant scheme.
    input.tenant_id = input.tenant_id.trim().to_string();
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

pub(crate) fn bounded_neighborhood(
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

pub(crate) fn pairformer_input_from_graph(
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

pub(crate) fn pairformer_score_to_candidate(
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

fn hot_candidate_pairs(
    considered: &BTreeSet<String>,
    nodes_by_id: &BTreeMap<String, &NodeRecord>,
    edges: &[&EdgeRecord],
    existing_direct_pairs: &BTreeSet<(String, String)>,
    limit: usize,
) -> Vec<(String, String)> {
    let mut by_pair = BTreeMap::<(String, String), f32>::new();
    let mut by_source = BTreeMap::<String, Vec<&EdgeRecord>>::new();
    for edge in edges {
        if !considered.contains(&edge.from_id) || !considered.contains(&edge.to_id) {
            continue;
        }
        by_source.entry(edge.from_id.clone()).or_default().push(edge);
    }

    for first in edges {
        if !considered.contains(&first.from_id) || !considered.contains(&first.to_id) {
            continue;
        }
        let Some(seconds) = by_source.get(&first.to_id) else {
            continue;
        };
        for second in seconds {
            if first.from_id == second.to_id {
                continue;
            }
            add_hot_pair_score(
                &mut by_pair,
                existing_direct_pairs,
                first.from_id.clone(),
                second.to_id.clone(),
                0.8 * (first.effective_confidence() * second.effective_confidence()).sqrt()
                    as f32,
            );
        }
    }

    let mut embeddings = considered
        .iter()
        .filter_map(|node_id| {
            nodes_by_id.get(node_id).map(|node| {
                let mut features = numeric_array_property(&node.properties, "embedding");
                features.extend(numeric_array_property(&node.properties, "features"));
                (node_id.clone(), features)
            })
        })
        .filter(|(_, features)| !features.is_empty())
        .collect::<Vec<_>>();
    embeddings.sort_by(|left, right| left.0.cmp(&right.0));
    for left_idx in 0..embeddings.len() {
        for right_idx in 0..embeddings.len() {
            if left_idx == right_idx {
                continue;
            }
            let similarity = cosine_similarity(&embeddings[left_idx].1, &embeddings[right_idx].1);
            if similarity > 0.25 {
                add_hot_pair_score(
                    &mut by_pair,
                    existing_direct_pairs,
                    embeddings[left_idx].0.clone(),
                    embeddings[right_idx].0.clone(),
                    0.25 + similarity.max(0.0) * 0.35,
                );
            }
        }
    }

    let mut recent_edges = edges
        .iter()
        .filter(|edge| considered.contains(&edge.from_id) && considered.contains(&edge.to_id))
        .filter_map(|edge| temporal_edge_timestamp_for_hot(edge).map(|timestamp| (*edge, timestamp)))
        .collect::<Vec<_>>();
    recent_edges.sort_by(|left, right| right.1.cmp(&left.1));
    recent_edges.truncate(limit.max(1));
    for (left_idx, (left, _)) in recent_edges.iter().enumerate() {
        for (right, _) in recent_edges.iter().skip(left_idx + 1) {
            for (source_id, target_id) in [
                (left.from_id.clone(), right.from_id.clone()),
                (left.from_id.clone(), right.to_id.clone()),
                (left.to_id.clone(), right.to_id.clone()),
            ] {
                add_hot_pair_score(
                    &mut by_pair,
                    existing_direct_pairs,
                    source_id,
                    target_id,
                    0.2,
                );
            }
        }
    }

    let mut rows = by_pair.into_iter().collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    rows.truncate(limit);
    rows.into_iter().map(|(pair, _)| pair).collect()
}

fn add_hot_pair_score(
    by_pair: &mut BTreeMap<(String, String), f32>,
    existing_direct_pairs: &BTreeSet<(String, String)>,
    source_id: String,
    target_id: String,
    score: f32,
) {
    if source_id == target_id || existing_direct_pairs.contains(&(source_id.clone(), target_id.clone())) {
        return;
    }
    let slot = by_pair.entry((source_id, target_id)).or_insert(0.0);
    *slot = (*slot).max(score);
}

fn hot_score_to_candidate(
    request: &DensificationRequest,
    score: &HotLinkScore,
    existing_direct_pairs: &BTreeSet<(String, String)>,
) -> Option<InferredEdgeCandidate> {
    if existing_direct_pairs.contains(&(score.source_id.clone(), score.target_id.clone())) {
        return None;
    }
    let support = score.support.as_ref()?;
    let raw_confidence = (score.score * support.confidence).clamp(0.0, 1.0);
    let confidence = raw_confidence.min(request.confidence_ceiling);
    if confidence < request.confidence_threshold {
        return None;
    }
    let proposed_edge_type = support.relation_hint.clone();
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "source_id": score.source_id,
        "target_id": score.target_id,
        "proposed_edge_type": proposed_edge_type,
        "support_path_edge_ids": support.edge_ids,
        "model_id": request.model_id,
        "hot_score": score.score,
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
        support_path_edge_ids: support.edge_ids.clone(),
        support_path_node_ids: support.node_ids.clone(),
    })
}

fn rank_property_candidates_for_keys(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
    only_keys: Option<BTreeSet<String>>,
) -> ThgResult<PropertyCandidateResult> {
    let request = request.normalized();
    if request.seed_node_ids.is_empty() {
        return Ok(PropertyCandidateResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            candidates: Vec::new(),
        });
    }
    let allowed_keys = only_keys;
    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let edge_refs = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && nodes_by_id.contains_key(&edge.from_id)
                && nodes_by_id.contains_key(&edge.to_id)
        })
        .collect::<Vec<_>>();
    let mut neighbors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut support_edges: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for edge in &edge_refs {
        neighbors
            .entry(edge.from_id.clone())
            .or_default()
            .insert(edge.to_id.clone());
        neighbors
            .entry(edge.to_id.clone())
            .or_default()
            .insert(edge.from_id.clone());
        support_edges
            .entry((edge.from_id.clone(), edge.to_id.clone()))
            .or_default()
            .push(edge.id.clone());
        support_edges
            .entry((edge.to_id.clone(), edge.from_id.clone()))
            .or_default()
            .push(edge.id.clone());
    }

    let mut considered = BTreeSet::new();
    let mut by_key: BTreeMap<(String, String, String), InferredPropertyCandidate> = BTreeMap::new();
    for seed_id in &request.seed_node_ids {
        let Some(seed_node) = nodes_by_id.get(seed_id) else {
            continue;
        };
        considered.insert(seed_id.clone());
        let mut support_nodes = neighbors
            .get(seed_id)
            .into_iter()
            .flat_map(|ids| ids.iter().cloned())
            .collect::<BTreeSet<_>>();
        if support_nodes.is_empty() {
            for (other_id, other) in &nodes_by_id {
                if *other_id != *seed_id && labels_overlap(seed_node, other) {
                    support_nodes.insert(other_id.clone());
                }
            }
        }
        considered.extend(support_nodes.iter().cloned());
        let proposals = property_value_votes(
            seed_node,
            support_nodes
                .iter()
                .filter_map(|id| nodes_by_id.get(id).copied()),
            allowed_keys.as_ref(),
        );
        for proposal in proposals {
            if proposal.confidence < request.confidence_threshold {
                continue;
            }
            let confidence = proposal.confidence.min(request.confidence_ceiling);
            let support_edge_ids = proposal
                .support_node_ids
                .iter()
                .flat_map(|support_node_id| {
                    support_edges
                        .get(&(seed_id.clone(), support_node_id.clone()))
                        .cloned()
                        .unwrap_or_default()
                })
                .collect::<Vec<_>>();
            let candidate = make_property_candidate(
                &request,
                seed_id,
                &proposal.property_key,
                proposal.value,
                confidence,
                support_edge_ids,
                proposal.support_node_ids,
            );
            let key = (
                candidate.target_node_id.clone(),
                candidate.property_key.clone(),
                stable_hash(&candidate.proposed_value),
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
    sort_property_candidates(&mut candidates);
    candidates.truncate(request.max_candidates);
    Ok(PropertyCandidateResult {
        tenant_id: request.tenant_id,
        considered_node_ids: considered.into_iter().collect(),
        candidates,
    })
}

#[derive(Clone, Debug)]
struct PropertyVote {
    property_key: String,
    value: Value,
    confidence: f32,
    support_node_ids: Vec<String>,
}

fn property_value_votes<'a>(
    target: &NodeRecord,
    support_nodes: impl Iterator<Item = &'a NodeRecord>,
    allowed_keys: Option<&BTreeSet<String>>,
) -> Vec<PropertyVote> {
    let mut votes: BTreeMap<String, BTreeMap<String, (Value, Vec<String>)>> = BTreeMap::new();
    for support in support_nodes {
        let Some(properties) = support.properties.as_object() else {
            continue;
        };
        for (key, value) in properties {
            if ignored_property_key(key) || !scalar_value(value) {
                continue;
            }
            if allowed_keys.is_some_and(|allowed| !allowed.contains(key)) {
                continue;
            }
            if node_has_property(target, key) {
                continue;
            }
            votes
                .entry(key.clone())
                .or_default()
                .entry(stable_hash(value))
                .or_insert_with(|| (value.clone(), Vec::new()))
                .1
                .push(support.id.clone());
        }
    }

    let mut out = Vec::new();
    for (property_key, by_value) in votes {
        let total = by_value
            .values()
            .map(|(_, support_ids)| support_ids.len())
            .sum::<usize>()
            .max(1);
        let Some((value, support_node_ids)) = by_value
            .into_values()
            .max_by(|(_, left), (_, right)| left.len().cmp(&right.len()))
        else {
            continue;
        };
        let agreement = support_node_ids.len() as f32 / total as f32;
        let confidence = (0.55 + agreement * 0.4).min(0.95);
        out.push(PropertyVote {
            property_key,
            value,
            confidence,
            support_node_ids,
        });
    }
    out
}

fn make_spatial_candidate(
    request: &DensificationRequest,
    source_id: &str,
    target_id: &str,
    proposed_edge_type: &str,
    distance_km: f64,
    confidence: f32,
) -> InferredEdgeCandidate {
    let support_path_node_ids = vec![source_id.to_string(), target_id.to_string()];
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "source_id": source_id,
        "target_id": target_id,
        "proposed_edge_type": proposed_edge_type,
        "distance_km": distance_km,
        "model_id": request.model_id,
    }));
    InferredEdgeCandidate {
        candidate_id,
        tenant_id: request.tenant_id.clone(),
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        proposed_edge_type: proposed_edge_type.to_string(),
        confidence,
        confidence_ceiling: request.confidence_ceiling,
        admission_tier: request.admission_tier.clone(),
        model_id: request.model_id.clone(),
        support_path_edge_ids: Vec::new(),
        support_path_node_ids,
    }
}

fn make_temporal_candidate(
    request: &DensificationRequest,
    source_id: &str,
    target_id: &str,
    proposed_edge_type: &str,
    left: &TemporalInterval,
    right: &TemporalInterval,
    confidence: f32,
) -> InferredEdgeCandidate {
    let support_path_node_ids = vec![source_id.to_string(), target_id.to_string()];
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "source_id": source_id,
        "target_id": target_id,
        "proposed_edge_type": proposed_edge_type,
        "left_start_ms": left.start_ms,
        "left_end_ms": left.end_ms,
        "right_start_ms": right.start_ms,
        "right_end_ms": right.end_ms,
        "model_id": request.model_id,
    }));
    InferredEdgeCandidate {
        candidate_id,
        tenant_id: request.tenant_id.clone(),
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        proposed_edge_type: proposed_edge_type.to_string(),
        confidence,
        confidence_ceiling: request.confidence_ceiling,
        admission_tier: request.admission_tier.clone(),
        model_id: request.model_id.clone(),
        support_path_edge_ids: Vec::new(),
        support_path_node_ids,
    }
}

fn make_property_candidate(
    request: &DensificationRequest,
    target_node_id: &str,
    property_key: &str,
    proposed_value: Value,
    confidence: f32,
    support_edge_ids: Vec<String>,
    support_node_ids: Vec<String>,
) -> InferredPropertyCandidate {
    let candidate_id = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "target_node_id": target_node_id,
        "property_key": property_key,
        "proposed_value": proposed_value,
        "support_edge_ids": support_edge_ids,
        "support_node_ids": support_node_ids,
        "model_id": request.model_id,
    }));
    InferredPropertyCandidate {
        candidate_id,
        tenant_id: request.tenant_id.clone(),
        target_node_id: target_node_id.to_string(),
        property_key: property_key.to_string(),
        proposed_value,
        confidence,
        confidence_ceiling: request.confidence_ceiling,
        admission_tier: request.admission_tier.clone(),
        model_id: request.model_id.clone(),
        support_edge_ids,
        support_node_ids,
    }
}

fn sort_edge_candidates(candidates: &mut [InferredEdgeCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.admission_tier.cmp(&right.admission_tier))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
}

fn sort_property_candidates(candidates: &mut [InferredPropertyCandidate]) {
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.admission_tier.cmp(&right.admission_tier))
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
}

fn existing_direct_edge_keys(snapshot: &GraphSnapshot) -> BTreeSet<(String, String, String)> {
    snapshot
        .edges
        .iter()
        .filter(|edge| !edge.tombstone)
        .map(|edge| {
            (
                edge.from_id.clone(),
                edge.to_id.clone(),
                edge.edge_type.clone(),
            )
        })
        .collect()
}

pub(crate) fn node_features(node: &NodeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&node.properties, "embedding");
    features.extend(numeric_array_property(&node.properties, "features"));
    features.extend(geo_features(&node.properties));
    features.extend(temporal_features(&node.properties));
    features.push(node.labels.len() as f32);
    features.push(node.version as f32 / 1024.0);
    features
}

pub(crate) fn edge_features(edge: &EdgeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&edge.properties, "embedding");
    features.extend(numeric_array_property(&edge.properties, "features"));
    features.extend(geo_features(&edge.properties));
    features.extend(temporal_features(&edge.properties));
    features.push(edge.effective_confidence() as f32);
    features.push((edge.edge_type.len() as f32).ln_1p());
    features
}

#[derive(Clone, Copy, Debug)]
struct GeoPoint {
    lat: f64,
    lon: f64,
}

#[derive(Clone, Copy, Debug)]
struct TemporalInterval {
    start_ms: i64,
    end_ms: i64,
    created_ms: Option<i64>,
}

fn geo_features(properties: &Value) -> Vec<f32> {
    let Some(point) = geo_point(properties) else {
        return Vec::new();
    };
    let mut features = vec![
        (point.lat / 90.0).clamp(-1.0, 1.0) as f32,
        (point.lon / 180.0).clamp(-1.0, 1.0) as f32,
    ];
    if let Some(token) = spatial_cell_token(point.lat, point.lon) {
        features.push(fold_categorical_feature(&token));
    }
    features
}

fn temporal_features(properties: &Value) -> Vec<f32> {
    let Some(interval) = temporal_interval(properties) else {
        return Vec::new();
    };
    let now = now_ms();
    let start_age_ms = now.saturating_sub(interval.start_ms);
    let created_age_ms = interval
        .created_ms
        .map(|created| now.saturating_sub(created))
        .unwrap_or(start_age_ms);
    let thirty_days_ms = 30.0 * 24.0 * 60.0 * 60.0 * 1000.0;
    let ten_years_ms = 10.0 * 365.0 * 24.0 * 60.0 * 60.0 * 1000.0;
    let recency = 1.0 / (1.0 + start_age_ms as f64 / thirty_days_ms);
    let age = (created_age_ms as f64 / ten_years_ms).clamp(0.0, 1.0);
    let duration = interval.end_ms.saturating_sub(interval.start_ms).max(0);
    let duration_feature = (duration as f64 / ten_years_ms).clamp(0.0, 1.0);
    vec![recency as f32, age as f32, duration_feature as f32]
}

fn geo_point(properties: &Value) -> Option<GeoPoint> {
    let pairs = [
        ("latitude", "longitude"),
        ("lat", "lon"),
        ("lat", "lng"),
        ("y", "x"),
    ];
    for (lat_key, lon_key) in pairs {
        let Some(lat) = numeric_property(properties, lat_key) else {
            continue;
        };
        let Some(lon) = numeric_property(properties, lon_key) else {
            continue;
        };
        if lat.is_finite()
            && lon.is_finite()
            && (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon)
        {
            return Some(GeoPoint { lat, lon });
        }
    }
    properties.get("geo").and_then(|geo| {
        let lat = numeric_property(geo, "lat").or_else(|| numeric_property(geo, "latitude"))?;
        let lon = numeric_property(geo, "lon")
            .or_else(|| numeric_property(geo, "lng"))
            .or_else(|| numeric_property(geo, "longitude"))?;
        if lat.is_finite()
            && lon.is_finite()
            && (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon)
        {
            Some(GeoPoint { lat, lon })
        } else {
            None
        }
    })
}

fn spatial_cell_token(lat: f64, lon: f64) -> Option<String> {
    let mut index = SpatialIndex::for_designation(SpatialDesignation {
        label: "ReflexiveSpatialFeature".to_string(),
        lat_property: "lat".to_string(),
        lon_property: "lon".to_string(),
        resolution: DEFAULT_SPATIAL_RESOLUTION,
    });
    index
        .upsert("__feature__", lat, lon)
        .ok()
        .map(|cell| format!("{cell:?}"))
}

fn temporal_interval(properties: &Value) -> Option<TemporalInterval> {
    let start = numeric_i64_property_any(
        properties,
        &["t_valid", "valid_from_ms", "valid_start_ms", "valid_from"],
    )?;
    let end = numeric_i64_property_any(
        properties,
        &[
            "t_invalid",
            "valid_to_ms",
            "valid_end_ms",
            "valid_until_ms",
            "invalid_at_ms",
        ],
    )
    .unwrap_or(start);
    let created_ms = numeric_i64_property_any(
        properties,
        &["t_created", "created_at_ms", "created_ms", "created_at"],
    );
    Some(TemporalInterval {
        start_ms: start.min(end),
        end_ms: end.max(start),
        created_ms,
    })
}

fn temporal_relation(
    seed_id: &str,
    seed: &TemporalInterval,
    other_id: &str,
    other: &TemporalInterval,
) -> Option<(String, String, String, f32)> {
    if intervals_overlap(seed, other) {
        let confidence = temporal_overlap_confidence(seed, other);
        return Some((
            seed_id.to_string(),
            other_id.to_string(),
            "CONCURRENT".to_string(),
            confidence,
        ));
    }
    if seed.end_ms <= other.start_ms {
        let gap = other.start_ms.saturating_sub(seed.end_ms);
        if gap <= DEFAULT_TEMPORAL_WINDOW_MS {
            let confidence = 1.0 - gap as f32 / DEFAULT_TEMPORAL_WINDOW_MS as f32;
            return Some((
                seed_id.to_string(),
                other_id.to_string(),
                "PRECEDES".to_string(),
                confidence,
            ));
        }
    }
    if other.end_ms <= seed.start_ms {
        let gap = seed.start_ms.saturating_sub(other.end_ms);
        if gap <= DEFAULT_TEMPORAL_WINDOW_MS {
            let confidence = 1.0 - gap as f32 / DEFAULT_TEMPORAL_WINDOW_MS as f32;
            return Some((
                other_id.to_string(),
                seed_id.to_string(),
                "PRECEDES".to_string(),
                confidence,
            ));
        }
    }
    None
}

fn intervals_overlap(left: &TemporalInterval, right: &TemporalInterval) -> bool {
    left.start_ms <= right.end_ms && right.start_ms <= left.end_ms
}

fn temporal_overlap_confidence(left: &TemporalInterval, right: &TemporalInterval) -> f32 {
    let overlap = left.end_ms.min(right.end_ms) - left.start_ms.max(right.start_ms);
    if overlap <= 0 {
        return 1.0;
    }
    let left_duration = (left.end_ms - left.start_ms).max(1);
    let right_duration = (right.end_ms - right.start_ms).max(1);
    (overlap as f32 / left_duration.min(right_duration) as f32).clamp(0.0, 1.0)
}

fn temporal_edge_timestamp_for_hot(edge: &EdgeRecord) -> Option<i64> {
    numeric_i64_property_any(
        &edge.properties,
        &[
            "timestamp_ms",
            "ts_ms",
            "t_valid",
            "valid_from_ms",
            "t_created",
            "created_at_ms",
            "created_ms",
            "timestamp",
        ],
    )
    .or_else(|| {
        edge.provenance
            .as_ref()
            .and_then(|provenance| provenance.timestamp.as_ref())
            .and_then(|raw| raw.parse::<i64>().ok())
    })
    .or(Some(edge.version as i64))
}

fn numeric_property(properties: &Value, key: &str) -> Option<f64> {
    properties.get(key).and_then(|value| {
        value
            .as_f64()
            .or_else(|| value.as_str().and_then(|raw| raw.parse::<f64>().ok()))
    })
}

fn numeric_i64_property_any(properties: &Value, keys: &[&str]) -> Option<i64> {
    keys.iter().find_map(|key| {
        properties.get(*key).and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|raw| i64::try_from(raw).ok()))
                .or_else(|| value.as_f64().map(|raw| raw as i64))
                .or_else(|| value.as_str().and_then(|raw| raw.parse::<i64>().ok()))
        })
    })
}

fn fold_categorical_feature(token: &str) -> f32 {
    let hash = stable_hash(json!({ "categorical": token }));
    let mut acc = 0_u32;
    for byte in hash.bytes().take(8) {
        acc = acc.wrapping_mul(31).wrapping_add(u32::from(byte));
    }
    acc as f32 / u32::MAX as f32
}

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let r_km = 6371.0_f64;
    let to_rad = std::f64::consts::PI / 180.0;
    let dlat = (lat2 - lat1) * to_rad;
    let dlon = (lon2 - lon1) * to_rad;
    let a = (dlat / 2.0).sin().powi(2)
        + (lat1 * to_rad).cos() * (lat2 * to_rad).cos() * (dlon / 2.0).sin().powi(2);
    2.0 * r_km * a.sqrt().asin()
}

fn labels_overlap(left: &NodeRecord, right: &NodeRecord) -> bool {
    left.labels.iter().any(|label| right.labels.contains(label))
}

fn node_has_property(node: &NodeRecord, key: &str) -> bool {
    node.properties
        .as_object()
        .and_then(|properties| properties.get(key))
        .is_some_and(|value| !value.is_null())
}

fn scalar_value(value: &Value) -> bool {
    value.is_string() || value.is_boolean() || value.is_number()
}

fn ignored_property_key(key: &str) -> bool {
    matches!(
        key,
        "id" | "tenant_id"
            | "embedding"
            | "features"
            | "source"
            | "actor"
            | "lat"
            | "lon"
            | "lng"
            | "latitude"
            | "longitude"
            | "x"
            | "y"
            | "t_valid"
            | "t_invalid"
            | "t_created"
            | "t_expired"
            | "created_at_ms"
            | "updated_at_ms"
            | "created_ms"
            | "valid_from_ms"
            | "valid_to_ms"
            | "valid_start_ms"
            | "valid_end_ms"
    )
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

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let width = left.len().min(right.len());
    let mut dot = 0.0;
    let mut left_norm = 0.0;
    let mut right_norm = 0.0;
    for idx in 0..width {
        dot += left[idx] * right[idx];
        left_norm += left[idx] * left[idx];
        right_norm += right[idx] * right[idx];
    }
    if left_norm <= f32::EPSILON || right_norm <= f32::EPSILON {
        0.0
    } else {
        dot / (left_norm.sqrt() * right_norm.sqrt())
    }
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
