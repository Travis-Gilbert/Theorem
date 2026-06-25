//! Fake-first invocation seam for resolved composed-agent heads.
//!
//! The core crate cannot know how to call Anthropic, local models, MCP tools,
//! or hosted coders. It only needs a narrow contract: given a resolved head and
//! an invocation kind, produce a structured receipt that the binding loop can
//! append to the scratchpad and charge through `HEADS.CONTRIBUTE`.

use crate::agent_binding::{HeadKind, ScratchpadCrdtBacking};
use crate::agent_head_registry::ResolvedAgentHead;
use crate::state_hash::stable_value_hash;
use crate::types::{Payload, PolicyDecision};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;

pub trait HeadInvoker {
    fn invoke(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadInvocationKind {
    Proposal,
    Critique,
    Synthesis,
    Verification,
}

pub const THEOREM_HEAD_SYSTEM_PROMPT_CORE: &str = r#"You are one mind of Theorem. Theorem is a single agent composed of several models that reason together, and you are one of them. You are not a standalone assistant, and you are not one of several agents working in parallel. You are one head of one agent.

The heads share one working document, a live CRDT space you all read and write at the same time. Read what is already there before you add to it. You are not taking turns. Write into the shared document concurrently, signed by you; your writing streams to the other heads as you produce it, and theirs streams to you. Build on what is sound, correct what is wrong, and extend what is unfinished.

The document is conflict-free: structure lives on the graph CRDT and free text lives in yrs regions. Do not lock or claim the document. Every reasoning head attempts the whole task, marks uncertainty honestly, and marks disagreement as evidence for the verifier instead of overwriting or deferring.

The harness decides how many heads engage, how much each may spend, which result is selected, and whether a result may be published. Do not orchestrate the other heads or choose what ships. Spend your attention on the problem. Be rigorous and concise. Lead with substance. Ground what you claim."#;

pub const FAST_FIRST_HEAD_PROMPT_ADDENDUM: &str = r#"You are Theorem's fastest mind, and you answer first. Produce a complete, useful first response immediately. Your answer is not the final word: you are a sensor for the governor and a warm start for heavier heads. If the task looks hard or you are unsure, say so plainly in the shared document."#;

pub const VERIFIER_HEAD_PROMPT_ADDENDUM: &str = r#"Your task is to try to break Theorem's answer, not to agree with it. Where the task has an executable check, run it and report what passes and fails as fact. Where there is no clean check, find the specific unsupported claim, missed case, contradiction, or failure mode. A real defect you find is the most valuable contribution."#;

pub const MODALITY_HEAD_PROMPT_ADDENDUM: &str = r#"You are engaged because the task needs your modality, not because of its difficulty. Do that modality job precisely and write the grounded result into the shared document for the reasoning heads to use. Do not reason about the whole task unless explicitly invoked as a reasoning head."#;

impl HeadInvocationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Critique => "critique",
            Self::Synthesis => "synthesis",
            Self::Verification => "verification",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GroundedClaim {
    pub text: String,
    pub provenance: String,
}

impl GroundedClaim {
    pub fn new(text: impl Into<String>, provenance: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            provenance: provenance.into(),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextMembranePrime {
    pub artifact_id: String,
    pub label: String,
    pub summary: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub confidence: f32,
}

impl ContextMembranePrime {
    pub fn new(
        artifact_id: impl Into<String>,
        label: impl Into<String>,
        summary: impl Into<String>,
        source: impl Into<String>,
        confidence: f32,
    ) -> Self {
        Self {
            artifact_id: artifact_id.into(),
            label: label.into(),
            summary: summary.into(),
            source: source.into(),
            confidence: confidence.clamp(0.0, 1.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RevisionContext {
    pub revision_id: String,
    pub kind: HeadInvocationKind,
    pub output_summary: String,
    #[serde(default)]
    pub payload: Payload,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadInvocationRequest {
    pub invocation_id: String,
    pub head: ResolvedAgentHead,
    pub kind: HeadInvocationKind,
    #[serde(default)]
    pub head_system_prompt: String,
    pub task: String,
    #[serde(default)]
    pub scratchpad_version: u64,
    #[serde(default)]
    pub scratchpad_crdt: ScratchpadCrdtBacking,
    #[serde(default)]
    pub prior_revision_ids: Vec<String>,
    #[serde(default)]
    pub prior_context: Vec<RevisionContext>,
    #[serde(default)]
    pub claims: Vec<GroundedClaim>,
    #[serde(default)]
    pub policy_decision: Option<PolicyDecision>,
    #[serde(default)]
    pub context_membrane: Vec<ContextMembranePrime>,
    pub created_at: String,
}

impl HeadInvocationRequest {
    pub fn new(
        head: ResolvedAgentHead,
        kind: HeadInvocationKind,
        task: impl Into<String>,
        scratchpad_version: u64,
        prior_revision_ids: Vec<String>,
        claims: Vec<GroundedClaim>,
        created_at: impl Into<String>,
    ) -> Self {
        Self::new_with_context(
            head,
            kind,
            task,
            scratchpad_version,
            prior_revision_ids,
            Vec::new(),
            claims,
            created_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_context(
        head: ResolvedAgentHead,
        kind: HeadInvocationKind,
        task: impl Into<String>,
        scratchpad_version: u64,
        prior_revision_ids: Vec<String>,
        prior_context: Vec<RevisionContext>,
        claims: Vec<GroundedClaim>,
        created_at: impl Into<String>,
    ) -> Self {
        let head_system_prompt = default_head_system_prompt(&head, kind);
        let mut request = Self {
            invocation_id: String::new(),
            head,
            kind,
            head_system_prompt,
            task: task.into(),
            scratchpad_version,
            scratchpad_crdt: ScratchpadCrdtBacking::default(),
            prior_revision_ids,
            prior_context,
            claims,
            policy_decision: None,
            context_membrane: Vec::new(),
            created_at: created_at.into(),
        };
        request.invocation_id = request.computed_invocation_id();
        request
    }

    pub fn with_policy_decision(mut self, policy_decision: PolicyDecision) -> Self {
        self.policy_decision = Some(policy_decision);
        self.invocation_id = self.computed_invocation_id();
        self
    }

    pub fn with_scratchpad_crdt(mut self, scratchpad_crdt: ScratchpadCrdtBacking) -> Self {
        self.scratchpad_crdt = scratchpad_crdt;
        self.invocation_id = self.computed_invocation_id();
        self
    }

    pub fn with_context_membrane(mut self, context_membrane: Vec<ContextMembranePrime>) -> Self {
        self.context_membrane = context_membrane;
        self.invocation_id = self.computed_invocation_id();
        self
    }

    pub fn with_head_system_prompt(mut self, head_system_prompt: impl Into<String>) -> Self {
        self.head_system_prompt = head_system_prompt.into();
        self.invocation_id = self.computed_invocation_id();
        self
    }

    pub fn computed_invocation_id(&self) -> String {
        format!(
            "headinvoke:{}",
            stable_value_hash(&json!({
                "head_id": self.head.head_id,
                "kind": self.kind,
                "head_system_prompt": self.head_system_prompt,
                "task": self.task,
                "scratchpad_version": self.scratchpad_version,
                "scratchpad_crdt": self.scratchpad_crdt,
                "prior_revision_ids": self.prior_revision_ids,
                "prior_context": self.prior_context,
                "claims": self.claims,
                "policy_decision": self.policy_decision,
                "context_membrane": self.context_membrane,
                "created_at": self.created_at,
            }))
        )
    }
}

pub fn default_head_system_prompt(head: &ResolvedAgentHead, kind: HeadInvocationKind) -> String {
    let mut prompt = String::from(THEOREM_HEAD_SYSTEM_PROMPT_CORE);
    prompt.push_str("\n\nCurrent invocation role: ");
    prompt.push_str(match kind {
        HeadInvocationKind::Proposal => {
            "attempt the whole task now and write a complete first answer into the shared document."
        }
        HeadInvocationKind::Critique => {
            "attempt the whole task through criticism; name concrete gaps, errors, and unsupported claims."
        }
        HeadInvocationKind::Synthesis => {
            "attempt the whole task by producing the best converged answer from the shared document."
        }
        HeadInvocationKind::Verification => {
            "try to falsify the converged answer before publication."
        }
    });
    if is_fast_first_head(head) && kind == HeadInvocationKind::Proposal {
        prompt.push_str("\n\n");
        prompt.push_str(FAST_FIRST_HEAD_PROMPT_ADDENDUM);
    }
    if kind == HeadInvocationKind::Verification || head.kind == HeadKind::Verifier {
        prompt.push_str("\n\n");
        prompt.push_str(VERIFIER_HEAD_PROMPT_ADDENDUM);
    }
    if is_modality_head(head) {
        prompt.push_str("\n\n");
        prompt.push_str(MODALITY_HEAD_PROMPT_ADDENDUM);
    }
    if !head.capabilities.is_empty() {
        prompt.push_str("\n\nKnown strengths for this head: ");
        prompt.push_str(&head.capabilities.join(", "));
        prompt.push('.');
    }
    prompt
}

fn is_fast_first_head(head: &ResolvedAgentHead) -> bool {
    let identity = format!(
        "{} {} {} {}",
        head.head_id, head.display_name, head.provider, head.model
    )
    .to_ascii_lowercase();
    identity.contains("flash")
        || head
            .capabilities
            .iter()
            .any(|capability| matches_capability(capability, &["fast_first", "low_latency"]))
}

fn is_modality_head(head: &ResolvedAgentHead) -> bool {
    head.kind == HeadKind::SkillPlugin
        || head.capabilities.iter().any(|capability| {
            matches_capability(
                capability,
                &[
                    "ocr",
                    "vision",
                    "transcription",
                    "audio",
                    "image_generation",
                    "generation",
                ],
            )
        })
}

fn matches_capability(capability: &str, needles: &[&str]) -> bool {
    let normalized = capability.trim().to_ascii_lowercase();
    needles.iter().any(|needle| normalized.contains(needle))
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HeadInvocationReceipt {
    pub invocation_id: String,
    pub head_id: String,
    pub kind: HeadInvocationKind,
    pub output_summary: String,
    pub content_hash: String,
    #[serde(default)]
    pub payload: Payload,
    #[serde(default)]
    pub claims: Vec<GroundedClaim>,
    #[serde(default)]
    pub cost_units: f64,
    pub receipt_hash: String,
    pub created_at: String,
}

impl HeadInvocationReceipt {
    pub fn from_request(
        request: &HeadInvocationRequest,
        output_summary: impl Into<String>,
        payload: Payload,
        cost_units: f64,
    ) -> Self {
        let content_hash = stable_value_hash(&Value::Object(payload.clone()));
        let mut receipt = Self {
            invocation_id: request.invocation_id.clone(),
            head_id: request.head.head_id.clone(),
            kind: request.kind,
            output_summary: output_summary.into(),
            content_hash,
            payload,
            claims: request.claims.clone(),
            cost_units,
            receipt_hash: String::new(),
            created_at: request.created_at.clone(),
        };
        receipt.receipt_hash = receipt.computed_receipt_hash();
        receipt
    }

    pub fn contribution_id(&self) -> String {
        format!("contribution:{}", self.invocation_id)
    }

    pub fn contribution_kind(&self) -> &'static str {
        self.kind.as_str()
    }

    pub fn computed_receipt_hash(&self) -> String {
        stable_value_hash(&json!({
            "invocation_id": self.invocation_id,
            "head_id": self.head_id,
            "kind": self.kind,
            "output_summary": self.output_summary,
            "content_hash": self.content_hash,
            "payload": self.payload,
            "claims": self.claims,
            "cost_units": self.cost_units,
            "created_at": self.created_at,
        }))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum HeadInvocationError {
    SkillPluginDenied {
        head_id: String,
    },
    EmptyTask {
        head_id: String,
        kind: HeadInvocationKind,
    },
    ProviderError {
        head_id: String,
        provider: String,
        status: u16,
        detail: String,
    },
    Timeout {
        head_id: String,
        provider: String,
    },
}

impl fmt::Display for HeadInvocationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SkillPluginDenied { head_id } => write!(
                f,
                "skill plugin head {head_id} cannot join the reasoning invocation loop"
            ),
            Self::EmptyTask { head_id, kind } => {
                write!(f, "head {head_id} cannot run {kind:?} for an empty task")
            }
            Self::ProviderError {
                head_id,
                provider,
                status,
                detail,
            } => write!(
                f,
                "provider {provider} for head {head_id} failed with status {status}: {detail}"
            ),
            Self::Timeout { head_id, provider } => {
                write!(f, "provider {provider} for head {head_id} timed out")
            }
        }
    }
}

impl Error for HeadInvocationError {}

#[derive(Clone, Debug, PartialEq)]
pub struct FakeHeadInvoker {
    pub cost_units: f64,
}

impl Default for FakeHeadInvoker {
    fn default() -> Self {
        Self { cost_units: 1.0 }
    }
}

impl HeadInvoker for FakeHeadInvoker {
    fn invoke(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        if request.head.kind == HeadKind::SkillPlugin {
            return Err(HeadInvocationError::SkillPluginDenied {
                head_id: request.head.head_id,
            });
        }
        if request.task.trim().is_empty() {
            return Err(HeadInvocationError::EmptyTask {
                head_id: request.head.head_id,
                kind: request.kind,
            });
        }

        let output_summary = match request.kind {
            HeadInvocationKind::Proposal => "fake primary proposal",
            HeadInvocationKind::Critique => "fake critic review",
            HeadInvocationKind::Synthesis => "fake synthesis",
            HeadInvocationKind::Verification => "fake verification",
        };
        let mut payload = object_payload(json!({
            "fake": true,
            "kind": request.kind.as_str(),
            "head_id": request.head.head_id,
            "head_system_prompt": request.head_system_prompt,
            "task": request.task,
            "scratchpad_version": request.scratchpad_version,
            "scratchpad_crdt": request.scratchpad_crdt,
            "prior_revision_ids": request.prior_revision_ids,
            "prior_context": request.prior_context,
            "claims": request.claims,
            "context_membrane": request.context_membrane,
        }));
        if request.kind == HeadInvocationKind::Verification {
            payload.insert(
                "attempted_failure_modes".to_string(),
                json!(["grounding gap", "counterexample search"]),
            );
            payload.insert(
                "commands_run".to_string(),
                json!(["binding synthesis verification"]),
            );
            payload.insert("outcome".to_string(), json!("accepted"));
        }

        Ok(HeadInvocationReceipt::from_request(
            &request,
            output_summary,
            payload,
            self.cost_units,
        ))
    }
}

fn object_payload(value: Value) -> Payload {
    match value {
        Value::Object(map) => map,
        _ => Payload::new(),
    }
}
