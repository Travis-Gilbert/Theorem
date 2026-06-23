use crate::state_hash::{empty_state_hash, hash_run_state};
use crate::types::{
    prefixed_id, EventState, GuardViolation, Payload, RunState, TransitionInput, TransitionResult,
};
use serde_json::{Map, Value};
use std::error::Error;
use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum HarnessError {
    Guard(Box<GuardViolation>),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HarnessError::Guard(violation) => write!(f, "{}", violation.message),
        }
    }
}

impl Error for HarnessError {}

pub fn apply_transition(
    state: Option<RunState>,
    transition: TransitionInput,
) -> Result<TransitionResult, HarnessError> {
    let (before_hash, next_run) = if transition.event_type == "RUN.CREATED" {
        (empty_state_hash(), created_run(state, &transition)?)
    } else {
        let current = state.ok_or_else(|| {
            guard_violation(
                "missing_run_state",
                format!("{} requires an existing run state", transition.event_type),
                "",
                "",
                Vec::new(),
                Payload::new(),
            )
        })?;
        if !transition.run_id.is_empty() && transition.run_id != current.run_id {
            let mut details = Payload::new();
            details.insert(
                "event_run_id".to_string(),
                Value::String(transition.run_id.clone()),
            );
            details.insert(
                "state_run_id".to_string(),
                Value::String(current.run_id.clone()),
            );
            return Err(guard_violation(
                "run_id_mismatch",
                format!(
                    "event run_id {} does not match {}",
                    transition.run_id, current.run_id
                ),
                "",
                "",
                Vec::new(),
                details,
            ));
        }
        let before_hash = hash_run_state(&current);
        (before_hash, advance_run(current, &transition)?)
    };

    let after_hash = hash_run_state(&next_run);
    let event = EventState {
        event_id: prefixed_id("event"),
        run_id: next_run.run_id.clone(),
        seq: next_run.last_event_seq,
        event_type: transition.event_type,
        payload: transition.payload,
        idempotency_key: transition.idempotency_key,
        state_hash_before: before_hash.clone(),
        state_hash_after: after_hash.clone(),
        created_at: transition.created_at,
        workstream_id: next_run.workstream_id.clone(),
        agent_host: next_run.agent_host.clone(),
        agent_model: next_run.agent_model.clone(),
        repo: payload_to_string(next_run.scope.get("repo")),
        branch: payload_to_string(next_run.scope.get("branch")),
        commit_sha: payload_to_string(next_run.scope.get("commit_sha")),
        privacy_scope: "private".to_string(),
    };

    Ok(TransitionResult {
        run: next_run,
        event,
        effects: Vec::new(),
        state_hash_before: before_hash,
        state_hash_after: after_hash,
    })
}

fn created_run(
    current: Option<RunState>,
    transition: &TransitionInput,
) -> Result<RunState, HarnessError> {
    if let Some(state) = current {
        return Err(guard_violation(
            "state_not_empty",
            "RUN.CREATED requires empty input state",
            "",
            state.status,
            Vec::new(),
            Payload::new(),
        ));
    }
    require_payload_fields(transition, transition_requirements("RUN.CREATED"))?;

    let scope = payload_object(transition.payload.get("scope"));
    let actor = payload_to_string(transition.payload.get("actor"));
    let mut run = RunState::new(
        payload_to_string(transition.payload.get("task")),
        actor.clone(),
        scope,
    );
    if !transition.run_id.is_empty() {
        run.run_id = transition.run_id.clone();
    }
    run.status = "created".to_string();
    run.last_event_seq = 1;
    run.created_at = transition.created_at.clone();
    run.updated_at = transition.created_at.clone();
    run.workstream_id = payload_to_string(run.scope.get("workstream_id"));
    run.agent_host = first_non_empty(&[payload_to_string(run.scope.get("agent_host")), actor]);
    run.agent_model = payload_to_string(run.scope.get("agent_model"));
    Ok(run)
}

fn advance_run(mut run: RunState, transition: &TransitionInput) -> Result<RunState, HarnessError> {
    let seq = run.last_event_seq + 1;
    reject_terminal_run(&run, transition)?;

    match transition.event_type.as_str() {
        "HOST.OBSERVED" => {
            validate_state_and_payload(&run, transition)?;
            run.status = "observed".to_string();
            run.host = transition.payload.clone();
        }
        "TASK.RESOLVED" => {
            validate_state_and_payload(&run, transition)?;
            run.status = "resolved".to_string();
            run.task_signature = payload_to_string(transition.payload.get("task_signature"));
        }
        "PROFILE.SELECTED" => {
            validate_state_and_payload(&run, transition)?;
            run.status = "profile_selected".to_string();
            run.profile = Some(transition.payload.clone());
        }
        "DOMAIN.RESOLVED" => {
            validate_state_and_payload(&run, transition)?;
            let mut profile = transition.payload.clone();
            let profile_id = first_non_empty(&[
                payload_to_string(profile.get("profile_id")),
                payload_to_string(profile.get("domain")),
            ]);
            let profile_version = first_non_empty(&[
                payload_to_string(profile.get("profile_version")),
                payload_to_string(profile.get("domain_version")),
            ]);
            profile.insert("profile_id".to_string(), Value::String(profile_id));
            profile.insert(
                "profile_version".to_string(),
                Value::String(profile_version),
            );
            run.status = "domain_resolved".to_string();
            run.profile = Some(profile);
        }
        "TOOLKIT.COMPILED" => {
            validate_state_and_payload(&run, transition)?;
            validate_toolkit_payload(&run, transition)?;
            run.status = "toolkit_compiled".to_string();
            run.toolkit = Some(transition.payload.clone());
        }
        "TOOLPACK.COMPILED" => {
            validate_state_and_payload(&run, transition)?;
            validate_toolkit_payload(&run, transition)?;
            run.status = "toolpack_compiled".to_string();
            run.toolkit = Some(transition.payload.clone());
        }
        "MAPS.LOADED" => {
            validate_state_and_payload(&run, transition)?;
            let mut update = Payload::new();
            update.insert(
                "maps".to_string(),
                Value::Object(transition.payload.clone()),
            );
            run.status = "maps_loaded".to_string();
            run.context = Some(merge_binding(run.context.as_ref(), update));
        }
        "CONTEXT.PLANNED" => {
            validate_state_and_payload(&run, transition)?;
            if payload_i64(transition.payload.get("budget_tokens")) <= 0 {
                return Err(guard_violation(
                    "invalid_context_budget",
                    "CONTEXT.PLANNED requires a positive token budget",
                    "",
                    run.status,
                    Vec::new(),
                    Payload::new(),
                ));
            }
            run.status = "context_planned".to_string();
            run.context = Some(merge_binding(
                run.context.as_ref(),
                transition.payload.clone(),
            ));
        }
        "CONTEXT.PACKED" => {
            validate_state_and_payload(&run, transition)?;
            validate_context_budget(&run, transition)?;
            run.status = "context_packed".to_string();
            run.context = Some(merge_binding(
                run.context.as_ref(),
                transition.payload.clone(),
            ));
        }
        "CONTEXT.COMPILED" => {
            validate_state_and_payload(&run, transition)?;
            validate_context_budget(&run, transition)?;
            run.status = "context_compiled".to_string();
            run.context = Some(merge_binding(
                run.context.as_ref(),
                transition.payload.clone(),
            ));
        }
        "CONTEXT.INJECTED" => {
            require_payload_fields(transition, transition_requirements("CONTEXT.INJECTED"))?;
            let artifact_id = context_artifact_id(&run);
            if artifact_id.is_empty() {
                return Err(guard_violation(
                    "missing_context_artifact",
                    "CONTEXT.INJECTED requires a packed context artifact.",
                    "context_packed",
                    run.status,
                    Vec::new(),
                    Payload::new(),
                ));
            }
            validate_state_and_payload(&run, transition)?;
            let payload_artifact_id = payload_to_string(transition.payload.get("artifact_id"));
            if payload_artifact_id != artifact_id {
                let mut details = Payload::new();
                details.insert("packed_artifact_id".to_string(), Value::String(artifact_id));
                details.insert(
                    "payload_artifact_id".to_string(),
                    Value::String(payload_artifact_id),
                );
                return Err(guard_violation(
                    "context_artifact_mismatch",
                    "CONTEXT.INJECTED artifact_id must match the packed artifact.",
                    "",
                    run.status,
                    Vec::new(),
                    details,
                ));
            }
            let mut update = Payload::new();
            update.insert(
                "injection".to_string(),
                Value::Object(transition.payload.clone()),
            );
            run.status = "context_injected".to_string();
            run.context = Some(merge_binding(run.context.as_ref(), update));
        }
        "AGENT.ACTING" => {
            validate_state_and_payload(&run, transition)?;
            run.status = "agent_acting".to_string();
        }
        "VALIDATION.STARTED" | "VALIDATION.RUNNING" => {
            validate_state_and_payload(&run, transition)?;
            let mut validator = transition.payload.clone();
            let phase = if transition.event_type == "VALIDATION.STARTED" {
                "started"
            } else {
                "running"
            };
            validator.insert("phase".to_string(), Value::String(phase.to_string()));
            run.status = "validating".to_string();
            run.validators.push(validator);
        }
        "VALIDATION.FINISHED" => {
            validate_state_and_payload(&run, transition)?;
            let mut validator = transition.payload.clone();
            validator.insert("phase".to_string(), Value::String("finished".to_string()));
            run.status = "outcome_recorded".to_string();
            run.validators.push(validator);
        }
        "OUTCOME.RECORDED" => {
            validate_state_and_payload(&run, transition)?;
            validate_outcome_payload(&run, transition)?;
            run.status = "outcome_recorded".to_string();
            run.outcome = Some(transition.payload.clone());
        }
        "LEARNING.PROPOSED" => {
            require_payload_fields(transition, transition_requirements("LEARNING.PROPOSED"))?;
            validate_learning_payload(&run, transition)?;
            validate_state_and_payload(&run, transition)?;
            run.status = "learning_proposed".to_string();
            run.learning_patches.push(transition.payload.clone());
        }
        "MEMORY.PATCHED" => {
            require_payload_fields(transition, transition_requirements("MEMORY.PATCHED"))?;
            if !payload_bool(transition.payload.get("review_required")) {
                return Err(guard_violation(
                    "memory_patch_review_required",
                    "Memory patches must require review before promotion.",
                    "",
                    run.status,
                    Vec::new(),
                    Payload::new(),
                ));
            }
            validate_state_and_payload(&run, transition)?;
            run.status = "memory_patched".to_string();
            run.learning_patches.push(transition.payload.clone());
        }
        "MAPS.UPDATED" => {
            validate_state_and_payload(&run, transition)?;
            let mut update = Payload::new();
            update.insert(
                "map_updates".to_string(),
                Value::Object(transition.payload.clone()),
            );
            run.status = "maps_updated".to_string();
            run.context = Some(merge_binding(run.context.as_ref(), update));
        }
        "REVIEW.QUEUED" => {
            validate_state_and_payload(&run, transition)?;
            let mut patch = transition.payload.clone();
            patch.insert(
                "status".to_string(),
                Value::String("review_queued".to_string()),
            );
            run.status = "review_queued".to_string();
            run.learning_patches.push(patch);
        }
        "FEDERATION.SIGNAL_PREPARED" => {
            validate_state_and_payload(&run, transition)?;
            validate_federation_payload(&run, transition)?;
            run.status = "federation_signal_prepared".to_string();
            run.federation_signals.push(transition.payload.clone());
        }
        "RUN.CLOSED" => {
            require_payload_fields(transition, transition_requirements("RUN.CLOSED"))?;
            if run.outcome.is_none() {
                return Err(guard_violation(
                    "missing_outcome",
                    "RUN.CLOSED requires a recorded outcome.",
                    "",
                    run.status,
                    Vec::new(),
                    Payload::new(),
                ));
            }
            validate_state_and_payload(&run, transition)?;
            run.status = "closed".to_string();
        }
        "RUN.FAILED" => {
            require_payload_fields(transition, transition_requirements("RUN.FAILED"))?;
            let mut outcome = Payload::new();
            outcome.insert("accepted".to_string(), Value::Bool(false));
            outcome.insert("tests_passed".to_string(), Value::Bool(false));
            outcome.insert(
                "summary".to_string(),
                Value::String(payload_to_string(transition.payload.get("message"))),
            );
            outcome.insert(
                "failure".to_string(),
                Value::Object(transition.payload.clone()),
            );
            run.status = "failed".to_string();
            run.outcome = Some(outcome);
        }
        "RUN.CANCELLED" => {
            require_payload_fields(transition, transition_requirements("RUN.CANCELLED"))?;
            let mut outcome = Payload::new();
            outcome.insert("accepted".to_string(), Value::Bool(false));
            outcome.insert("tests_passed".to_string(), Value::Bool(false));
            outcome.insert(
                "summary".to_string(),
                Value::String(payload_to_string(transition.payload.get("reason"))),
            );
            outcome.insert(
                "cancellation".to_string(),
                Value::Object(transition.payload.clone()),
            );
            run.status = "cancelled".to_string();
            run.outcome = Some(outcome);
        }
        "RUN.REPLAYED" | "RUN.FORKED" => {
            validate_state_and_payload(&run, transition)?;
            let mut scope = run.scope.clone();
            scope.insert(
                "source_run_id".to_string(),
                Value::String(payload_to_string(transition.payload.get("source_run_id"))),
            );
            scope.insert(
                "through_event_seq".to_string(),
                transition
                    .payload
                    .get("through_event_seq")
                    .cloned()
                    .unwrap_or(Value::Null),
            );
            run.status = "created".to_string();
            run.scope = scope;
            run.task_signature.clear();
            run.profile = None;
            run.toolkit = None;
            run.context = None;
            run.cache_events.clear();
            run.validators.clear();
            run.outcome = None;
            run.learning_patches.clear();
            run.federation_signals.clear();
        }
        event if is_compound_event(event) => {
            run.status = run.status.clone();
        }
        event if is_cache_event(event) => {
            run = append_cache_event(run, transition)?;
        }
        event if is_oracle_event(event) => {
            run = append_oracle_event(run, transition)?;
        }
        event if is_cua_event(event) => {
            run = append_cua_event(run, transition)?;
        }
        event if is_cmh_event(event) => {
            run = apply_cmh_transition(run, transition)?;
        }
        _ => {
            return Err(guard_violation(
                "unsupported_transition",
                format!("unsupported transition {}", transition.event_type),
                "",
                run.status,
                Vec::new(),
                Payload::new(),
            ));
        }
    }

    run.last_event_seq = seq;
    run.updated_at = transition.created_at.clone();
    Ok(run)
}

fn append_cache_event(
    mut run: RunState,
    transition: &TransitionInput,
) -> Result<RunState, HarnessError> {
    validate_event_transition(
        &run,
        transition,
        cache_event_requirements(transition.event_type.as_str()),
        cache_allowed_previous(transition.event_type.as_str()),
    )?;
    let mut cache_event = Payload::new();
    cache_event.insert(
        "event_type".to_string(),
        Value::String(transition.event_type.clone()),
    );
    cache_event.insert(
        "payload".to_string(),
        Value::Object(transition.payload.clone()),
    );
    cache_event.insert(
        "created_at".to_string(),
        Value::String(transition.created_at.clone()),
    );
    run.status = transition.event_type.to_lowercase().replace('.', "_");
    run.cache_events.push(cache_event);
    Ok(run)
}

fn append_oracle_event(
    mut run: RunState,
    transition: &TransitionInput,
) -> Result<RunState, HarnessError> {
    validate_event_transition(
        &run,
        transition,
        oracle_event_requirements(transition.event_type.as_str()),
        ORACLE_LIVE_STATUSES,
    )?;
    if transition.event_type == "ADAPTER.SELECTED" {
        let mut adapter_scope = payload_object(run.scope.get("adapter"));
        adapter_scope.insert(
            "adapter_id".to_string(),
            Value::String(payload_to_string(transition.payload.get("adapter_id"))),
        );
        adapter_scope.insert(
            "role".to_string(),
            Value::String(payload_to_string(transition.payload.get("role"))),
        );
        adapter_scope.insert(
            "previous_adapter_id".to_string(),
            Value::String(payload_to_string(
                transition.payload.get("previous_adapter_id"),
            )),
        );
        adapter_scope.insert(
            "selected_at".to_string(),
            Value::String(transition.created_at.clone()),
        );
        run.scope
            .insert("adapter".to_string(), Value::Object(adapter_scope));
    }
    Ok(run)
}

fn append_cua_event(run: RunState, transition: &TransitionInput) -> Result<RunState, HarnessError> {
    validate_event_transition(
        &run,
        transition,
        cua_event_requirements(transition.event_type.as_str()),
        ORACLE_LIVE_STATUSES,
    )?;
    Ok(run)
}

fn apply_cmh_transition(
    mut run: RunState,
    transition: &TransitionInput,
) -> Result<RunState, HarnessError> {
    require_payload_fields(
        transition,
        transition_requirements(transition.event_type.as_str()),
    )?;
    validate_state_and_payload(&run, transition)?;
    run.status = cmh_target_status(transition.event_type.as_str()).to_string();
    Ok(run)
}

fn validate_state_and_payload(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    require_payload_fields(
        transition,
        transition_requirements(transition.event_type.as_str()),
    )?;
    let allowed = allowed_previous_statuses(transition.event_type.as_str());
    if !allowed.is_empty() && !allowed.contains(&run.status.as_str()) {
        return Err(invalid_previous_state(
            transition.event_type.as_str(),
            allowed,
            run.status.as_str(),
        ));
    }
    Ok(())
}

fn validate_event_transition(
    run: &RunState,
    transition: &TransitionInput,
    requirements: &'static [&'static str],
    allowed_previous: &'static [&'static str],
) -> Result<(), HarnessError> {
    require_payload_fields(transition, requirements)?;
    if !allowed_previous.is_empty() && !allowed_previous.contains(&run.status.as_str()) {
        return Err(invalid_previous_state(
            transition.event_type.as_str(),
            allowed_previous,
            run.status.as_str(),
        ));
    }
    Ok(())
}

fn reject_terminal_run(run: &RunState, transition: &TransitionInput) -> Result<(), HarnessError> {
    if !TERMINAL_STATUSES.contains(&run.status.as_str()) {
        return Ok(());
    }
    if transition.event_type == "RUN.FORKED" && matches!(run.status.as_str(), "closed" | "failed") {
        return Ok(());
    }
    if transition.event_type == "RUN.REPLAYED" && run.status == "closed" {
        return Ok(());
    }
    if is_compound_event(&transition.event_type)
        && matches!(run.status.as_str(), "closed" | "failed")
    {
        return Ok(());
    }
    Err(guard_violation(
        "terminal_run_state",
        format!(
            "{} cannot be applied to terminal run {}.",
            transition.event_type, run.status
        ),
        "",
        run.status.clone(),
        Vec::new(),
        Payload::new(),
    ))
}

fn validate_toolkit_payload(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    let Some(profile) = run.profile.as_ref() else {
        return Ok(());
    };
    let permissions = profile
        .get("permissions")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| payload_to_string(Some(item)))
                .filter(|item| !item.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if permissions.is_empty() {
        return Ok(());
    }

    let tool_requirements = payload_object(transition.payload.get("tool_permission_requirements"));
    let selected_tools = value_array_strings(transition.payload.get("selected_tools"));
    let mut missing = Map::new();
    for tool_id in selected_tools {
        let required = value_array_strings(tool_requirements.get(&tool_id));
        let absent = required
            .into_iter()
            .filter(|required| !permissions.contains(required))
            .map(Value::String)
            .collect::<Vec<_>>();
        if !absent.is_empty() {
            missing.insert(tool_id, Value::Array(absent));
        }
    }
    if !missing.is_empty() {
        let mut details = Payload::new();
        details.insert("missing_permissions".to_string(), Value::Object(missing));
        return Err(guard_violation(
            "tool_permission_denied",
            "TOOLKIT.COMPILED selected tools outside profile policy.",
            "",
            run.status.clone(),
            Vec::new(),
            details,
        ));
    }
    Ok(())
}

fn validate_context_budget(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    let capsule_tokens = payload_i64(transition.payload.get("capsule_tokens"));
    let budget_tokens = payload_i64(transition.payload.get("budget_tokens"));
    if capsule_tokens > budget_tokens {
        let mut details = Payload::new();
        details.insert("capsule_tokens".to_string(), number_value(capsule_tokens));
        details.insert("budget_tokens".to_string(), number_value(budget_tokens));
        return Err(guard_violation(
            "context_budget_exceeded",
            "Context capsule exceeds the requested token budget.",
            "",
            run.status.clone(),
            Vec::new(),
            details,
        ));
    }
    Ok(())
}

fn validate_outcome_payload(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    let tests_passed = payload_bool(transition.payload.get("tests_passed"));
    let manual_override = payload_bool(transition.payload.get("manual_override"));
    let validator_results = transition
        .payload
        .get("validator_results")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    if tests_passed && !manual_override && run.validators.is_empty() && !validator_results {
        return Err(guard_violation(
            "missing_validator_result",
            "tests_passed=True requires validator evidence or manual override.",
            "",
            run.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    Ok(())
}

fn validate_learning_payload(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    if run.outcome.is_none() {
        return Err(guard_violation(
            "missing_outcome",
            "LEARNING.PROPOSED requires a recorded outcome.",
            "outcome_recorded",
            run.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    if !payload_bool(transition.payload.get("review_required")) {
        return Err(guard_violation(
            "learning_review_required",
            "Learning proposals must require review.",
            "",
            run.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    let blocked = [
        "raw_content",
        "file_text",
        "prompt",
        "model_output",
        "content",
    ]
    .into_iter()
    .filter(|field| transition.payload.contains_key(*field))
    .map(str::to_string)
    .collect::<Vec<_>>();
    if !blocked.is_empty() {
        let mut details = Payload::new();
        details.insert(
            "blocked_fields".to_string(),
            Value::Array(blocked.iter().cloned().map(Value::String).collect()),
        );
        return Err(guard_violation(
            "learning_raw_content_blocked",
            "Learning proposals may only store structural hashes.",
            "",
            run.status.clone(),
            Vec::new(),
            details,
        ));
    }
    Ok(())
}

fn validate_federation_payload(
    run: &RunState,
    transition: &TransitionInput,
) -> Result<(), HarnessError> {
    if !payload_bool(transition.payload.get("consent")) {
        return Err(guard_violation(
            "federation_consent_required",
            "Federation signal preparation requires explicit consent.",
            "",
            run.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    if payload_bool(transition.payload.get("raw_content_included")) {
        return Err(guard_violation(
            "federation_raw_content_blocked",
            "Federation signals cannot include raw content.",
            "",
            run.status.clone(),
            Vec::new(),
            Payload::new(),
        ));
    }
    Ok(())
}

fn require_payload_fields(
    transition: &TransitionInput,
    fields: &'static [&'static str],
) -> Result<(), HarnessError> {
    let missing = fields
        .iter()
        .copied()
        .filter(|field| is_missing_required(transition.payload.get(*field)))
        .map(str::to_string)
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }

    Err(guard_violation(
        missing_field_code(transition.event_type.as_str(), &missing),
        format!(
            "{} missing required payload fields: {}",
            transition.event_type,
            missing.join(", ")
        ),
        "",
        "",
        missing,
        Payload::new(),
    ))
}

fn invalid_previous_state(
    event_type: &str,
    allowed: &'static [&'static str],
    received: &str,
) -> HarnessError {
    let required = allowed.join(", ");
    guard_violation(
        "invalid_previous_state",
        format!("{event_type} requires status {required}; received {received}"),
        required,
        received.to_string(),
        Vec::new(),
        Payload::new(),
    )
}

fn missing_field_code(event_type: &str, missing: &[String]) -> String {
    if event_type == "PROFILE.SELECTED" && missing.iter().any(|field| field == "policy_hash") {
        return "missing_profile_policy_hash".to_string();
    }
    if event_type == "DOMAIN.RESOLVED" && missing.iter().any(|field| field == "policy_hash") {
        return "missing_domain_policy_hash".to_string();
    }
    if event_type == "CONTEXT.INJECTED" && missing.iter().any(|field| field == "artifact_id") {
        return "missing_context_artifact".to_string();
    }
    "missing_payload_fields".to_string()
}

fn guard_violation(
    code: impl Into<String>,
    message: impl Into<String>,
    required_state: impl Into<String>,
    received_state: impl Into<String>,
    missing_fields: Vec<String>,
    details: Payload,
) -> HarnessError {
    HarnessError::Guard(Box::new(GuardViolation {
        code: code.into(),
        message: message.into(),
        required_state: required_state.into(),
        received_state: received_state.into(),
        missing_fields,
        details,
    }))
}

fn context_artifact_id(run: &RunState) -> String {
    run.context
        .as_ref()
        .and_then(|context| context.get("artifact_id"))
        .map(|value| payload_to_string(Some(value)))
        .unwrap_or_default()
}

fn merge_binding(current: Option<&Payload>, update: Payload) -> Payload {
    let mut merged = current.cloned().unwrap_or_default();
    for (key, value) in update {
        merged.insert(key, value);
    }
    merged
}

fn payload_object(value: Option<&Value>) -> Payload {
    match value {
        Some(Value::Object(map)) => map.clone(),
        _ => Payload::new(),
    }
}

fn value_array_strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| payload_to_string(Some(item)))
            .filter(|item| !item.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn payload_to_string(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(value)) => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(Value::Bool(value)) => value.to_string(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

fn payload_i64(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(value)) => value.as_i64().unwrap_or(0),
        Some(Value::String(value)) => value.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn payload_bool(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value == "true",
        _ => false,
    }
}

fn number_value(value: i64) -> Value {
    Value::Number(value.into())
}

fn is_missing_required(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => true,
        Some(Value::String(value)) => value.is_empty(),
        _ => false,
    }
}

fn first_non_empty(values: &[String]) -> String {
    values
        .iter()
        .find(|value| !value.is_empty())
        .cloned()
        .unwrap_or_default()
}

fn is_cache_event(event: &str) -> bool {
    matches!(
        event,
        "CACHE.CHECKED"
            | "CACHE.HIT"
            | "CACHE.HIT_VALIDATED"
            | "CACHE.HIT_REJECTED"
            | "CACHE.MISS"
            | "CACHE.STAGE_REUSED"
            | "CACHE.ENTRY_STORED"
            | "CACHE.INVALIDATED"
    )
}

fn is_oracle_event(event: &str) -> bool {
    matches!(
        event,
        "ORACLE.REQUESTED" | "ORACLE.RETURNED" | "STATE.PATCHED" | "ADAPTER.SELECTED"
    )
}

fn is_cua_event(event: &str) -> bool {
    matches!(
        event,
        "DEVICE.SESSION.STARTED"
            | "DEVICE.SESSION.CLOSED"
            | "DEVICE.SESSION.ERRORED"
            | "CUA.SANDBOX.OPENED"
            | "CUA.SANDBOX.CLOSED"
            | "CUA.ACTION.OBSERVED"
            | "CUA.OBSERVATION.RECORDED"
            | "CUA.TRAJECTORY.EXPORTED"
    )
}

fn is_cmh_event(event: &str) -> bool {
    matches!(
        event,
        "MEMORY.SYNCED"
            | "HANDOFF.COMPILED"
            | "HANDOFF.INJECTED"
            | "SESSION.EVENT_RECORDED"
            | "MEMORY.CANONICALIZED"
            | "WORKSTREAM.UPDATED"
            | "NEXT_AGENT.READY"
    )
}

fn cmh_target_status(event: &str) -> &'static str {
    match event {
        "MEMORY.SYNCED" => "memory_synced",
        "HANDOFF.COMPILED" => "handoff_compiled",
        "HANDOFF.INJECTED" => "handoff_injected",
        "SESSION.EVENT_RECORDED" => "agent_acting",
        "MEMORY.CANONICALIZED" => "memory_canonicalized",
        "WORKSTREAM.UPDATED" => "workstream_updated",
        "NEXT_AGENT.READY" => "next_agent_ready",
        _ => "",
    }
}

fn transition_requirements(event: &str) -> &'static [&'static str] {
    match event {
        "RUN.CREATED" => &["task", "actor"],
        "HOST.OBSERVED" => &["repo", "branch", "commit_sha", "cwd"],
        "TASK.RESOLVED" => &["task_signature"],
        "PROFILE.SELECTED" => &["profile_id", "profile_version", "policy_hash"],
        "DOMAIN.RESOLVED" => &["domain", "domain_version", "policy_hash"],
        "TOOLKIT.COMPILED" | "TOOLPACK.COMPILED" => &[
            "selected_tools",
            "selected_plugins",
            "excluded_tools",
            "permission_reasons",
        ],
        "MAPS.LOADED" => &["maps"],
        "CONTEXT.PLANNED" => &["budget_tokens", "plan_hash", "candidate_token_count"],
        "CONTEXT.PACKED" | "CONTEXT.COMPILED" => &[
            "artifact_id",
            "capsule_tokens",
            "budget_tokens",
            "included_atom_count",
            "excluded_atom_count",
            "token_ledger",
        ],
        "CONTEXT.INJECTED" => &["artifact_id", "adapter", "target"],
        "AGENT.ACTING" => &["adapter", "started_at"],
        "VALIDATION.RUNNING" | "VALIDATION.STARTED" => &["validator_id", "command"],
        "VALIDATION.FINISHED" => &["validator_id", "status", "exit_code", "summary"],
        "OUTCOME.RECORDED" => &[
            "accepted",
            "tests_passed",
            "validator_results",
            "files_changed",
            "summary",
        ],
        "LEARNING.PROPOSED" => &[
            "patch_type",
            "confidence",
            "review_required",
            "payload_hash",
        ],
        "MEMORY.PATCHED" => &["patch_id", "status", "review_required", "payload_hash"],
        "MAPS.UPDATED" => &["maps", "state_hash"],
        "REVIEW.QUEUED" => &["review_type", "review_target_id"],
        "FEDERATION.SIGNAL_PREPARED" => &[
            "plugin_id",
            "profile_id",
            "task_type",
            "task_signature_hash",
            "context_shape_hash",
            "outcome_bucket",
            "token_bucket",
            "raw_content_included",
        ],
        "RUN.CLOSED" => &["summary", "closed_by"],
        "RUN.FAILED" => &["error_code", "message"],
        "RUN.CANCELLED" => &["reason", "cancelled_by"],
        "RUN.REPLAYED" => &["source_run_id"],
        "RUN.FORKED" => &["source_run_id", "through_event_seq"],
        "MEMORY.SYNCED" => &["workstream_id"],
        "HANDOFF.COMPILED" => &["handoff_id", "token_estimate"],
        "HANDOFF.INJECTED" => &["delivered_to", "delivered_at"],
        "SESSION.EVENT_RECORDED" => &["event_subtype"],
        "MEMORY.CANONICALIZED" => &["atoms_created", "atoms_updated", "atoms_superseded"],
        "WORKSTREAM.UPDATED" => &["workstream_id", "new_task_state"],
        "NEXT_AGENT.READY" => &["next_handoff_id"],
        _ => &[],
    }
}

fn allowed_previous_statuses(event: &str) -> &'static [&'static str] {
    match event {
        "HOST.OBSERVED" => &["created", "observed"],
        "TASK.RESOLVED" => &["created", "observed", "resolved"],
        "PROFILE.SELECTED" => &[
            "resolved",
            "cache_checked",
            "cache_hit_validated",
            "cache_hit_rejected",
            "cache_miss",
            "profile_selected",
        ],
        "DOMAIN.RESOLVED" => &[
            "resolved",
            "cache_checked",
            "cache_hit_validated",
            "cache_hit_rejected",
            "cache_miss",
            "profile_selected",
            "domain_resolved",
        ],
        "TOOLKIT.COMPILED" => &["profile_selected", "toolkit_compiled"],
        "TOOLPACK.COMPILED" => &[
            "profile_selected",
            "domain_resolved",
            "toolkit_compiled",
            "toolpack_compiled",
        ],
        "MAPS.LOADED" => &["toolkit_compiled", "toolpack_compiled", "maps_loaded"],
        "CONTEXT.PLANNED" => &[
            "toolkit_compiled",
            "toolpack_compiled",
            "maps_loaded",
            "context_planned",
        ],
        "CONTEXT.PACKED" => &["context_planned", "context_packed"],
        "CONTEXT.COMPILED" => &["context_planned", "context_compiled"],
        "CONTEXT.INJECTED" => &["context_packed", "context_compiled", "context_injected"],
        "AGENT.ACTING" => &["context_injected", "agent_acting"],
        "VALIDATION.RUNNING" | "VALIDATION.STARTED" => &["agent_acting", "validating"],
        "VALIDATION.FINISHED" => &["validating"],
        "OUTCOME.RECORDED" => &["agent_acting", "validating", "outcome_recorded"],
        "LEARNING.PROPOSED" => &["outcome_recorded", "learning_proposed"],
        "MEMORY.PATCHED" => &["outcome_recorded", "learning_proposed", "memory_patched"],
        "MAPS.UPDATED" => &[
            "outcome_recorded",
            "learning_proposed",
            "memory_patched",
            "maps_updated",
        ],
        "REVIEW.QUEUED" => &["learning_proposed", "review_queued"],
        "FEDERATION.SIGNAL_PREPARED" => &["review_queued", "federation_signal_prepared"],
        "RUN.CLOSED" => &[
            "outcome_recorded",
            "learning_proposed",
            "memory_patched",
            "maps_updated",
            "review_queued",
            "federation_signal_prepared",
            "memory_synced",
            "handoff_compiled",
            "handoff_injected",
            "memory_canonicalized",
            "workstream_updated",
            "next_agent_ready",
        ],
        "RUN.REPLAYED" => &["closed"],
        "RUN.FORKED" => &["closed", "failed"],
        "MEMORY.SYNCED" => &["created", "observed", "memory_synced"],
        "HANDOFF.COMPILED" => &["memory_synced", "handoff_compiled"],
        "HANDOFF.INJECTED" => &["handoff_compiled", "handoff_injected"],
        "SESSION.EVENT_RECORDED" => &["agent_acting"],
        "MEMORY.CANONICALIZED" => &["outcome_recorded", "memory_canonicalized"],
        "WORKSTREAM.UPDATED" => &["memory_canonicalized", "workstream_updated"],
        "NEXT_AGENT.READY" => &["workstream_updated", "next_agent_ready"],
        _ => &[],
    }
}

fn cache_event_requirements(event: &str) -> &'static [&'static str] {
    match event {
        "CACHE.CHECKED" => &["backend", "outcome"],
        "CACHE.HIT" => &["cache_entry_id", "backend"],
        "CACHE.HIT_VALIDATED" => &["cache_entry_id", "graph_state_hash"],
        "CACHE.HIT_REJECTED" => &["cache_entry_id", "rejection_reason"],
        "CACHE.MISS" => &["backend", "outcome"],
        "CACHE.STAGE_REUSED" => &["stage", "cache_entry_id"],
        "CACHE.ENTRY_STORED" => &["cache_entry_id", "backend"],
        "CACHE.INVALIDATED" => &["cache_entry_id", "reason"],
        _ => &[],
    }
}

fn cache_allowed_previous(event: &str) -> &'static [&'static str] {
    match event {
        "CACHE.CHECKED" => &["resolved"],
        "CACHE.HIT" => &["cache_checked"],
        "CACHE.HIT_VALIDATED" | "CACHE.HIT_REJECTED" => &["cache_checked", "cache_hit"],
        "CACHE.MISS" => &["cache_checked"],
        _ => &[],
    }
}

fn is_compound_event(event: &str) -> bool {
    event.starts_with("COMPOUND.")
}

fn oracle_event_requirements(event: &str) -> &'static [&'static str] {
    match event {
        "ORACLE.REQUESTED" => &["tool_name", "request_id"],
        "ORACLE.RETURNED" => &["tool_name", "request_id", "oracle_packet"],
        "STATE.PATCHED" => &["request_id", "applied_patch_ids", "rejected_patch_ids"],
        "ADAPTER.SELECTED" => &["adapter_id", "role"],
        _ => &[],
    }
}

fn cua_event_requirements(event: &str) -> &'static [&'static str] {
    match event {
        "DEVICE.SESSION.STARTED" => &["device_session_id", "provider"],
        "DEVICE.SESSION.CLOSED" => &["device_session_id"],
        "DEVICE.SESSION.ERRORED" => &["device_session_id", "error_code"],
        "CUA.SANDBOX.OPENED" => &["device_session_id", "sandbox_id"],
        "CUA.SANDBOX.CLOSED" => &["sandbox_id"],
        "CUA.ACTION.OBSERVED" => &["sandbox_id", "action_id", "kind", "seq"],
        "CUA.OBSERVATION.RECORDED" => &["sandbox_id", "observation_id", "kind", "seq"],
        "CUA.TRAJECTORY.EXPORTED" => &[
            "sandbox_id",
            "trajectory_id",
            "action_count",
            "observation_count",
        ],
        _ => &[],
    }
}

const TERMINAL_STATUSES: &[&str] = &["closed", "failed", "cancelled"];

const ORACLE_LIVE_STATUSES: &[&str] = &[
    "created",
    "observed",
    "resolved",
    "cache_checked",
    "cache_hit",
    "cache_hit_validated",
    "cache_hit_rejected",
    "cache_miss",
    "profile_selected",
    "toolkit_compiled",
    "context_planned",
    "context_packed",
    "context_injected",
    "agent_acting",
    "validating",
    "outcome_recorded",
    "learning_proposed",
    "review_queued",
    "federation_signal_prepared",
];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Map, Value};

    #[test]
    fn applies_full_minimal_lifecycle_and_hashes_each_event() {
        let run = created();
        assert_eq!(run.run.status, "created");
        assert_eq!(run.event.seq, 1);
        assert_eq!(run.state_hash_before, empty_state_hash());

        let run = apply(
            run.run,
            "HOST.OBSERVED",
            json!({
                "repo": "Theorem",
                "branch": "main",
                "commit_sha": "db238ec",
                "cwd": "/tmp/theorem"
            }),
        );
        let run = apply(
            run.run,
            "TASK.RESOLVED",
            json!({
                "task_signature": "sig:harness-port"
            }),
        );
        let run = apply(
            run.run,
            "PROFILE.SELECTED",
            json!({
                "profile_id": "commonplace-v3",
                "profile_version": "1",
                "policy_hash": "policy:1"
            }),
        );
        let run = apply(run.run, "TOOLKIT.COMPILED", toolkit_payload());
        let run = apply(
            run.run,
            "MAPS.LOADED",
            json!({
                "maps": [{"id": "codebase-map"}]
            }),
        );
        let run = apply(
            run.run,
            "CONTEXT.PLANNED",
            json!({
                "budget_tokens": 4000,
                "plan_hash": "plan:1",
                "candidate_token_count": 3200
            }),
        );
        let run = apply(
            run.run,
            "CONTEXT.PACKED",
            context_payload("ctx:1", 3000, 4000),
        );
        let run = apply(
            run.run,
            "CONTEXT.INJECTED",
            json!({
                "artifact_id": "ctx:1",
                "adapter": "codex",
                "target": "active_context"
            }),
        );
        let run = apply(
            run.run,
            "AGENT.ACTING",
            json!({
                "adapter": "codex",
                "started_at": "2026-06-01T00:00:00Z"
            }),
        );
        let run = apply(
            run.run,
            "OUTCOME.RECORDED",
            json!({
                "accepted": true,
                "tests_passed": true,
                "manual_override": true,
                "validator_results": [],
                "files_changed": ["rustyredcore_THG/crates/theorem-harness-core/src/state_machine.rs"],
                "summary": "kernel slice green"
            }),
        );
        let run = apply(
            run.run,
            "RUN.CLOSED",
            json!({
                "summary": "closed",
                "closed_by": "codex"
            }),
        );

        assert_eq!(run.run.status, "closed");
        assert_eq!(run.event.event_type, "RUN.CLOSED");
        assert_eq!(run.event.state_hash_after, run.state_hash_after);
        assert!(!run.state_hash_after.is_empty());
    }

    #[test]
    fn created_run_hash_matches_python_reference_fixture() {
        let result = apply_transition(
            None,
            transition(
                "RUN.CREATED",
                json!({
                    "task": "Port the harness",
                    "actor": "codex",
                    "scope": {
                        "repo": "Theorem",
                        "branch": "main",
                        "commit_sha": "db238ec",
                        "workstream_id": "harness-rust-port",
                        "agent_host": "codex",
                        "agent_model": "gpt-5"
                    }
                }),
            )
            .with_run_id("run:fixed"),
        )
        .unwrap();

        assert_eq!(
            result.state_hash_after,
            "37d095e467921c538f268b58f057b9ffee9cf5522559e887916d62944ecd818c"
        );
    }

    #[test]
    fn context_budget_exceeded_raises_guard_code() {
        let run = ready_for_context_pack();
        let error = apply_transition(
            Some(run),
            transition("CONTEXT.PACKED", context_payload("ctx:bad", 5000, 4000)),
        )
        .unwrap_err();

        assert_guard(error, "context_budget_exceeded");
    }

    #[test]
    fn context_injected_requires_matching_artifact() {
        let run = apply(
            ready_for_context_pack(),
            "CONTEXT.PACKED",
            context_payload("ctx:packed", 2000, 4000),
        );
        let error = apply_transition(
            Some(run.run),
            transition(
                "CONTEXT.INJECTED",
                json!({
                    "artifact_id": "ctx:other",
                    "adapter": "codex",
                    "target": "active_context"
                }),
            ),
        )
        .unwrap_err();

        assert_guard(error, "context_artifact_mismatch");
    }

    #[test]
    fn learning_and_memory_patches_require_review() {
        let run = outcome_recorded_run();
        let learning_error = apply_transition(
            Some(run.clone()),
            transition(
                "LEARNING.PROPOSED",
                json!({
                    "patch_type": "memory",
                    "confidence": 0.7,
                    "review_required": false,
                    "payload_hash": "hash:1"
                }),
            ),
        )
        .unwrap_err();
        assert_guard(learning_error, "learning_review_required");

        let memory_error = apply_transition(
            Some(run),
            transition(
                "MEMORY.PATCHED",
                json!({
                    "patch_id": "patch:1",
                    "status": "proposed",
                    "review_required": false,
                    "payload_hash": "hash:1"
                }),
            ),
        )
        .unwrap_err();
        assert_guard(memory_error, "memory_patch_review_required");
    }

    #[test]
    fn oracle_events_are_status_preserving_but_still_logged() {
        let run = agent_acting_run();
        let result = apply(
            run,
            "ORACLE.REQUESTED",
            json!({
                "tool_name": "deepseek_reason",
                "request_id": "oracle:1"
            }),
        );

        assert_eq!(result.run.status, "agent_acting");
        assert_eq!(result.run.last_event_seq, 9);
        assert_eq!(result.event.event_type, "ORACLE.REQUESTED");
    }

    #[test]
    fn cache_events_advance_cache_statuses() {
        let run = resolved_run();
        let checked = apply(
            run,
            "CACHE.CHECKED",
            json!({
                "backend": "rustyred",
                "outcome": "miss"
            }),
        );
        assert_eq!(checked.run.status, "cache_checked");

        let hit = apply(
            checked.run,
            "CACHE.HIT",
            json!({
                "cache_entry_id": "cache:1",
                "backend": "rustyred"
            }),
        );
        assert_eq!(hit.run.status, "cache_hit");
        assert_eq!(hit.run.cache_events.len(), 2);
    }

    #[test]
    fn terminal_runs_reject_observation_events_but_allow_fork() {
        let closed = closed_run();
        let error = apply_transition(
            Some(closed.clone()),
            transition(
                "ORACLE.REQUESTED",
                json!({
                    "tool_name": "deepseek_reason",
                    "request_id": "oracle:closed"
                }),
            ),
        )
        .unwrap_err();
        assert_guard(error, "terminal_run_state");

        let fork = apply(
            closed,
            "RUN.FORKED",
            json!({
                "source_run_id": "source:1",
                "through_event_seq": 4
            }),
        );
        assert_eq!(fork.run.status, "created");
        assert_eq!(
            fork.run.scope.get("source_run_id"),
            Some(&json!("source:1"))
        );
        assert!(fork.run.outcome.is_none());
        assert!(fork.run.context.is_none());
    }

    #[test]
    fn compound_events_append_after_terminal_run_without_reopening_it() {
        let closed = closed_run();
        let compound = apply(
            closed,
            "COMPOUND.CAPTURED",
            json!({
                "config_hash": "sha256:compound",
                "captured": true
            }),
        );

        assert_eq!(compound.run.status, "closed");
        assert_eq!(compound.event.event_type, "COMPOUND.CAPTURED");
    }

    fn closed_run() -> RunState {
        apply(
            outcome_recorded_run(),
            "RUN.CLOSED",
            json!({
                "summary": "closed",
                "closed_by": "codex"
            }),
        )
        .run
    }

    fn outcome_recorded_run() -> RunState {
        apply(
            agent_acting_run(),
            "OUTCOME.RECORDED",
            json!({
                "accepted": true,
                "tests_passed": true,
                "manual_override": true,
                "validator_results": [],
                "files_changed": [],
                "summary": "done"
            }),
        )
        .run
    }

    fn agent_acting_run() -> RunState {
        let run = apply(
            ready_for_context_pack(),
            "CONTEXT.PACKED",
            context_payload("ctx:1", 2000, 4000),
        );
        let run = apply(
            run.run,
            "CONTEXT.INJECTED",
            json!({
                "artifact_id": "ctx:1",
                "adapter": "codex",
                "target": "active_context"
            }),
        );
        apply(
            run.run,
            "AGENT.ACTING",
            json!({
                "adapter": "codex",
                "started_at": "2026-06-01T00:00:00Z"
            }),
        )
        .run
    }

    fn ready_for_context_pack() -> RunState {
        let run = apply(
            resolved_run(),
            "PROFILE.SELECTED",
            json!({
                "profile_id": "commonplace-v3",
                "profile_version": "1",
                "policy_hash": "policy:1"
            }),
        );
        let run = apply(run.run, "TOOLKIT.COMPILED", toolkit_payload());
        apply(
            run.run,
            "CONTEXT.PLANNED",
            json!({
                "budget_tokens": 4000,
                "plan_hash": "plan:1",
                "candidate_token_count": 3200
            }),
        )
        .run
    }

    fn resolved_run() -> RunState {
        apply(
            created().run,
            "TASK.RESOLVED",
            json!({
                "task_signature": "sig:harness-port"
            }),
        )
        .run
    }

    fn created() -> TransitionResult {
        apply_transition(
            None,
            transition(
                "RUN.CREATED",
                json!({
                    "task": "Port the harness",
                    "actor": "codex",
                    "scope": {
                        "repo": "Theorem",
                        "branch": "main",
                        "commit_sha": "db238ec",
                        "workstream_id": "harness-rust-port",
                        "agent_host": "codex",
                        "agent_model": "gpt-5"
                    }
                }),
            ),
        )
        .unwrap()
    }

    fn apply(run: RunState, event_type: &str, payload: Value) -> TransitionResult {
        apply_transition(Some(run), transition(event_type, payload)).unwrap()
    }

    fn transition(event_type: &str, payload: Value) -> TransitionInput {
        TransitionInput::new(event_type, object_payload(payload)).at("2026-06-01T00:00:00Z")
    }

    fn toolkit_payload() -> Value {
        json!({
            "selected_tools": ["rustyred"],
            "selected_plugins": [],
            "excluded_tools": [],
            "permission_reasons": {}
        })
    }

    fn context_payload(artifact_id: &str, capsule_tokens: i64, budget_tokens: i64) -> Value {
        json!({
            "artifact_id": artifact_id,
            "capsule_tokens": capsule_tokens,
            "budget_tokens": budget_tokens,
            "included_atom_count": 4,
            "excluded_atom_count": 2,
            "token_ledger": []
        })
    }

    fn object_payload(payload: Value) -> Payload {
        match payload {
            Value::Object(map) => map,
            _ => Map::new(),
        }
    }

    fn assert_guard(error: HarnessError, expected_code: &str) {
        match error {
            HarnessError::Guard(violation) => assert_eq!(violation.code, expected_code),
        }
    }
}
