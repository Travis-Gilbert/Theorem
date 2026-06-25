use crate::metrics_composite::FitnessTraitScores;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const TRAIT_EPSILON: f64 = 1e-9;
const ROOT_DEPTH_SCALE: f64 = 6.0;
const TEMPORAL_SPREAD_SCALE_MS: i64 = 30 * 24 * 60 * 60 * 1_000;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitnessObservation {
    pub root_depth: u32,
    pub source_id: String,
    pub support_ratio: f64,
    pub claim_specificity: f64,
    pub observed_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitnessTraitViolation {
    pub trait_name: String,
    pub before: f64,
    pub after: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitnessGateResult {
    pub passed: bool,
    pub before: FitnessTraitScores,
    pub after: FitnessTraitScores,
    pub violations: Vec<FitnessTraitViolation>,
}

pub fn measure_fitness_traits(observations: &[FitnessObservation]) -> FitnessTraitScores {
    if observations.is_empty() {
        return FitnessTraitScores {
            root_depth: 1.0,
            source_independence: 1.0,
            support_ratio: 1.0,
            claim_specificity: 1.0,
            temporal_spread: 1.0,
        };
    }

    let unique_sources = observations
        .iter()
        .map(|observation| observation.source_id.clone())
        .collect::<BTreeSet<_>>()
        .len();
    let min_time = observations
        .iter()
        .map(|observation| observation.observed_at_ms)
        .min()
        .unwrap_or_default();
    let max_time = observations
        .iter()
        .map(|observation| observation.observed_at_ms)
        .max()
        .unwrap_or_default();

    FitnessTraitScores {
        root_depth: round6(mean(
            observations
                .iter()
                .map(|observation| clamp01(observation.root_depth as f64 / ROOT_DEPTH_SCALE))
                .collect::<Vec<_>>()
                .as_slice(),
        )),
        source_independence: round6(unique_sources as f64 / observations.len() as f64),
        support_ratio: round6(mean(
            observations
                .iter()
                .map(|observation| clamp01(observation.support_ratio))
                .collect::<Vec<_>>()
                .as_slice(),
        )),
        claim_specificity: round6(mean(
            observations
                .iter()
                .map(|observation| clamp01(observation.claim_specificity))
                .collect::<Vec<_>>()
                .as_slice(),
        )),
        temporal_spread: round6(clamp01(
            (max_time - min_time).max(0) as f64 / TEMPORAL_SPREAD_SCALE_MS as f64,
        )),
    }
}

pub fn check_epistemic_fitness(
    before: FitnessTraitScores,
    after: FitnessTraitScores,
) -> FitnessGateResult {
    let violations = before
        .trait_map()
        .into_iter()
        .filter_map(|(trait_name, before_value)| {
            let after_value = after.trait_map().get(trait_name).copied().unwrap_or(0.0);
            if after_value + TRAIT_EPSILON < before_value {
                Some(FitnessTraitViolation {
                    trait_name: trait_name.to_string(),
                    before: before_value,
                    after: after_value,
                })
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    FitnessGateResult {
        passed: violations.is_empty(),
        before,
        after,
        violations,
    }
}

fn mean(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_collapsing_delta_is_rejected() {
        let before = measure_fitness_traits(&[
            obs("source:a", 0),
            obs("source:b", 1_000),
            obs("source:c", 2_000),
        ]);
        let after = measure_fitness_traits(&[
            obs("source:a", 0),
            obs("source:a", 1_000),
            obs("source:a", 2_000),
        ]);

        let result = check_epistemic_fitness(before, after);

        assert!(!result.passed);
        assert!(result
            .violations
            .iter()
            .any(|violation| violation.trait_name == "source_independence"));
    }

    fn obs(source_id: &str, observed_at_ms: i64) -> FitnessObservation {
        FitnessObservation {
            root_depth: 4,
            source_id: source_id.to_string(),
            support_ratio: 0.8,
            claim_specificity: 0.8,
            observed_at_ms,
        }
    }
}
