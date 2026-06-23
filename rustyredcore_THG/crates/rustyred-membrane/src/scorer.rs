use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Where a candidate entered the membrane. Scorers may weight arms differently.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceArm {
    Web,
    Code,
    Compaction,
    #[default]
    Other,
}

/// Lightweight epistemic features already known by the graph.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EpistemicFeatures {
    pub source_reliability: Option<f32>,
    pub support_ratio: Option<f32>,
}

impl EpistemicFeatures {
    /// Blend known epistemic signals onto a 0..1 scale. Unknown signals add no
    /// mass rather than pretending confidence.
    pub fn combined(&self) -> f32 {
        let mut sum = 0.0;
        let mut count = 0.0;
        if let Some(source_reliability) = self.source_reliability {
            sum += source_reliability.clamp(0.0, 1.0);
            count += 1.0;
        }
        if let Some(support_ratio) = self.support_ratio {
            sum += support_ratio.clamp(0.0, 1.0);
            count += 1.0;
        }
        if count == 0.0 {
            0.0
        } else {
            sum / count
        }
    }
}

/// A graph-resident piece of realized context.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidate {
    pub node_id: String,
    pub text: String,
    pub token_count: usize,
    pub ppr_proximity: f32,
    pub epistemic: EpistemicFeatures,
    /// Optional exact redundancy bucket, e.g. URL host, file path, or canonical
    /// claim id. The fill also computes lexical overlap when this is absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redundancy_key: Option<String>,
    #[serde(default)]
    pub source_arm: SourceArm,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl Candidate {
    pub fn new(node_id: impl Into<String>, text: impl Into<String>, token_count: usize) -> Self {
        Self {
            node_id: node_id.into(),
            text: text.into(),
            token_count,
            ppr_proximity: 0.0,
            epistemic: EpistemicFeatures::default(),
            redundancy_key: None,
            source_arm: SourceArm::Other,
            metadata: BTreeMap::new(),
        }
    }

    pub fn with_redundancy_key(mut self, redundancy_key: impl Into<String>) -> Self {
        let redundancy_key = redundancy_key.into();
        if !redundancy_key.trim().is_empty() {
            self.redundancy_key = Some(redundancy_key);
        }
        self
    }

    pub fn with_source_arm(mut self, source_arm: SourceArm) -> Self {
        self.source_arm = source_arm;
        self
    }
}

/// MMR lambda for budget fill. `1.0` is pure score-ordered greedy fill; lower
/// values put more pressure on redundancy. SPEC-SEARCH-RERANK-GATE-1.0 names
/// `0.7` as the v1 default.
pub const DEFAULT_MMR_LAMBDA: f32 = 0.7;
pub const DEFAULT_REDUNDANCY_PENALTY: f32 = 1.0 - DEFAULT_MMR_LAMBDA;

/// Query-local scoring context.
#[derive(Clone, Copy, Debug)]
pub struct ScoreContext<'a> {
    pub query: &'a str,
    pub active_node_ids: &'a [String],
    /// MMR score mass retained for candidate value.
    pub mmr_lambda: f32,
    /// Maximum score mass removed when a candidate duplicates already admitted
    /// context. Set to 0.0 for strict score-only fill.
    ///
    /// Kept for callers that still express the old contract as a penalty. It is
    /// always synchronized with `mmr_lambda` as `1.0 - mmr_lambda`.
    pub redundancy_penalty: f32,
}

impl<'a> ScoreContext<'a> {
    pub fn new(query: &'a str, active_node_ids: &'a [String]) -> Self {
        Self {
            query,
            active_node_ids,
            mmr_lambda: DEFAULT_MMR_LAMBDA,
            redundancy_penalty: DEFAULT_REDUNDANCY_PENALTY,
        }
    }

    pub fn without_redundancy(mut self) -> Self {
        self.mmr_lambda = 1.0;
        self.redundancy_penalty = 0.0;
        self
    }

    pub fn with_mmr_lambda(mut self, mmr_lambda: f32) -> Self {
        self.mmr_lambda = mmr_lambda.clamp(0.0, 1.0);
        self.redundancy_penalty = 1.0 - self.mmr_lambda;
        self
    }

    pub fn with_redundancy_penalty(mut self, redundancy_penalty: f32) -> Self {
        self.redundancy_penalty = redundancy_penalty.clamp(0.0, 1.0);
        self.mmr_lambda = 1.0 - self.redundancy_penalty;
        self
    }
}

pub trait Scorer: Send + Sync {
    fn score(&self, c: &Candidate, ctx: &ScoreContext<'_>) -> f32;
}
