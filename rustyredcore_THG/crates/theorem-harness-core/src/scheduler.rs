//! The scheduler (multi-head run execution, v0.1): what a head takes next.
//!
//! This is the connective tissue that turns the primitives (claim CAS + lease,
//! head fitness, verify-with-teeth) into one loop. Two rules:
//!
//! - Idle heads verify first. If a verify node assigned to this head has a target
//!   that proposed a patch, claim it. Review is the default idle behaviour, so
//!   the heads are essentially always refuting each other's work, which is the
//!   only reason two heads beat two of the same.
//! - Otherwise route by fitness. Take a ready, claimable implementation node
//!   whose node-type routes to this head (work-stealing plus the learned,
//!   explore-floored routing). The other head's preferred types are left for it.
//!
//! Deterministic given `explore_token`, so a multi-head run replays.

use crate::head_fitness::HeadFitness;
use crate::work_graph::{Millis, NodeStatus, WorkGraph};
use crate::work_graph_verify::VERIFY_NODE_TYPE;

/// A verify node is ready when its target has proposed a patch (the work is done
/// and is awaiting refutation). This is deliberately NOT the generic
/// prerequisite-accepted readiness, which is circular for a verify node (its
/// target is accepted only through the verify itself).
fn verify_target_proposed(graph: &WorkGraph, verify_id: &str) -> bool {
    graph
        .get(verify_id)
        .and_then(|verify| verify.parent_id.as_deref())
        .and_then(|target| graph.get(target))
        .is_some_and(|target| target.status == NodeStatus::PatchProposed)
}

/// The id of the next node this head should claim, or `None` if there is nothing
/// for it right now.
pub fn next_for_head(
    graph: &WorkGraph,
    fitness: &HeadFitness,
    head: &str,
    explore_token: u32,
    now: Millis,
) -> Option<String> {
    // 1. Idle heads verify first: a claimable verify node assigned to this head
    //    whose target has proposed a patch.
    for node in graph.nodes.values() {
        if node.node_type == VERIFY_NODE_TYPE
            && node.review_required_by.as_deref() == Some(head)
            && node.is_claimable(now)
            && verify_target_proposed(graph, &node.id)
        {
            return Some(node.id.clone());
        }
    }
    // 2. Otherwise route a ready implementation node by fitness.
    for node in graph.nodes.values() {
        if node.node_type != VERIFY_NODE_TYPE
            && node.is_claimable(now)
            && graph.is_ready(&node.id)
            && fitness.route(&node.node_type, explore_token).as_deref() == Some(head)
        {
            return Some(node.id.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::head_fitness::NodeResult;
    use crate::work_graph::{ClaimOutcome, TaskNode};
    use crate::work_graph_verify::{
        spawn_verify_node, submit_verify_receipt, verify_node_id, VerifyOutcome, VerifyReceipt,
    };

    const TTL: Millis = 100;

    fn graph_and_fitness() -> (WorkGraph, HeadFitness) {
        (
            WorkGraph::new("run-1"),
            HeadFitness::new(vec!["claude".to_string(), "codex".to_string()]),
        )
    }

    fn refutation(target: &str, reviewer: &str, defect: bool) -> VerifyReceipt {
        VerifyReceipt {
            target_node_id: target.to_string(),
            reviewer: reviewer.to_string(),
            attempted_failure_modes: vec!["stale-epoch reclaim".into(), "empty file scope".into()],
            commands_run: vec!["cargo test -p theorem-harness-core".into()],
            defect_found: defect,
            waived_risks: Vec::new(),
        }
    }

    /// The whole v0.1 spine, end to end, across two heads and one node:
    /// route -> claim (CAS + lease) -> propose -> auto-spawn verify for the other
    /// head -> idle-head claims the verify -> refute with teeth -> accept ->
    /// record fitness -> the run drains. Every piece was unit-tested alone; this
    /// proves they compose into a loop.
    #[test]
    fn one_node_runs_the_full_lifecycle_across_two_heads() {
        let (mut graph, mut fitness) = graph_and_fitness();
        graph.insert(TaskNode::open(
            "impl-x",
            "run-1",
            "rust_impl",
            "ship x",
            "seed",
        ));

        // The scheduler routes impl-x to a head. Cold start, all rates equal, the
        // argmax tie goes to the first head: claude.
        assert_eq!(
            next_for_head(&graph, &fitness, "claude", 999, 0).as_deref(),
            Some("impl-x")
        );

        // Claude claims it (the durable CAS proved this is collision-free), works
        // it, and proposes a patch.
        assert!(matches!(
            graph.claim("impl-x", "claude", 0, 0, TTL).unwrap(),
            ClaimOutcome::Won { epoch: 1 }
        ));
        graph.nodes.get_mut("impl-x").unwrap().status = NodeStatus::PatchProposed;

        // Completing the node auto-spawns the verify node for the OTHER head.
        let verify_id = spawn_verify_node(&mut graph, "impl-x", "codex").unwrap();

        // The scheduler now hands codex the verify first (idle heads verify), and
        // hands claude nothing (its node is awaiting review).
        assert_eq!(
            next_for_head(&graph, &fitness, "codex", 999, 0).as_deref(),
            Some(verify_id.as_str())
        );
        assert_eq!(next_for_head(&graph, &fitness, "claude", 999, 0), None);

        // Codex claims the verify and refutes with teeth (a real attempt, no
        // defect found): the target is accepted.
        assert!(matches!(
            graph.claim(&verify_id, "codex", 0, 0, TTL).unwrap(),
            ClaimOutcome::Won { .. }
        ));
        assert_eq!(
            submit_verify_receipt(&mut graph, &refutation("impl-x", "codex", false)),
            VerifyOutcome::TargetAccepted
        );
        assert_eq!(graph.get("impl-x").unwrap().status, NodeStatus::Accepted);

        // Record the outcome: claude's rust_impl output survived verification, so
        // fitness will route rust_impl to claude more often (with the floor).
        fitness.record("rust_impl", "claude", NodeResult::Accepted);
        assert!(fitness.rate("rust_impl", "claude") > 0.5);

        // The run drained: nothing claimable remains for either head.
        assert_eq!(next_for_head(&graph, &fitness, "claude", 999, 0), None);
        assert_eq!(next_for_head(&graph, &fitness, "codex", 999, 0), None);
    }

    /// A defect found in verification reopens the target (Rejected); the loop is
    /// closed because rejected work flows back to be redone.
    #[test]
    fn a_defect_reopens_the_target() {
        let (mut graph, _fitness) = graph_and_fitness();
        graph.insert(TaskNode::open(
            "impl-x",
            "run-1",
            "rust_impl",
            "ship x",
            "seed",
        ));
        graph.claim("impl-x", "claude", 0, 0, TTL).unwrap();
        graph.nodes.get_mut("impl-x").unwrap().status = NodeStatus::PatchProposed;
        spawn_verify_node(&mut graph, "impl-x", "codex");

        let outcome = submit_verify_receipt(&mut graph, &refutation("impl-x", "codex", true));
        assert_eq!(outcome, VerifyOutcome::DefectFound);
        assert_eq!(graph.get("impl-x").unwrap().status, NodeStatus::Rejected);
        // The verify itself is accepted (finding the defect was its job).
        assert_eq!(
            graph.get(&verify_node_id("impl-x")).unwrap().status,
            NodeStatus::Accepted
        );
    }

    /// An explore token routes the SAME node-type to the other head: the floor
    /// that keeps both heads in every type, so the scheduler cannot monoculture.
    #[test]
    fn explore_token_routes_impl_to_the_other_head() {
        let (mut graph, mut fitness) = graph_and_fitness();
        graph.insert(TaskNode::open(
            "impl-x",
            "run-1",
            "rust_impl",
            "ship x",
            "seed",
        ));
        // Make claude the clear argmax for rust_impl.
        for _ in 0..6 {
            fitness.record("rust_impl", "claude", NodeResult::Accepted);
            fitness.record("rust_impl", "codex", NodeResult::Rejected);
        }
        // Exploit hands impl-x to claude...
        assert_eq!(
            next_for_head(&graph, &fitness, "claude", 999, 0).as_deref(),
            Some("impl-x")
        );
        // ...but an explore token (token 1 -> heads[1] = codex) hands it to codex,
        // and not to claude. The losing head keeps a share.
        assert_eq!(next_for_head(&graph, &fitness, "claude", 1, 0), None);
        assert_eq!(
            next_for_head(&graph, &fitness, "codex", 1, 0).as_deref(),
            Some("impl-x")
        );
    }
}
