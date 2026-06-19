//! Agent Queue capture: a mobile-created TickTick task becomes a dispatch job.
//!
//! This is the mechanical, deterministic half of the local loop (CHK004-008). No
//! model is in this path: on each tick the daemon reads exactly one dedicated
//! list (the Agent Queue), converts each task there to a job, stamps the job id
//! back into the task, checks the task's `dispatched` subtask, and moves the task
//! out to the product list. Tasks anywhere else are never touched (CHK007).
//!
//! Idempotency: a task already stamped with its job id is skipped, and `job_submit`
//! itself dedupes by idempotency_key, so a task that fails to move stays capturable
//! without creating a duplicate job.

use serde::Serialize;
use serde_json::{json, Value};

use crate::config::CaptureConfig;
use crate::mcp::ToolGateway;
use crate::{AgentdError, AgentdResult};

/// MCP server name for the harness (jobs, memory, coordination).
pub const HARNESS: &str = "harness";
/// MCP server name for the TickTick task surface.
pub const TICKTICK: &str = "ticktick";

/// Marker stamped into a captured task's content so re-sweeps are idempotent and
/// the operator can see, on the phone, which job a task became.
pub const STAMP_PREFIX: &str = "[agentd] dispatched as ";

/// One task converted to a job in a sweep.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CapturedTask {
    pub task_id: String,
    pub job_id: String,
    /// False when `job_submit` matched an existing idempotency key.
    pub created: bool,
}

/// Outcome of one Agent Queue sweep.
#[derive(Clone, Debug, Serialize)]
pub struct CaptureReport {
    pub project_id: String,
    pub captured: Vec<CapturedTask>,
    /// Tasks skipped because they were already stamped/dispatched.
    pub skipped: usize,
}

/// Map a TickTick integer priority (0 none, 1 low, 3 medium, 5 high) to the job
/// board's P0/P1/P2 (CHK005).
pub fn priority_from_ticktick(priority: i64) -> &'static str {
    match priority {
        5 => "P0",
        3 => "P1",
        _ => "P2",
    }
}

/// Run one capture sweep over the configured Agent Queue list.
///
/// Returns the captured tasks. Errors only on a failure to read the list or
/// submit a job; per-task side effects (stamp, subtask, move) are best-effort so
/// one wedged task cannot stall the sweep.
pub fn run_capture(
    gateway: &dyn ToolGateway,
    config: &CaptureConfig,
    submitted_by: &str,
) -> AgentdResult<CaptureReport> {
    let project_id = resolve_agent_queue_id(gateway, config)?;
    let project = gateway.call_server(
        TICKTICK,
        "ticktick_get_project",
        json!({ "params": { "project_id": project_id, "response_format": "json" } }),
    )?;
    let payload = ticktick_json(&project);
    let tasks = payload
        .get("tasks")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut captured = Vec::new();
    let mut skipped = 0usize;
    for task in &tasks {
        // CHK007: only ever convert tasks that actually live in the Agent Queue.
        if let Some(task_project) = task.get("projectId").and_then(Value::as_str) {
            if task_project != project_id {
                continue;
            }
        }
        let Some(task_id) = task.get("id").and_then(Value::as_str) else {
            continue;
        };
        // Idempotency: a task already stamped was captured on a prior tick.
        if is_already_captured(task, config) {
            skipped += 1;
            continue;
        }

        let submission = build_submission(task, &project_id, submitted_by, config);
        let result = gateway.call_server(HARNESS, "job_submit", submission)?;
        let job_id = extract_job_id(&result);
        let created = result
            .get("created")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // CHK006: stamp the job id, check the dispatched subtask, move to product.
        let _ = stamp_and_dispatch(gateway, task, task_id, &project_id, &job_id, config);
        if let Some(dest) = config.dispatched_project_id.as_deref() {
            if dest != project_id {
                let _ = gateway.call_server(
                    TICKTICK,
                    "ticktick_move_task",
                    json!({ "params": {
                        "task_id": task_id,
                        "from_project_id": project_id,
                        "to_project_id": dest,
                    }}),
                );
            }
        }

        captured.push(CapturedTask {
            task_id: task_id.to_string(),
            job_id,
            created,
        });
    }

    Ok(CaptureReport {
        project_id,
        captured,
        skipped,
    })
}

/// Resolve the Agent Queue project id from config, by id first then by name.
fn resolve_agent_queue_id(
    gateway: &dyn ToolGateway,
    config: &CaptureConfig,
) -> AgentdResult<String> {
    if let Some(id) = config
        .agent_queue_project_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
    {
        return Ok(id.to_string());
    }
    let Some(name) = config
        .agent_queue_project_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
    else {
        return Err(AgentdError::Config(
            "capture.agent_queue_project_id or capture.agent_queue_project_name is required"
                .to_string(),
        ));
    };
    let projects = gateway.call_server(
        TICKTICK,
        "ticktick_list_projects",
        json!({ "params": { "response_format": "json" } }),
    )?;
    let list = ticktick_json(&projects);
    let array = list
        .as_array()
        .cloned()
        .or_else(|| list.get("projects").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    array
        .iter()
        .find(|project| project.get("name").and_then(Value::as_str) == Some(name))
        .and_then(|project| project.get("id").and_then(Value::as_str))
        .map(str::to_string)
        .ok_or_else(|| AgentdError::Config(format!("no TickTick list named '{name}'")))
}

/// Build the `job_submit` arguments for one task (CHK005).
fn build_submission(
    task: &Value,
    project_id: &str,
    submitted_by: &str,
    config: &CaptureConfig,
) -> Value {
    let title = task
        .get("title")
        .and_then(Value::as_str)
        .unwrap_or("Untitled Agent Queue task");
    let content = task.get("content").and_then(Value::as_str).unwrap_or("");
    let task_id = task.get("id").and_then(Value::as_str).unwrap_or("");
    let priority = task.get("priority").and_then(Value::as_i64).unwrap_or(0);

    let mut submission = json!({
        "title": title,
        "spec_inline": content,
        "repo": config.repo,
        "priority": priority_from_ticktick(priority),
        "submitted_by": submitted_by,
        "source_task_id": task_id,
        "source_project_id": project_id,
    });
    if let Some(head) = config.target_head.as_deref().filter(|h| !h.is_empty()) {
        submission["target_head"] = json!(head);
    }
    submission
}

/// Stamp the job id into the task content and check the dispatched subtask. One
/// `ticktick_update_task` call carries both (CHK006). Idempotent: if the content
/// is already stamped and the subtask already checked, nothing is written.
fn stamp_and_dispatch(
    gateway: &dyn ToolGateway,
    task: &Value,
    task_id: &str,
    project_id: &str,
    job_id: &str,
    config: &CaptureConfig,
) -> AgentdResult<()> {
    let content = task.get("content").and_then(Value::as_str).unwrap_or("");
    let stamped_content = stamped(content, job_id);
    let subtasks = dispatched_subtasks(task, &config.dispatched_subtask_title);

    gateway.call_server(
        TICKTICK,
        "ticktick_update_task",
        json!({ "params": {
            "task_id": task_id,
            "project_id": project_id,
            "content": stamped_content,
            "subtasks": subtasks,
        }}),
    )?;
    Ok(())
}

/// Append the dispatch stamp to the content unless it is already present.
fn stamped(content: &str, job_id: &str) -> String {
    if content.contains(STAMP_PREFIX) {
        return content.to_string();
    }
    let stamp = format!("{STAMP_PREFIX}{job_id}");
    if content.trim().is_empty() {
        stamp
    } else {
        format!("{content}\n\n{stamp}")
    }
}

/// The task's existing checklist items as an update payload, with the dispatched
/// item checked. The item is appended when the task did not already carry one.
fn dispatched_subtasks(task: &Value, dispatched_title: &str) -> Value {
    let mut items: Vec<Value> = Vec::new();
    let mut found = false;
    if let Some(existing) = task.get("items").and_then(Value::as_array) {
        for item in existing {
            let title = item.get("title").and_then(Value::as_str).unwrap_or("");
            let is_dispatched = title.eq_ignore_ascii_case(dispatched_title);
            if is_dispatched {
                found = true;
            }
            items.push(json!({
                "title": title,
                "status": if is_dispatched { 1 } else {
                    item.get("status").and_then(Value::as_i64).unwrap_or(0)
                },
            }));
        }
    }
    if !found {
        items.push(json!({ "title": dispatched_title, "status": 1 }));
    }
    json!(items)
}

/// A task is already captured when its content carries the dispatch stamp.
fn is_already_captured(task: &Value, _config: &CaptureConfig) -> bool {
    task.get("content")
        .and_then(Value::as_str)
        .map(|content| content.contains(STAMP_PREFIX))
        .unwrap_or(false)
}

/// The job id from a harness `job_submit` payload (`{job_id, created, job}`).
fn extract_job_id(result: &Value) -> String {
    result
        .get("job_id")
        .and_then(Value::as_str)
        .or_else(|| {
            result
                .get("job")
                .and_then(|job| job.get("job_id"))
                .and_then(Value::as_str)
        })
        .unwrap_or_default()
        .to_string()
}

/// Drill a TickTick MCP response down to its meaningful JSON value.
///
/// Session-mode MCP calls hand back the raw MCP result `{content:[{text}]}`; the
/// text is the tool's output, which may itself be `{"result": "<json string>"}`
/// or `{"result": {...}}` or the bare object. This tolerates every layer so the
/// caller always sees the inner value (e.g. the `{project, tasks}` object).
pub fn ticktick_json(value: &Value) -> Value {
    let mut current = match content_text(value) {
        Some(text) => serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text)),
        None => value.clone(),
    };
    for _ in 0..3 {
        let Some(inner) = current.get("result").cloned() else {
            break;
        };
        current = match inner {
            Value::String(raw) => serde_json::from_str::<Value>(&raw).unwrap_or(Value::String(raw)),
            other => other,
        };
    }
    current
}

/// Extract `content[0].text` from an MCP result envelope, if present.
fn content_text(value: &Value) -> Option<String> {
    if let Some(text) = value
        .get("content")
        .and_then(|content| content.get(0))
        .and_then(|entry| entry.get("text"))
        .and_then(Value::as_str)
    {
        return Some(text.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A recording fake gateway: returns canned responses per tool name and
    /// records every call so tests can assert the mechanical sequence.
    struct FakeGateway {
        calls: RefCell<Vec<(String, String, Value)>>,
        responses: RefCell<std::collections::HashMap<String, Value>>,
    }

    impl FakeGateway {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                responses: RefCell::new(std::collections::HashMap::new()),
            }
        }

        fn respond(&self, name: &str, value: Value) {
            self.responses.borrow_mut().insert(name.to_string(), value);
        }

        fn calls_to(&self, name: &str) -> Vec<Value> {
            self.calls
                .borrow()
                .iter()
                .filter(|(_, n, _)| n == name)
                .map(|(_, _, args)| args.clone())
                .collect()
        }
    }

    impl ToolGateway for FakeGateway {
        fn call_server(&self, server: &str, name: &str, arguments: Value) -> AgentdResult<Value> {
            self.calls
                .borrow_mut()
                .push((server.to_string(), name.to_string(), arguments));
            Ok(self
                .responses
                .borrow()
                .get(name)
                .cloned()
                .unwrap_or(json!({})))
        }
    }

    fn project_with_one_task() -> Value {
        // The exact shape ticktick_get_project returns (subtasks under `items`),
        // wrapped in the MCP content/result envelope a session call delivers.
        let inner = json!({
            "project": { "id": "list-aq", "name": "Agent Queue" },
            "tasks": [{
                "id": "task-1",
                "projectId": "list-aq",
                "title": "Add a /health endpoint",
                "content": "Add GET /health to theorem-grpc.",
                "priority": 3,
                "status": 0,
                "items": [{ "id": "sub-1", "title": "dispatched", "status": 0 }]
            }]
        });
        json!({ "content": [{ "type": "text", "text": inner.to_string() }] })
    }

    #[test]
    fn priority_maps_ticktick_ints_to_board_levels() {
        assert_eq!(priority_from_ticktick(5), "P0");
        assert_eq!(priority_from_ticktick(3), "P1");
        assert_eq!(priority_from_ticktick(1), "P2");
        assert_eq!(priority_from_ticktick(0), "P2");
    }

    #[test]
    fn ticktick_json_drills_content_and_result_layers() {
        // content -> text(json) with a nested stringified result.
        let envelope = json!({
            "content": [{ "type": "text", "text": json!({"result": json!({"ok": true}).to_string()}).to_string() }]
        });
        assert_eq!(ticktick_json(&envelope), json!({ "ok": true }));
        // bare object passes through.
        assert_eq!(ticktick_json(&json!({"tasks": []})), json!({"tasks": []}));
    }

    #[test]
    fn stamp_is_idempotent() {
        let once = stamped("body", "job-1");
        assert!(once.contains("[agentd] dispatched as job-1"));
        // Re-stamping content that already carries a stamp is a no-op.
        assert_eq!(stamped(&once, "job-2"), once);
    }

    #[test]
    fn dispatched_subtask_is_checked_and_preserves_others() {
        let task = json!({
            "items": [
                { "title": "dispatched", "status": 0 },
                { "title": "write tests", "status": 0 }
            ]
        });
        let subtasks = dispatched_subtasks(&task, "dispatched");
        let array = subtasks.as_array().unwrap();
        assert_eq!(array.len(), 2);
        assert_eq!(array[0]["title"], "dispatched");
        assert_eq!(array[0]["status"], 1);
        assert_eq!(array[1]["title"], "write tests");
        assert_eq!(array[1]["status"], 0);
    }

    #[test]
    fn dispatched_subtask_is_added_when_absent() {
        let task = json!({ "items": [] });
        let subtasks = dispatched_subtasks(&task, "dispatched");
        let array = subtasks.as_array().unwrap();
        assert_eq!(array.len(), 1);
        assert_eq!(array[0]["title"], "dispatched");
        assert_eq!(array[0]["status"], 1);
    }

    #[test]
    fn capture_converts_task_to_job_with_source_correspondence() {
        let gateway = FakeGateway::new();
        gateway.respond("ticktick_get_project", project_with_one_task());
        gateway.respond(
            "job_submit",
            json!({ "job_id": "job-abc", "created": true, "job": { "job_id": "job-abc" } }),
        );
        let config = CaptureConfig {
            enabled: true,
            agent_queue_project_id: Some("list-aq".to_string()),
            dispatched_project_id: Some("list-wip".to_string()),
            ..Default::default()
        };

        let report = run_capture(&gateway, &config, "theorem-agentd").unwrap();
        assert_eq!(report.captured.len(), 1);
        let captured = &report.captured[0];
        assert_eq!(captured.task_id, "task-1");
        assert_eq!(captured.job_id, "job-abc");

        // CHK005: the job carries title, content->spec_inline, mapped priority,
        // and the source correspondence so the loop can relay back.
        let submit = &gateway.calls_to("job_submit")[0];
        assert_eq!(submit["title"], "Add a /health endpoint");
        assert_eq!(submit["spec_inline"], "Add GET /health to theorem-grpc.");
        assert_eq!(submit["priority"], "P1");
        assert_eq!(submit["source_task_id"], "task-1");
        assert_eq!(submit["source_project_id"], "list-aq");

        // CHK006: the task is stamped, its dispatched subtask checked, and moved.
        let update = &gateway.calls_to("ticktick_update_task")[0];
        let content = update["params"]["content"].as_str().unwrap();
        assert!(content.contains("[agentd] dispatched as job-abc"));
        let subtasks = update["params"]["subtasks"].as_array().unwrap();
        assert_eq!(subtasks[0]["title"], "dispatched");
        assert_eq!(subtasks[0]["status"], 1);
        let move_call = &gateway.calls_to("ticktick_move_task")[0];
        assert_eq!(move_call["params"]["to_project_id"], "list-wip");
        assert_eq!(move_call["params"]["from_project_id"], "list-aq");
    }

    #[test]
    fn capture_skips_already_stamped_tasks() {
        // CHK008: a task captured on a prior tick (already stamped) is not
        // converted again.
        let inner = json!({
            "tasks": [{
                "id": "task-1",
                "projectId": "list-aq",
                "title": "x",
                "content": "body\n\n[agentd] dispatched as job-old",
                "priority": 0,
                "items": []
            }]
        });
        let gateway = FakeGateway::new();
        gateway.respond(
            "ticktick_get_project",
            json!({ "content": [{ "type": "text", "text": inner.to_string() }] }),
        );
        let config = CaptureConfig {
            enabled: true,
            agent_queue_project_id: Some("list-aq".to_string()),
            ..Default::default()
        };

        let report = run_capture(&gateway, &config, "theorem-agentd").unwrap();
        assert_eq!(report.captured.len(), 0);
        assert_eq!(report.skipped, 1);
        assert!(gateway.calls_to("job_submit").is_empty());
    }

    #[test]
    fn capture_resolves_project_by_name() {
        let gateway = FakeGateway::new();
        gateway.respond(
            "ticktick_list_projects",
            json!({ "content": [{ "type": "text", "text": json!([
                {"id": "list-aq", "name": "Agent Queue"},
                {"id": "list-other", "name": "Other"}
            ]).to_string() }] }),
        );
        gateway.respond(
            "ticktick_get_project",
            json!({ "content": [{ "type": "text", "text": json!({"tasks": []}).to_string() }] }),
        );
        let config = CaptureConfig {
            enabled: true,
            agent_queue_project_name: Some("Agent Queue".to_string()),
            ..Default::default()
        };

        let report = run_capture(&gateway, &config, "theorem-agentd").unwrap();
        assert_eq!(report.project_id, "list-aq");
        // get_project was called with the resolved id.
        let got = &gateway.calls_to("ticktick_get_project")[0];
        assert_eq!(got["params"]["project_id"], "list-aq");
    }
}
