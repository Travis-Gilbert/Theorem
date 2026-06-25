use crate::attribution::ConfigAttributionTable;
use crate::config_ledger::{ConfigDelta, ConfigLedger, ConfigLedgerError, ConfigState};
use crate::epistemic_fitness::FitnessGateResult;
use crate::improvement_rate::ImprovementRate;
use crate::shadow_eval::{ShadowEvalResult, ShadowVerdict};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopGateRejection {
    AttributionNotPositive,
    ShadowEvalFailed,
    Oscillating,
    FitnessDegraded,
    InFlightDeltaExists,
    BudgetThrottled,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoopGateVerdict {
    pub accepted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection: Option<LoopGateRejection>,
    pub attribution_credit: f64,
    pub composite_delta: f64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopClosureBudget {
    pub initial_capacity: u32,
    pub capacity: u32,
    pub available: u32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopGateState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub in_flight_delta_id: Option<String>,
    pub budget: LoopClosureBudget,
}

pub struct LoopClosureInput<'a> {
    pub delta: ConfigDelta,
    pub attribution_key: &'a str,
    pub attribution: &'a ConfigAttributionTable,
    pub shadow: &'a ShadowEvalResult,
    pub rate: &'a ImprovementRate,
    pub fitness: &'a FitnessGateResult,
}

impl LoopClosureBudget {
    pub fn new(capacity: u32) -> Self {
        let capacity = capacity.max(1);
        Self {
            initial_capacity: capacity,
            capacity,
            available: capacity,
        }
    }

    pub fn can_consume(&self) -> bool {
        self.available > 0
    }

    pub fn consume(&mut self) -> bool {
        if self.available == 0 {
            return false;
        }
        self.available -= 1;
        true
    }

    pub fn record_outcome(&mut self, composite_gain: f64) {
        if composite_gain <= 0.0 {
            self.capacity = self.capacity.saturating_sub(1).max(1);
            self.available = self.available.min(self.capacity);
            return;
        }
        self.capacity = (self.capacity + 1).min(self.initial_capacity);
        self.available = (self.available + 1).min(self.capacity);
    }
}

impl Default for LoopGateState {
    fn default() -> Self {
        Self {
            in_flight_delta_id: None,
            budget: LoopClosureBudget::new(1),
        }
    }
}

pub fn evaluate_loop_gate(
    state: &LoopGateState,
    delta_id: &str,
    attribution_key: &str,
    attribution: &ConfigAttributionTable,
    shadow: &ShadowEvalResult,
    rate: &ImprovementRate,
    fitness: &FitnessGateResult,
) -> LoopGateVerdict {
    let attribution_credit = attribution.credit(attribution_key);
    let rejection = if state
        .in_flight_delta_id
        .as_deref()
        .is_some_and(|in_flight| in_flight != delta_id)
    {
        Some(LoopGateRejection::InFlightDeltaExists)
    } else if !state.budget.can_consume() {
        Some(LoopGateRejection::BudgetThrottled)
    } else if !attribution.has_positive_credit(attribution_key) {
        Some(LoopGateRejection::AttributionNotPositive)
    } else if shadow.verdict != ShadowVerdict::Improve || !shadow.confidence_90_bar_met {
        Some(LoopGateRejection::ShadowEvalFailed)
    } else if rate.oscillating {
        Some(LoopGateRejection::Oscillating)
    } else if !fitness.passed {
        Some(LoopGateRejection::FitnessDegraded)
    } else {
        None
    };

    LoopGateVerdict {
        accepted: rejection.is_none(),
        rejection,
        attribution_credit,
        composite_delta: shadow.composite_delta,
    }
}

pub fn close_loop_if_allowed(
    state: &mut LoopGateState,
    ledger: &mut ConfigLedger,
    config: &mut ConfigState,
    input: LoopClosureInput<'_>,
) -> Result<LoopGateVerdict, ConfigLedgerError> {
    let verdict = evaluate_loop_gate(
        state,
        &input.delta.delta_id,
        input.attribution_key,
        input.attribution,
        input.shadow,
        input.rate,
        input.fitness,
    );
    if !verdict.accepted {
        return Ok(verdict);
    }

    state.in_flight_delta_id = Some(input.delta.delta_id.clone());
    state.budget.consume();
    ledger.apply_delta(config, input.delta)?;
    state.budget.record_outcome(input.shadow.composite_delta);
    state.in_flight_delta_id = None;
    Ok(verdict)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attribution::{ConfigAttributionTable, ConfigRunAttribution};
    use crate::config_ledger::{ConfigDelta, ConfigValueDelta};
    use crate::epistemic_fitness::check_epistemic_fitness;
    use crate::improvement_rate::{composite_point, compute_improvement_rate};
    use crate::metrics_composite::FitnessTraitScores;
    use crate::shadow_eval::ShadowEvalResult;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn gate_rejects_each_safety_path_and_accepts_clean_delta() {
        let attribution = positive_attribution();
        let passing_shadow = shadow(ShadowVerdict::Improve, true, 0.2);
        let stable_rate = rate(&[0.1, 0.2, 0.3, 0.4, 0.5]);
        let oscillating_rate = rate(&[0.1, 0.4, 0.2, 0.5, 0.3]);
        let fitness_pass = fitness(true);
        let fitness_fail = fitness(false);
        let state = LoopGateState {
            in_flight_delta_id: None,
            budget: LoopClosureBudget::new(1),
        };

        assert_eq!(
            evaluate_loop_gate(
                &state,
                "delta:1",
                "routing.weight.codex",
                &ConfigAttributionTable::default(),
                &passing_shadow,
                &stable_rate,
                &fitness_pass,
            )
            .rejection,
            Some(LoopGateRejection::AttributionNotPositive)
        );
        assert_eq!(
            evaluate_loop_gate(
                &state,
                "delta:1",
                "routing.weight.codex",
                &attribution,
                &shadow(ShadowVerdict::InsufficientChange, false, 0.2),
                &stable_rate,
                &fitness_pass,
            )
            .rejection,
            Some(LoopGateRejection::ShadowEvalFailed)
        );
        assert_eq!(
            evaluate_loop_gate(
                &state,
                "delta:1",
                "routing.weight.codex",
                &attribution,
                &passing_shadow,
                &oscillating_rate,
                &fitness_pass,
            )
            .rejection,
            Some(LoopGateRejection::Oscillating)
        );
        assert_eq!(
            evaluate_loop_gate(
                &state,
                "delta:1",
                "routing.weight.codex",
                &attribution,
                &passing_shadow,
                &stable_rate,
                &fitness_fail,
            )
            .rejection,
            Some(LoopGateRejection::FitnessDegraded)
        );

        let accepted = evaluate_loop_gate(
            &state,
            "delta:1",
            "routing.weight.codex",
            &attribution,
            &passing_shadow,
            &stable_rate,
            &fitness_pass,
        );
        assert!(accepted.accepted);
    }

    #[test]
    fn accepted_delta_is_logged_in_ledger() {
        let mut state = LoopGateState {
            in_flight_delta_id: None,
            budget: LoopClosureBudget::new(2),
        };
        let mut ledger = ConfigLedger::default();
        let mut config = ConfigState {
            values: BTreeMap::from([("routing.weight.codex".to_string(), json!(0.4))]),
            graph_version_id: Some("graph:v1".to_string()),
        };

        let verdict = close_loop_if_allowed(
            &mut state,
            &mut ledger,
            &mut config,
            LoopClosureInput {
                delta: ConfigDelta {
                    delta_id: "delta:1".to_string(),
                    description: "raise weight".to_string(),
                    values: vec![ConfigValueDelta {
                        key: "routing.weight.codex".to_string(),
                        before: json!(0.4),
                        after: json!(0.6),
                    }],
                    graph_version_before: Some("graph:v1".to_string()),
                    graph_version_after: Some("graph:v2".to_string()),
                },
                attribution_key: "routing.weight.codex",
                attribution: &positive_attribution(),
                shadow: &shadow(ShadowVerdict::Improve, true, 0.2),
                rate: &rate(&[0.1, 0.2, 0.3, 0.4, 0.5]),
                fitness: &fitness(true),
            },
        )
        .unwrap();

        assert!(verdict.accepted);
        assert_eq!(ledger.entries.len(), 1);
        assert_eq!(config.values.get("routing.weight.codex"), Some(&json!(0.6)));
    }

    #[test]
    fn budget_decays_after_no_gain_closures() {
        let mut budget = LoopClosureBudget::new(4);
        budget.record_outcome(0.0);
        budget.record_outcome(-0.1);
        budget.record_outcome(0.0);

        assert_eq!(budget.capacity, 1);
        assert!(budget.capacity < budget.initial_capacity);
    }

    fn positive_attribution() -> ConfigAttributionTable {
        ConfigAttributionTable::credit_runs(&[ConfigRunAttribution {
            config_keys: vec!["routing.weight.codex".to_string()],
            composite_delta: 0.2,
        }])
    }

    fn rate(values: &[f64]) -> ImprovementRate {
        let points = values
            .iter()
            .enumerate()
            .map(|(index, value)| composite_point(format!("p{index}"), *value, None))
            .collect::<Vec<_>>();
        compute_improvement_rate(&points, values.len()).unwrap()
    }

    fn fitness(pass: bool) -> crate::epistemic_fitness::FitnessGateResult {
        let before = FitnessTraitScores {
            root_depth: 0.5,
            source_independence: 0.5,
            support_ratio: 0.5,
            claim_specificity: 0.5,
            temporal_spread: 0.5,
        };
        let after = if pass {
            before.clone()
        } else {
            FitnessTraitScores {
                source_independence: 0.1,
                ..before.clone()
            }
        };
        check_epistemic_fitness(before, after)
    }

    fn shadow(verdict: ShadowVerdict, confidence: bool, delta: f64) -> ShadowEvalResult {
        ShadowEvalResult {
            delta_id: "delta:1".to_string(),
            baseline: empty_composite(),
            candidate: empty_composite(),
            composite_delta: delta,
            significance: json!({"confidence_90_bar_met": confidence}),
            confidence_90_bar_met: confidence,
            verdict,
            run_diffs: Vec::new(),
            safety_violations: Vec::new(),
        }
    }

    fn empty_composite() -> crate::metrics_composite::HarnessComposite {
        crate::metrics_composite::HarnessComposite {
            composite_version: crate::metrics_composite::COMPOSITE_VERSION.to_string(),
            sample_size: 0,
            productivity_score: 0.0,
            axes: crate::metrics_composite::CompositeAxes {
                task_completion_rate: 0.0,
                token_efficiency: 0.0,
                tool_call_efficiency: 0.0,
            },
            safety: None,
        }
    }
}
