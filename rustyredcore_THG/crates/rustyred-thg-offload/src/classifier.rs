//! The cost model ([`CostModel`]) and the eligibility classifier
//! ([`OffloadClassifier`]).

use serde::{Deserialize, Serialize};

use crate::operation::{OffloadDecision, OffloadOperation};

/// A simple, documented model for the GPU-seconds an offload saves.
///
/// The thesis (Part B) does not need a precise number; it needs a *recorded,
/// replayable cost delta*. The model is:
///
/// > `saved = units * (gpu_seconds_per_unit - cpu_seconds_per_unit)`, floored at 0
///
/// where `gpu_seconds_per_unit` is the per-op GPU-second baseline of doing the
/// operation inside an LLM forward pass and `cpu_seconds_per_unit` is the per-op
/// CPU cost of the substrate computation. The defaults encode the thesis's
/// "roughly two orders of magnitude cheaper per op" claim (GPU baseline ~100x
/// the CPU cost). The numbers are a knob (override per deployment / per measured
/// benchmark); the engine records whichever model produced the delta so the
/// decision is auditable.
///
/// When an affordance reports a *measured* `cpu_seconds`, the engine recomputes
/// the realized delta as `units * gpu_seconds_per_unit - measured_cpu_seconds`
/// (see [`CostModel::realized_saving`]), so the ledger's cumulative total
/// reflects real CPU time spent, not just the estimate.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    /// Per-unit GPU-second baseline of the operation as an LLM forward pass.
    pub gpu_seconds_per_unit: f64,
    /// Per-unit CPU-second cost of the substrate computation (the estimate used
    /// before a real measurement is available).
    pub cpu_seconds_per_unit: f64,
}

impl Default for CostModel {
    fn default() -> Self {
        // ~2 orders of magnitude: a unit costs 0.05 GPU-s as a forward pass vs
        // ~0.0005 CPU-s as substrate compute. The delta per unit is ~0.0495 s.
        Self {
            gpu_seconds_per_unit: 0.05,
            cpu_seconds_per_unit: 0.0005,
        }
    }
}

impl CostModel {
    /// Estimated GPU-seconds saved for an op of `units` scale, before any real
    /// CPU measurement: `units * (gpu_per_unit - cpu_per_unit)`, floored at 0.
    pub fn estimated_saving(&self, units: u64) -> f64 {
        let units = units.max(1) as f64;
        (units * (self.gpu_seconds_per_unit - self.cpu_seconds_per_unit)).max(0.0)
    }

    /// The GPU-second baseline that the offload AVOIDS: `units * gpu_per_unit`.
    pub fn gpu_baseline(&self, units: u64) -> f64 {
        units.max(1) as f64 * self.gpu_seconds_per_unit
    }

    /// Realized GPU-seconds saved given a *measured* CPU cost from the affordance:
    /// `gpu_baseline(units) - measured_cpu_seconds`, floored at 0. This is the
    /// honest delta the ledger accumulates after a real substrate run.
    pub fn realized_saving(&self, units: u64, measured_cpu_seconds: f64) -> f64 {
        (self.gpu_baseline(units) - measured_cpu_seconds).max(0.0)
    }
}

/// Classifies an [`OffloadOperation`] as offload-eligible or not, estimating the
/// GPU-seconds the offload would save.
///
/// The classifier is the policy layer: it does NOT execute anything. Eligibility
/// follows [`OffloadOperationKind::is_offload_eligible`] (the four symbolic
/// classes are eligible; neural synthesis is not). The GPU-second estimate comes
/// from the [`CostModel`]. The optional `affordance` field of the returned
/// [`OffloadDecision`] is filled in by the engine (which knows what is
/// registered); the classifier leaves it `None` and the engine annotates it.
#[derive(Clone, Copy, Debug, Default)]
pub struct OffloadClassifier {
    cost_model: CostModel,
}

impl OffloadClassifier {
    /// A classifier with the default ~2-orders-of-magnitude cost model.
    pub fn new() -> Self {
        Self::default()
    }

    /// A classifier with a custom cost model (e.g. seeded from a measured
    /// benchmark on a deployment).
    pub fn with_cost_model(cost_model: CostModel) -> Self {
        Self { cost_model }
    }

    /// The cost model this classifier uses (the engine reuses it to compute the
    /// realized saving from an affordance's measured CPU time).
    pub fn cost_model(&self) -> CostModel {
        self.cost_model
    }

    /// Classify one operation. An eligible op carries the estimated GPU-seconds
    /// saved; a non-eligible op carries `0.0` and a rationale that names why it
    /// stays on the GPU.
    pub fn classify(&self, operation: &OffloadOperation) -> OffloadDecision {
        let eligible = operation.kind.is_offload_eligible();
        if eligible {
            let saved = self.cost_model.estimated_saving(operation.units);
            OffloadDecision {
                eligible: true,
                affordance: None,
                estimated_gpu_seconds_saved: saved,
                rationale: format!(
                    "{} is exact + cheaper as CPU substrate compute (~{:.4} GPU-s saved over {} unit(s) at the configured model); offload-eligible",
                    operation.kind.tag(),
                    saved,
                    operation.units.max(1),
                ),
            }
        } else {
            OffloadDecision {
                eligible: false,
                affordance: None,
                estimated_gpu_seconds_saved: 0.0,
                rationale: format!(
                    "{} genuinely needs a neural model on the GPU (language/synthesis/generation); not offload-eligible",
                    operation.kind.tag(),
                ),
            }
        }
    }
}
