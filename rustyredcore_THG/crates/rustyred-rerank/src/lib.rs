//! Reranker-backed [`Scorer`](rustyred_membrane::Scorer) implementations.
//!
//! v1 keeps model execution behind [`CrossEncoder`]. The hot-path default is a
//! SequenceClassification-style single forward pass. Causal-LM rerankers can be
//! benchmarked elsewhere, but they do not sit on the default gate path.

pub mod cross_encoder;

use rustyred_membrane::{Candidate, ScoreContext, Scorer};

pub use cross_encoder::{
    select_small_cpu_sequence_classifier, BenchmarkLedger, CrossEncoder, HttpCrossEncoder,
    HttpListwiseReranker, LexicalCrossEncoder, ListwiseReranker, ModelBenchmark,
    NoopListwiseReranker, SequenceClassificationModel, BGE_RERANKER_V2_M3,
    GTE_RERANKER_MODERNBERT_BASE, JINA_RERANKER_V3,
};

pub const LISTWISE_RANK_SCORE_KEY: &str = "listwise_rank_score";
pub const DEFAULT_LISTWISE_RANK_WEIGHT: f32 = 0.20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArmWeights {
    pub relevance: f32,
    pub ppr: f32,
    pub epistemic: f32,
}

impl ArmWeights {
    pub const fn web() -> Self {
        Self {
            relevance: 0.78,
            ppr: 0.15,
            epistemic: 0.07,
        }
    }

    pub const fn code() -> Self {
        Self {
            relevance: 0.25,
            ppr: 0.70,
            epistemic: 0.05,
        }
    }

    pub const fn balanced() -> Self {
        Self {
            relevance: 0.55,
            ppr: 0.35,
            epistemic: 0.10,
        }
    }
}

pub struct RerankScorer {
    pub cross_encoder: Box<dyn CrossEncoder>,
    pub weights: ArmWeights,
}

impl RerankScorer {
    pub fn new(cross_encoder: Box<dyn CrossEncoder>, weights: ArmWeights) -> Self {
        Self {
            cross_encoder,
            weights,
        }
    }

    pub fn web(cross_encoder: Box<dyn CrossEncoder>) -> Self {
        Self::new(cross_encoder, ArmWeights::web())
    }

    pub fn code(cross_encoder: Box<dyn CrossEncoder>) -> Self {
        Self::new(cross_encoder, ArmWeights::code())
    }

    pub fn version(&self) -> String {
        format!("{}:membrane-v1", self.cross_encoder.model_id())
    }
}

impl Scorer for RerankScorer {
    fn score(&self, c: &Candidate, ctx: &ScoreContext<'_>) -> f32 {
        let relevance = self.cross_encoder.score(ctx.query, &c.text);
        self.weights.relevance * relevance
            + self.weights.ppr * c.ppr_proximity
            + self.weights.epistemic * c.epistemic.combined()
    }
}

pub struct ListwiseRankScorer<'a> {
    base: &'a dyn Scorer,
    weight: f32,
}

impl<'a> ListwiseRankScorer<'a> {
    pub fn new(base: &'a dyn Scorer) -> Self {
        Self::with_weight(base, DEFAULT_LISTWISE_RANK_WEIGHT)
    }

    pub fn with_weight(base: &'a dyn Scorer, weight: f32) -> Self {
        Self {
            base,
            weight: weight.max(0.0),
        }
    }
}

impl Scorer for ListwiseRankScorer<'_> {
    fn score(&self, c: &Candidate, ctx: &ScoreContext<'_>) -> f32 {
        let listwise = c
            .metadata
            .get(LISTWISE_RANK_SCORE_KEY)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        self.base.score(c, ctx) + self.weight * listwise
    }
}

pub fn stamp_listwise_rank(mut candidates: Vec<Candidate>) -> Vec<Candidate> {
    let len = candidates.len();
    if len == 0 {
        return candidates;
    }
    let denom = len.max(1) as f32;
    for (index, candidate) in candidates.iter_mut().enumerate() {
        let score = (len - index) as f32 / denom;
        candidate.metadata.insert(
            LISTWISE_RANK_SCORE_KEY.to_string(),
            format!("{score:.6}"),
        );
    }
    candidates
}

#[cfg(test)]
mod tests {
    use rustyred_membrane::{Candidate, EpistemicFeatures, ScoreContext, Scorer};

    use super::*;

    #[test]
    fn web_weights_are_relevance_dominant() {
        let scorer = RerankScorer::web(Box::new(LexicalCrossEncoder::new("test-web")));
        let active = Vec::new();
        let ctx = ScoreContext::new("modernbert reranker", &active).without_redundancy();
        let mut relevant = Candidate::new("relevant", "modernbert reranker", 4);
        relevant.ppr_proximity = 0.1;
        let mut central = Candidate::new("central", "unrelated", 4);
        central.ppr_proximity = 1.0;

        assert!(scorer.score(&relevant, &ctx) > scorer.score(&central, &ctx));
    }

    #[test]
    fn code_weights_are_ppr_dominant() {
        let scorer = RerankScorer::code(Box::new(LexicalCrossEncoder::new("test-code")));
        let active = Vec::new();
        let ctx = ScoreContext::new("parser", &active).without_redundancy();
        let mut relevant = Candidate::new("relevant", "parser", 4);
        relevant.ppr_proximity = 0.1;
        let mut central = Candidate::new("central", "unrelated", 4);
        central.ppr_proximity = 1.0;
        central.epistemic = EpistemicFeatures {
            source_reliability: Some(1.0),
            support_ratio: None,
        };

        assert!(scorer.score(&central, &ctx) > scorer.score(&relevant, &ctx));
    }

    #[test]
    fn listwise_rank_scorer_preserves_scorer_seam() {
        let base = RerankScorer::web(Box::new(LexicalCrossEncoder::new("test-web")));
        let active = Vec::new();
        let ctx = ScoreContext::new("parser", &active).without_redundancy();
        let first = Candidate::new("first", "parser", 1);
        let second = Candidate::new("second", "parser", 1);

        let ranked = stamp_listwise_rank(vec![second.clone(), first.clone()]);
        let scorer = ListwiseRankScorer::with_weight(&base, 0.2);

        assert!(scorer.score(&ranked[0], &ctx) > scorer.score(&ranked[1], &ctx));
    }
}
