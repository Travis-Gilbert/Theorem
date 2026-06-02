//! Substrate-enforced alignment for the composed agent (spec Part 5,
//! build-order step 9).
//!
//! The spec's central thesis: "alignment lives in the binding, not the heads."
//! A head asserting `allowed: true` at `POLICY.CHECKED` is the head asking; it
//! is not the binding enforcing. This module is the binding enforcing. Given
//! the run's recorded synthesis state and the publication payload, it computes
//! a verdict the kernel guard applies AFTER the head's assertion, so the binding
//! can only ever make publication HARDER, never easier:
//!
//! 1. Critic consensus (Part 5.3): a publication requires at least
//!    `MIN_CONSENSUS_HEADS` distinct heads recorded at `DRAFTS.SYNTHESIZED`, so
//!    heterogeneous review demonstrably happened.
//! 2. Action tier (Part 5.4): when the payload names an `action_tier` that the
//!    binding marks as requiring human authorization, `human_authorized` must be
//!    true; autonomy is bounded inversely to irreversibility.
//! 3. Grounding (Part 5.2): a publication must declare at least one claim, and
//!    every claim must carry non-empty `provenance`. A claimless or ungrounded
//!    publication is refused, so ungroundedness breaks loudly instead of passing.
//!
//! All three are unconditional (the action-tier check is inherently scoped to
//! publications that name a tier). Grounding is strict: no flag, no
//! enforce-if-present escape hatch. A publication that does not ground itself
//! cannot reach `PUBLISHED_TO_SUBSTRATE`.

use crate::agent_binding::{ActionTierPolicy, BindingError};
use crate::types::{GuardViolation, Payload};
use serde_json::{json, Value};
use std::collections::BTreeSet;

/// Minimum number of distinct synthesis-contributing heads required before a
/// composed agent may publish. Two means at least one head reviewed another's
/// proposal (the heterogeneity that catches single-model error).
pub const MIN_CONSENSUS_HEADS: usize = 2;

/// Compute the binding's publication verdict. Called by the `POLICY.CHECKED`
/// guard only when the head has asserted `allowed: true`; a returned error
/// overrides that assertion and blocks `PUBLISHED_TO_SUBSTRATE`.
pub fn evaluate_publication(
    synthesis_heads: &[String],
    action_tiers: &[ActionTierPolicy],
    payload: &Payload,
) -> Result<(), BindingError> {
    let distinct: BTreeSet<&str> = synthesis_heads
        .iter()
        .map(|head| head.trim())
        .filter(|head| !head.is_empty())
        .collect();
    if distinct.len() < MIN_CONSENSUS_HEADS {
        return Err(guard(
            "consensus_below_threshold",
            "publication requires heterogeneous critic consensus",
            json!({
                "distinct_synthesis_heads": distinct.len(),
                "required": MIN_CONSENSUS_HEADS,
            }),
        ));
    }

    if let Some(tier_id) = payload.get("action_tier").and_then(Value::as_str) {
        if let Some(policy) = action_tiers.iter().find(|tier| tier.tier_id == tier_id) {
            let authorized = payload
                .get("human_authorized")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if policy.requires_human_authorization && !authorized {
                return Err(guard(
                    "tier_requires_human_authorization",
                    "this action tier requires explicit human authorization",
                    json!({ "action_tier": tier_id }),
                ));
            }
        }
    }

    match payload.get("claims") {
        Some(Value::Array(claims)) if !claims.is_empty() => {
            for (index, claim) in claims.iter().enumerate() {
                if !claim_is_grounded(claim) {
                    return Err(guard(
                        "grounding_missing",
                        "every published claim must carry provenance",
                        json!({ "claim_index": index }),
                    ));
                }
            }
        }
        _ => {
            return Err(guard(
                "grounding_missing",
                "a publication must declare at least one grounded claim",
                json!({}),
            ));
        }
    }

    Ok(())
}

fn claim_is_grounded(claim: &Value) -> bool {
    match claim.get("provenance") {
        Some(Value::String(value)) => !value.trim().is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
        _ => false,
    }
}

fn guard(code: &str, message: &str, details: Value) -> BindingError {
    let details = match details {
        Value::Object(map) => map,
        _ => Payload::new(),
    };
    BindingError::Guard(Box::new(GuardViolation {
        code: code.to_string(),
        message: message.to_string(),
        required_state: String::new(),
        received_state: String::new(),
        missing_fields: Vec::new(),
        details,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    fn tiers() -> Vec<ActionTierPolicy> {
        vec![
            ActionTierPolicy::new("tier_one", "reversible", false),
            ActionTierPolicy::new("tier_three", "irreversible external", true),
        ]
    }

    fn payload(value: Value) -> Payload {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn err_code(error: BindingError) -> String {
        match error {
            BindingError::Guard(violation) => violation.code,
        }
    }

    #[test]
    fn claimless_publication_is_refused() {
        let heads = vec!["claude".to_string(), "deepseek".to_string()];
        let error = evaluate_publication(&heads, &tiers(), &payload(json!({}))).unwrap_err();
        assert_eq!(err_code(error), "grounding_missing");
    }

    #[test]
    fn single_head_fails_consensus() {
        let heads = vec!["claude".to_string(), "claude".to_string()];
        let error = evaluate_publication(&heads, &tiers(), &payload(json!({}))).unwrap_err();
        assert_eq!(err_code(error), "consensus_below_threshold");
    }

    #[test]
    fn ungrounded_claim_is_refused() {
        let heads = vec!["claude".to_string(), "deepseek".to_string()];
        let error = evaluate_publication(
            &heads,
            &tiers(),
            &payload(
                json!({ "claims": [{ "text": "x", "provenance": "src:1" }, { "text": "y" }] }),
            ),
        )
        .unwrap_err();
        assert_eq!(err_code(error), "grounding_missing");
    }

    #[test]
    fn grounded_claims_pass() {
        let heads = vec!["claude".to_string(), "deepseek".to_string()];
        evaluate_publication(
            &heads,
            &tiers(),
            &payload(json!({ "claims": [{ "provenance": ["src:1", "src:2"] }] })),
        )
        .unwrap();
    }

    #[test]
    fn tier_three_without_authorization_is_blocked() {
        let heads = vec!["claude".to_string(), "deepseek".to_string()];
        let error = evaluate_publication(
            &heads,
            &tiers(),
            &payload(json!({ "action_tier": "tier_three" })),
        )
        .unwrap_err();
        assert_eq!(err_code(error), "tier_requires_human_authorization");
    }

    #[test]
    fn tier_three_with_authorization_and_grounded_claims_passes() {
        let heads = vec!["claude".to_string(), "deepseek".to_string()];
        evaluate_publication(
            &heads,
            &tiers(),
            &payload(json!({
                "action_tier": "tier_three",
                "human_authorized": true,
                "claims": [{ "provenance": "src:1" }]
            })),
        )
        .unwrap();
    }
}
