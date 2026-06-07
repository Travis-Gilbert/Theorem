pub mod affordances;
pub mod agent_binding;
pub mod agent_head_registry;
pub mod alignment;
pub mod budget;
pub mod context_web;
pub mod federated_signals;
pub mod head_fitness;
pub mod head_invocation;
pub mod intra_agent_loop;
pub mod map_artifacts;
pub mod memory_contracts;
pub mod provider_head_adapter;
pub mod replay;
pub mod scheduler;
pub mod session_metrics;
pub mod state_hash;
pub mod state_machine;
pub mod toolgraph;
pub mod types;
pub mod work_graph;
pub mod work_graph_verify;

pub use affordances::{
    affordance_by_id, affordance_ids, default_affordance_registry, validate_affordance_registry,
    AffordanceContract, AffordanceReceipt,
};
pub use agent_binding::{
    apply_binding_transition, composition_hash, hash_agent_binding, ActionTierPolicy, AgentBinding,
    AgentHead, BindingBudgetScope, BindingCapabilityScope, BindingComposition, BindingError,
    BindingEventState, BindingIdentity, BindingLifecycleState, BindingMemoryScope,
    BindingTraceScope, BindingTransitionInput, BindingTransitionResult, HeadBudgetLimit,
    HeadContributionRecord, HeadCostProfile, HeadKind, HeadReliabilityProfile, HeadTransport,
    MemoryZone, MemoryZoneKind, PublishedScope, ScratchpadDocument, ScratchpadRevision, TraceTier,
};
pub use agent_head_registry::{
    AgentHeadEndpoint, AgentHeadKindSummary, AgentHeadRegistry, AgentHeadRegistryError,
    RegisteredAgentHead, ResolvedAgentHead,
};
pub use alignment::{evaluate_publication, MIN_CONSENSUS_HEADS};
pub use budget::{apply_contribution_charge, check_contribution_budget, BindingBudgetState};
pub use context_web::{
    is_generated_artifact, normalize_context_web_node_id, ContextWebAtom, ContextWebBudget,
    ContextWebCitation, ContextWebEdge, ContextWebEvaluation, ContextWebIndex, ContextWebPack,
    ContextWebPackInput, ContextWebPath, ContextWebPolicy, ContextWebSpendPlan,
    ContextWebStructuralBankResult, ContextWebTokenLedger, ContextWebValidationSummary,
    ContextWebValidatorFinding,
};
pub use federated_signals::{
    assert_no_raw_content, extract_structural_signal, observed_count_bucket,
    receive_federated_signal, success_rate_bucket, FederatedSignal, PrivacyViolation,
    StructuralSignalInput,
};
pub use head_fitness::{FitnessCounter, HeadFitness, NodeResult, RoutingPolicy};
pub use head_invocation::{
    FakeHeadInvoker, GroundedClaim, HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt,
    HeadInvocationRequest, HeadInvoker,
};
pub use intra_agent_loop::{
    run_fake_intra_agent_loop, run_intra_agent_loop_with_invoker, FakeIntraAgentLoopInput,
    FakeIntraAgentLoopResult, IntraAgentLoopError,
};
pub use map_artifacts::{
    compile_map_artifact, describe_map_artifact, scope_for_map_kind, stable_map_id,
    MapArtifactCompileInput, MapArtifactState, MapDeltaState, MapEntry,
};
pub use memory_contracts::{
    PrepareMemoryBank, PrepareMemoryContract, PrepareMemoryEvidence, PrepareMemoryHydrationHandle,
    PrepareMemoryPolicy, PrepareMemoryRecallPolicy, PrepareMemoryRecallPreview,
};
pub use provider_head_adapter::ProviderHeadExecutionContext;
pub use replay::{compare_runs, fork_events, fork_run, replay_events, replay_run, ReplayError};
pub use scheduler::next_for_head;
pub use session_metrics::{
    compare_modes, load_jsonl_metrics, summarize_pairformer_ab, PairformerSummaryRow,
    SessionMetricsState, PAIRFORMER_MODES,
};
pub use state_hash::{empty_state_hash, hash_run_state, stable_value_hash};
pub use state_machine::{apply_transition, HarnessError};
pub use toolgraph::{
    catalog_as_dicts, compile_task_toolkit, normalize_permissions, select_tools, BlockedTool,
    CompiledToolkit, ToolContract, ToolSelectionState, DEFAULT_PERMISSIONS,
};
pub use types::{
    AgentRunState, AgentStepState, EventState, GuardViolation, Payload, RunState, TransitionInput,
    TransitionResult,
};
pub use work_graph::{
    claim_task_node, heartbeat_task_node, ClaimLease, ClaimOutcome, Millis, NodeStatus, Receipt,
    TaskNode, WorkGraph, WorkGraphError,
};
pub use work_graph_verify::{
    spawn_verify_node, submit_verify_receipt, verify_node_id, VerifyOutcome, VerifyReceipt,
    VERIFY_NODE_TYPE,
};
