//! Sparse EdgeMPNN global completion scorer (NBFNet-style).
//!
//! The dense Pairformer runs only on bounded extracted subgraphs. Global
//! completion over the whole graph goes through this sparse path instead:
//! per-seed generalized Bellman-Ford message passing over COO edges, with
//! iteration and frontier caps, parent-pointer provenance, and advisory
//! candidate output that feeds the same quarantine pipeline as densification.
//! The learned part ranks within the enumerated reachable space; it never
//! authors edges.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::json;

use rustyred_thg_core::{stable_hash, GraphSnapshot, ThgError, ThgResult};
use rustyred_thg_ml::{
    choose_scatter_aggregation_path, FixedPointAggregator, MessageAggregator,
    ScatterAggregationPath, ScatterAggregationRequest,
};

use crate::reflexive::InferredEdgeCandidate;

pub const DEFAULT_COMPLETION_HIDDEN_DIM: usize = 16;
pub const DEFAULT_COMPLETION_LAYERS: usize = 3;
pub const DEFAULT_COMPLETION_MAX_FRONTIER_NODES: usize = 4_096;
pub const DEFAULT_COMPLETION_MAX_SEEDS: usize = 16;
pub const DEFAULT_COMPLETION_MAX_CANDIDATES: usize = 64;
pub const DEFAULT_COMPLETION_CONFIDENCE_CEILING: f32 = 0.74;
const ACTIVATION_EPSILON: f32 = 1e-4;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GlobalCompletionConfig {
    pub hidden_dim: usize,
    pub layers: usize,
    pub max_frontier_nodes: usize,
    pub max_seeds: usize,
    pub score_bias: f32,
    pub score_scale: f32,
}

impl Default for GlobalCompletionConfig {
    fn default() -> Self {
        Self {
            hidden_dim: DEFAULT_COMPLETION_HIDDEN_DIM,
            layers: DEFAULT_COMPLETION_LAYERS,
            max_frontier_nodes: DEFAULT_COMPLETION_MAX_FRONTIER_NODES,
            max_seeds: DEFAULT_COMPLETION_MAX_SEEDS,
            score_bias: -1.4,
            score_scale: 3.2,
        }
    }
}

impl GlobalCompletionConfig {
    pub fn normalized(mut self) -> Self {
        if self.hidden_dim == 0 {
            self.hidden_dim = DEFAULT_COMPLETION_HIDDEN_DIM;
        }
        if self.layers == 0 {
            self.layers = 1;
        }
        if self.max_frontier_nodes == 0 {
            self.max_frontier_nodes = DEFAULT_COMPLETION_MAX_FRONTIER_NODES;
        }
        if self.max_seeds == 0 {
            self.max_seeds = DEFAULT_COMPLETION_MAX_SEEDS;
        }
        if !self.score_bias.is_finite() {
            self.score_bias = -1.4;
        }
        if !self.score_scale.is_finite() || self.score_scale <= 0.0 {
            self.score_scale = 3.2;
        }
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GlobalCompletionRequest {
    pub tenant_id: String,
    pub seed_node_ids: Vec<String>,
    pub min_path_confidence: f32,
    pub confidence_threshold: f32,
    pub confidence_ceiling: f32,
    pub max_candidates: usize,
    pub admission_tier: String,
    pub model_id: String,
    pub allowed_edge_types: Vec<String>,
}

impl GlobalCompletionRequest {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = self.tenant_id.trim().to_string();
        self.seed_node_ids = self
            .seed_node_ids
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        self.min_path_confidence = self.min_path_confidence.clamp(0.0, 1.0);
        self.confidence_threshold = self.confidence_threshold.clamp(0.0, 1.0);
        if self.confidence_ceiling <= 0.0 {
            self.confidence_ceiling = DEFAULT_COMPLETION_CONFIDENCE_CEILING;
        }
        self.confidence_ceiling = self.confidence_ceiling.clamp(0.0, 1.0);
        if self.max_candidates == 0 {
            self.max_candidates = DEFAULT_COMPLETION_MAX_CANDIDATES;
        }
        self.admission_tier = self.admission_tier.trim().to_string();
        if self.admission_tier.is_empty() {
            self.admission_tier = "advisory_inferred".to_string();
        }
        self.model_id = self.model_id.trim().to_string();
        if self.model_id.is_empty() {
            self.model_id = "edge-mpnn-completion/det-v1".to_string();
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
pub struct GlobalCompletionResult {
    pub tenant_id: String,
    pub seeds_run: usize,
    pub layers_run: usize,
    pub frontier_capped: bool,
    pub aggregation_path: ScatterAggregationPath,
    pub aggregator_id: String,
    pub candidates: Vec<InferredEdgeCandidate>,
}

struct SparseGraph {
    node_ids: Vec<String>,
    node_index: BTreeMap<String, usize>,
    // COO arrays, aligned by edge slot.
    edge_src: Vec<usize>,
    edge_dst: Vec<usize>,
    edge_type_idx: Vec<usize>,
    edge_confidence: Vec<f32>,
    edge_ids: Vec<String>,
    relation_types: Vec<String>,
}

fn build_sparse_graph(snapshot: &GraphSnapshot, request: &GlobalCompletionRequest) -> SparseGraph {
    let node_ids = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let node_index = node_ids
        .iter()
        .enumerate()
        .map(|(idx, node_id)| (node_id.clone(), idx))
        .collect::<BTreeMap<_, _>>();
    let allowed = request
        .allowed_edge_types
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();

    let mut relation_index = BTreeMap::new();
    let mut relation_types = Vec::new();
    let mut edge_src = Vec::new();
    let mut edge_dst = Vec::new();
    let mut edge_type_idx = Vec::new();
    let mut edge_confidence = Vec::new();
    let mut edge_ids = Vec::new();

    for edge in &snapshot.edges {
        if edge.tombstone {
            continue;
        }
        let confidence = edge.effective_confidence() as f32;
        if confidence < request.min_path_confidence {
            continue;
        }
        if !allowed.is_empty() && !allowed.contains(&edge.edge_type) {
            continue;
        }
        let (Some(&src), Some(&dst)) = (node_index.get(&edge.from_id), node_index.get(&edge.to_id))
        else {
            continue;
        };
        let rel_idx = *relation_index
            .entry(edge.edge_type.clone())
            .or_insert_with(|| {
                relation_types.push(edge.edge_type.clone());
                relation_types.len() - 1
            });
        edge_src.push(src);
        edge_dst.push(dst);
        edge_type_idx.push(rel_idx);
        edge_confidence.push(confidence.clamp(0.0, 1.0));
        edge_ids.push(edge.id.clone());
    }

    SparseGraph {
        node_ids,
        node_index,
        edge_src,
        edge_dst,
        edge_type_idx,
        edge_confidence,
        edge_ids,
        relation_types,
    }
}

/// Per-node provenance: the strongest incoming edge slot that activated this
/// node, recorded the first time the node lights up and upgraded only by a
/// strictly stronger message. Walking parents from a target back toward the
/// seed yields the support path.
#[derive(Clone, Copy)]
struct ParentPointer {
    edge_slot: usize,
    strength: f32,
}

pub fn rank_global_completion_candidates(
    snapshot: &GraphSnapshot,
    request: GlobalCompletionRequest,
    config: GlobalCompletionConfig,
    aggregator: &dyn MessageAggregator,
) -> ThgResult<GlobalCompletionResult> {
    let request = request.normalized();
    let config = config.normalized();
    if request.seed_node_ids.is_empty() {
        return Err(ThgError::new(
            "invalid_global_completion",
            "at least one seed node id is required",
        ));
    }

    let graph = build_sparse_graph(snapshot, &request);
    let num_nodes = graph.node_ids.len();
    let relation_embeddings = graph
        .relation_types
        .iter()
        .map(|edge_type| seeded_vector(&format!("nbfnet_rel:{edge_type}"), config.hidden_dim))
        .collect::<Vec<_>>();
    let self_gate = seeded_vector("nbfnet_self_gate", config.hidden_dim);
    let agg_gate = seeded_vector("nbfnet_agg_gate", config.hidden_dim);
    let readout = seeded_vector("nbfnet_readout", config.hidden_dim);
    let query_embedding = seeded_vector("nbfnet_completion_query", config.hidden_dim);

    let seeds = request
        .seed_node_ids
        .iter()
        .filter_map(|seed| graph.node_index.get(seed).copied())
        .take(config.max_seeds)
        .collect::<Vec<_>>();
    let seeds_run = seeds.len();

    // Existing direct pairs are suppressed from candidate output: completion
    // proposes only edges the graph does not already materialize.
    let direct_pairs = graph
        .edge_src
        .iter()
        .zip(&graph.edge_dst)
        .map(|(src, dst)| (*src, *dst))
        .collect::<std::collections::BTreeSet<_>>();

    let mut frontier_capped = false;
    let mut policy_path = None;
    let mut by_key: BTreeMap<(String, String, String), InferredEdgeCandidate> = BTreeMap::new();

    for &seed_idx in &seeds {
        let mut hidden = vec![vec![0.0_f32; config.hidden_dim]; num_nodes];
        hidden[seed_idx] = query_embedding.clone();
        let mut parents: Vec<Option<ParentPointer>> = vec![None; num_nodes];

        for _ in 0..config.layers {
            // Sparse frontier: only nodes with signal emit messages.
            let mut active = (0..num_nodes)
                .filter(|idx| l1_norm(&hidden[*idx]) > ACTIVATION_EPSILON)
                .collect::<Vec<_>>();
            if active.is_empty() {
                break;
            }
            if active.len() > config.max_frontier_nodes {
                frontier_capped = true;
                active.sort_by(|left, right| {
                    l1_norm(&hidden[*right])
                        .partial_cmp(&l1_norm(&hidden[*left]))
                        .unwrap_or(Ordering::Equal)
                        .then_with(|| left.cmp(right))
                });
                active.truncate(config.max_frontier_nodes);
            }
            let active_set = active
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();

            let mut messages = Vec::new();
            let mut message_dst = Vec::new();
            let mut message_edge_slot = Vec::new();
            for slot in 0..graph.edge_src.len() {
                let src = graph.edge_src[slot];
                if !active_set.contains(&src) {
                    continue;
                }
                let rel = &relation_embeddings[graph.edge_type_idx[slot]];
                let confidence = graph.edge_confidence[slot];
                let message = hidden[src]
                    .iter()
                    .zip(rel)
                    .map(|(state, relation)| state * relation * confidence)
                    .collect::<Vec<_>>();
                messages.push(message);
                message_dst.push(graph.edge_dst[slot]);
                message_edge_slot.push(slot);
            }
            if messages.is_empty() {
                break;
            }

            // The aggregation policy is consulted per layer with the real
            // message volume; the deterministic contract requires the
            // portable path whenever the volume leaves the Burn-native zone.
            let path = choose_scatter_aggregation_path(ScatterAggregationRequest {
                num_edges: messages.len(),
                feature_dim: config.hidden_dim,
                deterministic_required: true,
                browser_webgpu_target: false,
                float_atomic_add_available: false,
                burn_native_max_elements: 0,
            });
            policy_path.get_or_insert(path);

            for (message, slot) in messages.iter().zip(&message_edge_slot) {
                let strength = l1_norm(message);
                if strength <= ACTIVATION_EPSILON {
                    continue;
                }
                let dst = graph.edge_dst[*slot];
                if dst == seed_idx {
                    continue;
                }
                match parents[dst] {
                    Some(existing) if existing.strength >= strength => {}
                    _ => {
                        parents[dst] = Some(ParentPointer {
                            edge_slot: *slot,
                            strength,
                        });
                    }
                }
            }

            let aggregated = aggregator.aggregate(&messages, &message_dst, num_nodes, false)?;
            // Sparse update: only message destinations and already-active
            // nodes change state; everything else stays zero untouched.
            let mut touched = message_dst
                .iter()
                .copied()
                .collect::<std::collections::BTreeSet<_>>();
            touched.extend(active_set.iter().copied());
            for node_idx in touched {
                if node_idx == seed_idx {
                    continue;
                }
                let mut updated = vec![0.0_f32; config.hidden_dim];
                let mut any_signal = false;
                for dim_idx in 0..config.hidden_dim {
                    let blended = self_gate[dim_idx] * hidden[node_idx][dim_idx]
                        + agg_gate[dim_idx] * aggregated[node_idx][dim_idx];
                    let activated = blended.tanh();
                    if activated.abs() > ACTIVATION_EPSILON {
                        any_signal = true;
                    }
                    updated[dim_idx] = activated;
                }
                if any_signal || l1_norm(&hidden[node_idx]) > ACTIVATION_EPSILON {
                    hidden[node_idx] = updated;
                }
            }
        }

        let seed_id = &graph.node_ids[seed_idx];
        for (target_idx, target_hidden) in hidden.iter().enumerate().take(num_nodes) {
            if target_idx == seed_idx {
                continue;
            }
            if direct_pairs.contains(&(seed_idx, target_idx)) {
                continue;
            }
            if l1_norm(target_hidden) <= ACTIVATION_EPSILON {
                continue;
            }
            let Some(support) =
                walk_support_path(&graph, &parents, seed_idx, target_idx, config.layers)
            else {
                continue;
            };
            let activation = sigmoid(
                config.score_bias + config.score_scale * dot(target_hidden, &readout).abs(),
            );
            let raw_confidence = (activation * support.confidence).clamp(0.0, 1.0);
            let confidence = raw_confidence.min(request.confidence_ceiling);
            if confidence < request.confidence_threshold {
                continue;
            }
            let target_id = &graph.node_ids[target_idx];
            let candidate_id = stable_hash(json!({
                "tenant_id": request.tenant_id,
                "source_id": seed_id,
                "target_id": target_id,
                "proposed_edge_type": support.relation_hint,
                "support_path_edge_ids": support.edge_ids,
                "model_id": request.model_id,
            }));
            let candidate = InferredEdgeCandidate {
                candidate_id,
                tenant_id: request.tenant_id.clone(),
                source_id: seed_id.clone(),
                target_id: target_id.clone(),
                proposed_edge_type: support.relation_hint.clone(),
                confidence,
                confidence_ceiling: request.confidence_ceiling,
                admission_tier: request.admission_tier.clone(),
                model_id: request.model_id.clone(),
                support_path_edge_ids: support.edge_ids,
                support_path_node_ids: support.node_ids,
            };
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

    Ok(GlobalCompletionResult {
        tenant_id: request.tenant_id,
        seeds_run,
        layers_run: config.layers,
        frontier_capped,
        aggregation_path: policy_path.unwrap_or(ScatterAggregationPath::FixedPointAtomicCompatible),
        aggregator_id: aggregator.aggregator_id().to_string(),
        candidates,
    })
}

pub fn rank_global_completion_candidates_default(
    snapshot: &GraphSnapshot,
    request: GlobalCompletionRequest,
    config: GlobalCompletionConfig,
) -> ThgResult<GlobalCompletionResult> {
    rank_global_completion_candidates(snapshot, request, config, &FixedPointAggregator::default())
}

struct SupportChain {
    edge_ids: Vec<String>,
    node_ids: Vec<String>,
    relation_hint: String,
    confidence: f32,
}

fn walk_support_path(
    graph: &SparseGraph,
    parents: &[Option<ParentPointer>],
    seed_idx: usize,
    target_idx: usize,
    max_steps: usize,
) -> Option<SupportChain> {
    let mut edge_slots = Vec::new();
    let mut cursor = target_idx;
    for _ in 0..max_steps {
        let pointer = parents[cursor]?;
        let slot = pointer.edge_slot;
        edge_slots.push(slot);
        cursor = graph.edge_src[slot];
        if cursor == seed_idx {
            edge_slots.reverse();
            let mut node_ids = Vec::with_capacity(edge_slots.len() + 1);
            node_ids.push(graph.node_ids[seed_idx].clone());
            let mut confidence = 1.0_f32;
            for slot in &edge_slots {
                node_ids.push(graph.node_ids[graph.edge_dst[*slot]].clone());
                confidence *= graph.edge_confidence[*slot];
            }
            let confidence = confidence.powf(1.0 / edge_slots.len().max(1) as f32);
            let first_type = &graph.relation_types[graph.edge_type_idx[edge_slots[0]]];
            let last_type =
                &graph.relation_types[graph.edge_type_idx[edge_slots[edge_slots.len() - 1]]];
            let relation_hint = if first_type == last_type {
                format!("INFERRED_{first_type}")
            } else {
                format!("INFERRED_{first_type}_THEN_{last_type}")
            };
            return Some(SupportChain {
                edge_ids: edge_slots
                    .iter()
                    .map(|slot| graph.edge_ids[*slot].clone())
                    .collect(),
                node_ids,
                relation_hint,
                confidence,
            });
        }
    }
    None
}

fn seeded_vector(seed: &str, dim: usize) -> Vec<f32> {
    (0..dim)
        .map(|idx| {
            let digest = stable_hash(json!({ "seed": seed, "idx": idx }));
            let hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
            let chunk = hex.get(0..8).unwrap_or("00000000");
            let value = u32::from_str_radix(chunk, 16).unwrap_or_default();
            value as f32 / u32::MAX as f32 * 2.0 - 1.0
        })
        .collect()
}

fn l1_norm(values: &[f32]) -> f32 {
    values.iter().map(|value| value.abs()).sum()
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-80.0, 80.0)).exp())
}
