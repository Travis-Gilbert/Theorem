//! Ensemble: the capability layer over RustyRedCore-THG.
//!
//! Ensemble is the pack-level registry + budgeted selector + trust ladder that sits
//! ABOVE the tool-level `rustyred-thg-affordances` crate (which selects individual
//! tools/connectors) and the single-kind `theorem-harness-runtime::skill_pack` (which
//! serves only `kind == skill_pack`). It registers `CapabilityPack`s of every kind
//! (skill, agent, tool, validator, renderer, compute, policy, domain, context) as
//! content-addressed nodes in the same GraphStore skill packs use, and -- in later
//! slices -- selects which packs/agents/tools to bring in per task under a budget,
//! emitting a replayable `EnsembleDecision`.
//!
//! Status: slices S1 (registry), S2 (budgeted selector + replayable `EnsembleDecision`), and S3
//! (trust-ladder gating in selection) are implemented as a pure library. MCP exposure (S4 --
//! `ensemble_register` / `ensemble_select` in Codex's hot `rustyred-thg-mcp`) stays a coordinated
//! follow-up. Tracked in `docs/plans/ensemble/ensemble-rs-implementation-plan.md`.

pub mod decision;
pub mod outcomes;
pub mod registry;
pub mod selector;
pub mod trust;

pub use decision::{EnsembleDecision, RejectedCandidate, SelectedCapability};
pub use outcomes::{
    effective_pack_fitness_from_node, record_pack_invocation, PackInvocationRecordRequest,
    PackInvocationRecordResult, PACK_INVOCATION_RECEIPT_LABEL, PACK_SEQUENCED_WITH,
    PACK_SERVED_TASK, TASK_IN_DOMAIN,
};
pub use registry::{
    get_pack, list_packs, pack_node_id, register_pack, CapabilityPack, EnsembleError,
    EnsembleGraphStore, EnsembleResult, PackExposure, PackKind, TrustTier, PACK_ARTIFACT_EDGE,
    PACK_EXPOSES_AFFORDANCE, PACK_IN_DOMAIN, PACK_LABEL, PACK_SOURCE_EDGE,
};
pub use selector::{
    select, select_from_store, select_unified_from_store, EnsembleSelectRequest,
    UnifiedSelectionEntry, UnifiedSelectionRequest, UnifiedSelectionResult,
};
pub use trust::{meets_floor, parse_trust_floor, passport_id, trust_rank, trust_score};
