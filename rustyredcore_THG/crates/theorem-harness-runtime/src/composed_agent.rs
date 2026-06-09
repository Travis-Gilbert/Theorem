use crate::binding_store::{load_binding, persist_binding_run_result, BindingRuntimeError};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::{
    run_intra_agent_loop_with_invoker, AgentBinding, AgentHead, BindingBudgetScope,
    BindingComposition, BindingError, BindingEventState, BindingIdentity, FakeIntraAgentLoopInput,
    GroundedClaim, HeadCostProfile, HeadInvocationError, HeadInvocationReceipt, HeadInvoker,
    HeadKind, HeadReliabilityProfile, HeadTransport, IntraAgentLoopError, ScratchpadRevision,
    TraceTier,
};

pub const DEFAULT_BINDING_ID: &str = "agent:theorem";

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
    let input = FakeIntraAgentLoopInput::new(task, claims);
    let result = run_intra_agent_loop_with_invoker(binding, input, invoker)?;
    persist_binding_run_result(store, &result.binding, &result.events)?;
    let policy_event = result
        .events
        .iter()
        .find(|event| event.event_type == "POLICY.CHECKED");
    Ok(ComposedAgentRunResult {
        binding_id: binding_id.clone(),
        run_id: result.binding.lifecycle.run_id.clone(),
        task: task.to_string(),
        published_claims: result
            .invocation_receipts
            .last()
            .map(|receipt| receipt.claims.clone())
            .unwrap_or_default(),
        consensus_head_set: result.binding.trace_scope.synthesis_heads.clone(),
        alignment_verdict: policy_event
            .map(|event| Value::Object(event.payload.clone()))
            .unwrap_or_else(|| json!({ "allowed": false, "reason": "policy_event_missing" })),
        events: result.events,
        scratchpad_revisions: result.scratchpad_revisions,
        invocation_receipts: result.invocation_receipts,
    })
}

pub fn default_theorem_binding(binding_id: &str) -> Result<AgentBinding, BindingError> {
    let mut binding = AgentBinding::new(
        BindingIdentity {
            agent_id: "theorem".to_string(),
            owner_id: "travis".to_string(),
            agent_name: "Theorem".to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set: vec!["claude".to_string(), "deepseek".to_string()],
        },
        BindingComposition {
            heads: vec![
                head(
                    "claude",
                    "anthropic",
                    std::env::var("ANTHROPIC_MODEL")
                        .unwrap_or_else(|_| "claude-3-5-sonnet-latest".to_string()),
                    "env:ANTHROPIC_API_KEY",
                    HeadTransport::Api,
                ),
                head(
                    "deepseek",
                    "deepseek",
                    std::env::var("DEEPSEEK_MODEL")
                        .unwrap_or_else(|_| "deepseek-reasoner".to_string()),
                    "env:DEEPSEEK_API_KEY",
                    HeadTransport::Mcp,
                ),
            ],
        },
        BindingBudgetScope::new("theorem", 32_000.0, 2),
    )?;
    binding.lifecycle.run_id = normalize_binding_id(binding_id);
    Ok(binding)
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

fn normalize_binding_id(binding_id: &str) -> String {
    let trimmed = binding_id.trim();
    if trimmed.is_empty() {
        DEFAULT_BINDING_ID.to_string()
    } else {
        trimmed.to_string()
    }
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
    use theorem_harness_core::FakeHeadInvoker;

    #[test]
    fn composed_agent_run_persists_binding_events_and_scratchpad() {
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
        assert_eq!(result.consensus_head_set, vec!["claude", "deepseek"]);
        assert_eq!(result.events.last().unwrap().event_type, "RUN.CLOSED");
        assert!(load_binding(&store, "agent:test").unwrap().is_some());
        assert_eq!(
            crate::load_binding_events(&store, "agent:test")
                .unwrap()
                .len(),
            result.events.len()
        );
    }
}
