use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::scorer::{Candidate, ScoreContext, Scorer};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Handle {
    pub node_id: String,
    pub digest: String,
    pub token_count: usize,
}

impl Handle {
    pub fn from_candidate(candidate: &Candidate) -> Self {
        Self {
            node_id: candidate.node_id.clone(),
            digest: text_digest(&candidate.text),
            token_count: candidate.token_count,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Admission {
    pub admitted: Vec<Candidate>,
    pub deferred: Vec<Handle>,
    pub tokens_admitted: usize,
    pub tokens_deferred: usize,
}

#[derive(Clone, Debug)]
struct ScoredCandidate {
    candidate: Candidate,
    base_score: f32,
    original_index: usize,
}

/// Shared admission/eviction fill.
///
/// The scorer owns arm-specific value. The fill owns cache mechanics: budget
/// fit, deterministic ordering, lossless overflow handles, and redundancy
/// pressure so a high-scoring cluster does not monopolize the window.
pub fn fill_to_budget(
    candidates: Vec<Candidate>,
    scorer: &dyn Scorer,
    ctx: &ScoreContext<'_>,
    budget_tokens: usize,
) -> Admission {
    let mut pool: Vec<ScoredCandidate> = candidates
        .into_iter()
        .enumerate()
        .map(|(original_index, candidate)| ScoredCandidate {
            base_score: scorer.score(&candidate, ctx),
            candidate,
            original_index,
        })
        .collect();

    pool.sort_by(compare_base_score);

    let mut admitted = Vec::new();
    let mut tokens_admitted = 0usize;
    while !pool.is_empty() && tokens_admitted < budget_tokens {
        let remaining_budget = budget_tokens.saturating_sub(tokens_admitted);
        let mut best_index: Option<usize> = None;
        let mut best_effective = f32::NEG_INFINITY;

        for (index, scored) in pool.iter().enumerate() {
            if scored.candidate.token_count > remaining_budget {
                continue;
            }
            let redundancy = max_redundancy(&scored.candidate, &admitted).clamp(0.0, 1.0);
            let lambda = ctx.mmr_lambda.clamp(0.0, 1.0);
            let effective = lambda * scored.base_score - (1.0 - lambda) * redundancy;
            match best_index {
                None => {
                    best_index = Some(index);
                    best_effective = effective;
                }
                Some(current) => {
                    if compare_effective(effective, scored, best_effective, &pool[current])
                        == Ordering::Less
                    {
                        best_index = Some(index);
                        best_effective = effective;
                    }
                }
            }
        }

        let Some(best_index) = best_index else {
            break;
        };
        let selected = pool.remove(best_index).candidate;
        tokens_admitted += selected.token_count;
        admitted.push(selected);
    }

    let tokens_deferred = pool.iter().map(|scored| scored.candidate.token_count).sum();
    let deferred = pool
        .iter()
        .map(|scored| Handle::from_candidate(&scored.candidate))
        .collect();

    Admission {
        admitted,
        deferred,
        tokens_admitted,
        tokens_deferred,
    }
}

fn compare_base_score(a: &ScoredCandidate, b: &ScoredCandidate) -> Ordering {
    b.base_score
        .partial_cmp(&a.base_score)
        .unwrap_or(Ordering::Equal)
        .then_with(|| a.original_index.cmp(&b.original_index))
        .then_with(|| a.candidate.node_id.cmp(&b.candidate.node_id))
}

/// Return Ordering::Less when the left candidate should win.
fn compare_effective(
    left_effective: f32,
    left: &ScoredCandidate,
    right_effective: f32,
    right: &ScoredCandidate,
) -> Ordering {
    right_effective
        .partial_cmp(&left_effective)
        .unwrap_or(Ordering::Equal)
        .then_with(|| {
            right
                .base_score
                .partial_cmp(&left.base_score)
                .unwrap_or(Ordering::Equal)
        })
        .then_with(|| left.original_index.cmp(&right.original_index))
        .then_with(|| left.candidate.node_id.cmp(&right.candidate.node_id))
}

fn max_redundancy(candidate: &Candidate, admitted: &[Candidate]) -> f32 {
    admitted
        .iter()
        .map(|existing| redundancy(candidate, existing))
        .fold(0.0_f32, f32::max)
}

fn redundancy(left: &Candidate, right: &Candidate) -> f32 {
    if let (Some(left_key), Some(right_key)) = (&left.redundancy_key, &right.redundancy_key) {
        if left_key == right_key {
            return 1.0;
        }
    }

    let left_terms = text_terms(&left.text);
    let right_terms = text_terms(&right.text);
    if left_terms.is_empty() || right_terms.is_empty() {
        return 0.0;
    }

    let intersection = left_terms.intersection(&right_terms).count() as f32;
    let union = left_terms.union(&right_terms).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn text_terms(text: &str) -> BTreeSet<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() > 2)
        .map(str::to_string)
        .collect()
}

fn text_digest(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ScoreContext, Scorer};

    #[derive(Clone, Copy)]
    struct PprScorer;

    impl Scorer for PprScorer {
        fn score(&self, c: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
            c.ppr_proximity
        }
    }

    fn candidate(id: &str, score: f32, tokens: usize) -> Candidate {
        let mut candidate = Candidate::new(id, format!("context {id}"), tokens);
        candidate.ppr_proximity = score;
        candidate
    }

    #[test]
    fn fill_admits_descending_score_within_budget_and_defers_handles() {
        let candidates = vec![
            candidate("low", 0.1, 8),
            candidate("high", 0.9, 5),
            candidate("mid", 0.5, 7),
        ];
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).without_redundancy();
        let admission = fill_to_budget(candidates, &PprScorer, &ctx, 12);

        assert_eq!(
            admission
                .admitted
                .iter()
                .map(|candidate| candidate.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["high", "mid"]
        );
        assert_eq!(admission.tokens_admitted, 12);
        assert_eq!(admission.tokens_deferred, 8);
        assert_eq!(admission.deferred[0].node_id, "low");
        assert_eq!(admission.deferred[0].token_count, 8);
        assert_eq!(admission.deferred[0].digest.len(), 64);
    }

    #[test]
    fn fill_skips_oversized_candidate_but_keeps_it_recoverable() {
        let candidates = vec![candidate("huge", 1.0, 30), candidate("fit", 0.2, 6)];
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).without_redundancy();
        let admission = fill_to_budget(candidates, &PprScorer, &ctx, 10);

        assert_eq!(admission.admitted[0].node_id, "fit");
        assert_eq!(admission.deferred[0].node_id, "huge");
        assert_eq!(admission.tokens_admitted, 6);
        assert_eq!(admission.tokens_deferred, 30);
    }

    #[test]
    fn fill_applies_redundancy_pressure_during_greedy_selection() {
        let mut duplicate = candidate("dup", 0.89, 5).with_redundancy_key("same");
        duplicate.text = "same same same alpha".to_string();
        let diverse = candidate("diverse", 0.82, 5).with_redundancy_key("other");
        let first = candidate("first", 0.9, 5).with_redundancy_key("same");
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).with_redundancy_penalty(0.2);

        let admission = fill_to_budget(vec![first, duplicate, diverse], &PprScorer, &ctx, 10);

        assert_eq!(
            admission
                .admitted
                .iter()
                .map(|candidate| candidate.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "diverse"]
        );
        assert_eq!(admission.deferred[0].node_id, "dup");
    }

    #[test]
    fn mmr_lambda_one_reproduces_score_ordered_greedy_fill() {
        let first = candidate("first", 0.9, 5).with_redundancy_key("same");
        let duplicate = candidate("dup", 0.89, 5).with_redundancy_key("same");
        let diverse = candidate("diverse", 0.82, 5).with_redundancy_key("other");
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).with_mmr_lambda(1.0);

        let admission = fill_to_budget(vec![first, duplicate, diverse], &PprScorer, &ctx, 10);

        assert_eq!(
            admission
                .admitted
                .iter()
                .map(|candidate| candidate.node_id.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "dup"]
        );
    }
}
