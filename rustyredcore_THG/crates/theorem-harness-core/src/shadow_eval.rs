use crate::metrics_composite::{composite, HarnessComposite};
use crate::replay::compare_runs;
use crate::session_metrics::{compare_modes, SessionMetricsState};
use crate::types::AgentRunState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const COMPOSITE_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShadowVerdict {
    Improve,
    Regress,
    Neutral,
    InsufficientChange,
    Unsafe,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowRunPair {
    pub before: AgentRunState,
    pub after: AgentRunState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowEvalInput {
    pub delta_id: String,
    pub baseline_metrics: Vec<SessionMetricsState>,
    pub candidate_metrics: Vec<SessionMetricsState>,
    #[serde(default)]
    pub run_pairs: Vec<ShadowRunPair>,
    #[serde(default)]
    pub safety_violations: Vec<String>,
    #[serde(default = "default_baseline_mode")]
    pub baseline_mode: String,
    #[serde(default = "default_candidate_mode")]
    pub candidate_mode: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ShadowEvalResult {
    pub delta_id: String,
    pub baseline: HarnessComposite,
    pub candidate: HarnessComposite,
    pub composite_delta: f64,
    pub significance: Value,
    pub confidence_90_bar_met: bool,
    pub verdict: ShadowVerdict,
    #[serde(default)]
    pub run_diffs: Vec<Value>,
    #[serde(default)]
    pub safety_violations: Vec<String>,
}

pub fn evaluate_shadow(input: ShadowEvalInput) -> ShadowEvalResult {
    let baseline = composite(&input.baseline_metrics);
    let candidate = composite(&input.candidate_metrics);
    let composite_delta = round6(candidate.productivity_score - baseline.productivity_score);
    let significance = significance_for_modes(
        &input.baseline_metrics,
        &input.candidate_metrics,
        &input.baseline_mode,
        &input.candidate_mode,
    );
    let confidence_90_bar_met = significance
        .get("confidence_90_bar_met")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let run_diffs = input
        .run_pairs
        .iter()
        .map(|pair| compare_runs(&pair.before, &pair.after))
        .collect::<Vec<_>>();
    let verdict = verdict_for(
        composite_delta,
        confidence_90_bar_met,
        input.safety_violations.is_empty(),
    );

    ShadowEvalResult {
        delta_id: input.delta_id,
        baseline,
        candidate,
        composite_delta,
        significance,
        confidence_90_bar_met,
        verdict,
        run_diffs,
        safety_violations: input.safety_violations,
    }
}

fn significance_for_modes(
    baseline_metrics: &[SessionMetricsState],
    candidate_metrics: &[SessionMetricsState],
    baseline_mode: &str,
    candidate_mode: &str,
) -> Value {
    let mut metrics = baseline_metrics
        .iter()
        .cloned()
        .map(|mut metric| {
            metric.pairformer_mode = baseline_mode.to_string();
            metric
        })
        .collect::<Vec<_>>();
    metrics.extend(candidate_metrics.iter().cloned().map(|mut metric| {
        metric.pairformer_mode = candidate_mode.to_string();
        metric
    }));
    compare_modes(&metrics, Some(baseline_mode), Some(candidate_mode))
}

fn verdict_for(delta: f64, confidence_90_bar_met: bool, safety_passed: bool) -> ShadowVerdict {
    if !safety_passed {
        return ShadowVerdict::Unsafe;
    }
    if delta < -COMPOSITE_EPSILON {
        return ShadowVerdict::Regress;
    }
    if delta.abs() <= COMPOSITE_EPSILON {
        return ShadowVerdict::InsufficientChange;
    }
    if confidence_90_bar_met {
        ShadowVerdict::Improve
    } else {
        ShadowVerdict::InsufficientChange
    }
}

fn default_baseline_mode() -> String {
    "baseline".to_string()
}

fn default_candidate_mode() -> String {
    "candidate".to_string()
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metric(session_id: usize, mode: &str, complete: bool, tokens: i64) -> SessionMetricsState {
        SessionMetricsState {
            total_input_tokens: tokens / 2,
            total_output_tokens: tokens - (tokens / 2),
            total_tool_calls: 2,
            task_completion: complete,
            pairformer_mode: mode.to_string(),
            task_category: "rust".to_string(),
            workstream_id: "godel".to_string(),
            session_id: format!("{mode}:{session_id}"),
            total_tokens: tokens,
        }
    }

    #[test]
    fn no_op_delta_is_insufficient_change() {
        let metrics = (0..50)
            .map(|index| metric(index, "baseline", true, 1_000))
            .collect::<Vec<_>>();
        let result = evaluate_shadow(ShadowEvalInput {
            delta_id: "delta:no-op".to_string(),
            baseline_metrics: metrics.clone(),
            candidate_metrics: metrics,
            run_pairs: Vec::new(),
            safety_violations: Vec::new(),
            baseline_mode: "baseline".to_string(),
            candidate_mode: "candidate".to_string(),
        });

        assert_eq!(result.verdict, ShadowVerdict::InsufficientChange);
    }

    #[test]
    fn known_good_delta_reports_improve_with_significance() {
        let baseline = (0..60)
            .map(|index| metric(index, "baseline", true, 10_000 + index as i64))
            .collect::<Vec<_>>();
        let candidate = (0..60)
            .map(|index| metric(index, "candidate", true, 4_000 + index as i64))
            .collect::<Vec<_>>();

        let result = evaluate_shadow(ShadowEvalInput {
            delta_id: "delta:good".to_string(),
            baseline_metrics: baseline,
            candidate_metrics: candidate,
            run_pairs: Vec::new(),
            safety_violations: Vec::new(),
            baseline_mode: "baseline".to_string(),
            candidate_mode: "candidate".to_string(),
        });

        assert_eq!(result.verdict, ShadowVerdict::Improve);
        assert!(result.confidence_90_bar_met);
        assert!(result.composite_delta > 0.0);
    }
}
