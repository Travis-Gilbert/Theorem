use crate::binding_store::{load_binding, persist_binding_run_result, BindingRuntimeError};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::state_hash::stable_value_hash;
use theorem_harness_core::{
    apply_binding_transition, composition_hash, run_intra_agent_loop_with_invoker, AgentBinding,
    AgentHead, AgentHeadRegistry, BindingBudgetScope, BindingComposition, BindingError,
    BindingEventState, BindingIdentity, BindingTransitionInput, Constitution,
    FakeIntraAgentLoopInput, FakeIntraAgentLoopResult, GroundedClaim, HeadCostProfile,
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadInvoker, HeadKind, HeadReliabilityProfile, HeadTransport, IntraAgentLoopError,
    ResolvedAgentHead, ScratchpadRevision, TraceTier,
};

pub const DEFAULT_BINDING_ID: &str = "agent:theorem";
pub const THEOREM_AGENT_HEADS_ENV: &str = "THEOREM_AGENT_HEADS";
pub const THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV: &str = "THEOREM_COMPOSED_AGENT_BUDGET_UNITS";
pub const DEFAULT_COMPOSED_AGENT_BUDGET_UNITS: f64 = 5_000.0;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ComposedAgentRunResult {
    pub binding_id: String,
    pub run_id: String,
    pub task: String,
    pub published_claims: Vec<GroundedClaim>,
    pub consensus_head_set: Vec<String>,
    pub alignment_verdict: Value,
    pub events: Vec<BindingEventState>,
    pub scratchpad_revisions: Vec<ScratchpadRevision>,
    pub invocation_receipts: Vec<HeadInvocationReceipt>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComposedAgentRuntimeError {
    BindingStore(BindingRuntimeError),
    Binding(BindingError),
    Loop(IntraAgentLoopError),
    InvalidInput(String),
}

impl fmt::Display for ComposedAgentRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindingStore(error) => write!(f, "{error}"),
            Self::Binding(error) => write!(f, "{error}"),
            Self::Loop(error) => write!(f, "{error}"),
            Self::InvalidInput(error) => write!(f, "{error}"),
        }
    }
}

impl Error for ComposedAgentRuntimeError {}

impl From<BindingRuntimeError> for ComposedAgentRuntimeError {
    fn from(value: BindingRuntimeError) -> Self {
        Self::BindingStore(value)
    }
}

impl From<BindingError> for ComposedAgentRuntimeError {
    fn from(value: BindingError) -> Self {
        Self::Binding(value)
    }
}

impl From<IntraAgentLoopError> for ComposedAgentRuntimeError {
    fn from(value: IntraAgentLoopError) -> Self {
        Self::Loop(value)
    }
}

pub type ComposedAgentRuntimeResult<T> = Result<T, ComposedAgentRuntimeError>;

pub fn run_composed_agent<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let claims = vec![GroundedClaim::new(
        task.trim(),
        format!("binding:{binding_id}:task"),
    )];
    run_composed_agent_with_claims(store, binding_id, task, claims, invoker)
}

pub fn run_composed_agent_with_claims<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    claims: Vec<GroundedClaim>,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let binding_id = normalize_binding_id(binding_id);
    let task = task.trim();
    if task.is_empty() {
        return Err(ComposedAgentRuntimeError::InvalidInput(
            "composed_agent_run requires task".to_string(),
        ));
    }
    let binding = match load_binding(store, &binding_id)? {
        Some(binding) => binding,
        None => default_theorem_binding(&binding_id)?,
    };
    let mut input = FakeIntraAgentLoopInput::new(task, claims);
    input.budget_units = composed_agent_budget_units()?;
    input.max_parallel_heads = input.max_parallel_heads.max(binding.reasoning_core_ids().len());
    let result = run_intra_agent_loop_with_invoker(binding, input, invoker)?;
    persist_binding_run_result(store, &result.binding, &result.events)?;
    let policy_event = result
        .events
        .iter()
        .find(|event| event.event_type == "POLICY.CHECKED");
    let alignment_verdict = policy_event
        .map(|event| Value::Object(event.payload.clone()))
        .unwrap_or_else(|| json!({ "allowed": false, "reason": "policy_event_missing" }));
    let published_claims = if verdict_allowed(&alignment_verdict) {
        result
            .invocation_receipts
            .last()
            .map(|receipt| receipt.claims.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(ComposedAgentRunResult {
        binding_id: binding_id.clone(),
        run_id: result.binding.lifecycle.run_id.clone(),
        task: task.to_string(),
        published_claims,
        consensus_head_set: result.binding.trace_scope.synthesis_heads.clone(),
        alignment_verdict,
        events: result.events,
        scratchpad_revisions: result.scratchpad_revisions,
        invocation_receipts: result.invocation_receipts,
    })
}

pub fn run_configured_composed_agent<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let claims = vec![GroundedClaim::new(
        task.trim(),
        format!("binding:{binding_id}:task"),
    )];
    run_configured_composed_agent_with_claims(store, binding_id, task, claims, invoker)
}

pub fn run_configured_composed_agent_with_claims<S: GraphStore>(
    store: &mut S,
    binding_id: &str,
    task: &str,
    claims: Vec<GroundedClaim>,
    invoker: &impl HeadInvoker,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let binding_id = normalize_binding_id(binding_id);
    let task = task.trim();
    if task.is_empty() {
        return Err(ComposedAgentRuntimeError::InvalidInput(
            "composed_agent_run requires task".to_string(),
        ));
    }
    let binding = match load_binding(store, &binding_id)? {
        Some(binding) => binding,
        None => default_theorem_binding(&binding_id)?,
    };
    let binding = runtime_candidate_binding(binding)?;
    let binding = binding_with_available_runtime_heads(binding)?;
    let mut input = FakeIntraAgentLoopInput::new(task, claims);
    input.budget_units = composed_agent_budget_units()?;
    input.max_parallel_heads = input.max_parallel_heads.max(binding.reasoning_core_ids().len());

    let registry = AgentHeadRegistry::from_binding(&binding)
        .map_err(|error| ComposedAgentRuntimeError::Loop(IntraAgentLoopError::Registry(error)))?;
    let heads = reasoning_heads(&registry);
    let result = if heads.len() == 1 {
        run_single_head_agent(binding, input, heads[0].clone(), invoker)?
    } else {
        run_intra_agent_loop_with_invoker(binding, input, invoker)?
    };
    persist_binding_run_result(store, &result.binding, &result.events)?;
    composed_result_from_loop_result(&binding_id, task, result)
}

pub fn default_theorem_binding(binding_id: &str) -> Result<AgentBinding, BindingError> {
    let configured_heads = configured_agent_heads_from_env();
    let (active_head_set, heads) = if configured_heads.is_empty() {
        default_candidate_heads()
    } else {
        (
            configured_heads
                .iter()
                .map(|head| head.head_id.clone())
                .collect(),
            configured_heads,
        )
    };
    let max_parallel_heads = active_head_set.len().max(2);
    let mut binding = AgentBinding::new(
        BindingIdentity {
            agent_id: "theorem".to_string(),
            owner_id: "travis".to_string(),
            agent_name: "Theorem".to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set,
        },
        BindingComposition { heads },
        BindingBudgetScope::new("theorem", 32_000.0, max_parallel_heads),
    )?;
    binding.lifecycle.run_id = normalize_binding_id(binding_id);
    Ok(binding)
}

fn default_candidate_heads() -> (Vec<String>, Vec<AgentHead>) {
    let heads = [
        "deepseek", "mistral", "minimax", "openai", "zhipu", "ai21", "gemma",
    ]
    .into_iter()
    .map(env_configured_head)
    .collect::<Vec<_>>();
    let active_head_set = heads.iter().map(|head| head.head_id.clone()).collect();
    (active_head_set, heads)
}

fn runtime_candidate_binding(
    mut binding: AgentBinding,
) -> ComposedAgentRuntimeResult<AgentBinding> {
    let candidates = {
        let configured_heads = configured_agent_heads_from_env();
        if configured_heads.is_empty() {
            default_candidate_heads().1
        } else {
            configured_heads
        }
    };
    if candidates.is_empty() {
        return Ok(binding);
    }

    let active_head_set = candidates
        .iter()
        .map(|head| head.head_id.clone())
        .collect::<Vec<_>>();
    for candidate in candidates {
        match binding
            .composition
            .heads
            .iter_mut()
            .find(|head| head.head_id == candidate.head_id)
        {
            Some(existing) => *existing = candidate,
            None => binding.composition.heads.push(candidate),
        }
    }
    binding.identity.active_head_set = sorted_unique(active_head_set);
    binding.identity.composition_hash = composition_hash(&binding);
    Ok(binding)
}

fn binding_with_available_runtime_heads(
    mut binding: AgentBinding,
) -> ComposedAgentRuntimeResult<AgentBinding> {
    let registry = AgentHeadRegistry::from_binding(&binding)
        .map_err(|error| ComposedAgentRuntimeError::Loop(IntraAgentLoopError::Registry(error)))?;
    let available = reasoning_heads(&registry)
        .into_iter()
        .filter(head_runtime_configured)
        .map(|head| head.head_id)
        .collect::<Vec<_>>();
    if available.is_empty() {
        return Err(ComposedAgentRuntimeError::InvalidInput(
            "composed_agent_run has no runtime-configured provider heads; set THEOREM_AGENT_HEADS and at least one matching *_API_KEY".to_string(),
        ));
    }
    binding.identity.active_head_set = sorted_unique(available);
    binding.identity.composition_hash = composition_hash(&binding);
    Ok(binding)
}

fn reasoning_heads(registry: &AgentHeadRegistry) -> Vec<ResolvedAgentHead> {
    registry
        .active_resolved_heads()
        .into_iter()
        .filter(|head| head.kind != HeadKind::SkillPlugin)
        .collect()
}

fn head_runtime_configured(head: &ResolvedAgentHead) -> bool {
    match head.endpoint.transport {
        HeadTransport::Api | HeadTransport::Mcp | HeadTransport::Hosted => {
            credential_ref_available(&head.credential_ref)
        }
        HeadTransport::Local => local_head_runtime_configured(&head.credential_ref),
    }
}

fn credential_ref_available(credential_ref: &str) -> bool {
    let credential_ref = credential_ref.trim();
    if let Some(env_name) = credential_ref.strip_prefix("env:") {
        return std::env::var(env_name.trim())
            .ok()
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false);
    }
    credential_ref.starts_with("static:") || credential_ref.starts_with("static-token:")
}

fn local_head_runtime_configured(credential_ref: &str) -> bool {
    let credential_ref = credential_ref.trim();
    credential_ref.is_empty()
        || matches!(
            credential_ref.to_ascii_lowercase().as_str(),
            "none" | "none:" | "disabled"
        )
        || credential_ref_available(credential_ref)
}

fn sorted_unique(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

fn run_single_head_agent<I: HeadInvoker>(
    mut binding: AgentBinding,
    input: FakeIntraAgentLoopInput,
    head: ResolvedAgentHead,
    invoker: &I,
) -> ComposedAgentRuntimeResult<FakeIntraAgentLoopResult> {
    let mut events = Vec::new();
    let mut revisions = Vec::new();
    let mut invocation_receipts = Vec::new();

    binding = apply_step(
        binding,
        "BINDING.RESOLVED",
        object_payload(json!({
            "binding_id": "agent:theorem",
            "composition_hash": "computed-by-kernel"
        })),
        &input.started_at,
        &mut events,
    )?;
    let registry = AgentHeadRegistry::from_binding(&binding)
        .map_err(|error| ComposedAgentRuntimeError::Loop(IntraAgentLoopError::Registry(error)))?;
    binding = apply_step(
        binding,
        "HEADS.PROBED",
        registry.heads_probed_payload(),
        &input.started_at,
        &mut events,
    )?;

    let scope_id = binding.working_memory_scope.scope_id.clone();
    let scratchpad_id = binding.working_memory_scope.scratchpad.document_id.clone();
    binding = apply_step(
        binding,
        "MEMORY_SCOPE.MOUNTED",
        object_payload(json!({
            "scope_id": scope_id,
            "scratchpad_id": scratchpad_id
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "CHARTER.COMPILED",
        object_payload(json!({
            "charter_hash": input.charter_hash,
            "stance": input.stance
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "CAPABILITIES.SELECTED",
        object_payload(json!({
            "capability_scope_hash": input.capability_scope_hash,
            "visible_tools": input.visible_tools,
            "callable_tools": input.callable_tools
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "BUDGET.ALLOCATED",
        object_payload(json!({
            "budget_units": input.budget_units,
            "max_parallel_heads": 1
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "RUN.STARTED",
        object_payload(json!({
            "task": input.task,
            "started_at": input.started_at
        })),
        &input.started_at,
        &mut events,
    )?;

    let opened = append_revision(
        &mut binding,
        &head.head_id,
        "single-head work opened",
        object_payload(json!({
            "task": input.task,
            "kind": "orientation",
            "mode": "single_head"
        })),
        &input.started_at,
    )?;
    let opened_revision_id = opened.revision_id.clone();
    revisions.push(opened);
    binding = apply_step(
        binding,
        "PRIVATE_WORK.OPENED",
        object_payload(json!({
            "scratchpad_revision_id": opened_revision_id
        })),
        &input.started_at,
        &mut events,
    )?;

    let constitution = Constitution::for_binding(&binding, &input.task, &input.claims);
    let policy_decision =
        constitution.head_turn_decision(&binding, &head.head_id, HeadInvocationKind::Proposal);
    let receipt = invoker
        .invoke(
            HeadInvocationRequest::new_with_context(
                head.clone(),
                HeadInvocationKind::Proposal,
                input.task.clone(),
                binding.working_memory_scope.scratchpad.version,
                revisions
                    .iter()
                    .map(|revision| revision.revision_id.clone())
                    .collect(),
                Vec::new(),
                input.claims.clone(),
                input.started_at.clone(),
            )
            .with_policy_decision(policy_decision),
        )
        .map_err(|error| ComposedAgentRuntimeError::Loop(IntraAgentLoopError::Invocation(error)))?;
    let proposal = append_invocation_revision(&mut binding, &receipt)?;
    binding = contribute_from_receipt(binding, &receipt, &input.started_at, &mut events)?;
    revisions.push(proposal);
    invocation_receipts.push(receipt.clone());

    binding = apply_step(
        binding,
        "DRAFTS.SYNTHESIZED",
        object_payload(json!({
            "synthesis_id": format!("synthesis:single:{}", head.head_id),
            "contributing_heads": [head.head_id.clone()],
            "mode": "single_head"
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "PUBLICATION.PROPOSED",
        object_payload(json!({
            "publication_id": input.publication_id,
            "draft_hash": input.draft_hash,
            "mode": "single_head"
        })),
        &input.started_at,
        &mut events,
    )?;
    let policy_decision = constitution.publication_decision(&binding);
    binding = apply_step(
        binding,
        "POLICY.CHECKED",
        object_payload(json!({
            "policy_receipt_id": policy_decision.decision_id,
            "allowed": policy_decision.allowed,
            "policy_decision": policy_decision,
            "claims": receipt.claims,
            "single_head_mode": true
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "PUBLISHED_TO_SUBSTRATE",
        object_payload(json!({
            "publication_id": input.publication_id,
            "substrate_receipt_id": input.substrate_receipt_id
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "OUTCOME.RECORDED",
        object_payload(json!({
            "outcome_id": input.outcome_id,
            "accepted": true,
            "summary": "single-head run published grounded output"
        })),
        &input.started_at,
        &mut events,
    )?;
    binding = apply_step(
        binding,
        "RUN.CLOSED",
        object_payload(json!({
            "summary": "single-head run closed",
            "closed_by": input.closed_by
        })),
        &input.started_at,
        &mut events,
    )?;

    Ok(FakeIntraAgentLoopResult {
        binding,
        events,
        scratchpad_revisions: revisions,
        invocation_receipts,
        primary_head_id: head.head_id.clone(),
        critic_head_id: head.head_id.clone(),
        synthesis_head_id: head.head_id,
    })
}

fn composed_result_from_loop_result(
    binding_id: &str,
    task: &str,
    result: FakeIntraAgentLoopResult,
) -> ComposedAgentRuntimeResult<ComposedAgentRunResult> {
    let policy_event = result
        .events
        .iter()
        .find(|event| event.event_type == "POLICY.CHECKED");
    let alignment_verdict = policy_event
        .map(|event| Value::Object(event.payload.clone()))
        .unwrap_or_else(|| json!({ "allowed": false, "reason": "policy_event_missing" }));
    let published_claims = if verdict_allowed(&alignment_verdict) {
        result
            .invocation_receipts
            .last()
            .map(|receipt| receipt.claims.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Ok(ComposedAgentRunResult {
        binding_id: binding_id.to_string(),
        run_id: result.binding.lifecycle.run_id.clone(),
        task: task.to_string(),
        published_claims,
        consensus_head_set: result.binding.trace_scope.synthesis_heads.clone(),
        alignment_verdict,
        events: result.events,
        scratchpad_revisions: result.scratchpad_revisions,
        invocation_receipts: result.invocation_receipts,
    })
}

fn append_invocation_revision(
    binding: &mut AgentBinding,
    receipt: &HeadInvocationReceipt,
) -> Result<ScratchpadRevision, ComposedAgentRuntimeError> {
    binding
        .append_scratchpad_revision(
            &receipt.head_id,
            &receipt.output_summary,
            receipt.content_hash.clone(),
            receipt.payload.clone(),
            &receipt.created_at,
        )
        .map_err(ComposedAgentRuntimeError::Binding)
}

fn append_revision(
    binding: &mut AgentBinding,
    head_id: &str,
    summary: &str,
    payload: serde_json::Map<String, Value>,
    created_at: &str,
) -> Result<ScratchpadRevision, ComposedAgentRuntimeError> {
    let content_hash = stable_value_hash(&Value::Object(payload.clone()));
    binding
        .append_scratchpad_revision(head_id, summary, content_hash, payload, created_at)
        .map_err(ComposedAgentRuntimeError::Binding)
}

fn contribute_from_receipt(
    binding: AgentBinding,
    receipt: &HeadInvocationReceipt,
    created_at: &str,
    events: &mut Vec<BindingEventState>,
) -> ComposedAgentRuntimeResult<AgentBinding> {
    apply_step(
        binding,
        "HEADS.CONTRIBUTE",
        object_payload(json!({
            "head_id": receipt.head_id,
            "contribution_id": receipt.contribution_id(),
            "contribution_kind": receipt.contribution_kind(),
            "cost_units": receipt.cost_units,
            "receipt_hash": receipt.receipt_hash,
            "weight": 1.0
        })),
        created_at,
        events,
    )
}

fn apply_step(
    binding: AgentBinding,
    event_type: &str,
    payload: serde_json::Map<String, Value>,
    created_at: &str,
    events: &mut Vec<BindingEventState>,
) -> ComposedAgentRuntimeResult<AgentBinding> {
    let mut transition = BindingTransitionInput::new(event_type, payload).at(created_at);
    transition.actor = "configured-composed-agent".to_string();
    let result = apply_binding_transition(binding, transition)?;
    events.push(result.event);
    Ok(result.binding)
}

fn object_payload(value: Value) -> serde_json::Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => serde_json::Map::new(),
    }
}

fn configured_agent_heads_from_env() -> Vec<AgentHead> {
    std::env::var(THEOREM_AGENT_HEADS_ENV)
        .ok()
        .map(|value| {
            split_csv(&value)
                .into_iter()
                .map(|head_id| env_configured_head(&head_id))
                .collect()
        })
        .unwrap_or_default()
}

fn env_configured_head(head_id: &str) -> AgentHead {
    let head_id = head_id.trim();
    let head_slug = env_slug(head_id);
    let provider = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_PROVIDER"),
        format!("{head_slug}_PROVIDER"),
    ])
    .unwrap_or_else(|| normalize_provider(head_id));
    let provider_slug = env_slug(&provider);
    let model = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_MODEL"),
        format!("{head_slug}_MODEL"),
        format!("{provider_slug}_MODEL"),
    ])
    .unwrap_or_else(|| default_model_for_provider(&provider));
    let credential_ref = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_CREDENTIAL_REF"),
        format!("THEOREM_AGENT_HEAD_{head_slug}_CREDENTIAL"),
    ])
    .unwrap_or_else(|| format!("env:{provider_slug}_API_KEY"));
    let transport = env_first([
        format!("THEOREM_AGENT_HEAD_{head_slug}_TRANSPORT"),
        format!("{head_slug}_TRANSPORT"),
    ])
    .as_deref()
    .and_then(parse_transport)
    .unwrap_or(HeadTransport::Api);
    head(head_id, &provider, model, &credential_ref, transport)
}

fn head(
    head_id: &str,
    provider: &str,
    model: impl Into<String>,
    credential_ref: &str,
    transport: HeadTransport,
) -> AgentHead {
    AgentHead {
        head_id: head_id.to_string(),
        display_name: head_id.to_string(),
        provider: provider.to_string(),
        model: model.into(),
        credential_ref: credential_ref.to_string(),
        transport,
        kind: HeadKind::ReasoningCore,
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}

fn env_first<const N: usize>(names: [String; N]) -> Option<String> {
    names.into_iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect()
}

fn env_slug(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

fn normalize_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "openapi" => "openai".to_string(),
        other => other.to_string(),
    }
}

fn default_model_for_provider(provider: &str) -> String {
    match normalize_provider(provider).as_str() {
        "anthropic" | "claude" => "claude-3-5-sonnet-latest".to_string(),
        "deepseek" => "deepseek-v4-pro".to_string(),
        "gemma" => "gemma3:latest".to_string(),
        "minimax" => "MiniMax-M3".to_string(),
        "mistral" => "mistral-large-latest".to_string(),
        "openai" => "gpt-4.1-mini".to_string(),
        "zhipu" => "glm-4-plus".to_string(),
        "ai21" => "jamba-large".to_string(),
        other => other.to_string(),
    }
}

fn parse_transport(value: &str) -> Option<HeadTransport> {
    match value.trim().to_ascii_lowercase().as_str() {
        "api" => Some(HeadTransport::Api),
        "mcp" => Some(HeadTransport::Mcp),
        "local" => Some(HeadTransport::Local),
        "hosted" => Some(HeadTransport::Hosted),
        _ => None,
    }
}

fn normalize_binding_id(binding_id: &str) -> String {
    let trimmed = binding_id.trim();
    if trimmed.is_empty() {
        DEFAULT_BINDING_ID.to_string()
    } else {
        trimmed.to_string()
    }
}

fn composed_agent_budget_units() -> ComposedAgentRuntimeResult<f64> {
    let Ok(raw) = std::env::var(THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV) else {
        return Ok(DEFAULT_COMPOSED_AGENT_BUDGET_UNITS);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(DEFAULT_COMPOSED_AGENT_BUDGET_UNITS);
    }
    let budget = trimmed.parse::<f64>().map_err(|_| {
        ComposedAgentRuntimeError::InvalidInput(format!(
            "{THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV} must be a positive number"
        ))
    })?;
    if !budget.is_finite() || budget <= 0.0 {
        return Err(ComposedAgentRuntimeError::InvalidInput(format!(
            "{THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV} must be a positive number"
        )));
    }
    Ok(budget)
}

fn verdict_allowed(verdict: &Value) -> bool {
    verdict
        .get("allowed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

impl From<HeadInvocationError> for ComposedAgentRuntimeError {
    fn from(value: HeadInvocationError) -> Self {
        Self::Loop(IntraAgentLoopError::Invocation(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use std::sync::Mutex;
    use theorem_harness_core::FakeHeadInvoker;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn composed_agent_run_persists_binding_events_and_scratchpad() {
        let _env = ScopedEnv::new([]);
        let mut store = InMemoryGraphStore::new();
        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:test",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(result.run_id, "agent:test");
        assert_eq!(result.consensus_head_set, vec!["ai21", "deepseek"]);
        assert_eq!(result.events.last().unwrap().event_type, "RUN.CLOSED");
        assert!(load_binding(&store, "agent:test").unwrap().is_some());
        assert_eq!(
            crate::load_binding_events(&store, "agent:test")
                .unwrap()
                .len(),
            result.events.len()
        );
    }

    #[test]
    fn composed_agent_run_uses_real_provider_sized_default_budget() {
        let _env = ScopedEnv::new([]);
        let mut store = InMemoryGraphStore::new();
        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:budget-default",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(
            allocated_budget_units(&result),
            DEFAULT_COMPOSED_AGENT_BUDGET_UNITS
        );
    }

    #[test]
    fn composed_agent_run_budget_can_be_overridden_by_env() {
        let _env = ScopedEnv::new([(THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV, "7500")]);
        let mut store = InMemoryGraphStore::new();
        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:budget-env",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(allocated_budget_units(&result), 7_500.0);
    }

    #[test]
    fn composed_agent_run_rejects_invalid_budget_env() {
        let _env = ScopedEnv::new([(THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV, "nope")]);
        let mut store = InMemoryGraphStore::new();
        let error = run_composed_agent_with_claims(
            &mut store,
            "agent:budget-invalid",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap_err();

        assert!(matches!(error, ComposedAgentRuntimeError::InvalidInput(_)));
    }

    #[test]
    fn default_binding_uses_env_configured_api_heads() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "mistral,openai,minimax,deepseek"),
            ("MISTRAL_MODEL", "mistral-small-latest"),
            ("OPENAI_MODEL", "gpt-4.1-mini"),
            ("MINIMAX_MODEL", "MiniMax-M3"),
            ("DEEPSEEK_MODEL", "deepseek-v4-pro"),
        ]);

        let binding = default_theorem_binding("agent:env").unwrap();

        assert_eq!(
            binding.identity.active_head_set,
            vec!["deepseek", "minimax", "mistral", "openai"]
        );
        let mistral = binding.head("mistral").unwrap();
        assert_eq!(mistral.provider, "mistral");
        assert_eq!(mistral.model, "mistral-small-latest");
        assert_eq!(mistral.credential_ref, "env:MISTRAL_API_KEY");
        assert_eq!(mistral.transport, HeadTransport::Api);
        let openai = binding.head("openai").unwrap();
        assert_eq!(openai.provider, "openai");
        assert_eq!(openai.credential_ref, "env:OPENAI_API_KEY");
        let minimax = binding.head("minimax").unwrap();
        assert_eq!(minimax.provider, "minimax");
        assert_eq!(minimax.model, "MiniMax-M3");
        assert_eq!(minimax.credential_ref, "env:MINIMAX_API_KEY");
        let deepseek = binding.head("deepseek").unwrap();
        assert_eq!(deepseek.model, "deepseek-v4-pro");
        assert_eq!(deepseek.transport, HeadTransport::Api);
    }

    #[test]
    fn default_binding_excludes_claude_api_head() {
        let _env = ScopedEnv::new([]);

        let binding = default_theorem_binding("agent:default").unwrap();

        assert!(binding.head("claude").is_none());
        assert!(binding.head("deepseek").is_some());
        assert!(binding
            .identity
            .active_head_set
            .contains(&"deepseek".to_string()));
    }

    #[test]
    fn configured_run_uses_single_available_provider_key() {
        let _env = ScopedEnv::new([("DEEPSEEK_API_KEY", "deepseek-test-secret")]);
        let mut store = InMemoryGraphStore::new();

        let result = run_configured_composed_agent_with_claims(
            &mut store,
            "agent:single-key",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(result.consensus_head_set, vec!["deepseek"]);
        assert_eq!(result.invocation_receipts.len(), 1);
        assert_eq!(result.invocation_receipts[0].head_id, "deepseek");
        assert_eq!(
            result
                .alignment_verdict
                .get("single_head_mode")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(!result.published_claims.is_empty());
        assert_eq!(result.events.last().unwrap().event_type, "RUN.CLOSED");
    }

    #[test]
    fn configured_run_combines_all_available_provider_keys() {
        let _env = ScopedEnv::new([
            ("DEEPSEEK_API_KEY", "deepseek-test-secret"),
            ("MISTRAL_API_KEY", "mistral-test-secret"),
        ]);
        let mut store = InMemoryGraphStore::new();

        let result = run_configured_composed_agent_with_claims(
            &mut store,
            "agent:two-keys",
            "publish",
            vec![GroundedClaim::new("grounded", "source:1")],
            &FakeHeadInvoker::default(),
        )
        .unwrap();

        assert_eq!(result.consensus_head_set, vec!["deepseek", "mistral"]);
        assert_eq!(result.invocation_receipts.len(), 3);
        assert_eq!(result.invocation_receipts[0].head_id, "deepseek");
        assert_eq!(result.invocation_receipts[1].head_id, "mistral");
        assert_eq!(result.invocation_receipts[2].head_id, "deepseek");
        assert_eq!(
            result
                .alignment_verdict
                .get("single_head_mode")
                .and_then(Value::as_bool),
            None
        );
    }

    #[test]
    fn env_configured_heads_support_openapi_alias_and_transport_override() {
        let _env = ScopedEnv::new([
            (THEOREM_AGENT_HEADS_ENV, "openapi,deepseek"),
            ("THEOREM_AGENT_HEAD_OPENAPI_MODEL", "gpt-4.1-mini"),
            ("THEOREM_AGENT_HEAD_DEEPSEEK_TRANSPORT", "mcp"),
        ]);

        let binding = default_theorem_binding("agent:env-alias").unwrap();

        let openapi = binding.head("openapi").unwrap();
        assert_eq!(openapi.provider, "openai");
        assert_eq!(openapi.model, "gpt-4.1-mini");
        assert_eq!(openapi.credential_ref, "env:OPENAI_API_KEY");
        let deepseek = binding.head("deepseek").unwrap();
        assert_eq!(deepseek.transport, HeadTransport::Mcp);
    }

    #[test]
    #[ignore = "requires THEOREM_LIVE_PROVIDER_TEST=1 and real provider keys"]
    fn live_provider_invoker_runs_three_head_binding_when_enabled() {
        if std::env::var("THEOREM_LIVE_PROVIDER_TEST").ok().as_deref() != Some("1") {
            eprintln!("set THEOREM_LIVE_PROVIDER_TEST=1 with live provider keys to run");
            return;
        }
        let configured_heads = std::env::var(THEOREM_AGENT_HEADS_ENV)
            .expect("set THEOREM_AGENT_HEADS=deepseek,mistral,minimax");
        assert!(
            configured_heads.contains("deepseek")
                && configured_heads.contains("mistral")
                && configured_heads.contains("minimax"),
            "THEOREM_AGENT_HEADS must include deepseek,mistral,minimax"
        );
        let mut store = InMemoryGraphStore::new();
        let invoker = crate::ProviderHeadInvoker::from_env().unwrap();

        let result = run_composed_agent_with_claims(
            &mut store,
            "agent:live-provider-test",
            "Return one grounded claim that this live provider test ran.",
            vec![GroundedClaim::new("live provider smoke input", "test:live")],
            &invoker,
        )
        .unwrap();

        assert!(result.invocation_receipts.len() >= 3);
        assert!(result
            .events
            .iter()
            .any(|event| event.event_type == "POLICY.CHECKED"));
        if verdict_allowed(&result.alignment_verdict) {
            assert!(!result.published_claims.is_empty());
        } else {
            assert!(result.published_claims.is_empty());
        }
    }

    struct ScopedEnv {
        saved: Vec<(String, Option<String>)>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        fn new<const N: usize>(pairs: [(&'static str, &'static str); N]) -> Self {
            let guard = ENV_LOCK
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let mut names = vec![
                THEOREM_AGENT_HEADS_ENV.to_string(),
                THEOREM_COMPOSED_AGENT_BUDGET_UNITS_ENV.to_string(),
                "MISTRAL_MODEL".to_string(),
                "OPENAI_MODEL".to_string(),
                "MINIMAX_MODEL".to_string(),
                "DEEPSEEK_MODEL".to_string(),
                "AI21_API_KEY".to_string(),
                "ANTHROPIC_API_KEY".to_string(),
                "CLAUDE_API_KEY".to_string(),
                "DEEPSEEK_API_KEY".to_string(),
                "GEMMA_API_KEY".to_string(),
                "MINIMAX_API_KEY".to_string(),
                "MISTRAL_API_KEY".to_string(),
                "OPENAI_API_KEY".to_string(),
                "ZHIPU_API_KEY".to_string(),
                "THEOREM_AGENT_HEAD_OPENAPI_MODEL".to_string(),
                "THEOREM_AGENT_HEAD_DEEPSEEK_TRANSPORT".to_string(),
            ];
            for (name, _) in pairs {
                names.push(name.to_string());
            }
            names.sort();
            names.dedup();
            let saved = names
                .into_iter()
                .map(|name| {
                    let value = std::env::var(&name).ok();
                    std::env::remove_var(&name);
                    (name, value)
                })
                .collect::<Vec<_>>();
            for (name, value) in pairs {
                std::env::set_var(name, value);
            }
            Self {
                saved,
                _guard: guard,
            }
        }
    }

    fn allocated_budget_units(result: &ComposedAgentRunResult) -> f64 {
        result
            .events
            .iter()
            .find(|event| event.event_type == "BUDGET.ALLOCATED")
            .and_then(|event| event.payload.get("budget_units"))
            .and_then(Value::as_f64)
            .expect("BUDGET.ALLOCATED carries budget_units")
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}
