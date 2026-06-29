use crate::alignment::evaluate_publication;
use crate::budget::{apply_contribution_charge, check_contribution_budget, BindingBudgetState};
use crate::state_hash::stable_value_hash;
use crate::types::{now_string, prefixed_id, GuardViolation, Payload};
use crate::user_model::{user_model_hash, UserModel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

pub const TIER_ONE_ACTION: &str = "tier_one";
pub const TIER_TWO_ACTION: &str = "tier_two";
pub const TIER_THREE_ACTION: &str = "tier_three";
pub const PROGRAMMABLE_CAPABILITY_ACTION_TIER: &str = TIER_TWO_ACTION;

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
    /// Optional persona/voice text that conditions every head contribution this
    /// binding produces. The text is owned by the caller that constructs the
    /// binding; loading it from disk (e.g. from
    /// `docs/plans/agent-theorem/constitution.md`) is a higher-layer concern.
    /// When present, the intra-agent loop threads it through every head
    /// invocation request so proposal, critique, synthesis, and verification
    /// share one voice.
    #[serde(default)]
    pub agent_constitution: Option<String>,
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
    Verifier,
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
    #[serde(default)]
    pub capability_scores: Vec<HeadCapabilityReliability>,
}

impl Default for HeadReliabilityProfile {
    fn default() -> Self {
        Self {
            success_rate: 0.0,
            median_latency_ms: 0,
            last_outcome_hash: String::new(),
            capability_scores: Vec::new(),
        }
    }
}

impl HeadReliabilityProfile {
    pub fn reliability_for(&self, capability: &str, domain: &str) -> f32 {
        let capability = capability.trim();
        let domain = domain.trim();
        let mut best: Option<(u8, f32)> = None;
        for score in &self.capability_scores {
            if score.capability.trim() != capability {
                continue;
            }
            let score_domain = score.domain.trim();
            let rank = if !domain.is_empty() && score_domain == domain {
                3
            } else if score_domain.is_empty() || score_domain == "general" {
                2
            } else {
                1
            };
            let rate = score.posterior_success_rate();
            if best.is_none_or(|(best_rank, best_rate)| {
                rank > best_rank || (rank == best_rank && rate > best_rate)
            }) {
                best = Some((rank, rate));
            }
        }
        best.map(|(_, rate)| rate).unwrap_or_else(|| {
            if self.success_rate > 0.0 {
                self.success_rate
            } else {
                0.5
            }
        })
    }

    pub fn record_capability_outcome(
        &mut self,
        capability: impl Into<String>,
        domain: impl Into<String>,
        accepted: bool,
        outcome_hash: impl Into<String>,
    ) {
        let capability = capability.into();
        let domain = domain.into();
        let outcome_hash = outcome_hash.into();
        let score = self
            .capability_scores
            .iter_mut()
            .find(|score| score.capability == capability && score.domain == domain);
        match score {
            Some(score) => score.record_outcome(accepted, outcome_hash.clone()),
            None => {
                let mut score = HeadCapabilityReliability::new(capability, domain, 0, 0);
                score.record_outcome(accepted, outcome_hash.clone());
                self.capability_scores.push(score);
            }
        }
        self.last_outcome_hash = outcome_hash;
        let successes = self
            .capability_scores
            .iter()
            .map(|score| score.successes)
            .sum::<u64>();
        let failures = self
            .capability_scores
            .iter()
            .map(|score| score.failures)
            .sum::<u64>();
        let total = successes + failures;
        if total > 0 {
            self.success_rate = successes as f32 / total as f32;
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadCapabilityReliability {
    pub capability: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub successes: u64,
    #[serde(default)]
    pub failures: u64,
    #[serde(default)]
    pub last_outcome_hash: String,
}

impl HeadCapabilityReliability {
    pub fn new(
        capability: impl Into<String>,
        domain: impl Into<String>,
        successes: u64,
        failures: u64,
    ) -> Self {
        Self {
            capability: capability.into(),
            domain: domain.into(),
            successes,
            failures,
            last_outcome_hash: String::new(),
        }
    }

    pub fn posterior_success_rate(&self) -> f32 {
        ((self.successes + 1) as f32) / ((self.successes + self.failures + 2) as f32)
    }

    pub fn record_outcome(&mut self, accepted: bool, outcome_hash: impl Into<String>) {
        if accepted {
            self.successes += 1;
        } else {
            self.failures += 1;
        }
        self.last_outcome_hash = outcome_hash.into();
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BindingSubtask {
    pub subtask_id: String,
    pub capability: String,
    #[serde(default)]
    pub domain: String,
}

impl BindingSubtask {
    pub fn new(
        subtask_id: impl Into<String>,
        capability: impl Into<String>,
        domain: impl Into<String>,
    ) -> Self {
        Self {
            subtask_id: subtask_id.into(),
            capability: capability.into(),
            domain: domain.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingRoutingDecision {
    pub subtask_id: String,
    pub capability: String,
    #[serde(default)]
    pub domain: String,
    pub head_id: String,
    pub posterior_success_rate: f32,
    #[serde(default)]
    pub explored: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingBudgetDecision {
    pub subtask_id: String,
    pub capability: String,
    #[serde(default)]
    pub domain: String,
    pub head_id: String,
    #[serde(default)]
    pub round_index: u32,
    #[serde(default)]
    pub posterior_success_rate: f32,
    #[serde(default)]
    pub current_confidence: f32,
    #[serde(default)]
    pub expected_value_units: f64,
    #[serde(default)]
    pub expected_cost_units: f64,
    #[serde(default)]
    pub marginal_expected_value_units: f64,
    #[serde(default)]
    pub remaining_budget_units: f64,
    #[serde(default)]
    pub should_invoke: bool,
    #[serde(default)]
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BindingHeadOutcome {
    pub head_id: String,
    pub capability: String,
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub outcome_hash: String,
}

impl BindingHeadOutcome {
    pub fn new(
        head_id: impl Into<String>,
        capability: impl Into<String>,
        domain: impl Into<String>,
        accepted: bool,
        outcome_hash: impl Into<String>,
    ) -> Self {
        Self {
            head_id: head_id.into(),
            capability: capability.into(),
            domain: domain.into(),
            accepted,
            outcome_hash: outcome_hash.into(),
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
    pub crdt: ScratchpadCrdtBacking,
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub revisions: Vec<ScratchpadRevision>,
    #[serde(default)]
    pub relations: Vec<ScratchpadRevisionRelation>,
    #[serde(default)]
    pub crdt_ops: Vec<ScratchpadCrdtOperation>,
    #[serde(default)]
    pub awareness: Vec<ScratchpadAwarenessEntry>,
}

impl ScratchpadDocument {
    pub fn new(document_id: impl Into<String>) -> Self {
        let document_id = document_id.into();
        Self {
            crdt: ScratchpadCrdtBacking::for_document(&document_id),
            document_id,
            version: 0,
            revisions: Vec::new(),
            relations: Vec::new(),
            crdt_ops: Vec::new(),
            awareness: Vec::new(),
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
        self.append_with_links(
            actor_head_id,
            summary,
            content_hash,
            payload,
            Vec::new(),
            Vec::new(),
            created_at,
        )
    }

    pub fn append_with_parents(
        &mut self,
        actor_head_id: impl Into<String>,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        parent_revision_ids: Vec<String>,
        created_at: impl Into<String>,
    ) -> ScratchpadRevision {
        self.append_with_links(
            actor_head_id,
            summary,
            content_hash,
            payload,
            parent_revision_ids,
            Vec::new(),
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_with_links(
        &mut self,
        actor_head_id: impl Into<String>,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        parent_revision_ids: Vec<String>,
        links: Vec<ScratchpadRevisionLink>,
        created_at: impl Into<String>,
    ) -> ScratchpadRevision {
        self.append_internal(
            actor_head_id.into(),
            summary.into(),
            content_hash.into(),
            payload,
            parent_revision_ids,
            links,
            created_at.into(),
            prefixed_id("scratchrev"),
            prefixed_id("scratchop"),
        )
    }

    /// Append a revision with caller-provided deterministic `revision_id` and
    /// `op_id`, bypassing the UUID-based generators used by `append` /
    /// `append_with_links`. Used by the binding-mount path so identical
    /// `(agent_id, user_model_hash, created_at)` inputs always produce the
    /// same scratchpad state hash on replay (PR #72 P1 fix).
    #[allow(clippy::too_many_arguments)]
    pub fn append_with_deterministic_ids(
        &mut self,
        actor_head_id: impl Into<String>,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        revision_id: String,
        op_id: String,
        created_at: impl Into<String>,
    ) -> ScratchpadRevision {
        self.append_internal(
            actor_head_id.into(),
            summary.into(),
            content_hash.into(),
            payload,
            Vec::new(),
            Vec::new(),
            created_at.into(),
            revision_id,
            op_id,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn append_internal(
        &mut self,
        actor_head_id: String,
        summary: String,
        content_hash: String,
        payload: Payload,
        parent_revision_ids: Vec<String>,
        links: Vec<ScratchpadRevisionLink>,
        created_at: String,
        revision_id: String,
        op_id: String,
    ) -> ScratchpadRevision {
        let parent_revision_ids = if parent_revision_ids.is_empty() {
            self.revisions
                .last()
                .map(|revision| vec![revision.revision_id.clone()])
                .unwrap_or_default()
        } else {
            clean_strings_preserve_order(parent_revision_ids)
        };
        let parent_revision_id = self
            .revisions
            .last()
            .and_then(|_| parent_revision_ids.first().cloned())
            .unwrap_or_default();
        self.version += 1;
        let revision = ScratchpadRevision {
            revision_id: revision_id.clone(),
            parent_revision_id,
            parent_revision_ids,
            seq: self.version,
            actor_head_id: actor_head_id.clone(),
            summary,
            content_hash,
            payload,
            created_at: created_at.clone(),
        };
        self.revisions.push(revision.clone());
        self.crdt_ops.push(ScratchpadCrdtOperation {
            op_id,
            actor_head_id: actor_head_id.clone(),
            revision_id: revision_id.clone(),
            op_kind: "upsert_revision".to_string(),
            graph_element_id: revision_id.clone(),
            text_region_id: scratchpad_region_for_payload(&revision.payload),
            content_hash: revision.content_hash.clone(),
            payload_hash: stable_value_hash(&Value::Object(revision.payload.clone())),
            created_at: created_at.clone(),
        });
        self.awareness.push(ScratchpadAwarenessEntry {
            actor_head_id: actor_head_id.clone(),
            region_id: scratchpad_region_for_payload(&revision.payload),
            revision_id: revision_id.clone(),
            status: "writing".to_string(),
            cursor: self.version,
            updated_at: created_at.clone(),
        });
        for link in links {
            self.relations.push(ScratchpadRevisionRelation {
                relation_id: prefixed_id("scratchrel"),
                from_revision_id: revision_id.clone(),
                to_revision_id: link.to_revision_id,
                relation_kind: link.relation_kind,
                actor_head_id: actor_head_id.clone(),
                summary: link.summary,
                payload: link.payload,
                created_at: created_at.clone(),
            });
        }
        revision
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScratchpadCrdtKind {
    #[default]
    GraphCrdtYrsRegions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadCrdtBacking {
    #[serde(default)]
    pub kind: ScratchpadCrdtKind,
    pub graph_root_id: String,
    pub yrs_doc_id: String,
    pub stream_topic: String,
    pub awareness_log_id: String,
    #[serde(default)]
    pub text_regions: Vec<ScratchpadTextRegion>,
}

impl Default for ScratchpadCrdtBacking {
    fn default() -> Self {
        Self::for_document("scratchpad:default")
    }
}

impl ScratchpadCrdtBacking {
    pub fn for_document(document_id: &str) -> Self {
        let suffix = document_id.replace(':', "_");
        Self {
            kind: ScratchpadCrdtKind::GraphCrdtYrsRegions,
            graph_root_id: format!("crdtgraph:{suffix}"),
            yrs_doc_id: format!("yrs:{suffix}"),
            stream_topic: format!("scratchpad.crdt.{suffix}"),
            awareness_log_id: format!("awareness:{suffix}"),
            text_regions: vec![
                ScratchpadTextRegion::new("orientation", "run/task and context membrane"),
                ScratchpadTextRegion::new("proposal", "streamed complete attempts"),
                ScratchpadTextRegion::new("critique", "streamed disagreement and gaps"),
                ScratchpadTextRegion::new("synthesis", "shared converging answer"),
                ScratchpadTextRegion::new("verification", "falsification receipts"),
            ],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadTextRegion {
    pub region_id: String,
    pub description: String,
}

impl ScratchpadTextRegion {
    pub fn new(region_id: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            region_id: region_id.into(),
            description: description.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadCrdtOperation {
    pub op_id: String,
    pub actor_head_id: String,
    pub revision_id: String,
    pub op_kind: String,
    pub graph_element_id: String,
    pub text_region_id: String,
    pub content_hash: String,
    pub payload_hash: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadAwarenessEntry {
    pub actor_head_id: String,
    pub region_id: String,
    pub revision_id: String,
    pub status: String,
    pub cursor: u64,
    #[serde(default = "now_string")]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadRevision {
    pub revision_id: String,
    #[serde(default)]
    pub parent_revision_id: String,
    #[serde(default)]
    pub parent_revision_ids: Vec<String>,
    pub seq: u64,
    pub actor_head_id: String,
    pub summary: String,
    pub content_hash: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScratchpadRelationKind {
    Forks,
    Annotates,
    Supersedes,
    Supports,
    Contradicts,
    Undercuts,
}

impl ScratchpadRelationKind {
    pub fn edge_type(self) -> &'static str {
        match self {
            Self::Forks => "HARNESS_SCRATCHPAD_FORKS",
            Self::Annotates => "HARNESS_SCRATCHPAD_ANNOTATES",
            Self::Supersedes => "HARNESS_SCRATCHPAD_SUPERSEDES",
            Self::Supports => "HARNESS_SCRATCHPAD_SUPPORTS",
            Self::Contradicts => "HARNESS_SCRATCHPAD_CONTRADICTS",
            Self::Undercuts => "HARNESS_SCRATCHPAD_UNDERCUTS",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadRevisionRelation {
    pub relation_id: String,
    pub from_revision_id: String,
    pub to_revision_id: String,
    pub relation_kind: ScratchpadRelationKind,
    pub actor_head_id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ScratchpadRevisionLink {
    pub to_revision_id: String,
    pub relation_kind: ScratchpadRelationKind,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub payload: Payload,
}

impl ScratchpadRevisionLink {
    pub fn new(
        to_revision_id: impl Into<String>,
        relation_kind: ScratchpadRelationKind,
        summary: impl Into<String>,
        payload: Payload,
    ) -> Self {
        Self {
            to_revision_id: to_revision_id.into(),
            relation_kind,
            summary: summary.into(),
            payload,
        }
    }
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
                ActionTierPolicy::new(TIER_ONE_ACTION, "reversible substrate action", false),
                ActionTierPolicy::new(
                    TIER_TWO_ACTION,
                    "consequential commit or capability registration action",
                    true,
                ),
                ActionTierPolicy::new(TIER_THREE_ACTION, "irreversible external action", true),
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
    #[serde(default)]
    pub verification_receipts: Vec<BindingVerificationReceipt>,
}

impl BindingTraceScope {
    pub fn for_agent(agent_id: &str) -> Self {
        Self {
            trace_id: format!("trace:{agent_id}"),
            trace_tier: TraceTier::Receipt,
            receipts_required: true,
            contributions: Vec::new(),
            synthesis_heads: Vec::new(),
            verification_receipts: Vec::new(),
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
pub struct BindingVerificationReceipt {
    pub verification_id: String,
    pub synthesis_id: String,
    pub verifier_head_id: String,
    pub target_revision_id: String,
    pub outcome: BindingVerificationOutcome,
    #[serde(default)]
    pub attempted_failure_modes: Vec<String>,
    #[serde(default)]
    pub commands_run: Vec<String>,
    #[serde(default)]
    pub receipt_hash: String,
    #[serde(default = "now_string")]
    pub created_at: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingVerificationOutcome {
    Accepted,
    DefectFound,
    Rejected,
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

    pub fn head_mut(&mut self, head_id: &str) -> Option<&mut AgentHead> {
        self.composition
            .heads
            .iter_mut()
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

    pub fn routeable_head_ids(&self) -> Vec<String> {
        let active = self.active_head_ids();
        self.composition
            .heads
            .iter()
            .filter(|head| active.contains(&head.head_id))
            .filter(|head| head.kind != HeadKind::SkillPlugin)
            .map(|head| head.head_id.clone())
            .collect()
    }

    pub fn route_subtask(
        &self,
        subtask: &BindingSubtask,
        explore_token: u32,
    ) -> Option<BindingRoutingDecision> {
        self.route_subtask_from_candidates(subtask, &self.routeable_head_ids(), explore_token)
    }

    pub fn route_subtask_from_candidates(
        &self,
        subtask: &BindingSubtask,
        candidate_head_ids: &[String],
        explore_token: u32,
    ) -> Option<BindingRoutingDecision> {
        let active = self.active_head_ids();
        let candidates = clean_strings(candidate_head_ids.to_vec())
            .into_iter()
            .filter_map(|head_id| self.head(&head_id))
            .filter(|head| active.contains(&head.head_id))
            .filter(|head| head.kind != HeadKind::SkillPlugin)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return None;
        }
        if explore_token < 150 {
            let head = candidates[(explore_token as usize) % candidates.len()];
            return Some(routing_decision_for(head, subtask, true));
        }

        let mut best = candidates[0];
        for head in candidates.iter().skip(1) {
            if routing_candidate_is_better(head, best, subtask) {
                best = head;
            }
        }
        Some(routing_decision_for(best, subtask, false))
    }

    pub fn value_aware_budget_decision(
        &self,
        subtask: &BindingSubtask,
        head_id: &str,
        round_index: u32,
        current_confidence: f32,
        expected_value_units: f64,
        expected_cost_units: f64,
    ) -> Option<BindingBudgetDecision> {
        let head = self.head(head_id)?;
        if !self.active_head_ids().contains(head_id) || head.kind == HeadKind::SkillPlugin {
            return None;
        }
        let posterior_success_rate = head
            .reliability_profile
            .reliability_for(&subtask.capability, &subtask.domain);
        let current_confidence = current_confidence.clamp(0.0, 1.0);
        let remaining_budget_units =
            remaining_run_budget_units(&self.budget_scope, &self.budget_state);
        let marginal_expected_value_units = (1.0 - f64::from(current_confidence))
            * f64::from(posterior_success_rate)
            * expected_value_units.max(0.0);
        let expected_cost_units = expected_cost_units.max(0.0);
        let round_index = round_index.max(1);
        let budget_available = expected_cost_units <= remaining_budget_units;
        let should_invoke = budget_available
            && (round_index == 1
                || marginal_expected_value_units >= expected_cost_units
                || expected_cost_units == 0.0);
        let reason = if !budget_available {
            "budget_exhausted"
        } else if round_index == 1 {
            "initial_round"
        } else if marginal_expected_value_units >= expected_cost_units {
            "marginal_value_positive"
        } else if expected_cost_units == 0.0 {
            "zero_cost_check"
        } else {
            "marginal_value_below_cost"
        };

        Some(BindingBudgetDecision {
            subtask_id: subtask.subtask_id.clone(),
            capability: subtask.capability.clone(),
            domain: subtask.domain.clone(),
            head_id: head.head_id.clone(),
            round_index,
            posterior_success_rate,
            current_confidence,
            expected_value_units: expected_value_units.max(0.0),
            expected_cost_units,
            marginal_expected_value_units,
            remaining_budget_units,
            should_invoke,
            reason: reason.to_string(),
        })
    }

    pub fn record_head_outcomes(
        &mut self,
        outcomes: Vec<BindingHeadOutcome>,
    ) -> Result<(), BindingError> {
        for outcome in outcomes {
            self.record_head_outcome(outcome)?;
        }
        self.identity.composition_hash = composition_hash(self);
        Ok(())
    }

    fn record_head_outcome(&mut self, outcome: BindingHeadOutcome) -> Result<(), BindingError> {
        let head = self.head_mut(&outcome.head_id).ok_or_else(|| {
            guard_violation(
                "unknown_outcome_head",
                format!("outcome references unregistered head {}", outcome.head_id),
                "registered_head",
                "unknown_head",
                vec!["head_id".to_string()],
                Payload::new(),
            )
        })?;
        head.reliability_profile.record_capability_outcome(
            outcome.capability,
            outcome.domain,
            outcome.accepted,
            outcome.outcome_hash,
        );
        Ok(())
    }

    pub fn append_scratchpad_revision(
        &mut self,
        actor_head_id: &str,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        created_at: impl Into<String>,
    ) -> Result<ScratchpadRevision, BindingError> {
        self.append_scratchpad_revision_with_links(
            actor_head_id,
            summary,
            content_hash,
            payload,
            Vec::new(),
            Vec::new(),
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_scratchpad_revision_with_links(
        &mut self,
        actor_head_id: &str,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        parent_revision_ids: Vec<String>,
        links: Vec<ScratchpadRevisionLink>,
        created_at: impl Into<String>,
    ) -> Result<ScratchpadRevision, BindingError> {
        self.ensure_scratchpad_actor(actor_head_id)?;
        self.ensure_known_revisions("unknown_scratchpad_parent", &parent_revision_ids)?;
        let link_targets = links
            .iter()
            .map(|link| link.to_revision_id.clone())
            .collect::<Vec<_>>();
        self.ensure_known_revisions("unknown_scratchpad_relation_target", &link_targets)?;
        Ok(self.working_memory_scope.scratchpad.append_with_links(
            actor_head_id,
            summary,
            content_hash,
            payload,
            parent_revision_ids,
            links,
            created_at,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_scratchpad_revision_with_parents(
        &mut self,
        actor_head_id: &str,
        summary: impl Into<String>,
        content_hash: impl Into<String>,
        payload: Payload,
        parent_revision_ids: Vec<String>,
        created_at: impl Into<String>,
    ) -> Result<ScratchpadRevision, BindingError> {
        self.append_scratchpad_revision_with_links(
            actor_head_id,
            summary,
            content_hash,
            payload,
            parent_revision_ids,
            Vec::new(),
            created_at,
        )
    }

    fn ensure_scratchpad_actor(&self, actor_head_id: &str) -> Result<(), BindingError> {
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
                "reasoning_core_or_specialized_coder_or_verifier",
                "skill_plugin",
                Vec::new(),
                Payload::new(),
            ));
        }
        Ok(())
    }

    fn ensure_known_revisions(
        &self,
        code: &str,
        revision_ids: &[String],
    ) -> Result<(), BindingError> {
        let known = self
            .working_memory_scope
            .scratchpad
            .revisions
            .iter()
            .map(|revision| revision.revision_id.clone())
            .collect::<BTreeSet<_>>();
        let unknown = clean_strings(revision_ids.to_vec())
            .into_iter()
            .filter(|revision_id| !known.contains(revision_id))
            .collect::<Vec<_>>();
        if unknown.is_empty() {
            return Ok(());
        }
        let mut details = Payload::new();
        details.insert(
            "revision_ids".to_string(),
            Value::Array(unknown.iter().cloned().map(Value::String).collect()),
        );
        Err(guard_violation(
            code,
            "scratchpad revision relation references unknown revision ids",
            "known_scratchpad_revision",
            "unknown_scratchpad_revision",
            unknown,
            details,
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
    mut transition: BindingTransitionInput,
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
    apply_binding_payload(&mut binding, &mut transition)?;
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
    transition: &mut BindingTransitionInput,
) -> Result<(), BindingError> {
    match transition.event_type.as_str() {
        "BINDING.RESOLVED" => {
            binding.identity.composition_hash = composition_hash(binding);
        }
        "MEMORY_SCOPE.MOUNTED" => {
            // S2: Optional `user_model` field. If present, ground every head's
            // turn on the same picture of the user by appending a Context
            // revision to the scratchpad before HEADS.CONTRIBUTE fires, and
            // stamp the model's content hash onto the stored event payload so
            // the receipt records which model was mounted. Absent payload
            // field => no-op (back-compat with the pre-S2 lifecycle).
            if let Some(value) = transition.payload.get("user_model") {
                match serde_json::from_value::<UserModel>(value.clone()) {
                    Ok(user_model) => {
                        let hash = user_model_hash(&user_model);
                        let model_value = serde_json::to_value(&user_model)
                            .expect("UserModel serialization should be infallible");
                        let mut payload = Payload::new();
                        payload.insert("kind".to_string(), Value::String("context".to_string()));
                        payload.insert("source".to_string(), Value::String("user_model".to_string()));
                        payload.insert("user_model".to_string(), model_value);
                        payload.insert("user_model_hash".to_string(), Value::String(hash.clone()));
                        // Deterministic revision/op ids so that replay or fork of
                        // an identical (agent_id, user_model_hash, created_at) input
                        // produces the same scratchpad state -- and therefore the
                        // same `state_hash_after` -- as the original apply. Without
                        // these the UUID-based generators in `ScratchpadDocument`
                        // would drift the binding state hash on every replay,
                        // breaking deterministic binding receipts for mounted
                        // user-model runs. See PR #72 P1.
                        let id_seed = stable_value_hash(&json!([
                            "memory_scope.mounted.user_model",
                            &binding.identity.agent_id,
                            &hash,
                            &transition.created_at,
                        ]));
                        let revision_id = format!("scratchrev:mount:{id_seed}");
                        let op_id = format!("scratchop:mount:{id_seed}");
                        binding
                            .working_memory_scope
                            .scratchpad
                            .append_with_deterministic_ids(
                                "binding:mount",
                                "user model mounted",
                                hash.clone(),
                                payload,
                                revision_id,
                                op_id,
                                transition.created_at.clone(),
                            );
                        transition
                            .payload
                            .insert("user_model_hash".to_string(), Value::String(hash));
                    }
                    Err(error) => {
                        return Err(guard_violation(
                            "invalid_user_model_payload",
                            format!(
                                "MEMORY_SCOPE.MOUNTED user_model field could not be parsed: {error}"
                            ),
                            "user_model",
                            "invalid_user_model",
                            vec!["user_model".to_string()],
                            Payload::new(),
                        ));
                    }
                }
            }
            // S3: when the runtime threads lineage memory into the mount
            // payload, project each entry as a scratchpad Context revision
            // before the binding's heads start contributing. The revisions are
            // attributed to a synthetic `lineage:agent_published` actor (a
            // kernel-injected source, not a head contribution) so the binding's
            // head/budget guards do not fire. Empty / absent lineage is a
            // no-op, which preserves the legacy MOUNTED behaviour. S2's
            // user_model path and S3's lineage_memory path are independent:
            // both fire when both payloads are present.
            let entries = payload_lineage_memory_entries(transition.payload.get("lineage_memory"));
            for entry in entries {
                let payload = entry.into_payload();
                let content_hash = stable_value_hash(&Value::Object(payload.clone()));
                binding.working_memory_scope.scratchpad.append(
                    "lineage:agent_published",
                    "lineage agent_published memory",
                    content_hash,
                    payload,
                    transition.created_at.clone(),
                );
            }
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
        "SYNTHESIS.VERIFIED" => {
            let verifier_head_id = payload_to_string(transition.payload.get("verifier_head_id"));
            binding
                .trace_scope
                .verification_receipts
                .push(BindingVerificationReceipt {
                    verification_id: payload_to_string(transition.payload.get("verification_id")),
                    synthesis_id: payload_to_string(transition.payload.get("synthesis_id")),
                    verifier_head_id: verifier_head_id.clone(),
                    target_revision_id: payload_to_string(
                        transition.payload.get("target_revision_id"),
                    ),
                    outcome: payload_verification_outcome(transition.payload.get("outcome")),
                    attempted_failure_modes: payload_array_strings(
                        transition.payload.get("attempted_failure_modes"),
                    ),
                    commands_run: payload_array_strings(transition.payload.get("commands_run")),
                    receipt_hash: payload_to_string(transition.payload.get("receipt_hash")),
                    created_at: transition.created_at.clone(),
                });
            apply_contribution_charge(
                &mut binding.budget_state,
                &verifier_head_id,
                payload_f64(transition.payload.get("cost_units")),
            );
        }
        "OUTCOME.RECORDED" => {
            let outcomes = payload_head_outcomes(transition.payload.get("head_outcomes"));
            if !outcomes.is_empty() {
                binding.record_head_outcomes(outcomes)?;
            }
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
        "SYNTHESIS.VERIFIED" => {
            let verifier_head_id = payload_to_string(transition.payload.get("verifier_head_id"));
            let head = binding.head(&verifier_head_id).ok_or_else(|| {
                guard_violation(
                    "unknown_verifier_head",
                    format!("verifier head {verifier_head_id} is not registered"),
                    "",
                    binding.lifecycle.status.clone(),
                    Vec::new(),
                    Payload::new(),
                )
            })?;
            if !binding.active_head_ids().contains(&verifier_head_id) {
                return Err(guard_violation(
                    "inactive_verifier_head",
                    format!("verifier head {verifier_head_id} is not active in this binding"),
                    "",
                    binding.lifecycle.status.clone(),
                    Vec::new(),
                    Payload::new(),
                ));
            }
            if head.kind == HeadKind::SkillPlugin {
                return Err(guard_violation(
                    "verification_plugin_denied",
                    "skill plugins cannot verify binding synthesis",
                    "reasoning_core_or_specialized_coder_or_verifier",
                    "skill_plugin",
                    Vec::new(),
                    Payload::new(),
                ));
            }
            if payload_array_strings(transition.payload.get("attempted_failure_modes")).is_empty() {
                return Err(guard_violation(
                    "synthesis_verification_missing_falsification",
                    "SYNTHESIS.VERIFIED requires at least one attempted failure mode",
                    "falsification_attempt",
                    "lgtm",
                    Vec::new(),
                    Payload::new(),
                ));
            }
            let target_revision_id =
                payload_to_string(transition.payload.get("target_revision_id"));
            let known = binding
                .working_memory_scope
                .scratchpad
                .revisions
                .iter()
                .any(|revision| revision.revision_id == target_revision_id);
            if !known {
                return Err(guard_violation(
                    "unknown_synthesis_revision",
                    "SYNTHESIS.VERIFIED references an unknown scratchpad revision",
                    "known_scratchpad_revision",
                    "unknown_scratchpad_revision",
                    vec!["target_revision_id".to_string()],
                    Payload::new(),
                ));
            }
            check_contribution_budget(
                &binding.budget_scope,
                &binding.budget_state,
                &verifier_head_id,
                payload_f64(transition.payload.get("cost_units")),
            )?;
        }
        "PUBLICATION.PROPOSED" if binding.lifecycle.status == "synthesis_verified" => {
            let accepted = binding
                .trace_scope
                .verification_receipts
                .last()
                .is_some_and(|receipt| receipt.outcome == BindingVerificationOutcome::Accepted);
            if !accepted {
                return Err(guard_violation(
                    "synthesis_verification_failed",
                    "publication requires the latest synthesis verification to accept the draft",
                    "accepted_verification",
                    "unaccepted_verification",
                    Vec::new(),
                    Payload::new(),
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
        "SYNTHESIS.VERIFIED" => &[
            "verification_id",
            "synthesis_id",
            "verifier_head_id",
            "target_revision_id",
            "outcome",
            "attempted_failure_modes",
        ],
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
        "SYNTHESIS.VERIFIED" => &["drafts_synthesized", "synthesis_verified"],
        "PUBLICATION.PROPOSED" => &[
            "drafts_synthesized",
            "synthesis_verified",
            "publication_proposed",
        ],
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
        "SYNTHESIS.VERIFIED" => "synthesis_verified",
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
        policy_layer: String::new(),
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

fn routing_decision_for(
    head: &AgentHead,
    subtask: &BindingSubtask,
    explored: bool,
) -> BindingRoutingDecision {
    BindingRoutingDecision {
        subtask_id: subtask.subtask_id.clone(),
        capability: subtask.capability.clone(),
        domain: subtask.domain.clone(),
        head_id: head.head_id.clone(),
        posterior_success_rate: head
            .reliability_profile
            .reliability_for(&subtask.capability, &subtask.domain),
        explored,
    }
}

fn routing_candidate_is_better(
    candidate: &AgentHead,
    incumbent: &AgentHead,
    subtask: &BindingSubtask,
) -> bool {
    let candidate_score = candidate
        .reliability_profile
        .reliability_for(&subtask.capability, &subtask.domain);
    let incumbent_score = incumbent
        .reliability_profile
        .reliability_for(&subtask.capability, &subtask.domain);
    if candidate_score > incumbent_score {
        return true;
    }
    if candidate_score < incumbent_score {
        return false;
    }

    let candidate_cost =
        candidate.cost_profile.input_unit_cost + candidate.cost_profile.output_unit_cost;
    let incumbent_cost =
        incumbent.cost_profile.input_unit_cost + incumbent.cost_profile.output_unit_cost;
    if candidate_cost < incumbent_cost {
        return true;
    }
    if candidate_cost > incumbent_cost {
        return false;
    }

    if candidate.reliability_profile.median_latency_ms
        < incumbent.reliability_profile.median_latency_ms
    {
        return true;
    }
    if candidate.reliability_profile.median_latency_ms
        > incumbent.reliability_profile.median_latency_ms
    {
        return false;
    }

    candidate.head_id < incumbent.head_id
}

fn remaining_run_budget_units(scope: &BindingBudgetScope, state: &BindingBudgetState) -> f64 {
    let run_cap = if scope.allocated_run_budget_units > 0.0 {
        scope.allocated_run_budget_units
    } else {
        scope.shared_budget_units
    };
    (run_cap - state.spent_total).max(0.0)
}

/// A single lineage memory record threaded into `MEMORY_SCOPE.MOUNTED` so a
/// new binding inherits Context from the agent's prior published memory. The
/// runtime computes these from the `binding_lineage` walk and the kernel
/// injects each as a scratchpad revision; no field is required, which keeps
/// the legacy `{scope_id, scratchpad_id}` MOUNTED payload byte-compatible.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BindingLineageMemoryEntry {
    #[serde(default)]
    pub source_binding_id: String,
    #[serde(default)]
    pub source_composition_hash: String,
    #[serde(default)]
    pub source_version: u32,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub patch_ids: Vec<String>,
    #[serde(default)]
    pub substrate_receipt_id: String,
    #[serde(default)]
    pub published_at: String,
}

impl BindingLineageMemoryEntry {
    pub fn into_payload(self) -> Payload {
        let mut payload = Payload::new();
        payload.insert("kind".to_string(), Value::String("lineage_memory".to_string()));
        payload.insert(
            "source_binding_id".to_string(),
            Value::String(self.source_binding_id),
        );
        payload.insert(
            "source_composition_hash".to_string(),
            Value::String(self.source_composition_hash),
        );
        payload.insert(
            "source_version".to_string(),
            Value::Number(self.source_version.into()),
        );
        payload.insert("summary".to_string(), Value::String(self.summary));
        payload.insert(
            "patch_ids".to_string(),
            Value::Array(self.patch_ids.into_iter().map(Value::String).collect()),
        );
        payload.insert(
            "substrate_receipt_id".to_string(),
            Value::String(self.substrate_receipt_id),
        );
        payload.insert("published_at".to_string(), Value::String(self.published_at));
        payload
    }
}

fn payload_lineage_memory_entries(value: Option<&Value>) -> Vec<BindingLineageMemoryEntry> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| serde_json::from_value::<BindingLineageMemoryEntry>(item.clone()).ok())
            // `BindingLineageMemoryEntry` carries `#[serde(default)]` on every
            // field so it round-trips legacy payloads, but that means an empty
            // `{}` element parses into a default-everything entry. Reject
            // entries missing the join key (`source_binding_id`) OR any
            // concrete payload signal (`substrate_receipt_id` / `patch_ids`).
            // Without this guard, MOUNTED would synthesize empty
            // `lineage:agent_published` revisions for stray empty items.
            .filter(|entry| {
                !entry.source_binding_id.trim().is_empty()
                    && (!entry.substrate_receipt_id.trim().is_empty()
                        || !entry.patch_ids.is_empty())
            })
            .collect(),
        _ => Vec::new(),
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

fn payload_verification_outcome(value: Option<&Value>) -> BindingVerificationOutcome {
    match payload_to_string(value).as_str() {
        "accepted" => BindingVerificationOutcome::Accepted,
        "defect_found" => BindingVerificationOutcome::DefectFound,
        _ => BindingVerificationOutcome::Rejected,
    }
}

fn payload_head_outcomes(value: Option<&Value>) -> Vec<BindingHeadOutcome> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| {
                let map = item.as_object()?;
                let head_id = payload_to_string(map.get("head_id"));
                let capability = payload_to_string(map.get("capability"));
                if head_id.trim().is_empty() || capability.trim().is_empty() {
                    return None;
                }
                let outcome_hash = payload_to_string(map.get("outcome_hash"));
                Some(BindingHeadOutcome::new(
                    head_id,
                    capability,
                    payload_to_string(map.get("domain")),
                    payload_bool(map.get("accepted")),
                    if outcome_hash.is_empty() {
                        stable_value_hash(item)
                    } else {
                        outcome_hash
                    },
                ))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn scratchpad_region_for_payload(payload: &Payload) -> String {
    match payload.get("kind").and_then(Value::as_str) {
        Some("proposal") => "proposal",
        Some("critique") => "critique",
        Some("synthesis") => "synthesis",
        Some("verification") => "verification",
        Some("orientation") => "orientation",
        _ => "synthesis",
    }
    .to_string()
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

fn clean_strings_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut cleaned = Vec::new();
    for value in values {
        let value = value.trim().to_string();
        if value.is_empty() || !seen.insert(value.clone()) {
            continue;
        }
        cleaned.push(value);
    }
    cleaned
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
    fn scratchpad_revision_can_merge_and_annotate_prior_work_as_a_dag() {
        let mut binding = fixture_binding();
        let proposal = binding
            .append_scratchpad_revision(
                "claude",
                "proposal",
                "hash:proposal",
                object_payload(json!({ "kind": "proposal" })),
                "2026-06-02T00:00:00Z",
            )
            .unwrap();
        let critique = binding
            .append_scratchpad_revision(
                "deepseek",
                "critique",
                "hash:critique",
                object_payload(json!({ "kind": "critique" })),
                "2026-06-02T00:00:01Z",
            )
            .unwrap();
        let synthesis = binding
            .append_scratchpad_revision_with_links(
                "claude",
                "synthesis",
                "hash:synthesis",
                object_payload(json!({ "kind": "synthesis" })),
                vec![proposal.revision_id.clone(), critique.revision_id.clone()],
                vec![
                    ScratchpadRevisionLink::new(
                        proposal.revision_id.clone(),
                        ScratchpadRelationKind::Supersedes,
                        "synthesis supersedes proposal",
                        Payload::new(),
                    ),
                    ScratchpadRevisionLink::new(
                        critique.revision_id.clone(),
                        ScratchpadRelationKind::Supports,
                        "critique supported synthesis",
                        Payload::new(),
                    ),
                ],
                "2026-06-02T00:00:02Z",
            )
            .unwrap();

        assert_eq!(
            synthesis.parent_revision_ids,
            vec![proposal.revision_id, critique.revision_id]
        );
        assert_eq!(binding.working_memory_scope.scratchpad.relations.len(), 2);
        assert_eq!(
            binding.working_memory_scope.scratchpad.relations[0].relation_kind,
            ScratchpadRelationKind::Supersedes
        );
    }

    #[test]
    fn routing_uses_per_capability_reliability_before_static_head_order() {
        let mut binding = fixture_binding();
        binding
            .composition
            .heads
            .iter_mut()
            .find(|head| head.head_id == "deepseek")
            .unwrap()
            .reliability_profile
            .capability_scores
            .push(HeadCapabilityReliability::new("rust_impl", "harness", 9, 1));
        binding
            .composition
            .heads
            .iter_mut()
            .find(|head| head.head_id == "claude")
            .unwrap()
            .reliability_profile
            .capability_scores
            .push(HeadCapabilityReliability::new("rust_impl", "harness", 1, 9));

        let decision = binding
            .route_subtask(&BindingSubtask::new("task:1", "rust_impl", "harness"), 999)
            .unwrap();

        assert_eq!(decision.head_id, "deepseek");
        assert!(decision.posterior_success_rate > 0.8);
        assert!(!decision.explored);
    }

    #[test]
    fn value_aware_budget_abstains_when_marginal_value_is_below_cost() {
        let mut binding = ready_for_contribution();
        binding
            .composition
            .heads
            .iter_mut()
            .find(|head| head.head_id == "claude")
            .unwrap()
            .reliability_profile
            .capability_scores
            .push(HeadCapabilityReliability::new("verification", "rust", 1, 9));

        let decision = binding
            .value_aware_budget_decision(
                &BindingSubtask::new("round:2:verification", "verification", "rust"),
                "claude",
                2,
                0.95,
                10.0,
                1.0,
            )
            .unwrap();

        assert!(!decision.should_invoke);
        assert_eq!(decision.reason, "marginal_value_below_cost");
    }

    #[test]
    fn outcome_recorded_compounds_per_head_capability_reliability() {
        let binding = apply(
            ready_for_outcome_recorded(),
            "OUTCOME.RECORDED",
            json!({
                "outcome_id": "outcome:1",
                "accepted": true,
                "summary": "published",
                "head_outcomes": [{
                    "head_id": "claude",
                    "capability": "proposal",
                    "domain": "rust",
                    "accepted": true,
                    "outcome_hash": "hash:proposal:accepted"
                }, {
                    "head_id": "deepseek",
                    "capability": "synthesis",
                    "domain": "rust",
                    "accepted": false,
                    "outcome_hash": "hash:synthesis:defect"
                }]
            }),
        )
        .binding;

        let claude = binding.head("claude").unwrap();
        assert_eq!(claude.reliability_profile.success_rate, 1.0);
        assert_eq!(
            claude
                .reliability_profile
                .reliability_for("proposal", "rust"),
            2.0_f32 / 3.0
        );
        let deepseek = binding.head("deepseek").unwrap();
        assert_eq!(deepseek.reliability_profile.success_rate, 0.0);
        assert_eq!(
            deepseek
                .reliability_profile
                .reliability_for("synthesis", "rust"),
            1.0_f32 / 3.0
        );
        assert_eq!(
            binding.identity.composition_hash,
            composition_hash(&binding)
        );
    }

    #[test]
    fn synthesis_verification_requires_falsification_before_publication() {
        let mut binding = ready_for_synthesis_verification();
        let synthesis_revision = binding
            .append_scratchpad_revision(
                "claude",
                "synthesis",
                "hash:synthesis",
                object_payload(json!({ "kind": "synthesis" })),
                "2026-06-02T00:00:00Z",
            )
            .unwrap();
        let error = apply_binding_transition(
            binding,
            transition(
                "SYNTHESIS.VERIFIED",
                json!({
                    "verification_id": "verify:1",
                    "synthesis_id": "synth:1",
                    "verifier_head_id": "deepseek",
                    "target_revision_id": synthesis_revision.revision_id,
                    "outcome": "accepted",
                    "attempted_failure_modes": [""]
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "synthesis_verification_missing_falsification");
    }

    #[test]
    fn accepted_synthesis_verification_is_recorded_on_the_trace() {
        let mut binding = ready_for_synthesis_verification();
        let synthesis_revision = binding
            .append_scratchpad_revision(
                "claude",
                "synthesis",
                "hash:synthesis",
                object_payload(json!({ "kind": "synthesis" })),
                "2026-06-02T00:00:00Z",
            )
            .unwrap();
        let verified = apply(
            binding,
            "SYNTHESIS.VERIFIED",
            json!({
                "verification_id": "verify:1",
                "synthesis_id": "synth:1",
                "verifier_head_id": "deepseek",
                "target_revision_id": synthesis_revision.revision_id,
                "outcome": "accepted",
                "attempted_failure_modes": ["counterexample pass"],
                "commands_run": ["cargo test -p theorem-harness-core"],
                "cost_units": 1.0
            }),
        );

        assert_eq!(verified.binding.lifecycle.status, "synthesis_verified");
        assert_eq!(verified.binding.trace_scope.verification_receipts.len(), 1);
        assert_eq!(
            verified.binding.trace_scope.verification_receipts[0].outcome,
            BindingVerificationOutcome::Accepted
        );
        assert_eq!(verified.binding.budget_state.spent_total, 1.0);
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
            ready_for_synthesis_verification(),
            "PUBLICATION.PROPOSED",
            json!({
                "publication_id": "pub:1",
                "draft_hash": "draft:1"
            }),
        );
        binding.binding
    }

    fn ready_for_outcome_recorded() -> AgentBinding {
        let binding = apply(
            ready_for_publication(),
            "POLICY.CHECKED",
            json!({
                "policy_receipt_id": "policy:1",
                "allowed": true,
                "claims": [{ "text": "grounded", "provenance": "source:1" }]
            }),
        );
        apply(
            binding.binding,
            "PUBLISHED_TO_SUBSTRATE",
            json!({
                "publication_id": "pub:1",
                "substrate_receipt_id": "substrate:1"
            }),
        )
        .binding
    }

    fn ready_for_synthesis_verification() -> AgentBinding {
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
        binding.binding
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
                agent_constitution: None,
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

    // ---- S3: MOUNTED arm projects lineage memory as Context revisions ----

    #[test]
    fn mounted_with_lineage_memory_appends_scratchpad_context_revisions() {
        let binding = fixture_binding();
        let binding = apply(
            binding,
            "BINDING.RESOLVED",
            json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
        );
        let binding = apply(
            binding.binding,
            "HEADS.PROBED",
            json!({ "probed_head_set": ["claude", "deepseek", "mistral_ocr"] }),
        );
        let revisions_before = binding.binding.working_memory_scope.scratchpad.revisions.len();
        let mounted = apply(
            binding.binding,
            "MEMORY_SCOPE.MOUNTED",
            json!({
                "scope_id": "bindingscope:theorem",
                "scratchpad_id": "scratchpad:theorem",
                "lineage_size": 2,
                "lineage_memory": [
                    {
                        "source_binding_id": "harness:binding:agent:theorem:v1",
                        "source_composition_hash": "hash:v1",
                        "source_version": 1,
                        "summary": "prior v1 published",
                        "patch_ids": ["patch:1", "patch:2"],
                        "substrate_receipt_id": "substrate:v1",
                        "published_at": "2026-06-01T00:00:00Z"
                    },
                    {
                        "source_binding_id": "harness:binding:agent:theorem:v2",
                        "source_composition_hash": "hash:v2",
                        "source_version": 2,
                        "summary": "prior v2 published",
                        "patch_ids": ["patch:3"],
                        "substrate_receipt_id": "substrate:v2",
                        "published_at": "2026-06-02T00:00:00Z"
                    }
                ]
            }),
        );
        let scratchpad = &mounted.binding.working_memory_scope.scratchpad;
        let lineage_revs = scratchpad
            .revisions
            .iter()
            .filter(|revision| revision.actor_head_id == "lineage:agent_published")
            .collect::<Vec<_>>();
        assert_eq!(
            lineage_revs.len(),
            2,
            "MOUNTED with lineage_memory must append one revision per entry"
        );
        assert_eq!(
            scratchpad.revisions.len(),
            revisions_before + 2,
            "no other revisions should be touched"
        );
        // Order is preserved: array index N maps to revision N (in append order).
        assert_eq!(
            lineage_revs[0]
                .payload
                .get("source_version")
                .and_then(Value::as_u64),
            Some(1)
        );
        assert_eq!(
            lineage_revs[0]
                .payload
                .get("substrate_receipt_id")
                .and_then(Value::as_str),
            Some("substrate:v1")
        );
        assert_eq!(
            lineage_revs[1]
                .payload
                .get("source_version")
                .and_then(Value::as_u64),
            Some(2)
        );
        assert_eq!(
            lineage_revs[1]
                .payload
                .get("source_binding_id")
                .and_then(Value::as_str),
            Some("harness:binding:agent:theorem:v2")
        );
    }

    #[test]
    fn mounted_without_lineage_memory_leaves_scratchpad_untouched() {
        let binding = fixture_binding();
        let binding = apply(
            binding,
            "BINDING.RESOLVED",
            json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
        );
        let binding = apply(
            binding.binding,
            "HEADS.PROBED",
            json!({ "probed_head_set": ["claude", "deepseek", "mistral_ocr"] }),
        );
        let revisions_before = binding.binding.working_memory_scope.scratchpad.revisions.len();
        let mounted = apply(
            binding.binding,
            "MEMORY_SCOPE.MOUNTED",
            json!({
                "scope_id": "bindingscope:theorem",
                "scratchpad_id": "scratchpad:theorem"
            }),
        );
        assert_eq!(
            mounted.binding.working_memory_scope.scratchpad.revisions.len(),
            revisions_before,
            "legacy MOUNTED payload must not synthesize any scratchpad revisions"
        );
        assert_eq!(mounted.binding.lifecycle.status, "memory_scope_mounted");
    }

    // ---- F1: payload_lineage_memory_entries filters synthetic empties ----

    #[test]
    fn payload_lineage_memory_entries_filters_empty_and_keyless_entries() {
        let array = json!([
            // Pure empty entry (every field defaults via serde(default)).
            {},
            // Missing the join key entirely.
            {"source_binding_id": "", "patch_ids": ["patch:1"]},
            // Has join key but no payload signal at all.
            {"source_binding_id": "b1", "patch_ids": [], "substrate_receipt_id": ""},
            // Valid: join key + a substrate receipt.
            {
                "source_binding_id": "b-keep-receipt",
                "substrate_receipt_id": "substrate:1",
                "patch_ids": []
            },
            // Valid: join key + patch ids.
            {
                "source_binding_id": "b-keep-patches",
                "substrate_receipt_id": "",
                "patch_ids": ["patch:1"]
            }
        ]);
        let entries = payload_lineage_memory_entries(Some(&array));
        assert_eq!(
            entries.len(),
            2,
            "only entries with join key AND a payload signal must survive"
        );
        assert_eq!(entries[0].source_binding_id, "b-keep-receipt");
        assert_eq!(entries[1].source_binding_id, "b-keep-patches");
    }
}
