//! Bounded Pairformer inference core for reflexive graph scoring.
//!
//! This module implements the complete block structure needed by the
//! Reflexive RustyRed plan while keeping graph mutation outside the model:
//! pair representations are updated by triangle operations, singles are
//! updated separately with pair bias, and outputs are advisory link scores.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use rustyred_thg_core::{stable_hash, ThgError, ThgResult};

pub const DEFAULT_PAIRFORMER_PAIR_DIM: usize = 16;
pub const DEFAULT_PAIRFORMER_SINGLE_DIM: usize = 16;
pub const DEFAULT_PAIRFORMER_BLOCKS: usize = 4;
pub const DEFAULT_PAIRFORMER_TRANSITION_HIDDEN_DIM: usize = 32;
pub const DEFAULT_PAIRFORMER_MAX_NODES: usize = 128;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerConfig {
    pub pair_dim: usize,
    pub single_dim: usize,
    pub blocks: usize,
    pub transition_hidden_dim: usize,
    pub max_nodes: usize,
    pub triangle_update_scale: f32,
    pub triangle_attention_scale: f32,
    pub transition_scale: f32,
    pub single_update_scale: f32,
    pub link_score_bias: f32,
    pub link_score_scale: f32,
    pub residual_clamp: f32,
}

impl Default for PairformerConfig {
    fn default() -> Self {
        Self {
            pair_dim: DEFAULT_PAIRFORMER_PAIR_DIM,
            single_dim: DEFAULT_PAIRFORMER_SINGLE_DIM,
            blocks: DEFAULT_PAIRFORMER_BLOCKS,
            transition_hidden_dim: DEFAULT_PAIRFORMER_TRANSITION_HIDDEN_DIM,
            max_nodes: DEFAULT_PAIRFORMER_MAX_NODES,
            triangle_update_scale: 0.55,
            triangle_attention_scale: 0.18,
            transition_scale: 0.08,
            single_update_scale: 0.20,
            link_score_bias: -1.05,
            link_score_scale: 4.0,
            residual_clamp: 8.0,
        }
    }
}

impl PairformerConfig {
    pub fn normalized(mut self) -> Self {
        if self.pair_dim == 0 {
            self.pair_dim = DEFAULT_PAIRFORMER_PAIR_DIM;
        }
        if self.single_dim == 0 {
            self.single_dim = DEFAULT_PAIRFORMER_SINGLE_DIM;
        }
        if self.blocks == 0 {
            self.blocks = 1;
        }
        if self.transition_hidden_dim == 0 {
            self.transition_hidden_dim = self.pair_dim.max(self.single_dim).max(1) * 2;
        }
        if self.max_nodes == 0 {
            self.max_nodes = DEFAULT_PAIRFORMER_MAX_NODES;
        }
        if self.residual_clamp <= 0.0 || !self.residual_clamp.is_finite() {
            self.residual_clamp = 8.0;
        }
        self.triangle_update_scale = finite_or(self.triangle_update_scale, 0.55);
        self.triangle_attention_scale = finite_or(self.triangle_attention_scale, 0.18);
        self.transition_scale = finite_or(self.transition_scale, 0.08);
        self.single_update_scale = finite_or(self.single_update_scale, 0.20);
        self.link_score_bias = finite_or(self.link_score_bias, -1.05);
        self.link_score_scale = finite_or(self.link_score_scale, 4.0);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerNodeInput {
    pub node_id: String,
    pub features: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerEdgeInput {
    pub edge_id: String,
    pub source_id: String,
    pub target_id: String,
    pub edge_type: String,
    pub features: Vec<f32>,
    pub confidence: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerInput {
    pub nodes: Vec<PairformerNodeInput>,
    pub edges: Vec<PairformerEdgeInput>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerPairRepresentation {
    pub source_id: String,
    pub target_id: String,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerSupportPath {
    pub edge_ids: Vec<String>,
    pub node_ids: Vec<String>,
    pub relation_hint: String,
    pub confidence: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerLinkScore {
    pub source_id: String,
    pub target_id: String,
    pub score: f32,
    pub support_path: Option<PairformerSupportPath>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerOutput {
    pub node_ids: Vec<String>,
    pub single_representations: Vec<Vec<f32>>,
    pub pair_representations: Vec<PairformerPairRepresentation>,
    pub link_scores: Vec<PairformerLinkScore>,
}

type PairGrid = Vec<Vec<Vec<f32>>>;

pub fn run_pairformer(
    input: &PairformerInput,
    config: PairformerConfig,
) -> ThgResult<PairformerOutput> {
    let config = config.normalized();
    validate_pairformer_input(input, &config)?;

    let node_ids = input
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    let node_index = node_ids
        .iter()
        .enumerate()
        .map(|(idx, node_id)| (node_id.clone(), idx))
        .collect::<BTreeMap<_, _>>();
    let mut single = input
        .nodes
        .iter()
        .map(|node| seed_vector(&node.node_id, &node.features, config.single_dim, 0.25))
        .collect::<Vec<_>>();
    let mut pair = initialize_pair_grid(input, &node_index, &single, &config);

    for _ in 0..config.blocks {
        triangle_multiplicative_update_outgoing(&mut pair, &config);
        triangle_multiplicative_update_incoming(&mut pair, &config);
        triangle_attention_starting_node(&mut pair, &config);
        triangle_attention_ending_node(&mut pair, &config);
        pair_transition(&mut pair, &config);
        single = single_attention_with_pair_bias(&single, &pair, &config);
    }

    let support_paths = best_two_hop_support_paths(input);
    let pair_representations = flatten_pair_representations(&node_ids, &pair);
    let link_scores = score_links(&node_ids, &pair, &support_paths, &config);

    Ok(PairformerOutput {
        node_ids,
        single_representations: single,
        pair_representations,
        link_scores,
    })
}

fn validate_pairformer_input(input: &PairformerInput, config: &PairformerConfig) -> ThgResult<()> {
    if input.nodes.is_empty() {
        return Err(ThgError::new(
            "invalid_pairformer_input",
            "at least one node is required",
        ));
    }
    if input.nodes.len() > config.max_nodes {
        return Err(ThgError::new(
            "pairformer_bound_exceeded",
            format!(
                "Pairformer input has {} nodes, above max_nodes {}",
                input.nodes.len(),
                config.max_nodes
            ),
        ));
    }

    let mut node_ids = BTreeSet::new();
    for node in &input.nodes {
        let node_id = node.node_id.trim();
        if node_id.is_empty() {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                "node_id is required",
            ));
        }
        if !node_ids.insert(node_id.to_string()) {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                format!("duplicate node_id {node_id}"),
            ));
        }
        if node.features.iter().any(|value| !value.is_finite()) {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                format!("node {node_id} has non-finite features"),
            ));
        }
    }

    for edge in &input.edges {
        if edge.edge_id.trim().is_empty()
            || edge.source_id.trim().is_empty()
            || edge.target_id.trim().is_empty()
            || edge.edge_type.trim().is_empty()
        {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                "edge_id, source_id, target_id, and edge_type are required",
            ));
        }
        if !node_ids.contains(&edge.source_id) || !node_ids.contains(&edge.target_id) {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                format!(
                    "edge {} references an endpoint outside the bounded node set",
                    edge.edge_id
                ),
            ));
        }
        if !edge.confidence.is_finite() {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                format!("edge {} has non-finite confidence", edge.edge_id),
            ));
        }
        if edge.features.iter().any(|value| !value.is_finite()) {
            return Err(ThgError::new(
                "invalid_pairformer_input",
                format!("edge {} has non-finite features", edge.edge_id),
            ));
        }
    }

    Ok(())
}

fn initialize_pair_grid(
    input: &PairformerInput,
    node_index: &BTreeMap<String, usize>,
    single: &[Vec<f32>],
    config: &PairformerConfig,
) -> PairGrid {
    let node_count = input.nodes.len();
    let mut pair = vec![vec![vec![0.0_f32; config.pair_dim]; node_count]; node_count];
    for source_idx in 0..node_count {
        for target_idx in 0..node_count {
            let source_id = &input.nodes[source_idx].node_id;
            let target_id = &input.nodes[target_idx].node_id;
            let seed_id = format!("{source_id}->{target_id}");
            pair[source_idx][target_idx] = seed_vector(
                &seed_id,
                &[],
                config.pair_dim,
                if source_idx == target_idx { 0.08 } else { 0.02 },
            );
            if config.pair_dim > 0 {
                pair[source_idx][target_idx][0] +=
                    if source_idx == target_idx { 0.12 } else { 0.0 };
            }
            if config.pair_dim > 2 {
                pair[source_idx][target_idx][2] +=
                    0.05 * cosine_similarity(&single[source_idx], &single[target_idx]);
            }
        }
    }

    for edge in &input.edges {
        let Some(&source_idx) = node_index.get(&edge.source_id) else {
            continue;
        };
        let Some(&target_idx) = node_index.get(&edge.target_id) else {
            continue;
        };
        let projected = seed_vector(&edge.edge_id, &edge.features, config.pair_dim, 0.35);
        let relation_marker = stable_noise(&edge.edge_type, 0) * 0.2;
        let confidence = edge.confidence.clamp(0.0, 1.0);
        for (slot, value) in pair[source_idx][target_idx].iter_mut().zip(projected) {
            *slot += value * confidence.max(0.05);
        }
        pair[source_idx][target_idx][0] += confidence;
        if config.pair_dim > 1 {
            pair[source_idx][target_idx][1] += 1.0;
        }
        if config.pair_dim > 2 {
            pair[source_idx][target_idx][2] += relation_marker;
        }
        clamp_vector(&mut pair[source_idx][target_idx], config.residual_clamp);
    }

    pair
}

fn triangle_multiplicative_update_outgoing(pair: &mut PairGrid, config: &PairformerConfig) {
    let original = pair.clone();
    let node_count = original.len();
    let norm = (node_count as f32).sqrt().max(1.0);
    for source_idx in 0..node_count {
        for target_idx in 0..node_count {
            for dim_idx in 0..config.pair_dim {
                let mut sum = 0.0;
                for pivot_idx in 0..node_count {
                    sum += original[source_idx][pivot_idx][dim_idx]
                        * original[pivot_idx][target_idx][dim_idx];
                }
                pair[source_idx][target_idx][dim_idx] += config.triangle_update_scale * sum / norm;
            }
            clamp_vector(&mut pair[source_idx][target_idx], config.residual_clamp);
        }
    }
}

fn triangle_multiplicative_update_incoming(pair: &mut PairGrid, config: &PairformerConfig) {
    let original = pair.clone();
    let node_count = original.len();
    let norm = (node_count as f32).sqrt().max(1.0);
    for source_idx in 0..node_count {
        for target_idx in 0..node_count {
            for dim_idx in 0..config.pair_dim {
                let mut sum = 0.0;
                for pivot_idx in 0..node_count {
                    sum += original[pivot_idx][source_idx][dim_idx]
                        * original[target_idx][pivot_idx][dim_idx];
                }
                pair[source_idx][target_idx][dim_idx] += config.triangle_update_scale * sum / norm;
            }
            clamp_vector(&mut pair[source_idx][target_idx], config.residual_clamp);
        }
    }
}

fn triangle_attention_starting_node(pair: &mut PairGrid, config: &PairformerConfig) {
    let original = pair.clone();
    let node_count = original.len();
    let norm = (config.pair_dim as f32).sqrt().max(1.0);
    for source_idx in 0..node_count {
        for target_idx in 0..node_count {
            let query = &original[source_idx][target_idx];
            let mut logits = Vec::with_capacity(node_count);
            for pivot_idx in 0..node_count {
                let pair_bias = mean(&original[target_idx][pivot_idx]) * 0.25;
                logits.push(dot(query, &original[source_idx][pivot_idx]) / norm + pair_bias);
            }
            let weights = softmax(&logits);
            let mut attended = vec![0.0; config.pair_dim];
            for pivot_idx in 0..node_count {
                for (slot, value) in attended
                    .iter_mut()
                    .zip(original[source_idx][pivot_idx].iter())
                {
                    *slot += weights[pivot_idx] * value;
                }
            }
            for (slot, value) in pair[source_idx][target_idx].iter_mut().zip(attended) {
                *slot += config.triangle_attention_scale * value;
            }
            clamp_vector(&mut pair[source_idx][target_idx], config.residual_clamp);
        }
    }
}

fn triangle_attention_ending_node(pair: &mut PairGrid, config: &PairformerConfig) {
    let original = pair.clone();
    let node_count = original.len();
    let norm = (config.pair_dim as f32).sqrt().max(1.0);
    for source_idx in 0..node_count {
        for target_idx in 0..node_count {
            let query = &original[source_idx][target_idx];
            let mut logits = Vec::with_capacity(node_count);
            for pivot_idx in 0..node_count {
                let pair_bias = mean(&original[pivot_idx][source_idx]) * 0.25;
                logits.push(dot(query, &original[pivot_idx][target_idx]) / norm + pair_bias);
            }
            let weights = softmax(&logits);
            let mut attended = vec![0.0; config.pair_dim];
            for pivot_idx in 0..node_count {
                for (slot, value) in attended
                    .iter_mut()
                    .zip(original[pivot_idx][target_idx].iter())
                {
                    *slot += weights[pivot_idx] * value;
                }
            }
            for (slot, value) in pair[source_idx][target_idx].iter_mut().zip(attended) {
                *slot += config.triangle_attention_scale * value;
            }
            clamp_vector(&mut pair[source_idx][target_idx], config.residual_clamp);
        }
    }
}

fn pair_transition(pair: &mut PairGrid, config: &PairformerConfig) {
    for row in pair {
        for values in row {
            let delta = swiglu_transition(values, config.pair_dim, config.transition_hidden_dim);
            for (slot, value) in values.iter_mut().zip(delta) {
                *slot += config.transition_scale * value;
            }
            clamp_vector(values, config.residual_clamp);
        }
    }
}

fn single_attention_with_pair_bias(
    single: &[Vec<f32>],
    pair: &PairGrid,
    config: &PairformerConfig,
) -> Vec<Vec<f32>> {
    let node_count = single.len();
    let norm = (config.single_dim as f32).sqrt().max(1.0);
    let mut updated = single.to_vec();
    for source_idx in 0..node_count {
        let query = &single[source_idx];
        let mut logits = Vec::with_capacity(node_count);
        for target_idx in 0..node_count {
            let pair_bias = mean(&pair[source_idx][target_idx]);
            logits.push(dot(query, &single[target_idx]) / norm + pair_bias);
        }
        let weights = softmax(&logits);
        let mut attended = vec![0.0; config.single_dim];
        for target_idx in 0..node_count {
            for (slot, value) in attended.iter_mut().zip(single[target_idx].iter()) {
                *slot += weights[target_idx] * value;
            }
        }
        let transition = swiglu_transition(
            &attended,
            config.single_dim,
            config.transition_hidden_dim.max(config.single_dim),
        );
        for dim_idx in 0..config.single_dim {
            let residual =
                attended[dim_idx] - single[source_idx][dim_idx] + 0.25 * transition[dim_idx];
            updated[source_idx][dim_idx] += config.single_update_scale * residual;
        }
        clamp_vector(&mut updated[source_idx], config.residual_clamp);
    }
    updated
}

pub(crate) fn best_two_hop_support_paths(
    input: &PairformerInput,
) -> BTreeMap<(String, String), PairformerSupportPath> {
    let mut by_source = BTreeMap::<String, Vec<&PairformerEdgeInput>>::new();
    for edge in &input.edges {
        by_source
            .entry(edge.source_id.clone())
            .or_default()
            .push(edge);
    }

    let mut support_paths: BTreeMap<(String, String), PairformerSupportPath> = BTreeMap::new();
    for first in &input.edges {
        let Some(seconds) = by_source.get(&first.target_id) else {
            continue;
        };
        for second in seconds {
            if first.source_id == second.target_id {
                continue;
            }
            let confidence =
                (first.confidence.clamp(0.0, 1.0) * second.confidence.clamp(0.0, 1.0)).sqrt();
            let support = PairformerSupportPath {
                edge_ids: vec![first.edge_id.clone(), second.edge_id.clone()],
                node_ids: vec![
                    first.source_id.clone(),
                    first.target_id.clone(),
                    second.target_id.clone(),
                ],
                relation_hint: normalized_inferred_edge_type(&first.edge_type, &second.edge_type),
                confidence,
            };
            let key = (first.source_id.clone(), second.target_id.clone());
            match support_paths.get(&key) {
                Some(prior) if prior.confidence >= support.confidence => {}
                _ => {
                    support_paths.insert(key, support);
                }
            }
        }
    }
    support_paths
}

fn flatten_pair_representations(
    node_ids: &[String],
    pair: &PairGrid,
) -> Vec<PairformerPairRepresentation> {
    let mut representations = Vec::with_capacity(node_ids.len() * node_ids.len());
    for (source_idx, source_id) in node_ids.iter().enumerate() {
        for (target_idx, target_id) in node_ids.iter().enumerate() {
            representations.push(PairformerPairRepresentation {
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                values: pair[source_idx][target_idx].clone(),
            });
        }
    }
    representations
}

fn score_links(
    node_ids: &[String],
    pair: &PairGrid,
    support_paths: &BTreeMap<(String, String), PairformerSupportPath>,
    config: &PairformerConfig,
) -> Vec<PairformerLinkScore> {
    let mut scores = Vec::new();
    for (source_idx, source_id) in node_ids.iter().enumerate() {
        for (target_idx, target_id) in node_ids.iter().enumerate() {
            if source_idx == target_idx {
                continue;
            }
            let support_path = support_paths
                .get(&(source_id.clone(), target_id.clone()))
                .cloned();
            let support_confidence = support_path
                .as_ref()
                .map(|support| support.confidence.clamp(0.0, 1.0));
            let support_signal = support_confidence.unwrap_or(0.0);
            let values = &pair[source_idx][target_idx];
            let pair_signal = values.first().copied().unwrap_or_default().max(0.0)
                + positive_mean(values) * 0.25
                + values.get(1).copied().unwrap_or_default().max(0.0) * 0.08
                + support_signal;
            let activation =
                sigmoid(config.link_score_bias + config.link_score_scale * pair_signal);
            let score = match support_confidence {
                Some(confidence) => 0.5 + 0.5 * activation * confidence,
                None => 0.5 * activation,
            };
            scores.push(PairformerLinkScore {
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                score,
                support_path,
            });
        }
    }
    scores
}

fn seed_vector(id: &str, features: &[f32], dim: usize, jitter_scale: f32) -> Vec<f32> {
    let mut out = (0..dim)
        .map(|idx| stable_noise(id, idx) * jitter_scale)
        .collect::<Vec<_>>();
    if features.is_empty() {
        return out;
    }

    for (feature_idx, value) in features.iter().enumerate() {
        let normalized = value.tanh();
        let slot = feature_idx % dim;
        let pass = feature_idx / dim + 1;
        out[slot] += normalized / pass as f32;
        let mixed_slot = (feature_idx * 7 + 3) % dim;
        out[mixed_slot] += normalized * stable_noise(id, feature_idx + dim) * 0.25;
    }
    clamp_vector(&mut out, 4.0);
    out
}

fn swiglu_transition(input: &[f32], output_dim: usize, hidden_dim: usize) -> Vec<f32> {
    if output_dim == 0 || input.is_empty() {
        return Vec::new();
    }
    let hidden_dim = hidden_dim.max(output_dim).max(1);
    let norm = (hidden_dim as f32).sqrt().max(1.0);
    let mut hidden = vec![0.0; hidden_dim];
    for hidden_idx in 0..hidden_dim {
        let left = deterministic_projection(input, hidden_idx, 11);
        let gate = deterministic_projection(input, hidden_idx, 37);
        hidden[hidden_idx] = silu(left) * gate;
    }

    let mut out = vec![0.0; output_dim];
    for dim_idx in 0..output_dim {
        let mut sum = 0.0;
        for (hidden_idx, value) in hidden.iter().enumerate() {
            let sign = if (hidden_idx + dim_idx) % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            let weight = sign * (0.5 + stable_noise("swiglu", hidden_idx + dim_idx).abs());
            sum += value * weight;
        }
        out[dim_idx] = sum / norm;
    }
    out
}

fn deterministic_projection(input: &[f32], hidden_idx: usize, salt: usize) -> f32 {
    let len = input.len();
    let a = input[(hidden_idx + salt) % len];
    let b = input[(hidden_idx * 7 + salt * 3) % len];
    let c = input[(hidden_idx * 13 + salt) % len];
    0.5 * a + 0.35 * b - 0.15 * c
}

fn normalized_inferred_edge_type(left: &str, right: &str) -> String {
    if left == right {
        format!("INFERRED_{left}")
    } else {
        format!("INFERRED_{left}_THEN_{right}")
    }
}

fn stable_noise(seed: &str, idx: usize) -> f32 {
    let digest = stable_hash(json!({ "seed": seed, "idx": idx }));
    let hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    let chunk = hex.get(0..8).unwrap_or("00000000");
    let value = u32::from_str_radix(chunk, 16).unwrap_or_default();
    value as f32 / u32::MAX as f32 * 2.0 - 1.0
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max = logits
        .iter()
        .copied()
        .filter(|value| value.is_finite())
        .fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return vec![1.0 / logits.len() as f32; logits.len()];
    }
    let mut weights = logits
        .iter()
        .map(|value| (value - max).exp())
        .collect::<Vec<_>>();
    let sum = weights.iter().copied().sum::<f32>();
    if !sum.is_finite() || sum <= 1e-12 {
        return vec![1.0 / logits.len() as f32; logits.len()];
    }
    for weight in &mut weights {
        *weight /= sum;
    }
    weights
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-80.0, 80.0)).exp())
}

fn silu(value: f32) -> f32 {
    value * sigmoid(value)
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().copied().sum::<f32>() / values.len() as f32
    }
}

fn positive_mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values
            .iter()
            .copied()
            .map(|value| value.max(0.0))
            .sum::<f32>()
            / values.len() as f32
    }
}

fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    let dot_product = dot(left, right);
    let left_norm = dot(left, left);
    let right_norm = dot(right, right);
    if left_norm <= 1e-12 || right_norm <= 1e-12 {
        0.0
    } else {
        dot_product / (left_norm.sqrt() * right_norm.sqrt())
    }
}

fn clamp_vector(values: &mut [f32], clamp: f32) {
    let clamp = clamp.abs().max(1.0);
    for value in values {
        *value = finite_or(*value, 0.0).clamp(-clamp, clamp);
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}
