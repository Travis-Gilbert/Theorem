//! Deterministic fake-head intra-agent loop scaffold.
//!
//! This module exercises the composed-agent lifecycle without provider calls.
//! It is intentionally a coordinator over the existing binding kernel: the
//! registry chooses fake resolved heads, scratchpad revisions record the
//! proposal/critique/synthesis work, and `apply_binding_transition` remains the
//! only place that enforces budget, consensus, action-tier, and grounding rules.

use crate::agent_binding::{
    apply_binding_transition, AgentBinding, BindingBudgetDecision, BindingError, BindingEventState,
    BindingHeadOutcome, BindingRoutingDecision, BindingSubtask, BindingTransitionInput,
    BindingTransitionResult, BindingVerificationOutcome, HeadKind, ScratchpadRelationKind,
    ScratchpadRevision, ScratchpadRevisionLink,
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
use std::collections::BTreeSet;
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
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default = "default_routing_explore_token")]
    pub routing_explore_token: u32,
    #[serde(default = "default_max_rounds")]
    pub max_rounds: u32,
    #[serde(default = "default_convergence_confidence_threshold")]
    pub convergence_confidence_threshold: f32,
    #[serde(default = "default_expected_value_units")]
    pub expected_value_units: f64,
    #[serde(default = "default_expected_invocation_cost_units")]
    pub expected_invocation_cost_units: f64,
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
            domain: default_domain(),
            routing_explore_token: default_routing_explore_token(),
            max_rounds: default_max_rounds(),
            convergence_confidence_threshold: default_convergence_confidence_threshold(),
            expected_value_units: default_expected_value_units(),
            expected_invocation_cost_units: default_expected_invocation_cost_units(),
            started_at: "2026-06-02T00:00:00Z".to_string(),
            closed_by: "fake-loop".to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingLoopRound {
    pub round_index: u32,
    pub proposal_head_id: String,
    pub critic_head_id: String,
    pub synthesis_head_id: String,
    pub verifier_head_id: String,
    pub synthesis_revision_id: String,
    pub verification_revision_id: String,
    pub verification_outcome: BindingVerificationOutcome,
    pub confidence: f32,
    pub stop_reason: String,
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
    pub verifier_head_id: String,
    #[serde(default)]
    pub routing_decisions: Vec<BindingRoutingDecision>,
    #[serde(default)]
    pub budget_decisions: Vec<BindingBudgetDecision>,
    #[serde(default)]
    pub rounds: Vec<BindingLoopRound>,
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

    let mut binding = binding;
    let mut events = Vec::new();
    let mut revisions = Vec::new();
    let mut invocation_receipts = Vec::new();
    let mut routing_decisions = Vec::new();
    let mut budget_decisions = Vec::new();
    let mut rounds = Vec::new();
    let mut primary_head_id = String::new();
    let mut critic_head_id = String::new();
    let mut synthesis_head_id = String::new();
    let mut verifier_head_id = String::new();

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
        &heads[0].head_id,
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

    let max_rounds = input.max_rounds.max(1);
    let mut current_confidence = 0.0_f32;
    let mut contributing_heads = BTreeSet::new();
    let mut final_synthesis_id = String::new();
    let mut final_synthesis_revision_id = String::new();
    let mut final_verification_receipt: Option<HeadInvocationReceipt> = None;

    for round_index in 1..=max_rounds {
        let primary_route = route_head(
            &binding,
            &heads,
            HeadInvocationKind::Proposal,
            &input.domain,
            &[],
            input.routing_explore_token.saturating_add(round_index),
        )?;
        let primary = primary_route.0;
        let primary_decision = primary_route.1;
        let primary_budget = budget_decision_for_route(
            &binding,
            &primary_decision,
            round_index,
            current_confidence,
            &input,
        )?;
        routing_decisions.push(primary_decision.clone());
        budget_decisions.push(primary_budget.clone());

        let proposal_receipt = invoke_head(
            invoker,
            primary.clone(),
            HeadInvocationKind::Proposal,
            &binding,
            &input,
            &revisions,
            &constitution,
        )?;
        let proposal = append_invocation_revision(&mut binding, &proposal_receipt, &revisions)?;
        binding =
            contribute_from_receipt(binding, &proposal_receipt, &input.started_at, &mut events)?;
        revisions.push(proposal);
        invocation_receipts.push(proposal_receipt);
        contributing_heads.insert(primary.head_id.clone());

        let critic_route = route_head(
            &binding,
            &heads,
            HeadInvocationKind::Critique,
            &input.domain,
            &[primary.head_id.clone()],
            input.routing_explore_token.saturating_add(round_index),
        )?;
        let critic = critic_route.0;
        let critic_decision = critic_route.1;
        let critic_budget = budget_decision_for_route(
            &binding,
            &critic_decision,
            round_index,
            current_confidence,
            &input,
        )?;
        routing_decisions.push(critic_decision.clone());
        budget_decisions.push(critic_budget.clone());

        let critique_receipt = invoke_head(
            invoker,
            critic.clone(),
            HeadInvocationKind::Critique,
            &binding,
            &input,
            &revisions,
            &constitution,
        )?;
        let critique = append_invocation_revision(&mut binding, &critique_receipt, &revisions)?;
        binding =
            contribute_from_receipt(binding, &critique_receipt, &input.started_at, &mut events)?;
        revisions.push(critique);
        invocation_receipts.push(critique_receipt);
        contributing_heads.insert(critic.head_id.clone());

        let synthesis_route = route_head(
            &binding,
            &heads,
            HeadInvocationKind::Synthesis,
            &input.domain,
            &[],
            input.routing_explore_token.saturating_add(round_index),
        )?;
        let synthesis = synthesis_route.0;
        let synthesis_decision = synthesis_route.1;
        let synthesis_budget = budget_decision_for_route(
            &binding,
            &synthesis_decision,
            round_index,
            current_confidence,
            &input,
        )?;
        routing_decisions.push(synthesis_decision.clone());
        budget_decisions.push(synthesis_budget.clone());

        let synthesis_receipt = invoke_head(
            invoker,
            synthesis.clone(),
            HeadInvocationKind::Synthesis,
            &binding,
            &input,
            &revisions,
            &constitution,
        )?;
        let synthesis_revision =
            append_invocation_revision(&mut binding, &synthesis_receipt, &revisions)?;
        let synthesis_revision_id = synthesis_revision.revision_id.clone();
        binding =
            contribute_from_receipt(binding, &synthesis_receipt, &input.started_at, &mut events)?;
        revisions.push(synthesis_revision);
        invocation_receipts.push(synthesis_receipt);
        contributing_heads.insert(synthesis.head_id.clone());

        let verifier_route = route_head(
            &binding,
            &heads,
            HeadInvocationKind::Verification,
            &input.domain,
            &[synthesis.head_id.clone()],
            input.routing_explore_token.saturating_add(round_index),
        )?;
        let verifier = verifier_route.0;
        let verifier_decision = verifier_route.1;
        let verifier_budget = budget_decision_for_route(
            &binding,
            &verifier_decision,
            round_index,
            current_confidence,
            &input,
        )?;
        routing_decisions.push(verifier_decision.clone());
        budget_decisions.push(verifier_budget.clone());

        let verification_receipt = invoke_head(
            invoker,
            verifier.clone(),
            HeadInvocationKind::Verification,
            &binding,
            &input,
            &revisions,
            &constitution,
        )?;
        let verification_revision =
            append_invocation_revision(&mut binding, &verification_receipt, &revisions)?;
        let verification_revision_id = verification_revision.revision_id.clone();
        let verification_outcome = verification_outcome_from_receipt(&verification_receipt);
        let confidence = round_confidence(
            verification_outcome,
            &[
                &primary_decision,
                &critic_decision,
                &synthesis_decision,
                &verifier_decision,
            ],
        );
        current_confidence = confidence;
        revisions.push(verification_revision);

        final_synthesis_id = format!("synthesis:fake-loop:{round_index}");
        final_synthesis_revision_id = synthesis_revision_id.clone();
        final_verification_receipt = Some(verification_receipt.clone());
        primary_head_id = primary.head_id.clone();
        critic_head_id = critic.head_id.clone();
        synthesis_head_id = synthesis.head_id.clone();
        verifier_head_id = verifier.head_id.clone();

        let stop_reason = if verification_outcome == BindingVerificationOutcome::Accepted
            && confidence >= input.convergence_confidence_threshold
        {
            "verified_converged".to_string()
        } else if round_index >= max_rounds {
            "max_rounds_reached".to_string()
        } else {
            let continuation = continuation_budget_decision(
                &binding,
                &verifier.head_id,
                round_index + 1,
                confidence,
                &input,
            )?;
            let should_continue = continuation.should_invoke;
            let reason = continuation.reason.clone();
            budget_decisions.push(continuation);
            if should_continue {
                String::new()
            } else {
                reason
            }
        };
        let should_continue = stop_reason.is_empty();

        rounds.push(BindingLoopRound {
            round_index,
            proposal_head_id: primary.head_id.clone(),
            critic_head_id: critic.head_id.clone(),
            synthesis_head_id: synthesis.head_id.clone(),
            verifier_head_id: verifier.head_id.clone(),
            synthesis_revision_id,
            verification_revision_id,
            verification_outcome,
            confidence,
            stop_reason: if should_continue {
                "continue".to_string()
            } else {
                stop_reason.clone()
            },
        });

        if should_continue {
            binding = contribute_from_receipt(
                binding,
                &verification_receipt,
                &input.started_at,
                &mut events,
            )?;
            invocation_receipts.push(verification_receipt);
            continue;
        }

        break;
    }

    let verification_receipt = final_verification_receipt
        .expect("at least one loop round should always produce a verification receipt");
    let verification_attempts = payload_array(
        verification_receipt.payload.get("attempted_failure_modes"),
        vec![
            "grounding gap".to_string(),
            "counterexample search".to_string(),
        ],
    );
    let verification_commands = payload_array(
        verification_receipt.payload.get("commands_run"),
        vec!["binding synthesis verification".to_string()],
    );
    let verification_outcome = payload_string(
        verification_receipt.payload.get("outcome"),
        "accepted".to_string(),
    );
    let verification_id = format!("verification:{}", verification_receipt.invocation_id);
    let verification_head_id = verification_receipt.head_id.clone();
    let verification_receipt_hash = verification_receipt.receipt_hash.clone();
    let verification_cost_units = verification_receipt.cost_units;

    binding = apply_step(
        binding,
        "DRAFTS.SYNTHESIZED",
        object_payload(json!({
            "synthesis_id": final_synthesis_id,
            "synthesis_revision_id": final_synthesis_revision_id,
            "contributing_heads": contributing_heads.into_iter().collect::<Vec<_>>()
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;

    binding = apply_step(
        binding,
        "SYNTHESIS.VERIFIED",
        object_payload(json!({
            "verification_id": verification_id,
            "synthesis_id": final_synthesis_id,
            "verifier_head_id": verification_head_id,
            "target_revision_id": final_synthesis_revision_id,
            "outcome": verification_outcome,
            "attempted_failure_modes": verification_attempts,
            "commands_run": verification_commands,
            "receipt_hash": verification_receipt_hash,
            "cost_units": verification_cost_units
        })),
        &input.started_at,
        &mut events,
    )?
    .binding;
    invocation_receipts.push(verification_receipt);

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

    let head_outcomes = head_outcomes_for_rounds(&rounds, &input.domain, &input.outcome_id);
    binding = apply_step(
        binding,
        "OUTCOME.RECORDED",
        object_payload(json!({
            "outcome_id": input.outcome_id,
            "accepted": true,
            "summary": "fake-head loop published grounded output",
            "head_outcomes": head_outcomes
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
        primary_head_id,
        critic_head_id,
        synthesis_head_id,
        verifier_head_id,
        routing_decisions,
        budget_decisions,
        rounds,
    })
}

fn reasoning_heads(registry: &AgentHeadRegistry) -> Vec<ResolvedAgentHead> {
    registry
        .active_resolved_heads()
        .into_iter()
        .filter(|head| head.kind != HeadKind::SkillPlugin)
        .collect()
}

fn route_head(
    binding: &AgentBinding,
    heads: &[ResolvedAgentHead],
    kind: HeadInvocationKind,
    domain: &str,
    exclude_head_ids: &[String],
    explore_token: u32,
) -> Result<(ResolvedAgentHead, BindingRoutingDecision), IntraAgentLoopError> {
    let mut candidates = heads
        .iter()
        .filter(|head| !exclude_head_ids.contains(&head.head_id))
        .map(|head| head.head_id.clone())
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        candidates = heads.iter().map(|head| head.head_id.clone()).collect();
    }
    let subtask = BindingSubtask::new(
        format!("binding:{}", kind.as_str()),
        kind.as_str(),
        domain.to_string(),
    );
    let decision = binding
        .route_subtask_from_candidates(&subtask, &candidates, explore_token)
        .ok_or(IntraAgentLoopError::NotEnoughReasoningHeads {
            available: heads.len(),
        })?;
    let head = heads
        .iter()
        .find(|head| head.head_id == decision.head_id)
        .cloned()
        .ok_or(IntraAgentLoopError::NotEnoughReasoningHeads {
            available: heads.len(),
        })?;
    Ok((head, decision))
}

fn budget_decision_for_route(
    binding: &AgentBinding,
    routing_decision: &BindingRoutingDecision,
    round_index: u32,
    current_confidence: f32,
    input: &FakeIntraAgentLoopInput,
) -> Result<BindingBudgetDecision, IntraAgentLoopError> {
    let subtask = BindingSubtask::new(
        routing_decision.subtask_id.clone(),
        routing_decision.capability.clone(),
        routing_decision.domain.clone(),
    );
    binding
        .value_aware_budget_decision(
            &subtask,
            &routing_decision.head_id,
            round_index,
            current_confidence,
            input.expected_value_units,
            input.expected_invocation_cost_units,
        )
        .ok_or(IntraAgentLoopError::NotEnoughReasoningHeads {
            available: binding.routeable_head_ids().len(),
        })
}

fn continuation_budget_decision(
    binding: &AgentBinding,
    head_id: &str,
    round_index: u32,
    current_confidence: f32,
    input: &FakeIntraAgentLoopInput,
) -> Result<BindingBudgetDecision, IntraAgentLoopError> {
    let subtask = BindingSubtask::new(
        format!("binding:round:{round_index}"),
        "round_continuation",
        input.domain.clone(),
    );
    binding
        .value_aware_budget_decision(
            &subtask,
            head_id,
            round_index,
            current_confidence,
            input.expected_value_units,
            input.expected_invocation_cost_units * 5.0,
        )
        .ok_or(IntraAgentLoopError::NotEnoughReasoningHeads {
            available: binding.routeable_head_ids().len(),
        })
}

fn verification_outcome_from_receipt(
    receipt: &HeadInvocationReceipt,
) -> BindingVerificationOutcome {
    match payload_string(receipt.payload.get("outcome"), "accepted".to_string()).as_str() {
        "accepted" => BindingVerificationOutcome::Accepted,
        "defect_found" => BindingVerificationOutcome::DefectFound,
        _ => BindingVerificationOutcome::Rejected,
    }
}

fn round_confidence(
    verification_outcome: BindingVerificationOutcome,
    routing_decisions: &[&BindingRoutingDecision],
) -> f32 {
    if verification_outcome != BindingVerificationOutcome::Accepted {
        return 0.0;
    }
    let total = routing_decisions
        .iter()
        .map(|decision| decision.posterior_success_rate)
        .sum::<f32>();
    if routing_decisions.is_empty() {
        0.0
    } else {
        total / routing_decisions.len() as f32
    }
}

fn head_outcomes_for_rounds(
    rounds: &[BindingLoopRound],
    domain: &str,
    outcome_id: &str,
) -> Vec<BindingHeadOutcome> {
    let mut outcomes = Vec::new();
    for round in rounds {
        let round_accepted = round.verification_outcome == BindingVerificationOutcome::Accepted;
        outcomes.push(round_head_outcome(
            &round.proposal_head_id,
            "proposal",
            domain,
            round_accepted,
            outcome_id,
            round.round_index,
        ));
        outcomes.push(round_head_outcome(
            &round.critic_head_id,
            "critique",
            domain,
            round_accepted,
            outcome_id,
            round.round_index,
        ));
        outcomes.push(round_head_outcome(
            &round.synthesis_head_id,
            "synthesis",
            domain,
            round_accepted,
            outcome_id,
            round.round_index,
        ));
        outcomes.push(round_head_outcome(
            &round.verifier_head_id,
            "verification",
            domain,
            round.verification_outcome != BindingVerificationOutcome::Rejected,
            outcome_id,
            round.round_index,
        ));
    }
    outcomes
}

fn round_head_outcome(
    head_id: &str,
    capability: &str,
    domain: &str,
    accepted: bool,
    outcome_id: &str,
    round_index: u32,
) -> BindingHeadOutcome {
    let outcome_hash = stable_value_hash(&json!({
        "outcome_id": outcome_id,
        "round_index": round_index,
        "head_id": head_id,
        "capability": capability,
        "domain": domain,
        "accepted": accepted,
    }));
    BindingHeadOutcome::new(head_id, capability, domain, accepted, outcome_hash)
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
        "verification" => Some(HeadInvocationKind::Verification),
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
    prior_revisions: &[ScratchpadRevision],
) -> Result<ScratchpadRevision, IntraAgentLoopError> {
    let parent_revision_ids = parent_revision_ids_for(receipt, prior_revisions);
    let links = revision_links_for(receipt, prior_revisions);
    binding
        .append_scratchpad_revision_with_links(
            &receipt.head_id,
            &receipt.output_summary,
            receipt.content_hash.clone(),
            receipt.payload.clone(),
            parent_revision_ids,
            links,
            &receipt.created_at,
        )
        .map_err(IntraAgentLoopError::Binding)
}

fn parent_revision_ids_for(
    receipt: &HeadInvocationReceipt,
    prior_revisions: &[ScratchpadRevision],
) -> Vec<String> {
    match receipt.kind {
        HeadInvocationKind::Synthesis => prior_revisions
            .iter()
            .filter(|revision| {
                revision
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "proposal" || kind == "critique")
            })
            .map(|revision| revision.revision_id.clone())
            .collect(),
        HeadInvocationKind::Verification => prior_revisions
            .iter()
            .rev()
            .find(|revision| {
                revision
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "synthesis")
            })
            .map(|revision| vec![revision.revision_id.clone()])
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn revision_links_for(
    receipt: &HeadInvocationReceipt,
    prior_revisions: &[ScratchpadRevision],
) -> Vec<ScratchpadRevisionLink> {
    match receipt.kind {
        HeadInvocationKind::Critique => prior_revisions
            .iter()
            .rev()
            .find(|revision| {
                revision
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "proposal")
            })
            .map(|revision| {
                vec![ScratchpadRevisionLink::new(
                    revision.revision_id.clone(),
                    ScratchpadRelationKind::Annotates,
                    "critique annotates proposal",
                    Payload::new(),
                )]
            })
            .unwrap_or_default(),
        HeadInvocationKind::Synthesis => prior_revisions
            .iter()
            .filter(|revision| {
                revision
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "proposal" || kind == "critique")
            })
            .map(|revision| {
                ScratchpadRevisionLink::new(
                    revision.revision_id.clone(),
                    ScratchpadRelationKind::Supersedes,
                    "synthesis supersedes prior partial work",
                    Payload::new(),
                )
            })
            .collect(),
        HeadInvocationKind::Verification => prior_revisions
            .iter()
            .rev()
            .find(|revision| {
                revision
                    .payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind == "synthesis")
            })
            .map(|revision| {
                let relation_kind =
                    if payload_string(receipt.payload.get("outcome"), "accepted".to_string())
                        == "defect_found"
                    {
                        ScratchpadRelationKind::Undercuts
                    } else {
                        ScratchpadRelationKind::Supports
                    };
                vec![ScratchpadRevisionLink::new(
                    revision.revision_id.clone(),
                    relation_kind,
                    "verification checks synthesis",
                    Payload::new(),
                )]
            })
            .unwrap_or_default(),
        HeadInvocationKind::Proposal => Vec::new(),
    }
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

fn payload_array(value: Option<&Value>, fallback: Vec<String>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => {
            let values = items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>();
            if values.is_empty() {
                fallback
            } else {
                values
            }
        }
        _ => fallback,
    }
}

fn payload_string(value: Option<&Value>, fallback: String) -> String {
    value
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or(fallback)
}

fn default_domain() -> String {
    "general".to_string()
}

fn default_routing_explore_token() -> u32 {
    999
}

fn default_max_rounds() -> u32 {
    1
}

fn default_convergence_confidence_threshold() -> f32 {
    0.5
}

fn default_expected_value_units() -> f64 {
    10.0
}

fn default_expected_invocation_cost_units() -> f64 {
    1.0
}
