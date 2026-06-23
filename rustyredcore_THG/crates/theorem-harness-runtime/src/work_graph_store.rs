//! GraphStore persistence for the multi-head work graph (v0.1 list item #2).
//!
//! Core (`theorem_harness_core::work_graph`) stays pure; this module owns durable
//! state and replay. A `TaskNode` persists as a graph node (label `TaskNode`)
//! with the whole node serialized as properties, plus structural edges
//! (`PREREQUISITE_OF`, `REFINED_INTO`, `CLAIMED_BY`). A run's work graph
//! reconstructs by querying its `TaskNode`s.
//!
//! `claim_task_node_durable` does read -> pure CAS -> write. It is atomic across
//! heads when executed in the single harness-server handler process: the two
//! head processes call it over MCP and the server serializes them, and the
//! re-read inside the call is what makes the compare-and-swap real (a head that
//! read a stale epoch loses on the re-read). A store-level CAS primitive for a
//! truly concurrent (multi-writer) store is the follow-up; the epoch check makes
//! any lost write detectable rather than silent.

use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreError, NodeQuery, NodeRecord};
use serde_json::{json, Value};
use theorem_harness_core::work_graph::{
    claim_task_node, ClaimOutcome, Millis, TaskNode, WorkGraph,
};

pub const TASK_NODE_LABEL: &str = "TaskNode";
pub const EDGE_PREREQUISITE_OF: &str = "PREREQUISITE_OF";
pub const EDGE_REFINED_INTO: &str = "REFINED_INTO";
pub const EDGE_CLAIMED_BY: &str = "CLAIMED_BY";

#[derive(Clone, Debug)]
pub enum WorkGraphStoreError {
    UnknownNode { run_id: String, node_id: String },
    Store(String),
    Serialization(String),
    Deserialization(String),
}

impl std::fmt::Display for WorkGraphStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkGraphStoreError::UnknownNode { run_id, node_id } => {
                write!(f, "unknown task node {node_id} in run {run_id}")
            }
            WorkGraphStoreError::Store(message) => write!(f, "store error: {message}"),
            WorkGraphStoreError::Serialization(message) => {
                write!(f, "serialization error: {message}")
            }
            WorkGraphStoreError::Deserialization(message) => {
                write!(f, "deserialization error: {message}")
            }
        }
    }
}

impl std::error::Error for WorkGraphStoreError {}

impl From<GraphStoreError> for WorkGraphStoreError {
    fn from(error: GraphStoreError) -> Self {
        WorkGraphStoreError::Store(format!("{}: {}", error.code, error.message))
    }
}

type Result<T> = std::result::Result<T, WorkGraphStoreError>;

pub fn task_node_graph_id(run_id: &str, node_id: &str) -> String {
    format!("work-graph:{run_id}:node:{node_id}")
}

fn task_node_record(node: &TaskNode) -> Result<NodeRecord> {
    let properties = serde_json::to_value(node)
        .map_err(|error| WorkGraphStoreError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        task_node_graph_id(&node.run_id, &node.id),
        [TASK_NODE_LABEL],
        properties,
    ))
}

fn refined_into_edge(run_id: &str, parent: &str, child: &str) -> EdgeRecord {
    EdgeRecord::new(
        format!("work-graph:{run_id}:refined-into:{parent}->{child}"),
        task_node_graph_id(run_id, parent),
        EDGE_REFINED_INTO,
        task_node_graph_id(run_id, child),
        json!({ "run_id": run_id }),
    )
}

fn prerequisite_of_edge(run_id: &str, prerequisite: &str, node: &str) -> EdgeRecord {
    EdgeRecord::new(
        format!("work-graph:{run_id}:prereq-of:{prerequisite}->{node}"),
        task_node_graph_id(run_id, prerequisite),
        EDGE_PREREQUISITE_OF,
        task_node_graph_id(run_id, node),
        json!({ "run_id": run_id }),
    )
}

fn node_exists<S: GraphStore>(store: &S, run_id: &str, node_id: &str) -> bool {
    store
        .get_node(&task_node_graph_id(run_id, node_id))
        .is_some()
}

/// Persist a node plus its TaskNode-to-TaskNode structural edges.
///
/// The store rejects edges to missing endpoints, so an edge is emitted only when
/// the other endpoint already exists; a not-yet-persisted endpoint simply defers
/// its edge. Reconstruction (`load_work_graph`) uses the node's own `parent_id`
/// and `prerequisites` fields as the source of truth, so the edges are a graph-
/// query convenience, never load-bearing. `CLAIMED_BY` is deferred until heads
/// are first-class graph nodes (the `EDGE_CLAIMED_BY` label is reserved); the
/// claim owner lives on the node as a property meanwhile.
pub fn persist_task_node<S: GraphStore>(store: &mut S, node: &TaskNode) -> Result<()> {
    store.upsert_node(task_node_record(node)?)?;
    if let Some(parent) = node.parent_id.as_deref() {
        if node_exists(store, &node.run_id, parent) {
            store.upsert_edge(refined_into_edge(&node.run_id, parent, &node.id))?;
        }
    }
    for prerequisite in &node.prerequisites {
        if node_exists(store, &node.run_id, prerequisite) {
            store.upsert_edge(prerequisite_of_edge(&node.run_id, prerequisite, &node.id))?;
        }
    }
    Ok(())
}

/// Persist every node of an in-memory work graph (seed / bulk write).
pub fn persist_work_graph<S: GraphStore>(store: &mut S, graph: &WorkGraph) -> Result<()> {
    for node in graph.nodes.values() {
        persist_task_node(store, node)?;
    }
    Ok(())
}

pub fn load_task_node<S: GraphStore>(
    store: &S,
    run_id: &str,
    node_id: &str,
) -> Result<Option<TaskNode>> {
    match store.get_node(&task_node_graph_id(run_id, node_id)) {
        Some(record) => {
            let node = serde_json::from_value::<TaskNode>(record.properties.clone())
                .map_err(|error| WorkGraphStoreError::Deserialization(error.to_string()))?;
            Ok(Some(node))
        }
        None => Ok(None),
    }
}

/// Reconstruct a run's work graph from its persisted nodes (replay / queue read).
pub fn load_work_graph<S: GraphStore>(store: &S, run_id: &str) -> Result<WorkGraph> {
    let mut graph = WorkGraph::new(run_id);
    let query = NodeQuery::label(TASK_NODE_LABEL)
        .with_property("run_id", Value::String(run_id.to_string()));
    for record in store.query_nodes(query) {
        let node = serde_json::from_value::<TaskNode>(record.properties)
            .map_err(|error| WorkGraphStoreError::Deserialization(error.to_string()))?;
        graph.insert(node);
    }
    Ok(graph)
}

/// Durable compare-and-swap claim: read the node, run the pure CAS, persist only
/// on a win. The re-read inside this call is the atomicity boundary when run in a
/// single handler process.
pub fn claim_task_node_durable<S: GraphStore>(
    store: &mut S,
    run_id: &str,
    node_id: &str,
    head_id: &str,
    expected_epoch: u64,
    now: Millis,
    lease_ttl: Millis,
) -> Result<ClaimOutcome> {
    let mut node = load_task_node(store, run_id, node_id)?.ok_or_else(|| {
        WorkGraphStoreError::UnknownNode {
            run_id: run_id.to_string(),
            node_id: node_id.to_string(),
        }
    })?;
    let outcome = claim_task_node(&mut node, head_id, expected_epoch, now, lease_ttl);
    if matches!(outcome, ClaimOutcome::Won { .. }) {
        persist_task_node(store, &node)?;
    }
    Ok(outcome)
}

/// Durable claim-and-refine: the live owner declares discovered file scope and
/// persists the split children. Mirrors `WorkGraph::refine` against the store.
pub fn refine_task_node_durable<S: GraphStore>(
    store: &mut S,
    run_id: &str,
    parent_id: &str,
    head_id: &str,
    discovered_file_scope: Vec<String>,
    children: Vec<TaskNode>,
    now: Millis,
) -> Result<()> {
    let mut parent = load_task_node(store, run_id, parent_id)?.ok_or_else(|| {
        WorkGraphStoreError::UnknownNode {
            run_id: run_id.to_string(),
            node_id: parent_id.to_string(),
        }
    })?;
    let owns = parent
        .claim
        .as_ref()
        .is_some_and(|lease| lease.owner == head_id && !lease.is_expired(now));
    if !owns {
        return Err(WorkGraphStoreError::Store(format!(
            "head {head_id} is not the live owner of {parent_id}"
        )));
    }
    parent.file_scope = discovered_file_scope;
    persist_task_node(store, &parent)?;
    for mut child in children {
        child.parent_id = Some(parent_id.to_string());
        child.run_id = run_id.to_string();
        persist_task_node(store, &child)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use theorem_harness_core::work_graph::NodeStatus;

    const TTL: Millis = 100;

    fn open_node(run_id: &str, id: &str) -> TaskNode {
        TaskNode::open(id, run_id, "rust_impl", format!("do {id}"), "seed")
    }

    #[test]
    fn persist_and_load_round_trips() {
        let mut store = InMemoryGraphStore::default();
        let node = open_node("run-1", "n1");
        persist_task_node(&mut store, &node).unwrap();

        let loaded = load_task_node(&store, "run-1", "n1").unwrap().unwrap();
        assert_eq!(loaded, node);

        let graph = load_work_graph(&store, "run-1").unwrap();
        assert_eq!(graph.nodes.len(), 1);
        assert_eq!(graph.get("n1"), Some(&node));
    }

    #[test]
    fn durable_claim_persists_on_win_and_reread_rejects_stale() {
        let mut store = InMemoryGraphStore::default();
        persist_task_node(&mut store, &open_node("run-1", "create-crate")).unwrap();

        // Head A wins, the claim is durable.
        let a = claim_task_node_durable(&mut store, "run-1", "create-crate", "claude", 0, 0, TTL)
            .unwrap();
        assert!(matches!(a, ClaimOutcome::Won { epoch: 1 }));
        let stored = load_task_node(&store, "run-1", "create-crate")
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, NodeStatus::Claimed);
        assert_eq!(stored.claim.as_ref().unwrap().owner, "claude");

        // Head B raced with the same stale expected_epoch (0); the re-read finds
        // epoch 1, so B loses. The duplicate-crate, killed at the durable layer.
        let b = claim_task_node_durable(&mut store, "run-1", "create-crate", "codex", 0, 0, TTL)
            .unwrap();
        assert!(matches!(b, ClaimOutcome::Lost { .. }));
        assert_eq!(
            load_task_node(&store, "run-1", "create-crate")
                .unwrap()
                .unwrap()
                .claim
                .unwrap()
                .owner,
            "claude",
            "the loser did not overwrite the owner"
        );
    }

    #[test]
    fn unknown_node_claim_errors() {
        let mut store = InMemoryGraphStore::default();
        let outcome = claim_task_node_durable(&mut store, "run-1", "ghost", "claude", 0, 0, TTL);
        assert!(matches!(
            outcome,
            Err(WorkGraphStoreError::UnknownNode { .. })
        ));
    }

    #[test]
    fn durable_refine_persists_children_and_replays() {
        let mut store = InMemoryGraphStore::default();
        persist_task_node(&mut store, &open_node("run-1", "ship")).unwrap();
        claim_task_node_durable(&mut store, "run-1", "ship", "claude", 0, 0, TTL).unwrap();

        refine_task_node_durable(
            &mut store,
            "run-1",
            "ship",
            "claude",
            vec!["router.rs".into()],
            vec![open_node("run-1", "child-a"), open_node("run-1", "child-b")],
            5,
        )
        .unwrap();

        // The whole graph replays from the store: parent scope + both children.
        let graph = load_work_graph(&store, "run-1").unwrap();
        assert_eq!(graph.nodes.len(), 3);
        assert_eq!(graph.get("ship").unwrap().file_scope, vec!["router.rs"]);
        assert_eq!(
            graph.get("child-a").unwrap().parent_id.as_deref(),
            Some("ship")
        );

        // A non-owner cannot refine.
        let denied =
            refine_task_node_durable(&mut store, "run-1", "ship", "codex", vec![], vec![], 5);
        assert!(denied.is_err());
    }
}
