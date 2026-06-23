//! Compaction-fitness scorer: the eviction-side [`Scorer`] that feeds the same
//! [`fill_to_budget`](crate::fill_to_budget) as admission.
//!
//! Admission control (the membrane gate proper) scores with a graph-aware
//! reranker. Eviction control scores with *compaction fitness*: recency,
//! reference count, and proximity to the active set. It carries no
//! cross-encoder. Both are `Scorer` implementations and both feed the single
//! fill, which is the architectural invariant of SPEC-CONTEXT-MEMBRANE-1.0:
//! two scorers, one fill, one graph tier underneath.
//!
//! SPEC-CONTEXT-COMPACTION-1.0 `page_back` becomes a caller of `fill_to_budget`
//! with this scorer rather than carrying its own greedy fill, which is what
//! keeps a second fill from drifting into the tree. This scorer lives in the
//! membrane (rather than a not-yet-in-tree compaction crate) so the shared fill
//! and its eviction scorer ship together and stay reconciled.

use crate::gate::{fill_to_budget, Admission};
use crate::scorer::{Candidate, ScoreContext, Scorer};

/// Metadata key carrying a normalized recency value in `0.0..=1.0` (1.0 =
/// most recent). Read from [`Candidate::metadata`].
pub const RECENCY_NORM_KEY: &str = "recency_norm";
/// Metadata key carrying an integer reference count (how many live turns or
/// nodes still point at this context).
pub const REFERENCE_COUNT_KEY: &str = "reference_count";

/// Relative weights for eviction fitness. They need not sum to 1.0; the fill
/// only compares scores within one invocation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompactionWeights {
    pub recency: f32,
    pub reference_count: f32,
    pub proximity: f32,
}

impl Default for CompactionWeights {
    fn default() -> Self {
        Self {
            recency: 0.4,
            reference_count: 0.3,
            proximity: 0.3,
        }
    }
}

/// Eviction-fitness scorer. High score == worth keeping in the window; low
/// score == safe to page back to a graph handle.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompactionFitnessScorer {
    pub weights: CompactionWeights,
}

impl CompactionFitnessScorer {
    pub fn new(weights: CompactionWeights) -> Self {
        Self { weights }
    }
}

impl Scorer for CompactionFitnessScorer {
    fn score(&self, c: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
        let recency = c
            .metadata
            .get(RECENCY_NORM_KEY)
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0);
        // Reference count is unbounded; squash to 0..1 so one heavily-referenced
        // item cannot swamp the recency and proximity terms.
        let reference_count = c
            .metadata
            .get(REFERENCE_COUNT_KEY)
            .and_then(|value| value.parse::<f32>().ok())
            .map(|count| {
                let count = count.max(0.0);
                count / (1.0 + count)
            })
            .unwrap_or(0.0);
        let proximity = c.ppr_proximity.clamp(0.0, 1.0);

        self.weights.recency * recency
            + self.weights.reference_count * reference_count
            + self.weights.proximity * proximity
    }
}

/// SPEC-CONTEXT-COMPACTION `page_back` entry point. It pages lower-fitness
/// context back to recoverable handles by calling the shared fill, not a second
/// greedy implementation.
pub fn page_back(
    candidates: Vec<Candidate>,
    ctx: &ScoreContext<'_>,
    budget_tokens: usize,
) -> Admission {
    page_back_with_scorer(
        candidates,
        &CompactionFitnessScorer::default(),
        ctx,
        budget_tokens,
    )
}

pub fn page_back_with_scorer(
    candidates: Vec<Candidate>,
    scorer: &dyn Scorer,
    ctx: &ScoreContext<'_>,
    budget_tokens: usize,
) -> Admission {
    fill_to_budget(candidates, scorer, ctx, budget_tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scorer::{Candidate, ScoreContext};

    fn ctx_item(id: &str, recency: f32, refs: u32, proximity: f32, tokens: usize) -> Candidate {
        let mut candidate = Candidate::new(id, format!("turn {id}"), tokens);
        candidate.ppr_proximity = proximity;
        candidate
            .metadata
            .insert(RECENCY_NORM_KEY.to_string(), recency.to_string());
        candidate
            .metadata
            .insert(REFERENCE_COUNT_KEY.to_string(), refs.to_string());
        candidate
    }

    #[test]
    fn eviction_keeps_recent_referenced_proximate_context() {
        // Same shared fill as admission; only the scorer differs.
        let candidates = vec![
            ctx_item("stale", 0.05, 0, 0.0, 4),
            ctx_item("hot", 0.95, 6, 0.9, 4),
            ctx_item("warm", 0.5, 2, 0.4, 4),
        ];
        let active = Vec::new();
        let ctx = ScoreContext::new("task", &active).without_redundancy();
        let scorer = CompactionFitnessScorer::default();

        // Budget for two of three items: the stale one pages back.
        let admission = page_back_with_scorer(candidates, &scorer, &ctx, 8);
        let kept: Vec<&str> = admission
            .admitted
            .iter()
            .map(|candidate| candidate.node_id.as_str())
            .collect();
        assert_eq!(kept, vec!["hot", "warm"]);
        assert_eq!(admission.deferred[0].node_id, "stale");
    }

    #[test]
    fn page_back_uses_default_compaction_scorer() {
        let candidates = vec![
            ctx_item("stale", 0.0, 0, 0.0, 4),
            ctx_item("recent", 1.0, 0, 0.0, 4),
        ];
        let active = Vec::new();
        let ctx = ScoreContext::new("task", &active).without_redundancy();

        let admission = page_back(candidates, &ctx, 4);

        assert_eq!(admission.admitted[0].node_id, "recent");
        assert_eq!(admission.deferred[0].node_id, "stale");
    }
}
