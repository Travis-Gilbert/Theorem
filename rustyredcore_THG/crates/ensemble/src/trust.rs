//! Trust-ladder gating for ensemble selection (slice S3).
//!
//! The `TrustTier` type itself lives in `registry.rs` because it is part of the persisted
//! `CapabilityPack`. This module holds the *gating logic* the selector applies: ranking tiers,
//! turning a tier into a selection-weight contribution, parsing a minimum-trust floor (as written
//! by the priors), and enforcing that floor. Keeping the type in `registry.rs` and the policy here
//! avoids churning the persisted contract while still giving the selector a single place to reason
//! about trust.

use crate::registry::TrustTier;

/// Monotonic rank of a trust tier. Higher = more trusted. Used for floor comparisons.
pub fn trust_rank(tier: &TrustTier) -> u8 {
    match tier {
        TrustTier::Unverified => 0,
        TrustTier::FirstParty { .. } => 1,
    }
}

/// Selection-weight contribution of a tier, in `[0.0, 1.0]`. The selector multiplies this by the
/// configured `trust_weight` and adds it to a pack's relevance, so trust acts as a bounded bonus
/// rather than dominating relevance.
pub fn trust_score(tier: &TrustTier) -> f64 {
    match tier {
        TrustTier::Unverified => 0.0,
        TrustTier::FirstParty { .. } => 1.0,
    }
}

/// Parse a minimum-trust floor label (e.g. `priors.min_trust`) into a rank. Unknown or empty
/// labels map to `0` (allow every tier), so an absent/garbage floor never silently excludes packs.
pub fn parse_trust_floor(label: &str) -> u8 {
    match label.trim().to_ascii_lowercase().as_str() {
        "first_party" | "firstparty" | "first-party" => 1,
        _ => 0,
    }
}

/// Whether a tier satisfies a minimum rank floor.
pub fn meets_floor(tier: &TrustTier, floor: u8) -> bool {
    trust_rank(tier) >= floor
}

/// The passport id when the pack is first-party, else `None`.
pub fn passport_id(tier: &TrustTier) -> Option<&str> {
    match tier {
        TrustTier::FirstParty { passport_id } => Some(passport_id.as_str()),
        TrustTier::Unverified => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn first_party() -> TrustTier {
        TrustTier::FirstParty {
            passport_id: "passport-123".to_string(),
        }
    }

    #[test]
    fn rank_and_score_are_monotonic() {
        assert!(trust_rank(&TrustTier::Unverified) < trust_rank(&first_party()));
        assert!(trust_score(&TrustTier::Unverified) < trust_score(&first_party()));
    }

    #[test]
    fn floor_parsing_is_lenient() {
        assert_eq!(parse_trust_floor("first_party"), 1);
        assert_eq!(parse_trust_floor("  First-Party "), 1);
        assert_eq!(parse_trust_floor("unverified"), 0);
        assert_eq!(parse_trust_floor(""), 0);
        assert_eq!(parse_trust_floor("garbage"), 0);
    }

    #[test]
    fn floor_enforcement() {
        // floor 0 admits everyone
        assert!(meets_floor(&TrustTier::Unverified, 0));
        assert!(meets_floor(&first_party(), 0));
        // floor 1 excludes unverified, admits first-party
        assert!(!meets_floor(&TrustTier::Unverified, 1));
        assert!(meets_floor(&first_party(), 1));
    }

    #[test]
    fn passport_extraction() {
        assert_eq!(passport_id(&first_party()), Some("passport-123"));
        assert_eq!(passport_id(&TrustTier::Unverified), None);
    }
}
