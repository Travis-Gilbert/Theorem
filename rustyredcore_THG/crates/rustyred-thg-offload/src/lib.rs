//! Compute-offload engine for the RustyRed substrate.
//!
//! Grounds the compute-offload thesis from
//! `docs/reference/commonplace-substrate-architecture-part-4-1.md` (Part A): a
//! meaningful class of operations currently done inside LLM forward passes -
//! (1) logical/Datalog derivation, (2) probabilistic/reliability inference,
//! (3) constraint solving, (4) graph algorithms (PageRank/PPR/community/
//! centrality) - are CHEAPER (roughly two orders of magnitude per op) and MORE
//! CORRECT (exact, not approximated) when run as CPU substrate compute instead
//! of GPU inference. Language understanding / synthesis / generation
//! ([`OffloadOperationKind::NeuralSynthesis`]) genuinely needs the GPU and is
//! NOT offload-eligible.
//!
//! This crate is the missing *engine*: it
//!
//! * classifies an [`OffloadOperation`] as offload-eligible or not
//!   ([`OffloadClassifier`]),
//! * routes an eligible op to a registered CPU substrate affordance
//!   ([`OffloadAffordance`]) via [`OffloadEngine::route`],
//! * and records every decision plus the cumulative `gpu_seconds_saved` into an
//!   [`OffloadLedger`] - the monetization metric the thesis names (Part B: the
//!   measured GPU-second delta is what makes a credit-per-month product viable).
//!
//! ## Honesty contract
//!
//! Only ONE affordance is wired against a real substrate API: the
//! [`GraphAlgorithm`](OffloadOperationKind::GraphAlgorithm) affordance
//! ([`GraphCentralityAffordance`]) computes EXACT PageRank on the CPU via
//! [`rustyred_thg_core::pagerank`] (an `EdgeRecord` power-iteration). The other
//! three eligible kinds are eligible-but-unrouted until a real engine is wired:
//! [`OffloadEngine::route`] records them honestly as
//! [`OffloadOutcome::EligibleUnrouted`] (with the GPU-seconds that WOULD be
//! saved), never a fabricated result. A non-eligible op records
//! [`OffloadOutcome::NotEligible`]. This mirrors the ambient-pass discipline:
//! report a missing capability as a degraded state, never guess one.

#![forbid(unsafe_code)]

mod affordance;
mod classifier;
mod engine;
mod ledger;
mod operation;

pub use affordance::{
    GraphCentralityAffordance, OffloadAffordance, OffloadAffordanceError, OffloadAffordanceResult,
    GRAPH_PAGERANK_AFFORDANCE,
};
pub use classifier::{CostModel, OffloadClassifier};
pub use engine::{OffloadEngine, OffloadOutcome, OffloadRouteReport};
pub use ledger::{LedgerEntry, OffloadLedger};
pub use operation::{OffloadDecision, OffloadOperation, OffloadOperationKind};
