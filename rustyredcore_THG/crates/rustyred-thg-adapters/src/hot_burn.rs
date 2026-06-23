//! Burn learning lane for HOT.
//!
//! The default `hot.rs` scorer is the deterministic oracle/scaffold.  This
//! module is the trainable decoder surface for GPU/full-precision runs: it
//! consumes HOT pair representations and learns the link-probability head with
//! Burn parameters.  The bounded extraction, patching, co-occurrence, and BRT
//! reference encoder stay shared with the CPU lane so parity tests can compare
//! features before swapping in heavier tensor kernels.

use burn::module::Module;
use burn::nn::{Linear, LinearConfig};
use burn::tensor::activation::{sigmoid, silu};
use burn::tensor::backend::Backend;
use burn::tensor::{Tensor, TensorData};

use serde::{Deserialize, Serialize};

use rustyred_thg_core::{ThgError, ThgResult};

use crate::hot::{HotLinkTrainingExample, HotTrainingConfig};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct BurnHotConfig {
    pub input_dim: usize,
    pub hidden_dim: usize,
}

impl BurnHotConfig {
    pub fn from_training_config(config: &HotTrainingConfig, input_dim: usize) -> Self {
        Self {
            input_dim,
            hidden_dim: config.model.decoder_hidden_dim.max(1),
        }
    }

    pub fn normalized(mut self) -> Self {
        if self.input_dim == 0 {
            self.input_dim = 1;
        }
        if self.hidden_dim == 0 {
            self.hidden_dim = 1;
        }
        self
    }

    pub fn init<B: Backend>(&self, device: &B::Device) -> BurnHot<B> {
        let config = self.normalized();
        BurnHot {
            hidden_gate: LinearConfig::new(config.input_dim, config.hidden_dim).init(device),
            hidden_up: LinearConfig::new(config.input_dim, config.hidden_dim)
                .with_bias(false)
                .init(device),
            output: LinearConfig::new(config.hidden_dim, 1).init(device),
        }
    }
}

#[derive(Module, Debug)]
pub struct BurnHot<B: Backend> {
    hidden_gate: Linear<B>,
    hidden_up: Linear<B>,
    output: Linear<B>,
}

impl<B: Backend> BurnHot<B> {
    /// Forward over HOT pair representations `[batch, input_dim]`, returning
    /// calibrated link probabilities `[batch, 1]`.
    pub fn forward(&self, pair_features: Tensor<B, 2>) -> Tensor<B, 2> {
        let hidden = silu(self.hidden_gate.forward(pair_features.clone()))
            * self.hidden_up.forward(pair_features);
        sigmoid(self.output.forward(hidden))
    }
}

pub fn featurize_hot_training_examples<B: Backend>(
    device: &B::Device,
    examples: &[HotLinkTrainingExample],
    config: BurnHotConfig,
) -> ThgResult<(Tensor<B, 2>, Tensor<B, 2>)> {
    let config = config.normalized();
    if examples.is_empty() {
        return Err(ThgError::new(
            "invalid_hot_training_data",
            "at least one HOT training example is required",
        ));
    }
    let mut features = Vec::with_capacity(examples.len() * config.input_dim);
    let mut labels = Vec::with_capacity(examples.len());
    for example in examples {
        for idx in 0..config.input_dim {
            features.push(example.features.get(idx).copied().unwrap_or_default().tanh());
        }
        labels.push(if example.label { 1.0 } else { 0.0 });
    }
    Ok((
        Tensor::<B, 2>::from_data(
            TensorData::new(features, [examples.len(), config.input_dim]),
            device,
        ),
        Tensor::<B, 2>::from_data(TensorData::new(labels, [examples.len(), 1]), device),
    ))
}
