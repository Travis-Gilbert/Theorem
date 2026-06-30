use crate::{load_events, HarnessRuntimeError, RuntimeResult};
use rustyred_thg_core::{GraphStore, NodeQuery};
use serde_json::{json, Value};
use theorem_harness_core::{
    export_trainset_for_intent, fitness_gate_from_observations, gepa_feedback_point,
    reservoir_feedback_from_observations, trainset_jsonl, AgentRunState, AgentStepState,
    EventState, FitnessObservation, GepaFeedbackInput, GepaProposalError, GepaTrainSession,
    Payload, ReservoirFeedback, RunState, SessionMetricsState, TrainExample,
};

const DEFAULT_PAIRFORMER_MODE: &str = "off";
const RUN_LABEL: &str = "HarnessRun";

pub fn gepa_train_sessions_for_intent<S: GraphStore>(
    store: &S,
    intent_id: &str,
) -> RuntimeResult<Vec<GepaTrainSession>> {
    let intent_id = intent_id.trim();
    if intent_id.is_empty() {
        return Ok(Vec::new());
    }
    let runs = GraphStore::query_nodes(
        store,
        NodeQuery {
            label: Some(RUN_LABEL.to_string()),
            properties: Default::default(),
            limit: None,
            include_expired: false,
        },
    );

    runs.into_iter()
        .map(|node| {
            serde_json::from_value::<RunState>(node.properties)
                .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<RuntimeResult<Vec<_>>>()?
        .into_iter()
        .filter(|run| run_matches_intent(run, intent_id))
        .map(|run| {
            let events = load_events(store, &run.run_id)?;
            Ok(train_session_from_run(run, events, intent_id))
        })
        .collect()
}

pub fn gepa_trainset_for_intent<S: GraphStore>(
    store: &S,
    intent_id: &str,
) -> RuntimeResult<Vec<TrainExample>> {
    let sessions = gepa_train_sessions_for_intent(store, intent_id)?;
    export_trainset_for_intent(&sessions, intent_id).map_err(gepa_error)
}

pub fn gepa_trainset_jsonl_for_intent<S: GraphStore>(
    store: &S,
    intent_id: &str,
) -> RuntimeResult<String> {
    let examples = gepa_trainset_for_intent(store, intent_id)?;
    trainset_jsonl(&examples).map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))
}

pub fn gepa_trainset_json_for_intent<S: GraphStore>(
    store: &S,
    intent_id: &str,
) -> RuntimeResult<Value> {
    let examples = gepa_trainset_for_intent(store, intent_id)?;
    Ok(json!({
        "intent_id": intent_id,
        "count": examples.len(),
        "examples": examples,
    }))
}

fn train_session_from_run(
    run: RunState,
    events: Vec<EventState>,
    intent_id: &str,
) -> GepaTrainSession {
    let outcome = run.outcome.clone().map(Value::Object);
    let metric = session_metric_from_run(&run, &events);
    let feedback = gepa_feedback_point(GepaFeedbackInput {
        metric,
        fitness: fitness_from_outcome(outcome.as_ref()),
        shadow: None,
        reservoir: reservoir_from_outcome(outcome.as_ref()),
    });
    GepaTrainSession {
        session_id: run.run_id.clone(),
        intent_id: intent_id.to_string(),
        input: input_from_run(&run, &events),
        trace: agent_trace_from_run(&run, &events),
        outcome,
        feedback,
    }
}

fn run_matches_intent(run: &RunState, intent_id: &str) -> bool {
    [
        run.scope.get("intent_id"),
        run.scope.get("gepa_intent_id"),
        run.scope.get("task_intent"),
    ]
    .into_iter()
    .flatten()
    .any(|value| value.as_str() == Some(intent_id))
        || run.task_signature == intent_id
}

fn input_from_run(run: &RunState, events: &[EventState]) -> Value {
    for key in ["gepa_input", "input", "user_prompt"] {
        if let Some(value) = run.scope.get(key) {
            return normalize_input_value(key, value.clone());
        }
    }
    events
        .iter()
        .find(|event| event.event_type == "RUN.CREATED")
        .and_then(|event| {
            let scope = event.payload.get("scope").and_then(Value::as_object);
            ["gepa_input", "input", "user_prompt"]
                .iter()
                .find_map(|key| {
                    event
                        .payload
                        .get(*key)
                        .cloned()
                        .or_else(|| scope.and_then(|scope| scope.get(*key).cloned()))
                        .map(|value| (*key, value))
                })
        })
        .map(|(key, value)| normalize_input_value(key, value))
        .unwrap_or_else(|| json!({ "task": run.task }))
}

fn normalize_input_value(key: &str, value: Value) -> Value {
    if key == "user_prompt" {
        json!({ "user_prompt": value })
    } else {
        value
    }
}

fn agent_trace_from_run(run: &RunState, events: &[EventState]) -> AgentRunState {
    AgentRunState {
        run_id: run.run_id.clone(),
        task: run.task.clone(),
        actor: run.actor.clone(),
        scope: run.scope.clone(),
        status: run.status.clone(),
        steps: events
            .iter()
            .map(|event| AgentStepState {
                step_id: event.event_id.clone(),
                run_id: event.run_id.clone(),
                kind: event.event_type.clone(),
                payload: event.payload.clone(),
                created_at: event.created_at.clone(),
            })
            .collect(),
        search_runs: Vec::new(),
        artifacts: run
            .context
            .clone()
            .map_or_else(Vec::new, |payload| vec![payload]),
        memory_patches: run.learning_patches.clone(),
        validations: run.validators.clone(),
        created_at: run.created_at.clone(),
        updated_at: run.updated_at.clone(),
    }
}

fn session_metric_from_run(run: &RunState, events: &[EventState]) -> SessionMetricsState {
    let outcome = run.outcome.as_ref();
    let input_tokens = first_i64(
        outcome,
        &[
            "total_input_tokens",
            "input_tokens",
            "prompt_tokens",
            "usage.input_tokens",
            "usage.prompt_tokens",
        ],
    )
    .or_else(|| sum_event_i64(events, &["input_tokens", "prompt_tokens"]));
    let output_tokens = first_i64(
        outcome,
        &[
            "total_output_tokens",
            "output_tokens",
            "completion_tokens",
            "usage.output_tokens",
            "usage.completion_tokens",
        ],
    )
    .or_else(|| sum_event_i64(events, &["output_tokens", "completion_tokens"]));
    let total_tokens = first_i64(outcome, &["total_tokens", "tokens", "usage.total_tokens"])
        .or_else(|| {
            input_tokens
                .zip(output_tokens)
                .map(|(input, output)| input + output)
        })
        .or(input_tokens)
        .or(output_tokens)
        .unwrap_or_default();
    let total_input_tokens =
        input_tokens.unwrap_or_else(|| total_tokens.saturating_sub(output_tokens.unwrap_or(0)));
    let total_output_tokens =
        output_tokens.unwrap_or_else(|| total_tokens.saturating_sub(total_input_tokens));

    SessionMetricsState {
        total_input_tokens,
        total_output_tokens,
        total_tool_calls: first_i64(outcome, &["total_tool_calls", "tool_calls"])
            .or_else(|| count_tool_events(events))
            .unwrap_or_default(),
        task_completion: task_completed(run),
        pairformer_mode: scope_string(run, "pairformer_mode", DEFAULT_PAIRFORMER_MODE),
        task_category: scope_string(run, "task_category", "unknown"),
        workstream_id: if run.workstream_id.is_empty() {
            scope_string(run, "workstream_id", "")
        } else {
            run.workstream_id.clone()
        },
        session_id: run.run_id.clone(),
        total_tokens,
    }
}

fn task_completed(run: &RunState) -> bool {
    let Some(outcome) = run.outcome.as_ref() else {
        return false;
    };
    bool_field(outcome, "accepted")
        .or_else(|| bool_field(outcome, "tests_passed"))
        .unwrap_or_else(|| {
            payload_string_field(outcome, "outcome_bucket")
                .or_else(|| payload_string_field(outcome, "outcome"))
                .is_some_and(|value| matches!(value.as_str(), "accepted" | "positive" | "success"))
        })
}

fn fitness_from_outcome(
    outcome: Option<&Value>,
) -> Option<theorem_harness_core::FitnessGateResult> {
    let outcome = outcome?;
    let before = observation_array(
        outcome,
        &["fitness_before_observations", "before_observations"],
    );
    let after = observation_array(
        outcome,
        &["fitness_after_observations", "after_observations"],
    );
    if before.is_empty() || after.is_empty() {
        None
    } else {
        Some(fitness_gate_from_observations(&before, &after))
    }
}

fn reservoir_from_outcome(outcome: Option<&Value>) -> ReservoirFeedback {
    let Some(outcome) = outcome else {
        return ReservoirFeedback::default();
    };
    let before = observation_array(
        outcome,
        &["fitness_before_observations", "before_observations"],
    );
    let after = observation_array(
        outcome,
        &["fitness_after_observations", "after_observations"],
    );
    let mut feedback = if before.is_empty() || after.is_empty() {
        ReservoirFeedback::default()
    } else {
        reservoir_feedback_from_observations(&before, &after)
    };
    feedback.unsupported_claims = string_array(outcome, "unsupported_claims");
    feedback.contradictions = string_array(outcome, "contradictions");
    feedback.notes = string_array(outcome, "notes");
    if let Some(note) = string_field(outcome, "feedback") {
        feedback.notes.push(note);
    }
    if let Some(summary) = string_field(outcome, "summary") {
        feedback.notes.push(summary);
    }
    feedback
}

fn observation_array(value: &Value, keys: &[&str]) -> Vec<FitnessObservation> {
    keys.iter()
        .find_map(|key| value.pointer(&format!("/{}", key.replace('.', "/"))))
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| serde_json::from_value::<FitnessObservation>(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

fn first_i64(payload: Option<&Payload>, keys: &[&str]) -> Option<i64> {
    let value = Value::Object(payload?.clone());
    keys.iter().find_map(|key| i64_at(&value, key))
}

fn i64_at(value: &Value, key: &str) -> Option<i64> {
    value
        .pointer(&format!("/{}", key.replace('.', "/")))
        .and_then(|value| {
            value
                .as_i64()
                .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
                .or_else(|| value.as_str().and_then(|text| text.parse::<i64>().ok()))
        })
        .map(|number| number.max(0))
}

fn sum_event_i64(events: &[EventState], keys: &[&str]) -> Option<i64> {
    let total = events
        .iter()
        .filter_map(|event| first_i64(Some(&event.payload), keys))
        .sum::<i64>();
    (total > 0).then_some(total)
}

fn count_tool_events(events: &[EventState]) -> Option<i64> {
    let count = events
        .iter()
        .filter(|event| {
            (event.event_type.contains("TOOL") && !event.event_type.starts_with("TOOLKIT."))
                || event.payload.contains_key("tool")
                || event.payload.contains_key("tool_id")
        })
        .count();
    (count > 0).then_some(count as i64)
}

fn bool_field(payload: &Payload, key: &str) -> Option<bool> {
    payload.get(key).and_then(Value::as_bool)
}

fn payload_string_field(payload: &Payload, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

fn string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn scope_string(run: &RunState, key: &str, fallback: &str) -> String {
    run.scope
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_string()
}

fn gepa_error(error: GepaProposalError) -> HarnessRuntimeError {
    HarnessRuntimeError::Deserialization(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::append_transition;
    use rustyred_thg_core::InMemoryGraphStore;
    use serde_json::Map;
    use theorem_harness_core::TransitionInput;

    const INTENT_ID: &str = "intent:prompt-improver";
    const RUN_ID: &str = "run-gepa-runtime-1";
    const TS: &str = "2026-06-30T00:00:00Z";

    #[test]
    fn exports_trainset_from_persisted_runs_for_intent() {
        let mut store = InMemoryGraphStore::new();
        persist_prompt_run(&mut store, RUN_ID, true);

        let examples = gepa_trainset_for_intent(&store, INTENT_ID).unwrap();
        let jsonl = gepa_trainset_jsonl_for_intent(&store, INTENT_ID).unwrap();

        assert_eq!(examples.len(), 1);
        assert_eq!(examples[0].intent_id, INTENT_ID);
        assert_eq!(examples[0].input["user_prompt"], json!("make this better"));
        assert_eq!(
            examples[0].outcome["improved_prompt"],
            json!("make this better, precisely")
        );
        assert_eq!(examples[0].trace.steps.len(), 9);
        assert!(examples[0].feedback.contains("source_independence"));
        assert!(jsonl.contains("\"intent_id\":\"intent:prompt-improver\""));
    }

    #[test]
    fn trainset_export_errors_when_matching_run_lacks_outcome() {
        let mut store = InMemoryGraphStore::new();
        persist_prompt_run(&mut store, "run-missing-outcome", false);

        let error = gepa_trainset_for_intent(&store, INTENT_ID).unwrap_err();

        assert!(error.to_string().contains("missing captured outcome"));
    }

    #[test]
    fn input_fallback_reads_run_created_scope_payload() {
        let mut run = RunState::new("fallback task", "codex", Map::new());
        run.run_id = "run-scope-fallback".to_string();
        let event = serde_json::from_value::<EventState>(json!({
            "run_id": run.run_id.clone(),
            "seq": 1,
            "type": "RUN.CREATED",
            "payload": {
                "scope": {
                    "gepa_input": { "user_prompt": "from created scope" }
                }
            }
        }))
        .unwrap();

        let input = input_from_run(&run, &[event]);

        assert_eq!(input["user_prompt"], json!("from created scope"));
    }

    #[test]
    fn metric_fallback_keeps_tokens_non_negative_and_skips_toolkit_events() {
        let mut run = RunState::new("improve prompt", "codex", Map::new());
        run.run_id = "run-metric-fallback".to_string();
        run.outcome = Some(
            json!({
                "accepted": true,
                "total_input_tokens": 100
            })
            .as_object()
            .cloned()
            .unwrap(),
        );
        let events = vec![
            event_state(
                &run.run_id,
                "TOOLKIT.COMPILED",
                json!({"selected_tools": ["prompt-improver"]}),
            ),
            event_state(
                &run.run_id,
                "AGENT.ACTING",
                json!({"tool": "prompt-improver"}),
            ),
        ];

        let metric = session_metric_from_run(&run, &events);

        assert_eq!(metric.total_tokens, 100);
        assert_eq!(metric.total_input_tokens, 100);
        assert_eq!(metric.total_output_tokens, 0);
        assert_eq!(metric.total_tool_calls, 1);
    }

    fn persist_prompt_run(store: &mut InMemoryGraphStore, run_id: &str, with_outcome: bool) {
        let created = append_transition(
            store,
            None,
            transition(
                run_id,
                "RUN.CREATED",
                json!({
                    "task": "improve prompt",
                    "actor": "codex",
                    "scope": {
                        "intent_id": INTENT_ID,
                        "gepa_input": {"user_prompt": "make this better"},
                        "task_category": "prompt",
                        "workstream_id": "gepa-runtime"
                    }
                }),
            ),
        )
        .unwrap();
        let task_resolved = append_transition(
            store,
            Some(created.run),
            transition(
                run_id,
                "TASK.RESOLVED",
                json!({"task_signature": INTENT_ID}),
            ),
        )
        .unwrap();
        let profile_selected = append_transition(
            store,
            Some(task_resolved.run),
            transition(
                run_id,
                "PROFILE.SELECTED",
                json!({
                    "profile_id": "test-profile",
                    "profile_version": "1",
                    "policy_hash": "policy:test"
                }),
            ),
        )
        .unwrap();
        let toolkit_compiled = append_transition(
            store,
            Some(profile_selected.run),
            transition(
                run_id,
                "TOOLKIT.COMPILED",
                json!({
                    "selected_tools": ["prompt-improver"],
                    "selected_plugins": [],
                    "excluded_tools": [],
                    "permission_reasons": []
                }),
            ),
        )
        .unwrap();
        let context_planned = append_transition(
            store,
            Some(toolkit_compiled.run),
            transition(
                run_id,
                "CONTEXT.PLANNED",
                json!({
                    "budget_tokens": 4000,
                    "plan_hash": "plan:test",
                    "candidate_token_count": 1200
                }),
            ),
        )
        .unwrap();
        let context_packed = append_transition(
            store,
            Some(context_planned.run),
            transition(
                run_id,
                "CONTEXT.PACKED",
                json!({
                    "artifact_id": "ctx:test",
                    "capsule_tokens": 900,
                    "budget_tokens": 4000,
                    "included_atom_count": 3,
                    "excluded_atom_count": 0,
                    "token_ledger": {}
                }),
            ),
        )
        .unwrap();
        let context_injected = append_transition(
            store,
            Some(context_packed.run),
            transition(
                run_id,
                "CONTEXT.INJECTED",
                json!({
                    "artifact_id": "ctx:test",
                    "adapter": "codex",
                    "target": "active_context"
                }),
            ),
        )
        .unwrap();
        let acting = append_transition(
            store,
            Some(context_injected.run),
            transition(
                run_id,
                "AGENT.ACTING",
                json!({
                    "adapter": "codex",
                    "started_at": TS,
                    "tool": "prompt-improver"
                }),
            ),
        )
        .unwrap();
        if with_outcome {
            let _ = append_transition(
                store,
                Some(acting.run),
                transition(
                    run_id,
                    "OUTCOME.RECORDED",
                    json!({
                        "accepted": true,
                        "tests_passed": true,
                        "validator_results": [],
                        "files_changed": [],
                        "manual_override": true,
                        "improved_prompt": "make this better, precisely",
                        "total_input_tokens": 100,
                        "total_output_tokens": 40,
                        "total_tool_calls": 1,
                        "fitness_before_observations": [
                            obs("source:a", 0),
                            obs("source:b", 1000)
                        ],
                        "fitness_after_observations": [
                            obs("source:a", 0),
                            obs("source:a", 1000)
                        ],
                        "summary": "prompt rewritten"
                    }),
                ),
            )
            .unwrap();
        }
    }

    fn transition(run_id: &str, event_type: &str, payload: Value) -> TransitionInput {
        TransitionInput {
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: payload.as_object().cloned().unwrap_or_else(Map::new),
            actor: "codex".to_string(),
            idempotency_key: String::new(),
            created_at: TS.to_string(),
        }
    }

    fn event_state(run_id: &str, event_type: &str, payload: Value) -> EventState {
        serde_json::from_value(json!({
            "run_id": run_id,
            "seq": 1,
            "type": event_type,
            "payload": payload
        }))
        .unwrap()
    }

    fn obs(source_id: &str, observed_at_ms: i64) -> Value {
        json!({
            "root_depth": 4,
            "source_id": source_id,
            "support_ratio": 0.8,
            "claim_specificity": 0.8,
            "observed_at_ms": observed_at_ms
        })
    }
}
