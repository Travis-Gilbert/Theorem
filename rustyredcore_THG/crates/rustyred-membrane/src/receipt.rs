use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Source {
    Web,
    Code,
    Compaction,
    Other,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MembraneReceipt {
    pub source: Source,
    pub candidates_scored: usize,
    pub tokens_admitted: usize,
    pub tokens_deferred: usize,
    pub reranker_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_token_delta_vs_baseline: Option<i64>,
}

impl MembraneReceipt {
    pub fn content_address(&self) -> String {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        blake3::hash(&bytes).to_hex().to_string()
    }
}
