//! Verify-node-with-teeth (multi-head run execution, v0.1 list item #3).
//!
//! Verification is what an idle head does, not a scheduled skill. When a head
//! proposes a patch for a node, a sibling verify node is spawned for the OTHER
//! head; the target node reaches `Accepted` only after its verify node accepts.
//!
//! The teeth: a verify node accepts only on a falsification ATTEMPT (the reviewer
//! records what it tried that could have failed), never an LGTM, and a verify
//! that finds a defect reopens the target. This is the existing Critique role
//! (`head_invocation::HeadInvocationKind::Critique`) and the consensus gate
//! (`alignment::evaluate_publication` + `MIN_CONSENSUS_HEADS`) made continuous
//! and claimable at node granularity. Fitness rewards defect discovery, so a
//! verify that breaks a patch is a high-value outcome, not a delay.

use serde::{Deserialize, Serialize};

use crate::work_graph::{NodeStatus, TaskNode, WorkGraph};

pub const VERIFY_NODE_TYPE: &str = "verify";

/// A verify node's receipt. The teeth: `attempted_failure_modes` must be
/// non-empty (a real attempt to break the patch); `defect_found` flips the
/// target to reopened. An empty attempt is an LGTM and does not pass the gate.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VerifyReceipt {
    pub target_node_id: String,
    pub reviewer: String,
    #[serde(default)]
    pub attempted_failure_modes: Vec<String>,
    #[serde(default)]
    pub commands_run: Vec<String>,
    pub defect_found: bool,
    #[serde(default)]
    pub waived_risks: Vec<String>,
}

impl VerifyReceipt {
    /// A real falsification attempt: the reviewer probed at least one way the
    /// patch could fail. The incentive is to break it, not bless it.
    pub fn is_falsification_attempt(&self) -> bool {
        !self.attempted_failure_modes.is_empty()
    }
}

pub fn verify_node_id(target_id: &str) -> String {
    format!("verify:{target_id}")
}

/// The outcome of submitting a verify receipt for a target.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyOutcome {
    /// Real attempt, no defect: verify and target both `Accepted`.
    TargetAccepted,
    /// Real attempt found a defect: verify `Accepted` (it did its job), target
    /// `Rejected` and reopened for rework.
    DefectFound,
    /// LGTM: no failure mode was attempted. Rejected; nothing changes.
    NotAFalsificationAttempt,
    /// The submitter is not the head the verify node was assigned to.
    WrongReviewer,
    /// No verify node exists for the target.
    UnknownVerifyNode,
}

/// Auto-spawn the sibling verify node for a target that has proposed a patch,
/// assigned to `reviewer_head` (the head that did not implement the target).
///
/// The verify links to its target via `parent_id` (the VERIFIES relationship),
/// NOT a prerequisite: a prerequisite requires the target Accepted, but the
/// target is accepted only THROUGH this verify, which would be circular and the
/// verify would never become ready. The verify is ready to claim once its target
/// reaches `PatchProposed` (the scheduler checks that). Inserts it; returns the
/// verify node id, or `None` if the target is absent.
pub fn spawn_verify_node(
    graph: &mut WorkGraph,
    target_id: &str,
    reviewer_head: &str,
) -> Option<String> {
    let run_id = graph.get(target_id)?.run_id.clone();
    let id = verify_node_id(target_id);
    let mut node = TaskNode::open(
        &id,
        &run_id,
        VERIFY_NODE_TYPE,
        format!("refute {target_id}"),
        "substrate",
    );
    node.parent_id = Some(target_id.to_string());
    node.review_required_by = Some(reviewer_head.to_string());
    graph.insert(node);
    Some(id)
}

/// Submit a verify receipt. This is the ONLY path to `Accept` a target: a target
/// reaches `Accepted` only through its verify node accepting a real falsification
/// attempt by the assigned reviewer. Mutates the verify and target statuses.
pub fn submit_verify_receipt(graph: &mut WorkGraph, receipt: &VerifyReceipt) -> VerifyOutcome {
    let verify_id = verify_node_id(&receipt.target_node_id);
    let assigned = match graph.get(&verify_id) {
        Some(node) => node.review_required_by.clone(),
        None => return VerifyOutcome::UnknownVerifyNode,
    };
    if assigned.as_deref() != Some(receipt.reviewer.as_str()) {
        return VerifyOutcome::WrongReviewer;
    }
    if !receipt.is_falsification_attempt() {
        // LGTM: the gate rejects it; the verify node stays open for a real attempt.
        return VerifyOutcome::NotAFalsificationAttempt;
    }

    set_status(graph, &verify_id, NodeStatus::Accepted);
    if receipt.defect_found {
        set_status(graph, &receipt.target_node_id, NodeStatus::Rejected);
        VerifyOutcome::DefectFound
    } else {
        set_status(graph, &receipt.target_node_id, NodeStatus::Accepted);
        VerifyOutcome::TargetAccepted
    }
}

fn set_status(graph: &mut WorkGraph, node_id: &str, status: NodeStatus) {
    if let Some(node) = graph.nodes.get_mut(node_id) {
        node.status = status;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proposed_target(graph: &mut WorkGraph, id: &str, owner: &str) {
        let mut node = TaskNode::open(id, "run-1", "rust_impl", format!("do {id}"), owner);
        node.status = NodeStatus::PatchProposed;
        graph.insert(node);
    }

    fn receipt(target: &str, reviewer: &str, attempted: &[&str], defect: bool) -> VerifyReceipt {
        VerifyReceipt {
            target_node_id: target.to_string(),
            reviewer: reviewer.to_string(),
            attempted_failure_modes: attempted.iter().map(|s| s.to_string()).collect(),
            commands_run: vec!["cargo test".into()],
            defect_found: defect,
            waived_risks: Vec::new(),
        }
    }

    #[test]
    fn spawn_creates_verify_assigned_to_other_head() {
        let mut graph = WorkGraph::new("run-1");
        proposed_target(&mut graph, "impl-x", "claude");
        let vid = spawn_verify_node(&mut graph, "impl-x", "codex").unwrap();
        let v = graph.get(&vid).unwrap();
        assert_eq!(v.node_type, VERIFY_NODE_TYPE);
        assert_eq!(v.review_required_by.as_deref(), Some("codex"));
        assert!(
            v.prerequisites.is_empty(),
            "verify links by parent_id, not a circular prerequisite"
        );
        assert_eq!(v.parent_id.as_deref(), Some("impl-x"));
    }

    #[test]
    fn target_accepted_only_after_verify_accepts_a_real_attempt() {
        let mut graph = WorkGraph::new("run-1");
        proposed_target(&mut graph, "impl-x", "claude");
        spawn_verify_node(&mut graph, "impl-x", "codex");

        // Not Accepted while the verify is pending.
        assert_ne!(graph.get("impl-x").unwrap().status, NodeStatus::Accepted);

        let outcome = submit_verify_receipt(
            &mut graph,
            &receipt(
                "impl-x",
                "codex",
                &["null input", "concurrent claim"],
                false,
            ),
        );
        assert_eq!(outcome, VerifyOutcome::TargetAccepted);
        assert_eq!(graph.get("impl-x").unwrap().status, NodeStatus::Accepted);
        assert_eq!(
            graph.get(&verify_node_id("impl-x")).unwrap().status,
            NodeStatus::Accepted
        );
    }

    #[test]
    fn lgtm_receipt_is_rejected_and_target_stays_unaccepted() {
        let mut graph = WorkGraph::new("run-1");
        proposed_target(&mut graph, "impl-x", "claude");
        spawn_verify_node(&mut graph, "impl-x", "codex");

        // No failure mode attempted = LGTM = rejected by the gate.
        let outcome = submit_verify_receipt(&mut graph, &receipt("impl-x", "codex", &[], false));
        assert_eq!(outcome, VerifyOutcome::NotAFalsificationAttempt);
        assert_ne!(graph.get("impl-x").unwrap().status, NodeStatus::Accepted);
    }

    #[test]
    fn defect_found_reopens_the_target() {
        let mut graph = WorkGraph::new("run-1");
        proposed_target(&mut graph, "impl-x", "claude");
        spawn_verify_node(&mut graph, "impl-x", "codex");

        let outcome = submit_verify_receipt(
            &mut graph,
            &receipt("impl-x", "codex", &["off-by-one in epoch"], true),
        );
        assert_eq!(outcome, VerifyOutcome::DefectFound);
        // The verify did its job (Accepted); the target is reopened (Rejected).
        assert_eq!(
            graph.get(&verify_node_id("impl-x")).unwrap().status,
            NodeStatus::Accepted
        );
        assert_eq!(graph.get("impl-x").unwrap().status, NodeStatus::Rejected);
    }

    #[test]
    fn the_implementer_cannot_review_its_own_node() {
        let mut graph = WorkGraph::new("run-1");
        proposed_target(&mut graph, "impl-x", "claude");
        spawn_verify_node(&mut graph, "impl-x", "codex");
        let outcome =
            submit_verify_receipt(&mut graph, &receipt("impl-x", "claude", &["tried"], false));
        assert_eq!(outcome, VerifyOutcome::WrongReviewer);
        assert_ne!(graph.get("impl-x").unwrap().status, NodeStatus::Accepted);
    }
}
