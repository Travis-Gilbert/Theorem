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
//! Status: slice S1 (registry) implemented. Budgeted selector (S2), trust gating in
//! selection (S3), and MCP exposure (S4, Codex's hot `rustyred-thg-mcp`) are tracked in
//! `docs/plans/ensemble/ensemble-rs-implementation-plan.md`.

pub mod decision;
pub mod registry;

pub use decision::{EnsembleDecision, RejectedCandidate, SelectedCapability};
pub use registry::{
    get_pack, pack_node_id, register_pack, CapabilityPack, EnsembleError, EnsembleGraphStore,
    EnsembleResult, PackExposure, PackKind, TrustTier, PACK_ARTIFACT_EDGE, PACK_LABEL,
    PACK_SOURCE_EDGE,
};
