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

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, Weak};
use std::time::{Duration, Instant};

use rustyred_thg_core::{now_ms, stable_hash, RedCoreGraphStore};
use serde_json::{json, Value};

use crate::{
    commit_prepared_ingest, default_parse_budget_ms, normalize_tenant,
    prepare_codebase_ingest_resolved, resolve_ingest_config, snapshot_for_operation,
    stage_repo_for_ingest, CodeIndexError, IngestCodebaseInput, IngestCodebaseOutput,
    IngestPipelineOptions, RepoFetchCaps,
};

const MAX_RETAINED_JOBS: usize = 64;
const WORKER_IDLE_POLL: Duration = Duration::from_secs(2);

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
}

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
    request: Option<IngestJobRequest>,
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
        inner.jobs.insert(
            job_id.clone(),
            IngestJobRecord {
                status: status.clone(),
                events: Vec::new(),
                request: Some(request),
            },
        );
        inner.order.push_back(job_id.clone());
        inner.queue.push_back(job_id);
        prune_retained_jobs(&mut inner);
        drop(inner);
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
        record.events.push(IngestJobEvent {
            sequence,
            recorded_at_ms: now_ms().max(0) as u64,
            kind,
        });
        drop(inner);
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
        if let Some(record) = inner.jobs.get_mut(job_id) {
            record.status.state = IngestJobState::Running;
            record.status.stage = "start".to_string();
            record.status.updated_at_ms = now_ms().max(0) as u64;
        }
        drop(inner);
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
    let options = IngestPipelineOptions {
        prior,
        sink: Some(&sink),
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
    let Some(store) = store.upgrade() else {
        return fail(store_dropped_error());
    };
    let output = {
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
