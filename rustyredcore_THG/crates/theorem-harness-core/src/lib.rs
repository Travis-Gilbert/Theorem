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
//! - [`cmh`]: pure CMH atom-id and handoff-hash contracts.
//! - [`bgi`]: deterministic BGI JSON/hash receipt contracts.
//! - [`agent_binding`], [`memory_contracts`], [`map_artifacts`]: shared contract
//!   types the runtime and SDK persist.
//!
//! Product-level picture: see `docs/site/concepts/the-harness.md`.

pub mod affordances;
pub mod agent_binding;
pub mod agent_head_registry;
pub mod alignment;
pub mod attribution;
pub mod bgi;
pub mod budget;
pub mod cmh;
pub mod config_ledger;
pub mod constitution;
pub mod context_manager;
pub mod context_web;
pub mod epistemic_fitness;
pub mod federated_signals;
pub mod gepa_feedback;
pub mod gepa_proposer;
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
pub mod user_model;
pub mod work_graph;
pub mod work_graph_verify;

pub use affordances::{
    affordance_by_id, affordance_ids, default_affordance_registry, validate_affordance_registry,
    AffordanceContract, AffordanceReceipt,
};
pub use agent_binding::{
    apply_binding_transition, composition_hash, hash_agent_binding, ActionTierPolicy, AgentBinding,
    AgentHead, BindingBudgetDecision, BindingBudgetScope, BindingCapabilityScope,
    BindingComposition, BindingError, BindingEventState, BindingHeadOutcome, BindingIdentity,
    BindingLifecycleState, BindingLineageMemoryEntry, BindingMemoryScope, BindingRoutingDecision,
    BindingSubtask, BindingTraceScope, BindingTransitionInput, BindingTransitionResult,
    BindingVerificationOutcome, BindingVerificationReceipt, HeadBudgetLimit,
    HeadCapabilityReliability, HeadContributionRecord, HeadCostProfile, HeadKind,
    HeadReliabilityProfile, HeadTransport, MemoryZone, MemoryZoneKind, PublishedScope,
    ScratchpadAwarenessEntry, ScratchpadCrdtBacking, ScratchpadCrdtKind, ScratchpadCrdtOperation,
    ScratchpadDocument, ScratchpadRelationKind, ScratchpadRevision, ScratchpadRevisionLink,
    ScratchpadRevisionRelation, ScratchpadTextRegion, TraceTier,
};
pub use agent_head_registry::{
    AgentHeadEndpoint, AgentHeadKindSummary, AgentHeadRegistry, AgentHeadRegistryError,
    RegisteredAgentHead, ResolvedAgentHead,
};
pub use alignment::{evaluate_publication, MIN_CONSENSUS_HEADS};
pub use attribution::{
    ConfigAttributionCounter, ConfigAttributionTable, ConfigOutcome, ConfigRunAttribution,
};
pub use budget::{apply_contribution_charge, check_contribution_budget, BindingBudgetState};
pub use cmh::{cmh_atom_id_v1, cmh_body_hash, cmh_handoff_state_hash_v1};
pub use config_ledger::{
    AppliedConfigDelta, ConfigDelta, ConfigDeltaStatus, ConfigLedger, ConfigLedgerError,
    ConfigState, ConfigValueDelta, RollbackReceipt,
};
pub use constitution::{
    default_authority_order, Constitution, GLOBAL_LAW_LAYER, LIVE_EVIDENCE_LAYER,
    PROJECT_LAW_LAYER, REQUEST_LAYER,
};
pub use context_manager::{
    autocompact_with_summary, microcompact, ContextCheckResult, ContextGuard, ContextManager,
    ContextManagerConfig, ContextManagerStats, ContextMessage, ContextReduction, MicrocompactStats,
    ProviderUsage, ToolCallEnvelope, ToolResultEnvelope, CLEARED_TOOL_RESULT_PLACEHOLDER,
    COMPACTION_TRIGGER_THRESHOLD, DEFAULT_KEEP_RECENT_TOOL_RESULTS, HARD_LIMIT_THRESHOLD,
    MAX_CONSECUTIVE_REDUCTION_FAILURES,
};
pub use context_web::{
    is_generated_artifact, normalize_context_web_node_id, ContextWebAtom, ContextWebBudget,
    ContextWebCitation, ContextWebEdge, ContextWebEvaluation, ContextWebIndex, ContextWebPack,
    ContextWebPackInput, ContextWebPath, ContextWebPolicy, ContextWebSpendPlan,
    ContextWebStructuralBankResult, ContextWebTokenLedger, ContextWebValidationSummary,
    ContextWebValidatorFinding,
};
pub use epistemic_fitness::{
    check_epistemic_fitness, measure_fitness_traits, FitnessGateResult, FitnessObservation,
    FitnessTraitViolation,
};
pub use federated_signals::{
    assert_no_raw_content, extract_structural_signal, observed_count_bucket,
    receive_federated_signal, success_rate_bucket, FederatedSignal, PrivacyViolation,
    StructuralSignalInput,
};
pub use gepa_feedback::{
    fitness_gate_from_observations, gepa_feedback_point, reservoir_feedback_from_observations,
    FeedbackPoint, GepaFeedbackInput, ReservoirFeedback,
};
pub use gepa_proposer::{
    configured_head_system_prompt, default_gepa_instruction_registry, export_trainset_for_intent,
    gepa_val_subscores, head_role_instruction_key, ingest_gepa_candidate,
    route_gepa_candidate_through_gate, serve_user_prompt_improver,
    serve_user_prompt_improver_with_head, trainset_jsonl, GepaGateRouteInput, GepaGateRouteResult,
    GepaInstructionCandidate, GepaProposalError, GepaTrainSession, InstructionKeyRegistry,
    InstructionKeySpec, InstructionSource, TrainExample, UserPromptImproverReceipt,
    UserPromptImproverRequest, HEAD_ADDENDUM_FAST_FIRST_KEY, HEAD_ADDENDUM_MODALITY_KEY,
    HEAD_ADDENDUM_VERIFIER_KEY, HEAD_ROLE_CRITIQUE_KEY, HEAD_ROLE_PROPOSAL_KEY,
    HEAD_ROLE_SYNTHESIS_KEY, HEAD_ROLE_VERIFICATION_KEY, HEAD_SYSTEM_CORE_KEY,
    INSTRUCTION_KEY_PREFIX, USER_PROMPT_IMPROVER_KEY, USER_PROMPT_IMPROVER_SEED_INSTRUCTION,
};
pub use head_fitness::{FitnessCounter, HeadFitness, NodeResult, RoutingPolicy};
pub use head_invocation::{
    default_head_system_prompt, ContextMembranePrime, FakeHeadInvoker, GroundedClaim,
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadInvoker, RevisionContext,
};
pub use improvement_rate::{
    composite_point, composite_points_from_metrics, compute_improvement_rate,
    load_composite_points_from_jsonl, AxisRate, CompositePoint, ImprovementRate,
};
pub use intra_agent_loop::{
    run_fake_intra_agent_loop, run_intra_agent_loop_with_invoker, BindingLoopRound,
    FakeIntraAgentLoopInput, FakeIntraAgentLoopResult, IntraAgentLoopError,
};
pub use job::{
    idempotency_key_for, new_job_id, Job, JobReceipt, JobSubmission, Priority, TargetHead,
    LANE_CLAUDE, LANE_CODEX,
};
pub use loop_gate::{
    close_loop_if_allowed, evaluate_loop_gate, LoopClosureBudget, LoopClosureInput,
    LoopGateRejection, LoopGateState, LoopGateVerdict,
};
pub use map_artifacts::{
    compile_map_artifact, describe_map_artifact, scope_for_map_kind, stable_map_id,
    MapArtifactCompileInput, MapArtifactState, MapDeltaState, MapEntry,
};
pub use memory_contracts::{
    PrepareMemoryBank, PrepareMemoryContract, PrepareMemoryEvidence, PrepareMemoryHydrationHandle,
    PrepareMemoryPolicy, PrepareMemoryRecallPolicy, PrepareMemoryRecallPreview,
};
pub use metrics_composite::{
    composite, composite_with_safety, session_composite_point, CompositeAxes, FitnessTraitScores,
    HarnessComposite, COMPOSITE_VERSION,
};
pub use provider_head_adapter::ProviderHeadExecutionContext;
pub use replay::{compare_runs, fork_events, fork_run, replay_events, replay_run, ReplayError};
pub use scheduler::next_for_head;
pub use session_metrics::{
    compare_modes, load_jsonl_metrics, summarize_pairformer_ab, PairformerSummaryRow,
    SessionMetricsState, PAIRFORMER_MODES,
};
pub use shadow_eval::{
    evaluate_shadow, ShadowEvalInput, ShadowEvalResult, ShadowRunPair, ShadowVerdict,
};
pub use state_hash::{empty_state_hash, hash_run_state, stable_value_hash};
pub use state_machine::{apply_transition, HarnessError};
pub use toolgraph::{
    catalog_as_dicts, compile_task_toolkit, normalize_permissions, select_tools, BlockedTool,
    CompiledToolkit, ToolContract, ToolSelectionState, DEFAULT_PERMISSIONS,
};
pub use types::{
    AgentRunState, AgentStepState, EventState, GuardViolation, Payload, PolicyCheck,
    PolicyDecision, PolicyLayer, RunState, TransitionInput, TransitionResult,
};
pub use user_model::{
    user_model_hash, UserModel, UserModelNote, UserModelProjectRef, UserModelReference,
};
pub use work_graph::{
    claim_task_node, heartbeat_task_node, ClaimLease, ClaimOutcome, Millis, NodeStatus, Receipt,
    TaskNode, WorkGraph, WorkGraphError,
};
pub use work_graph_verify::{
    spawn_verify_node, submit_verify_receipt, verify_node_id, VerifyOutcome, VerifyReceipt,
    VERIFY_NODE_TYPE,
};
