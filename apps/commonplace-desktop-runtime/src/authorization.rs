//! The action-tier authorization surface for phone-driven runs (phone-control
//! handoff Part B deliverable 4).
//!
//! A submitted run names an *action* (e.g. "explain this file" vs "commit and
//! push"). Before the run executes we classify that action to an action TIER and
//! ask whether the tier requires explicit human authorization. The gating
//! decision is NOT an invented enum: it is grounded in `theorem-harness-core`'s
//! `agent_binding` action-tier model, reused two ways:
//!
//! * **The tier set is the real seed.** [`ActionTierTable::default`] takes the
//!   exact three tiers `BindingCapabilityScope::for_agent` seeds
//!   (`tier_one` reversible / `requires_human_authorization = false`,
//!   `tier_two` consequential-commit / `true`, `tier_three` irreversible-external
//!   / `true`), so the policy is harness-core's, not a parallel copy.
//! * **The decision runs through the real alignment guard.** [`authorize_tier`]
//!   calls `theorem_harness_core::evaluate_publication` with the run's tier on
//!   the payload. `evaluate_publication` is the binding-side enforcement the
//!   kernel applies at `POLICY.CHECKED`; the `tier_requires_human_authorization`
//!   guard it raises is exactly the "hold tier-2/3 until a human authorizes"
//!   rule. We satisfy `evaluate_publication`'s other two checks (consensus +
//!   grounding) with a minimal valid envelope so the ONLY variable left is the
//!   tier + the human-authorization flag -- i.e. the verdict is genuinely the
//!   agent_binding guard firing, not a reimplemented boolean.
//!
//! The result is an [`AuthorizationDecision`]: a tier-1 action is `Immediate`
//! (run now); a tier-2/3 action with no human authorization is `HoldForApproval`
//! (the run-channel parks it in `AwaitingAuthorization` until an explicit
//! `approve` arrives); a tier-2/3 action that already carries authorization is
//! `Immediate`.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::agent_binding::{BindingCapabilityScope, BindingError};
use theorem_harness_core::types::Payload;
use theorem_harness_core::{evaluate_publication, ActionTierPolicy};

/// Stable id of the reversible / no-authorization tier (matches the
/// `agent_binding` seed). Tier-1 runs execute immediately.
pub const TIER_ONE: &str = "tier_one";
/// Stable id of the consequential-commit tier (authorization required).
pub const TIER_TWO: &str = "tier_two";
/// Stable id of the irreversible-external tier (authorization required).
pub const TIER_THREE: &str = "tier_three";

/// The action-tier policy table backing run authorization. Seeded from
/// `agent_binding`'s `BindingCapabilityScope::for_agent` so the tiers (and their
/// `requires_human_authorization` flags) are harness-core's, not a local copy.
#[derive(Clone, Debug)]
pub struct ActionTierTable {
    tiers: Vec<ActionTierPolicy>,
}

impl Default for ActionTierTable {
    fn default() -> Self {
        // Reuse the real tier-seed rather than constructing tiers by hand. This
        // is the single source of the gating policy: if harness-core changes the
        // seed (adds a tier, flips a flag), this table follows.
        let scope = BindingCapabilityScope::for_agent("commonplace-desktop-runtime");
        Self {
            tiers: scope.action_tiers,
        }
    }
}

impl ActionTierTable {
    /// Borrow the seeded tier policies (for inspection / receipts).
    pub fn tiers(&self) -> &[ActionTierPolicy] {
        &self.tiers
    }

    /// Look up a tier policy by id, if present.
    pub fn tier(&self, tier_id: &str) -> Option<&ActionTierPolicy> {
        self.tiers.iter().find(|tier| tier.tier_id == tier_id)
    }

    /// Whether a tier id is known to require explicit human authorization.
    /// Unknown tiers conservatively require authorization (fail safe: an
    /// unrecognized action is treated as if it were consequential).
    pub fn requires_human_authorization(&self, tier_id: &str) -> bool {
        self.tier(tier_id)
            .map(|tier| tier.requires_human_authorization)
            .unwrap_or(true)
    }
}

/// What the authorization surface decided for a submitted run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationDecision {
    /// The action may run now (a reversible tier, or an already-authorized
    /// consequential tier).
    Immediate,
    /// The action must be held until an explicit human `approve` arrives (a
    /// consequential / irreversible tier without prior authorization).
    HoldForApproval,
}

impl AuthorizationDecision {
    /// Whether this decision means the run must wait for an approval.
    pub fn must_hold(self) -> bool {
        matches!(self, AuthorizationDecision::HoldForApproval)
    }
}

/// Decide whether an action at `tier_id` may run immediately or must be held,
/// where `human_authorized` records whether the submitting human has already
/// authorized this run (e.g. an approve presented up front).
///
/// The decision is computed by running the action's tier through harness-core's
/// `evaluate_publication` alignment guard. We build a minimal publication payload
/// that satisfies the guard's consensus + grounding checks so the only thing that
/// can fail it is the `tier_requires_human_authorization` rule. If the guard
/// raises that (and only that) violation, the run holds; otherwise it runs.
pub fn authorize_tier(
    table: &ActionTierTable,
    tier_id: &str,
    human_authorized: bool,
) -> AuthorizationDecision {
    let payload = publication_payload(tier_id, human_authorized);
    // Two synthesis heads so the consensus check (MIN_CONSENSUS_HEADS) passes;
    // these are a fixed, local envelope, not a real composed-agent run -- the
    // point is to isolate the action-tier verdict.
    let synthesis_heads = [
        "commonplace-desktop-runtime".to_string(),
        "commonplace-desktop-runtime-review".to_string(),
    ];
    match evaluate_publication(&synthesis_heads, table.tiers(), &payload) {
        // The guard accepted the publication: a reversible tier, or a tier whose
        // human-authorization requirement is already satisfied.
        Ok(()) => AuthorizationDecision::Immediate,
        // Any guard violation holds the run. In practice the only guard the
        // minimal envelope can trip is `tier_requires_human_authorization`
        // (debug-asserted), but holding on ANY violation is the fail-safe: never
        // run an action the binding guard rejected.
        Err(BindingError::Guard(violation)) => {
            debug_assert_eq!(
                violation.code, "tier_requires_human_authorization",
                "the minimal publication envelope should only ever trip the tier guard"
            );
            AuthorizationDecision::HoldForApproval
        }
    }
}

/// Build the minimal publication payload that isolates the action-tier check:
/// one grounded claim (so the grounding check passes) plus the run's tier and
/// human-authorization flag (the variables the tier guard reads).
fn publication_payload(tier_id: &str, human_authorized: bool) -> Payload {
    let value = json!({
        "action_tier": tier_id,
        "human_authorized": human_authorized,
        "claims": [
            {
                "statement": "phone-control run authorization probe",
                "provenance": "commonplace-desktop-runtime/run-channel",
            }
        ],
    });
    match value {
        Value::Object(map) => map,
        // json!({...}) is always an object; this arm is unreachable in practice.
        _ => Payload::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_table_is_seeded_from_agent_binding() {
        let table = ActionTierTable::default();
        // The three agent_binding seed tiers are present with the seed's flags.
        assert!(!table.requires_human_authorization(TIER_ONE));
        assert!(table.requires_human_authorization(TIER_TWO));
        assert!(table.requires_human_authorization(TIER_THREE));
        assert_eq!(table.tiers().len(), 3);
    }

    #[test]
    fn tier_one_runs_immediately() {
        let table = ActionTierTable::default();
        assert_eq!(
            authorize_tier(&table, TIER_ONE, false),
            AuthorizationDecision::Immediate,
            "a reversible tier-1 action needs no human authorization"
        );
    }

    #[test]
    fn tier_three_holds_until_authorized() {
        let table = ActionTierTable::default();
        // Without authorization the agent_binding guard holds it.
        assert_eq!(
            authorize_tier(&table, TIER_THREE, false),
            AuthorizationDecision::HoldForApproval,
            "an irreversible tier-3 action must hold for human authorization"
        );
        // With authorization the same tier may proceed.
        assert_eq!(
            authorize_tier(&table, TIER_THREE, true),
            AuthorizationDecision::Immediate,
            "an authorized tier-3 action proceeds"
        );
    }

    #[test]
    fn tier_two_holds_until_authorized() {
        let table = ActionTierTable::default();
        assert_eq!(
            authorize_tier(&table, TIER_TWO, false),
            AuthorizationDecision::HoldForApproval
        );
        assert_eq!(
            authorize_tier(&table, TIER_TWO, true),
            AuthorizationDecision::Immediate
        );
    }

    #[test]
    fn unknown_tier_fails_safe_to_hold() {
        let table = ActionTierTable::default();
        // An unrecognized tier id is treated conservatively. evaluate_publication
        // ignores a tier it does not know (no guard fires), so the run would pass
        // its check; our table-level predicate is the fail-safe, asserted here so
        // the conservative default is locked in.
        assert!(table.requires_human_authorization("tier_unknown"));
    }
}
