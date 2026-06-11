//! D1: async ingest jobs (submit plus stream).
//!
//! `submit` returns a job id immediately; one background worker per runtime
//! clones, walks, and parses with NO store lock and takes the lock only for
//! the prior-generation snapshot (reindex) and the final `commit_batch`.
//! Watchers replay the ordered event log (`clone_done`, `walk_done`,
//! `parse_progress`, `commit_done`, then `finished` or `failed`); pollers
//! read a status snapshot.
//!
//! Jobs are in-memory: they do not survive a process restart, and the
//! registry retains only the most recent jobs. The worker thread holds a
//! `Weak` store reference so dropping the runtime lets it exit instead of
//! pinning the store open forever.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use rustyred_thg_core::{
    now_ms, stable_hash, GraphMutation, GraphMutationBatch, NodeQuery, NodeRecord,
    RedCoreGraphStore,
};
use serde_json::{json, Value};

use crate::{
    commit_prepared_ingest, default_parse_budget_ms, load_file_texts, normalize_tenant,
    prepare_codebase_ingest_resolved, resolve_ingest_config, snapshot_for_operation,
    stage_repo_for_ingest, CodeIndexError, IngestCodebaseInput, IngestCodebaseOutput,
    IngestPipelineOptions, RepoFetchCaps, SOURCE,
};

const MAX_RETAINED_JOBS: usize = 64;
const WORKER_IDLE_POLL: Duration = Duration::from_secs(2);
/// Label for the durable ingest-job mirror node.
pub const CODE_INGEST_JOB_LABEL: &str = "CodeIngestJob";
/// Cap on durably-persisted TERMINAL jobs; older ones are TTL-expired on submit
/// so the mirror does not grow without bound. Queued/running jobs are never
/// pruned.
const MAX_PERSISTED_TERMINAL_JOBS: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IngestJobState {
    Queued,
    Running,
    Finished,
    Failed,
    BudgetExceeded,
}

impl IngestJobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IngestJobState::Queued => "queued",
            IngestJobState::Running => "running",
            IngestJobState::Finished => "finished",
            IngestJobState::Failed => "failed",
            IngestJobState::BudgetExceeded => "budget_exceeded",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            IngestJobState::Finished | IngestJobState::Failed | IngestJobState::BudgetExceeded
        )
    }

    fn from_str(raw: &str) -> IngestJobState {
        match raw {
            "running" => IngestJobState::Running,
            "finished" => IngestJobState::Finished,
            "failed" => IngestJobState::Failed,
            "budget_exceeded" => IngestJobState::BudgetExceeded,
            _ => IngestJobState::Queued,
        }
    }
}

// The `Finished` variant intentionally carries the full ingest output so the
// terminal stream event and the durable mirror need no second lookup; the
// milestone event list is short (one per stage), so the size spread is benign.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum IngestJobEventKind {
    CloneDone { ms: u64 },
    WalkDone { files_found: u64 },
    ParseProgress { done: u64, total: u64 },
    CommitDone { graph_version: u64 },
    Finished { output: IngestCodebaseOutput },
    Failed { code: String, message: String },
}

impl IngestJobEventKind {
    pub fn label(&self) -> &'static str {
        match self {
            IngestJobEventKind::CloneDone { .. } => "clone_done",
            IngestJobEventKind::WalkDone { .. } => "walk_done",
            IngestJobEventKind::ParseProgress { .. } => "parse_progress",
            IngestJobEventKind::CommitDone { .. } => "commit_done",
            IngestJobEventKind::Finished { .. } => "finished",
            IngestJobEventKind::Failed { .. } => "failed",
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            IngestJobEventKind::CloneDone { ms } => json!({ "event": "clone_done", "ms": ms }),
            IngestJobEventKind::WalkDone { files_found } => {
                json!({ "event": "walk_done", "files_found": files_found })
            }
            IngestJobEventKind::ParseProgress { done, total } => {
                json!({ "event": "parse_progress", "done": done, "total": total })
            }
            IngestJobEventKind::CommitDone { graph_version } => {
                json!({ "event": "commit_done", "graph_version": graph_version })
            }
            IngestJobEventKind::Finished { output } => {
                json!({ "event": "finished", "output": output.to_json() })
            }
            IngestJobEventKind::Failed { code, message } => {
                json!({ "event": "failed", "code": code, "message": message })
            }
        }
    }
}

impl IngestJobEventKind {
    /// Rebuild a milestone event kind from its persisted JSON (D-jobs recovery).
    /// `ParseProgress` is never persisted, so it is not parsed here.
    fn from_json(value: &Value) -> Option<IngestJobEventKind> {
        let u64_at = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
        let str_at = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        match value.get("event").and_then(Value::as_str)? {
            "clone_done" => Some(IngestJobEventKind::CloneDone { ms: u64_at("ms") }),
            "walk_done" => Some(IngestJobEventKind::WalkDone {
                files_found: u64_at("files_found"),
            }),
            "parse_progress" => Some(IngestJobEventKind::ParseProgress {
                done: u64_at("done"),
                total: u64_at("total"),
            }),
            "commit_done" => Some(IngestJobEventKind::CommitDone {
                graph_version: u64_at("graph_version"),
            }),
            "finished" => Some(IngestJobEventKind::Finished {
                output: crate::ingest_output_from_json(value.get("output")?)?,
            }),
            "failed" => Some(IngestJobEventKind::Failed {
                code: str_at("code"),
                message: str_at("message"),
            }),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct IngestJobEvent {
    pub sequence: u64,
    pub recorded_at_ms: u64,
    pub kind: IngestJobEventKind,
}

impl IngestJobEvent {
    pub fn to_json(&self) -> Value {
        let mut payload = self.kind.to_json();
        if let Some(object) = payload.as_object_mut() {
            object.insert("sequence".to_string(), json!(self.sequence));
            object.insert("recorded_at_ms".to_string(), json!(self.recorded_at_ms));
        }
        payload
    }

    fn from_json(value: &Value) -> Option<IngestJobEvent> {
        Some(IngestJobEvent {
            sequence: value.get("sequence").and_then(Value::as_u64).unwrap_or(0),
            recorded_at_ms: value
                .get("recorded_at_ms")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            kind: IngestJobEventKind::from_json(value)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct IngestJobStatus {
    pub job_id: String,
    pub tenant_id: String,
    pub repo_id: String,
    pub operation: String,
    pub state: IngestJobState,
    pub stage: String,
    pub files_total: u64,
    pub files_done: u64,
    pub submitted_at_ms: u64,
    pub updated_at_ms: u64,
    pub output: Option<IngestCodebaseOutput>,
    pub error_code: String,
    pub error_message: String,
}

impl IngestJobStatus {
    pub fn to_json(&self) -> Value {
        json!({
            "job_id": self.job_id,
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "operation": self.operation,
            "state": self.state.as_str(),
            "stage": self.stage,
            "files_total": self.files_total,
            "files_done": self.files_done,
            "submitted_at_ms": self.submitted_at_ms,
            "updated_at_ms": self.updated_at_ms,
            "output": self.output.as_ref().map(IngestCodebaseOutput::to_json),
            "error_code": self.error_code,
            "error_message": self.error_message,
        })
    }
}

/// What to run. `repo_url` non-empty routes through the CA-1 shallow clone;
/// otherwise `input.repo_path` is a local directory. `parse_budget_ms`:
/// `None` uses the `THEOREM_CODE_INGEST_PARSE_BUDGET_MS` default, `Some(0)`
/// disables the budget, `Some(n)` caps the parse stage at `n` milliseconds.
#[derive(Clone, Debug, Default)]
pub struct IngestJobRequest {
    pub input: IngestCodebaseInput,
    pub operation: String,
    pub repo_url: String,
    pub caps: RepoFetchCaps,
    pub parse_budget_ms: Option<u64>,
    #[cfg(test)]
    pub(crate) test_pause: Option<Arc<TestIngestPause>>,
}

/// Test-only barrier between prepare and commit, so a test can prove the
/// store stays unlocked during the heavy phase.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct TestIngestPause {
    state: Mutex<TestPauseState>,
    signal: Condvar,
}

#[cfg(test)]
#[derive(Debug, Default)]
struct TestPauseState {
    arrived: bool,
    released: bool,
}

#[cfg(test)]
impl TestIngestPause {
    pub(crate) fn wait_until_arrived(&self) {
        let mut state = self.state.lock().expect("test pause state");
        while !state.arrived {
            state = self.signal.wait(state).expect("test pause wait");
        }
    }

    pub(crate) fn release(&self) {
        let mut state = self.state.lock().expect("test pause state");
        state.released = true;
        drop(state);
        self.signal.notify_all();
    }

    fn arrive_and_wait(&self) {
        let mut state = self.state.lock().expect("test pause state");
        state.arrived = true;
        self.signal.notify_all();
        while !state.released {
            state = self.signal.wait(state).expect("test pause wait");
        }
    }
}

struct IngestJobRecord {
    status: IngestJobStatus,
    events: Vec<IngestJobEvent>,
    /// Takeable by the worker when the job starts running.
    request: Option<IngestJobRequest>,
    /// Immutable serialized request, kept so a durable mirror write (and a
    /// post-restart re-run of an interrupted job) has the inputs even after the
    /// worker has taken `request`.
    request_json: Value,
}

/// What a persist write needs, captured under the registry lock and written to
/// the store after the lock is dropped (so the store lock is never taken while
/// holding the registry lock).
struct PersistSnapshot {
    status: IngestJobStatus,
    milestone_events: Vec<IngestJobEvent>,
    request_json: Value,
    prune: bool,
}

#[derive(Default)]
struct RegistryInner {
    jobs: BTreeMap<String, IngestJobRecord>,
    queue: VecDeque<String>,
    order: VecDeque<String>,
    submitted: u64,
}

pub struct IngestJobRegistry {
    inner: Mutex<RegistryInner>,
    signal: Condvar,
    /// When set, job lifecycle transitions mirror into this RedCore store as
    /// `CodeIngestJob` nodes so submitted jobs survive a process restart. Held
    /// weakly so the registry never keeps the store alive on its own.
    persist: Option<Weak<Mutex<RedCoreGraphStore>>>,
}

impl Default for IngestJobRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl IngestJobRegistry {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
            signal: Condvar::new(),
            persist: None,
        }
    }

    /// A registry that durably mirrors job state into `store`.
    pub(crate) fn with_persistence(store: Weak<Mutex<RedCoreGraphStore>>) -> Self {
        Self {
            inner: Mutex::new(RegistryInner::default()),
            signal: Condvar::new(),
            persist: Some(store),
        }
    }

    /// Capture a persist snapshot from a record (milestone events only; the
    /// high-frequency ParseProgress is never persisted, keeping the store lock
    /// off the parse loop).
    fn snapshot(record: &IngestJobRecord, prune: bool) -> PersistSnapshot {
        PersistSnapshot {
            status: record.status.clone(),
            milestone_events: record
                .events
                .iter()
                .filter(|event| !matches!(event.kind, IngestJobEventKind::ParseProgress { .. }))
                .cloned()
                .collect(),
            request_json: record.request_json.clone(),
            prune,
        }
    }

    /// Write a snapshot to the durable mirror. Acquires the store lock briefly
    /// and only outside the registry lock; a dropped store is a silent no-op.
    fn persist(&self, snapshot: PersistSnapshot) {
        let Some(weak) = self.persist.as_ref() else {
            return;
        };
        let Some(store) = weak.upgrade() else {
            return;
        };
        let Ok(mut store) = store.lock() else {
            return;
        };
        let node = job_node(&snapshot);
        let _ = store.commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(node)]));
        if snapshot.prune {
            prune_persisted_terminal_jobs(&mut store);
        }
    }

    pub(crate) fn submit(&self, mut request: IngestJobRequest) -> IngestJobStatus {
        if request.operation != "reindex" {
            request.operation = "ingest".to_string();
        }
        let tenant_id = normalize_tenant(&request.input.tenant_id);
        let submitted_at_ms = now_ms().max(0) as u64;
        let mut inner = self.inner.lock().expect("ingest job registry");
        inner.submitted += 1;
        let job_id = format!(
            "code:ingest-job:{}",
            stable_hash(json!({
                "tenant_id": tenant_id,
                "operation": request.operation,
                "repo_path": request.input.repo_path,
                "repo_url": request.repo_url,
                "submitted_at_ms": submitted_at_ms,
                "sequence": inner.submitted,
            }))
        );
        let status = IngestJobStatus {
            job_id: job_id.clone(),
            tenant_id,
            repo_id: request.input.repo_id.trim().to_string(),
            operation: request.operation.clone(),
            state: IngestJobState::Queued,
            stage: "queued".to_string(),
            files_total: 0,
            files_done: 0,
            submitted_at_ms,
            updated_at_ms: submitted_at_ms,
            output: None,
            error_code: String::new(),
            error_message: String::new(),
        };
        let request_json = request_to_json(&request);
        let record = IngestJobRecord {
            status: status.clone(),
            events: Vec::new(),
            request: Some(request),
            request_json,
        };
        let snapshot = Self::snapshot(&record, true);
        inner.jobs.insert(job_id.clone(), record);
        inner.order.push_back(job_id.clone());
        inner.queue.push_back(job_id);
        prune_retained_jobs(&mut inner);
        drop(inner);
        self.persist(snapshot);
        self.signal.notify_all();
        status
    }

    pub fn status(&self, job_id: &str) -> Option<IngestJobStatus> {
        let inner = self.inner.lock().expect("ingest job registry");
        inner.jobs.get(job_id).map(|record| record.status.clone())
    }

    /// Events with `sequence > after_sequence`. Returns `None` for an unknown
    /// job; the bool reports whether the job is terminal (no more events).
    pub fn events_after(
        &self,
        job_id: &str,
        after_sequence: u64,
    ) -> Option<(Vec<IngestJobEvent>, bool)> {
        let inner = self.inner.lock().expect("ingest job registry");
        let record = inner.jobs.get(job_id)?;
        Some((
            record
                .events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .cloned()
                .collect(),
            record.status.state.is_terminal(),
        ))
    }

    /// Like `events_after`, but blocks up to `timeout` until at least one new
    /// event exists or the job is terminal. A timeout returns an empty vec
    /// with `terminal == false` so callers can loop.
    pub fn wait_events(
        &self,
        job_id: &str,
        after_sequence: u64,
        timeout: Duration,
    ) -> Option<(Vec<IngestJobEvent>, bool)> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().expect("ingest job registry");
        loop {
            let record = inner.jobs.get(job_id)?;
            let events: Vec<IngestJobEvent> = record
                .events
                .iter()
                .filter(|event| event.sequence > after_sequence)
                .cloned()
                .collect();
            let terminal = record.status.state.is_terminal();
            if !events.is_empty() || terminal {
                return Some((events, terminal));
            }
            let now = Instant::now();
            if now >= deadline {
                return Some((Vec::new(), false));
            }
            let (guard, _) = self
                .signal
                .wait_timeout(inner, deadline - now)
                .expect("ingest job registry wait");
            inner = guard;
        }
    }

    pub(crate) fn record_event(&self, job_id: &str, kind: IngestJobEventKind) {
        let mut inner = self.inner.lock().expect("ingest job registry");
        let Some(record) = inner.jobs.get_mut(job_id) else {
            return;
        };
        match &kind {
            IngestJobEventKind::CloneDone { .. } => {
                record.status.stage = "walk".to_string();
            }
            IngestJobEventKind::WalkDone { files_found } => {
                record.status.stage = "parse".to_string();
                record.status.files_total = *files_found;
            }
            IngestJobEventKind::ParseProgress { done, total } => {
                record.status.stage = "parse".to_string();
                record.status.files_done = *done;
                record.status.files_total = *total;
            }
            IngestJobEventKind::CommitDone { .. } => {
                record.status.stage = "done".to_string();
            }
            IngestJobEventKind::Finished { output } => {
                record.status.stage = "done".to_string();
                record.status.repo_id = output.repo_id.clone();
                record.status.state = if output.status == "budget_exceeded" {
                    IngestJobState::BudgetExceeded
                } else {
                    IngestJobState::Finished
                };
                record.status.output = Some(output.clone());
            }
            IngestJobEventKind::Failed { code, message } => {
                record.status.stage = "failed".to_string();
                record.status.state = IngestJobState::Failed;
                record.status.error_code = code.clone();
                record.status.error_message = message.clone();
            }
        }
        let sequence = record.events.len() as u64 + 1;
        record.status.updated_at_ms = now_ms().max(0) as u64;
        let persist_event = !matches!(kind, IngestJobEventKind::ParseProgress { .. });
        record.events.push(IngestJobEvent {
            sequence,
            recorded_at_ms: now_ms().max(0) as u64,
            kind,
        });
        // Persist milestones and terminals (which carry the output/error), and
        // prune on terminal. ParseProgress stays in memory only, so the store
        // lock is never taken inside the parse loop.
        let snapshot = persist_event.then(|| {
            let prune = record.status.state.is_terminal();
            Self::snapshot(record, prune)
        });
        drop(inner);
        if let Some(snapshot) = snapshot {
            self.persist(snapshot);
        }
        self.signal.notify_all();
    }

    fn set_stage(&self, job_id: &str, stage: &str) {
        let mut inner = self.inner.lock().expect("ingest job registry");
        if let Some(record) = inner.jobs.get_mut(job_id) {
            record.status.stage = stage.to_string();
            record.status.updated_at_ms = now_ms().max(0) as u64;
        }
        drop(inner);
        self.signal.notify_all();
    }

    fn mark_running(&self, job_id: &str) {
        let mut inner = self.inner.lock().expect("ingest job registry");
        let snapshot = inner.jobs.get_mut(job_id).map(|record| {
            record.status.state = IngestJobState::Running;
            record.status.stage = "start".to_string();
            record.status.updated_at_ms = now_ms().max(0) as u64;
            Self::snapshot(record, false)
        });
        drop(inner);
        if let Some(snapshot) = snapshot {
            self.persist(snapshot);
        }
        self.signal.notify_all();
    }

    fn next_queued(&self, timeout: Duration) -> Option<(String, IngestJobRequest)> {
        let mut inner = self.inner.lock().expect("ingest job registry");
        loop {
            while let Some(job_id) = inner.queue.pop_front() {
                if let Some(request) = inner
                    .jobs
                    .get_mut(&job_id)
                    .and_then(|record| record.request.take())
                {
                    return Some((job_id, request));
                }
            }
            let (guard, wait) = self
                .signal
                .wait_timeout(inner, timeout)
                .expect("ingest job registry wait");
            inner = guard;
            if wait.timed_out() {
                return None;
            }
        }
    }

    /// Load durably-mirrored jobs from the store into the in-memory registry on
    /// startup. Terminal jobs are restored read-only (queryable, streamable);
    /// jobs that were queued or running when the process died are re-enqueued
    /// (stage `recovered`) to run again, since their in-memory parse state did
    /// not survive and the final commit is atomic (a re-run either redoes work
    /// that never committed or re-commits a fresh, superseding generation).
    /// Returns the number of re-enqueued (runnable) jobs.
    pub(crate) fn recover_from_store(&self, store: &RedCoreGraphStore) -> usize {
        let nodes = store
            .query_nodes(NodeQuery::label(CODE_INGEST_JOB_LABEL).with_limit(100_000))
            .unwrap_or_default();
        let mut recovered: Vec<(u64, String, IngestJobRecord, bool)> = Vec::new();
        for node in nodes {
            if let Some((record, runnable)) = job_record_from_node(&node) {
                recovered.push((
                    record.status.submitted_at_ms,
                    record.status.job_id.clone(),
                    record,
                    runnable,
                ));
            }
        }
        // Oldest first so the in-memory retention order matches submit order.
        recovered.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

        let mut inner = self.inner.lock().expect("ingest job registry");
        let mut runnable_count = 0;
        for (_, job_id, record, runnable) in recovered {
            if inner.jobs.contains_key(&job_id) {
                continue;
            }
            inner.order.push_back(job_id.clone());
            if runnable {
                inner.queue.push_back(job_id.clone());
                runnable_count += 1;
            }
            inner.jobs.insert(job_id, record);
        }
        prune_retained_jobs(&mut inner);
        drop(inner);
        if runnable_count > 0 {
            self.signal.notify_all();
        }
        runnable_count
    }
}

/// Evict the oldest TERMINAL jobs beyond the retention cap. Queued/running
/// jobs are never evicted.
fn prune_retained_jobs(inner: &mut RegistryInner) {
    while inner.jobs.len() > MAX_RETAINED_JOBS {
        let evict = inner.order.iter().position(|job_id| {
            inner
                .jobs
                .get(job_id)
                .map(|record| record.status.state.is_terminal())
                .unwrap_or(true)
        });
        let Some(position) = evict else {
            return;
        };
        if let Some(job_id) = inner.order.remove(position) {
            inner.jobs.remove(&job_id);
        }
    }
}

/// Build the durable `CodeIngestJob` mirror node for a snapshot. The request is
/// stored so an interrupted job can re-run after a restart; the output (when
/// finished) and the milestone events are stored so a recovered job is
/// queryable and streamable.
fn job_node(snapshot: &PersistSnapshot) -> NodeRecord {
    let status = &snapshot.status;
    let events: Vec<Value> = snapshot
        .milestone_events
        .iter()
        .map(IngestJobEvent::to_json)
        .collect();
    NodeRecord::new(
        &status.job_id,
        [CODE_INGEST_JOB_LABEL],
        json!({
            "tenant_id": status.tenant_id,
            "repo_id": status.repo_id,
            "operation": status.operation,
            "state": status.state.as_str(),
            "stage": status.stage,
            "files_total": status.files_total,
            "files_done": status.files_done,
            "submitted_at_ms": status.submitted_at_ms,
            "updated_at_ms": status.updated_at_ms,
            "error_code": status.error_code,
            "error_message": status.error_message,
            "request": snapshot.request_json,
            "output": status.output.as_ref().map(IngestCodebaseOutput::to_json),
            "milestone_events": events,
            "source": SOURCE,
        }),
    )
}

/// Rebuild an in-memory record from a persisted `CodeIngestJob` node. Returns
/// `(record, runnable)`; a non-terminal (interrupted) job comes back as
/// `Queued`/`recovered` with its request restored so it can re-run.
fn job_record_from_node(node: &NodeRecord) -> Option<(IngestJobRecord, bool)> {
    let props = &node.properties;
    let str_at = |key: &str| {
        props
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let u64_at = |key: &str| props.get(key).and_then(Value::as_u64).unwrap_or(0);

    let persisted_state = IngestJobState::from_str(&str_at("state"));
    let terminal = persisted_state.is_terminal();
    let output = props
        .get("output")
        .filter(|value| !value.is_null())
        .and_then(crate::ingest_output_from_json);
    let events: Vec<IngestJobEvent> = props
        .get("milestone_events")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(IngestJobEvent::from_json).collect())
        .unwrap_or_default();
    let request_json = props.get("request").cloned().unwrap_or_else(|| json!({}));

    let mut status = IngestJobStatus {
        job_id: node.id.clone(),
        tenant_id: str_at("tenant_id"),
        repo_id: str_at("repo_id"),
        operation: str_at("operation"),
        state: persisted_state,
        stage: str_at("stage"),
        files_total: u64_at("files_total"),
        files_done: u64_at("files_done"),
        submitted_at_ms: u64_at("submitted_at_ms"),
        updated_at_ms: u64_at("updated_at_ms"),
        output,
        error_code: str_at("error_code"),
        error_message: str_at("error_message"),
    };

    let (request, runnable) = if terminal {
        (None, false)
    } else {
        // Interrupted: re-run from the persisted request, or drop if it cannot
        // be reconstructed (no inputs to re-run from).
        match request_from_json(&request_json) {
            Some(request) => {
                status.state = IngestJobState::Queued;
                status.stage = "recovered".to_string();
                (Some(request), true)
            }
            None => return None,
        }
    };

    Some((
        IngestJobRecord {
            status,
            events,
            request,
            request_json,
        },
        runnable,
    ))
}

/// TTL-expire durable terminal jobs beyond the cap so the mirror stays bounded.
/// Queued/running jobs are never expired. Runs on submit (bounded frequency).
fn prune_persisted_terminal_jobs(store: &mut RedCoreGraphStore) {
    let Ok(nodes) =
        store.query_nodes(NodeQuery::label(CODE_INGEST_JOB_LABEL).with_limit(100_000))
    else {
        return;
    };
    let mut terminal: Vec<(u64, String)> = nodes
        .into_iter()
        .filter(|node| {
            IngestJobState::from_str(
                node.properties
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("queued"),
            )
            .is_terminal()
        })
        .map(|node| {
            (
                node.properties
                    .get("submitted_at_ms")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                node.id,
            )
        })
        .collect();
    if terminal.len() <= MAX_PERSISTED_TERMINAL_JOBS {
        return;
    }
    // Newest first; expire everything past the cap.
    terminal.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
    let expired_boundary = now_ms().saturating_sub(1);
    let mut expired_any = false;
    for (_, job_id) in terminal.into_iter().skip(MAX_PERSISTED_TERMINAL_JOBS) {
        if store.set_node_ttl(&job_id, Some(expired_boundary)).is_ok() {
            expired_any = true;
        }
    }
    if expired_any {
        let _ = store.purge_expired_nodes();
    }
}

/// Serialize a job request for the durable mirror. The `#[cfg(test)]` pause
/// barrier is intentionally not serialized.
fn request_to_json(request: &IngestJobRequest) -> Value {
    let input = &request.input;
    json!({
        "input": {
            "tenant_id": input.tenant_id,
            "repo_path": input.repo_path,
            "repo_id": input.repo_id,
            "include_extensions": input.include_extensions,
            "exclude_dirs": input.exclude_dirs,
            "max_files": input.max_files,
            "max_file_bytes": input.max_file_bytes,
            "max_total_bytes": input.max_total_bytes,
            "actor": input.actor,
        },
        "operation": request.operation,
        "repo_url": request.repo_url,
        "caps": {
            "max_total_bytes": request.caps.max_total_bytes,
            "clone_timeout_ms": request.caps.clone_timeout_ms,
        },
        "parse_budget_ms": request.parse_budget_ms,
    })
}

fn request_from_json(value: &Value) -> Option<IngestJobRequest> {
    let input_value = value.get("input")?;
    let str_at = |source: &Value, key: &str| {
        source
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let u64_at = |source: &Value, key: &str| source.get(key).and_then(Value::as_u64).unwrap_or(0);
    let string_vec = |source: &Value, key: &str| {
        source
            .get(key)
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    };
    let input = IngestCodebaseInput {
        tenant_id: str_at(input_value, "tenant_id"),
        repo_path: str_at(input_value, "repo_path"),
        repo_id: str_at(input_value, "repo_id"),
        include_extensions: string_vec(input_value, "include_extensions"),
        exclude_dirs: string_vec(input_value, "exclude_dirs"),
        max_files: u64_at(input_value, "max_files"),
        max_file_bytes: u64_at(input_value, "max_file_bytes"),
        max_total_bytes: u64_at(input_value, "max_total_bytes"),
        actor: str_at(input_value, "actor"),
    };
    let caps_value = value.get("caps").cloned().unwrap_or_else(|| json!({}));
    let caps = RepoFetchCaps {
        max_total_bytes: caps_value
            .get("max_total_bytes")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| RepoFetchCaps::default().max_total_bytes),
        clone_timeout_ms: caps_value
            .get("clone_timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or_else(|| RepoFetchCaps::default().clone_timeout_ms),
    };
    // `..Default::default()` fills the `#[cfg(test)]` pause field; in non-test
    // builds every field is already named, so the update is a no-op there.
    #[cfg_attr(not(test), allow(clippy::needless_update))]
    Some(IngestJobRequest {
        input,
        operation: str_at(value, "operation"),
        repo_url: str_at(value, "repo_url"),
        caps,
        parse_budget_ms: value.get("parse_budget_ms").and_then(Value::as_u64),
        ..Default::default()
    })
}

/// The single ingest worker. Holds the store WEAKLY: once every runtime clone
/// is dropped the loop exits at the next idle poll instead of keeping the
/// store (and its file handles) alive forever.
pub(crate) fn ingest_worker_loop(
    store: Weak<Mutex<RedCoreGraphStore>>,
    registry: Arc<IngestJobRegistry>,
) {
    loop {
        let Some((job_id, request)) = registry.next_queued(WORKER_IDLE_POLL) else {
            if store.strong_count() == 0 {
                return;
            }
            continue;
        };
        run_ingest_job(&store, &registry, &job_id, request);
    }
}

fn run_ingest_job(
    store: &Weak<Mutex<RedCoreGraphStore>>,
    registry: &Arc<IngestJobRegistry>,
    job_id: &str,
    request: IngestJobRequest,
) {
    let started = Instant::now();
    registry.mark_running(job_id);
    let fail = |error: CodeIndexError| {
        registry.record_event(
            job_id,
            IngestJobEventKind::Failed {
                code: error.code,
                message: error.message,
            },
        );
    };

    let url = if request.repo_url.trim().is_empty() {
        None
    } else {
        registry.set_stage(job_id, "clone");
        Some((request.repo_url.as_str(), &request.caps))
    };
    let (input, clone_ms, _fetched) = match stage_repo_for_ingest(request.input.clone(), url) {
        Ok(staged) => staged,
        Err(error) => return fail(error),
    };
    if !request.repo_url.trim().is_empty() {
        registry.record_event(job_id, IngestJobEventKind::CloneDone { ms: clone_ms });
    }

    let resolve_started = Instant::now();
    let config = match resolve_ingest_config(input) {
        Ok(config) => config,
        Err(error) => return fail(error),
    };
    let resolve_ms = crate::elapsed_ms(resolve_started);

    // Brief lock: snapshot the prior generation for an incremental reindex.
    let prior = {
        let Some(store) = store.upgrade() else {
            return fail(store_dropped_error());
        };
        let guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => return fail(store_poisoned_error()),
        };
        match snapshot_for_operation(&guard, &request.operation, &config) {
            Ok(prior) => prior,
            Err(error) => return fail(error),
        }
    };

    // Heavy phase: walk, load, parse, mutation build. NO store lock held, so
    // concurrent searches on the same store proceed during the parse.
    registry.set_stage(job_id, "walk");
    let sink = |kind: IngestJobEventKind| registry.record_event(job_id, kind);
    // Carried-text loader for incremental edge reinference: takes the store
    // lock for exactly one batch read after the change split, off the parse
    // path. Upgrades the Weak so a dropped runtime yields an empty map.
    let text_store = store.clone();
    let text_tenant = config.tenant_id.clone();
    let carried_text_loader = move |hashes: &[String]| match text_store.upgrade() {
        Some(store) => match store.lock() {
            Ok(store) => load_file_texts(&store, &text_tenant, hashes),
            Err(_) => HashMap::new(),
        },
        None => HashMap::new(),
    };
    let options = IngestPipelineOptions {
        prior,
        sink: Some(&sink),
        carried_text_loader: Some(&carried_text_loader),
        parse_budget_ms: request
            .parse_budget_ms
            .unwrap_or_else(default_parse_budget_ms),
    };
    let prepared =
        match prepare_codebase_ingest_resolved(config, clone_ms, resolve_ms, started, options) {
            Ok(prepared) => prepared,
            Err(error) => return fail(error),
        };
    drop(_fetched);

    #[cfg(test)]
    if let Some(pause) = &request.test_pause {
        pause.arrive_and_wait();
    }

    registry.set_stage(job_id, "commit");
    // Scope the upgraded (strong) store handle to JUST the commit so the worker
    // does not pin the store open while it records the terminal events: once
    // every runtime clone is dropped the store can release its directory lock
    // promptly (the event recording below only re-upgrades transiently inside
    // the durable mirror write).
    let output = {
        let Some(store) = store.upgrade() else {
            return fail(store_dropped_error());
        };
        let mut guard = match store.lock() {
            Ok(guard) => guard,
            Err(_) => return fail(store_poisoned_error()),
        };
        commit_prepared_ingest(&mut guard, prepared, &request.operation)
    };
    match output {
        Ok(output) => {
            registry.record_event(
                job_id,
                IngestJobEventKind::CommitDone {
                    graph_version: output.graph_version,
                },
            );
            registry.record_event(job_id, IngestJobEventKind::Finished { output });
        }
        Err(error) => fail(error),
    }
}

fn store_dropped_error() -> CodeIndexError {
    CodeIndexError {
        code: "code_index_store_dropped".to_string(),
        message: "code index store was dropped before the job could commit".to_string(),
    }
}

fn store_poisoned_error() -> CodeIndexError {
    CodeIndexError {
        code: "code_index_lock_poisoned".to_string(),
        message: "code index RedCore store lock poisoned".to_string(),
    }
}
