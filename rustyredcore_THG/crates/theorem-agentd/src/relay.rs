//! Milestone relay: a run becomes visible and steerable from the phone.
//!
//! This closes the loop's return path (CHK009-015). For each job that was captured
//! from a TickTick task, the daemon relays the sparse milestones a run produces -
//! started, PR-opened, merged, failed - back onto the originating task. The split
//! the plan calls for is enforced here:
//!
//! - mechanical (this module): which transitions have happened, dedup so a
//!   milestone is relayed once, delivery to the TickTick MCP, and completion on
//!   merge. None of this is left to the model, because a status that reads more
//!   complete than the run is a bug.
//! - model-written: the milestone *line* itself. The summary prose is composed by
//!   the model; the loop still guarantees the load-bearing fact (a PR-opened line
//!   carries the PR URL).
//!
//! There is no per-`job_note` mirror: relays fire at transitions only, deduped by
//! `relay:<milestone>` marker receipts written back onto the job thread (CHK011).

use serde::Deserialize;
use serde_json::{json, Value};

use crate::capture::{ticktick_json, HARNESS, TICKTICK};
use crate::mcp::ToolGateway;
use crate::model::ModelClient;
use crate::AgentdResult;

/// Prefix of a relay marker receipt. A receipt whose text starts with this is the
/// daemon's own dedup bookkeeping, not a run signal, and is skipped by detection.
pub const RELAY_MARKER_PREFIX: &str = "relay:";

/// A milestone a run can reach. Ordered started -> pr_opened -> (merged | failed).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Milestone {
    Started,
    PrOpened { url: String },
    Merged,
    Failed { reason: String },
}

impl Milestone {
    /// The dedup key written as `relay:<kind>`.
    pub fn kind(&self) -> &'static str {
        match self {
            Milestone::Started => "started",
            Milestone::PrOpened { .. } => "pr_opened",
            Milestone::Merged => "merged",
            Milestone::Failed { .. } => "failed",
        }
    }
}

/// Just the job fields the relay reads. Parsed from the harness `job_list`
/// payload, keeping the daemon decoupled from the harness core Job type (it talks
/// to the harness over JSON, which is the boundary).
#[derive(Clone, Debug, Deserialize)]
struct JobView {
    job_id: String,
    #[serde(default)]
    source_task_id: Option<String>,
    #[serde(default)]
    source_project_id: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    archived_at: Option<String>,
    #[serde(default)]
    archived_reason: Option<String>,
    #[serde(default)]
    receipts: Vec<ReceiptView>,
}

#[derive(Clone, Debug, Deserialize)]
struct ReceiptView {
    #[serde(default)]
    text: String,
}

/// What a relay sweep did, for logging and tests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RelayReport {
    /// (task_id, milestone_kind) pairs relayed this sweep.
    pub relayed: Vec<(String, String)>,
    /// task_ids completed this sweep (merge reached).
    pub completed: Vec<String>,
}

/// Run one relay sweep over every job that carries a source task.
pub fn run_relays(
    gateway: &dyn ToolGateway,
    model: &ModelClient,
    actor: &str,
) -> AgentdResult<RelayReport> {
    let listed = gateway.call_server(HARNESS, "job_list", json!({}))?;
    let jobs = parse_jobs(&listed);
    let mut report = RelayReport::default();
    for job in jobs {
        let (Some(task_id), Some(project_id)) =
            (job.source_task_id.clone(), job.source_project_id.clone())
        else {
            // Not a captured job, or pre-correspondence: nothing to relay to.
            continue;
        };
        relay_one_job(
            gateway,
            model,
            actor,
            &job,
            &task_id,
            &project_id,
            &mut report,
        )?;
    }
    Ok(report)
}

#[allow(clippy::too_many_arguments)]
fn relay_one_job(
    gateway: &dyn ToolGateway,
    model: &ModelClient,
    actor: &str,
    job: &JobView,
    task_id: &str,
    project_id: &str,
    report: &mut RelayReport,
) -> AgentdResult<()> {
    let agenda = relay_agenda(job);
    if agenda.is_empty() {
        return Ok(());
    }

    // Read the task once: its current content (to append to) and whether the
    // operator already completed it by hand.
    let task = gateway.call_server(
        TICKTICK,
        "ticktick_get_task",
        json!({ "params": { "project_id": project_id, "task_id": task_id, "response_format": "json" } }),
    )?;
    let task = ticktick_json(&task);

    // CHK014: a manual completion always wins. Stop relaying, record markers so we
    // never re-check, and leave the operator's task alone.
    if task_is_completed(&task) {
        for milestone in &agenda {
            mark_relayed(gateway, actor, &job.job_id, milestone)?;
        }
        return Ok(());
    }

    let existing_content = task
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let mut lines = Vec::new();
    let mut complete = false;
    for milestone in &agenda {
        lines.push(compose_milestone_line(model, &job.job_id, milestone));
        if matches!(milestone, Milestone::Merged) {
            complete = true;
        }
    }

    // One content update carries every new status line (no per-note mirror).
    let content = append_status_lines(&existing_content, &lines);
    gateway.call_server(
        TICKTICK,
        "ticktick_update_task",
        json!({ "params": { "task_id": task_id, "project_id": project_id, "content": content } }),
    )?;

    // CHK013: complete the task only when the job records the merge.
    if complete {
        gateway.call_server(
            TICKTICK,
            "ticktick_complete_task",
            json!({ "params": { "project_id": project_id, "task_id": task_id } }),
        )?;
        report.completed.push(task_id.to_string());
    }

    // Mark each milestone relayed so it never fires again (CHK011/CHK012).
    for milestone in &agenda {
        mark_relayed(gateway, actor, &job.job_id, milestone)?;
        report
            .relayed
            .push((task_id.to_string(), milestone.kind().to_string()));
    }
    Ok(())
}

/// The un-relayed milestones a job has reached, in relay order.
fn relay_agenda(job: &JobView) -> Vec<Milestone> {
    let relayed = relayed_kinds(job);
    reached_milestones(job)
        .into_iter()
        .filter(|milestone| !relayed.contains(milestone.kind()))
        .collect()
}

/// Every milestone a job has reached, derived from its durable state and receipts.
fn reached_milestones(job: &JobView) -> Vec<Milestone> {
    let mut out = Vec::new();
    if job.started_at.is_some() {
        out.push(Milestone::Started);
    }
    if let Some(url) = first_pr_url(job) {
        out.push(Milestone::PrOpened { url });
    }
    // Terminal: a recorded merge wins over a failure signal if both are present.
    if is_merged(job) {
        out.push(Milestone::Merged);
    } else if let Some(reason) = failure_reason(job) {
        out.push(Milestone::Failed { reason });
    }
    out
}

/// The set of milestone kinds already relayed, from `relay:<kind>` markers.
fn relayed_kinds(job: &JobView) -> std::collections::BTreeSet<String> {
    job.receipts
        .iter()
        .filter_map(|receipt| {
            receipt
                .text
                .trim()
                .strip_prefix(RELAY_MARKER_PREFIX)
                .map(|kind| kind.trim().to_string())
        })
        .collect()
}

/// Non-marker receipt texts (run signals, not the daemon's own bookkeeping).
fn signal_texts(job: &JobView) -> impl Iterator<Item = &str> {
    job.receipts
        .iter()
        .map(|receipt| receipt.text.as_str())
        .filter(|text| !text.trim_start().starts_with(RELAY_MARKER_PREFIX))
}

/// The first GitHub PR URL in any run signal receipt.
fn first_pr_url(job: &JobView) -> Option<String> {
    signal_texts(job).find_map(extract_pr_url)
}

/// Pull an `https://github.com/<owner>/<repo>/pull/<n>` URL out of free text.
pub fn extract_pr_url(text: &str) -> Option<String> {
    text.split(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '(' || c == ')')
        .map(|token| token.trim_end_matches(|c: char| matches!(c, '.' | ',' | ';' | '>' | ']')))
        .find(|token| {
            token.contains("github.com")
                && token.contains("/pull/")
                && token
                    .rsplit("/pull/")
                    .next()
                    .map(|tail| tail.chars().next().is_some_and(|c| c.is_ascii_digit()))
                    .unwrap_or(false)
        })
        .map(str::to_string)
}

/// True when the job records a merge: an archive reason or a receipt saying so.
fn is_merged(job: &JobView) -> bool {
    if let Some(reason) = job.archived_reason.as_deref() {
        let reason = reason.to_ascii_lowercase();
        if reason.contains("merge") || reason.contains("done") || reason.contains("complete") {
            return true;
        }
    }
    if job.archived_at.is_some() && job.archived_reason.is_none() {
        // Archived with no reason is treated as a clean completion.
        return true;
    }
    signal_texts(job).any(|text| text.to_ascii_lowercase().contains("merged"))
}

/// A failure reason, if the job records one (archive reason or non-zero exit).
fn failure_reason(job: &JobView) -> Option<String> {
    if let Some(reason) = job.archived_reason.as_deref() {
        let lower = reason.to_ascii_lowercase();
        if lower.contains("fail")
            || lower.contains("cancel")
            || lower.contains("drop")
            || lower.contains("abort")
            || lower.contains("error")
        {
            return Some(reason.to_string());
        }
    }
    if signal_texts(job).any(receipt_is_nonzero_exit) {
        return Some("non-zero exit".to_string());
    }
    None
}

/// True when a receiver exit receipt carries a non-zero, non-null exit code.
fn receipt_is_nonzero_exit(text: &str) -> bool {
    let Some(rest) = text.split("\"exit_code\":").nth(1) else {
        return false;
    };
    let token: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    matches!(token.parse::<i64>(), Ok(code) if code != 0)
}

/// Append model-composed status lines under a task's existing content.
fn append_status_lines(content: &str, lines: &[String]) -> String {
    let mut out = content.trim_end().to_string();
    for line in lines {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(line);
    }
    out
}

/// Compose one milestone line: model prose with a deterministic fallback, and the
/// load-bearing fact (PR URL) guaranteed present regardless of what the model wrote.
fn compose_milestone_line(model: &ModelClient, job_id: &str, milestone: &Milestone) -> String {
    let (instruction, fallback) = match milestone {
        Milestone::Started => (
            format!("Job {job_id} started running on a head. Write the status line."),
            "Started: a head is now working this task.".to_string(),
        ),
        Milestone::PrOpened { url } => (
            format!("Job {job_id} opened a pull request at {url}. Write the status line and include the URL."),
            format!("PR opened: {url}"),
        ),
        Milestone::Merged => (
            format!("Job {job_id} merged its pull request. Write the status line."),
            "Merged: the pull request landed.".to_string(),
        ),
        Milestone::Failed { reason } => (
            format!("Job {job_id} failed ({reason}). Write the status line."),
            format!("Failed: {reason}."),
        ),
    };
    let line = model.compose_line(&instruction, &fallback);
    // CHK010/CHK012: the PR-opened line must carry the PR link, whatever the model
    // chose to write.
    if let Milestone::PrOpened { url } = milestone {
        if !line.contains(url) {
            return format!("{line} {url}");
        }
    }
    line
}

/// Write a `relay:<kind>` marker receipt onto the job thread (dedup bookkeeping).
fn mark_relayed(
    gateway: &dyn ToolGateway,
    actor: &str,
    job_id: &str,
    milestone: &Milestone,
) -> AgentdResult<()> {
    gateway.call_server(
        HARNESS,
        "job_note",
        json!({
            "job_id": job_id,
            "actor": actor,
            "text": format!("{RELAY_MARKER_PREFIX}{}", milestone.kind()),
        }),
    )?;
    Ok(())
}

/// A TickTick task the operator already completed: status is set or a completed
/// time is present.
fn task_is_completed(task: &Value) -> bool {
    if task.get("completedTime").is_some() {
        return true;
    }
    task.get("status")
        .and_then(Value::as_i64)
        .map(|status| status != 0)
        .unwrap_or(false)
}

/// Parse the harness `job_list` payload (`{count, jobs:[...]}`) into JobViews.
fn parse_jobs(listed: &Value) -> Vec<JobView> {
    let jobs = listed
        .get("jobs")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| {
            listed
                .get("result")
                .and_then(|result| result.get("jobs"))
                .and_then(Value::as_array)
                .cloned()
        })
        .unwrap_or_default();
    jobs.into_iter()
        .filter_map(|job| serde_json::from_value::<JobView>(job).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RuleModelClient;
    use std::cell::RefCell;

    fn job(receipts: &[&str], started: bool, archived_reason: Option<&str>) -> JobView {
        JobView {
            job_id: "job-1".to_string(),
            source_task_id: Some("task-1".to_string()),
            source_project_id: Some("list-aq".to_string()),
            started_at: started.then(|| "1.0Z".to_string()),
            archived_at: archived_reason.map(|_| "2.0Z".to_string()),
            archived_reason: archived_reason.map(str::to_string),
            receipts: receipts
                .iter()
                .map(|text| ReceiptView {
                    text: text.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn extracts_pr_url_from_noisy_text() {
        assert_eq!(
            extract_pr_url("opened https://github.com/Travis-Gilbert/theorem/pull/42 for review."),
            Some("https://github.com/Travis-Gilbert/theorem/pull/42".to_string())
        );
        assert_eq!(extract_pr_url("no url here"), None);
        // A /pull/ path without a trailing number is not a PR URL.
        assert_eq!(extract_pr_url("https://github.com/x/y/pulls"), None);
    }

    #[test]
    fn reached_milestones_orders_started_pr_merged() {
        let j = job(
            &["opened https://github.com/o/r/pull/7", "PR #7 merged"],
            true,
            None,
        );
        let reached = reached_milestones(&j);
        assert_eq!(reached[0], Milestone::Started);
        assert_eq!(
            reached[1],
            Milestone::PrOpened {
                url: "https://github.com/o/r/pull/7".to_string()
            }
        );
        assert_eq!(reached[2], Milestone::Merged);
    }

    #[test]
    fn nonzero_exit_is_a_failure() {
        let j = job(
            &["child exited: {\"source\":\"receiver_exit\",\"exit_code\":1}"],
            true,
            None,
        );
        assert_eq!(
            reached_milestones(&j).last(),
            Some(&Milestone::Failed {
                reason: "non-zero exit".to_string()
            })
        );
        // A clean exit is not a failure.
        let ok = job(
            &["child exited: {\"source\":\"receiver_exit\",\"exit_code\":0}"],
            true,
            None,
        );
        assert!(!reached_milestones(&ok)
            .iter()
            .any(|m| matches!(m, Milestone::Failed { .. })));
    }

    #[test]
    fn relay_agenda_skips_already_relayed() {
        let mut j = job(&["opened https://github.com/o/r/pull/7"], true, None);
        j.receipts.push(ReceiptView {
            text: "relay:started".to_string(),
        });
        let agenda = relay_agenda(&j);
        // started already relayed; only pr_opened remains.
        assert_eq!(agenda.len(), 1);
        assert!(matches!(agenda[0], Milestone::PrOpened { .. }));
    }

    #[test]
    fn relay_markers_are_not_treated_as_signals() {
        // A relay:pr_opened marker that echoes a URL must not itself be detected
        // as the PR-opened signal.
        let j = JobView {
            job_id: "job-1".to_string(),
            source_task_id: Some("t".to_string()),
            source_project_id: Some("p".to_string()),
            started_at: Some("1Z".to_string()),
            archived_at: None,
            archived_reason: None,
            receipts: vec![ReceiptView {
                text: "relay:pr_opened https://github.com/o/r/pull/7".to_string(),
            }],
        };
        assert_eq!(first_pr_url(&j), None);
    }

    // A recording fake gateway driving the full sweep deterministically.
    struct FakeGateway {
        calls: RefCell<Vec<(String, String, Value)>>,
        job_list: Value,
        task: Value,
    }

    impl ToolGateway for FakeGateway {
        fn call_server(&self, server: &str, name: &str, arguments: Value) -> AgentdResult<Value> {
            self.calls
                .borrow_mut()
                .push((server.to_string(), name.to_string(), arguments));
            let value = match name {
                "job_list" => self.job_list.clone(),
                "ticktick_get_task" => self.task.clone(),
                _ => json!({}),
            };
            Ok(value)
        }
    }

    impl FakeGateway {
        fn calls_to(&self, name: &str) -> Vec<Value> {
            self.calls
                .borrow()
                .iter()
                .filter(|(_, n, _)| n == name)
                .map(|(_, _, a)| a.clone())
                .collect()
        }
    }

    #[test]
    fn full_sweep_relays_started_pr_merged_and_completes() {
        // A job that started, opened a PR, and merged, captured from task-1.
        let job_list = json!({
            "jobs": [{
                "job_id": "job-1",
                "source_task_id": "task-1",
                "source_project_id": "list-aq",
                "started_at": "1Z",
                "archived_at": "3Z",
                "archived_reason": "done",
                "receipts": [
                    { "actor": "head", "at": "2Z", "text": "opened https://github.com/o/r/pull/7" }
                ]
            }]
        });
        let task = json!({ "content": [{ "type": "text", "text": json!({"id":"task-1","status":0,"content":"original"}).to_string() }] });
        let gateway = FakeGateway {
            calls: RefCell::new(Vec::new()),
            job_list,
            task,
        };
        let model = ModelClient::Rule(RuleModelClient {
            default_room_id: "r".to_string(),
            actor: "theorem-agentd".to_string(),
        });

        let report = run_relays(&gateway, &model, "theorem-agentd").unwrap();

        // CHK012: exactly started, pr_opened, merged relayed; CHK013: completed.
        let kinds: Vec<&str> = report.relayed.iter().map(|(_, k)| k.as_str()).collect();
        assert_eq!(kinds, vec!["started", "pr_opened", "merged"]);
        assert_eq!(report.completed, vec!["task-1".to_string()]);

        // The single content update carries all three lines, and the PR line has
        // the link (CHK010).
        let update = &gateway.calls_to("ticktick_update_task")[0];
        let content = update["params"]["content"].as_str().unwrap();
        assert!(content.contains("original"));
        assert!(content.contains("https://github.com/o/r/pull/7"));
        assert!(content.matches('\n').count() >= 3);

        // complete_task fired, and three relay markers were written.
        assert_eq!(gateway.calls_to("ticktick_complete_task").len(), 1);
        let markers: Vec<String> = gateway
            .calls_to("job_note")
            .iter()
            .map(|c| c["text"].as_str().unwrap_or("").to_string())
            .collect();
        assert_eq!(
            markers,
            vec!["relay:started", "relay:pr_opened", "relay:merged"]
        );
    }

    #[test]
    fn manual_completion_wins() {
        // CHK014/CHK015: the operator completed the task by hand (status != 0).
        // The sweep records markers but never re-posts or re-completes.
        let job_list = json!({
            "jobs": [{
                "job_id": "job-1",
                "source_task_id": "task-1",
                "source_project_id": "list-aq",
                "started_at": "1Z",
                "receipts": []
            }]
        });
        let task = json!({ "content": [{ "type": "text", "text": json!({"id":"task-1","status":2,"completedTime":"9Z"}).to_string() }] });
        let gateway = FakeGateway {
            calls: RefCell::new(Vec::new()),
            job_list,
            task,
        };
        let model = ModelClient::Rule(RuleModelClient {
            default_room_id: "r".to_string(),
            actor: "theorem-agentd".to_string(),
        });

        let report = run_relays(&gateway, &model, "theorem-agentd").unwrap();
        assert!(report.completed.is_empty());
        assert!(gateway.calls_to("ticktick_update_task").is_empty());
        assert!(gateway.calls_to("ticktick_complete_task").is_empty());
        // The started milestone is still marked so we stop checking.
        assert_eq!(gateway.calls_to("job_note").len(), 1);
    }

    #[test]
    fn jobs_without_source_task_are_ignored() {
        let job_list = json!({
            "jobs": [{ "job_id": "job-x", "started_at": "1Z", "receipts": [] }]
        });
        let gateway = FakeGateway {
            calls: RefCell::new(Vec::new()),
            job_list,
            task: json!({}),
        };
        let model = ModelClient::Rule(RuleModelClient {
            default_room_id: "r".to_string(),
            actor: "a".to_string(),
        });
        let report = run_relays(&gateway, &model, "a").unwrap();
        assert!(report.relayed.is_empty());
        assert!(gateway.calls_to("ticktick_get_task").is_empty());
    }
}
