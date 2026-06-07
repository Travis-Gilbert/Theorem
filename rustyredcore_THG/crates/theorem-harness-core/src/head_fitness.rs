//! Head fitness + explore-aware routing (multi-head run execution, v0.1 #6).
//!
//! Heads are not interchangeable. The scheduler routes a node-type to the head
//! whose past output for that type survived verification, learned from node
//! receipts. But a pure argmax collapses to a monoculture: route every impl node
//! to the fast head, and the other head never earns impl receipts, so its
//! fitness never updates, so the fast head wins impl forever, and the
//! disagreement that catches defects dies. So routing keeps an epsilon-explore
//! band that spreads uniformly across heads: every head keeps a nonzero share of
//! every node-type (the floor), and the policy is probed against, so estimates
//! stay fresh.
//!
//! Determinism: the explore decision is a caller-supplied token, not an internal
//! RNG, so a multi-head run replays through the existing replay path (the same
//! discipline as the work-graph kernel passing logical time in).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A node outcome that updates fitness: did this head's node for this node-type
/// reach `Accepted` (survived verification) or `Rejected`?
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeResult {
    Accepted,
    Rejected,
}

/// Accepted/total counters for one (node_type, head). Laplace-smoothed so a head
/// with no history sits at the neutral cold-start prior, not 0 or 1.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct FitnessCounter {
    pub accepted: u64,
    pub total: u64,
}

impl FitnessCounter {
    fn record(&mut self, result: NodeResult) {
        self.total += 1;
        if matches!(result, NodeResult::Accepted) {
            self.accepted += 1;
        }
    }

    /// Laplace-smoothed acceptance rate `(accepted + 1) / (total + 2)`: no
    /// history yields 0.5, and a single outcome moves it gently rather than to a
    /// degenerate 0 or 1.
    pub fn rate(&self) -> f64 {
        (self.accepted as f64 + 1.0) / (self.total as f64 + 2.0)
    }
}

/// Routing knobs. `explore_epsilon_milli` is epsilon * 1000 (integer so the
/// policy stays `Eq` and deterministic): the share of decisions spread uniformly
/// across heads instead of going to the argmax. That band IS the per-head floor.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoutingPolicy {
    pub explore_epsilon_milli: u32,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        // 15% of decisions explore (spread across heads); 85% exploit the argmax.
        Self {
            explore_epsilon_milli: 150,
        }
    }
}

/// Per-(node_type, head) fitness over a fixed set of heads, plus the routing
/// policy. Cold-start neutral; receipts move it.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HeadFitness {
    pub heads: Vec<String>,
    counters: BTreeMap<String, BTreeMap<String, FitnessCounter>>,
    pub policy: RoutingPolicy,
}

impl HeadFitness {
    pub fn new(heads: Vec<String>) -> Self {
        Self {
            heads,
            counters: BTreeMap::new(),
            policy: RoutingPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: RoutingPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Record a node outcome for (node_type, head).
    pub fn record(&mut self, node_type: &str, head: &str, result: NodeResult) {
        self.counters
            .entry(node_type.to_string())
            .or_default()
            .entry(head.to_string())
            .or_default()
            .record(result);
    }

    /// Smoothed acceptance rate for (node_type, head); 0.5 with no history.
    pub fn rate(&self, node_type: &str, head: &str) -> f64 {
        self.counters
            .get(node_type)
            .and_then(|by_head| by_head.get(head))
            .map(FitnessCounter::rate)
            .unwrap_or(0.5)
    }

    /// Route a node-type to a head.
    ///
    /// `explore_token` is a deterministic value in `[0, 1000)` supplied by the
    /// caller (so the decision replays). If it falls in the explore band it
    /// spreads across heads (the floor: every head keeps a nonzero share);
    /// otherwise it exploits the argmax rate, ties broken by head order.
    pub fn route(&self, node_type: &str, explore_token: u32) -> Option<String> {
        if self.heads.is_empty() {
            return None;
        }
        if explore_token < self.policy.explore_epsilon_milli {
            let index = (explore_token as usize) % self.heads.len();
            return Some(self.heads[index].clone());
        }
        let mut best: Option<(&String, f64)> = None;
        for head in &self.heads {
            let rate = self.rate(node_type, head);
            // Strictly greater keeps the first head on a tie (deterministic).
            if best.is_none_or(|(_, best_rate)| rate > best_rate) {
                best = Some((head, rate));
            }
        }
        best.map(|(head, _)| head.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn fitness() -> HeadFitness {
        HeadFitness::new(vec!["claude".to_string(), "codex".to_string()])
    }

    #[test]
    fn cold_start_is_neutral_and_exploits_first_head_on_a_tie() {
        let f = fitness();
        assert_eq!(f.rate("rust_impl", "claude"), 0.5);
        assert_eq!(f.rate("rust_impl", "codex"), 0.5);
        // Exploit (token past the explore band), all rates equal -> first head.
        assert_eq!(f.route("rust_impl", 999), Some("claude".to_string()));
    }

    #[test]
    fn receipts_move_routing_to_the_head_that_survives_verification() {
        let mut f = fitness();
        for _ in 0..5 {
            f.record("rust_impl", "claude", NodeResult::Accepted);
            f.record("rust_impl", "codex", NodeResult::Rejected);
        }
        assert!(f.rate("rust_impl", "claude") > f.rate("rust_impl", "codex"));
        // Exploit routes to the survivor.
        assert_eq!(f.route("rust_impl", 999), Some("claude".to_string()));
    }

    #[test]
    fn the_losing_head_keeps_a_nonzero_share_no_monoculture() {
        let mut f = fitness();
        // Make claude the clear argmax for rust_impl.
        for _ in 0..8 {
            f.record("rust_impl", "claude", NodeResult::Accepted);
            f.record("rust_impl", "codex", NodeResult::Rejected);
        }
        // Exploit always picks claude...
        assert_eq!(f.route("rust_impl", 800), Some("claude".to_string()));
        // ...but the explore band keeps codex alive: the floor that preserves the
        // disagreement which catches defects.
        let explored: BTreeSet<String> = (0..f.policy.explore_epsilon_milli)
            .filter_map(|token| f.route("rust_impl", token))
            .collect();
        assert!(
            explored.contains("codex"),
            "the losing head is never shut out"
        );
        assert!(explored.contains("claude"));
    }

    #[test]
    fn different_node_types_route_independently() {
        let mut f = fitness();
        for _ in 0..5 {
            f.record("rust_impl", "codex", NodeResult::Accepted);
            f.record("verify", "claude", NodeResult::Accepted);
        }
        // Each head is the survivor for a different node-type.
        assert_eq!(f.route("rust_impl", 999), Some("codex".to_string()));
        assert_eq!(f.route("verify", 999), Some("claude".to_string()));
    }

    #[test]
    fn no_heads_routes_none() {
        let f = HeadFitness::new(Vec::new());
        assert_eq!(f.route("rust_impl", 0), None);
    }
}
