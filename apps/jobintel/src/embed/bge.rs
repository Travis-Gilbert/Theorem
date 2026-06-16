//! Real local sentence-transformers encoder: bge-small-en-v1.5 (D=384) via
//! candle. This is the spec's named default model. It is feature-gated (`bge`)
//! so the heavy candle + tokenizers + hf-hub dependency tree, and the ~130MB
//! weight download, are opt-in and never burden the offline demo path.
//!
//! Recipe (BAAI bge model card): tokenize -> BERT forward -> CLS pooling (the
//! [CLS] token's last hidden state) -> L2 normalize. Weights are pulled from
//! the HF hub once at construction; `embed()` only runs inference afterward.

use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use hf_hub::api::sync::Api;
use hf_hub::{Repo, RepoType};
use tokenizers::Tokenizer;

use super::Embedder;
use crate::config::Config as JobConfig;
use crate::error::{JobIntelError, Result};

const MODEL_ID: &str = "BAAI/bge-small-en-v1.5";

pub struct BgeEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    dim: usize,
}

impl BgeEmbedder {
    pub fn new(config: &JobConfig) -> Result<Self> {
        let device = Device::Cpu;
        let repo = Api::new()
            .map_err(embed_err)?
            .repo(Repo::new(MODEL_ID.to_string(), RepoType::Model));

        let config_path = repo.get("config.json").map_err(embed_err)?;
        let tokenizer_path = repo.get("tokenizer.json").map_err(embed_err)?;
        let weights_path = repo.get("model.safetensors").map_err(embed_err)?;

        let bert_config: Config = serde_json::from_str(&std::fs::read_to_string(config_path)?)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(embed_err)?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
                .map_err(embed_err)?
        };
        let model = BertModel::load(vb, &bert_config).map_err(embed_err)?;
        let dim = bert_config.hidden_size;
        if dim != config.embed_dim {
            eprintln!(
                "  note: bge hidden_size={} differs from JOBINTEL_EMBED_DIM={}; using {}",
                dim, config.embed_dim, dim
            );
        }

        Ok(Self {
            model,
            tokenizer,
            device,
            dim,
        })
    }

    fn encode(&self, text: &str) -> Result<Vec<f32>> {
        let tokens = self.tokenizer.encode(text, true).map_err(embed_err)?;
        let ids = tokens.get_ids().to_vec();
        let token_ids = Tensor::new(ids.as_slice(), &self.device)
            .map_err(embed_err)?
            .unsqueeze(0)
            .map_err(embed_err)?;
        let token_type_ids = token_ids.zeros_like().map_err(embed_err)?;

        // [1, seq, hidden]
        let hidden = self
            .model
            .forward(&token_ids, &token_type_ids, None)
            .map_err(embed_err)?;
        // CLS pooling: take the first token, [1, 1, hidden] -> [1, hidden].
        let cls = hidden
            .narrow(1, 0, 1)
            .map_err(embed_err)?
            .squeeze(1)
            .map_err(embed_err)?;
        let normalized = normalize_l2(&cls).map_err(embed_err)?;
        let vec = normalized
            .squeeze(0)
            .map_err(embed_err)?
            .to_vec1::<f32>()
            .map_err(embed_err)?;
        Ok(vec)
    }
}

impl Embedder for BgeEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.encode(text)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "bge-small-en-v1.5"
    }
}

fn normalize_l2(t: &Tensor) -> candle_core::Result<Tensor> {
    let norm = t.sqr()?.sum_keepdim(1)?.sqrt()?;
    t.broadcast_div(&norm)
}

fn embed_err<E: std::fmt::Display>(e: E) -> JobIntelError {
    JobIntelError::Embed(e.to_string())
}
