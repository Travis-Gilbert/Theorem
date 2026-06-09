//! GraphStore persistence for the composed-agent `AgentBinding`.
//!
//! The runtime seam for the binding kernel in `theorem-harness-core`
//! (`agent_binding.rs`): persist an `AgentBinding`, its lifecycle events, and
//! its versioned scratchpad revisions as graph nodes plus append-chain edges,
//! so a composed agent's binding survives a `RedCoreGraphStore` reopen and
//! replays deterministically. This is the sibling of `event_log.rs` (which
//! persists the single-agent `RunState`/`EventState`); it follows the same
//! idempotent upsert + contiguous-append + conflict-detection discipline.
//!
//! Storage mapping (spec Part 6): the in-process `GraphStore` is Theorem's
//! canonical durable layer. The binding node carries the full binding
//! (identity, composition, scopes, lifecycle, and the scratchpad it owns);
//! each scratchpad revision is also persisted as its own node + chain edge so
//! the versioned document is queryable without loading the whole binding.

use crate::writing_style;
use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::{
    apply_binding_transition, AgentBinding, BindingError, BindingEventState,
    BindingTransitionInput, BindingTransitionResult, ScratchpadRevision,
};

pub type BindingRuntimeResult<T> = Result<T, BindingRuntimeError>;

/// Errors from persisting a binding run: kernel guard violations, store I/O,
/// (de)serialization, and append-chain integrity failures.
#[derive(Clone, Debug, PartialEq)]
pub enum BindingRuntimeError {
    Kernel(BindingError),
    Store(GraphStoreError),
    Serialization(String),
    Deserialization(String),
    EventConflict {
        event_id: String,
        run_id: String,
        seq: u64,
    },
    EventGap {
        run_id: String,
        expected_previous_seq: u64,
    },
}

impl fmt::Display for BindingRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::EventConflict {
                event_id,
                run_id,
                seq,
            } => write!(
                f,
                "binding event log conflict at {event_id} for run {run_id} seq {seq}"
            ),
            Self::EventGap {
                run_id,
                expected_previous_seq,
            } => write!(
                f,
                "binding event log gap for run {run_id}: missing previous seq {expected_previous_seq}"
            ),
        }
    }
}

impl Error for BindingRuntimeError {}

impl From<BindingError> for BindingRuntimeError {
    fn from(value: BindingError) -> Self {
        Self::Kernel(value)
    }
}

impl From<GraphStoreError> for BindingRuntimeError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

/// Apply a binding lifecycle transition through the kernel and persist the
/// resulting binding, event, and scratchpad revisions to the store.
pub fn append_binding_transition<S: GraphStore>(
    store: &mut S,
    binding: AgentBinding,
    transition: BindingTransitionInput,
) -> BindingRuntimeResult<BindingTransitionResult> {
    let transition = writing_style::enrich_binding_transition(transition);
    let result = apply_binding_transition(binding, transition)?;
    persist_binding_transition_result(store, &result)?;
    Ok(result)
}

/// Persist a `BindingTransitionResult`: the binding node (with its scratchpad
/// revisions), the binding event node, and the append-chain edges.
pub fn persist_binding_transition_result<S: GraphStore>(
    store: &mut S,
    result: &BindingTransitionResult,
) -> BindingRuntimeResult<()> {
    ensure_binding_append_position(store, &result.event)?;
    persist_binding(store, &result.binding, &result.state_hash_after)?;

    let event_node = binding_event_node(&result.event)?;
    let event_id = event_node.id.clone();
    let event_already_present = match store.get_node(&event_id) {
        Some(existing) if binding_event_matches(existing, &result.event) => true,
        Some(_) => {
            return Err(BindingRuntimeError::EventConflict {
                event_id,
                run_id: result.event.run_id.clone(),
                seq: result.event.seq,
            });
        }
        None => false,
    };
    if !event_already_present {
        upsert_node_if_changed(store, event_node)?;
    }

    upsert_edge_if_changed(store, binding_event_of_edge(&result.event)?)?;
    if result.event.seq > 1 {
        upsert_edge_if_changed(store, previous_binding_event_edge(&result.event)?)?;
    }
    Ok(())
}

pub fn persist_binding_run_result<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
    events: &[BindingEventState],
) -> BindingRuntimeResult<()> {
    let state_hash = events
        .last()
        .map(|event| event.state_hash_after.as_str())
        .unwrap_or_default();
    persist_binding(store, binding, state_hash)?;
    for event in events {
        persist_binding_event_state(store, event)?;
    }
    Ok(())
}

pub fn persist_binding_event_state<S: GraphStore>(
    store: &mut S,
    event: &BindingEventState,
) -> BindingRuntimeResult<()> {
    ensure_binding_append_position(store, event)?;
    let event_node = binding_event_node(event)?;
    let event_id = event_node.id.clone();
    let event_already_present = match store.get_node(&event_id) {
        Some(existing) if binding_event_matches(existing, event) => true,
        Some(_) => {
            return Err(BindingRuntimeError::EventConflict {
                event_id,
                run_id: event.run_id.clone(),
                seq: event.seq,
            });
        }
        None => false,
    };
    if !event_already_present {
        upsert_node_if_changed(store, event_node)?;
    }

    upsert_edge_if_changed(store, binding_event_of_edge(event)?)?;
    if event.seq > 1 {
        upsert_edge_if_changed(store, previous_binding_event_edge(event)?)?;
    }
    Ok(())
}

/// Persist a binding node plus every scratchpad revision it owns (idempotent).
/// Used both by transition persistence and directly after a scratchpad append
/// (which happens outside the lifecycle transitions).
pub fn persist_binding<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
    state_hash: &str,
) -> BindingRuntimeResult<()> {
    upsert_node_if_changed(store, binding_node(binding, state_hash)?)?;
    persist_scratchpad_revisions(store, binding)?;
    Ok(())
}

fn persist_scratchpad_revisions<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
) -> BindingRuntimeResult<()> {
    let document_id = &binding.working_memory_scope.scratchpad.document_id;
    let run_id = &binding.lifecycle.run_id;
    for revision in &binding.working_memory_scope.scratchpad.revisions {
        upsert_node_if_changed(store, scratchpad_revision_node(document_id, revision)?)?;
        upsert_edge_if_changed(
            store,
            scratchpad_revision_of_edge(document_id, run_id, revision),
        )?;
        if revision.seq > 1 {
            upsert_edge_if_changed(
                store,
                previous_scratchpad_revision_edge(document_id, revision),
            )?;
        }
    }
    Ok(())
}

pub fn load_binding<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> BindingRuntimeResult<Option<AgentBinding>> {
    store
        .get_node(&binding_node_id(run_id))
        .map(|node| {
            serde_json::from_value::<AgentBinding>(node.properties.clone())
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .transpose()
}

pub fn load_binding_events<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> BindingRuntimeResult<Vec<BindingEventState>> {
    let mut events = store
        .query_nodes(
            NodeQuery::label("BindingEvent")
                .with_property("run_id", Value::String(run_id.to_string())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<BindingEventState>(node.properties)
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<BindingRuntimeResult<Vec<_>>>()?;
    events.sort_by_key(|event| event.seq);
    Ok(events)
}

pub fn load_scratchpad_revisions<S: GraphStore>(
    store: &S,
    document_id: &str,
) -> BindingRuntimeResult<Vec<ScratchpadRevision>> {
    let mut revisions = store
        .query_nodes(
            NodeQuery::label("ScratchpadRevision")
                .with_property("document_id", Value::String(document_id.to_string())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<ScratchpadRevision>(node.properties)
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<BindingRuntimeResult<Vec<_>>>()?;
    revisions.sort_by_key(|revision| revision.seq);
    Ok(revisions)
}

pub fn binding_node_id(run_id: &str) -> String {
    format!("harness:binding:{run_id}")
}

pub fn binding_event_node_id(run_id: &str, seq: u64) -> String {
    format!("harness:bindingevent:{run_id}:{seq:020}")
}

pub fn scratchpad_revision_node_id(document_id: &str, seq: u64) -> String {
    format!("harness:scratchrev:{document_id}:{seq:020}")
}

fn binding_node(binding: &AgentBinding, state_hash: &str) -> BindingRuntimeResult<NodeRecord> {
    let mut properties = serde_json::to_value(binding)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    properties["state_hash"] = Value::String(state_hash.to_string());
    Ok(NodeRecord::new(
        binding_node_id(&binding.lifecycle.run_id),
        ["AgentBinding"],
        properties,
    ))
}

fn binding_event_node(event: &BindingEventState) -> BindingRuntimeResult<NodeRecord> {
    let properties = serde_json::to_value(event)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        binding_event_node_id(&event.run_id, event.seq),
        ["BindingEvent"],
        properties,
    ))
}

fn scratchpad_revision_node(
    document_id: &str,
    revision: &ScratchpadRevision,
) -> BindingRuntimeResult<NodeRecord> {
    let mut properties = serde_json::to_value(revision)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    // The struct has no document_id field; the node carries it so revisions are
    // queryable per document. serde ignores the extra field on deserialization.
    properties["document_id"] = Value::String(document_id.to_string());
    Ok(NodeRecord::new(
        scratchpad_revision_node_id(document_id, revision.seq),
        ["ScratchpadRevision"],
        properties,
    ))
}

fn binding_event_of_edge(event: &BindingEventState) -> BindingRuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:edge:binding-event-of:{}:{:020}",
            event.run_id, event.seq
        ),
        binding_event_node_id(&event.run_id, event.seq),
        "HARNESS_BINDING_EVENT_OF",
        binding_node_id(&event.run_id),
        json!({
            "run_id": event.run_id,
            "seq": event.seq,
            "type": event.event_type,
            "binding_status_before": event.binding_status_before,
            "binding_status_after": event.binding_status_after,
            "state_hash_before": event.state_hash_before,
            "state_hash_after": event.state_hash_after,
        }),
    ))
}

fn previous_binding_event_edge(event: &BindingEventState) -> BindingRuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:edge:binding-event-next:{}:{:020}",
            event.run_id, event.seq
        ),
        binding_event_node_id(&event.run_id, event.seq - 1),
        "HARNESS_BINDING_EVENT_NEXT",
        binding_event_node_id(&event.run_id, event.seq),
        json!({
            "run_id": event.run_id,
            "from_seq": event.seq - 1,
            "to_seq": event.seq,
        }),
    ))
}

fn scratchpad_revision_of_edge(
    document_id: &str,
    run_id: &str,
    revision: &ScratchpadRevision,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrev-of:{}:{:020}",
            document_id, revision.seq
        ),
        scratchpad_revision_node_id(document_id, revision.seq),
        "HARNESS_SCRATCHPAD_REVISION_OF",
        binding_node_id(run_id),
        json!({
            "document_id": document_id,
            "seq": revision.seq,
            "actor_head_id": revision.actor_head_id,
            "content_hash": revision.content_hash,
        }),
    )
}

fn previous_scratchpad_revision_edge(
    document_id: &str,
    revision: &ScratchpadRevision,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrev-next:{}:{:020}",
            document_id, revision.seq
        ),
        scratchpad_revision_node_id(document_id, revision.seq - 1),
        "HARNESS_SCRATCHPAD_REVISION_NEXT",
        scratchpad_revision_node_id(document_id, revision.seq),
        json!({
            "document_id": document_id,
            "from_seq": revision.seq - 1,
            "to_seq": revision.seq,
        }),
    )
}

fn ensure_binding_append_position<S: GraphStore>(
    store: &S,
    event: &BindingEventState,
) -> BindingRuntimeResult<()> {
    if event.seq <= 1 {
        return Ok(());
    }
    let previous_seq = event.seq - 1;
    if store
        .get_node(&binding_event_node_id(&event.run_id, previous_seq))
        .is_none()
    {
        return Err(BindingRuntimeError::EventGap {
            run_id: event.run_id.clone(),
            expected_previous_seq: previous_seq,
        });
    }
    Ok(())
}

fn binding_event_matches(existing: &NodeRecord, event: &BindingEventState) -> bool {
    existing.properties.get("run_id").and_then(Value::as_str) == Some(event.run_id.as_str())
        && existing.properties.get("seq").and_then(Value::as_u64) == Some(event.seq)
        && existing.properties.get("type").and_then(Value::as_str)
            == Some(event.event_type.as_str())
        && existing.properties.get("payload") == Some(&Value::Object(event.payload.clone()))
        && existing
            .properties
            .get("state_hash_before")
            .and_then(Value::as_str)
            == Some(event.state_hash_before.as_str())
        && existing
            .properties
            .get("state_hash_after")
            .and_then(Value::as_str)
            == Some(event.state_hash_after.as_str())
}

fn upsert_node_if_changed<S: GraphStore>(store: &mut S, node: NodeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_node(&node.id)
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed<S: GraphStore>(store: &mut S, edge: EdgeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_edge(&edge.id)
        .map(|existing| {
            !existing.tombstone
                && existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.edge_type == edge.edge_type
                && existing.properties == edge.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_edge(edge)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, RedCoreGraphStore, RedCoreOptions};
    use serde_json::{json, Map};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use theorem_harness_core::{
        hash_agent_binding, AgentHead, BindingBudgetScope, BindingComposition, BindingIdentity,
        HeadCostProfile, HeadKind, HeadReliabilityProfile, HeadTransport, Payload, TraceTier,
    };

    const TS: &str = "2026-06-02T00:00:00Z";

    #[test]
    fn append_binding_transition_persists_binding_event_and_edges() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let run_id = binding.lifecycle.run_id.clone();

        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();
        let probed = append_binding_transition(
            &mut store,
            resolved.binding,
            transition(
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude", "deepseek"] }),
            ),
        )
        .unwrap();

        assert!(store.get_node(&binding_node_id(&run_id)).is_some());
        assert!(store.get_node(&binding_event_node_id(&run_id, 1)).is_some());
        assert!(store.get_node(&binding_event_node_id(&run_id, 2)).is_some());
        assert!(store
            .get_edge(&format!("harness:edge:binding-event-of:{run_id}:{:020}", 2))
            .is_some());
        assert!(store
            .get_edge(&format!(
                "harness:edge:binding-event-next:{run_id}:{:020}",
                2
            ))
            .is_some());

        let events = load_binding_events(&store, &run_id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].event_type, "HEADS.PROBED");

        let loaded = load_binding(&store, &run_id).unwrap().unwrap();
        assert_eq!(loaded.lifecycle.status, probed.binding.lifecycle.status);
        assert_eq!(loaded.lifecycle.status, "heads_probed");
        assert_eq!(loaded.identity.agent_id, "theorem");
    }

    #[test]
    fn loaded_binding_hash_matches_persisted_state() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let run_id = binding.lifecycle.run_id.clone();
        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();

        let loaded = load_binding(&store, &run_id).unwrap().unwrap();
        assert_eq!(hash_agent_binding(&loaded), resolved.state_hash_after);
    }

    #[test]
    fn redcore_reopens_persisted_binding() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-binding-store-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let run_id;

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let binding = fixture_binding();
            run_id = binding.lifecycle.run_id.clone();
            let resolved = append_binding_transition(
                &mut store,
                binding,
                transition(
                    "BINDING.RESOLVED",
                    json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
                ),
            )
            .unwrap();
            append_binding_transition(
                &mut store,
                resolved.binding,
                transition("HEADS.PROBED", json!({ "probed_head_set": ["claude"] })),
            )
            .unwrap();
        }

        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let events = load_binding_events(&store, &run_id).unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event_type, "HEADS.PROBED");
            let binding = load_binding(&store, &run_id).unwrap().unwrap();
            assert_eq!(binding.lifecycle.status, "heads_probed");
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn scratchpad_revisions_persist_and_chain() {
        let mut store = InMemoryGraphStore::new();
        let mut binding = fixture_binding();
        let document_id = binding.working_memory_scope.scratchpad.document_id.clone();
        binding
            .append_scratchpad_revision("claude", "proposal", "hash:1", Payload::new(), TS)
            .unwrap();
        binding
            .append_scratchpad_revision("deepseek", "critique", "hash:2", Payload::new(), TS)
            .unwrap();

        persist_binding(&mut store, &binding, &hash_agent_binding(&binding)).unwrap();

        let revisions = load_scratchpad_revisions(&store, &document_id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].seq, 1);
        assert_eq!(revisions[1].seq, 2);
        assert_eq!(revisions[1].parent_revision_id, revisions[0].revision_id);
        assert!(store
            .get_edge(&format!(
                "harness:edge:scratchrev-next:{document_id}:{:020}",
                2
            ))
            .is_some());
        assert!(store
            .get_edge(&format!(
                "harness:edge:scratchrev-of:{document_id}:{:020}",
                1
            ))
            .is_some());
    }

    #[test]
    fn binding_append_requires_contiguous_events() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let resolved = apply_binding_transition(
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();
        // Force a non-contiguous seq to prove the append-position guard fires.
        let mut skipped = resolved.clone();
        skipped.event.seq = 3;
        let error = persist_binding_transition_result(&mut store, &skipped).unwrap_err();
        assert_eq!(
            error,
            BindingRuntimeError::EventGap {
                run_id: skipped.event.run_id.clone(),
                expected_previous_seq: 2,
            }
        );
    }

    #[test]
    fn binding_append_rejects_conflicting_event_at_same_sequence() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();

        let mut conflicting = resolved.clone();
        conflicting.event.state_hash_after = "different".to_string();
        conflicting.state_hash_after = "different".to_string();
        let error = persist_binding_transition_result(&mut store, &conflicting).unwrap_err();
        assert_eq!(
            error,
            BindingRuntimeError::EventConflict {
                event_id: binding_event_node_id(&resolved.event.run_id, 1),
                run_id: resolved.event.run_id.clone(),
                seq: 1,
            }
        );
    }

    #[test]
    fn full_binding_lifecycle_persists_and_reopens() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-binding-lifecycle-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let run_id;

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let mut binding = fixture_binding();
            run_id = binding.lifecycle.run_id.clone();
            for (event, payload) in lifecycle_events() {
                binding = drive(&mut store, binding, event, payload);
            }
            assert_eq!(binding.lifecycle.status, "closed");
            assert_eq!(binding.lifecycle.last_event_seq, 16);
        }

        // Reopen the durable store and prove the whole composed-agent run is
        // intact: 16 contiguous lifecycle events, the binding at terminal
        // status, and one recorded head contribution receipt.
        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let events = load_binding_events(&store, &run_id).unwrap();
            assert_eq!(events.len(), 16);
            assert_eq!(events.first().unwrap().event_type, "BINDING.RESOLVED");
            assert_eq!(events.last().unwrap().event_type, "RUN.CLOSED");
            let synthesis = events
                .iter()
                .find(|event| event.event_type == "DRAFTS.SYNTHESIZED")
                .unwrap();
            assert_eq!(
                synthesis.payload["style_receipts"][0]["receipt"]["pack_hash"],
                json!(prose_check::pack_hash())
            );
            for (idx, event) in events.iter().enumerate() {
                assert_eq!(event.seq, (idx as u64) + 1);
            }
            let binding = load_binding(&store, &run_id).unwrap().unwrap();
            assert_eq!(binding.lifecycle.status, "closed");
            assert_eq!(binding.trace_scope.contributions.len(), 1);
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    fn drive(
        store: &mut impl GraphStore,
        binding: AgentBinding,
        event: &str,
        payload: Value,
    ) -> AgentBinding {
        append_binding_transition(store, binding, transition(event, payload))
            .unwrap()
            .binding
    }

    fn lifecycle_events() -> Vec<(&'static str, Value)> {
        vec![
            (
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
            (
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude", "deepseek"] }),
            ),
            (
                "MEMORY_SCOPE.MOUNTED",
                json!({ "scope_id": "bindingscope:theorem", "scratchpad_id": "scratchpad:theorem" }),
            ),
            (
                "CHARTER.COMPILED",
                json!({ "charter_hash": "charter:1", "stance": "grounded composed agent" }),
            ),
            (
                "CAPABILITIES.SELECTED",
                json!({ "capability_scope_hash": "cap:1", "visible_tools": ["datalog"], "callable_tools": ["datalog"] }),
            ),
            (
                "BUDGET.ALLOCATED",
                json!({ "budget_units": 25.0, "max_parallel_heads": 2 }),
            ),
            (
                "RUN.STARTED",
                json!({ "task": "answer with Theorem voice", "started_at": "2026-06-02T00:00:00Z" }),
            ),
            (
                "PRIVATE_WORK.OPENED",
                json!({ "scratchpad_revision_id": "scratchrev:1" }),
            ),
            (
                "HEADS.CONTRIBUTE",
                json!({ "head_id": "claude", "contribution_id": "contrib:1", "contribution_kind": "proposal" }),
            ),
            (
                "DRAFTS.SYNTHESIZED",
                json!({ "synthesis_id": "synth:1", "contributing_heads": ["claude", "deepseek"] }),
            ),
            (
                "PUBLICATION.PROPOSED",
                json!({ "publication_id": "pub:1", "draft_hash": "draft:1" }),
            ),
            (
                "POLICY.CHECKED",
                json!({
                    "policy_receipt_id": "policy:1",
                    "allowed": true,
                    "claims": [{ "text": "grounded", "provenance": "src:1" }]
                }),
            ),
            (
                "PUBLISHED_TO_SUBSTRATE",
                json!({ "publication_id": "pub:1", "substrate_receipt_id": "substrate:1" }),
            ),
            (
                "OUTCOME.RECORDED",
                json!({ "outcome_id": "outcome:1", "accepted": true, "summary": "published" }),
            ),
            (
                "MEMORY_PATCHES.PROPOSED",
                json!({ "patch_ids": ["patch:1"], "review_required": true }),
            ),
            (
                "RUN.CLOSED",
                json!({ "summary": "closed", "closed_by": "claude-code" }),
            ),
        ]
    }

    fn fixture_binding() -> AgentBinding {
        AgentBinding::new(
            BindingIdentity {
                agent_id: "theorem".to_string(),
                owner_id: "travis".to_string(),
                agent_name: "Theorem".to_string(),
                composition_hash: String::new(),
                version: 1,
                trust_tier: "first_party".to_string(),
                active_head_set: vec!["claude".to_string(), "deepseek".to_string()],
            },
            BindingComposition {
                heads: vec![
                    head("claude", "anthropic", "claude", HeadKind::ReasoningCore),
                    head("deepseek", "deepseek", "v4", HeadKind::ReasoningCore),
                ],
            },
            BindingBudgetScope::new("theorem", 100.0, 3),
        )
        .unwrap()
    }

    fn head(head_id: &str, provider: &str, model: &str, kind: HeadKind) -> AgentHead {
        AgentHead {
            head_id: head_id.to_string(),
            display_name: head_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            credential_ref: format!("credential:{head_id}"),
            transport: HeadTransport::Api,
            kind,
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }

    fn transition(event_type: &str, payload: Value) -> BindingTransitionInput {
        BindingTransitionInput::new(
            event_type,
            payload.as_object().cloned().unwrap_or_else(Map::new),
        )
        .at(TS)
    }
}
