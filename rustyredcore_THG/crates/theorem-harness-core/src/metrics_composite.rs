use crate::session_metrics::SessionMetricsState;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const COMPOSITE_VERSION: &str = "godel_substrate_v1";

const TASK_COMPLETION_WEIGHT: f64 = 0.50;
const TOKEN_EFFICIENCY_WEIGHT: f64 = 0.30;
const TOOL_CALL_EFFICIENCY_WEIGHT: f64 = 0.20;
const TOKEN_EFFICIENCY_SCALE: f64 = 10_000.0;
const TOOL_CALL_EFFICIENCY_SCALE: f64 = 10.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HarnessComposite {
    pub composite_version: String,
    pub sample_size: usize,
    pub productivity_score: f64,
    pub axes: CompositeAxes,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<FitnessTraitScores>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompositeAxes {
    pub task_completion_rate: f64,
    pub token_efficiency: f64,
    pub tool_call_efficiency: f64,
}

impl CompositeAxes {
    pub fn weighted_productivity_score(&self) -> f64 {
        round6(
            self.task_completion_rate * TASK_COMPLETION_WEIGHT
                + self.token_efficiency * TOKEN_EFFICIENCY_WEIGHT
                + self.tool_call_efficiency * TOOL_CALL_EFFICIENCY_WEIGHT,
        )
    }

    pub fn as_axis_map(&self) -> BTreeMap<String, f64> {
        BTreeMap::from([
            (
                "task_completion_rate".to_string(),
                self.task_completion_rate,
            ),
            ("token_efficiency".to_string(), self.token_efficiency),
            (
                "tool_call_efficiency".to_string(),
                self.tool_call_efficiency,
            ),
        ])
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FitnessTraitScores {
    pub root_depth: f64,
    pub source_independence: f64,
    pub support_ratio: f64,
    pub claim_specificity: f64,
    pub temporal_spread: f64,
}

impl FitnessTraitScores {
    pub fn min_score(&self) -> f64 {
        self.root_depth
            .min(self.source_independence)
            .min(self.support_ratio)
            .min(self.claim_specificity)
            .min(self.temporal_spread)
    }

    pub fn trait_map(&self) -> BTreeMap<&'static str, f64> {
        BTreeMap::from([
            ("root_depth", self.root_depth),
            ("source_independence", self.source_independence),
            ("support_ratio", self.support_ratio),
            ("claim_specificity", self.claim_specificity),
            ("temporal_spread", self.temporal_spread),
        ])
    }
}

pub fn composite(metrics: &[SessionMetricsState]) -> HarnessComposite {
    composite_with_safety(metrics, None)
}

pub fn composite_with_safety(
    metrics: &[SessionMetricsState],
    safety: Option<FitnessTraitScores>,
) -> HarnessComposite {
    let axes = aggregate_axes(metrics);
    HarnessComposite {
        composite_version: COMPOSITE_VERSION.to_string(),
        sample_size: metrics.len(),
        productivity_score: axes.weighted_productivity_score(),
        axes,
        safety,
    }
}

pub fn session_composite_point(metric: &SessionMetricsState) -> (f64, CompositeAxes) {
    let axes = CompositeAxes {
        task_completion_rate: if metric.task_completion { 1.0 } else { 0.0 },
        token_efficiency: efficiency_from_cost(metric.total_tokens as f64, TOKEN_EFFICIENCY_SCALE),
        tool_call_efficiency: efficiency_from_cost(
            metric.total_tool_calls as f64,
            TOOL_CALL_EFFICIENCY_SCALE,
        ),
    };
    (axes.weighted_productivity_score(), axes)
}

fn aggregate_axes(metrics: &[SessionMetricsState]) -> CompositeAxes {
    if metrics.is_empty() {
        return CompositeAxes {
            task_completion_rate: 0.0,
            token_efficiency: 0.0,
            tool_call_efficiency: 0.0,
        };
    }

    let task_completion_rate = metrics
        .iter()
        .filter(|metric| metric.task_completion)
        .count() as f64
        / metrics.len() as f64;
    let mean_tokens = mean(
        metrics
            .iter()
            .map(|metric| metric.total_tokens as f64)
            .collect::<Vec<_>>()
            .as_slice(),
    );
    let mean_tool_calls = mean(
        metrics
            .iter()
            .map(|metric| metric.total_tool_calls as f64)
            .collect::<Vec<_>>()
            .as_slice(),
    );

    CompositeAxes {
        task_completion_rate: round6(task_completion_rate),
        token_efficiency: efficiency_from_cost(mean_tokens, TOKEN_EFFICIENCY_SCALE),
        tool_call_efficiency: efficiency_from_cost(mean_tool_calls, TOOL_CALL_EFFICIENCY_SCALE),
    }
}

fn efficiency_from_cost(cost: f64, scale: f64) -> f64 {
    if cost <= 0.0 {
        return 1.0;
    }
    round6(scale / (scale + cost))
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(session_id: &str, complete: bool, tokens: i64, calls: i64) -> SessionMetricsState {
        SessionMetricsState {
            total_input_tokens: tokens / 2,
            total_output_tokens: tokens - (tokens / 2),
            total_tool_calls: calls,
            task_completion: complete,
            pairformer_mode: "off".to_string(),
            task_category: "rust".to_string(),
            workstream_id: "godel".to_string(),
            session_id: session_id.to_string(),
            total_tokens: tokens,
        }
    }

    #[test]
    fn composite_is_deterministic_and_versioned() {
        let metrics = vec![metric("a", true, 4_000, 4), metric("b", false, 12_000, 12)];
        let first = composite(&metrics);
        let second = composite(&metrics);

        assert_eq!(first, second);
        assert_eq!(first.composite_version, COMPOSITE_VERSION);
        assert_eq!(first.sample_size, 2);
        assert_eq!(first.axes.task_completion_rate, 0.5);
    }

    #[test]
    fn safety_score_is_reported_separately_from_productivity() {
        let metrics = vec![metric("a", true, 2_000, 1)];
        let safety = FitnessTraitScores {
            root_depth: 0.9,
            source_independence: 0.2,
            support_ratio: 0.8,
            claim_specificity: 0.7,
            temporal_spread: 0.6,
        };

        let without_safety = composite(&metrics);
        let with_safety = composite_with_safety(&metrics, Some(safety.clone()));

        assert_eq!(
            without_safety.productivity_score,
            with_safety.productivity_score
        );
        assert_eq!(with_safety.safety, Some(safety));
    }
}
