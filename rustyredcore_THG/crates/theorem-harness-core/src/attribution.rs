use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const DELTA_EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigOutcome {
    Positive,
    Negative,
    Neutral,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigAttributionCounter {
    pub positive: u64,
    pub negative: u64,
    pub neutral: u64,
    pub total_delta: f64,
}

impl ConfigAttributionCounter {
    pub fn record(&mut self, outcome: ConfigOutcome, composite_delta: f64) {
        match outcome {
            ConfigOutcome::Positive => self.positive += 1,
            ConfigOutcome::Negative => self.negative += 1,
            ConfigOutcome::Neutral => self.neutral += 1,
        }
        self.total_delta += composite_delta;
    }

    pub fn total(&self) -> u64 {
        self.positive + self.negative + self.neutral
    }

    pub fn credit(&self) -> f64 {
        let denominator = self.total() as f64 + 3.0;
        round6(
            (self.positive as f64 + 1.0) / denominator - (self.negative as f64 + 1.0) / denominator,
        )
    }

    pub fn has_positive_credit(&self) -> bool {
        self.credit() > 0.0 && self.total_delta > 0.0
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ConfigAttributionTable {
    pub counters: BTreeMap<String, ConfigAttributionCounter>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ConfigRunAttribution {
    pub config_keys: Vec<String>,
    pub composite_delta: f64,
}

impl ConfigAttributionTable {
    pub fn record(&mut self, config_key: &str, outcome: ConfigOutcome, composite_delta: f64) {
        self.counters
            .entry(config_key.to_string())
            .or_default()
            .record(outcome, composite_delta);
    }

    pub fn credit(&self, config_key: &str) -> f64 {
        self.counters
            .get(config_key)
            .map(ConfigAttributionCounter::credit)
            .unwrap_or(0.0)
    }

    pub fn has_positive_credit(&self, config_key: &str) -> bool {
        self.counters
            .get(config_key)
            .is_some_and(ConfigAttributionCounter::has_positive_credit)
    }

    pub fn credit_runs(runs: &[ConfigRunAttribution]) -> Self {
        let mut table = Self::default();
        for run in runs {
            let outcome = outcome_for_delta(run.composite_delta);
            for key in &run.config_keys {
                table.record(key, outcome, run.composite_delta);
            }
        }
        table
    }
}

fn outcome_for_delta(delta: f64) -> ConfigOutcome {
    if delta > DELTA_EPSILON {
        ConfigOutcome::Positive
    } else if delta < -DELTA_EPSILON {
        ConfigOutcome::Negative
    } else {
        ConfigOutcome::Neutral
    }
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_present_only_in_positive_runs_gets_positive_credit() {
        let table = ConfigAttributionTable::credit_runs(&[
            ConfigRunAttribution {
                config_keys: vec!["routing.weight.codex".to_string()],
                composite_delta: 0.10,
            },
            ConfigRunAttribution {
                config_keys: vec!["routing.weight.codex".to_string()],
                composite_delta: 0.20,
            },
        ]);

        assert!(table.credit("routing.weight.codex") > 0.0);
        assert!(table.has_positive_credit("routing.weight.codex"));
    }

    #[test]
    fn equally_positive_and_negative_key_is_neutral() {
        let table = ConfigAttributionTable::credit_runs(&[
            ConfigRunAttribution {
                config_keys: vec!["threshold.context".to_string()],
                composite_delta: 0.10,
            },
            ConfigRunAttribution {
                config_keys: vec!["threshold.context".to_string()],
                composite_delta: -0.10,
            },
        ]);

        assert_eq!(table.credit("threshold.context"), 0.0);
        assert!(!table.has_positive_credit("threshold.context"));
    }
}
