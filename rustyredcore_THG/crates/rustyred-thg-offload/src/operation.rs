//! The described unit of work ([`OffloadOperation`]), the class taxonomy
//! ([`OffloadOperationKind`]), and the classifier verdict ([`OffloadDecision`]).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// The class of an operation, used to decide offload eligibility.
///
/// The first four are the thesis's offload-eligible symbolic classes (Part A):
/// each is something LLMs do "badly and expensively" and symbolic CPU compute
/// does exactly and cheaply. [`NeuralSynthesis`](Self::NeuralSynthesis) is the
/// one that genuinely needs the GPU - language understanding, synthesis, and
/// generation - and is NOT offload-eligible.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OffloadOperationKind {
    /// Exact logical derivation ("what follows from these facts") - Datalog, not
    /// an LLM reasoning chain. Offload-eligible.
    LogicalDerivation,
    /// Source reliability / expected-value-of-information - Beta-Bernoulli math,
    /// not an LLM estimate. Offload-eligible.
    ProbabilisticInference,
    /// Constraint satisfaction ("can these assumptions all hold") - a solver,
    /// not an LLM reasoning about consistency. Offload-eligible.
    ConstraintSolving,
    /// Graph algorithms (PageRank/PPR/community/centrality/shortest paths) - the
    /// substrate graph engine, not an LLM approximating graph structure.
    /// Offload-eligible, and the one kind wired to a real affordance here.
    GraphAlgorithm,
    /// Language understanding, synthesis, generation. Genuinely needs a neural
    /// model on the GPU. NOT offload-eligible - the honest "this stays on GPU".
    NeuralSynthesis,
}

impl OffloadOperationKind {
    /// Whether this class is offload-eligible (cheaper + more correct as CPU
    /// substrate compute). The four symbolic classes are; neural synthesis is
    /// not. This is the single source of the eligibility predicate.
    pub fn is_offload_eligible(self) -> bool {
        match self {
            OffloadOperationKind::LogicalDerivation
            | OffloadOperationKind::ProbabilisticInference
            | OffloadOperationKind::ConstraintSolving
            | OffloadOperationKind::GraphAlgorithm => true,
            OffloadOperationKind::NeuralSynthesis => false,
        }
    }

    /// Stable lower-case tag for queryable receipts / ledger rows.
    pub fn tag(self) -> &'static str {
        match self {
            OffloadOperationKind::LogicalDerivation => "logical_derivation",
            OffloadOperationKind::ProbabilisticInference => "probabilistic_inference",
            OffloadOperationKind::ConstraintSolving => "constraint_solving",
            OffloadOperationKind::GraphAlgorithm => "graph_algorithm",
            OffloadOperationKind::NeuralSynthesis => "neural_synthesis",
        }
    }
}

/// A described unit of work the engine can classify and (if eligible) route.
///
/// The operation is described, not embodied: `kind` drives classification, and
/// `payload` carries whatever the matching affordance needs to actually compute
/// (e.g. the graph edges for the [`GraphAlgorithm`](OffloadOperationKind::GraphAlgorithm)
/// affordance). `units` is the operation's scale (e.g. node count for a graph
/// op, fact count for a derivation) and feeds the cost model so a bigger op
/// records a bigger GPU-second delta.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffloadOperation {
    /// A human/machine label for this operation instance (e.g.
    /// `"similar_item_centrality"`). Carried through into the ledger.
    pub label: String,
    /// The class that drives eligibility.
    pub kind: OffloadOperationKind,
    /// The scale of the operation (nodes, facts, constraints...). Used by the
    /// cost model to scale the estimated GPU-seconds saved; `>= 1` is sane.
    pub units: u64,
    /// Affordance-specific input. The graph-centrality affordance reads
    /// `payload["edges"]` (an array of `EdgeRecord`); other affordances define
    /// their own shape. Empty object when the op carries no inline input.
    pub payload: Value,
}

impl OffloadOperation {
    /// A bare operation of `kind` labeled `label`, scaled to one unit and with an
    /// empty payload. Builder methods add scale / payload.
    pub fn new(label: impl Into<String>, kind: OffloadOperationKind) -> Self {
        Self {
            label: label.into(),
            kind,
            units: 1,
            payload: json!({}),
        }
    }

    /// Set the operation scale (clamped to at least 1 so the cost model never
    /// reads a zero-unit op as free).
    pub fn with_units(mut self, units: u64) -> Self {
        self.units = units.max(1);
        self
    }

    /// Attach the affordance input payload.
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }
}

/// The classifier verdict for one [`OffloadOperation`]: whether it is eligible,
/// which affordance would serve it (if one is registered for the kind), the
/// estimated GPU-seconds the offload saves, and a human-readable rationale.
///
/// `estimated_gpu_seconds_saved` is the delta from the [`CostModel`](crate::CostModel):
/// the GPU-second cost of doing the op as a forward pass MINUS the CPU cost of
/// the substrate computation. For a non-eligible op it is `0.0` (nothing is
/// moved off the GPU). It is an estimate from a documented model - the exact
/// numbers are a knob; the point is a recorded, replayable cost delta (thesis
/// Part B).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OffloadDecision {
    /// Whether the operation should run as CPU substrate compute.
    pub eligible: bool,
    /// The affordance id that would serve this op, if one is registered for the
    /// kind. `None` for a non-eligible op, or for an eligible kind that has no
    /// affordance wired yet (eligible-but-unrouted).
    pub affordance: Option<String>,
    /// Estimated GPU-seconds saved by offloading (cost model delta). `0.0` for a
    /// non-eligible op.
    pub estimated_gpu_seconds_saved: f64,
    /// Why this verdict was reached.
    pub rationale: String,
}
