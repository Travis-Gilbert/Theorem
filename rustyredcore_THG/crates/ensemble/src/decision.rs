//! The replayable Ensemble decision artifact (the former OrchestrateDecision).
//!
//! Emitted by the budgeted selector (slice S2). Captured here so the registry and the
//! selector share one contract. Content-addressable for replay, audit, and as a training
//! signal.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use theorem_harness_core::stable_value_hash;

/// A capability the selector chose to bring in for a task.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SelectedCapability {
    pub kind: String,
    pub pack_content_hash: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub cost_units: u64,
}

/// A candidate the selector considered and rejected, with the reason (for audit + training).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RejectedCandidate {
    pub kind: String,
    pub pack_content_hash: String,
    pub reason: String,
}

/// The replayable decision the selector emits per task. Deterministic from
/// (task, budget, candidate registry state, priors).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct EnsembleDecision {
    pub task: String,
    #[serde(default)]
    pub budget_units: Option<u64>,
    #[serde(default)]
    pub spent_units: u64,
    #[serde(default)]
    pub selected: Vec<SelectedCapability>,
    #[serde(default)]
    pub rejected: Vec<RejectedCandidate>,
    #[serde(default)]
    pub risk: String,
    /// The priors the selector used (e.g. fitness / trust weights), echoed for replay.
    #[serde(default)]
    pub priors: Value,
}

impl EnsembleDecision {
    /// Content address of the decision, for replay/audit dedup and as a training key.
    pub fn content_address(&self) -> String {
        stable_value_hash(&serde_json::to_value(self).unwrap_or(Value::Null))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decision_content_address_is_stable() {
        let mk = || EnsembleDecision {
            task: "refactor crate".to_string(),
            budget_units: Some(1000),
            spent_units: 120,
            selected: vec![SelectedCapability {
                kind: "skill".to_string(),
                pack_content_hash: "abc".to_string(),
                reason: "best fit".to_string(),
                score: 0.9,
                cost_units: 120,
            }],
            rejected: vec![RejectedCandidate {
                kind: "tool".to_string(),
                pack_content_hash: "def".to_string(),
                reason: "over budget".to_string(),
            }],
            risk: "low".to_string(),
            priors: json!({ "trust_weight": 0.5 }),
        };
        assert_eq!(mk().content_address(), mk().content_address());
        assert!(!mk().content_address().is_empty());
    }
}
