//! Trainable Pairformer over Burn (`pairformer-burn-cubecl` + autodiff).
//!
//! This is the learned counterpart to the deterministic structural scorer in
//! `pairformer.rs`: the same AF3 block structure (gated triangle
//! multiplicative updates, multi-head triangle attention biased by the third
//! edge, SwiGLU transitions, single attention with pair bias, no pair
//! flow-back from single), but with real `Param` weights, a self-supervised
//! masked-edge training objective, Recorder persistence, and registration in
//! the model-artifact graph. The bounded-space rule is unchanged: output is
//! link scores over an extracted neighborhood; admission still flows through
//! the quarantine pipeline with provenance.

use std::collections::BTreeSet;
use std::path::Path;

use burn::module::{AutodiffModule, Module};
use burn::nn::{LayerNorm, LayerNormConfig, Linear, LinearConfig};
use burn::optim::{AdamWConfig, GradientsParams, Optimizer};
use burn::record::{BinFileRecorder, FullPrecisionSettings};
use burn::tensor::activation::{log_sigmoid, sigmoid, silu, softmax};
use burn::tensor::backend::{AutodiffBackend, Backend};
use burn::tensor::{Int, Tensor, TensorData};

use serde::{Deserialize, Serialize};
use serde_json::json;

use rustyred_thg_core::{GraphSnapshot, ThgError, ThgResult};

use crate::pairformer::{best_two_hop_support_paths, PairformerInput, PairformerLinkScore};
use crate::reflexive::{
    bounded_neighborhood, pairformer_input_from_graph, pairformer_score_to_candidate,
    DensificationRequest, DensificationResult,
};
use crate::training_substrate::{
    register_model_artifact, ModelArtifactInput, ModelWritebackResult,
};
use crate::types::AdapterGraphStore;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct BurnPairformerConfig {
    pub node_in_dim: usize,
    pub edge_in_dim: usize,
    pub pair_dim: usize,
    pub single_dim: usize,
    pub heads: usize,
    pub blocks: usize,
    pub transition_mult: usize,
    pub max_nodes: usize,
}

impl Default for BurnPairformerConfig {
    fn default() -> Self {
        Self {
            node_in_dim: 16,
            edge_in_dim: 8,
            pair_dim: 32,
            single_dim: 32,
            heads: 4,
            blocks: 2,
            transition_mult: 4,
            max_nodes: 64,
        }
    }
}

impl BurnPairformerConfig {
    pub fn normalized(mut self) -> Self {
        if self.node_in_dim == 0 {
            self.node_in_dim = 16;
        }
        if self.edge_in_dim == 0 {
            self.edge_in_dim = 8;
        }
        if self.pair_dim == 0 {
            self.pair_dim = 32;
        }
        if self.heads == 0 {
            self.heads = 1;
        }
        // Heads must divide the pair and single dims for the reshape.
        while self.pair_dim % self.heads != 0 {
            self.heads -= 1;
        }
        if self.single_dim == 0 {
            self.single_dim = self.pair_dim;
        }
        while self.single_dim % self.heads != 0 {
            self.single_dim += 1;
        }
        if self.blocks == 0 {
            self.blocks = 1;
        }
        if self.transition_mult == 0 {
            self.transition_mult = 2;
        }
        if self.max_nodes == 0 {
            self.max_nodes = 64;
        }
        self
    }

    /// Channels of the raw edge grid handed to the edge embedder: folded
    /// edge features plus a confidence channel plus a presence indicator.
    pub fn edge_grid_dim(&self) -> usize {
        self.edge_in_dim + 2
    }

    pub fn init<B: Backend>(&self, device: &B::Device) -> BurnPairformer<B> {
        let config = self.normalized();
        let linear = |input: usize, output: usize| LinearConfig::new(input, output).init(device);
        let linear_no_bias = |input: usize, output: usize| {
            LinearConfig::new(input, output)
                .with_bias(false)
                .init(device)
        };
        let blocks = (0..config.blocks)
            .map(|_| PairformerBlock {
                tri_mul_outgoing: TriangleMultiplication::init(&config, device),
                tri_mul_incoming: TriangleMultiplication::init(&config, device),
                tri_att_starting: TriangleAttention::init(&config, device),
                tri_att_ending: TriangleAttention::init(&config, device),
                pair_transition: Transition::init(config.pair_dim, config.transition_mult, device),
                single_attention: SingleAttentionWithPairBias::init(&config, device),
                single_transition: Transition::init(
                    config.single_dim,
                    config.transition_mult,
                    device,
                ),
            })
            .collect();
        BurnPairformer {
            node_embed: linear(config.node_in_dim, config.single_dim),
            pair_left: linear_no_bias(config.single_dim, config.pair_dim),
            pair_right: linear_no_bias(config.single_dim, config.pair_dim),
            edge_embed: linear(config.edge_grid_dim(), config.pair_dim),
            blocks,
            link_norm: LayerNormConfig::new(config.pair_dim).init(device),
            link_head: linear(config.pair_dim, 1),
        }
    }
}

/// Gated triangle multiplicative update (outgoing or incoming, chosen at
/// forward time; each orientation owns its own module instance).
#[derive(Module, Debug)]
pub struct TriangleMultiplication<B: Backend> {
    norm: LayerNorm<B>,
    left_proj: Linear<B>,
    right_proj: Linear<B>,
    left_gate: Linear<B>,
    right_gate: Linear<B>,
    out_norm: LayerNorm<B>,
    out_proj: Linear<B>,
    out_gate: Linear<B>,
}

impl<B: Backend> TriangleMultiplication<B> {
    fn init(config: &BurnPairformerConfig, device: &B::Device) -> Self {
        let c = config.pair_dim;
        Self {
            norm: LayerNormConfig::new(c).init(device),
            left_proj: LinearConfig::new(c, c).init(device),
            right_proj: LinearConfig::new(c, c).init(device),
            left_gate: LinearConfig::new(c, c).init(device),
            right_gate: LinearConfig::new(c, c).init(device),
            out_norm: LayerNormConfig::new(c).init(device),
            out_proj: LinearConfig::new(c, c).init(device),
            out_gate: LinearConfig::new(c, c).init(device),
        }
    }

    /// `outgoing`: t[i,j,c] = sum_k a[i,k,c] * b[j,k,c]
    /// `incoming`: t[i,j,c] = sum_k a[k,i,c] * b[k,j,c]
    fn forward(&self, pair: Tensor<B, 3>, outgoing: bool) -> Tensor<B, 3> {
        let normed = self.norm.forward(pair);
        let a = sigmoid(self.left_gate.forward(normed.clone()))
            * self.left_proj.forward(normed.clone());
        let b = sigmoid(self.right_gate.forward(normed.clone()))
            * self.right_proj.forward(normed.clone());
        // [N, N, C] -> [C, N, N] for per-channel batched matmul.
        let a_chan = a.permute([2, 0, 1]);
        let b_chan = b.permute([2, 0, 1]);
        let product = if outgoing {
            a_chan.matmul(b_chan.swap_dims(1, 2))
        } else {
            a_chan.swap_dims(1, 2).matmul(b_chan)
        };
        let triangle = product.permute([1, 2, 0]);
        let gate = sigmoid(self.out_gate.forward(normed));
        gate * self.out_proj.forward(self.out_norm.forward(triangle))
    }
}

/// Multi-head triangle attention around the starting node, with logits
/// biased by the third edge. The ending-node orientation runs the same
/// module on the transposed pair grid (its own weights) and transposes back.
#[derive(Module, Debug)]
pub struct TriangleAttention<B: Backend> {
    norm: LayerNorm<B>,
    query: Linear<B>,
    key: Linear<B>,
    value: Linear<B>,
    bias: Linear<B>,
    gate: Linear<B>,
    output: Linear<B>,
}

impl<B: Backend> TriangleAttention<B> {
    fn init(config: &BurnPairformerConfig, device: &B::Device) -> Self {
        let c = config.pair_dim;
        Self {
            norm: LayerNormConfig::new(c).init(device),
            query: LinearConfig::new(c, c).with_bias(false).init(device),
            key: LinearConfig::new(c, c).with_bias(false).init(device),
            value: LinearConfig::new(c, c).with_bias(false).init(device),
            bias: LinearConfig::new(c, config.heads)
                .with_bias(false)
                .init(device),
            gate: LinearConfig::new(c, c).init(device),
            output: LinearConfig::new(c, c).init(device),
        }
    }

    fn forward(&self, pair: Tensor<B, 3>, heads: usize) -> Tensor<B, 3> {
        let [n, _, c] = pair.dims();
        let head_dim = c / heads;
        let normed = self.norm.forward(pair);

        let split = |tensor: Tensor<B, 3>| -> Tensor<B, 4> {
            // [N, N, C] -> [N, N, H, Dh] -> [H, N, N, Dh]
            tensor
                .reshape([n, n, heads, head_dim])
                .permute([2, 0, 1, 3])
        };
        let queries = split(self.query.forward(normed.clone()));
        let keys = split(self.key.forward(normed.clone()));
        let values = split(self.value.forward(normed.clone()));

        // logits[h, i, j, k] = q[h,i,j,:].k[h,i,k,:] / sqrt(Dh) + bias[h,j,k]
        let scale = (head_dim as f32).sqrt().max(1.0);
        let logits = queries.matmul(keys.swap_dims(2, 3)) / scale;
        let edge_bias = self
            .bias
            .forward(normed.clone())
            .permute([2, 0, 1])
            .reshape([heads, 1, n, n])
            .expand([heads, n, n, n]);
        let attention = softmax(logits + edge_bias, 3);

        let attended = attention.matmul(values); // [H, N, N, Dh]
        let merged = attended.permute([1, 2, 0, 3]).reshape([n, n, c]);
        let gate = sigmoid(self.gate.forward(normed));
        self.output.forward(gate * merged)
    }
}

/// SwiGLU transition: down(silu(gate(x)) * up(x)), pre-normed.
#[derive(Module, Debug)]
pub struct Transition<B: Backend> {
    norm: LayerNorm<B>,
    gate_proj: Linear<B>,
    up_proj: Linear<B>,
    down_proj: Linear<B>,
}

impl<B: Backend> Transition<B> {
    fn init(dim: usize, mult: usize, device: &B::Device) -> Self {
        let hidden = dim * mult;
        Self {
            norm: LayerNormConfig::new(dim).init(device),
            gate_proj: LinearConfig::new(dim, hidden).with_bias(false).init(device),
            up_proj: LinearConfig::new(dim, hidden).with_bias(false).init(device),
            down_proj: LinearConfig::new(hidden, dim).with_bias(false).init(device),
        }
    }

    fn forward<const D: usize>(&self, input: Tensor<B, D>) -> Tensor<B, D> {
        let normed = self.norm.forward(input);
        self.down_proj
            .forward(silu(self.gate_proj.forward(normed.clone())) * self.up_proj.forward(normed))
    }
}

/// Single-representation attention with additive pair bias. The single
/// stream reads from the pair stream; nothing flows back into the pair.
#[derive(Module, Debug)]
pub struct SingleAttentionWithPairBias<B: Backend> {
    norm_single: LayerNorm<B>,
    norm_pair: LayerNorm<B>,
    query: Linear<B>,
    key: Linear<B>,
    value: Linear<B>,
    pair_bias: Linear<B>,
    gate: Linear<B>,
    output: Linear<B>,
}

impl<B: Backend> SingleAttentionWithPairBias<B> {
    fn init(config: &BurnPairformerConfig, device: &B::Device) -> Self {
        let cs = config.single_dim;
        Self {
            norm_single: LayerNormConfig::new(cs).init(device),
            norm_pair: LayerNormConfig::new(config.pair_dim).init(device),
            query: LinearConfig::new(cs, cs).with_bias(false).init(device),
            key: LinearConfig::new(cs, cs).with_bias(false).init(device),
            value: LinearConfig::new(cs, cs).with_bias(false).init(device),
            pair_bias: LinearConfig::new(config.pair_dim, config.heads)
                .with_bias(false)
                .init(device),
            gate: LinearConfig::new(cs, cs).init(device),
            output: LinearConfig::new(cs, cs).init(device),
        }
    }

    fn forward(&self, single: Tensor<B, 2>, pair: Tensor<B, 3>, heads: usize) -> Tensor<B, 2> {
        let [n, cs] = single.dims();
        let head_dim = cs / heads;
        let normed = self.norm_single.forward(single);

        let split = |tensor: Tensor<B, 2>| -> Tensor<B, 3> {
            // [N, Cs] -> [N, H, Dh] -> [H, N, Dh]
            tensor.reshape([n, heads, head_dim]).permute([1, 0, 2])
        };
        let queries = split(self.query.forward(normed.clone()));
        let keys = split(self.key.forward(normed.clone()));
        let values = split(self.value.forward(normed.clone()));

        let scale = (head_dim as f32).sqrt().max(1.0);
        let logits = queries.matmul(keys.swap_dims(1, 2)) / scale; // [H, N, N]
        let bias = self
            .pair_bias
            .forward(self.norm_pair.forward(pair))
            .permute([2, 0, 1]); // [H, N, N]
        let attention = softmax(logits + bias, 2);

        let attended = attention.matmul(values); // [H, N, Dh]
        let merged = attended.permute([1, 0, 2]).reshape([n, cs]);
        let gate = sigmoid(self.gate.forward(normed));
        self.output.forward(gate * merged)
    }
}

#[derive(Module, Debug)]
pub struct PairformerBlock<B: Backend> {
    tri_mul_outgoing: TriangleMultiplication<B>,
    tri_mul_incoming: TriangleMultiplication<B>,
    tri_att_starting: TriangleAttention<B>,
    tri_att_ending: TriangleAttention<B>,
    pair_transition: Transition<B>,
    single_attention: SingleAttentionWithPairBias<B>,
    single_transition: Transition<B>,
}

impl<B: Backend> PairformerBlock<B> {
    fn forward(
        &self,
        pair: Tensor<B, 3>,
        single: Tensor<B, 2>,
        heads: usize,
    ) -> (Tensor<B, 3>, Tensor<B, 2>) {
        let pair = pair.clone() + self.tri_mul_outgoing.forward(pair, true);
        let pair = pair.clone() + self.tri_mul_incoming.forward(pair, false);
        let pair = pair.clone() + self.tri_att_starting.forward(pair, heads);
        // Ending-node orientation: same computation on the transposed grid
        // with this orientation's own weights, transposed back.
        let pair = pair.clone()
            + self
                .tri_att_ending
                .forward(pair.swap_dims(0, 1), heads)
                .swap_dims(0, 1);
        let pair = pair.clone() + self.pair_transition.forward(pair);
        let single = single.clone() + self.single_attention.forward(single, pair.clone(), heads);
        let single = single.clone() + self.single_transition.forward(single);
        (pair, single)
    }
}

#[derive(Module, Debug)]
pub struct BurnPairformer<B: Backend> {
    node_embed: Linear<B>,
    pair_left: Linear<B>,
    pair_right: Linear<B>,
    edge_embed: Linear<B>,
    blocks: Vec<PairformerBlock<B>>,
    link_norm: LayerNorm<B>,
    link_head: Linear<B>,
}

impl<B: Backend> BurnPairformer<B> {
    /// Forward over featurized inputs: node features `[N, node_in]` and the
    /// raw edge grid `[N, N, edge_in + 2]`. Returns link logits `[N, N]`.
    pub fn forward(
        &self,
        node_features: Tensor<B, 2>,
        edge_grid: Tensor<B, 3>,
        heads: usize,
    ) -> Tensor<B, 2> {
        let [n, _] = node_features.dims();
        let single = self.node_embed.forward(node_features);
        let pair_dim = self.pair_left.weight.val().dims()[1];
        let left = self
            .pair_left
            .forward(single.clone())
            .reshape([n, 1, pair_dim])
            .expand([n, n, pair_dim]);
        let right = self
            .pair_right
            .forward(single.clone())
            .reshape([1, n, pair_dim])
            .expand([n, n, pair_dim]);
        let mut pair = left + right + self.edge_embed.forward(edge_grid);
        let mut single = single;
        for block in self.blocks.iter() {
            let (next_pair, next_single) = block.forward(pair, single, heads);
            pair = next_pair;
            single = next_single;
        }
        self.link_head
            .forward(self.link_norm.forward(pair))
            .reshape([n, n])
    }
}

/// Deterministic fold of variable-length features into a fixed width.
fn fold_features(features: &[f32], dim: usize) -> Vec<f32> {
    let mut out = vec![0.0_f32; dim];
    for (idx, value) in features.iter().enumerate() {
        let pass = (idx / dim + 1) as f32;
        out[idx % dim] += value.tanh() / pass;
    }
    out
}

/// Featurized tensors for one bounded neighborhood. `masked` directed pairs
/// are presented as absent (zero fold, zero confidence, zero indicator) so
/// the model must reconstruct them.
pub fn featurize_pairformer_input<B: Backend>(
    device: &B::Device,
    input: &PairformerInput,
    config: &BurnPairformerConfig,
    masked: &BTreeSet<(usize, usize)>,
) -> ThgResult<(Tensor<B, 2>, Tensor<B, 3>, Vec<String>)> {
    let config = config.normalized();
    let n = input.nodes.len();
    if n == 0 {
        return Err(ThgError::new(
            "invalid_pairformer_input",
            "at least one node is required",
        ));
    }
    if n > config.max_nodes {
        return Err(ThgError::new(
            "pairformer_bound_exceeded",
            format!("input has {n} nodes, above max_nodes {}", config.max_nodes),
        ));
    }
    let node_ids = input
        .nodes
        .iter()
        .map(|node| node.node_id.clone())
        .collect::<Vec<_>>();
    let index = node_ids
        .iter()
        .enumerate()
        .map(|(idx, node_id)| (node_id.clone(), idx))
        .collect::<std::collections::BTreeMap<_, _>>();

    let mut node_flat = Vec::with_capacity(n * config.node_in_dim);
    for node in &input.nodes {
        node_flat.extend(fold_features(&node.features, config.node_in_dim));
    }

    let grid_dim = config.edge_grid_dim();
    let mut edge_flat = vec![0.0_f32; n * n * grid_dim];
    for edge in &input.edges {
        let (Some(&source), Some(&target)) =
            (index.get(&edge.source_id), index.get(&edge.target_id))
        else {
            continue;
        };
        if masked.contains(&(source, target)) {
            continue;
        }
        let base = (source * n + target) * grid_dim;
        let folded = fold_features(&edge.features, config.edge_in_dim);
        for (offset, value) in folded.iter().enumerate() {
            edge_flat[base + offset] += value;
        }
        let confidence = edge.confidence.clamp(0.0, 1.0);
        edge_flat[base + config.edge_in_dim] = edge_flat[base + config.edge_in_dim].max(confidence);
        edge_flat[base + config.edge_in_dim + 1] = 1.0;
    }

    let node_features =
        Tensor::from_data(TensorData::new(node_flat, [n, config.node_in_dim]), device);
    let edge_grid = Tensor::from_data(TensorData::new(edge_flat, [n, n, grid_dim]), device);
    Ok((node_features, edge_grid, node_ids))
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerTrainingConfig {
    pub epochs: usize,
    pub learning_rate: f64,
    pub mask_fraction: f32,
    pub negatives_per_positive: usize,
    pub seed: u64,
}

impl Default for PairformerTrainingConfig {
    fn default() -> Self {
        Self {
            epochs: 60,
            learning_rate: 3e-3,
            mask_fraction: 0.3,
            negatives_per_positive: 2,
            seed: 17,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PairformerTrainingReport {
    pub epochs: usize,
    pub initial_loss: f32,
    pub final_loss: f32,
    pub epoch_losses: Vec<f32>,
    pub final_ranking_accuracy: f32,
    pub positives_per_epoch: usize,
    pub negatives_per_epoch: usize,
}

/// Minimal deterministic RNG for masking and negative sampling: keeps the
/// training loop reproducible without adding a rand dependency.
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self {
            state: seed.max(1).wrapping_mul(0x9E37_79B9_7F4A_7C15),
        }
    }

    fn next(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x
    }

    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound.max(1) as u64) as usize
    }

    fn chance(&mut self, fraction: f32) -> bool {
        (self.next() % 10_000) as f32 / 10_000.0 < fraction
    }
}

/// Train by masked-edge recovery: hide a fraction of observed directed
/// edges, score the hidden pairs against sampled non-edges, and minimize
/// binary cross-entropy on the link logits. The graph is its own label
/// source. Returns the trained autodiff module plus the loss/accuracy
/// report; call `.valid()` on the module for the inference backend.
pub fn train_pairformer<B: AutodiffBackend>(
    device: &B::Device,
    input: &PairformerInput,
    model_config: BurnPairformerConfig,
    training: PairformerTrainingConfig,
) -> ThgResult<(BurnPairformer<B>, PairformerTrainingReport)> {
    let model_config = model_config.normalized();
    B::seed(device, training.seed);
    let mut model = model_config.init::<B>(device);
    let mut optimizer = AdamWConfig::new().init::<B, BurnPairformer<B>>();
    let mut rng = XorShift64::new(training.seed);

    let n = input.nodes.len();
    let index = input
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| (node.node_id.clone(), idx))
        .collect::<std::collections::BTreeMap<_, _>>();
    let directed_edges = input
        .edges
        .iter()
        .filter_map(|edge| {
            let source = *index.get(&edge.source_id)?;
            let target = *index.get(&edge.target_id)?;
            (source != target).then_some((source, target))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if directed_edges.is_empty() {
        return Err(ThgError::new(
            "invalid_pairformer_training",
            "training requires at least one directed edge",
        ));
    }

    let mut epoch_losses = Vec::with_capacity(training.epochs);
    let mut final_ranking_accuracy = 0.0_f32;
    let mut positives_per_epoch = 0usize;
    let mut negatives_per_epoch = 0usize;

    for _ in 0..training.epochs.max(1) {
        // Sample this epoch's masked positives (at least one).
        let mut masked: BTreeSet<(usize, usize)> = BTreeSet::new();
        for edge in &directed_edges {
            if rng.chance(training.mask_fraction) {
                masked.insert(*edge);
            }
        }
        if masked.is_empty() {
            masked.insert(directed_edges[rng.below(directed_edges.len())]);
        }
        let positives = masked.iter().copied().collect::<Vec<_>>();

        // Sample non-edge negatives (excluding self-pairs and any edge).
        let edge_set = directed_edges.iter().copied().collect::<BTreeSet<_>>();
        let mut negatives = Vec::new();
        let wanted = positives.len() * training.negatives_per_positive.max(1);
        let mut attempts = 0usize;
        while negatives.len() < wanted && attempts < wanted * 50 {
            attempts += 1;
            let source = rng.below(n);
            let target = rng.below(n);
            if source == target || edge_set.contains(&(source, target)) {
                continue;
            }
            negatives.push((source, target));
        }
        positives_per_epoch = positives.len();
        negatives_per_epoch = negatives.len();

        let (node_features, edge_grid, _) =
            featurize_pairformer_input::<B>(device, input, &model_config, &masked)?;
        let logits = model.forward(node_features, edge_grid, model_config.heads);
        let flat = logits.reshape([n * n]);

        let mut sample_indices = Vec::with_capacity(positives.len() + negatives.len());
        let mut sample_targets = Vec::with_capacity(positives.len() + negatives.len());
        for (source, target) in positives.iter().chain(&negatives) {
            sample_indices.push((source * n + target) as i64);
        }
        for _ in 0..positives.len() {
            sample_targets.push(1.0_f32);
        }
        for _ in 0..negatives.len() {
            sample_targets.push(0.0_f32);
        }
        let count = sample_indices.len();
        let indices =
            Tensor::<B, 1, Int>::from_data(TensorData::new(sample_indices, [count]), device);
        let targets = Tensor::<B, 1>::from_data(TensorData::new(sample_targets, [count]), device);
        let selected = flat.select(0, indices);

        // Stable BCE-with-logits via log-sigmoid.
        let loss = -(targets.clone() * log_sigmoid(selected.clone())
            + (targets.neg() + 1.0) * log_sigmoid(selected.neg()))
        .mean();
        let loss_value = loss
            .clone()
            .into_data()
            .to_vec::<f32>()
            .map_err(|error| ThgError::new("burn_tensor_readback", format!("{error:?}")))?[0];
        epoch_losses.push(loss_value);

        let grads = GradientsParams::from_grads(loss.backward(), &model);
        model = optimizer.step(training.learning_rate, model, grads);

        // Ranking accuracy on this epoch's samples with the updated model.
        // AutodiffBackend shares its Device type with InnerBackend, so the
        // same device value drives the inference pass.
        let valid_model = model.valid();
        let (node_features, edge_grid, _) =
            featurize_pairformer_input::<B::InnerBackend>(device, input, &model_config, &masked)?;
        let scores = valid_model
            .forward(node_features, edge_grid, model_config.heads)
            .reshape([n * n])
            .into_data()
            .to_vec::<f32>()
            .map_err(|error| ThgError::new("burn_tensor_readback", format!("{error:?}")))?;
        final_ranking_accuracy = ranking_accuracy(&scores, n, &positives, &negatives);
    }

    let report = PairformerTrainingReport {
        epochs: training.epochs.max(1),
        initial_loss: epoch_losses.first().copied().unwrap_or(0.0),
        final_loss: epoch_losses.last().copied().unwrap_or(0.0),
        epoch_losses,
        final_ranking_accuracy,
        positives_per_epoch,
        negatives_per_epoch,
    };
    Ok((model, report))
}

/// Fraction of (positive, negative) pairs ranked correctly.
pub fn ranking_accuracy(
    flat_scores: &[f32],
    n: usize,
    positives: &[(usize, usize)],
    negatives: &[(usize, usize)],
) -> f32 {
    if positives.is_empty() || negatives.is_empty() {
        return 0.0;
    }
    let mut correct = 0usize;
    let mut total = 0usize;
    for (ps, pt) in positives {
        let positive_score = flat_scores[ps * n + pt];
        for (ns, nt) in negatives {
            total += 1;
            if positive_score > flat_scores[ns * n + nt] {
                correct += 1;
            }
        }
    }
    correct as f32 / total.max(1) as f32
}

/// Score all links of a bounded neighborhood with a trained model. Scores
/// are sigmoid(logit); two-hop support paths attach provenance where they
/// exist, which the candidate conversion requires for admission.
pub fn score_links_with_trained<B: Backend>(
    model: &BurnPairformer<B>,
    device: &B::Device,
    input: &PairformerInput,
    config: &BurnPairformerConfig,
) -> ThgResult<Vec<PairformerLinkScore>> {
    let config = config.normalized();
    let (node_features, edge_grid, node_ids) =
        featurize_pairformer_input::<B>(device, input, &config, &BTreeSet::new())?;
    let n = node_ids.len();
    let scores = model
        .forward(node_features, edge_grid, config.heads)
        .reshape([n * n])
        .into_data()
        .to_vec::<f32>()
        .map_err(|error| ThgError::new("burn_tensor_readback", format!("{error:?}")))?;
    let support_paths = best_two_hop_support_paths(input);

    let mut link_scores = Vec::new();
    for (source_idx, source_id) in node_ids.iter().enumerate() {
        for (target_idx, target_id) in node_ids.iter().enumerate() {
            if source_idx == target_idx {
                continue;
            }
            link_scores.push(PairformerLinkScore {
                source_id: source_id.clone(),
                target_id: target_id.clone(),
                score: 1.0 / (1.0 + (-scores[source_idx * n + target_idx]).exp()),
                support_path: support_paths
                    .get(&(source_id.clone(), target_id.clone()))
                    .cloned(),
            });
        }
    }
    Ok(link_scores)
}

/// Trained-model counterpart of `rank_pairformer_densification_candidates`:
/// same bounded neighborhood extraction, same direct-edge suppression, same
/// quarantine-bound candidate output; only the scorer is learned.
pub fn rank_trained_pairformer_densification_candidates<B: Backend>(
    snapshot: &GraphSnapshot,
    request: DensificationRequest,
    model: &BurnPairformer<B>,
    device: &B::Device,
    config: &BurnPairformerConfig,
) -> ThgResult<DensificationResult> {
    let request = request.normalized();
    let config = config.normalized();
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
        .collect::<std::collections::BTreeMap<_, _>>();
    let allowed = request
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
                && (allowed.is_empty() || allowed.contains(&edge.edge_type))
                && nodes_by_id.contains_key(&edge.from_id)
                && nodes_by_id.contains_key(&edge.to_id)
        })
        .collect::<Vec<_>>();

    let (mut considered, mut bounded) = bounded_neighborhood(&request, &edge_refs);
    considered.retain(|node_id| nodes_by_id.contains_key(node_id));
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
    let link_scores = score_links_with_trained(model, device, &input, &config)?;

    let mut candidates = link_scores
        .iter()
        .filter_map(|score| pairformer_score_to_candidate(&request, score, &existing_direct_pairs))
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
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

/// Persist trained weights with the binary file recorder.
pub fn save_pairformer_file<B: Backend>(model: BurnPairformer<B>, path: &Path) -> ThgResult<()> {
    model
        .save_file(
            path.to_path_buf(),
            &BinFileRecorder::<FullPrecisionSettings>::default(),
        )
        .map_err(|error| ThgError::new("pairformer_save_failed", format!("{error:?}")))
}

/// Load trained weights into a freshly initialized module of the same config.
pub fn load_pairformer_file<B: Backend>(
    config: &BurnPairformerConfig,
    path: &Path,
    device: &B::Device,
) -> ThgResult<BurnPairformer<B>> {
    config
        .normalized()
        .init::<B>(device)
        .load_file(
            path.to_path_buf(),
            &BinFileRecorder::<FullPrecisionSettings>::default(),
            device,
        )
        .map_err(|error| ThgError::new("pairformer_load_failed", format!("{error:?}")))
}

/// Register a trained Pairformer in the model-artifact graph: weights live
/// in the file at `artifact_uri`, never in graph nodes; the artifact node
/// carries config, metrics, and provenance.
#[allow(clippy::too_many_arguments)]
pub fn register_trained_pairformer_artifact<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    model_id: &str,
    artifact_uri: &str,
    config: &BurnPairformerConfig,
    report: &PairformerTrainingReport,
    trained_on_node_ids: Vec<String>,
    source_graph_version: u64,
    actor: Option<&str>,
) -> ThgResult<ModelWritebackResult> {
    let dataset_hash = rustyred_thg_core::stable_hash(json!({
        "trained_on_node_ids": trained_on_node_ids,
        "source_graph_version": source_graph_version,
    }));
    register_model_artifact(
        store,
        ModelArtifactInput {
            model_id: model_id.to_string(),
            tenant_id: tenant_id.to_string(),
            model_type: "pairformer-burn".to_string(),
            s3_uri: artifact_uri.to_string(),
            dataset_hash,
            source_graph_version,
            trained_on_node_ids,
            metrics: json!({
                "initial_loss": report.initial_loss,
                "final_loss": report.final_loss,
                "final_ranking_accuracy": report.final_ranking_accuracy,
                "epochs": report.epochs,
                "config": config,
            }),
            promotion_decision: "candidate".to_string(),
            manifest_version: 1,
        },
        actor,
    )
}
