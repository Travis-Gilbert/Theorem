//! Connector-as-substrate learning registry.
//!
//! MCP connector tools become first-class `Affordance` graph nodes. The
//! substrate learns which affordances to reach for from accumulated outcomes
//! (PPR over `SERVED_TASK`/`PRODUCED_OUTCOME` edges + fitness), scoped per
//! agent by a `CapabilityScope` (the capability-scope plane of the
//! AgentBinding). This is not a passthrough aggregator: selection compounds
//! with use because it rides the same graph the rest of the substrate learns on.
//!
//! Layering: this crate sits above `rustyred-thg-core` (graph store, PPR) and
//! reuses `theorem-harness-core` (the affordance vocabulary + the pairformer
//! A/B validation gate). It is the structural sibling of the LoRA adapter
//! catalog (`rustyred-thg-adapters`) and the dependency-shape sibling of
//! `theorem-harness-runtime`.

pub mod outcomes;
pub mod registry;
pub mod selection;
pub mod training;
pub mod types;

pub use outcomes::{
    affordance_nodes, effective_affordance_fitness_from_node, record_invocation,
};
pub use registry::{register_builtin_affordances, register_connector, upsert_affordance};
pub use selection::{select_affordances, select_affordances_by_embedding};
pub use training::{
    export_affordance_training_view, pairformer_eval_node_id, pairformer_model_node_id,
    pairformer_validation_gate, register_pairformer_artifact, AffordanceRankingPair,
    AffordanceTrainingExport, PairformerArtifactInput, PairformerWritebackResult,
    EVALUATED_BY, EVALUATION_RECEIPT_LABEL, MODEL_ARTIFACT_LABEL, PROMOTED_TO_ACTIVE, TRAINED_ON,
};
pub use types::{
    affordance_node_id, affordance_vector_designation, connector_node_id,
    edge_with_affordance_provenance, invocation_receipt_node_id, normalize_tenant_id,
    task_type_node_id, tenant_node_id, Affordance, AffordanceGraphStore, AffordanceRef,
    AffordanceUpsertResult, CapabilityScope, ConnectorManifest, ConnectorRegisterResult,
    InvocationRecordRequest, InvocationRecordResult, SelectionRequest, ToolManifest,
    AFFORDANCE_LABEL, CONNECTOR_LABEL, DEFAULT_BASE_FITNESS, DEFAULT_COLD_START_SCORE,
    DEFAULT_MIN_FITNESS, INVOCATION_RECEIPT_LABEL, OFFERS, PRODUCED_OUTCOME, SEQUENCED_WITH,
    SERVED_TASK, TASK_TYPE_LABEL, THG_AFFORDANCE_SOURCE,
};

#[cfg(test)]
#[path = "tests/registry_test.rs"]
mod registry_test;

#[cfg(test)]
#[path = "tests/outcomes_test.rs"]
mod outcomes_test;

#[cfg(test)]
#[path = "tests/selection_test.rs"]
mod selection_test;

#[cfg(test)]
#[path = "tests/training_test.rs"]
mod training_test;
