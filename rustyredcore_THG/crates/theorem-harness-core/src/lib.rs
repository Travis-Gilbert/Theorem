//! # theorem-harness-core
//!
//! The pure harness kernel: run state, transition guards, content-addressed
//! state hashing, replay/fork helpers, and permission-aware toolkit selection.
//! It has no storage and no network — logic only, parity-tested against the
//! Python reference corpora — so the GraphStore-backed persistence
//! (`theorem-harness-runtime`) and the SDK surface (`theorem-harness`) layer on
//! top without the kernel ever depending on them.
//!
//! Key modules:
//! - [`state_machine`] and [`state_hash`]: the transition executor, guard table,
//!   and the content-addressed hash of run state after each transition.
//! - [`replay`]: deterministic replay and fork from a transition sequence.
//! - [`toolgraph`]: permission-aware toolkit compilation (which tools a run may use).
//! - [`job`]: the typed Dispatch v2 job domain.
//! - [`work_graph`] and [`work_graph_verify`]: the durable multi-head work graph
//!   and its adversarial-verify contracts.
//! - [`affordances`]: the contract types learned tool selection ranks over.
//! - [`agent_binding`], [`memory_contracts`], [`map_artifacts`]: shared contract
//!   types the runtime and SDK persist.
//!
//! Product-level picture: see `docs/site/concepts/the-harness.md`.

pub mod affordances;
pub mod agent_binding;
pub mod agent_head_registry;
pub mod alignment;
pub mod attribution;
pub mod budget;
pub mod config_ledger;
pub mod constitution;
pub mod context_manager;
pub mod context_web;
pub mod epistemic_fitness;
pub mod federated_signals;
pub mod head_fitness;
pub mod head_invocation;
pub mod improvement_rate;
pub mod intra_agent_loop;
pub mod job;
pub mod loop_gate;
pub mod map_artifacts;
pub mod memory_contracts;
pub mod metrics_composite;
pub mod provider_head_adapter;
pub mod replay;
pub mod scheduler;
pub mod session_metrics;
pub mod shadow_eval;
pub mod state_hash;
pub mod state_machine;
pub mod toolgraph;
pub mod types;
pub mod work_graph;
pub mod work_graph_verify;

pub use affordances::{
    AffordanceContract, AffordanceReceipt, affordance_by_id, affordance_ids,
    default_affordance_registry, validate_affordance_registry,
};
pub use agent_binding::{
    ActionTierPolicy, AgentBinding, AgentHead, BindingBudgetScope, BindingCapabilityScope,
    BindingComposition, BindingError, BindingEventState, BindingIdentity, BindingLifecycleState,
    BindingMemoryScope, BindingRoutingDecision, BindingSubtask, BindingTraceScope,
    BindingTransitionInput, BindingTransitionResult, BindingVerificationOutcome,
    BindingVerificationReceipt, HeadBudgetLimit, HeadCapabilityReliability, HeadContributionRecord,
    HeadCostProfile, HeadKind, HeadReliabilityProfile, HeadTransport, MemoryZone, MemoryZoneKind,
    PublishedScope, ScratchpadDocument, ScratchpadRelationKind, ScratchpadRevision,
    ScratchpadRevisionLink, ScratchpadRevisionRelation, TraceTier, apply_binding_transition,
    composition_hash, hash_agent_binding,
};
pub use agent_head_registry::{
    AgentHeadEndpoint, AgentHeadKindSummary, AgentHeadRegistry, AgentHeadRegistryError,
    RegisteredAgentHead, ResolvedAgentHead,
};
pub use alignment::{MIN_CONSENSUS_HEADS, evaluate_publication};
pub use attribution::{
    ConfigAttributionCounter, ConfigAttributionTable, ConfigOutcome, ConfigRunAttribution,
};
pub use budget::{BindingBudgetState, apply_contribution_charge, check_contribution_budget};
pub use config_ledger::{
    AppliedConfigDelta, ConfigDelta, ConfigDeltaStatus, ConfigLedger, ConfigLedgerError,
    ConfigState, ConfigValueDelta, RollbackReceipt,
};
pub use constitution::{
    Constitution, GLOBAL_LAW_LAYER, LIVE_EVIDENCE_LAYER, PROJECT_LAW_LAYER, REQUEST_LAYER,
    default_authority_order,
};
pub use context_manager::{
    CLEARED_TOOL_RESULT_PLACEHOLDER, COMPACTION_TRIGGER_THRESHOLD, ContextCheckResult,
    ContextGuard, ContextManager, ContextManagerConfig, ContextManagerStats, ContextMessage,
    ContextReduction, DEFAULT_KEEP_RECENT_TOOL_RESULTS, HARD_LIMIT_THRESHOLD,
    MAX_CONSECUTIVE_REDUCTION_FAILURES, MicrocompactStats, ProviderUsage, ToolCallEnvelope,
    ToolResultEnvelope, autocompact_with_summary, microcompact,
};
pub use context_web::{
    ContextWebAtom, ContextWebBudget, ContextWebCitation, ContextWebEdge, ContextWebEvaluation,
    ContextWebIndex, ContextWebPack, ContextWebPackInput, ContextWebPath, ContextWebPolicy,
    ContextWebSpendPlan, ContextWebStructuralBankResult, ContextWebTokenLedger,
    ContextWebValidationSummary, ContextWebValidatorFinding, is_generated_artifact,
    normalize_context_web_node_id,
};
pub use epistemic_fitness::{
    FitnessGateResult, FitnessObservation, FitnessTraitViolation, check_epistemic_fitness,
    measure_fitness_traits,
};
pub use federated_signals::{
    FederatedSignal, PrivacyViolation, StructuralSignalInput, assert_no_raw_content,
    extract_structural_signal, observed_count_bucket, receive_federated_signal,
    success_rate_bucket,
};
pub use head_fitness::{FitnessCounter, HeadFitness, NodeResult, RoutingPolicy};
pub use head_invocation::{
    FakeHeadInvoker, GroundedClaim, HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt,
    HeadInvocationRequest, HeadInvoker, RevisionContext,
};
pub use improvement_rate::{
    AxisRate, CompositePoint, ImprovementRate, composite_point, composite_points_from_metrics,
    compute_improvement_rate, load_composite_points_from_jsonl,
};
pub use intra_agent_loop::{
    FakeIntraAgentLoopInput, FakeIntraAgentLoopResult, IntraAgentLoopError,
    run_fake_intra_agent_loop, run_intra_agent_loop_with_invoker,
};
pub use job::{
    Job, JobReceipt, JobSubmission, LANE_CLAUDE, LANE_CODEX, Priority, TargetHead,
    idempotency_key_for, new_job_id,
};
pub use loop_gate::{
    LoopClosureBudget, LoopClosureInput, LoopGateRejection, LoopGateState, LoopGateVerdict,
    close_loop_if_allowed, evaluate_loop_gate,
};
pub use map_artifacts::{
    MapArtifactCompileInput, MapArtifactState, MapDeltaState, MapEntry, compile_map_artifact,
    describe_map_artifact, scope_for_map_kind, stable_map_id,
};
pub use memory_contracts::{
    PrepareMemoryBank, PrepareMemoryContract, PrepareMemoryEvidence, PrepareMemoryHydrationHandle,
    PrepareMemoryPolicy, PrepareMemoryRecallPolicy, PrepareMemoryRecallPreview,
};
pub use metrics_composite::{
    COMPOSITE_VERSION, CompositeAxes, FitnessTraitScores, HarnessComposite, composite,
    composite_with_safety, session_composite_point,
};
pub use provider_head_adapter::ProviderHeadExecutionContext;
pub use replay::{ReplayError, compare_runs, fork_events, fork_run, replay_events, replay_run};
pub use scheduler::next_for_head;
pub use session_metrics::{
    PAIRFORMER_MODES, PairformerSummaryRow, SessionMetricsState, compare_modes, load_jsonl_metrics,
    summarize_pairformer_ab,
};
pub use shadow_eval::{
    ShadowEvalInput, ShadowEvalResult, ShadowRunPair, ShadowVerdict, evaluate_shadow,
};
pub use state_hash::{empty_state_hash, hash_run_state, stable_value_hash};
pub use state_machine::{HarnessError, apply_transition};
pub use toolgraph::{
    BlockedTool, CompiledToolkit, DEFAULT_PERMISSIONS, ToolContract, ToolSelectionState,
    catalog_as_dicts, compile_task_toolkit, normalize_permissions, select_tools,
};
pub use types::{
    AgentRunState, AgentStepState, EventState, GuardViolation, Payload, PolicyCheck,
    PolicyDecision, PolicyLayer, RunState, TransitionInput, TransitionResult,
};
pub use work_graph::{
    ClaimLease, ClaimOutcome, Millis, NodeStatus, Receipt, TaskNode, WorkGraph, WorkGraphError,
    claim_task_node, heartbeat_task_node,
};
pub use work_graph_verify::{
    VERIFY_NODE_TYPE, VerifyOutcome, VerifyReceipt, spawn_verify_node, submit_verify_receipt,
    verify_node_id,
};
