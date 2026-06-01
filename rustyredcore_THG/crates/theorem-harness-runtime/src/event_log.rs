use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;
use theorem_harness_core::{
    apply_transition, replay_events, EventState, HarnessError, RunState, TransitionInput,
    TransitionResult,
};

pub type RuntimeResult<T> = Result<T, HarnessRuntimeError>;

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessRuntimeError {
    Kernel(HarnessError),
    Store(GraphStoreError),
    MissingRunId,
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

impl fmt::Display for HarnessRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::MissingRunId => write!(f, "transition replay from storage requires run_id"),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::EventConflict {
                event_id,
                run_id,
                seq,
            } => write!(
                f,
                "event log conflict at {event_id} for run {run_id} seq {seq}"
            ),
            Self::EventGap {
                run_id,
                expected_previous_seq,
            } => write!(
                f,
                "event log gap for run {run_id}: missing previous seq {expected_previous_seq}"
            ),
        }
    }
}

impl Error for HarnessRuntimeError {}

impl From<HarnessError> for HarnessRuntimeError {
    fn from(value: HarnessError) -> Self {
        Self::Kernel(value)
    }
}

impl From<GraphStoreError> for HarnessRuntimeError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

pub fn append_transition<S: GraphStore>(
    store: &mut S,
    state: Option<RunState>,
    transition: TransitionInput,
) -> RuntimeResult<TransitionResult> {
    let result = apply_transition(state, transition)?;
    persist_transition_result(store, &result)?;
    Ok(result)
}

pub fn append_transition_from_store<S: GraphStore>(
    store: &mut S,
    transition: TransitionInput,
) -> RuntimeResult<TransitionResult> {
    let state = if transition.event_type == "RUN.CREATED" {
        if transition.run_id.is_empty() {
            None
        } else {
            load_run(store, &transition.run_id)?
        }
    } else {
        if transition.run_id.is_empty() {
            return Err(HarnessRuntimeError::MissingRunId);
        }
        load_run(store, &transition.run_id)?
    };
    append_transition(store, state, transition)
}

pub fn persist_transition_result<S: GraphStore>(
    store: &mut S,
    result: &TransitionResult,
) -> RuntimeResult<()> {
    let run_node = run_node(&result.run, &result.state_hash_after)?;
    let event_node = event_node(&result.event)?;
    ensure_append_position(store, &result.event)?;
    upsert_node_if_changed(store, run_node)?;

    let event_id = event_node.id.clone();
    let event_already_present = match store.get_node(&event_id) {
        Some(existing) if event_matches(existing, &result.event) => true,
        Some(_) => {
            return Err(HarnessRuntimeError::EventConflict {
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

    upsert_edge_if_changed(store, event_of_run_edge(&result.event)?)?;
    if result.event.seq > 1 {
        upsert_edge_if_changed(store, previous_event_edge(&result.event)?)?;
    }
    Ok(())
}

pub fn load_run<S: GraphStore>(store: &S, run_id: &str) -> RuntimeResult<Option<RunState>> {
    store
        .get_node(&run_node_id(run_id))
        .map(|node| {
            serde_json::from_value::<RunState>(node.properties.clone())
                .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))
        })
        .transpose()
}

pub fn load_events<S: GraphStore>(store: &S, run_id: &str) -> RuntimeResult<Vec<EventState>> {
    let mut events = store
        .query_nodes(
            NodeQuery::label("HarnessEvent")
                .with_property("run_id", Value::String(run_id.to_string())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<EventState>(node.properties)
                .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<RuntimeResult<Vec<_>>>()?;
    events.sort_by_key(|event| event.seq);
    Ok(events)
}

pub fn replay_persisted_run<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> RuntimeResult<Option<RunState>> {
    let events = load_events(store, run_id)?;
    replay_events(&events).map_err(HarnessRuntimeError::Kernel)
}

pub fn run_node_id(run_id: &str) -> String {
    format!("harness:run:{run_id}")
}

pub fn event_node_id(run_id: &str, seq: u64) -> String {
    format!("harness:event:{run_id}:{seq:020}")
}

fn run_node(run: &RunState, state_hash: &str) -> RuntimeResult<NodeRecord> {
    let mut properties = serde_json::to_value(run)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    properties["state_hash"] = Value::String(state_hash.to_string());
    Ok(NodeRecord::new(
        run_node_id(&run.run_id),
        ["HarnessRun"],
        properties,
    ))
}

fn event_node(event: &EventState) -> RuntimeResult<NodeRecord> {
    let properties = serde_json::to_value(event)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        event_node_id(&event.run_id, event.seq),
        ["HarnessEvent"],
        properties,
    ))
}

fn event_of_run_edge(event: &EventState) -> RuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!("harness:edge:event-of:{}:{:020}", event.run_id, event.seq),
        event_node_id(&event.run_id, event.seq),
        "HARNESS_EVENT_OF",
        run_node_id(&event.run_id),
        json!({
            "run_id": event.run_id,
            "seq": event.seq,
            "state_hash_before": event.state_hash_before,
            "state_hash_after": event.state_hash_after,
            "type": event.event_type,
        }),
    ))
}

fn previous_event_edge(event: &EventState) -> RuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!("harness:edge:event-next:{}:{:020}", event.run_id, event.seq),
        event_node_id(&event.run_id, event.seq - 1),
        "HARNESS_EVENT_NEXT",
        event_node_id(&event.run_id, event.seq),
        json!({
            "run_id": event.run_id,
            "from_seq": event.seq - 1,
            "to_seq": event.seq,
        }),
    ))
}

fn ensure_append_position<S: GraphStore>(store: &S, event: &EventState) -> RuntimeResult<()> {
    if event.seq <= 1 {
        return Ok(());
    }
    let previous_seq = event.seq - 1;
    if store
        .get_node(&event_node_id(&event.run_id, previous_seq))
        .is_none()
    {
        return Err(HarnessRuntimeError::EventGap {
            run_id: event.run_id.clone(),
            expected_previous_seq: previous_seq,
        });
    }
    Ok(())
}

fn event_matches(existing: &NodeRecord, event: &EventState) -> bool {
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
    use theorem_harness_core::{hash_run_state, TransitionInput};

    const RUN_ID: &str = "run-runtime-0001";
    const TS: &str = "2026-06-01T00:00:00+00:00";

    #[test]
    fn append_transition_persists_run_event_and_edges() {
        let mut store = InMemoryGraphStore::new();
        let created = append_transition(&mut store, None, created_transition()).unwrap();
        let observed = append_transition(
            &mut store,
            Some(created.run),
            transition(
                "HOST.OBSERVED",
                json!({
                    "repo": "Theorem",
                    "branch": "main",
                    "commit_sha": "abc123",
                    "cwd": "/repo/Theorem",
                }),
            ),
        )
        .unwrap();

        assert!(store.get_node(&run_node_id(RUN_ID)).is_some());
        assert!(store.get_node(&event_node_id(RUN_ID, 1)).is_some());
        assert!(store.get_node(&event_node_id(RUN_ID, 2)).is_some());
        assert!(store
            .get_edge(&format!("harness:edge:event-of:{RUN_ID}:{:020}", 2))
            .is_some());
        assert!(store
            .get_edge(&format!("harness:edge:event-next:{RUN_ID}:{:020}", 2))
            .is_some());

        let loaded_events = load_events(&store, RUN_ID).unwrap();
        assert_eq!(loaded_events.len(), 2);
        assert_eq!(loaded_events[0].seq, 1);
        assert_eq!(loaded_events[1].event_type, "HOST.OBSERVED");

        let loaded_run = load_run(&store, RUN_ID).unwrap().unwrap();
        assert_eq!(loaded_run.status, observed.run.status);
        assert_eq!(
            loaded_run
                .scope
                .get("repo")
                .and_then(serde_json::Value::as_str),
            Some("Theorem")
        );
    }

    #[test]
    fn replay_persisted_run_rebuilds_state_from_events() {
        let mut store = InMemoryGraphStore::new();
        let created = append_transition(&mut store, None, created_transition()).unwrap();
        let resolved = append_transition(
            &mut store,
            Some(created.run),
            transition("TASK.RESOLVED", json!({"task_signature": "sig-runtime"})),
        )
        .unwrap();

        let replayed = replay_persisted_run(&store, RUN_ID).unwrap().unwrap();
        assert_eq!(replayed.status, resolved.run.status);
        assert_eq!(hash_run_state(&replayed), resolved.state_hash_after);
    }

    #[test]
    fn redcore_reopens_persisted_harness_events() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-harness-runtime-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let created = append_transition(&mut store, None, created_transition()).unwrap();
            append_transition(
                &mut store,
                Some(created.run),
                transition("TASK.RESOLVED", json!({"task_signature": "sig-redcore"})),
            )
            .unwrap();
        }

        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let events = load_events(&store, RUN_ID).unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event_type, "TASK.RESOLVED");
            let run = load_run(&store, RUN_ID).unwrap().unwrap();
            assert_eq!(run.status, "resolved");
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn append_requires_contiguous_events() {
        let mut store = InMemoryGraphStore::new();
        let result = apply_transition(None, created_transition()).unwrap();
        let mut skipped = result.clone();
        skipped.event.seq = 2;
        skipped.run.last_event_seq = 2;
        let error = persist_transition_result(&mut store, &skipped).unwrap_err();
        assert_eq!(
            error,
            HarnessRuntimeError::EventGap {
                run_id: RUN_ID.to_string(),
                expected_previous_seq: 1,
            }
        );
    }

    #[test]
    fn append_rejects_conflicting_event_at_same_sequence() {
        let mut store = InMemoryGraphStore::new();
        let result = append_transition(&mut store, None, created_transition()).unwrap();
        let mut conflicting = result.clone();
        conflicting.event.state_hash_after = "different".to_string();
        conflicting.state_hash_after = "different".to_string();

        let error = persist_transition_result(&mut store, &conflicting).unwrap_err();
        assert_eq!(
            error,
            HarnessRuntimeError::EventConflict {
                event_id: event_node_id(RUN_ID, 1),
                run_id: RUN_ID.to_string(),
                seq: 1,
            }
        );
    }

    fn created_transition() -> TransitionInput {
        transition(
            "RUN.CREATED",
            json!({
                "task": "persist harness events",
                "actor": "codex",
                "scope": {
                    "repo": "Theorem",
                    "branch": "main",
                    "commit_sha": "abc123",
                    "workstream_id": "ws-runtime",
                    "agent_host": "codex",
                    "agent_model": "gpt-5",
                },
            }),
        )
    }

    fn transition(event_type: &str, payload: Value) -> TransitionInput {
        TransitionInput {
            run_id: RUN_ID.to_string(),
            event_type: event_type.to_string(),
            payload: payload.as_object().cloned().unwrap_or_else(Map::new),
            actor: "codex".to_string(),
            idempotency_key: String::new(),
            created_at: TS.to_string(),
        }
    }
}
