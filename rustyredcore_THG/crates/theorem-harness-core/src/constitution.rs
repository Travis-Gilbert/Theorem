//! Layered constitution receipts for composed-agent turns.
//!
//! This module is intentionally pure: it records the authority order the harness
//! enforces around a turn, while the existing binding guards still perform the
//! hard publication checks in `alignment.rs`.

use crate::agent_binding::AgentBinding;
use crate::head_invocation::{GroundedClaim, HeadInvocationKind};
use crate::state_hash::stable_value_hash;
use crate::types::{PolicyCheck, PolicyDecision, PolicyLayer};
use serde_json::json;

pub const GLOBAL_LAW_LAYER: &str = "global_law";
pub const PROJECT_LAW_LAYER: &str = "project_law";
pub const REQUEST_LAYER: &str = "current_request";
pub const LIVE_EVIDENCE_LAYER: &str = "live_evidence";

pub fn default_authority_order() -> Vec<String> {
    vec![
        GLOBAL_LAW_LAYER.to_string(),
        PROJECT_LAW_LAYER.to_string(),
        REQUEST_LAYER.to_string(),
        LIVE_EVIDENCE_LAYER.to_string(),
    ]
}

#[derive(Clone, Debug, PartialEq)]
pub struct Constitution {
    layers: Vec<PolicyLayer>,
}

impl Constitution {
    pub fn for_binding(binding: &AgentBinding, task: &str, claims: &[GroundedClaim]) -> Self {
        let tier_count = binding.capability_scope.action_tiers.len();
        let claim_count = claims.len();
        Self {
            layers: vec![
                PolicyLayer {
                    layer_id: GLOBAL_LAW_LAYER.to_string(),
                    source: "theorem-harness-core/alignment.rs".to_string(),
                    summary: format!(
                        "epistemic floor active; {tier_count} action-tier policies available"
                    ),
                    authority: 0,
                },
                PolicyLayer {
                    layer_id: PROJECT_LAW_LAYER.to_string(),
                    source: ".theorem/constitution.json".to_string(),
                    summary: "project law layer reserved for caller-supplied policy".to_string(),
                    authority: 1,
                },
                PolicyLayer {
                    layer_id: REQUEST_LAYER.to_string(),
                    source: "current task".to_string(),
                    summary: summarize_request(task),
                    authority: 2,
                },
                PolicyLayer {
                    layer_id: LIVE_EVIDENCE_LAYER.to_string(),
                    source: "turn evidence".to_string(),
                    summary: format!("{claim_count} grounded-claim candidates carried into turn"),
                    authority: 3,
                },
            ],
        }
    }

    pub fn layers(&self) -> &[PolicyLayer] {
        &self.layers
    }

    pub fn authority_order(&self) -> Vec<String> {
        self.layers
            .iter()
            .map(|layer| layer.layer_id.clone())
            .collect()
    }

    pub fn head_turn_decision(
        &self,
        binding: &AgentBinding,
        head_id: &str,
        kind: HeadInvocationKind,
    ) -> PolicyDecision {
        let mut checks = base_checks(binding);
        checks.push(PolicyCheck {
            check_id: "head_turn_scope".to_string(),
            layer_id: REQUEST_LAYER.to_string(),
            status: "allow".to_string(),
            summary: format!(
                "head {head_id} may run {} inside this binding",
                kind.as_str()
            ),
        });
        self.decision("head_turn", true, checks)
    }

    pub fn publication_decision(&self, binding: &AgentBinding) -> PolicyDecision {
        let mut checks = base_checks(binding);
        checks.push(PolicyCheck {
            check_id: "publication_alignment_guard".to_string(),
            layer_id: GLOBAL_LAW_LAYER.to_string(),
            status: "defer_to_binding_guard".to_string(),
            summary: "POLICY.CHECKED still applies consensus, action-tier, and grounding guards"
                .to_string(),
        });
        self.decision("publication", true, checks)
    }

    fn decision(&self, purpose: &str, allowed: bool, checks: Vec<PolicyCheck>) -> PolicyDecision {
        let mut decision = PolicyDecision {
            decision_id: String::new(),
            allowed,
            authority_order: self.authority_order(),
            layers: self.layers.clone(),
            checks,
            receipt_hash: String::new(),
        };
        decision.receipt_hash = stable_value_hash(&json!({
            "purpose": purpose,
            "allowed": decision.allowed,
            "authority_order": &decision.authority_order,
            "layers": &decision.layers,
            "checks": &decision.checks,
        }));
        decision.decision_id = format!("policy:{}", decision.receipt_hash);
        decision
    }
}

fn base_checks(binding: &AgentBinding) -> Vec<PolicyCheck> {
    vec![
        PolicyCheck {
            check_id: "authority_order".to_string(),
            layer_id: GLOBAL_LAW_LAYER.to_string(),
            status: "enforced".to_string(),
            summary: "global law precedes project law, request, then live evidence".to_string(),
        },
        PolicyCheck {
            check_id: "action_tiers".to_string(),
            layer_id: GLOBAL_LAW_LAYER.to_string(),
            status: "available".to_string(),
            summary: format!(
                "{} action-tier policies govern publication and tool risk",
                binding.capability_scope.action_tiers.len()
            ),
        },
        PolicyCheck {
            check_id: "epistemic_floor".to_string(),
            layer_id: GLOBAL_LAW_LAYER.to_string(),
            status: "enforced".to_string(),
            summary: "publication can only become harder after a head asks".to_string(),
        },
    ]
}

fn summarize_request(task: &str) -> String {
    let trimmed = task.trim();
    if trimmed.len() <= 120 {
        return trimmed.to_string();
    }
    format!("{}...", &trimmed[..120])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AgentBinding, AgentHead, BindingBudgetScope, BindingComposition, BindingIdentity,
        HeadCostProfile, HeadKind, HeadReliabilityProfile, HeadTransport, TraceTier,
    };

    #[test]
    fn default_authority_order_is_stable() {
        assert_eq!(
            default_authority_order(),
            vec![
                "global_law",
                "project_law",
                "current_request",
                "live_evidence"
            ]
        );
    }

    #[test]
    fn policy_decision_binds_layers_and_checks_to_receipt() {
        let binding = binding();
        let constitution = Constitution::for_binding(&binding, "ship the tracer", &[]);
        let decision =
            constitution.head_turn_decision(&binding, "deepseek", HeadInvocationKind::Proposal);

        assert!(decision.allowed);
        assert_eq!(decision.authority_order, default_authority_order());
        assert_eq!(decision.layers.len(), 4);
        assert!(decision
            .checks
            .iter()
            .any(|check| check.check_id == "epistemic_floor"));
        assert!(decision.decision_id.starts_with("policy:"));
        assert_eq!(decision.receipt_hash.len(), 64);
    }

    fn binding() -> AgentBinding {
        AgentBinding::new(
            BindingIdentity {
                agent_id: "theorem".to_string(),
                owner_id: "travis".to_string(),
                agent_name: "Theorem".to_string(),
                composition_hash: String::new(),
                version: 1,
                trust_tier: "first_party".to_string(),
                active_head_set: vec!["deepseek".to_string(), "mistral".to_string()],
                agent_constitution: None,
            },
            BindingComposition {
                heads: vec![head("deepseek", "deepseek"), head("mistral", "mistral")],
            },
            BindingBudgetScope::new("theorem", 100.0, 2),
        )
        .unwrap()
    }

    fn head(head_id: &str, provider: &str) -> AgentHead {
        AgentHead {
            head_id: head_id.to_string(),
            display_name: head_id.to_string(),
            provider: provider.to_string(),
            model: "test".to_string(),
            credential_ref: format!("env:{}", provider.to_uppercase()),
            transport: HeadTransport::Api,
            kind: HeadKind::ReasoningCore,
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }
}
