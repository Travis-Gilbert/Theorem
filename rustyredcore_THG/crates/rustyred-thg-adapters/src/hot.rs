//! Native HOT (Higher-Order Temporal) link scorer for reflexive memory.
//!
//! This is the bounded, deterministic reference lane for SPEC-8.  It mirrors
//! the Pairformer split in this crate: mutation stays outside the model, input
//! extraction is capped, and the output is an advisory link score with a
//! support path that the organizer/admission layer can quarantine or admit.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{stable_hash, EdgeRecord, GraphSnapshot, NodeRecord, ThgError, ThgResult};

use crate::pairformer::PairformerSupportPath;

pub const DEFAULT_HOT_ALIGNED_DIM: usize = 50;
pub const DEFAULT_HOT_TIME_ENCODING_DIM: usize = 100;
pub const DEFAULT_HOT_COOCCURRENCE_DIM: usize = 50;
pub const DEFAULT_HOT_OUTPUT_DIM: usize = 172;
pub const DEFAULT_HOT_HEADS: usize = 4;
pub const DEFAULT_HOT_BRT_CELLS: usize = 2;
pub const DEFAULT_HOT_HORIZONTAL_CELL_INDEX: usize = 1;
pub const DEFAULT_HOT_BLOCK_SIZE: usize = 16;
pub const DEFAULT_HOT_SEGMENT_SIZE: usize = 32;
pub const DEFAULT_HOT_STATE_VECTORS: usize = 32;
pub const DEFAULT_HOT_PATCH_SIZE: usize = 2;
pub const DEFAULT_HOT_S1: usize = 16;
pub const DEFAULT_HOT_S2: usize = 8;
pub const DEFAULT_HOT_MAX_NODES: usize = 128;
pub const DEFAULT_HOT_MAX_TEMPORAL_EDGES: usize = 512;
pub const DEFAULT_HOT_DECODER_HIDDEN_DIM: usize = 64;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotConfig {
    pub aligned_dim: usize,
    pub time_encoding_dim: usize,
    pub cooccurrence_dim: usize,
    pub output_dim: usize,
    pub attention_heads: usize,
    pub brt_cells: usize,
    pub horizontal_cell_index: usize,
    pub brt_block_size: usize,
    pub segment_size: usize,
    pub state_vectors: usize,
    pub patch_size: usize,
    pub s1: usize,
    pub s2: usize,
    pub max_nodes: usize,
    pub max_temporal_edges: usize,
    pub decoder_hidden_dim: usize,
    pub link_score_bias: f32,
    pub link_score_scale: f32,
    pub residual_clamp: f32,
}

impl Default for HotConfig {
    fn default() -> Self {
        Self {
            aligned_dim: DEFAULT_HOT_ALIGNED_DIM,
            time_encoding_dim: DEFAULT_HOT_TIME_ENCODING_DIM,
            cooccurrence_dim: DEFAULT_HOT_COOCCURRENCE_DIM,
            output_dim: DEFAULT_HOT_OUTPUT_DIM,
            attention_heads: DEFAULT_HOT_HEADS,
            brt_cells: DEFAULT_HOT_BRT_CELLS,
            horizontal_cell_index: DEFAULT_HOT_HORIZONTAL_CELL_INDEX,
            brt_block_size: DEFAULT_HOT_BLOCK_SIZE,
            segment_size: DEFAULT_HOT_SEGMENT_SIZE,
            state_vectors: DEFAULT_HOT_STATE_VECTORS,
            patch_size: DEFAULT_HOT_PATCH_SIZE,
            s1: DEFAULT_HOT_S1,
            s2: DEFAULT_HOT_S2,
            max_nodes: DEFAULT_HOT_MAX_NODES,
            max_temporal_edges: DEFAULT_HOT_MAX_TEMPORAL_EDGES,
            decoder_hidden_dim: DEFAULT_HOT_DECODER_HIDDEN_DIM,
            link_score_bias: -0.85,
            link_score_scale: 3.2,
            residual_clamp: 8.0,
        }
    }
}

impl HotConfig {
    pub fn normalized(mut self) -> Self {
        if self.aligned_dim == 0 {
            self.aligned_dim = DEFAULT_HOT_ALIGNED_DIM;
        }
        if self.time_encoding_dim == 0 {
            self.time_encoding_dim = DEFAULT_HOT_TIME_ENCODING_DIM;
        }
        if self.time_encoding_dim % 2 != 0 {
            self.time_encoding_dim += 1;
        }
        if self.cooccurrence_dim == 0 {
            self.cooccurrence_dim = DEFAULT_HOT_COOCCURRENCE_DIM;
        }
        if self.output_dim == 0 {
            self.output_dim = DEFAULT_HOT_OUTPUT_DIM;
        }
        if self.attention_heads == 0 {
            self.attention_heads = 1;
        }
        while self.attention_heads > 1 && self.output_dim % self.attention_heads != 0 {
            self.attention_heads -= 1;
        }
        if self.brt_cells == 0 {
            self.brt_cells = 1;
        }
        if self.horizontal_cell_index >= self.brt_cells {
            self.horizontal_cell_index = self.brt_cells - 1;
        }
        if self.brt_block_size == 0 {
            self.brt_block_size = DEFAULT_HOT_BLOCK_SIZE;
        }
        if self.segment_size == 0 {
            self.segment_size = self.brt_block_size.max(1);
        }
        if self.state_vectors == 0 {
            self.state_vectors = DEFAULT_HOT_STATE_VECTORS;
        }
        if self.patch_size == 0 {
            self.patch_size = 1;
        }
        if self.s1 == 0 {
            self.s1 = DEFAULT_HOT_S1;
        }
        if self.s2 == 0 {
            self.s2 = DEFAULT_HOT_S2;
        }
        if self.max_nodes == 0 {
            self.max_nodes = DEFAULT_HOT_MAX_NODES;
        }
        if self.max_temporal_edges == 0 {
            self.max_temporal_edges = DEFAULT_HOT_MAX_TEMPORAL_EDGES;
        }
        if self.decoder_hidden_dim == 0 {
            self.decoder_hidden_dim = DEFAULT_HOT_DECODER_HIDDEN_DIM;
        }
        self.link_score_bias = finite_or(self.link_score_bias, -0.85);
        self.link_score_scale = finite_or(self.link_score_scale, 3.2);
        if self.residual_clamp <= 0.0 || !self.residual_clamp.is_finite() {
            self.residual_clamp = 8.0;
        }
        self
    }

    pub fn per_node_width(&self) -> usize {
        self.aligned_dim * 4
    }

    pub fn pair_sequence_width(&self) -> usize {
        self.per_node_width() * 2
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotNode {
    pub node_id: String,
    pub features: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTemporalEdge {
    pub source_id: String,
    pub target_id: String,
    pub timestamp: i64,
    pub edge_type: String,
    pub features: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotInput {
    pub nodes: Vec<HotNode>,
    pub temporal_edges: Vec<HotTemporalEdge>,
    pub query_pairs: Vec<(String, String)>,
    pub as_of: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotNeighborInteraction {
    pub root_id: String,
    pub neighbor_id: String,
    pub hop: u8,
    pub edge: HotTemporalEdge,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotFeatureMatrices {
    pub node_id: String,
    pub partner_id: String,
    pub neighbor_ids: Vec<String>,
    pub hop_marks: Vec<u8>,
    pub node_matrix: Vec<Vec<f32>>,
    pub edge_matrix: Vec<Vec<f32>>,
    pub time_matrix: Vec<Vec<f32>>,
    pub cooccurrence_matrix: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotPairSequence {
    pub source_id: String,
    pub target_id: String,
    pub source_patched_len: usize,
    pub target_patched_len: usize,
    pub patched_sequence_len: usize,
    pub rows: Vec<Vec<f32>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotNodeRepresentation {
    pub node_id: String,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotPairRepresentation {
    pub source_id: String,
    pub target_id: String,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotLinkScore {
    pub source_id: String,
    pub target_id: String,
    pub score: f32,
    pub support: Option<PairformerSupportPath>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotOutput {
    pub node_representations: Vec<HotNodeRepresentation>,
    pub pair_representations: Vec<HotPairRepresentation>,
    pub link_scores: Vec<HotLinkScore>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTimeEncoder {
    pub frequencies: Vec<f32>,
}

impl HotTimeEncoder {
    pub fn from_config(config: &HotConfig) -> Self {
        let dim = config.time_encoding_dim.max(2);
        let pairs = (dim / 2).max(1);
        let frequencies = (0..pairs)
            .map(|idx| {
                let exponent = idx as f32 / pairs as f32;
                1.0 / 10_000.0_f32.powf(exponent)
            })
            .collect();
        Self { frequencies }
    }

    pub fn encode(&self, delta_ms: i64, dim: usize) -> Vec<f32> {
        tgat_time_encoding_with_frequencies(delta_ms, dim, &self.frequencies)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HotNegativeSamplingScheme {
    Random,
    Historical,
    Inductive,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTrainingConfig {
    pub learning_rate: f32,
    pub epochs: usize,
    pub batch_size: usize,
    pub negative_sampling: HotNegativeSamplingScheme,
    pub transductive: bool,
    pub inductive: bool,
    pub l2_weight_decay: f32,
    pub model: HotConfig,
}

impl Default for HotTrainingConfig {
    fn default() -> Self {
        Self {
            learning_rate: 1e-4,
            epochs: 50,
            batch_size: 100,
            negative_sampling: HotNegativeSamplingScheme::Random,
            transductive: true,
            inductive: true,
            l2_weight_decay: 1e-4,
            model: HotConfig::default(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotPairLabel {
    pub source_id: String,
    pub target_id: String,
    pub label: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotLinkTrainingExample {
    pub source_id: String,
    pub target_id: String,
    pub features: Vec<f32>,
    pub label: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotLearnedModel {
    pub input_dim: usize,
    pub hidden_dim: usize,
    pub hidden_weights: Vec<Vec<f32>>,
    pub hidden_bias: Vec<f32>,
    pub output_weights: Vec<f32>,
    pub output_bias: f32,
    pub model_id: String,
}

impl HotLearnedModel {
    pub fn score_features(&self, features: &[f32]) -> f32 {
        if self.input_dim == 0 || self.hidden_dim == 0 {
            return 0.0;
        }
        let x = normalized_training_features(features, self.input_dim);
        let hidden = self
            .hidden_weights
            .iter()
            .zip(self.hidden_bias.iter())
            .map(|(weights, bias)| relu(dot(weights, &x) + *bias))
            .collect::<Vec<_>>();
        sigmoid(dot(&self.output_weights, &hidden) + self.output_bias).clamp(0.0, 1.0)
    }

    pub fn score_hot_link(&self, score: &HotLinkScore, representation: &[f32]) -> HotLinkScore {
        let learned = self.score_features(representation);
        let support_confidence = score
            .support
            .as_ref()
            .map(|support| support.confidence.clamp(0.0, 1.0))
            .unwrap_or(0.5);
        HotLinkScore {
            source_id: score.source_id.clone(),
            target_id: score.target_id.clone(),
            score: (0.2 * score.score + 0.8 * learned * support_confidence).clamp(0.0, 1.0),
            support: score.support.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTrainingReport {
    pub epochs: usize,
    pub batch_size: usize,
    pub learning_rate: f32,
    pub negative_sampling: HotNegativeSamplingScheme,
    pub average_precision: f32,
    pub auc_roc: f32,
    pub examples: usize,
    pub final_loss: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTemporalSplitConfig {
    pub holdout_fraction: f32,
    pub negatives_per_positive: usize,
    pub max_positive_edges: usize,
}

impl Default for HotTemporalSplitConfig {
    fn default() -> Self {
        Self {
            holdout_fraction: 0.2,
            negatives_per_positive: 1,
            max_positive_edges: 512,
        }
    }
}

impl HotTemporalSplitConfig {
    pub fn normalized(mut self) -> Self {
        if !self.holdout_fraction.is_finite() {
            self.holdout_fraction = 0.2;
        }
        self.holdout_fraction = self.holdout_fraction.clamp(0.05, 0.5);
        self.negatives_per_positive = self.negatives_per_positive.max(1);
        if self.max_positive_edges == 0 {
            self.max_positive_edges = 512;
        }
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTimedPairLabel {
    pub source_id: String,
    pub target_id: String,
    pub label: bool,
    pub as_of: i64,
    pub positive_edge_id: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotPairPrediction {
    pub source_id: String,
    pub target_id: String,
    pub label: bool,
    pub as_of: i64,
    pub score: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTemporalLinkDataset {
    pub train_labels: Vec<HotTimedPairLabel>,
    pub test_labels: Vec<HotTimedPairLabel>,
    pub train_examples: Vec<HotLinkTrainingExample>,
    pub test_examples: Vec<HotLinkTrainingExample>,
    pub positive_edges: usize,
    pub train_positive_edges: usize,
    pub test_positive_edges: usize,
    pub max_timestamp: i64,
}

pub fn hot_input_from_snapshot(
    snapshot: &GraphSnapshot,
    query_pairs: Vec<(String, String)>,
    as_of: i64,
    config: HotConfig,
) -> ThgResult<HotInput> {
    let config = config.normalized();
    let nodes_by_id = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node))
        .collect::<BTreeMap<_, _>>();
    let all_edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && nodes_by_id.contains_key(&edge.from_id)
                && nodes_by_id.contains_key(&edge.to_id)
        })
        .filter_map(|edge| {
            let timestamp = temporal_edge_timestamp(edge)?;
            if timestamp < as_of {
                Some(hot_temporal_edge_from_record(edge, timestamp))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let adjacency = temporal_adjacency(&all_edges, as_of);
    let mut relevant_edge_keys = BTreeSet::new();
    let mut relevant_node_ids = BTreeSet::new();
    for (source_id, target_id) in &query_pairs {
        if !nodes_by_id.contains_key(source_id) || !nodes_by_id.contains_key(target_id) {
            continue;
        }
        relevant_node_ids.insert(source_id.clone());
        relevant_node_ids.insert(target_id.clone());
        for root_id in [source_id, target_id] {
            for interaction in collect_neighbor_interactions(root_id, &adjacency, as_of, &config) {
                relevant_node_ids.insert(interaction.neighbor_id);
                relevant_node_ids.insert(interaction.edge.source_id.clone());
                relevant_node_ids.insert(interaction.edge.target_id.clone());
                relevant_edge_keys.insert(temporal_edge_key(&interaction.edge));
            }
        }
    }

    let mut temporal_edges = all_edges
        .into_iter()
        .filter(|edge| relevant_edge_keys.contains(&temporal_edge_key(edge)))
        .collect::<Vec<_>>();
    temporal_edges.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| left.source_id.cmp(&right.source_id))
            .then_with(|| left.target_id.cmp(&right.target_id))
    });
    if temporal_edges.len() > config.max_temporal_edges {
        temporal_edges.truncate(config.max_temporal_edges);
    }

    let mut nodes = relevant_node_ids
        .into_iter()
        .filter_map(|node_id| {
            nodes_by_id.get(&node_id).map(|node| HotNode {
                node_id: node.id.clone(),
                features: hot_node_features(node),
            })
        })
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    if nodes.len() > config.max_nodes {
        nodes.truncate(config.max_nodes);
    }

    let retained = nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<BTreeSet<_>>();
    let query_pairs = query_pairs
        .into_iter()
        .filter(|(source_id, target_id)| {
            retained.contains(source_id) && retained.contains(target_id)
        })
        .collect::<Vec<_>>();

    Ok(HotInput {
        nodes,
        temporal_edges,
        query_pairs,
        as_of,
    })
}

pub fn extract_higher_order_temporal_neighbors(
    input: &HotInput,
    root_id: &str,
    config: HotConfig,
) -> Vec<HotNeighborInteraction> {
    let config = config.normalized();
    let adjacency = temporal_adjacency(&input.temporal_edges, input.as_of);
    collect_neighbor_interactions(root_id, &adjacency, input.as_of, &config)
}

pub fn build_hot_feature_matrices(
    input: &HotInput,
    node_id: &str,
    partner_id: &str,
    config: HotConfig,
) -> ThgResult<HotFeatureMatrices> {
    let config = config.normalized();
    validate_hot_input(input, &config)?;
    let node_ids = input
        .nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect::<BTreeSet<_>>();
    if !node_ids.contains(node_id) || !node_ids.contains(partner_id) {
        return Err(ThgError::new(
            "invalid_hot_input",
            format!("query endpoint {node_id} or {partner_id} is outside the node set"),
        ));
    }

    let own = extract_higher_order_temporal_neighbors(input, node_id, config.clone());
    let partner = extract_higher_order_temporal_neighbors(input, partner_id, config.clone());
    Ok(feature_matrices_from_interactions(
        input, node_id, partner_id, &own, &partner, &config,
    ))
}

pub fn patch_align_and_concatenate(
    input: &HotInput,
    source_id: &str,
    target_id: &str,
    config: HotConfig,
) -> ThgResult<HotPairSequence> {
    let config = config.normalized();
    let source = build_hot_feature_matrices(input, source_id, target_id, config.clone())?;
    let target = build_hot_feature_matrices(input, target_id, source_id, config.clone())?;
    let source_rows = patched_node_rows(&source, &config);
    let target_rows = patched_node_rows(&target, &config);
    let patched_sequence_len = source_rows.len().max(target_rows.len());
    let mut rows = Vec::with_capacity(patched_sequence_len);
    let zero = vec![0.0; config.per_node_width()];
    for idx in 0..patched_sequence_len {
        let mut row = source_rows
            .get(idx)
            .cloned()
            .unwrap_or_else(|| zero.clone());
        row.extend(
            target_rows
                .get(idx)
                .cloned()
                .unwrap_or_else(|| zero.clone()),
        );
        rows.push(row);
    }

    Ok(HotPairSequence {
        source_id: source_id.to_string(),
        target_id: target_id.to_string(),
        source_patched_len: source_rows.len(),
        target_patched_len: target_rows.len(),
        patched_sequence_len,
        rows,
    })
}

pub fn run_hot(input: &HotInput, config: HotConfig) -> ThgResult<HotOutput> {
    let config = config.normalized();
    validate_hot_input(input, &config)?;
    let node_ids = input
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<BTreeSet<_>>();
    let mut node_reps = BTreeMap::<String, Vec<Vec<f32>>>::new();
    let mut pair_representations = Vec::new();
    let mut link_scores = Vec::new();

    for (source_id, target_id) in &input.query_pairs {
        if source_id == target_id {
            continue;
        }
        if !node_ids.contains(source_id) || !node_ids.contains(target_id) {
            return Err(ThgError::new(
                "invalid_hot_input",
                format!("query pair {source_id}->{target_id} references a missing node"),
            ));
        }
        let pair_sequence =
            patch_align_and_concatenate(input, source_id, target_id, config.clone())?;
        let encoded_rows = block_recurrent_encode(&pair_sequence.rows, &config);
        let support = best_temporal_support_path(input, source_id, target_id, &config);
        let source_rep =
            pooled_node_representation(&encoded_rows, &pair_sequence.rows, 0, source_id, &config);
        let target_rep = pooled_node_representation(
            &encoded_rows,
            &pair_sequence.rows,
            config.per_node_width(),
            target_id,
            &config,
        );
        node_reps
            .entry(source_id.clone())
            .or_default()
            .push(source_rep.clone());
        node_reps
            .entry(target_id.clone())
            .or_default()
            .push(target_rep.clone());
        let pair_values = pair_representation_values(&source_rep, &target_rep, &config);
        let score = decode_link_score(&pair_values, support.as_ref(), &config);
        pair_representations.push(HotPairRepresentation {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            values: pair_values,
        });
        link_scores.push(HotLinkScore {
            source_id: source_id.clone(),
            target_id: target_id.clone(),
            score,
            support,
        });
    }

    let node_representations = node_reps
        .into_iter()
        .map(|(node_id, reps)| HotNodeRepresentation {
            node_id,
            values: average_rows(&reps, config.output_dim),
        })
        .collect();

    Ok(HotOutput {
        node_representations,
        pair_representations,
        link_scores,
    })
}

pub fn tgat_time_encoding(delta_ms: i64, dim: usize) -> Vec<f32> {
    let encoder = HotTimeEncoder::from_config(&HotConfig {
        time_encoding_dim: dim,
        ..HotConfig::default()
    });
    encoder.encode(delta_ms, dim)
}

pub fn evaluate_hot_predictions(
    scored_labels: &[(f32, bool)],
    config: HotTrainingConfig,
) -> HotTrainingReport {
    HotTrainingReport {
        epochs: config.epochs,
        batch_size: config.batch_size,
        learning_rate: config.learning_rate,
        negative_sampling: config.negative_sampling,
        average_precision: average_precision(scored_labels),
        auc_roc: auc_roc(scored_labels),
        examples: scored_labels.len(),
        final_loss: logistic_loss(scored_labels),
    }
}

pub fn hot_training_examples_from_input(
    input: &HotInput,
    labels: &[HotPairLabel],
    config: HotConfig,
) -> ThgResult<Vec<HotLinkTrainingExample>> {
    let output = run_hot(input, config)?;
    let labels = labels
        .iter()
        .map(|label| {
            (
                (label.source_id.clone(), label.target_id.clone()),
                label.label,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let score_features = output
        .link_scores
        .iter()
        .map(|score| {
            let support_confidence = score
                .support
                .as_ref()
                .map(|support| support.confidence.clamp(0.0, 1.0))
                .unwrap_or(0.0);
            let support_len = score
                .support
                .as_ref()
                .map(|support| support.node_ids.len() as f32 / 4.0)
                .unwrap_or(0.0);
            (
                (score.source_id.clone(), score.target_id.clone()),
                vec![score.score, support_confidence, support_len.clamp(0.0, 1.0)],
            )
        })
        .collect::<BTreeMap<_, _>>();
    Ok(output
        .pair_representations
        .into_iter()
        .filter_map(|pair| {
            let label = labels.get(&(pair.source_id.clone(), pair.target_id.clone()))?;
            let mut features = pair.values;
            if let Some(extra) =
                score_features.get(&(pair.source_id.clone(), pair.target_id.clone()))
            {
                features.extend(extra);
            }
            Some(HotLinkTrainingExample {
                source_id: pair.source_id,
                target_id: pair.target_id,
                features,
                label: *label,
            })
        })
        .collect())
}

pub fn train_hot_link_model(
    examples: &[HotLinkTrainingExample],
    config: HotTrainingConfig,
) -> ThgResult<(HotLearnedModel, HotTrainingReport)> {
    if examples.is_empty() {
        return Err(ThgError::new(
            "invalid_hot_training_data",
            "at least one HOT training example is required",
        ));
    }
    let input_dim = examples
        .iter()
        .map(|example| example.features.len())
        .max()
        .unwrap_or(0);
    if input_dim == 0 {
        return Err(ThgError::new(
            "invalid_hot_training_data",
            "training examples must contain pair features",
        ));
    }
    let hidden_dim = config.model.decoder_hidden_dim.max(1);
    let mut model = HotLearnedModel {
        input_dim,
        hidden_dim,
        hidden_weights: (0..hidden_dim)
            .map(|hidden_idx| {
                (0..input_dim)
                    .map(|feature_idx| {
                        stable_noise("hot-learned-hidden", hidden_idx * input_dim + feature_idx)
                            * 0.03
                    })
                    .collect()
            })
            .collect(),
        hidden_bias: vec![0.01; hidden_dim],
        output_weights: (0..hidden_dim)
            .map(|idx| stable_noise("hot-learned-output", idx) * 0.03)
            .collect(),
        output_bias: 0.0,
        model_id: "hot-learned/native-mlp-v1".to_string(),
    };

    let learning_rate = config.learning_rate.max(1e-6);
    let epochs = config.epochs.max(1);
    let l2 = config.l2_weight_decay.max(0.0);
    let mut final_loss = 0.0;
    for _ in 0..epochs {
        final_loss = 0.0;
        for example in examples {
            let x = normalized_training_features(&example.features, input_dim);
            let mut hidden_pre = vec![0.0; hidden_dim];
            let mut hidden = vec![0.0; hidden_dim];
            for hidden_idx in 0..hidden_dim {
                hidden_pre[hidden_idx] =
                    dot(&model.hidden_weights[hidden_idx], &x) + model.hidden_bias[hidden_idx];
                hidden[hidden_idx] = relu(hidden_pre[hidden_idx]);
            }
            let logit = dot(&model.output_weights, &hidden) + model.output_bias;
            let prediction = sigmoid(logit).clamp(1e-6, 1.0 - 1e-6);
            let label = if example.label { 1.0 } else { 0.0 };
            final_loss += -(label * prediction.ln() + (1.0 - label) * (1.0 - prediction).ln());
            let error = prediction - label;

            let old_output_weights = model.output_weights.clone();
            for hidden_idx in 0..hidden_dim {
                let grad = error * hidden[hidden_idx] + l2 * model.output_weights[hidden_idx];
                model.output_weights[hidden_idx] -= learning_rate * grad;
            }
            model.output_bias -= learning_rate * error;

            for hidden_idx in 0..hidden_dim {
                let hidden_grad = if hidden_pre[hidden_idx] > 0.0 {
                    error * old_output_weights[hidden_idx]
                } else {
                    0.0
                };
                for (feature_idx, value) in x.iter().enumerate().take(input_dim) {
                    let grad =
                        hidden_grad * *value + l2 * model.hidden_weights[hidden_idx][feature_idx];
                    model.hidden_weights[hidden_idx][feature_idx] -= learning_rate * grad;
                }
                model.hidden_bias[hidden_idx] -= learning_rate * hidden_grad;
            }
        }
        final_loss /= examples.len() as f32;
    }

    let scored_labels = examples
        .iter()
        .map(|example| (model.score_features(&example.features), example.label))
        .collect::<Vec<_>>();
    let report = HotTrainingReport {
        epochs,
        batch_size: config.batch_size,
        learning_rate,
        negative_sampling: config.negative_sampling,
        average_precision: average_precision(&scored_labels),
        auc_roc: auc_roc(&scored_labels),
        examples: examples.len(),
        final_loss,
    };
    Ok((model, report))
}

pub fn hot_temporal_edge_timestamp_for_record(edge: &EdgeRecord) -> Option<i64> {
    temporal_edge_timestamp(edge)
}

pub fn hot_temporal_link_dataset_from_snapshot(
    snapshot: &GraphSnapshot,
    config: HotConfig,
    split: HotTemporalSplitConfig,
) -> ThgResult<HotTemporalLinkDataset> {
    let config = config.normalized();
    let split = split.normalized();
    let node_ids = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut temporal_edges = snapshot
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && edge.from_id != edge.to_id
                && node_ids.contains(&edge.from_id)
                && node_ids.contains(&edge.to_id)
        })
        .filter_map(|edge| temporal_edge_timestamp(edge).map(|timestamp| (edge, timestamp)))
        .collect::<Vec<_>>();
    temporal_edges.sort_by(|left, right| {
        left.1
            .cmp(&right.1)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });
    if temporal_edges.len() > split.max_positive_edges {
        let keep_from = temporal_edges.len() - split.max_positive_edges;
        temporal_edges = temporal_edges.into_iter().skip(keep_from).collect();
    }
    if temporal_edges.len() < 2 {
        return Err(ThgError::new(
            "invalid_hot_training_data",
            "HOT temporal training requires at least two timestamped directed edges",
        ));
    }

    let holdout = ((temporal_edges.len() as f32) * split.holdout_fraction)
        .round()
        .max(1.0) as usize;
    let holdout = holdout.min(temporal_edges.len() - 1);
    let train_len = temporal_edges.len() - holdout;
    let all_positive_pairs = temporal_edges
        .iter()
        .map(|(edge, _)| (edge.from_id.clone(), edge.to_id.clone()))
        .collect::<BTreeSet<_>>();
    let sorted_node_ids = node_ids.into_iter().collect::<Vec<_>>();

    let train_labels = timed_labels_for_edges(
        &temporal_edges[..train_len],
        &sorted_node_ids,
        &all_positive_pairs,
        split.negatives_per_positive,
    );
    let test_labels = timed_labels_for_edges(
        &temporal_edges[train_len..],
        &sorted_node_ids,
        &all_positive_pairs,
        split.negatives_per_positive,
    );
    let train_examples =
        hot_training_examples_from_timed_labels(snapshot, &train_labels, config.clone())?;
    let test_examples =
        hot_training_examples_from_timed_labels(snapshot, &test_labels, config.clone())?;
    if train_examples.is_empty() || test_examples.is_empty() {
        return Err(ThgError::new(
            "invalid_hot_training_data",
            "HOT temporal split produced no train or test examples",
        ));
    }

    Ok(HotTemporalLinkDataset {
        train_labels,
        test_labels,
        train_examples,
        test_examples,
        positive_edges: temporal_edges.len(),
        train_positive_edges: train_len,
        test_positive_edges: holdout,
        max_timestamp: temporal_edges
            .last()
            .map(|(_, timestamp)| *timestamp)
            .unwrap_or_default(),
    })
}

pub fn hot_training_examples_from_timed_labels(
    snapshot: &GraphSnapshot,
    labels: &[HotTimedPairLabel],
    config: HotConfig,
) -> ThgResult<Vec<HotLinkTrainingExample>> {
    let config = config.normalized();
    let mut examples = Vec::with_capacity(labels.len());
    for label in labels {
        let input = hot_input_from_snapshot(
            snapshot,
            vec![(label.source_id.clone(), label.target_id.clone())],
            label.as_of,
            config.clone(),
        )?;
        let mut derived = hot_training_examples_from_input(
            &input,
            &[HotPairLabel {
                source_id: label.source_id.clone(),
                target_id: label.target_id.clone(),
                label: label.label,
            }],
            config.clone(),
        )?;
        examples.append(&mut derived);
    }
    Ok(examples)
}

pub fn score_hot_timed_labels(
    snapshot: &GraphSnapshot,
    labels: &[HotTimedPairLabel],
    config: HotConfig,
) -> ThgResult<Vec<HotPairPrediction>> {
    let config = config.normalized();
    let mut predictions = Vec::with_capacity(labels.len());
    for label in labels {
        let input = hot_input_from_snapshot(
            snapshot,
            vec![(label.source_id.clone(), label.target_id.clone())],
            label.as_of,
            config.clone(),
        )?;
        let output = run_hot(&input, config.clone())?;
        let score = output
            .link_scores
            .iter()
            .find(|score| score.source_id == label.source_id && score.target_id == label.target_id)
            .map(|score| score.score)
            .unwrap_or(0.0);
        predictions.push(HotPairPrediction {
            source_id: label.source_id.clone(),
            target_id: label.target_id.clone(),
            label: label.label,
            as_of: label.as_of,
            score,
        });
    }
    Ok(predictions)
}

fn validate_hot_input(input: &HotInput, config: &HotConfig) -> ThgResult<()> {
    if input.nodes.is_empty() {
        return Err(ThgError::new(
            "invalid_hot_input",
            "at least one node is required",
        ));
    }
    if input.nodes.len() > config.max_nodes {
        return Err(ThgError::new(
            "hot_bound_exceeded",
            format!(
                "HOT input has {} nodes, above max_nodes {}",
                input.nodes.len(),
                config.max_nodes
            ),
        ));
    }
    if input.temporal_edges.len() > config.max_temporal_edges {
        return Err(ThgError::new(
            "hot_bound_exceeded",
            format!(
                "HOT input has {} temporal edges, above max_temporal_edges {}",
                input.temporal_edges.len(),
                config.max_temporal_edges
            ),
        ));
    }
    let mut node_ids = BTreeSet::new();
    for node in &input.nodes {
        if node.node_id.trim().is_empty() {
            return Err(ThgError::new("invalid_hot_input", "node_id is required"));
        }
        if !node_ids.insert(node.node_id.clone()) {
            return Err(ThgError::new(
                "invalid_hot_input",
                format!("duplicate node_id {}", node.node_id),
            ));
        }
        if node.features.iter().any(|value| !value.is_finite()) {
            return Err(ThgError::new(
                "invalid_hot_input",
                format!("node {} has non-finite features", node.node_id),
            ));
        }
    }
    for edge in &input.temporal_edges {
        if edge.source_id.trim().is_empty()
            || edge.target_id.trim().is_empty()
            || edge.edge_type.trim().is_empty()
        {
            return Err(ThgError::new(
                "invalid_hot_input",
                "temporal edge source_id, target_id, and edge_type are required",
            ));
        }
        if !node_ids.contains(&edge.source_id) || !node_ids.contains(&edge.target_id) {
            return Err(ThgError::new(
                "invalid_hot_input",
                "temporal edge references an endpoint outside the bounded node set",
            ));
        }
        if edge.timestamp >= input.as_of {
            return Err(ThgError::new(
                "invalid_hot_input",
                "temporal edge timestamp must be strictly before as_of",
            ));
        }
        if edge.features.iter().any(|value| !value.is_finite()) {
            return Err(ThgError::new(
                "invalid_hot_input",
                "temporal edge has non-finite features",
            ));
        }
    }
    Ok(())
}

fn timed_labels_for_edges(
    edges: &[(&EdgeRecord, i64)],
    node_ids: &[String],
    all_positive_pairs: &BTreeSet<(String, String)>,
    negatives_per_positive: usize,
) -> Vec<HotTimedPairLabel> {
    let mut labels = Vec::with_capacity(edges.len() * (1 + negatives_per_positive));
    let mut emitted_negatives = BTreeSet::new();
    for (edge, timestamp) in edges {
        labels.push(HotTimedPairLabel {
            source_id: edge.from_id.clone(),
            target_id: edge.to_id.clone(),
            label: true,
            as_of: *timestamp,
            positive_edge_id: Some(edge.id.clone()),
        });
        for (source_id, target_id) in deterministic_negative_pairs(
            edge,
            *timestamp,
            node_ids,
            all_positive_pairs,
            negatives_per_positive,
            &mut emitted_negatives,
        ) {
            labels.push(HotTimedPairLabel {
                source_id,
                target_id,
                label: false,
                as_of: *timestamp,
                positive_edge_id: Some(edge.id.clone()),
            });
        }
    }
    labels
}

fn deterministic_negative_pairs(
    positive: &EdgeRecord,
    timestamp: i64,
    node_ids: &[String],
    all_positive_pairs: &BTreeSet<(String, String)>,
    count: usize,
    emitted: &mut BTreeSet<(String, String, i64)>,
) -> Vec<(String, String)> {
    if node_ids.len() < 2 || count == 0 {
        return Vec::new();
    }
    let mut negatives = Vec::with_capacity(count);
    let start = stable_index(
        json!({
            "source": positive.from_id,
            "target": positive.to_id,
            "timestamp": timestamp,
            "edge_id": positive.id,
        }),
        node_ids.len() * node_ids.len(),
    );
    let total = node_ids.len() * node_ids.len();
    for offset in 0..(total * 2) {
        if negatives.len() >= count {
            break;
        }
        let flat = (start + offset * 17) % total;
        let source_id = node_ids[flat / node_ids.len()].clone();
        let target_id = node_ids[flat % node_ids.len()].clone();
        if source_id == target_id
            || all_positive_pairs.contains(&(source_id.clone(), target_id.clone()))
            || !emitted.insert((source_id.clone(), target_id.clone(), timestamp))
        {
            continue;
        }
        negatives.push((source_id, target_id));
    }
    negatives
}

fn stable_index<T: Serialize>(value: T, modulo: usize) -> usize {
    if modulo == 0 {
        return 0;
    }
    let digest = stable_hash(value);
    let hex = digest.strip_prefix("sha256:").unwrap_or(&digest);
    usize::from_str_radix(&hex[..hex.len().min(12)], 16).unwrap_or(0) % modulo
}

fn collect_neighbor_interactions(
    root_id: &str,
    adjacency: &BTreeMap<String, Vec<HotTemporalEdge>>,
    as_of: i64,
    config: &HotConfig,
) -> Vec<HotNeighborInteraction> {
    let mut one_hop = adjacency
        .get(root_id)
        .into_iter()
        .flat_map(|edges| edges.iter())
        .filter(|edge| edge.timestamp < as_of)
        .take(config.s1)
        .map(|edge| HotNeighborInteraction {
            root_id: root_id.to_string(),
            neighbor_id: edge.target_id.clone(),
            hop: 1,
            edge: edge.clone(),
        })
        .collect::<Vec<_>>();

    let mut two_hop = Vec::new();
    for first in &one_hop {
        let Some(edges) = adjacency.get(&first.neighbor_id) else {
            continue;
        };
        for edge in edges.iter().filter(|edge| edge.timestamp < as_of) {
            if edge.target_id == root_id {
                continue;
            }
            two_hop.push(HotNeighborInteraction {
                root_id: root_id.to_string(),
                neighbor_id: edge.target_id.clone(),
                hop: 2,
                edge: edge.clone(),
            });
        }
    }
    two_hop.sort_by(|left, right| {
        right
            .edge
            .timestamp
            .cmp(&left.edge.timestamp)
            .then_with(|| left.neighbor_id.cmp(&right.neighbor_id))
    });
    two_hop.truncate(config.s2);
    one_hop.extend(two_hop);
    one_hop.sort_by(|left, right| {
        right
            .edge
            .timestamp
            .cmp(&left.edge.timestamp)
            .then_with(|| left.hop.cmp(&right.hop))
            .then_with(|| left.neighbor_id.cmp(&right.neighbor_id))
    });
    one_hop
}

fn feature_matrices_from_interactions(
    input: &HotInput,
    node_id: &str,
    partner_id: &str,
    own: &[HotNeighborInteraction],
    partner: &[HotNeighborInteraction],
    config: &HotConfig,
) -> HotFeatureMatrices {
    let node_features = input
        .nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.features.as_slice()))
        .collect::<BTreeMap<_, _>>();
    let own_counts = neighbor_counts(own);
    let partner_counts = neighbor_counts(partner);
    let encoder = HotTimeEncoder::from_config(config);

    let mut neighbor_ids = Vec::with_capacity(own.len());
    let mut hop_marks = Vec::with_capacity(own.len());
    let mut node_matrix = Vec::with_capacity(own.len());
    let mut edge_matrix = Vec::with_capacity(own.len());
    let mut time_matrix = Vec::with_capacity(own.len());
    let mut cooccurrence_matrix = Vec::with_capacity(own.len());

    for interaction in own {
        neighbor_ids.push(interaction.neighbor_id.clone());
        hop_marks.push(interaction.hop);
        let mut node_row = node_features
            .get(interaction.neighbor_id.as_str())
            .copied()
            .unwrap_or_default()
            .to_vec();
        match interaction.hop {
            1 => node_row.extend([1.0, 0.0]),
            _ => node_row.extend([0.0, 1.0]),
        }
        node_matrix.push(node_row);

        let mut edge_row = interaction.edge.features.clone();
        edge_row.push(fold_categorical_feature(&interaction.edge.edge_type));
        edge_matrix.push(edge_row);

        let delta_ms = input.as_of.saturating_sub(interaction.edge.timestamp);
        time_matrix.push(encoder.encode(delta_ms, config.time_encoding_dim));

        let own_count = *own_counts.get(&interaction.neighbor_id).unwrap_or(&0) as f32;
        let partner_count = *partner_counts.get(&interaction.neighbor_id).unwrap_or(&0) as f32;
        cooccurrence_matrix.push(cooccurrence_encoding(
            &interaction.neighbor_id,
            own_count,
            partner_count,
            config.cooccurrence_dim,
            node_id,
            partner_id,
        ));
    }

    HotFeatureMatrices {
        node_id: node_id.to_string(),
        partner_id: partner_id.to_string(),
        neighbor_ids,
        hop_marks,
        node_matrix,
        edge_matrix,
        time_matrix,
        cooccurrence_matrix,
    }
}

fn patched_node_rows(matrices: &HotFeatureMatrices, config: &HotConfig) -> Vec<Vec<f32>> {
    let node = align_patched_matrix(
        &matrices.node_matrix,
        config.patch_size,
        config.aligned_dim,
        "node",
    );
    let edge = align_patched_matrix(
        &matrices.edge_matrix,
        config.patch_size,
        config.aligned_dim,
        "edge",
    );
    let time = align_patched_matrix(
        &matrices.time_matrix,
        config.patch_size,
        config.aligned_dim,
        "time",
    );
    let co = align_patched_matrix(
        &matrices.cooccurrence_matrix,
        config.patch_size,
        config.aligned_dim,
        "cooccurrence",
    );
    let len = node.len().max(edge.len()).max(time.len()).max(co.len());
    let zero = vec![0.0; config.aligned_dim];
    (0..len)
        .map(|idx| {
            let mut row = node.get(idx).cloned().unwrap_or_else(|| zero.clone());
            row.extend(edge.get(idx).cloned().unwrap_or_else(|| zero.clone()));
            row.extend(time.get(idx).cloned().unwrap_or_else(|| zero.clone()));
            row.extend(co.get(idx).cloned().unwrap_or_else(|| zero.clone()));
            row
        })
        .collect()
}

fn align_patched_matrix(
    matrix: &[Vec<f32>],
    patch_size: usize,
    aligned_dim: usize,
    salt: &str,
) -> Vec<Vec<f32>> {
    patch_rows(matrix, patch_size)
        .into_iter()
        .enumerate()
        .map(|(idx, row)| deterministic_align(&row, aligned_dim, &format!("{salt}:{idx}")))
        .collect()
}

fn patch_rows(matrix: &[Vec<f32>], patch_size: usize) -> Vec<Vec<f32>> {
    if matrix.is_empty() {
        return Vec::new();
    }
    let patch_size = patch_size.max(1);
    matrix
        .chunks(patch_size)
        .map(|chunk| {
            let width = chunk.iter().map(Vec::len).max().unwrap_or(0);
            if width == 0 {
                return Vec::new();
            }
            let mut row = vec![0.0; width];
            for item in chunk {
                for (idx, value) in item.iter().enumerate() {
                    row[idx] += *value;
                }
            }
            for value in &mut row {
                *value /= chunk.len() as f32;
            }
            row
        })
        .collect()
}

fn block_recurrent_encode(sequence: &[Vec<f32>], config: &HotConfig) -> Vec<Vec<f32>> {
    if sequence.is_empty() {
        return vec![vec![0.0; config.output_dim]];
    }
    let mut state = initial_state_vectors(config);
    let mut outputs = Vec::with_capacity(sequence.len());
    let segment_size = config.segment_size.max(config.brt_block_size).max(1);
    let block_size = config.brt_block_size.max(1);

    for (segment_idx, segment) in sequence.chunks(segment_size).enumerate() {
        for (block_idx, block) in segment.chunks(block_size).enumerate() {
            let block_rows = block
                .iter()
                .enumerate()
                .map(|(idx, row)| {
                    let absolute_idx = segment_idx * segment_size + block_idx * block_size + idx;
                    let mut projected = deterministic_align(
                        row,
                        config.output_dim,
                        &format!("hot-token:{absolute_idx}"),
                    );
                    add_position_embedding(&mut projected, absolute_idx, config);
                    projected
                })
                .collect::<Vec<_>>();
            let local = multi_head_self_attention(&block_rows, config.attention_heads);
            let cross = cross_attention_with_state(&block_rows, &state, config.attention_heads);
            let mut block_outputs = Vec::with_capacity(block_rows.len());
            for idx in 0..block_rows.len() {
                let gate = sigmoid(mean(&block_rows[idx]) + mean(&cross[idx]) - mean(&state[0]));
                let mut mixed = blend(&local[idx], &cross[idx], gate);
                let ff = geglu_feed_forward(&mixed, config.output_dim, config.decoder_hidden_dim);
                for (slot, value) in mixed.iter_mut().zip(ff) {
                    *slot += 0.15 * value;
                }
                clamp_vector(&mut mixed, config.residual_clamp);
                block_outputs.push(mixed);
            }
            state = update_recurrent_state(&state, &block_outputs, config);
            outputs.extend(block_outputs);
        }
    }
    outputs
}

fn initial_state_vectors(config: &HotConfig) -> Vec<Vec<f32>> {
    (0..config.state_vectors)
        .map(|idx| {
            let mut row = (0..config.output_dim)
                .map(|dim| stable_noise("hot-state", idx * config.output_dim + dim) * 0.05)
                .collect::<Vec<_>>();
            clamp_vector(&mut row, config.residual_clamp);
            row
        })
        .collect()
}

fn multi_head_self_attention(rows: &[Vec<f32>], heads: usize) -> Vec<Vec<f32>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let dim = rows[0].len();
    let heads = heads.max(1).min(dim.max(1));
    let head_dim = (dim / heads).max(1);
    rows.iter()
        .enumerate()
        .map(|(idx, query)| {
            let mut out = vec![0.0; dim];
            for head in 0..heads {
                let start = head * head_dim;
                let end = if head == heads - 1 {
                    dim
                } else {
                    ((head + 1) * head_dim).min(dim)
                };
                if start >= end {
                    continue;
                }
                let logits = rows
                    .iter()
                    .map(|key| dot(&query[start..end], &key[start..end]) / (end - start) as f32)
                    .collect::<Vec<_>>();
                let weights = softmax(&logits);
                for (row_idx, row) in rows.iter().enumerate() {
                    for dim_idx in start..end {
                        out[dim_idx] += weights[row_idx] * row[dim_idx];
                    }
                }
            }
            let gate = sigmoid(mean(query) + stable_noise("hot-self-gate", idx));
            blend(query, &out, gate)
        })
        .collect()
}

fn cross_attention_with_state(
    rows: &[Vec<f32>],
    state: &[Vec<f32>],
    heads: usize,
) -> Vec<Vec<f32>> {
    if rows.is_empty() {
        return Vec::new();
    }
    let dim = rows[0].len();
    let heads = heads.max(1).min(dim.max(1));
    let head_dim = (dim / heads).max(1);
    rows.iter()
        .map(|query| {
            let mut out = vec![0.0; dim];
            for head in 0..heads {
                let start = head * head_dim;
                let end = if head == heads - 1 {
                    dim
                } else {
                    ((head + 1) * head_dim).min(dim)
                };
                if start >= end {
                    continue;
                }
                let logits = state
                    .iter()
                    .map(|state_row| {
                        dot(&query[start..end], &state_row[start..end]) / (end - start) as f32
                    })
                    .collect::<Vec<_>>();
                let weights = softmax(&logits);
                for (state_idx, state_row) in state.iter().enumerate() {
                    for dim_idx in start..end {
                        out[dim_idx] += weights[state_idx] * state_row[dim_idx];
                    }
                }
            }
            out
        })
        .collect()
}

fn update_recurrent_state(
    state: &[Vec<f32>],
    block_outputs: &[Vec<f32>],
    config: &HotConfig,
) -> Vec<Vec<f32>> {
    let summary = average_rows(block_outputs, config.output_dim);
    state
        .iter()
        .enumerate()
        .map(|(idx, prior)| {
            let mut candidate = deterministic_align(
                &summary,
                config.output_dim,
                &format!("hot-state-update:{idx}"),
            );
            let self_mix = multi_head_self_attention(&[prior.clone(), candidate.clone()], 1);
            if let Some(updated) = self_mix.get(1) {
                candidate = updated.clone();
            }
            let gate = sigmoid(mean(&summary) + stable_noise("hot-state-ema", idx));
            let mut next = blend(&candidate, prior, gate);
            clamp_vector(&mut next, config.residual_clamp);
            next
        })
        .collect()
}

fn pooled_node_representation(
    encoded_rows: &[Vec<f32>],
    raw_rows: &[Vec<f32>],
    raw_offset: usize,
    node_id: &str,
    config: &HotConfig,
) -> Vec<f32> {
    let pooled_encoded = average_rows(encoded_rows, config.output_dim);
    let raw_width = config.per_node_width();
    let raw = raw_rows
        .iter()
        .map(|row| {
            let end = (raw_offset + raw_width).min(row.len());
            if raw_offset >= end {
                Vec::new()
            } else {
                row[raw_offset..end].to_vec()
            }
        })
        .collect::<Vec<_>>();
    let raw_projected = deterministic_align(
        &average_rows(&raw, raw_width),
        config.output_dim,
        &format!("hot-node-pool:{node_id}"),
    );
    let mut out = pooled_encoded
        .iter()
        .zip(raw_projected)
        .map(|(left, right)| 0.65 * *left + 0.35 * right)
        .collect::<Vec<_>>();
    clamp_vector(&mut out, config.residual_clamp);
    out
}

fn pair_representation_values(source: &[f32], target: &[f32], config: &HotConfig) -> Vec<f32> {
    let mut values = Vec::with_capacity(config.output_dim * 4);
    values.extend_from_slice(source);
    values.extend_from_slice(target);
    values.extend(
        source
            .iter()
            .zip(target)
            .map(|(left, right)| (left - right).abs()),
    );
    values.extend(source.iter().zip(target).map(|(left, right)| left * right));
    values
}

fn decode_link_score(
    pair_values: &[f32],
    support: Option<&PairformerSupportPath>,
    config: &HotConfig,
) -> f32 {
    let hidden = geglu_feed_forward(
        pair_values,
        config.decoder_hidden_dim,
        config.decoder_hidden_dim,
    );
    let positive_signal = positive_mean(&hidden) + positive_mean(pair_values) * 0.25;
    let support_signal = support
        .map(|support| support.confidence.clamp(0.0, 1.0))
        .unwrap_or(0.0);
    let activation = sigmoid(
        config.link_score_bias + config.link_score_scale * (positive_signal + support_signal),
    );
    match support {
        Some(support) => 0.45 + 0.55 * activation * support.confidence.clamp(0.0, 1.0),
        None => 0.5 * activation,
    }
    .clamp(0.0, 1.0)
}

fn best_temporal_support_path(
    input: &HotInput,
    source_id: &str,
    target_id: &str,
    config: &HotConfig,
) -> Option<PairformerSupportPath> {
    let source = extract_higher_order_temporal_neighbors(input, source_id, config.clone());
    let target = extract_higher_order_temporal_neighbors(input, target_id, config.clone());
    let mut best = None::<PairformerSupportPath>;

    for first in source.iter().filter(|item| item.hop == 1) {
        for second in input.temporal_edges.iter().filter(|edge| {
            edge.source_id == first.neighbor_id
                && edge.target_id == target_id
                && edge.timestamp < input.as_of
        }) {
            let recency = temporal_recency_confidence(input.as_of, first.edge.timestamp)
                * temporal_recency_confidence(input.as_of, second.timestamp);
            let confidence = recency.sqrt().clamp(0.0, 1.0);
            let support = PairformerSupportPath {
                edge_ids: vec![support_edge_id(&first.edge), support_edge_id(second)],
                node_ids: vec![
                    source_id.to_string(),
                    first.neighbor_id.clone(),
                    target_id.to_string(),
                ],
                relation_hint: normalized_hot_edge_type(&first.edge.edge_type, &second.edge_type),
                confidence,
            };
            if best
                .as_ref()
                .is_none_or(|prior| support.confidence > prior.confidence)
            {
                best = Some(support);
            }
        }
    }

    let target_by_neighbor = target
        .iter()
        .filter(|item| item.hop == 1)
        .map(|item| (item.neighbor_id.as_str(), item))
        .collect::<BTreeMap<_, _>>();
    for own in source.iter().filter(|item| item.hop == 1) {
        let Some(partner) = target_by_neighbor.get(own.neighbor_id.as_str()) else {
            continue;
        };
        let recency = temporal_recency_confidence(input.as_of, own.edge.timestamp)
            * temporal_recency_confidence(input.as_of, partner.edge.timestamp);
        let confidence = (0.72 * recency.sqrt()).clamp(0.0, 1.0);
        let support = PairformerSupportPath {
            edge_ids: vec![support_edge_id(&own.edge), support_edge_id(&partner.edge)],
            node_ids: vec![
                source_id.to_string(),
                own.neighbor_id.clone(),
                target_id.to_string(),
            ],
            relation_hint: format!("TEMPORAL_SHARED_{}", own.edge.edge_type),
            confidence,
        };
        if best
            .as_ref()
            .is_none_or(|prior| support.confidence > prior.confidence)
        {
            best = Some(support);
        }
    }
    best
}

fn temporal_adjacency(
    temporal_edges: &[HotTemporalEdge],
    as_of: i64,
) -> BTreeMap<String, Vec<HotTemporalEdge>> {
    let mut adjacency = BTreeMap::<String, Vec<HotTemporalEdge>>::new();
    for edge in temporal_edges.iter().filter(|edge| edge.timestamp < as_of) {
        adjacency
            .entry(edge.source_id.clone())
            .or_default()
            .push(edge.clone());
    }
    for edges in adjacency.values_mut() {
        edges.sort_by(|left, right| {
            right
                .timestamp
                .cmp(&left.timestamp)
                .then_with(|| left.target_id.cmp(&right.target_id))
                .then_with(|| left.edge_type.cmp(&right.edge_type))
        });
    }
    adjacency
}

fn hot_temporal_edge_from_record(edge: &EdgeRecord, timestamp: i64) -> HotTemporalEdge {
    HotTemporalEdge {
        source_id: edge.from_id.clone(),
        target_id: edge.to_id.clone(),
        timestamp,
        edge_type: edge.edge_type.clone(),
        features: hot_edge_features(edge),
    }
}

fn hot_node_features(node: &NodeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&node.properties, "embedding");
    features.extend(numeric_array_property(&node.properties, "features"));
    features.extend(temporal_feature_values(&node.properties));
    features.push(node.labels.len() as f32);
    features.push(node.version as f32 / 1024.0);
    features
}

fn hot_edge_features(edge: &EdgeRecord) -> Vec<f32> {
    let mut features = numeric_array_property(&edge.properties, "embedding");
    features.extend(numeric_array_property(&edge.properties, "features"));
    features.extend(temporal_feature_values(&edge.properties));
    features.push(edge.effective_confidence() as f32);
    features.push((edge.edge_type.len() as f32).ln_1p());
    features
}

fn temporal_edge_timestamp(edge: &EdgeRecord) -> Option<i64> {
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

fn temporal_feature_values(properties: &Value) -> Vec<f32> {
    let Some(start) = numeric_i64_property_any(
        properties,
        &["t_valid", "valid_from_ms", "valid_start_ms", "timestamp_ms"],
    ) else {
        return Vec::new();
    };
    let end = numeric_i64_property_any(
        properties,
        &["t_invalid", "valid_to_ms", "valid_end_ms", "valid_until_ms"],
    )
    .unwrap_or(start);
    let duration = end.saturating_sub(start).abs().max(1) as f32;
    vec![
        (start as f32 / 86_400_000.0).tanh(),
        duration.ln_1p() / 32.0,
    ]
}

fn neighbor_counts(interactions: &[HotNeighborInteraction]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for interaction in interactions {
        *counts.entry(interaction.neighbor_id.clone()).or_insert(0) += 1;
    }
    counts
}

fn cooccurrence_encoding(
    neighbor_id: &str,
    own_count: f32,
    partner_count: f32,
    dim: usize,
    node_id: &str,
    partner_id: &str,
) -> Vec<f32> {
    let left = count_mlp(
        own_count,
        dim,
        &format!("hot-co-left:{node_id}:{partner_id}:{neighbor_id}"),
    );
    let right = count_mlp(
        partner_count,
        dim,
        &format!("hot-co-right:{partner_id}:{node_id}:{neighbor_id}"),
    );
    left.into_iter()
        .zip(right)
        .map(|(a, b)| (a + b).max(0.0))
        .collect()
}

fn count_mlp(count: f32, dim: usize, salt: &str) -> Vec<f32> {
    (0..dim)
        .map(|idx| {
            let hidden = (count * (0.5 + stable_noise(salt, idx).abs())
                + stable_noise(salt, idx + dim) * 0.1)
                .max(0.0);
            hidden * (0.5 + stable_noise("hot-count-out", idx).abs())
        })
        .collect()
}

fn tgat_time_encoding_with_frequencies(delta_ms: i64, dim: usize, frequencies: &[f32]) -> Vec<f32> {
    let dim = if dim % 2 == 0 { dim } else { dim + 1 };
    let pairs = (dim / 2).max(1);
    let scale = (1.0 / dim as f32).sqrt();
    let delta = delta_ms.max(0) as f32 / 1000.0;
    let mut out = Vec::with_capacity(dim);
    for idx in 0..pairs {
        let frequency = frequencies
            .get(idx)
            .copied()
            .unwrap_or_else(|| 1.0 / 10_000.0_f32.powf(idx as f32 / pairs as f32));
        let phase = frequency * delta;
        out.push(phase.cos() * scale);
        out.push(phase.sin() * scale);
    }
    out.truncate(dim);
    out
}

fn add_position_embedding(row: &mut [f32], position: usize, config: &HotConfig) {
    for (idx, value) in row.iter_mut().enumerate() {
        let frequency =
            1.0 / 10_000.0_f32.powf((idx % config.output_dim) as f32 / config.output_dim as f32);
        let phase = position as f32 * frequency;
        let pos = if idx % 2 == 0 {
            phase.sin()
        } else {
            phase.cos()
        };
        *value += pos * 0.02;
    }
}

fn deterministic_align(input: &[f32], dim: usize, salt: &str) -> Vec<f32> {
    let mut out = (0..dim)
        .map(|idx| stable_noise(salt, idx) * 0.01)
        .collect::<Vec<_>>();
    if input.is_empty() {
        return out;
    }
    for (idx, value) in input.iter().enumerate() {
        let normalized = value.tanh();
        let slot = idx % dim;
        let pass = idx / dim + 1;
        out[slot] += normalized / pass as f32;
        let mixed_slot = (idx * 7 + 3) % dim;
        out[mixed_slot] += normalized * stable_noise(salt, idx + dim) * 0.2;
    }
    clamp_vector(&mut out, 4.0);
    out
}

fn geglu_feed_forward(input: &[f32], output_dim: usize, hidden_dim: usize) -> Vec<f32> {
    if input.is_empty() || output_dim == 0 {
        return Vec::new();
    }
    let hidden_dim = hidden_dim.max(output_dim).max(1);
    let norm = (hidden_dim as f32).sqrt().max(1.0);
    let hidden = (0..hidden_dim)
        .map(|idx| {
            let left = deterministic_projection(input, idx, 13);
            let gate = gelu(deterministic_projection(input, idx, 41));
            left * gate
        })
        .collect::<Vec<_>>();
    let mut out = vec![0.0; output_dim];
    for (dim_idx, slot) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        for (hidden_idx, value) in hidden.iter().enumerate() {
            let sign = if (hidden_idx + dim_idx) % 2 == 0 {
                1.0
            } else {
                -1.0
            };
            let weight = sign * (0.5 + stable_noise("hot-geglu", hidden_idx + dim_idx).abs());
            sum += value * weight;
        }
        *slot = sum / norm;
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

fn average_rows(rows: &[Vec<f32>], dim: usize) -> Vec<f32> {
    if rows.is_empty() || dim == 0 {
        return vec![0.0; dim];
    }
    let mut out = vec![0.0; dim];
    for row in rows {
        for idx in 0..dim {
            out[idx] += row.get(idx).copied().unwrap_or_default();
        }
    }
    for value in &mut out {
        *value /= rows.len() as f32;
    }
    out
}

fn blend(left: &[f32], right: &[f32], left_gate: f32) -> Vec<f32> {
    let dim = left.len().max(right.len());
    (0..dim)
        .map(|idx| {
            left.get(idx).copied().unwrap_or_default() * left_gate
                + right.get(idx).copied().unwrap_or_default() * (1.0 - left_gate)
        })
        .collect()
}

fn temporal_recency_confidence(as_of: i64, timestamp: i64) -> f32 {
    let delta_days = as_of.saturating_sub(timestamp).max(0) as f32 / 86_400_000.0;
    (1.0 / (1.0 + delta_days / 30.0)).clamp(0.05, 1.0)
}

fn support_edge_id(edge: &HotTemporalEdge) -> String {
    stable_hash(json!({
        "hot_edge": true,
        "source_id": edge.source_id,
        "target_id": edge.target_id,
        "timestamp": edge.timestamp,
        "edge_type": edge.edge_type,
    }))
}

fn temporal_edge_key(edge: &HotTemporalEdge) -> String {
    support_edge_id(edge)
}

fn normalized_hot_edge_type(left: &str, right: &str) -> String {
    if left == right {
        format!("TEMPORAL_INFERRED_{left}")
    } else {
        format!("TEMPORAL_INFERRED_{left}_THEN_{right}")
    }
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
                .collect()
        })
        .unwrap_or_default()
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

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f32>() / values.len() as f32
    }
}

fn positive_mean(values: &[f32]) -> f32 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().map(|value| value.max(0.0)).sum::<f32>() / values.len() as f32
    }
}

fn softmax(logits: &[f32]) -> Vec<f32> {
    if logits.is_empty() {
        return Vec::new();
    }
    let max = logits
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, |acc, value| acc.max(value));
    let exps = logits
        .iter()
        .map(|value| (value - max).exp())
        .collect::<Vec<_>>();
    let sum = exps.iter().sum::<f32>().max(f32::EPSILON);
    exps.into_iter().map(|value| value / sum).collect()
}

fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value).exp())
}

fn gelu(value: f32) -> f32 {
    0.5 * value * (1.0 + (0.797_884_6 * (value + 0.044_715 * value.powi(3))).tanh())
}

fn relu(value: f32) -> f32 {
    value.max(0.0)
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

fn stable_noise(id: &str, idx: usize) -> f32 {
    let hash = stable_hash(json!({ "id": id, "idx": idx }));
    let mut acc = 0_u32;
    for byte in hash.bytes().take(8) {
        acc = acc.wrapping_mul(33).wrapping_add(u32::from(byte));
    }
    (acc as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn clamp_vector(values: &mut [f32], limit: f32) {
    for value in values {
        if !value.is_finite() {
            *value = 0.0;
        } else {
            *value = value.clamp(-limit, limit);
        }
    }
}

fn average_precision(scored_labels: &[(f32, bool)]) -> f32 {
    let mut rows = scored_labels.to_vec();
    rows.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(Ordering::Equal));
    let positives = rows.iter().filter(|(_, label)| *label).count();
    if positives == 0 {
        return 0.0;
    }
    let mut hits = 0_usize;
    let mut precision_sum = 0.0;
    for (rank, (_, label)) in rows.iter().enumerate() {
        if *label {
            hits += 1;
            precision_sum += hits as f32 / (rank + 1) as f32;
        }
    }
    precision_sum / positives as f32
}

fn auc_roc(scored_labels: &[(f32, bool)]) -> f32 {
    let positives = scored_labels.iter().filter(|(_, label)| *label).count();
    let negatives = scored_labels.len().saturating_sub(positives);
    if positives == 0 || negatives == 0 {
        return 0.0;
    }
    let mut wins = 0.0;
    for (pos_score, pos_label) in scored_labels.iter().filter(|(_, label)| *label) {
        debug_assert!(*pos_label);
        for (neg_score, _) in scored_labels.iter().filter(|(_, label)| !*label) {
            wins += match pos_score.partial_cmp(neg_score).unwrap_or(Ordering::Equal) {
                Ordering::Greater => 1.0,
                Ordering::Equal => 0.5,
                Ordering::Less => 0.0,
            };
        }
    }
    wins / (positives * negatives) as f32
}

fn logistic_loss(scored_labels: &[(f32, bool)]) -> f32 {
    if scored_labels.is_empty() {
        return 0.0;
    }
    let total = scored_labels
        .iter()
        .map(|(score, label)| {
            let prediction = score.clamp(1e-6, 1.0 - 1e-6);
            let label = if *label { 1.0 } else { 0.0 };
            -(label * prediction.ln() + (1.0 - label) * (1.0 - prediction).ln())
        })
        .sum::<f32>();
    total / scored_labels.len() as f32
}

fn normalized_training_features(features: &[f32], input_dim: usize) -> Vec<f32> {
    let mut out = vec![0.0; input_dim];
    for (idx, slot) in out.iter_mut().enumerate() {
        *slot = features.get(idx).copied().unwrap_or_default().tanh();
    }
    out
}
