//! Fake-first invocation seam for resolved composed-agent heads.
//!
//! The core crate cannot know how to call Anthropic, local models, MCP tools,
//! or hosted coders. It only needs a narrow contract: given a resolved head and
//! an invocation kind, produce a structured receipt that the binding loop can
//! append to the scratchpad and charge through `HEADS.CONTRIBUTE`.

use crate::agent_binding::HeadKind;
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
}

impl HeadInvocationKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Proposal => "proposal",
            Self::Critique => "critique",
            Self::Synthesis => "synthesis",
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
    pub task: String,
    #[serde(default)]
    pub scratchpad_version: u64,
    #[serde(default)]
    pub prior_revision_ids: Vec<String>,
    #[serde(default)]
    pub prior_context: Vec<RevisionContext>,
    #[serde(default)]
    pub claims: Vec<GroundedClaim>,
    #[serde(default)]
    pub policy_decision: Option<PolicyDecision>,
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
        let mut request = Self {
            invocation_id: String::new(),
            head,
            kind,
            task: task.into(),
            scratchpad_version,
            prior_revision_ids,
            prior_context,
            claims,
            policy_decision: None,
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

    pub fn computed_invocation_id(&self) -> String {
        format!(
            "headinvoke:{}",
            stable_value_hash(&json!({
                "head_id": self.head.head_id,
                "kind": self.kind,
                "task": self.task,
                "scratchpad_version": self.scratchpad_version,
                "prior_revision_ids": self.prior_revision_ids,
                "prior_context": self.prior_context,
                "claims": self.claims,
                "policy_decision": self.policy_decision,
                "created_at": self.created_at,
            }))
        )
    }
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
        };
        let payload = object_payload(json!({
            "fake": true,
            "kind": request.kind.as_str(),
            "head_id": request.head.head_id,
            "task": request.task,
            "scratchpad_version": request.scratchpad_version,
            "prior_revision_ids": request.prior_revision_ids,
            "prior_context": request.prior_context,
            "claims": request.claims,
        }));

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
