//! The substrate-computation seam ([`OffloadAffordance`]) and the one real-wired
//! affordance ([`GraphCentralityAffordance`], exact PageRank over the substrate
//! graph engine).

use std::collections::HashMap;

use rustyred_thg_core::{pagerank, EdgeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::operation::{OffloadOperation, OffloadOperationKind};

/// Affordance id of the wired exact-PageRank graph affordance. Stable so ledger
/// rows and tests can name it.
pub const GRAPH_PAGERANK_AFFORDANCE: &str = "graph_pagerank_exact";

/// Why a substrate affordance could not produce a result. This is a genuine
/// computation failure (malformed input, missing payload), distinct from "the
/// op was not eligible" and from "no affordance is registered for the kind" -
/// those are engine-level honest outcomes, not affordance errors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct OffloadAffordanceError {
    /// Machine-stable reason code.
    pub code: String,
    /// Human-readable detail.
    pub message: String,
}

impl OffloadAffordanceError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for OffloadAffordanceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for OffloadAffordanceError {}

/// What a substrate affordance produced for one operation: a structured,
/// serializable result plus the measured CPU-seconds the computation took.
///
/// The CPU cost is recorded honestly so the ledger's GPU-second saving is a real
/// delta (GPU-cost-avoided minus CPU-cost-spent), not a one-sided number.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffloadAffordanceResult {
    /// The affordance id that produced this (e.g. [`GRAPH_PAGERANK_AFFORDANCE`]).
    pub affordance: String,
    /// The structured result of the substrate computation.
    pub result: Value,
    /// A short machine-readable summary of the result for ledger rows / receipts
    /// (so a surface need not parse the full `result`).
    pub summary: Value,
    /// Measured CPU-seconds the computation took. Feeds the realized GPU-second
    /// delta in the ledger.
    pub cpu_seconds: f64,
}

/// A substrate affordance: the actual CPU computation that replaces an LLM
/// forward pass for one offload-eligible [`OffloadOperationKind`].
///
/// An affordance declares the single kind it serves ([`Self::kind`]) and an id
/// ([`Self::id`]); the engine registers it against that kind and calls
/// [`Self::execute`] when an eligible op of that kind is routed. `execute`
/// returns a real result or a genuine computation error - it never fakes a
/// result.
pub trait OffloadAffordance: Send + Sync {
    /// The kind this affordance serves. The engine routes ops of this kind here.
    fn kind(&self) -> OffloadOperationKind;

    /// Stable affordance id (carried into decisions and ledger rows).
    fn id(&self) -> &str;

    /// Run the substrate computation for `operation`. Returns the structured
    /// result + measured CPU-seconds, or a genuine computation error.
    fn execute(&self, operation: &OffloadOperation)
        -> Result<OffloadAffordanceResult, OffloadAffordanceError>;
}

/// REAL-WIRED graph-algorithm affordance: exact PageRank centrality over a
/// graph, computed on the CPU by [`rustyred_thg_core::pagerank`] (a power-
/// iteration over `EdgeRecord`s).
///
/// This is the thesis's "what are the most central entities in this knowledge
/// subgraph?" operation (Part B query set) done as exact substrate compute
/// instead of an LLM approximation. It reads `operation.payload["edges"]` as an
/// array of [`EdgeRecord`] (the substrate's own edge type), runs PageRank, and
/// returns per-node scores plus the single highest-ranked node.
#[derive(Clone, Copy, Debug)]
pub struct GraphCentralityAffordance {
    /// PageRank damping factor (standard 0.85).
    pub damping: f64,
    /// Power-iteration cap.
    pub max_iter: usize,
    /// L1 convergence tolerance.
    pub tolerance: f64,
}

impl Default for GraphCentralityAffordance {
    fn default() -> Self {
        // Canonical PageRank constants (Brin/Page): damping 0.85, generous
        // iteration cap, tight tolerance. Exact and deterministic.
        Self {
            damping: 0.85,
            max_iter: 100,
            tolerance: 1e-8,
        }
    }
}

impl GraphCentralityAffordance {
    /// Parse `payload["edges"]` into the substrate's [`EdgeRecord`] type.
    fn parse_edges(payload: &Value) -> Result<Vec<EdgeRecord>, OffloadAffordanceError> {
        let raw = payload.get("edges").ok_or_else(|| {
            OffloadAffordanceError::new(
                "missing_edges",
                "graph-centrality payload has no `edges` array",
            )
        })?;
        serde_json::from_value::<Vec<EdgeRecord>>(raw.clone()).map_err(|error| {
            OffloadAffordanceError::new(
                "malformed_edges",
                format!("`edges` did not deserialize as [EdgeRecord]: {error}"),
            )
        })
    }
}

impl OffloadAffordance for GraphCentralityAffordance {
    fn kind(&self) -> OffloadOperationKind {
        OffloadOperationKind::GraphAlgorithm
    }

    fn id(&self) -> &str {
        GRAPH_PAGERANK_AFFORDANCE
    }

    fn execute(
        &self,
        operation: &OffloadOperation,
    ) -> Result<OffloadAffordanceResult, OffloadAffordanceError> {
        let edges = Self::parse_edges(&operation.payload)?;

        // The actual substrate compute: exact power-iteration PageRank on the
        // CPU. `rustyred_thg_core::pagerank` returns score-per-node summing to
        // 1.0 (dangling mass redistributed) - exact, not an LLM approximation.
        let start = std::time::Instant::now();
        let scores: HashMap<String, f64> = pagerank(&edges, self.damping, self.max_iter, self.tolerance);
        let cpu_seconds = start.elapsed().as_secs_f64();

        // Highest-centrality node (the "most central entity"), deterministic on
        // ties via the node id.
        let top = scores
            .iter()
            .max_by(|a, b| {
                a.1.partial_cmp(b.1)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| b.0.cmp(a.0))
            })
            .map(|(id, score)| (id.clone(), *score));

        let summary = json!({
            "algorithm": "pagerank",
            "node_count": scores.len(),
            "edge_count": edges.iter().filter(|edge| !edge.tombstone).count(),
            "top_node": top.as_ref().map(|(id, _)| id.clone()),
            "top_score": top.as_ref().map(|(_, score)| *score),
        });
        let result = json!({
            "algorithm": "pagerank",
            "damping": self.damping,
            "scores": scores,
        });

        Ok(OffloadAffordanceResult {
            affordance: self.id().to_string(),
            result,
            summary,
            cpu_seconds,
        })
    }
}
