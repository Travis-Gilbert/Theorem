//! The routing engine ([`OffloadEngine`]), the per-op outcome
//! ([`OffloadOutcome`]), and the route report ([`OffloadRouteReport`]).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::affordance::{OffloadAffordance, OffloadAffordanceError, OffloadAffordanceResult};
use crate::classifier::OffloadClassifier;
use crate::ledger::{LedgerEntry, OffloadLedger};
use crate::operation::{OffloadDecision, OffloadOperation, OffloadOperationKind};

/// What actually happened to one routed operation. The variants are first-class
/// and honest: an eligible op with no registered affordance is
/// [`EligibleUnrouted`](Self::EligibleUnrouted) (the saving is recorded as an
/// estimate, the result is absent), never a fabricated success.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum OffloadOutcome {
    /// The op was eligible, an affordance was registered for its kind, and the
    /// substrate computation ran. Carries the real result.
    Executed {
        /// The result the affordance produced.
        result: OffloadAffordanceResult,
    },
    /// The op was eligible but NO affordance is registered for its kind yet. The
    /// honest fallback: the GPU-second saving is recorded as the estimate, the
    /// gap is named, nothing is faked.
    EligibleUnrouted {
        /// The kind that has no wired affordance.
        kind: OffloadOperationKind,
    },
    /// The op was eligible and an affordance ran but FAILED to compute (genuine
    /// computation error). No saving is credited.
    ExecutionFailed {
        /// The affordance error.
        error: OffloadAffordanceError,
    },
    /// The op was not offload-eligible; it stays on the GPU. No saving.
    NotEligible,
}

/// The full result of one [`OffloadEngine::route`] call: the classifier verdict,
/// the outcome, and the GPU-seconds credited to the ledger for this op.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffloadRouteReport {
    /// The classifier verdict (with the engine's affordance annotation filled
    /// in).
    pub decision: OffloadDecision,
    /// What happened to the op.
    pub outcome: OffloadOutcome,
    /// GPU-seconds credited to the cumulative ledger total for this op.
    pub gpu_seconds_saved: f64,
}

impl OffloadRouteReport {
    /// Whether the op actually executed on a wired substrate affordance.
    pub fn executed(&self) -> bool {
        matches!(self.outcome, OffloadOutcome::Executed { .. })
    }

    /// The affordance result if the op executed, else `None`.
    pub fn result(&self) -> Option<&OffloadAffordanceResult> {
        match &self.outcome {
            OffloadOutcome::Executed { result } => Some(result),
            _ => None,
        }
    }
}

/// The compute-offload engine: classify an operation, and if it is offload-
/// eligible execute it on the registered CPU substrate affordance for its kind,
/// recording every decision (and the cumulative `gpu_seconds_saved`) into an
/// [`OffloadLedger`].
///
/// Register affordances with [`Self::register`]. The default engine wires the
/// one real affordance ([`crate::GraphCentralityAffordance`]) for the
/// [`GraphAlgorithm`](OffloadOperationKind::GraphAlgorithm) kind; the other
/// three eligible kinds are unrouted until a real engine is wired, and
/// [`Self::route`] records them honestly as
/// [`OffloadOutcome::EligibleUnrouted`].
pub struct OffloadEngine {
    classifier: OffloadClassifier,
    affordances: HashMap<OffloadOperationKind, Box<dyn OffloadAffordance>>,
    ledger: OffloadLedger,
}

impl Default for OffloadEngine {
    /// An engine with the default classifier/cost-model and the one real-wired
    /// affordance registered: exact-PageRank graph centrality.
    fn default() -> Self {
        Self::new(OffloadClassifier::new())
            .with_affordance(crate::affordance::GraphCentralityAffordance::default())
    }
}

impl OffloadEngine {
    /// A bare engine with the given classifier and NO affordances registered.
    /// Every eligible op will record as eligible-but-unrouted until affordances
    /// are added.
    pub fn new(classifier: OffloadClassifier) -> Self {
        Self {
            classifier,
            affordances: HashMap::new(),
            ledger: OffloadLedger::new(),
        }
    }

    /// Register an affordance for the kind it serves (builder style). A later
    /// registration for the same kind replaces an earlier one.
    pub fn with_affordance(mut self, affordance: impl OffloadAffordance + 'static) -> Self {
        self.register(affordance);
        self
    }

    /// Register an affordance for the kind it serves.
    pub fn register(&mut self, affordance: impl OffloadAffordance + 'static) {
        self.affordances
            .insert(affordance.kind(), Box::new(affordance));
    }

    /// Whether an affordance is registered for `kind`.
    pub fn has_affordance(&self, kind: OffloadOperationKind) -> bool {
        self.affordances.contains_key(&kind)
    }

    /// The ledger (read-only): replayable decisions + cumulative
    /// `gpu_seconds_saved`.
    pub fn ledger(&self) -> &OffloadLedger {
        &self.ledger
    }

    /// Convenience: the cumulative GPU-seconds saved so far.
    pub fn gpu_seconds_saved(&self) -> f64 {
        self.ledger.gpu_seconds_saved()
    }

    /// Classify `operation` WITHOUT executing or recording it: the classifier
    /// verdict, annotated with the affordance the engine would use if one is
    /// registered for the kind. Useful for a dry-run / planning surface.
    pub fn classify(&self, operation: &OffloadOperation) -> OffloadDecision {
        let mut decision = self.classifier.classify(operation);
        if decision.eligible {
            decision.affordance = self
                .affordances
                .get(&operation.kind)
                .map(|affordance| affordance.id().to_string());
        }
        decision
    }

    /// Route one operation: classify it, and if eligible execute it on the
    /// registered affordance for its kind, recording the decision + result (or
    /// the honest eligible-but-unrouted state) into the ledger.
    ///
    /// The GPU-seconds credited:
    /// * executed -> realized saving (GPU baseline minus measured CPU time),
    /// * eligible-but-unrouted -> the estimate (available once an affordance is
    ///   wired),
    /// * execution failed / not eligible -> `0.0`.
    pub fn route(&mut self, operation: &OffloadOperation) -> OffloadRouteReport {
        let mut decision = self.classify(operation);

        // Non-eligible: stays on the GPU. Honest zero.
        if !decision.eligible {
            return self.finish(operation, decision, OffloadOutcome::NotEligible, 0.0, Value::Null);
        }

        // Eligible but no affordance wired: honest eligible-but-unrouted. Credit
        // the ESTIMATE (the saving that becomes real once an affordance lands).
        let Some(affordance) = self.affordances.get(&operation.kind) else {
            let estimate = decision.estimated_gpu_seconds_saved;
            let outcome = OffloadOutcome::EligibleUnrouted {
                kind: operation.kind,
            };
            decision.rationale = format!(
                "{} (no affordance registered for {}; eligible-but-unrouted, estimate banked until one is wired)",
                decision.rationale,
                operation.kind.tag(),
            );
            return self.finish(operation, decision, outcome, estimate, Value::Null);
        };

        // Eligible + wired: run the real substrate computation.
        match affordance.execute(operation) {
            Ok(result) => {
                // Realized saving from the measured CPU time, via the same cost
                // model the classifier used (so estimate vs realized are
                // comparable).
                let realized = self
                    .classifier
                    .cost_model()
                    .realized_saving(operation.units, result.cpu_seconds);
                let summary = result.summary.clone();
                let outcome = OffloadOutcome::Executed { result };
                self.finish(operation, decision, outcome, realized, summary)
            }
            Err(error) => {
                let outcome = OffloadOutcome::ExecutionFailed {
                    error: error.clone(),
                };
                decision.rationale =
                    format!("{} (affordance failed: {})", decision.rationale, error);
                self.finish(operation, decision, outcome, 0.0, Value::Null)
            }
        }
    }

    /// Record one decision row into the ledger and build the route report.
    fn finish(
        &mut self,
        operation: &OffloadOperation,
        decision: OffloadDecision,
        outcome: OffloadOutcome,
        gpu_seconds_saved: f64,
        result_summary: Value,
    ) -> OffloadRouteReport {
        self.ledger.record(LedgerEntry {
            operation: operation.label.clone(),
            kind: operation.kind,
            units: operation.units.max(1),
            decision: decision.clone(),
            outcome: outcome.clone(),
            gpu_seconds_saved,
            result_summary,
        });
        OffloadRouteReport {
            decision,
            outcome,
            gpu_seconds_saved,
        }
    }
}
