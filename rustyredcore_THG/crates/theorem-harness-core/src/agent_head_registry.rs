//! Pure AgentHead registry resolution for composed-agent bindings.
//!
//! The registry turns the declarative `AgentBinding` composition plane into a
//! callable endpoint view, but it deliberately stops before provider execution:
//! resolution returns fake transport targets plus credential references only.
//! Runtime adapters can later exchange those references for actual credentials
//! outside the GraphStore-backed binding state.

use crate::agent_binding::{
    AgentBinding, AgentHead, BindingTransitionInput, HeadCostProfile, HeadKind,
    HeadReliabilityProfile, HeadTransport, TraceTier,
};
use crate::types::Payload;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum AgentHeadRegistryError {
    DuplicateHeadId {
        head_id: String,
    },
    UnknownHead {
        head_id: String,
    },
    InactiveHead {
        head_id: String,
    },
    TransportMismatch {
        head_id: String,
        requested: HeadTransport,
        registered: HeadTransport,
    },
    CredentialMaterialRejected {
        head_id: String,
    },
}

impl fmt::Display for AgentHeadRegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateHeadId { head_id } => {
                write!(f, "AgentHead registry contains duplicate head {head_id}")
            }
            Self::UnknownHead { head_id } => {
                write!(f, "AgentHead registry does not contain head {head_id}")
            }
            Self::InactiveHead { head_id } => {
                write!(f, "AgentHead {head_id} is not active for this binding")
            }
            Self::TransportMismatch {
                head_id,
                requested,
                registered,
            } => write!(
                f,
                "AgentHead {head_id} is registered for {registered:?}, not {requested:?}"
            ),
            Self::CredentialMaterialRejected { head_id } => write!(
                f,
                "AgentHead {head_id} appears to contain raw credential material"
            ),
        }
    }
}

impl Error for AgentHeadRegistryError {}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AgentHeadRegistry {
    pub heads: BTreeMap<String, RegisteredAgentHead>,
}

impl AgentHeadRegistry {
    pub fn from_binding(binding: &AgentBinding) -> Result<Self, AgentHeadRegistryError> {
        let active_head_set = binding.active_head_ids();
        let mut heads = BTreeMap::new();
        for head in &binding.composition.heads {
            let registered =
                RegisteredAgentHead::from_head(head, active_head_set.contains(&head.head_id))?;
            if heads
                .insert(registered.head_id.clone(), registered)
                .is_some()
            {
                return Err(AgentHeadRegistryError::DuplicateHeadId {
                    head_id: head.head_id.clone(),
                });
            }
        }
        Ok(Self { heads })
    }

    pub fn resolve(
        &self,
        head_id: &str,
        transport: HeadTransport,
    ) -> Result<ResolvedAgentHead, AgentHeadRegistryError> {
        let head = self
            .heads
            .get(head_id)
            .ok_or_else(|| AgentHeadRegistryError::UnknownHead {
                head_id: head_id.to_string(),
            })?;
        if !head.active {
            return Err(AgentHeadRegistryError::InactiveHead {
                head_id: head_id.to_string(),
            });
        }
        if head.transport != transport {
            return Err(AgentHeadRegistryError::TransportMismatch {
                head_id: head_id.to_string(),
                requested: transport,
                registered: head.transport.clone(),
            });
        }
        Ok(head.resolved())
    }

    pub fn resolve_registered_transport(
        &self,
        head_id: &str,
    ) -> Result<ResolvedAgentHead, AgentHeadRegistryError> {
        let head = self
            .heads
            .get(head_id)
            .ok_or_else(|| AgentHeadRegistryError::UnknownHead {
                head_id: head_id.to_string(),
            })?;
        self.resolve(head_id, head.transport.clone())
    }

    pub fn active_resolved_heads(&self) -> Vec<ResolvedAgentHead> {
        self.heads
            .values()
            .filter(|head| head.active)
            .map(RegisteredAgentHead::resolved)
            .collect()
    }

    pub fn probed_head_set(&self) -> Vec<String> {
        self.heads
            .values()
            .filter(|head| head.active)
            .map(|head| head.head_id.clone())
            .collect()
    }

    pub fn kind_summary(&self) -> AgentHeadKindSummary {
        let mut summary = AgentHeadKindSummary::default();
        for head in self.heads.values().filter(|head| head.active) {
            match head.kind {
                HeadKind::ReasoningCore => summary.reasoning_cores.push(head.head_id.clone()),
                HeadKind::SkillPlugin => summary.skill_plugins.push(head.head_id.clone()),
                HeadKind::SpecializedCoder => summary.specialized_coders.push(head.head_id.clone()),
                HeadKind::Verifier => summary.verifiers.push(head.head_id.clone()),
            }
        }
        summary
    }

    pub fn heads_probed_payload(&self) -> Payload {
        let mut payload = Payload::new();
        payload.insert(
            "probed_head_set".to_string(),
            Value::Array(
                self.probed_head_set()
                    .into_iter()
                    .map(Value::String)
                    .collect(),
            ),
        );
        payload.insert(
            "resolved_heads".to_string(),
            serde_json::to_value(self.active_resolved_heads())
                .expect("resolved heads should serialize"),
        );
        payload.insert(
            "kind_summary".to_string(),
            serde_json::to_value(self.kind_summary()).expect("kind summary should serialize"),
        );
        payload
    }

    pub fn heads_probed_transition(&self) -> BindingTransitionInput {
        BindingTransitionInput::new("HEADS.PROBED", self.heads_probed_payload())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegisteredAgentHead {
    pub head_id: String,
    #[serde(default)]
    pub display_name: String,
    pub provider: String,
    pub model: String,
    pub transport: HeadTransport,
    pub kind: HeadKind,
    pub endpoint: AgentHeadEndpoint,
    pub credential_ref: String,
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
    #[serde(default)]
    pub active: bool,
}

impl RegisteredAgentHead {
    pub fn from_head(head: &AgentHead, active: bool) -> Result<Self, AgentHeadRegistryError> {
        reject_credential_material(&head.head_id, &head.credential_ref)?;
        Ok(Self {
            head_id: head.head_id.clone(),
            display_name: head.display_name.clone(),
            provider: head.provider.clone(),
            model: head.model.clone(),
            transport: head.transport.clone(),
            kind: head.kind.clone(),
            endpoint: AgentHeadEndpoint::fake_for(head),
            credential_ref: head.credential_ref.clone(),
            capabilities: clean_strings(head.capabilities.clone()),
            cost_profile: head.cost_profile.clone(),
            reliability_profile: head.reliability_profile.clone(),
            allowed_tools: clean_strings(head.allowed_tools.clone()),
            trace_tier: head.trace_tier.clone(),
            active,
        })
    }

    fn resolved(&self) -> ResolvedAgentHead {
        ResolvedAgentHead {
            head_id: self.head_id.clone(),
            display_name: self.display_name.clone(),
            provider: self.provider.clone(),
            model: self.model.clone(),
            kind: self.kind.clone(),
            endpoint: self.endpoint.clone(),
            credential_ref: self.credential_ref.clone(),
            capabilities: self.capabilities.clone(),
            cost_profile: self.cost_profile.clone(),
            reliability_profile: self.reliability_profile.clone(),
            allowed_tools: self.allowed_tools.clone(),
            trace_tier: self.trace_tier.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentHeadEndpoint {
    pub transport: HeadTransport,
    pub target: String,
    #[serde(default = "default_true")]
    pub fake: bool,
}

impl AgentHeadEndpoint {
    pub fn fake_for(head: &AgentHead) -> Self {
        let transport = transport_slug(&head.transport);
        Self {
            transport: head.transport.clone(),
            target: format!(
                "fake://{}/{}/{}/{}",
                transport,
                target_part(&head.provider),
                target_part(&head.model),
                target_part(&head.head_id)
            ),
            fake: true,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolvedAgentHead {
    pub head_id: String,
    #[serde(default)]
    pub display_name: String,
    pub provider: String,
    pub model: String,
    pub kind: HeadKind,
    pub endpoint: AgentHeadEndpoint,
    pub credential_ref: String,
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

impl ResolvedAgentHead {
    pub fn manifest(&self) -> Value {
        json!({
            "head_id": self.head_id,
            "display_name": self.display_name,
            "provider": self.provider,
            "model": self.model,
            "kind": self.kind,
            "endpoint": self.endpoint,
            "credential_ref": self.credential_ref,
            "capabilities": self.capabilities,
            "allowed_tools": self.allowed_tools,
            "trace_tier": self.trace_tier,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentHeadKindSummary {
    #[serde(default)]
    pub reasoning_cores: Vec<String>,
    #[serde(default)]
    pub skill_plugins: Vec<String>,
    #[serde(default)]
    pub specialized_coders: Vec<String>,
    #[serde(default)]
    pub verifiers: Vec<String>,
}

fn reject_credential_material(
    head_id: &str,
    credential_ref: &str,
) -> Result<(), AgentHeadRegistryError> {
    let trimmed = credential_ref.trim();
    let lowered = trimmed.to_ascii_lowercase();
    let raw_markers = [
        "sk-",
        "sk_",
        "ghp_",
        "github_pat_",
        "hf_",
        "bearer ",
        "api_key=",
        "apikey=",
        "secret=",
        "-----begin ",
    ];
    let appears_raw = raw_markers
        .iter()
        .any(|marker| lowered.starts_with(marker) || lowered.contains(marker));
    if appears_raw || trimmed.contains('\n') || trimmed.contains('\r') {
        return Err(AgentHeadRegistryError::CredentialMaterialRejected {
            head_id: head_id.to_string(),
        });
    }
    Ok(())
}

fn transport_slug(transport: &HeadTransport) -> &'static str {
    match transport {
        HeadTransport::Api => "api",
        HeadTransport::Mcp => "mcp",
        HeadTransport::Local => "local",
        HeadTransport::Hosted => "hosted",
    }
}

fn target_part(value: &str) -> String {
    value
        .trim()
        .chars()
        .map(|character| match character {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' => character,
            _ => '_',
        })
        .collect()
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

fn default_true() -> bool {
    true
}
