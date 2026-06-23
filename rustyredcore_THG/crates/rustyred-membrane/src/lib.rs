//! Shared context admission and eviction membrane.
//!
//! The context window is treated as a cache over graph-resident nodes. Callers
//! generate candidates, a [`Scorer`] ranks them for the current arm, and the
//! shared gate admits a budgeted subset while converting overflow into
//! recoverable graph handles.

pub mod compaction;
pub mod gate;
pub mod receipt;
pub mod scorer;

/// Graph-backed lossless overflow recovery (`context_fetch`) + receipt emit.
/// Behind the `graph-store` feature so the default crate has no graph dep.
#[cfg(feature = "graph-store")]
pub mod recover;

pub use compaction::{
    page_back, page_back_with_scorer, CompactionFitnessScorer, CompactionWeights,
};
pub use gate::{fill_to_budget, Admission, Handle};
pub use receipt::{MembraneReceipt, Source};
pub use scorer::{Candidate, EpistemicFeatures, ScoreContext, Scorer, SourceArm};

#[cfg(feature = "graph-store")]
pub use recover::{
    admit_to_budget, context_fetch, emit_receipt, persist_deferred, DEFERRED_CONTEXT_LABEL,
};
