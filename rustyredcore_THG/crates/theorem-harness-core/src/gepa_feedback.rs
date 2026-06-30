use crate::epistemic_fitness::{
    check_epistemic_fitness, measure_fitness_traits, FitnessGateResult, FitnessObservation,
};
use crate::metrics_composite::{session_composite_point, CompositeAxes};
use crate::session_metrics::SessionMetricsState;
use crate::shadow_eval::{ShadowEvalResult, ShadowVerdict};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReservoirFeedback {
    #[serde(default)]
    pub before_observation_count: usize,
    #[serde(default)]
    pub before_unique_source_count: usize,
    #[serde(default)]
    pub after_observation_count: usize,
    #[serde(default)]
    pub after_unique_source_count: usize,
    #[serde(default)]
    pub unsupported_claims: Vec<String>,
    #[serde(default)]
    pub contradictions: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GepaFeedbackInput {
    pub metric: SessionMetricsState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fitness: Option<FitnessGateResult>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<ShadowEvalResult>,
    #[serde(default)]
    pub reservoir: ReservoirFeedback,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeedbackPoint {
    pub score: f64,
    pub axes: CompositeAxes,
    pub feedback: String,
    #[serde(default)]
    pub signals: Vec<String>,
}

pub fn reservoir_feedback_from_observations(
    before: &[FitnessObservation],
    after: &[FitnessObservation],
) -> ReservoirFeedback {
    ReservoirFeedback {
        before_observation_count: before.len(),
        before_unique_source_count: unique_source_count(before),
        after_observation_count: after.len(),
        after_unique_source_count: unique_source_count(after),
        unsupported_claims: Vec::new(),
        contradictions: Vec::new(),
        notes: Vec::new(),
    }
}

pub fn fitness_gate_from_observations(
    before: &[FitnessObservation],
    after: &[FitnessObservation],
) -> FitnessGateResult {
    check_epistemic_fitness(
        measure_fitness_traits(before),
        measure_fitness_traits(after),
    )
}

pub fn gepa_feedback_point(input: GepaFeedbackInput) -> FeedbackPoint {
    let (score, axes) = session_composite_point(&input.metric);
    let mut feedback = Vec::new();
    let mut signals = Vec::new();

    if let Some(fitness) = input.fitness.as_ref() {
        for violation in &fitness.violations {
            let drop = round6(violation.before - violation.after);
            feedback.push(format!(
                "Epistemic fitness trait `{}` dropped from {:.6} to {:.6} (drop {:.6}).",
                violation.trait_name, violation.before, violation.after, drop
            ));
            signals.push(format!("fitness:{}", violation.trait_name));
        }
    }

    if input.reservoir.before_unique_source_count > input.reservoir.after_unique_source_count {
        feedback.push(format!(
            "Reservoir source count dropped from {} unique sources to {}.",
            input.reservoir.before_unique_source_count, input.reservoir.after_unique_source_count
        ));
        signals.push("reservoir:source_count_drop".to_string());
    }

    let mut unsupported_claims = cleaned_sorted(input.reservoir.unsupported_claims);
    for claim in unsupported_claims.drain(..) {
        feedback.push(format!("Unsupported claim surfaced by reservoir: {claim}."));
        signals.push("reservoir:unsupported_claim".to_string());
    }

    let mut contradictions = cleaned_sorted(input.reservoir.contradictions);
    for contradiction in contradictions.drain(..) {
        feedback.push(format!(
            "Contradiction surfaced by reservoir: {contradiction}."
        ));
        signals.push("reservoir:contradiction".to_string());
    }

    let mut notes = cleaned_sorted(input.reservoir.notes);
    for note in notes.drain(..) {
        feedback.push(format!("Reservoir note: {note}."));
        signals.push("reservoir:note".to_string());
    }

    if let Some(shadow) = input.shadow.as_ref() {
        if shadow.verdict == ShadowVerdict::Unsafe {
            feedback.push(format!(
                "Shadow evaluation marked delta `{}` unsafe.",
                shadow.delta_id
            ));
            signals.push("shadow:unsafe".to_string());
        }
        let mut safety_violations = cleaned_sorted(shadow.safety_violations.clone());
        for violation in safety_violations.drain(..) {
            feedback.push(format!("Shadow safety violation: {violation}."));
            signals.push("shadow:safety_violation".to_string());
        }
        for (index, diff) in shadow.run_diffs.iter().enumerate() {
            feedback.push(format!(
                "Run diff {}: {}.",
                index + 1,
                summarize_run_diff(diff)
            ));
            signals.push("shadow:run_diff".to_string());
        }
    }

    if feedback.is_empty() {
        feedback.push(format!(
            "No actionable reservoir-grounded feedback surfaced; productivity score is {:.6}.",
            score
        ));
        signals.push("feedback:none".to_string());
    }

    signals.sort();
    signals.dedup();

    FeedbackPoint {
        score,
        axes,
        feedback: feedback.join(" "),
        signals,
    }
}

fn unique_source_count(observations: &[FitnessObservation]) -> usize {
    observations
        .iter()
        .map(|observation| observation.source_id.as_str())
        .collect::<BTreeSet<_>>()
        .len()
}

fn cleaned_sorted(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn summarize_run_diff(diff: &Value) -> String {
    let summary = diff.get("summary").and_then(Value::as_object);
    if let Some(summary) = summary {
        let added = summary.get("added").and_then(Value::as_u64).unwrap_or(0);
        let removed = summary.get("removed").and_then(Value::as_u64).unwrap_or(0);
        let changed = summary.get("changed").and_then(Value::as_u64).unwrap_or(0);
        return format!("added {added} steps, removed {removed} steps, changed {changed} steps");
    }
    "trace changed without a structured summary".to_string()
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_collapsing_session_names_trait_and_source_count_drop() {
        let before = vec![
            obs("source:a", 0),
            obs("source:b", 1_000),
            obs("source:c", 2_000),
        ];
        let after = vec![
            obs("source:a", 0),
            obs("source:a", 1_000),
            obs("source:a", 2_000),
        ];

        let point = gepa_feedback_point(GepaFeedbackInput {
            metric: metric("session:source-collapse"),
            fitness: Some(fitness_gate_from_observations(&before, &after)),
            shadow: None,
            reservoir: reservoir_feedback_from_observations(&before, &after),
        });

        assert!(point.feedback.contains("source_independence"));
        assert!(point
            .feedback
            .contains("source count dropped from 3 unique sources to 1"));
        assert_eq!(point.signals[0], "fitness:source_independence");
        assert!(point.score > 0.0);
    }

    #[test]
    fn unsupported_claims_and_contradictions_are_natural_language_feedback() {
        let point = gepa_feedback_point(GepaFeedbackInput {
            metric: metric("session:unsupported"),
            fitness: None,
            shadow: None,
            reservoir: ReservoirFeedback {
                unsupported_claims: vec!["claim lacks citation".to_string()],
                contradictions: vec!["answer conflicts with graph fact".to_string()],
                ..ReservoirFeedback::default()
            },
        });

        assert!(point.feedback.contains("Unsupported claim surfaced"));
        assert!(point.feedback.contains("Contradiction surfaced"));
    }

    fn metric(session_id: &str) -> SessionMetricsState {
        SessionMetricsState {
            total_input_tokens: 1_000,
            total_output_tokens: 500,
            total_tool_calls: 2,
            task_completion: true,
            pairformer_mode: "candidate".to_string(),
            task_category: "godel".to_string(),
            workstream_id: "gepa".to_string(),
            session_id: session_id.to_string(),
            total_tokens: 1_500,
        }
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
