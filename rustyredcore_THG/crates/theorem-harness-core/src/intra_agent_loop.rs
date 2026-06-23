//! Deterministic fake-head intra-agent loop scaffold.
//!
//! This module exercises the composed-agent lifecycle without provider calls.
//! It is intentionally a coordinator over the existing binding kernel: the
//! registry chooses fake resolved heads, scratchpad revisions record the
//! proposal/critique/synthesis work, and `apply_binding_transition` remains the
//! only place that enforces budget, consensus, action-tier, and grounding rules.

use crate::agent_binding::{
    apply_binding_transition, AgentBinding, BindingError, BindingEventState,
    BindingTransitionInput, BindingTransitionResult, HeadKind, ScratchpadRevision,
};
use crate::agent_head_registry::{AgentHeadRegistry, AgentHeadRegistryError, ResolvedAgentHead};
use crate::constitution::Constitution;
use crate::head_invocation::{
    FakeHeadInvoker, GroundedClaim, HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt,
    HeadInvocationRequest, HeadInvoker, RevisionContext,
};
use crate::state_hash::stable_value_hash;
use crate::types::Payload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum IntraAgentLoopError {
    Binding(BindingError),
    Registry(AgentHeadRegistryError),
    Invocation(HeadInvocationError),
    NotEnoughReasoningHeads { available: usize },
}

impl fmt::Display for IntraAgentLoopError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Binding(error) => write!(f, "{error}"),
            Self::Registry(error) => write!(f, "{error}"),
            Self::Invocation(error) => write!(f, "{error}"),
            Self::NotEnoughReasoningHeads { available } => write!(
                f,
                "fake intra-agent loop requires at least two active non-plugin heads; found {available}"
            ),
        }
    }
}

impl Error for IntraAgentLoopError {}

impl From<BindingError> for IntraAgentLoopError {
    fn from(value: BindingError) -> Self {
        Self::Binding(value)
    }
}

impl From<AgentHeadRegistryError> for IntraAgentLoopError {
    fn from(value: AgentHeadRegistryError) -> Self {
        Self::Registry(value)
    }
}

impl From<HeadInvocationError> for IntraAgentLoopError {
    fn from(value: HeadInvocationError) -> Self {
        Self::Invocation(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FakeIntraAgentLoopInput {
    pub task: String,
    pub charter_hash: String,
    pub stance: String,
    pub capability_scope_hash: String,
    #[serde(default)]
    pub visible_tools: Vec<String>,
    #[serde(default)]
    pub callable_tools: Vec<String>,
    pub budget_units: f64,
    pub max_parallel_heads: usize,
    pub publication_id: String,
    pub draft_hash: String,
    pub substrate_receipt_id: String,
    pub outcome_id: String,
    #[serde(default)]
    pub claims: Vec<GroundedClaim>,
    pub started_at: String,
    pub closed_by: String,
}

impl FakeIntraAgentLoopInput {
    pub fn new(task: impl Into<String>, claims: Vec<GroundedClaim>) -> Self {
        Self {
            task: task.into(),
            charter_hash: "charter:fake-loop".to_string(),
            stance: "grounded fake-head composed-agent loop".to_string(),
            capability_scope_hash: "capability:fake-loop".to_string(),
            visible_tools: vec!["datalog.derive".to_string()],
            callable_tools: vec!["datalog.derive".to_string()],
            budget_units: 6.0,
            max_parallel_heads: 2,
            publication_id: "publication:fake-loop".to_string(),
            draft_hash: "draft:fake-loop".to_string(),
            substrate_receipt_id: "substrate:fake-loop".to_string(),
            outcome_id: "outcome:fake-loop".to_string(),
            claims,
            started_at: "2026-06-02T00:00:00Z".to_string(),
            closed_by: "fake-loop".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FakeIntraAgentLoopResult {
    pub binding: AgentBinding,
    pub events: Vec<BindingEventState>,
    pub scratchpad_revisions: Vec<ScratchpadRevision>,
    pub invocation_receipts: Vec<HeadInvocationReceipt>,
    pub primary_head_id: String,
    pub critic_head_id: String,
    pub synthesis_head_id: String,
}

pub fn run_fake_intra_agent_loop(
    binding: AgentBinding,
    input: FakeIntraAgentLoopInput,
) -> Result<FakeIntraAgentLoopResult, IntraAgentLoopError> {
    run_intra_agent_loop_with_invoker(binding, input, &FakeHeadInvoker::default())
}

pub fn run_intra_agent_loop_with_invoker<I: HeadInvoker>(
    binding: AgentBinding,
    input: FakeIntraAgentLoopInput,
    invoker: &I,
) -> Result<FakeIntraAgentLoopResult, IntraAgentLoopError> {
    let registry = AgentHeadRegistry::from_binding(&binding)?;
    let heads = reasoning_heads(&registry);
    if heads.len() < 2 {
        return Err(IntraAgentLoopError::NotEnoughReasoningHeads {
            available: heads.len(),
        });
    }

    let primary = heads[0].clone();
    let critic = heads[1].clone();
    let synthesis = heads.get(2).cloned().unwrap_or_else(|| primary.clone());
    let mut binding = binding;
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
    )?
    .binding;

    binding = apply_step(
        binding,
        "HEADS.PROBED",
        registry.heads_probed_payload(),
        &input.started_at,
        &mut events,
    )?
    .binding;

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
    )?
    .binding;

    binding = apply_step(
        binding,
        "CHARTER.COMPILED",
        object_payload(json!({
            "charter_hash": input.charter_hash,
            "stance": input.stance
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

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
    )?
    .binding;

    binding = apply_step(
        binding,
        "BUDGET.ALLOCATED",
        object_payload(json!({
            "budget_units": input.budget_units,
            "max_parallel_heads": input.max_parallel_heads
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "RUN.STARTED",
        object_payload(json!({
            "task": input.task,
            "started_at": input.started_at
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    let opened = append_revision(
        &mut binding,
        &primary.head_id,
        "private work opened",
        object_payload(json!({
            "task": input.task,
            "kind": "orientation"
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
    )?
    .binding;

    let constitution = Constitution::for_binding(&binding, &input.task, &input.claims);

    let proposal_receipt = invoke_head(
        invoker,
        primary.clone(),
        HeadInvocationKind::Proposal,
        &binding,
        &input,
        &revisions,
        &constitution,
    )?;
    let proposal = append_invocation_revision(&mut binding, &proposal_receipt)?;
    binding = contribute_from_receipt(binding, &proposal_receipt, &input.started_at, &mut events)?;
    revisions.push(proposal);
    invocation_receipts.push(proposal_receipt);

    let critique_receipt = invoke_head(
        invoker,
        critic.clone(),
        HeadInvocationKind::Critique,
        &binding,
        &input,
        &revisions,
        &constitution,
    )?;
    let critique = append_invocation_revision(&mut binding, &critique_receipt)?;
    binding = contribute_from_receipt(binding, &critique_receipt, &input.started_at, &mut events)?;
    revisions.push(critique);
    invocation_receipts.push(critique_receipt);

    let synthesis_receipt = invoke_head(
        invoker,
        synthesis.clone(),
        HeadInvocationKind::Synthesis,
        &binding,
        &input,
        &revisions,
        &constitution,
    )?;
    let synthesis_revision = append_invocation_revision(&mut binding, &synthesis_receipt)?;
    binding = contribute_from_receipt(binding, &synthesis_receipt, &input.started_at, &mut events)?;
    revisions.push(synthesis_revision);
    invocation_receipts.push(synthesis_receipt);

    binding = apply_step(
        binding,
        "DRAFTS.SYNTHESIZED",
        object_payload(json!({
            "synthesis_id": "synthesis:fake-loop",
            "contributing_heads": [primary.head_id.clone(), critic.head_id.clone()]
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "PUBLICATION.PROPOSED",
        object_payload(json!({
            "publication_id": input.publication_id,
            "draft_hash": input.draft_hash
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    let policy_decision = constitution.publication_decision(&binding);
    binding = apply_step(
        binding,
        "POLICY.CHECKED",
        object_payload(json!({
            "policy_receipt_id": policy_decision.decision_id,
            "allowed": policy_decision.allowed,
            "policy_decision": policy_decision,
            "claims": input.claims
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "PUBLISHED_TO_SUBSTRATE",
        object_payload(json!({
            "publication_id": input.publication_id,
            "substrate_receipt_id": input.substrate_receipt_id
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "OUTCOME.RECORDED",
        object_payload(json!({
            "outcome_id": input.outcome_id,
            "accepted": true,
            "summary": "fake-head loop published grounded output"
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "RUN.CLOSED",
        object_payload(json!({
            "summary": "fake-head loop closed",
            "closed_by": input.closed_by
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    Ok(FakeIntraAgentLoopResult {
        binding,
        events,
        scratchpad_revisions: revisions,
        invocation_receipts,
        primary_head_id: primary.head_id.clone(),
        critic_head_id: critic.head_id.clone(),
        synthesis_head_id: synthesis.head_id.clone(),
    })
}

fn reasoning_heads(registry: &AgentHeadRegistry) -> Vec<ResolvedAgentHead> {
    registry
        .active_resolved_heads()
        .into_iter()
        .filter(|head| head.kind != HeadKind::SkillPlugin)
        .collect()
}

fn invoke_head<I: HeadInvoker>(
    invoker: &I,
    head: ResolvedAgentHead,
    kind: HeadInvocationKind,
    binding: &AgentBinding,
    input: &FakeIntraAgentLoopInput,
    revisions: &[ScratchpadRevision],
    constitution: &Constitution,
) -> Result<HeadInvocationReceipt, IntraAgentLoopError> {
    let prior_revision_ids = revisions
        .iter()
        .map(|revision| revision.revision_id.clone())
        .collect();
    let prior_context = revisions.iter().filter_map(revision_context).collect();
    let policy_decision = constitution.head_turn_decision(binding, &head.head_id, kind);
    let request = HeadInvocationRequest::new_with_context(
        head,
        kind,
        input.task.clone(),
        binding.working_memory_scope.scratchpad.version,
        prior_revision_ids,
        prior_context,
        input.claims.clone(),
        input.started_at.clone(),
    )
    .with_policy_decision(policy_decision);
    invoker
        .invoke(request)
        .map_err(IntraAgentLoopError::Invocation)
}

fn revision_context(revision: &ScratchpadRevision) -> Option<RevisionContext> {
    let kind = revision
        .payload
        .get("kind")
        .and_then(Value::as_str)
        .and_then(parse_invocation_kind)?;
    Some(RevisionContext {
        revision_id: revision.revision_id.clone(),
        kind,
        output_summary: revision.summary.clone(),
        payload: revision.payload.clone(),
    })
}

fn parse_invocation_kind(kind: &str) -> Option<HeadInvocationKind> {
    match kind {
        "proposal" => Some(HeadInvocationKind::Proposal),
        "critique" => Some(HeadInvocationKind::Critique),
        "synthesis" => Some(HeadInvocationKind::Synthesis),
        _ => None,
    }
}

fn contribute_from_receipt(
    binding: AgentBinding,
    receipt: &HeadInvocationReceipt,
    created_at: &str,
    events: &mut Vec<BindingEventState>,
) -> Result<AgentBinding, IntraAgentLoopError> {
    Ok(apply_step(
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
    )?
    .binding)
}

fn append_invocation_revision(
    binding: &mut AgentBinding,
    receipt: &HeadInvocationReceipt,
) -> Result<ScratchpadRevision, IntraAgentLoopError> {
    binding
        .append_scratchpad_revision(
            &receipt.head_id,
            &receipt.output_summary,
            receipt.content_hash.clone(),
            receipt.payload.clone(),
            &receipt.created_at,
        )
        .map_err(IntraAgentLoopError::Binding)
}

fn append_revision(
    binding: &mut AgentBinding,
    head_id: &str,
    summary: &str,
    payload: Payload,
    created_at: &str,
) -> Result<ScratchpadRevision, IntraAgentLoopError> {
    let content_hash = stable_value_hash(&Value::Object(payload.clone()));
    binding
        .append_scratchpad_revision(head_id, summary, content_hash, payload, created_at)
        .map_err(IntraAgentLoopError::Binding)
}

fn apply_step(
    binding: AgentBinding,
    event_type: &str,
    payload: Payload,
    created_at: &str,
    events: &mut Vec<BindingEventState>,
) -> Result<BindingTransitionResult, IntraAgentLoopError> {
    let mut transition = BindingTransitionInput::new(event_type, payload).at(created_at);
    transition.actor = "fake-intra-agent-loop".to_string();
    let result = apply_binding_transition(binding, transition)?;
    events.push(result.event.clone());
    Ok(result)
}

fn object_payload(value: Value) -> Payload {
    match value {
        Value::Object(map) => map,
        _ => Payload::new(),
    }
}
