//! Multi-head run execution: the transactional work graph (v0.1 foundation).
//!
//! The headline property: two heads racing one node produce exactly one winner.
//! The duplicate `*-browse-agent` crate becomes impossible because "create the
//! crate" is one claimable node; only one head can own it, the other observes
//! `claimed` and routes elsewhere. Plan:
//! `docs/plans/multi-head-run-execution/HANDOFF.md`.
//!
//! This module is the pure, deterministic CAS kernel: no clock, no I/O (logical
//! time is passed in, so lease expiry is testable). GraphStore persistence and
//! the apply sequencer layer on top in `theorem-harness-runtime`.
//!
//! Two safety properties baked in beyond the handoff:
//! - Claim LEASE: a crashed or context-compacted head must not pin a node
//!   forever. A claim carries an expiry; past it the node is reclaimable, and
//!   the epoch bump on reclaim rejects the dead head's late operation.
//! - Receipt re-run contract: the substrate is not required to trust a head's
//!   "test passed" assertion; the receipt carries the command and a slot for the
//!   substrate's own re-derived result (see `Receipt`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Logical time in milliseconds. Passed in by the caller so the kernel stays
/// pure and deterministic (claim lease expiry is unit-testable without a clock).
pub type Millis = u64;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeStatus {
    Open,
    Claimed,
    PatchProposed,
    Verifying,
    Accepted,
    Rejected,
}

impl NodeStatus {
    /// Accepted and Rejected are terminal: never re-claimable.
    pub fn is_terminal(self) -> bool {
        matches!(self, NodeStatus::Accepted | NodeStatus::Rejected)
    }
}

/// A head's hold on a node. The `epoch` is the value the claim was granted at;
/// the node's `claim_epoch` is the monotonic compare-and-swap target.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ClaimLease {
    pub owner: String,
    pub epoch: u64,
    pub granted_at: Millis,
    pub expires_at: Millis,
    pub last_heartbeat: Millis,
}

impl ClaimLease {
    /// A lease is expired (the node reclaimable) once `now` reaches `expires_at`.
    pub fn is_expired(&self, now: Millis) -> bool {
        now >= self.expires_at
    }
}

/// Proof attached at node completion.
///
/// The substrate does not have to trust the head: `claimed_status` is what the
/// head asserts; `verified_status` is what the substrate (or a CI head) got by
/// RE-RUNNING `command`. A receipt binds to `base_commit` (a green result is
/// only green against the base it ran on, so it is stale after a rebase). For
/// proofs too expensive to re-run, bind `artifact_hash` and accept at a lower
/// trust tier.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Receipt {
    pub kind: String,
    pub command: String,
    pub base_commit: String,
    pub claimed_status: String,
    #[serde(default)]
    pub verified_status: Option<String>,
    #[serde(default)]
    pub artifact_hash: String,
}

impl Receipt {
    /// Trusted only when the substrate re-ran the command and got the asserted
    /// result. A head asserting "pass" is a claim, not a proof.
    pub fn is_substrate_verified(&self) -> bool {
        self.verified_status.as_deref() == Some(self.claimed_status.as_str())
    }
}

/// A unit of work in the run. Lives in the GraphStore beside RunState; this is
/// the in-memory shape the CAS kernel operates on.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub run_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Binds a skill-pack motor program (Ensemble selection keyed on node_type).
    pub node_type: String,
    /// What "accepted" means for this node.
    pub goal: String,
    #[serde(default)]
    pub prerequisites: Vec<String>,
    /// Declared at refine time; a ROUTING HINT, never a write lock.
    #[serde(default)]
    pub file_scope: Vec<String>,
    pub status: NodeStatus,
    #[serde(default)]
    pub claim: Option<ClaimLease>,
    /// Monotonic compare-and-swap target. Survives claim and unclaim.
    pub claim_epoch: u64,
    #[serde(default)]
    pub receipts: Vec<Receipt>,
    #[serde(default)]
    pub created_by: String,
    /// For a verify node: the head that must review the target (the other head).
    #[serde(default)]
    pub review_required_by: Option<String>,
}

impl TaskNode {
    pub fn open(
        id: impl Into<String>,
        run_id: impl Into<String>,
        node_type: impl Into<String>,
        goal: impl Into<String>,
        created_by: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            run_id: run_id.into(),
            parent_id: None,
            node_type: node_type.into(),
            goal: goal.into(),
            prerequisites: Vec::new(),
            file_scope: Vec::new(),
            status: NodeStatus::Open,
            claim: None,
            claim_epoch: 0,
            receipts: Vec::new(),
            created_by: created_by.into(),
            review_required_by: None,
        }
    }

    pub fn with_prerequisites(mut self, prerequisites: Vec<String>) -> Self {
        self.prerequisites = prerequisites;
        self
    }

    /// Claimable when Open, or when a held lease has expired (crashed-head
    /// reclaim). Terminal nodes are never claimable.
    pub fn is_claimable(&self, now: Millis) -> bool {
        if self.status.is_terminal() {
            return false;
        }
        matches!(self.status, NodeStatus::Open)
            || self
                .claim
                .as_ref()
                .is_some_and(|lease| lease.is_expired(now))
    }
}

/// The result of a claim attempt. Exactly one of two racing heads gets `Won`.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClaimOutcome {
    Won {
        epoch: u64,
    },
    /// The CAS lost: epoch moved, the node is held by a live lease, or it is
    /// terminal. Carries the current node so the losing head can route on.
    Lost {
        current: Box<TaskNode>,
    },
}

/// Compare-and-swap claim. Succeeds only when `expected_epoch` matches the
/// node's current `claim_epoch` AND the node is claimable. On success the epoch
/// bumps (so any later op against the old epoch, including a dead head's, loses)
/// and a fresh lease is granted.
pub fn claim_task_node(
    node: &mut TaskNode,
    head_id: &str,
    expected_epoch: u64,
    now: Millis,
    lease_ttl: Millis,
) -> ClaimOutcome {
    if node.claim_epoch != expected_epoch || !node.is_claimable(now) {
        return ClaimOutcome::Lost {
            current: Box::new(node.clone()),
        };
    }
    let new_epoch = node.claim_epoch + 1;
    node.claim_epoch = new_epoch;
    node.status = NodeStatus::Claimed;
    node.claim = Some(ClaimLease {
        owner: head_id.to_string(),
        epoch: new_epoch,
        granted_at: now,
        expires_at: now.saturating_add(lease_ttl),
        last_heartbeat: now,
    });
    ClaimOutcome::Won { epoch: new_epoch }
}

/// Extend the lease of a node the head still owns. Keeps a long-running claim
/// alive without a reclaim race. No-op (false) if the head is not the live owner.
pub fn heartbeat_task_node(
    node: &mut TaskNode,
    head_id: &str,
    now: Millis,
    lease_ttl: Millis,
) -> bool {
    match node.claim.as_mut() {
        Some(lease) if lease.owner == head_id && !lease.is_expired(now) => {
            lease.last_heartbeat = now;
            lease.expires_at = now.saturating_add(lease_ttl);
            true
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum WorkGraphError {
    UnknownNode(String),
    NodeNotClaimed(String),
    NotClaimOwner { node: String, head: String },
}

/// The in-memory work graph for one run. A `BTreeMap` keeps node iteration
/// deterministic (replay and hashing friendly).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkGraph {
    pub run_id: String,
    pub nodes: BTreeMap<String, TaskNode>,
}

impl WorkGraph {
    pub fn new(run_id: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            nodes: BTreeMap::new(),
        }
    }

    pub fn insert(&mut self, node: TaskNode) {
        self.nodes.insert(node.id.clone(), node);
    }

    pub fn get(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    /// A node is ready when every prerequisite exists and is Accepted.
    pub fn is_ready(&self, id: &str) -> bool {
        match self.nodes.get(id) {
            Some(node) => node.prerequisites.iter().all(|pre| {
                self.nodes
                    .get(pre)
                    .is_some_and(|p| p.status == NodeStatus::Accepted)
            }),
            None => false,
        }
    }

    /// Ready nodes a head could claim now: claimable (Open or expired) and all
    /// prerequisites accepted. This is the work-stealing queue.
    pub fn claimable_ready_nodes(&self, now: Millis) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|node| node.is_claimable(now) && self.is_ready(&node.id))
            .collect()
    }

    /// Claim a node by id. Returns `UnknownNode` if absent, else the CAS outcome.
    pub fn claim(
        &mut self,
        id: &str,
        head_id: &str,
        expected_epoch: u64,
        now: Millis,
        lease_ttl: Millis,
    ) -> Result<ClaimOutcome, WorkGraphError> {
        let node = self
            .nodes
            .get_mut(id)
            .ok_or_else(|| WorkGraphError::UnknownNode(id.to_string()))?;
        Ok(claim_task_node(
            node,
            head_id,
            expected_epoch,
            now,
            lease_ttl,
        ))
    }

    /// Claim-and-refine: a claimed node declares the file scope it just
    /// discovered and splits into children. Only the live claim owner may
    /// refine. Children are inserted; the parent stays Claimed.
    pub fn refine(
        &mut self,
        parent_id: &str,
        head_id: &str,
        discovered_file_scope: Vec<String>,
        children: Vec<TaskNode>,
        now: Millis,
    ) -> Result<(), WorkGraphError> {
        let parent = self
            .nodes
            .get_mut(parent_id)
            .ok_or_else(|| WorkGraphError::UnknownNode(parent_id.to_string()))?;
        match parent.claim.as_ref() {
            Some(lease) if lease.owner == head_id && !lease.is_expired(now) => {}
            Some(_) => {
                return Err(WorkGraphError::NotClaimOwner {
                    node: parent_id.to_string(),
                    head: head_id.to_string(),
                })
            }
            None => return Err(WorkGraphError::NodeNotClaimed(parent_id.to_string())),
        }
        parent.file_scope = discovered_file_scope;
        for mut child in children {
            child.parent_id = Some(parent_id.to_string());
            child.run_id = self.run_id.clone();
            self.nodes.insert(child.id.clone(), child);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Millis = 100;

    fn open_node(id: &str) -> TaskNode {
        TaskNode::open(id, "run-1", "rust_impl", format!("do {id}"), "seed")
    }

    /// The headline proof: two heads race one node, exactly one wins. This is
    /// the duplicate `*-browse-agent` crate made impossible.
    #[test]
    fn two_heads_race_one_node_exactly_one_winner() {
        let mut graph = WorkGraph::new("run-1");
        graph.insert(open_node("create-crate"));

        // Both heads read epoch 0, both attempt the claim.
        let a = graph
            .claim("create-crate", "claude", 0, 1_000, TTL)
            .unwrap();
        let b = graph.claim("create-crate", "codex", 0, 1_000, TTL).unwrap();

        let wins = [&a, &b]
            .iter()
            .filter(|o| matches!(o, ClaimOutcome::Won { .. }))
            .count();
        assert_eq!(wins, 1, "exactly one head wins the node");
        assert!(matches!(a, ClaimOutcome::Won { epoch: 1 }));
        assert!(matches!(b, ClaimOutcome::Lost { .. }));

        let node = graph.get("create-crate").unwrap();
        assert_eq!(node.status, NodeStatus::Claimed);
        assert_eq!(node.claim.as_ref().unwrap().owner, "claude");
        assert_eq!(node.claim_epoch, 1);
    }

    /// A crashed head's claim expires and is reclaimable; its late op loses.
    #[test]
    fn expired_lease_reclaimable_and_stale_op_rejected() {
        let mut graph = WorkGraph::new("run-1");
        graph.insert(open_node("n"));

        // Head A claims at t=0 (lease to t=100), then "crashes" (no heartbeat).
        assert!(matches!(
            graph.claim("n", "claude", 0, 0, TTL).unwrap(),
            ClaimOutcome::Won { epoch: 1 }
        ));

        // Before expiry, B cannot reclaim.
        assert!(matches!(
            graph.claim("n", "codex", 1, 50, TTL).unwrap(),
            ClaimOutcome::Lost { .. }
        ));

        // After expiry (t=200), B reclaims at the current epoch.
        assert!(matches!(
            graph.claim("n", "codex", 1, 200, TTL).unwrap(),
            ClaimOutcome::Won { epoch: 2 }
        ));
        assert_eq!(
            graph.get("n").unwrap().claim.as_ref().unwrap().owner,
            "codex"
        );

        // A's late op against the old epoch is rejected (epoch moved to 2).
        assert!(matches!(
            graph.claim("n", "claude", 1, 250, TTL).unwrap(),
            ClaimOutcome::Lost { .. }
        ));
    }

    /// Heartbeat keeps a live claim from being reclaimed.
    #[test]
    fn heartbeat_extends_lease() {
        let mut graph = WorkGraph::new("run-1");
        graph.insert(open_node("n"));
        graph.claim("n", "claude", 0, 0, TTL).unwrap();

        let node = graph.nodes.get_mut("n").unwrap();
        assert!(heartbeat_task_node(node, "claude", 80, TTL)); // extends to 180
        assert!(!heartbeat_task_node(node, "codex", 80, TTL)); // not the owner

        // At t=120 the original lease would have expired (100), but the
        // heartbeat pushed it to 180, so the node is still held.
        assert!(matches!(
            graph.claim("n", "codex", 1, 120, TTL).unwrap(),
            ClaimOutcome::Lost { .. }
        ));
    }

    #[test]
    fn terminal_node_is_not_claimable() {
        let mut graph = WorkGraph::new("run-1");
        let mut done = open_node("done");
        done.status = NodeStatus::Accepted;
        graph.insert(done);
        assert!(matches!(
            graph.claim("done", "claude", 0, 0, TTL).unwrap(),
            ClaimOutcome::Lost { .. }
        ));
    }

    #[test]
    fn ready_requires_prerequisites_accepted() {
        let mut graph = WorkGraph::new("run-1");
        graph.insert(open_node("base"));
        graph.insert(open_node("dependent").with_prerequisites(vec!["base".into()]));

        assert!(graph.is_ready("base"));
        assert!(!graph.is_ready("dependent"), "prereq not accepted yet");
        assert!(!graph
            .claimable_ready_nodes(0)
            .iter()
            .any(|n| n.id == "dependent"));

        // Accept the prerequisite; the dependent becomes ready.
        graph.nodes.get_mut("base").unwrap().status = NodeStatus::Accepted;
        assert!(graph.is_ready("dependent"));
        assert!(graph
            .claimable_ready_nodes(0)
            .iter()
            .any(|n| n.id == "dependent"));
    }

    /// Claim-and-refine: the owner declares discovered file scope and splits
    /// into children that replay as part of the graph.
    #[test]
    fn claimed_node_refines_into_children_with_discovered_scope() {
        let mut graph = WorkGraph::new("run-1");
        graph.insert(open_node("ship-browse-loop"));
        graph
            .claim("ship-browse-loop", "claude", 0, 0, TTL)
            .unwrap();

        graph
            .refine(
                "ship-browse-loop",
                "claude",
                vec!["rustyred-thg-server/src/router.rs".into()],
                vec![open_node("router-loop-fn"), open_node("loop-test")],
                10,
            )
            .unwrap();

        let parent = graph.get("ship-browse-loop").unwrap();
        assert_eq!(parent.file_scope, vec!["rustyred-thg-server/src/router.rs"]);
        assert_eq!(
            graph.get("router-loop-fn").unwrap().parent_id.as_deref(),
            Some("ship-browse-loop")
        );

        // A non-owner cannot refine.
        assert!(matches!(
            graph.refine("ship-browse-loop", "codex", vec![], vec![], 10),
            Err(WorkGraphError::NotClaimOwner { .. })
        ));
    }
}
