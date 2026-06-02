use crate::alignment::evaluate_publication;
use crate::budget::{apply_contribution_charge, check_contribution_budget, BindingBudgetState};
use crate::state_hash::stable_value_hash;
use crate::types::{now_string, prefixed_id, GuardViolation, Payload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum BindingError {
    Guard(Box<GuardViolation>),
}

impl fmt::Display for BindingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindingError::Guard(violation) => write!(f, "{}", violation.message),
        }
    }
}

impl Error for BindingError {}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingIdentity {
    pub agent_id: String,
    pub owner_id: String,
    pub agent_name: String,
    #[serde(default)]
    pub composition_hash: String,
    #[serde(default = "default_version")]
    pub version: u32,
    pub trust_tier: String,
    #[serde(default)]
    pub active_head_set: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingComposition {
    pub heads: Vec<AgentHead>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentHead {
    pub head_id: String,
    #[serde(default)]
    pub display_name: String,
    pub provider: String,
    pub model: String,
    pub credential_ref: String,
    pub transport: HeadTransport,
    pub kind: HeadKind,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub cost_profile: HeadCostProfile,
    #[serde(default)]
    pub reliability_profile: HeadReliabilityProfile,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub trace_tier: TraceTier,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadTransport {
    Api,
    Mcp,
    Local,
    Hosted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadKind {
    ReasoningCore,
    SkillPlugin,
    SpecializedCoder,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadCostProfile {
    #[serde(default)]
    pub input_unit_cost: f64,
    #[serde(default)]
    pub output_unit_cost: f64,
    #[serde(default)]
    pub max_context_tokens: u64,
}

impl Default for HeadCostProfile {
    fn default() -> Self {
        Self {
            input_unit_cost: 0.0,
            output_unit_cost: 0.0,
            max_context_tokens: 0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadReliabilityProfile {
    #[serde(default)]
    pub success_rate: f32,
    #[serde(default)]
    pub median_latency_ms: u64,
    #[serde(default)]
    pub last_outcome_hash: String,
}

impl Default for HeadReliabilityProfile {
    fn default() -> Self {
        Self {
            success_rate: 0.0,
            median_latency_ms: 0,
            last_outcome_hash: String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TraceTier {
    Minimal,
    #[default]
    Receipt,
    Full,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingMemoryScope {
    pub scope_id: String,
    pub scratchpad: ScratchpadDocument,
    #[serde(default)]
    pub zones: Vec<MemoryZone>,
}

impl BindingMemoryScope {
    pub fn for_agent(agent_id: &str) -> Self {
        Self {
            scope_id: format!("bindingscope:{agent_id}"),
            scratchpad: ScratchpadDocument::new(format!("scratchpad:{agent_id}")),
            zones: vec![
                MemoryZone::new("head_local", MemoryZoneKind::HeadLocal, "one head only"),
                MemoryZone::new(
                    "binding_private",
                    MemoryZoneKind::BindingPrivate,
                    "active reasoning heads in this binding",
                ),
                MemoryZone::new(
                    "agent_published",
                    MemoryZoneKind::AgentPublished,
                    "committed agent-visible state",
                ),
                MemoryZone::new("commons", MemoryZoneKind::Commons, "shared substrate state"),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryZone {
    pub zone_id: String,
    pub kind: MemoryZoneKind,
    pub visibility: String,
}

impl MemoryZone {
    pub fn new(
        zone_id: impl Into<String>,
        kind: MemoryZoneKind,
        visibility: impl Into<String>,
    ) -> Self {
        Self {
            zone_id: zone_id.into(),
            kind,
            visibility: visibility.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryZoneKind {
    HeadLocal,
    BindingPrivate,
    AgentPublished,
    Commons,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadDocument {
    pub document_id: String,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub revisions: Vec<ScratchpadRevision>,
}

impl ScratchpadDocument {
    pub fn new(document_id: impl Into<String>) -> Self {
        Self {
            document_id: document_id.into(),
            version: 0,
            revisions: Vec::new(),
        }
    }

    pub fn append(
        &mut self,
        actor_head_id: impl Into<String>,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        created_at: impl Into<String>,
    ) -> ScratchpadRevision {
        let parent_revision_id = self
            .revisions
            .last()
            .map(|revision| revision.revision_id.clone())
            .unwrap_or_default();
        self.version += 1;
        let revision = ScratchpadRevision {
            revision_id: prefixed_id("scratchrev"),
            parent_revision_id,
            seq: self.version,
            actor_head_id: actor_head_id.into(),
            summary: summary.into(),
            content_hash: content_hash.into(),
            payload,
            created_at: created_at.into(),
        };
        self.revisions.push(revision.clone());
        revision
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadRevision {
    pub revision_id: String,
    #[serde(default)]
    pub parent_revision_id: String,
    pub seq: u64,
    pub actor_head_id: String,
    pub summary: String,
    pub content_hash: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PublishedScope {
    pub scope_id: String,
    #[serde(default)]
    pub visible_artifact_types: Vec<String>,
}

impl PublishedScope {
    pub fn for_agent(agent_id: &str) -> Self {
        Self {
            scope_id: format!("published:{agent_id}"),
            visible_artifact_types: vec![
                "claim".to_string(),
                "context_artifact".to_string(),
                "handoff_artifact".to_string(),
                "publication_event".to_string(),
                "tool_receipt".to_string(),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingCapabilityScope {
    pub scope_id: String,
    #[serde(default)]
    pub charter_hash: String,
    #[serde(default)]
    pub charter_summary: String,
    #[serde(default)]
    pub visible_tools: Vec<String>,
    #[serde(default)]
    pub callable_tools: Vec<String>,
    #[serde(default)]
    pub confirmation_gated_tools: Vec<String>,
    #[serde(default)]
    pub binding_private_tools: Vec<String>,
    #[serde(default)]
    pub action_tiers: Vec<ActionTierPolicy>,
}

impl BindingCapabilityScope {
    pub fn for_agent(agent_id: &str) -> Self {
        Self {
            scope_id: format!("capability:{agent_id}"),
            charter_hash: String::new(),
            charter_summary: String::new(),
            visible_tools: Vec::new(),
            callable_tools: Vec::new(),
            confirmation_gated_tools: Vec::new(),
            binding_private_tools: Vec::new(),
            action_tiers: vec![
                ActionTierPolicy::new("tier_one", "reversible substrate action", false),
                ActionTierPolicy::new("tier_two", "consequential commit action", true),
                ActionTierPolicy::new("tier_three", "irreversible external action", true),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionTierPolicy {
    pub tier_id: String,
    pub description: String,
    pub requires_human_authorization: bool,
}

impl ActionTierPolicy {
    pub fn new(
        tier_id: impl Into<String>,
        description: impl Into<String>,
        requires_human_authorization: bool,
    ) -> Self {
        Self {
            tier_id: tier_id.into(),
            description: description.into(),
            requires_human_authorization,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingBudgetScope {
    pub scope_id: String,
    pub shared_budget_units: f64,
    #[serde(default)]
    pub allocated_run_budget_units: f64,
    #[serde(default)]
    pub escalation_threshold_units: f64,
    #[serde(default)]
    pub background_allowance_units: f64,
    #[serde(default = "default_parallel_heads")]
    pub max_parallel_heads: usize,
    #[serde(default)]
    pub per_head_limits: Vec<HeadBudgetLimit>,
}

impl BindingBudgetScope {
    pub fn new(agent_id: &str, shared_budget_units: f64, max_parallel_heads: usize) -> Self {
        Self {
            scope_id: format!("budget:{agent_id}"),
            shared_budget_units,
            allocated_run_budget_units: 0.0,
            escalation_threshold_units: shared_budget_units,
            background_allowance_units: 0.0,
            max_parallel_heads,
            per_head_limits: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadBudgetLimit {
    pub head_id: String,
    pub max_units: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingTraceScope {
    pub trace_id: String,
    #[serde(default)]
    pub trace_tier: TraceTier,
    #[serde(default = "default_true")]
    pub receipts_required: bool,
    #[serde(default)]
    pub contributions: Vec<HeadContributionRecord>,
    #[serde(default)]
    pub synthesis_heads: Vec<String>,
}

impl BindingTraceScope {
    pub fn for_agent(agent_id: &str) -> Self {
        Self {
            trace_id: format!("trace:{agent_id}"),
            trace_tier: TraceTier::Receipt,
            receipts_required: true,
            contributions: Vec::new(),
            synthesis_heads: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadContributionRecord {
    pub contribution_id: String,
    pub head_id: String,
    pub contribution_kind: String,
    #[serde(default)]
    pub weight: f32,
    #[serde(default)]
    pub receipt_hash: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingLifecycleState {
    pub run_id: String,
    #[serde(default = "created_status")]
    pub status: String,
    #[serde(default)]
    pub last_event_seq: u64,
    #[serde(default = "now_string")]
    pub created_at: String,
    #[serde(default = "now_string")]
    pub updated_at: String,
}

impl BindingLifecycleState {
    pub fn new() -> Self {
        let now = now_string();
        Self {
            run_id: prefixed_id("bindingrun"),
            status: created_status(),
            last_event_seq: 0,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

impl Default for BindingLifecycleState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentBinding {
    pub identity: BindingIdentity,
    pub composition: BindingComposition,
    pub working_memory_scope: BindingMemoryScope,
    pub published_scope: PublishedScope,
    pub capability_scope: BindingCapabilityScope,
    pub budget_scope: BindingBudgetScope,
    pub trace_scope: BindingTraceScope,
    #[serde(default)]
    pub budget_state: BindingBudgetState,
    #[serde(default)]
    pub lifecycle: BindingLifecycleState,
}

impl AgentBinding {
    pub fn new(
        mut identity: BindingIdentity,
        composition: BindingComposition,
        budget_scope: BindingBudgetScope,
    ) -> Result<Self, BindingError> {
        identity.active_head_set = clean_strings(identity.active_head_set);
        let agent_id = identity.agent_id.clone();
        let mut binding = Self {
            identity,
            composition,
            working_memory_scope: BindingMemoryScope::for_agent(&agent_id),
            published_scope: PublishedScope::for_agent(&agent_id),
            capability_scope: BindingCapabilityScope::for_agent(&agent_id),
            budget_scope,
            trace_scope: BindingTraceScope::for_agent(&agent_id),
            budget_state: BindingBudgetState::default(),
            lifecycle: BindingLifecycleState::new(),
        };
        if binding.identity.composition_hash.trim().is_empty() {
            binding.identity.composition_hash = composition_hash(&binding);
        }
        validate_binding(&binding)?;
        Ok(binding)
    }

    pub fn active_head_ids(&self) -> BTreeSet<String> {
        self.identity.active_head_set.iter().cloned().collect()
    }

    pub fn head(&self, head_id: &str) -> Option<&AgentHead> {
        self.composition
            .heads
            .iter()
            .find(|head| head.head_id == head_id)
    }

    pub fn reasoning_core_ids(&self) -> Vec<String> {
        let active = self.active_head_ids();
        self.composition
            .heads
            .iter()
            .filter(|head| active.contains(&head.head_id))
            .filter(|head| head.kind == HeadKind::ReasoningCore)
            .map(|head| head.head_id.clone())
            .collect()
    }

    pub fn append_scratchpad_revision(
        &mut self,
        actor_head_id: &str,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        created_at: impl Into<String>,
    ) -> Result<ScratchpadRevision, BindingError> {
        let head = self.head(actor_head_id).ok_or_else(|| {
            guard_violation(
                "unknown_binding_head",
                format!("head {actor_head_id} is not registered in this binding"),
                "",
                "",
                Vec::new(),
                Payload::new(),
            )
        })?;
        if !self.active_head_ids().contains(actor_head_id) {
            return Err(guard_violation(
                "inactive_binding_head",
                format!("head {actor_head_id} is not active in this binding"),
                "",
                "",
                Vec::new(),
                Payload::new(),
            ));
        }
        if head.kind == HeadKind::SkillPlugin {
            return Err(guard_violation(
                "scratchpad_plugin_denied",
                "skill plugins are tools, not scratchpad-sharing reasoning heads",
                "reasoning_core_or_specialized_coder",
                "skill_plugin",
                Vec::new(),
                Payload::new(),
            ));
        }
        Ok(self.working_memory_scope.scratchpad.append(
            actor_head_id,
            summary,
            content_hash,
            payload,
            created_at,
        ))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingTransitionInput {
    #[serde(default)]
    pub run_id: String,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default)]
    pub actor: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

impl BindingTransitionInput {
    pub fn new(event_type: impl Into<String>, payload: Payload) -> Self {
        Self {
            run_id: String::new(),
            event_type: event_type.into(),
            payload,
            actor: String::new(),
            created_at: now_string(),
        }
    }

    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn at(mut self, created_at: impl Into<String>) -> Self {
        self.created_at = created_at.into();
        self
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingEventState {
    #[serde(default = "binding_event_id")]
    pub event_id: String,
    pub run_id: String,
    pub seq: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub payload: Payload,
    pub binding_status_before: String,
    pub binding_status_after: String,
    pub state_hash_before: String,
    pub state_hash_after: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingTransitionResult {
    pub binding: AgentBinding,
    pub event: BindingEventState,
    pub state_hash_before: String,
    pub state_hash_after: String,
}

pub fn apply_binding_transition(
    mut binding: AgentBinding,
    transition: BindingTransitionInput,
) -> Result<BindingTransitionResult, BindingError> {
    validate_binding(&binding)?;
    if binding_target_status(&transition.event_type).is_empty() {
        return Err(guard_violation(
            "unsupported_binding_transition",
            format!("unsupported binding transition {}", transition.event_type),
            "",
            binding.lifecycle.status,
            Vec::new(),
            Payload::new(),
        ));
    }
    reject_terminal_binding(&binding, &transition)?;
    if !transition.run_id.is_empty() && transition.run_id != binding.lifecycle.run_id {
        let mut details = Payload::new();
        details.insert(
            "event_run_id".to_string(),
            Value::String(transition.run_id.clone()),
        );
        details.insert(
            "state_run_id".to_string(),
            Value::String(binding.lifecycle.run_id.clone()),
        );
        return Err(guard_violation(
            "binding_run_id_mismatch",
            "binding transition run_id does not match the binding lifecycle run",
            "",
            binding.lifecycle.status,
            Vec::new(),
            details,
        ));
    }

    require_payload_fields(
        &transition,
        binding_transition_requirements(&transition.event_type),
    )?;
    validate_binding_previous_state(&binding, &transition)?;
    apply_binding_guard(&binding, &transition)?;

    let before_status = binding.lifecycle.status.clone();
    let before_hash = hash_agent_binding(&binding);
    apply_binding_payload(&mut binding, &transition)?;
    binding.lifecycle.status = binding_target_status(&transition.event_type).to_string();
    binding.lifecycle.last_event_seq += 1;
    binding.lifecycle.updated_at = transition.created_at.clone();
    let after_hash = hash_agent_binding(&binding);
    let event = BindingEventState {
        event_id: binding_event_id(),
        run_id: binding.lifecycle.run_id.clone(),
        seq: binding.lifecycle.last_event_seq,
        event_type: transition.event_type,
        payload: transition.payload,
        binding_status_before: before_status,
        binding_status_after: binding.lifecycle.status.clone(),
        state_hash_before: before_hash.clone(),
        state_hash_after: after_hash.clone(),
        actor: transition.actor,
        created_at: transition.created_at,
    };

    Ok(BindingTransitionResult {
        binding,
        event,
        state_hash_before: before_hash,
        state_hash_after: after_hash,
    })
}

pub fn hash_agent_binding(binding: &AgentBinding) -> String {
    let data =
        serde_json::to_value(binding).expect("AgentBinding serialization should be infallible");
    stable_value_hash(&json!({
        "identity": data.get("identity").cloned().unwrap_or(Value::Null),
        "working_memory_scope": data.get("working_memory_scope").cloned().unwrap_or(Value::Null),
        "published_scope": data.get("published_scope").cloned().unwrap_or(Value::Null),
        "capability_scope": data.get("capability_scope").cloned().unwrap_or(Value::Null),
        "budget_scope": data.get("budget_scope").cloned().unwrap_or(Value::Null),
        "budget_state": data.get("budget_state").cloned().unwrap_or(Value::Null),
        "trace_scope": data.get("trace_scope").cloned().unwrap_or(Value::Null),
        "lifecycle": data.get("lifecycle").cloned().unwrap_or(Value::Null),
    }))
}

pub fn composition_hash(binding: &AgentBinding) -> String {
    stable_value_hash(&json!({
        "heads": binding.composition.heads,
        "active_head_set": sorted_strings(&binding.identity.active_head_set),
    }))
}

fn validate_binding(binding: &AgentBinding) -> Result<(), BindingError> {
    let mut missing = Vec::new();
    for (field, value) in [
        ("agent_id", binding.identity.agent_id.as_str()),
        ("owner_id", binding.identity.owner_id.as_str()),
        ("agent_name", binding.identity.agent_name.as_str()),
        ("trust_tier", binding.identity.trust_tier.as_str()),
    ] {
        if value.trim().is_empty() {
            missing.push(field.to_string());
        }
    }
    if binding.composition.heads.is_empty() {
        missing.push("composition.heads".to_string());
    }
    if binding.identity.active_head_set.is_empty() {
        missing.push("active_head_set".to_string());
    }
    if binding.budget_scope.shared_budget_units < 0.0 {
        missing.push("budget_scope.shared_budget_units".to_string());
    }
    if binding.budget_scope.max_parallel_heads == 0 {
        missing.push("budget_scope.max_parallel_heads".to_string());
    }
    if !missing.is_empty() {
        return Err(guard_violation(
            "invalid_agent_binding",
            format!(
                "AgentBinding missing or invalid fields: {}",
                missing.join(", ")
            ),
            "",
            binding.lifecycle.status.clone(),
            missing,
            Payload::new(),
        ));
    }

    let head_ids = binding
        .composition
        .heads
        .iter()
        .map(|head| head.head_id.clone())
        .collect::<BTreeSet<_>>();
    let unknown = binding
        .identity
        .active_head_set
        .iter()
        .filter(|head_id| !head_ids.contains(*head_id))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        let mut details = Payload::new();
        details.insert(
            "unknown_heads".to_string(),
            Value::Array(unknown.iter().cloned().map(Value::String).collect()),
        );
        return Err(guard_violation(
            "unknown_active_heads",
            "active_head_set contains heads not present in the composition",
            "",
            binding.lifecycle.status.clone(),
            unknown,
            details,
        ));
    }

    for head in &binding.composition.heads {
        let mut head_missing = Vec::new();
        for (field, value) in [
            ("head_id", head.head_id.as_str()),
            ("provider", head.provider.as_str()),
            ("model", head.model.as_str()),
            ("credential_ref", head.credential_ref.as_str()),
        ] {
            if value.trim().is_empty() {
                head_missing.push(field.to_string());
            }
        }
        if !head_missing.is_empty() {
            return Err(guard_violation(
                "invalid_agent_head",
                format!(
                    "AgentHead {} missing fields: {}",
                    head.head_id,
                    head_missing.join(", ")
                ),
                "",
                binding.lifecycle.status.clone(),
                head_missing,
                Payload::new(),
            ));
        }
    }

    Ok(())
}

fn apply_binding_payload(
    binding: &mut AgentBinding,
    transition: &BindingTransitionInput,
) -> Result<(), BindingError> {
    match transition.event_type.as_str() {
        "BINDING.RESOLVED" => {
            binding.identity.composition_hash = composition_hash(binding);
        }
        "CHARTER.COMPILED" => {
            binding.capability_scope.charter_hash =
                payload_to_string(transition.payload.get("charter_hash"));
            binding.capability_scope.charter_summary =
                payload_to_string(transition.payload.get("stance"));
        }
        "CAPABILITIES.SELECTED" => {
            binding.capability_scope.visible_tools =
                payload_array_strings(transition.payload.get("visible_tools"));
            binding.capability_scope.callable_tools =
                payload_array_strings(transition.payload.get("callable_tools"));
            binding.capability_scope.confirmation_gated_tools =
                payload_array_strings(transition.payload.get("confirmation_gated_tools"));
            binding.capability_scope.binding_private_tools =
                payload_array_strings(transition.payload.get("binding_private_tools"));
        }
        "BUDGET.ALLOCATED" => {
            binding.budget_scope.allocated_run_budget_units =
                payload_f64(transition.payload.get("budget_units"));
            binding.budget_scope.max_parallel_heads =
                payload_usize(transition.payload.get("max_parallel_heads"));
        }
        "HEADS.CONTRIBUTE" => {
            let head_id = payload_to_string(transition.payload.get("head_id"));
            binding
                .trace_scope
                .contributions
                .push(HeadContributionRecord {
                    contribution_id: payload_to_string(transition.payload.get("contribution_id")),
                    head_id: head_id.clone(),
                    contribution_kind: payload_to_string(
                        transition.payload.get("contribution_kind"),
                    ),
                    weight: payload_f32(transition.payload.get("weight")),
                    receipt_hash: payload_to_string(transition.payload.get("receipt_hash")),
                    created_at: transition.created_at.clone(),
                });
            apply_contribution_charge(
                &mut binding.budget_state,
                &head_id,
                payload_f64(transition.payload.get("cost_units")),
            );
        }
        "DRAFTS.SYNTHESIZED" => {
            binding.trace_scope.synthesis_heads =
                payload_array_strings(transition.payload.get("contributing_heads"));
        }
        _ => {}
    }
    Ok(())
}

fn apply_binding_guard(
    binding: &AgentBinding,
    transition: &BindingTransitionInput,
) -> Result<(), BindingError> {
    match transition.event_type.as_str() {
        "BUDGET.ALLOCATED" => {
            let budget_units = payload_f64(transition.payload.get("budget_units"));
            if budget_units <= 0.0 {
                return Err(guard_violation(
                    "invalid_binding_budget",
                    "BUDGET.ALLOCATED requires a positive budget_units value",
                    "",
                    binding.lifecycle.status.clone(),
                    Vec::new(),
                    Payload::new(),
                ));
            }
            if budget_units > binding.budget_scope.shared_budget_units {
                let mut details = Payload::new();
                details.insert("budget_units".to_string(), json!(budget_units));
                details.insert(
                    "shared_budget_units".to_string(),
                    json!(binding.budget_scope.shared_budget_units),
                );
                return Err(guard_violation(
                    "binding_budget_exceeded",
                    "BUDGET.ALLOCATED exceeds the binding shared budget",
                    "",
                    binding.lifecycle.status.clone(),
                    Vec::new(),
                    details,
                ));
            }
        }
        "HEADS.CONTRIBUTE" => {
            let head_id = payload_to_string(transition.payload.get("head_id"));
            if !binding.active_head_ids().contains(&head_id) {
                return Err(guard_violation(
                    "inactive_binding_head",
                    format!("head {head_id} is not active in this binding"),
                    "",
                    binding.lifecycle.status.clone(),
                    Vec::new(),
                    Payload::new(),
                ));
            }
            check_contribution_budget(
                &binding.budget_scope,
                &binding.budget_state,
                &head_id,
                payload_f64(transition.payload.get("cost_units")),
            )?;
        }
        "DRAFTS.SYNTHESIZED" => {
            let active = binding.active_head_ids();
            let unknown = payload_array_strings(transition.payload.get("contributing_heads"))
                .into_iter()
                .filter(|head_id| !active.contains(head_id))
                .collect::<Vec<_>>();
            if !unknown.is_empty() {
                let mut details = Payload::new();
                details.insert(
                    "unknown_heads".to_string(),
                    Value::Array(unknown.iter().cloned().map(Value::String).collect()),
                );
                return Err(guard_violation(
                    "unknown_synthesis_heads",
                    "DRAFTS.SYNTHESIZED includes heads outside the active set",
                    "",
                    binding.lifecycle.status.clone(),
                    unknown,
                    details,
                ));
            }
        }
        "POLICY.CHECKED" if !payload_bool(transition.payload.get("allowed")) => {
            return Err(guard_violation(
                "binding_policy_denied",
                "POLICY.CHECKED denied the proposed publication",
                "policy_allowed",
                "policy_denied",
                Vec::new(),
                Payload::new(),
            ));
        }
        "POLICY.CHECKED" => {
            evaluate_publication(
                &binding.trace_scope.synthesis_heads,
                &binding.capability_scope.action_tiers,
                &transition.payload,
            )?;
        }
        "MEMORY_PATCHES.PROPOSED" if !payload_bool(transition.payload.get("review_required")) => {
            return Err(guard_violation(
                "binding_memory_patch_review_required",
                "memory patches proposed by a binding must require review",
                "",
                binding.lifecycle.status.clone(),
                Vec::new(),
                Payload::new(),
            ));
        }
        _ => {}
    }
    Ok(())
}

fn reject_terminal_binding(
    binding: &AgentBinding,
    transition: &BindingTransitionInput,
) -> Result<(), BindingError> {
    if binding.lifecycle.status == "closed" {
        return Err(guard_violation(
            "terminal_binding_state",
            format!(
                "{} cannot be applied to terminal binding state closed",
                transition.event_type
            ),
            "",
            binding.lifecycle.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    Ok(())
}

fn validate_binding_previous_state(
    binding: &AgentBinding,
    transition: &BindingTransitionInput,
) -> Result<(), BindingError> {
    let allowed = binding_allowed_previous_statuses(&transition.event_type);
    if allowed.is_empty() || allowed.contains(&binding.lifecycle.status.as_str()) {
        return Ok(());
    }
    Err(guard_violation(
        "invalid_binding_previous_state",
        format!(
            "{} requires status {}; received {}",
            transition.event_type,
            allowed.join(", "),
            binding.lifecycle.status
        ),
        allowed.join(", "),
        binding.lifecycle.status.clone(),
        Vec::new(),
        Payload::new(),
    ))
}

fn require_payload_fields(
    transition: &BindingTransitionInput,
    fields: &'static [&'static str],
) -> Result<(), BindingError> {
    let missing = fields
        .iter()
        .copied()
        .filter(|field| is_missing_required(transition.payload.get(*field)))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(guard_violation(
        "missing_binding_payload_fields",
        format!(
            "{} missing required payload fields: {}",
            transition.event_type,
            missing.join(", ")
        ),
        "",
        "",
        missing,
        Payload::new(),
    ))
}

fn binding_transition_requirements(event: &str) -> &'static [&'static str] {
    match event {
        "BINDING.RESOLVED" => &["binding_id", "composition_hash"],
        "HEADS.PROBED" => &["probed_head_set"],
        "MEMORY_SCOPE.MOUNTED" => &["scope_id", "scratchpad_id"],
        "CHARTER.COMPILED" => &["charter_hash", "stance"],
        "CAPABILITIES.SELECTED" => &["capability_scope_hash", "visible_tools", "callable_tools"],
        "BUDGET.ALLOCATED" => &["budget_units", "max_parallel_heads"],
        "RUN.STARTED" => &["task", "started_at"],
        "PRIVATE_WORK.OPENED" => &["scratchpad_revision_id"],
        "HEADS.CONTRIBUTE" => &["head_id", "contribution_id", "contribution_kind"],
        "DRAFTS.SYNTHESIZED" => &["synthesis_id", "contributing_heads"],
        "PUBLICATION.PROPOSED" => &["publication_id", "draft_hash"],
        "POLICY.CHECKED" => &["policy_receipt_id", "allowed"],
        "PUBLISHED_TO_SUBSTRATE" => &["publication_id", "substrate_receipt_id"],
        "OUTCOME.RECORDED" => &["outcome_id", "accepted", "summary"],
        "MEMORY_PATCHES.PROPOSED" => &["patch_ids", "review_required"],
        "RUN.CLOSED" => &["summary", "closed_by"],
        _ => &[],
    }
}

fn binding_allowed_previous_statuses(event: &str) -> &'static [&'static str] {
    match event {
        "BINDING.RESOLVED" => &["created", "binding_resolved"],
        "HEADS.PROBED" => &["binding_resolved", "heads_probed"],
        "MEMORY_SCOPE.MOUNTED" => &["heads_probed", "memory_scope_mounted"],
        "CHARTER.COMPILED" => &["memory_scope_mounted", "charter_compiled"],
        "CAPABILITIES.SELECTED" => &["charter_compiled", "capabilities_selected"],
        "BUDGET.ALLOCATED" => &["capabilities_selected", "budget_allocated"],
        "RUN.STARTED" => &["budget_allocated", "run_started"],
        "PRIVATE_WORK.OPENED" => &["run_started", "private_work_opened"],
        "HEADS.CONTRIBUTE" => &["private_work_opened", "heads_contribute"],
        "DRAFTS.SYNTHESIZED" => &["heads_contribute", "drafts_synthesized"],
        "PUBLICATION.PROPOSED" => &["drafts_synthesized", "publication_proposed"],
        "POLICY.CHECKED" => &["publication_proposed", "policy_checked"],
        "PUBLISHED_TO_SUBSTRATE" => &["policy_checked", "published_to_substrate"],
        "OUTCOME.RECORDED" => &["published_to_substrate", "outcome_recorded"],
        "MEMORY_PATCHES.PROPOSED" => &["outcome_recorded", "memory_patches_proposed"],
        "RUN.CLOSED" => &["outcome_recorded", "memory_patches_proposed"],
        _ => &[],
    }
}

fn binding_target_status(event: &str) -> &'static str {
    match event {
        "BINDING.RESOLVED" => "binding_resolved",
        "HEADS.PROBED" => "heads_probed",
        "MEMORY_SCOPE.MOUNTED" => "memory_scope_mounted",
        "CHARTER.COMPILED" => "charter_compiled",
        "CAPABILITIES.SELECTED" => "capabilities_selected",
        "BUDGET.ALLOCATED" => "budget_allocated",
        "RUN.STARTED" => "run_started",
        "PRIVATE_WORK.OPENED" => "private_work_opened",
        "HEADS.CONTRIBUTE" => "heads_contribute",
        "DRAFTS.SYNTHESIZED" => "drafts_synthesized",
        "PUBLICATION.PROPOSED" => "publication_proposed",
        "POLICY.CHECKED" => "policy_checked",
        "PUBLISHED_TO_SUBSTRATE" => "published_to_substrate",
        "OUTCOME.RECORDED" => "outcome_recorded",
        "MEMORY_PATCHES.PROPOSED" => "memory_patches_proposed",
        "RUN.CLOSED" => "closed",
        _ => "",
    }
}

fn guard_violation(
    code: impl Into<String>,
    message: impl Into<String>,
    required_state: impl Into<String>,
    received_state: impl Into<String>,
    missing_fields: Vec<String>,
    details: Payload,
) -> BindingError {
    BindingError::Guard(Box::new(GuardViolation {
        code: code.into(),
        message: message.into(),
        required_state: required_state.into(),
        received_state: received_state.into(),
        missing_fields,
        details,
    }))
}

fn payload_to_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn payload_array_strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => clean_strings(
            items
                .iter()
                .map(|item| payload_to_string(Some(item)))
                .collect(),
        ),
        _ => Vec::new(),
    }
}

fn payload_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value == "true",
        _ => false,
    }
}

fn payload_f64(value: Option<&Value>) -> f64 {
    match value {
        Some(Value::Number(value)) => value.as_f64().unwrap_or(0.0),
        Some(Value::String(value)) => value.parse::<f64>().unwrap_or(0.0),
        _ => 0.0,
    }
}

fn payload_f32(value: Option<&Value>) -> f32 {
    payload_f64(value) as f32
}

fn payload_usize(value: Option<&Value>) -> usize {
    match value {
        Some(Value::Number(value)) => value.as_u64().unwrap_or(0) as usize,
        Some(Value::String(value)) => value.parse::<usize>().unwrap_or(0),
        _ => 0,
    }
}

fn is_missing_required(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.is_empty(),
        Some(Value::Array(items)) => items.is_empty(),
        _ => false,
    }
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sorted_strings(values: &[String]) -> Vec<String> {
    clean_strings(values.to_vec())
}

fn default_version() -> u32 {
    1
}

fn default_parallel_heads() -> usize {
    1
}

fn default_true() -> bool {
    true
}

fn created_status() -> String {
    "created".to_string()
}

fn binding_event_id() -> String {
    prefixed_id("bindingevent")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    #[test]
    fn composition_hash_changes_when_active_roster_changes() {
        let mut first = fixture_binding();
        let first_hash = first.identity.composition_hash.clone();
        first.identity.active_head_set = vec!["claude".to_string()];
        let second_hash = composition_hash(&first);

        assert_ne!(first_hash, second_hash);
    }

    #[test]
    fn binding_lifecycle_reaches_closed_with_receipts() {
        let binding = fixture_binding();
        let binding = apply(
            binding,
            "BINDING.RESOLVED",
            json!({
                "binding_id": "agent:theorem",
                "composition_hash": "caller:ignored"
            }),
        );
        assert_eq!(binding.event.seq, 1);
        assert_eq!(binding.binding.lifecycle.status, "binding_resolved");
        assert_eq!(
            binding.binding.identity.composition_hash,
            composition_hash(&binding.binding)
        );

        let binding = apply(
            binding.binding,
            "HEADS.PROBED",
            json!({
                "probed_head_set": ["claude", "deepseek", "mistral_ocr"]
            }),
        );
        let binding = apply(
            binding.binding,
            "MEMORY_SCOPE.MOUNTED",
            json!({
                "scope_id": "bindingscope:theorem",
                "scratchpad_id": "scratchpad:theorem"
            }),
        );
        let binding = apply(
            binding.binding,
            "CHARTER.COMPILED",
            json!({
                "charter_hash": "charter:1",
                "stance": "grounded composed agent"
            }),
        );
        let binding = apply(
            binding.binding,
            "CAPABILITIES.SELECTED",
            json!({
                "capability_scope_hash": "capability:1",
                "visible_tools": ["datalog", "probabilistic"],
                "callable_tools": ["datalog"],
                "confirmation_gated_tools": ["publisher"],
                "binding_private_tools": ["scratchpad"]
            }),
        );
        let binding = apply(
            binding.binding,
            "BUDGET.ALLOCATED",
            json!({
                "budget_units": 25.0,
                "max_parallel_heads": 2
            }),
        );
        assert_eq!(
            binding.binding.budget_scope.allocated_run_budget_units,
            25.0
        );
        assert_eq!(binding.binding.budget_scope.max_parallel_heads, 2);

        let binding = apply(
            binding.binding,
            "RUN.STARTED",
            json!({
                "task": "answer with Theorem voice",
                "started_at": "2026-06-02T00:00:00Z"
            }),
        );
        let binding = apply(
            binding.binding,
            "PRIVATE_WORK.OPENED",
            json!({
                "scratchpad_revision_id": "scratchrev:1"
            }),
        );
        let binding = apply(
            binding.binding,
            "HEADS.CONTRIBUTE",
            json!({
                "head_id": "claude",
                "contribution_id": "contrib:1",
                "contribution_kind": "proposal",
                "weight": 0.6,
                "receipt_hash": "receipt:1"
            }),
        );
        assert_eq!(binding.binding.trace_scope.contributions.len(), 1);

        let binding = apply(
            binding.binding,
            "DRAFTS.SYNTHESIZED",
            json!({
                "synthesis_id": "synth:1",
                "contributing_heads": ["claude", "deepseek"]
            }),
        );
        let binding = apply(
            binding.binding,
            "PUBLICATION.PROPOSED",
            json!({
                "publication_id": "pub:1",
                "draft_hash": "draft:1"
            }),
        );
        let binding = apply(
            binding.binding,
            "POLICY.CHECKED",
            json!({
                "policy_receipt_id": "policy:1",
                "allowed": true,
                "claims": [{ "text": "Theorem published a grounded answer", "provenance": "src:1" }]
            }),
        );
        let binding = apply(
            binding.binding,
            "PUBLISHED_TO_SUBSTRATE",
            json!({
                "publication_id": "pub:1",
                "substrate_receipt_id": "substrate:1"
            }),
        );
        let binding = apply(
            binding.binding,
            "OUTCOME.RECORDED",
            json!({
                "outcome_id": "outcome:1",
                "accepted": true,
                "summary": "published"
            }),
        );
        let binding = apply(
            binding.binding,
            "MEMORY_PATCHES.PROPOSED",
            json!({
                "patch_ids": ["patch:1"],
                "review_required": true
            }),
        );
        let binding = apply(
            binding.binding,
            "RUN.CLOSED",
            json!({
                "summary": "closed",
                "closed_by": "codex"
            }),
        );

        assert_eq!(binding.binding.lifecycle.status, "closed");
        assert_eq!(binding.event.seq, 16);
        assert_ne!(binding.state_hash_before, binding.state_hash_after);
    }

    #[test]
    fn budget_allocation_is_a_hard_guard() {
        let binding = ready_for_budget();
        let error = apply_binding_transition(
            binding,
            transition(
                "BUDGET.ALLOCATED",
                json!({
                    "budget_units": 101.0,
                    "max_parallel_heads": 3
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "binding_budget_exceeded");
    }

    #[test]
    fn inactive_head_cannot_contribute() {
        let binding = ready_for_contribution();
        let error = apply_binding_transition(
            binding,
            transition(
                "HEADS.CONTRIBUTE",
                json!({
                    "head_id": "mistral_ocr",
                    "contribution_id": "contrib:plugin",
                    "contribution_kind": "proposal"
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "inactive_binding_head");
    }

    #[test]
    fn skill_plugins_cannot_append_to_private_scratchpad() {
        let mut binding = fixture_binding();
        binding
            .identity
            .active_head_set
            .push("mistral_ocr".to_string());
        let error = binding
            .append_scratchpad_revision(
                "mistral_ocr",
                "ocr result",
                "hash:ocr",
                object_payload(json!({ "pages": 2 })),
                "2026-06-02T00:00:00Z",
            )
            .unwrap_err();

        assert_guard(error, "scratchpad_plugin_denied");
    }

    #[test]
    fn reasoning_core_appends_versioned_scratchpad_revision() {
        let mut binding = fixture_binding();
        let first = binding
            .append_scratchpad_revision(
                "claude",
                "initial proposal",
                "hash:proposal",
                object_payload(json!({ "claim_count": 3 })),
                "2026-06-02T00:00:00Z",
            )
            .unwrap();
        let second = binding
            .append_scratchpad_revision(
                "deepseek",
                "critique",
                "hash:critique",
                object_payload(json!({ "finding_count": 1 })),
                "2026-06-02T00:00:01Z",
            )
            .unwrap();

        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
        assert_eq!(second.parent_revision_id, first.revision_id);
        assert_eq!(binding.working_memory_scope.scratchpad.version, 2);
    }

    #[test]
    fn policy_denial_blocks_publication_path() {
        let error = apply_binding_transition(
            ready_for_publication(),
            transition(
                "POLICY.CHECKED",
                json!({
                    "policy_receipt_id": "policy:blocked",
                    "allowed": false
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "binding_policy_denied");
    }

    #[test]
    fn contribution_over_run_budget_is_blocked() {
        let error = apply_binding_transition(
            ready_for_contribution(),
            transition(
                "HEADS.CONTRIBUTE",
                json!({
                    "head_id": "claude",
                    "contribution_id": "contrib:big",
                    "contribution_kind": "proposal",
                    "cost_units": 30.0
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "binding_budget_overspent");
    }

    #[test]
    fn publication_below_consensus_is_blocked() {
        let binding = apply(
            ready_for_contribution(),
            "HEADS.CONTRIBUTE",
            json!({
                "head_id": "claude",
                "contribution_id": "contrib:1",
                "contribution_kind": "proposal"
            }),
        );
        let binding = apply(
            binding.binding,
            "DRAFTS.SYNTHESIZED",
            json!({
                "synthesis_id": "synth:1",
                "contributing_heads": ["claude"]
            }),
        );
        let binding = apply(
            binding.binding,
            "PUBLICATION.PROPOSED",
            json!({
                "publication_id": "pub:1",
                "draft_hash": "draft:1"
            }),
        );
        let error = apply_binding_transition(
            binding.binding,
            transition(
                "POLICY.CHECKED",
                json!({ "policy_receipt_id": "policy:1", "allowed": true }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "consensus_below_threshold");
    }

    #[test]
    fn claimless_publication_is_blocked_at_policy_check() {
        let error = apply_binding_transition(
            ready_for_publication(),
            transition(
                "POLICY.CHECKED",
                json!({ "policy_receipt_id": "policy:1", "allowed": true }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "grounding_missing");
    }

    #[test]
    fn tier_three_publication_requires_human_authorization() {
        let error = apply_binding_transition(
            ready_for_publication(),
            transition(
                "POLICY.CHECKED",
                json!({
                    "policy_receipt_id": "policy:1",
                    "allowed": true,
                    "action_tier": "tier_three"
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "tier_requires_human_authorization");
    }

    fn ready_for_budget() -> AgentBinding {
        let binding = apply(
            fixture_binding(),
            "BINDING.RESOLVED",
            json!({
                "binding_id": "agent:theorem",
                "composition_hash": "caller:ignored"
            }),
        );
        let binding = apply(
            binding.binding,
            "HEADS.PROBED",
            json!({
                "probed_head_set": ["claude", "deepseek"]
            }),
        );
        let binding = apply(
            binding.binding,
            "MEMORY_SCOPE.MOUNTED",
            json!({
                "scope_id": "bindingscope:theorem",
                "scratchpad_id": "scratchpad:theorem"
            }),
        );
        let binding = apply(
            binding.binding,
            "CHARTER.COMPILED",
            json!({
                "charter_hash": "charter:1",
                "stance": "grounded"
            }),
        );
        apply(
            binding.binding,
            "CAPABILITIES.SELECTED",
            json!({
                "capability_scope_hash": "capability:1",
                "visible_tools": ["datalog"],
                "callable_tools": ["datalog"]
            }),
        )
        .binding
    }

    fn ready_for_contribution() -> AgentBinding {
        let binding = apply(
            ready_for_budget(),
            "BUDGET.ALLOCATED",
            json!({
                "budget_units": 25.0,
                "max_parallel_heads": 2
            }),
        );
        let binding = apply(
            binding.binding,
            "RUN.STARTED",
            json!({
                "task": "compose",
                "started_at": "2026-06-02T00:00:00Z"
            }),
        );
        apply(
            binding.binding,
            "PRIVATE_WORK.OPENED",
            json!({
                "scratchpad_revision_id": "scratchrev:1"
            }),
        )
        .binding
    }

    fn ready_for_publication() -> AgentBinding {
        let binding = apply(
            ready_for_contribution(),
            "HEADS.CONTRIBUTE",
            json!({
                "head_id": "claude",
                "contribution_id": "contrib:1",
                "contribution_kind": "proposal"
            }),
        );
        let binding = apply(
            binding.binding,
            "DRAFTS.SYNTHESIZED",
            json!({
                "synthesis_id": "synth:1",
                "contributing_heads": ["claude", "deepseek"]
            }),
        );
        apply(
            binding.binding,
            "PUBLICATION.PROPOSED",
            json!({
                "publication_id": "pub:1",
                "draft_hash": "draft:1"
            }),
        )
        .binding
    }

    fn fixture_binding() -> AgentBinding {
        AgentBinding::new(
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
                    head("claude", "anthropic", "claude", HeadKind::ReasoningCore),
                    head("deepseek", "deepseek", "v4", HeadKind::ReasoningCore),
                    head("mistral_ocr", "mistral", "voxtral", HeadKind::SkillPlugin),
                ],
            },
            BindingBudgetScope::new("theorem", 100.0, 3),
        )
        .unwrap()
    }

    fn head(head_id: &str, provider: &str, model: &str, kind: HeadKind) -> AgentHead {
        AgentHead {
            head_id: head_id.to_string(),
            display_name: head_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            credential_ref: format!("credential:{head_id}"),
            transport: HeadTransport::Api,
            kind,
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }

    fn apply(binding: AgentBinding, event_type: &str, payload: Value) -> BindingTransitionResult {
        apply_binding_transition(binding, transition(event_type, payload)).unwrap()
    }

    fn transition(event_type: &str, payload: Value) -> BindingTransitionInput {
        BindingTransitionInput::new(event_type, object_payload(payload)).at("2026-06-02T00:00:00Z")
    }

    fn object_payload(payload: Value) -> Payload {
        match payload {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn assert_guard(error: BindingError, expected_code: &str) {
        match error {
            BindingError::Guard(violation) => assert_eq!(violation.code, expected_code),
        }
    }
}
