use crate::state_machine::{apply_transition, HarnessError};
use crate::types::{AgentRunState, AgentStepState, EventState, Payload, RunState, TransitionInput};
use serde_json::{json, Value};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum ReplayError {
    EmptyEventStream,
    EventReplayFailed(HarnessError),
}

impl fmt::Display for ReplayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReplayError::EmptyEventStream => write!(f, "Cannot fork an empty event stream."),
            ReplayError::EventReplayFailed(error) => {
                write!(f, "Cannot replay events for fork: {error}")
            }
        }
    }
}

impl Error for ReplayError {}

pub fn replay_run(run: &AgentRunState) -> Vec<AgentStepState> {
    let mut indexed = run.steps.iter().cloned().enumerate().collect::<Vec<_>>();
    indexed.sort_by(|left, right| {
        let created = left.1.created_at.cmp(&right.1.created_at);
        if created == std::cmp::Ordering::Equal {
            left.0.cmp(&right.0)
        } else {
            created
        }
    });
    indexed.into_iter().map(|(_, step)| step).collect()
}

pub fn fork_run(
    run: &AgentRunState,
    through_step_id: Option<&str>,
    actor: Option<&str>,
) -> AgentRunState {
    let mut copied_steps = Vec::new();
    for step in replay_run(run) {
        let should_stop = through_step_id == Some(step.step_id.as_str());
        copied_steps.push(step);
        if should_stop {
            break;
        }
    }

    let source_step_id = through_step_id
        .map(str::to_string)
        .or_else(|| copied_steps.last().map(|step| step.step_id.clone()))
        .unwrap_or_default();

    let mut scope = run.scope.clone();
    scope.insert(
        "forked_from".to_string(),
        json!({
            "run_id": run.run_id,
            "through_step_id": source_step_id,
        }),
    );

    let mut fork = AgentRunState::new(&run.task, actor.unwrap_or(&run.actor), scope);
    for step in copied_steps {
        let mut payload = step.payload.clone();
        payload.insert(
            "replayed_from_step_id".to_string(),
            Value::String(step.step_id),
        );
        let fork_run_id = fork.run_id.clone();
        fork = fork.with_step(AgentStepState::new(fork_run_id, step.kind, payload));
    }
    fork
}

pub fn compare_runs(before: &AgentRunState, after: &AgentRunState) -> Value {
    let before_steps = keyed_steps(before);
    let after_steps = keyed_steps(after);

    let before_keys = before_steps
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();
    let after_keys = after_steps
        .iter()
        .map(|(key, _)| key.clone())
        .collect::<Vec<_>>();

    let mut removed_keys = before_keys
        .iter()
        .filter(|key| !after_keys.contains(key))
        .cloned()
        .collect::<Vec<_>>();
    let mut added_keys = after_keys
        .iter()
        .filter(|key| !before_keys.contains(key))
        .cloned()
        .collect::<Vec<_>>();
    let mut shared_keys = before_keys
        .iter()
        .filter(|key| after_keys.contains(key))
        .cloned()
        .collect::<Vec<_>>();
    removed_keys.sort();
    added_keys.sort();
    shared_keys.sort();

    let changed = shared_keys
        .into_iter()
        .filter(|key| {
            let before = lookup_step(&before_steps, key);
            let after = lookup_step(&after_steps, key);
            before.kind != after.kind || before.payload != after_payload(after)
        })
        .collect::<Vec<_>>();

    let added_steps = added_keys
        .iter()
        .map(|key| step_value(lookup_step(&after_steps, key)))
        .collect::<Vec<_>>();
    let removed_steps = removed_keys
        .iter()
        .map(|key| step_value(lookup_step(&before_steps, key)))
        .collect::<Vec<_>>();
    let changed_steps = changed
        .iter()
        .map(|key| {
            let before_step = lookup_step(&before_steps, key);
            let after_step = lookup_step(&after_steps, key);
            json!({
                "step_key": key,
                "before": step_value(before_step),
                "after": step_value(after_step),
            })
        })
        .collect::<Vec<_>>();

    json!({
        "before_run_id": before.run_id,
        "after_run_id": after.run_id,
        "added_steps": added_steps,
        "removed_steps": removed_steps,
        "changed_steps": changed_steps,
        "summary": {
            "added": added_keys.len(),
            "removed": removed_keys.len(),
            "changed": changed.len(),
        }
    })
}

pub fn replay_events(events: &[EventState]) -> Result<Option<RunState>, HarnessError> {
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.seq);
    let mut state = None;
    for event in ordered {
        let transition = TransitionInput {
            run_id: event.run_id,
            event_type: event.event_type,
            payload: event.payload,
            actor: String::new(),
            idempotency_key: event.idempotency_key,
            created_at: event.created_at,
        };
        state = Some(apply_transition(state, transition)?.run);
    }
    Ok(state)
}

pub fn fork_events(
    events: &[EventState],
    through_event_seq: Option<u64>,
    actor: Option<&str>,
) -> Result<RunState, ReplayError> {
    let mut ordered = events.to_vec();
    ordered.sort_by_key(|event| event.seq);
    if ordered.is_empty() {
        return Err(ReplayError::EmptyEventStream);
    }
    let selected = ordered
        .into_iter()
        .filter(|event| {
            through_event_seq
                .map(|seq| event.seq <= seq)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    let through_seq = selected
        .last()
        .map(|event| event.seq)
        .ok_or(ReplayError::EmptyEventStream)?;
    let state = replay_events(&selected)
        .map_err(ReplayError::EventReplayFailed)?
        .ok_or(ReplayError::EmptyEventStream)?;

    let mut scope = state.scope.clone();
    scope.insert(
        "forked_from".to_string(),
        Value::String(state.run_id.clone()),
    );
    scope.insert(
        "through_event_seq".to_string(),
        Value::Number(through_seq.into()),
    );

    let mut fork = RunState::new(&state.task, actor.unwrap_or(&state.actor), scope);
    fork.task_signature = state.task_signature;
    Ok(fork)
}

fn keyed_steps(run: &AgentRunState) -> Vec<(String, AgentStepState)> {
    replay_run(run)
        .into_iter()
        .map(|step| (step_key(&step), step))
        .collect()
}

fn step_key(step: &AgentStepState) -> String {
    step.payload
        .get("replayed_from_step_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| step.step_id.clone())
}

fn after_payload(step: &AgentStepState) -> Payload {
    let mut payload = step.payload.clone();
    payload.remove("replayed_from_step_id");
    payload
}

fn lookup_step<'a>(steps: &'a [(String, AgentStepState)], key: &str) -> &'a AgentStepState {
    steps
        .iter()
        .find(|(candidate, _)| candidate == key)
        .map(|(_, step)| step)
        .expect("step key should exist")
}

fn step_value(step: &AgentStepState) -> Value {
    serde_json::to_value(step).expect("AgentStepState serialization should be infallible")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{apply_transition, TransitionInput};
    use serde_json::{json, Map};

    #[test]
    fn replay_fork_and_compare_use_step_provenance() {
        let run = AgentRunState::new("compare redis harness", "codex", Map::new());
        let first = AgentStepState::new(
            run.run_id.clone(),
            "tool_call",
            payload(json!({"tool": "native_search"})),
        )
        .with_step_id("step:1")
        .at("2026-06-01T00:00:01Z");
        let second = AgentStepState::new(
            run.run_id.clone(),
            "observation",
            payload(json!({"ok": true})),
        )
        .with_step_id("step:2")
        .at("2026-06-01T00:00:02Z");
        let third = AgentStepState::new(
            run.run_id.clone(),
            "validation",
            payload(json!({"status": "needs_review"})),
        )
        .with_step_id("step:3")
        .at("2026-06-01T00:00:03Z");
        let run = run
            .with_step(first)
            .with_step(second.clone())
            .with_step(third);

        assert_eq!(
            replay_run(&run)
                .into_iter()
                .map(|step| step.step_id)
                .collect::<Vec<_>>(),
            vec!["step:1", "step:2", "step:3"]
        );

        let fork = fork_run(&run, Some(second.step_id.as_str()), Some("agent-2"));
        let comparison = compare_runs(&run, &fork);

        assert_ne!(fork.run_id, run.run_id);
        assert_eq!(fork.actor, "agent-2");
        assert_eq!(
            fork.scope
                .get("forked_from")
                .and_then(|value| value.pointer("/through_step_id")),
            Some(&json!("step:2"))
        );
        assert_eq!(fork.steps.len(), 2);
        assert_eq!(
            comparison.pointer("/removed_steps/0/step_id"),
            Some(&json!("step:3"))
        );
    }

    #[test]
    fn replay_events_and_fork_events_rebuild_v3_state() {
        let created = apply_transition(
            None,
            TransitionInput::new(
                "RUN.CREATED",
                payload(json!({
                    "task": "port harness",
                    "actor": "codex",
                    "scope": {"repo": "Theorem"}
                })),
            )
            .with_run_id("run:v3"),
        )
        .unwrap();
        let resolved = apply_transition(
            Some(created.run),
            TransitionInput::new(
                "TASK.RESOLVED",
                payload(json!({"task_signature": "sig:v3"})),
            )
            .with_run_id("run:v3"),
        )
        .unwrap();

        let events = vec![created.event, resolved.event];
        let replayed = replay_events(&events).unwrap().unwrap();
        assert_eq!(replayed.status, "resolved");
        assert_eq!(replayed.task_signature, "sig:v3");

        let fork = fork_events(&events, Some(2), Some("claude")).unwrap();
        assert_eq!(fork.actor, "claude");
        assert_eq!(fork.task_signature, "sig:v3");
        assert_eq!(fork.scope.get("forked_from"), Some(&json!("run:v3")));
    }

    fn payload(value: Value) -> Payload {
        match value {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }
}
