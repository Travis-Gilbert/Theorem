//! The replayable decision ledger ([`OffloadLedger`]) and its rows
//! ([`LedgerEntry`]).

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::engine::OffloadOutcome;
use crate::operation::{OffloadDecision, OffloadOperationKind};

/// One recorded routing decision: the operation, the classifier verdict, the
/// outcome, and the GPU-seconds this entry actually credited to the cumulative
/// total. Replayable: serializing the `Vec<LedgerEntry>` and re-summing
/// `gpu_seconds_saved` reproduces the engine's running total exactly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// The operation label.
    pub operation: String,
    /// The operation kind tag.
    pub kind: OffloadOperationKind,
    /// The operation scale.
    pub units: u64,
    /// The classifier verdict (eligibility + estimate + chosen affordance).
    pub decision: OffloadDecision,
    /// What actually happened (executed / eligible-but-unrouted / not eligible).
    pub outcome: OffloadOutcome,
    /// GPU-seconds this entry credited to the cumulative total. For an executed
    /// op this is the realized saving (GPU baseline minus measured CPU time);
    /// for eligible-but-unrouted it is the estimate (the saving available once an
    /// affordance is wired); for a non-eligible op it is `0.0`.
    pub gpu_seconds_saved: f64,
    /// A compact machine-readable result summary when the op executed, else null.
    pub result_summary: Value,
}

/// An append-only ledger of offload decisions plus the cumulative
/// `gpu_seconds_saved` - the monetization metric from the thesis (Part B: the
/// measured GPU-second delta is the cost foundation for a credit-per-month
/// product).
///
/// The ledger distinguishes *credited* savings (the running total, summed over
/// every entry including eligible-but-unrouted estimates) from *realized*
/// savings (only ops that actually executed on a wired affordance) via
/// [`Self::realized_gpu_seconds_saved`], so a reader can tell banked-from-real-
/// compute from would-save-once-wired.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OffloadLedger {
    entries: Vec<LedgerEntry>,
    gpu_seconds_saved: f64,
}

impl OffloadLedger {
    /// An empty ledger.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one decision row, crediting its `gpu_seconds_saved` to the
    /// cumulative total. Returns the credited amount for convenience.
    pub fn record(&mut self, entry: LedgerEntry) -> f64 {
        let credited = entry.gpu_seconds_saved;
        self.gpu_seconds_saved += credited;
        self.entries.push(entry);
        credited
    }

    /// Cumulative GPU-seconds saved across every recorded decision (the headline
    /// monetization metric). Includes eligible-but-unrouted estimates.
    pub fn gpu_seconds_saved(&self) -> f64 {
        self.gpu_seconds_saved
    }

    /// Cumulative GPU-seconds saved counting ONLY operations that actually
    /// executed on a wired substrate affordance (realized, not estimated).
    pub fn realized_gpu_seconds_saved(&self) -> f64 {
        self.entries
            .iter()
            .filter(|entry| matches!(entry.outcome, OffloadOutcome::Executed { .. }))
            .map(|entry| entry.gpu_seconds_saved)
            .sum()
    }

    /// All recorded entries, in record order (replayable).
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// Number of recorded decisions.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ledger has no entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
