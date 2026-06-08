//! Job board: GraphStore-backed dispatch v2 verbs.
//!
//! Dispatch v2 intentionally removes the old guarded lifecycle. A job is a
//! durable thread with derived state:
//!
//! - pending: `started_at` and `archived_at` are both null
//! - started: `started_at` is set and `archived_at` is null
//! - archived: `archived_at` is set
//!
//! The only infrastructure invariant is the receiver's set-once start write,
//! represented through `job_note` with `start_session_ref`.

use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreResult, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use theorem_harness_core::types::now_string;
use theorem_harness_core::{Job, JobReceipt, JobSubmission};

use crate::event_log::{run_node_id, HarnessRuntimeError, RuntimeResult};

/// Graph label for a dispatch job node.
pub const JOB_LABEL: &str = "Job";
/// Edge from a job to its spec doc node (only when spec_ref is a doc_id).
pub const EDGE_JOB_FOR_SPEC: &str = "JOB_FOR_SPEC";
/// Edge from a job to the run it was launched as.
pub const EDGE_DISPATCHED_AS: &str = "DISPATCHED_AS";

/// Outcome of `job_submit`: the job, plus whether it was newly created.
#[derive(Clone, Debug, Serialize)]
pub struct JobSubmitOutcome {
    pub job: Job,
    pub created: bool,
}

/// Outcome of a mutating verb against an existing job.
#[derive(Clone, Debug, Serialize)]
pub struct JobActionResult {
    pub job: Job,
    pub applied: bool,
    pub message: String,
}

/// Input for `job_note`. Plain notes append a receipt. Receiver notes may also
/// request the one CAS-like start write or clear a failed pre-launch start.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobNoteInput {
    pub actor: String,
    pub text: String,
    #[serde(default)]
    pub refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_session_ref: Option<String>,
    #[serde(default)]
    pub clear_started: bool,
}

/// Stable node id for a job: `harness:job:{job_id}`.
pub fn job_node_id(job_id: &str) -> String {
    format!("harness:job:{job_id}")
}

/// `job_submit`: create a pending job or upsert an existing one by idempotency key.
pub fn job_submit<S: GraphStore>(
    store: &mut S,
    submission: JobSubmission,
    submitted_by: impl Into<String>,
) -> RuntimeResult<JobSubmitOutcome> {
    let submitted_by = submitted_by.into();
    let candidate = Job::from_submission(submission, submitted_by.clone())
        .map_err(HarnessRuntimeError::Deserialization)?;

    if let Some(mut existing) = find_by_idempotency_key(store, &candidate.idempotency_key)? {
        let mut changed = false;
        if existing.priority != candidate.priority {
            existing.priority = candidate.priority;
            changed = true;
        }
        if candidate.spec_ref.is_some() && existing.spec_ref != candidate.spec_ref {
            existing.spec_ref = candidate.spec_ref;
            existing.spec_inline = None;
            changed = true;
        } else if candidate.spec_inline.is_some() && existing.spec_inline != candidate.spec_inline {
            existing.spec_inline = candidate.spec_inline;
            existing.spec_ref = None;
            changed = true;
        }
        if existing.not_before != candidate.not_before {
            existing.not_before = candidate.not_before;
            changed = true;
        }
        if changed {
            persist_job(store, &existing)?;
            maybe_link_spec(store, &existing)?;
        }
        return Ok(JobSubmitOutcome {
            job: existing,
            created: false,
        });
    }

    persist_job(store, &candidate)?;
    maybe_link_spec(store, &candidate)?;
    Ok(JobSubmitOutcome {
        job: candidate,
        created: true,
    })
}

/// `job_list`: the board, ordered by priority then submitted_at.
pub fn job_list<S: GraphStore>(
    store: &S,
    repo: Option<&str>,
    state: Option<&str>,
) -> RuntimeResult<Vec<Job>> {
    let state = state.map(normalize_state_filter).transpose()?;
    let mut jobs: Vec<Job> = list_jobs(store)?
        .into_iter()
        .filter(|job| repo.map(|repo| job.repo == repo).unwrap_or(true))
        .filter(|job| {
            state
                .as_deref()
                .map(|state| job.derived_state() == state)
                .unwrap_or(true)
        })
        .collect();
    sort_jobs(&mut jobs);
    Ok(jobs)
}

/// `job_note`: append a receipt. When `start_session_ref` is present, this is
/// also the receiver's set-once start write.
pub fn job_note<S: GraphStore>(
    store: &mut S,
    job_id: &str,
    input: JobNoteInput,
) -> RuntimeResult<Option<JobActionResult>> {
    let Some(mut job) = load_job(store, job_id)? else {
        return Ok(None);
    };

    if input.start_session_ref.is_some() {
        if job.archived_at.is_some() {
            return Ok(Some(JobActionResult {
                message: "job is archived; start skipped".to_string(),
                applied: false,
                job,
            }));
        }
        if job.started_at.is_some() {
            return Ok(Some(JobActionResult {
                message: "job already started".to_string(),
                applied: false,
                job,
            }));
        }
        job.started_at = Some(now_string());
        job.session_ref = input.start_session_ref.clone();
        job.receipts
            .push(JobReceipt::new(input.actor, input.text, input.refs));
        persist_job(store, &job)?;
        link_dispatched_as(store, &job)?;
        return Ok(Some(JobActionResult {
            message: "job started".to_string(),
            applied: true,
            job,
        }));
    }

    if input.clear_started {
        job.started_at = None;
        job.session_ref = None;
    }
    job.receipts
        .push(JobReceipt::new(input.actor, input.text, input.refs));
    persist_job(store, &job)?;
    Ok(Some(JobActionResult {
        message: if input.clear_started {
            "job noted and start cleared"
        } else {
            "job noted"
        }
        .to_string(),
        applied: true,
        job,
    }))
}

/// `job_archive`: archive the thread with a reason.
pub fn job_archive<S: GraphStore>(
    store: &mut S,
    job_id: &str,
    reason: impl Into<String>,
    actor: impl Into<String>,
) -> RuntimeResult<Option<JobActionResult>> {
    let Some(mut job) = load_job(store, job_id)? else {
        return Ok(None);
    };
    let reason = reason.into();
    let actor = actor.into();
    let changed = job.archived_at.is_none() || job.archived_reason.as_deref() != Some(&reason);
    if job.archived_at.is_none() {
        job.archived_at = Some(now_string());
    }
    job.archived_reason = Some(reason.clone());
    job.receipts.push(JobReceipt::new(
        actor,
        format!("archived: {reason}"),
        Vec::new(),
    ));
    persist_job(store, &job)?;
    Ok(Some(JobActionResult {
        message: "job archived".to_string(),
        applied: changed,
        job,
    }))
}

/// Load one job by id.
pub fn load_job<S: GraphStore>(store: &S, job_id: &str) -> RuntimeResult<Option<Job>> {
    store
        .get_node(&job_node_id(job_id))
        .map(|node| node_to_job(&node))
        .transpose()
}

fn list_jobs<S: GraphStore>(store: &S) -> RuntimeResult<Vec<Job>> {
    store
        .query_nodes(NodeQuery::label(JOB_LABEL))
        .into_iter()
        .map(|node| node_to_job(&node))
        .collect()
}

fn find_by_idempotency_key<S: GraphStore>(store: &S, key: &str) -> RuntimeResult<Option<Job>> {
    let mut hits = store
        .query_nodes(
            NodeQuery::label(JOB_LABEL)
                .with_property("idempotency_key", Value::String(key.to_string())),
        )
        .into_iter()
        .map(|node| node_to_job(&node))
        .collect::<RuntimeResult<Vec<_>>>()?;
    sort_jobs(&mut hits);
    Ok(hits.into_iter().next())
}

fn node_to_job(node: &NodeRecord) -> RuntimeResult<Job> {
    if node.properties.get("status").is_some() {
        return legacy_node_to_job(&node.properties)
            .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()));
    }
    serde_json::from_value::<Job>(node.properties.clone())
        .or_else(|_| legacy_node_to_job(&node.properties))
        .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))
}

fn legacy_node_to_job(properties: &Value) -> Result<Job, serde_json::Error> {
    let status = properties
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("Queued");
    let claimed_at = text_property(properties, "claimed_at");
    let closed_at = text_property(properties, "closed_at");
    let migration_time = now_string();
    let mut receipts = Vec::new();
    if let Some(claimed_by) = text_property(properties, "claimed_by") {
        receipts.push(JobReceipt::new(
            "migration",
            format!("legacy claimed_by: {claimed_by}"),
            Vec::new(),
        ));
    }
    if let Some(notes) = text_property(properties, "notes") {
        receipts.push(JobReceipt::new("migration", notes, Vec::new()));
    }
    let mut job = Job {
        job_id: text_property(properties, "job_id").unwrap_or_else(|| "job-migrated".to_string()),
        title: text_property(properties, "title").unwrap_or_else(|| "Untitled job".to_string()),
        spec_ref: text_property(properties, "spec_ref"),
        spec_inline: text_property(properties, "spec_inline"),
        repo: text_property(properties, "repo").unwrap_or_else(|| "unknown/repo".to_string()),
        priority: serde_json::from_value(
            properties.get("priority").cloned().unwrap_or(json!("P2")),
        )?,
        target_head: serde_json::from_value(
            properties
                .get("target_head")
                .cloned()
                .unwrap_or(json!("either")),
        )?,
        not_before: text_property(properties, "not_before"),
        submitted_by: text_property(properties, "submitted_by")
            .unwrap_or_else(|| "unknown".to_string()),
        submitted_at: text_property(properties, "submitted_at").unwrap_or_else(now_string),
        started_at: None,
        session_ref: text_property(properties, "session_ref"),
        archived_at: None,
        archived_reason: None,
        idempotency_key: text_property(properties, "idempotency_key").unwrap_or_else(|| {
            theorem_harness_core::idempotency_key_for(
                text_property(properties, "spec_ref")
                    .as_deref()
                    .unwrap_or("legacy"),
                text_property(properties, "title")
                    .as_deref()
                    .unwrap_or("legacy"),
            )
        }),
        receipts,
    };

    match status {
        "Queued" | "Open" => {}
        "Claimed" | "Running" | "PrOpen" | "Verifying" => {
            job.started_at = claimed_at.or(Some(migration_time));
        }
        "Done" => {
            job.archived_at = closed_at.or(Some(migration_time));
            job.archived_reason = Some("done".to_string());
        }
        "Failed" | "Cancelled" | "Dropped" => {
            job.archived_at = closed_at.or(Some(migration_time));
            job.archived_reason = Some(status.to_lowercase());
        }
        _ => {}
    }
    Ok(job)
}

fn text_property(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
}

fn job_node(job: &Job) -> RuntimeResult<NodeRecord> {
    let properties = serde_json::to_value(job)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        job_node_id(&job.job_id),
        [JOB_LABEL],
        properties,
    ))
}

fn persist_job<S: GraphStore>(store: &mut S, job: &Job) -> RuntimeResult<()> {
    upsert_node_if_changed(store, job_node(job)?)?;
    Ok(())
}

fn maybe_link_spec<S: GraphStore>(store: &mut S, job: &Job) -> RuntimeResult<()> {
    let Some(spec_ref) = job.spec_ref.as_deref() else {
        return Ok(());
    };
    if !spec_ref_is_doc_id(spec_ref) || store.get_node(spec_ref).is_none() {
        return Ok(());
    }
    upsert_edge_if_changed(
        store,
        EdgeRecord::new(
            format!("harness:edge:job-for-spec:{}", job.job_id),
            job_node_id(&job.job_id),
            EDGE_JOB_FOR_SPEC,
            spec_ref.to_string(),
            json!({ "job_id": job.job_id, "spec_ref": spec_ref }),
        ),
    )
}

fn link_dispatched_as<S: GraphStore>(store: &mut S, job: &Job) -> RuntimeResult<()> {
    let Some(run_id) = job.session_ref.as_deref() else {
        return Ok(());
    };
    if store.get_node(&run_node_id(run_id)).is_none() {
        return Ok(());
    }
    upsert_edge_if_changed(
        store,
        EdgeRecord::new(
            format!("harness:edge:dispatched-as:{}", job.job_id),
            job_node_id(&job.job_id),
            EDGE_DISPATCHED_AS,
            run_node_id(run_id),
            json!({ "job_id": job.job_id, "run_id": run_id }),
        ),
    )
}

fn spec_ref_is_doc_id(spec_ref: &str) -> bool {
    !spec_ref.contains('/') && !spec_ref.contains('.')
}

fn sort_jobs(jobs: &mut [Job]) {
    jobs.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.submitted_at.cmp(&b.submitted_at))
            .then_with(|| a.job_id.cmp(&b.job_id))
    });
}

fn normalize_state_filter(state: &str) -> RuntimeResult<&'static str> {
    match state.to_ascii_lowercase().as_str() {
        "pending" => Ok("pending"),
        "started" => Ok("started"),
        "archived" => Ok("archived"),
        other => Err(HarnessRuntimeError::Deserialization(format!(
            "invalid job state '{other}'; expected pending, started, or archived"
        ))),
    }
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

fn upsert_edge_if_changed<S: GraphStore>(store: &mut S, edge: EdgeRecord) -> RuntimeResult<()> {
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
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use theorem_harness_core::{Priority, TargetHead, LANE_CLAUDE};

    fn submission(title: &str) -> JobSubmission {
        JobSubmission {
            title: title.to_string(),
            spec_ref: Some(format!("docs/plans/{title}/HANDOFF.md")),
            spec_inline: None,
            repo: "Travis-Gilbert/theorem".to_string(),
            priority: None,
            target_head: None,
            not_before: None,
            idempotency_key: None,
        }
    }

    fn submission_priority(title: &str, priority: Priority) -> JobSubmission {
        let mut s = submission(title);
        s.priority = Some(priority);
        s
    }

    #[test]
    fn submit_then_job_list_orders_by_priority_and_filters_state() {
        let mut store = InMemoryGraphStore::new();
        job_submit(
            &mut store,
            submission_priority("p1-job", Priority::P1),
            "claude.ai",
        )
        .unwrap();
        job_submit(
            &mut store,
            submission_priority("p0-job", Priority::P0),
            "claude.ai",
        )
        .unwrap();

        let jobs = job_list(&store, None, None).unwrap();
        assert_eq!(jobs.len(), 2);
        assert_eq!(jobs[0].priority, Priority::P0);
        assert_eq!(jobs[0].derived_state(), "pending");
        assert_eq!(job_list(&store, None, Some("pending")).unwrap().len(), 2);
        assert!(job_list(&store, None, Some("started")).unwrap().is_empty());
        assert!(job_list(&store, Some("other/repo"), None)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn duplicate_idempotency_key_upserts_allowed_fields() {
        let mut store = InMemoryGraphStore::new();
        let first = job_submit(
            &mut store,
            submission_priority("dia", Priority::P2),
            "claude.ai",
        )
        .unwrap();
        assert!(first.created);

        let mut second_submission = submission_priority("dia", Priority::P0);
        second_submission.not_before = Some("2099-01-01T00:00:00Z".to_string());
        let second = job_submit(&mut store, second_submission, "codex").unwrap();
        assert!(!second.created);
        assert_eq!(second.job.job_id, first.job.job_id);
        assert_eq!(second.job.priority, Priority::P0);
        assert_eq!(
            second.job.not_before.as_deref(),
            Some("2099-01-01T00:00:00Z")
        );
        assert_eq!(job_list(&store, None, None).unwrap().len(), 1);
    }

    #[test]
    fn receipts_append_from_multiple_actors() {
        let mut store = InMemoryGraphStore::new();
        let submitted = job_submit(&mut store, submission("thread"), "claude.ai").unwrap();
        let job_id = submitted.job.job_id.clone();
        job_note(
            &mut store,
            &job_id,
            JobNoteInput {
                actor: "codex".to_string(),
                text: "commit abc".to_string(),
                refs: vec!["abc".to_string()],
                start_session_ref: None,
                clear_started: false,
            },
        )
        .unwrap();
        job_note(
            &mut store,
            &job_id,
            JobNoteInput {
                actor: "claude".to_string(),
                text: "reviewed".to_string(),
                refs: Vec::new(),
                start_session_ref: None,
                clear_started: false,
            },
        )
        .unwrap();
        let job = load_job(&store, &job_id).unwrap().unwrap();
        assert_eq!(job.receipts.len(), 2);
        assert_eq!(job.receipts[0].actor, "codex");
        assert_eq!(job.receipts[1].actor, "claude");
    }

    #[test]
    fn set_once_start_race_has_one_winner() {
        let mut store = InMemoryGraphStore::new();
        let submitted = job_submit(&mut store, submission("race"), "claude.ai").unwrap();
        let job_id = submitted.job.job_id.clone();

        let won = job_note(
            &mut store,
            &job_id,
            JobNoteInput {
                actor: "receiver-a".to_string(),
                text: "starting session".to_string(),
                refs: Vec::new(),
                start_session_ref: Some("session-a".to_string()),
                clear_started: false,
            },
        )
        .unwrap()
        .unwrap();
        assert!(won.applied);
        assert_eq!(won.job.derived_state(), "started");

        let lost = job_note(
            &mut store,
            &job_id,
            JobNoteInput {
                actor: "receiver-b".to_string(),
                text: "starting session".to_string(),
                refs: Vec::new(),
                start_session_ref: Some("session-b".to_string()),
                clear_started: false,
            },
        )
        .unwrap()
        .unwrap();
        assert!(!lost.applied);
        assert_eq!(lost.job.session_ref.as_deref(), Some("session-a"));
    }

    #[test]
    fn archive_sets_reason_and_derived_state() {
        let mut store = InMemoryGraphStore::new();
        let submitted = job_submit(&mut store, submission("done"), "claude.ai").unwrap();
        let archived = job_archive(&mut store, &submitted.job.job_id, "done", "codex")
            .unwrap()
            .unwrap();
        assert!(archived.applied);
        assert_eq!(archived.job.derived_state(), "archived");
        assert_eq!(archived.job.archived_reason.as_deref(), Some("done"));
        assert_eq!(archived.job.receipts.last().unwrap().text, "archived: done");
    }

    #[test]
    fn legacy_statuses_migrate_to_derived_state() {
        let mut store = InMemoryGraphStore::new();
        let statuses = [
            ("Queued", "pending", None),
            ("Open", "pending", None),
            ("Claimed", "started", None),
            ("Running", "started", None),
            ("PrOpen", "started", None),
            ("Verifying", "started", None),
            ("Done", "archived", Some("done")),
            ("Failed", "archived", Some("failed")),
            ("Cancelled", "archived", Some("cancelled")),
            ("Dropped", "archived", Some("dropped")),
        ];
        for (idx, (status, state, reason)) in statuses.iter().enumerate() {
            let job_id = format!("job-legacy-{idx}");
            store
                .upsert_node(NodeRecord::new(
                    job_node_id(&job_id),
                    [JOB_LABEL],
                    json!({
                        "job_id": job_id,
                        "title": status,
                        "spec_ref": "docs/plans/x/HANDOFF.md",
                        "repo": "Travis-Gilbert/theorem",
                        "priority": "P2",
                        "target_head": "Either",
                        "status": status,
                        "submitted_by": "legacy",
                        "submitted_at": "2026-06-08T00:00:00Z",
                        "claimed_by": "receiver-old",
                        "claimed_at": "2026-06-08T00:01:00Z",
                        "closed_at": "2026-06-08T00:02:00Z",
                        "idempotency_key": format!("legacy-{idx}")
                    }),
                ))
                .unwrap();
            let migrated = load_job(&store, &job_id).unwrap().unwrap();
            assert_eq!(migrated.derived_state(), *state);
            assert_eq!(migrated.archived_reason.as_deref(), *reason);
            if *status != "Queued" && *status != "Open" {
                assert!(!migrated.receipts.is_empty());
            }
        }
    }

    #[test]
    fn target_head_is_only_a_receiver_hint() {
        let mut job = Job::from_submission(submission("hint"), "x").unwrap();
        job.target_head = TargetHead::Codex;
        assert!(job.is_pending());
        assert!(!job.target_head.matches_lanes(&[LANE_CLAUDE.to_string()]));
    }

    #[test]
    fn job_survives_redcore_reopen() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-job-board-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let job_id;
        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let submitted = job_submit(&mut store, submission("Dia"), "claude.ai").unwrap();
            job_id = submitted.job.job_id.clone();
            job_note(
                &mut store,
                &job_id,
                JobNoteInput {
                    actor: "receiver".to_string(),
                    text: "starting".to_string(),
                    refs: Vec::new(),
                    start_session_ref: Some("session-1".to_string()),
                    clear_started: false,
                },
            )
            .unwrap();
        }
        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let reloaded = load_job(&store, &job_id).unwrap().unwrap();
            assert_eq!(reloaded.derived_state(), "started");
            assert_eq!(reloaded.session_ref.as_deref(), Some("session-1"));
        }
        let _ = fs::remove_dir_all(data_dir);
    }
}
