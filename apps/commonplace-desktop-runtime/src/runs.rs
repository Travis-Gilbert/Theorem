//! The run channel for phone-driven runs (phone-control handoff Part B
//! deliverables 3 + 5).
//!
//! A paired phone submits a *run* (an intent/spec for the local agent to carry
//! out against the working tree). This module owns the run's lifecycle, its
//! event stream, the executor seam that does the actual work, and the in-flight
//! controls (approve / redirect / stop).
//!
//! # Lifecycle
//!
//! ```text
//! Submitted ─▶ (AwaitingAuthorization)? ─▶ Running ─▶ Done | Stopped | Failed
//! ```
//!
//! * **Submitted** -> the run record exists; the registry is about to dispatch.
//! * **AwaitingAuthorization** -> the run's action is a tier-2/3 action (per the
//!   [`crate::authorization`] surface, grounded in `agent_binding` tiers) and no
//!   human authorization was presented. The run HOLDS here and the gated action
//!   is NOT executed until an explicit [`approve`](RunRegistry::approve) arrives.
//! * **Running** -> the executor is producing events.
//! * **Done / Stopped / Failed** -> terminal. `Stopped` is the result of an
//!   honored [`stop`](RunRegistry::stop) (cooperative cancel).
//!
//! # Streaming
//!
//! Each run has a broadcast channel of [`RunEvent`]s typed as
//! `Trace | Obligation | Diff | Status` (the handoff streams "traces,
//! obligations, and diffs"). The control endpoint serves these to the phone over
//! Server-Sent Events; tests subscribe to the channel directly. Late subscribers
//! also get the full backlog from [`RunRecord::events`] so a phone that connects
//! after submission does not miss the early trace.
//!
//! # Executor seam
//!
//! [`RunExecutor`] is the seam the real desktop wires to the actual agent runner
//! (theorem-receiver spawns `claude` / `codex` against the working tree). This
//! slice ships [`MockExecutor`], which emits a scripted, cancellation-aware event
//! sequence so the whole channel is cargo-testable WITHOUT spawning a real agent.
//! The real executor is a desktop-layer follow-up (see `IMPLEMENTATION.md`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rustyred_thg_core::now_ms;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::authorization::{authorize_tier, ActionTierTable, AuthorizationDecision, TIER_ONE};

/// Capacity of each run's broadcast event channel. Large enough that a slow SSE
/// reader does not lose events under normal run sizes; a reader that lags past
/// this still recovers the full ordered history from [`RunRecord::events`].
const EVENT_CHANNEL_CAPACITY: usize = 256;

/// The phone-submitted specification for a run.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunSpec {
    /// The intent / instruction for the run (free text from the phone).
    pub intent: String,
    /// The action tier id this run is classified to (one of
    /// [`crate::authorization`]'s `TIER_*`). Defaults to tier-1 when omitted, so
    /// an unclassified run is treated as reversible-and-immediate ONLY if the
    /// caller explicitly leaves it unset; callers that cannot classify should
    /// pass a consequential tier to force a hold.
    #[serde(default = "default_tier")]
    pub action_tier: String,
    /// Whether the submitting human authorized this run up front (e.g. an approve
    /// presented at submission). A tier-2/3 run with this set runs immediately;
    /// without it, it holds.
    #[serde(default)]
    pub human_authorized: bool,
}

fn default_tier() -> String {
    TIER_ONE.to_string()
}

impl RunSpec {
    /// A tier-1 (reversible, immediate) run with the given intent.
    pub fn tier_one(intent: impl Into<String>) -> Self {
        Self {
            intent: intent.into(),
            action_tier: TIER_ONE.to_string(),
            human_authorized: false,
        }
    }

    /// A run at an explicit tier (use the `authorization::TIER_*` ids).
    pub fn at_tier(intent: impl Into<String>, action_tier: impl Into<String>) -> Self {
        Self {
            intent: intent.into(),
            action_tier: action_tier.into(),
            human_authorized: false,
        }
    }
}

/// The lifecycle state of a run.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Submitted,
    AwaitingAuthorization,
    Running,
    Done,
    Stopped,
    Failed,
}

impl RunState {
    /// Whether this is a terminal state (no further transitions).
    pub fn is_terminal(self) -> bool {
        matches!(self, RunState::Done | RunState::Stopped | RunState::Failed)
    }
}

/// The kind of a run event. The handoff streams "traces, obligations, and
/// diffs"; `Status` carries lifecycle transitions so a phone watching only the
/// stream still sees state changes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    /// A free-form progress trace line.
    Trace,
    /// An obligation the run has taken on or discharged (e.g. "must run tests
    /// before commit"). Part of the handoff's first-class stream.
    Obligation,
    /// A proposed or applied diff against the working tree.
    Diff,
    /// A lifecycle transition (the new [`RunState`] is in `body`).
    Status,
}

/// A single event emitted during a run. Streamed to the phone over SSE and kept
/// in the run record's ordered backlog.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RunEvent {
    /// Monotonic per-run sequence number (0-based), so a late subscriber can
    /// reconcile the backlog with the live tail.
    pub seq: u64,
    pub kind: RunEventKind,
    /// Event payload (a trace line, an obligation description, a diff body, or a
    /// status string).
    pub body: String,
    /// Emission time (epoch ms).
    pub at_ms: i64,
}

/// A run record: its id, spec, current state, and ordered event backlog.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunRecord {
    pub run_id: String,
    pub spec: RunSpec,
    pub state: RunState,
    /// Why the run is in [`RunState::AwaitingAuthorization`] / what tier gated it.
    /// `None` once the run has moved past authorization.
    pub authorization: Option<AuthorizationDecision>,
    /// The full ordered event history (the SSE backlog).
    pub events: Vec<RunEvent>,
    pub submitted_at_ms: i64,
}

/// A handle the executor uses to emit events for a run and to observe
/// cooperative cancellation. The registry hands one of these to
/// [`RunExecutor::start`]; the executor calls [`emit`](RunEventSink::emit) for
/// each trace/obligation/diff and checks [`is_cancelled`](RunEventSink::is_cancelled)
/// at safe points so a [`stop`](RunRegistry::stop) is honored mid-run.
#[derive(Clone)]
pub struct RunEventSink {
    run_id: String,
    inner: Arc<RunInner>,
}

impl RunEventSink {
    /// Emit one event of `kind` with `body`. The event is appended to the run's
    /// ordered backlog (assigning the next sequence number) and broadcast to live
    /// subscribers. A broadcast send with no live receivers is fine: the backlog
    /// is the durable record.
    pub fn emit(&self, kind: RunEventKind, body: impl Into<String>) {
        emit_event(&self.inner, &self.run_id, kind, body.into());
    }

    /// Whether a cooperative cancel (a [`stop`](RunRegistry::stop)) has been
    /// requested for this run. A well-behaved executor checks this between units
    /// of work and returns early when it is true.
    pub fn is_cancelled(&self) -> bool {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&self.run_id)
            .map(|live| live.cancel.load(Ordering::SeqCst))
            .unwrap_or(true)
    }

    /// Drain any redirect instructions injected since the last drain. A
    /// [`redirect`](RunRegistry::redirect) appends a new instruction; the executor
    /// polls this to pick up steering mid-run.
    pub fn drain_redirects(&self) -> Vec<String> {
        let mut runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
        runs.get_mut(&self.run_id)
            .map(|live| std::mem::take(&mut live.redirects))
            .unwrap_or_default()
    }
}

/// The seam the registry drives to actually run a run. The real desktop impl
/// spawns the local agent (theorem-receiver -> `claude` / `codex`) against the
/// working tree; for tests and the cargo oracle, [`MockExecutor`] emits a
/// scripted sequence.
///
/// `start` runs to completion (it is invoked on a blocking task by the registry)
/// and returns the terminal [`RunState`] the run reached. It must:
/// * emit its traces/obligations/diffs through `sink`,
/// * poll `sink.is_cancelled()` and return [`RunState::Stopped`] promptly when a
///   stop was requested,
/// * return [`RunState::Done`] on success or [`RunState::Failed`] on error.
pub trait RunExecutor: Send + Sync + 'static {
    fn start(&self, spec: &RunSpec, sink: &RunEventSink) -> RunState;
}

/// A scripted executor for tests and the cargo oracle. It emits a Trace, an
/// Obligation, and a Diff (the three first-class stream kinds), checking
/// cancellation between each step so a [`stop`](RunRegistry::stop) is honored,
/// and folds any drained redirects into extra Trace events so redirect wiring is
/// exercised end-to-end. It NEVER spawns a real process.
#[derive(Clone, Default)]
pub struct MockExecutor {
    /// When set, the executor pauses (busy-waits on cancellation) after the first
    /// event so a test can deterministically stop it mid-run. The wait is bounded
    /// so a test cannot hang if no stop arrives.
    pause_after_first_event: bool,
}

impl MockExecutor {
    /// A mock that runs straight through to completion.
    pub fn new() -> Self {
        Self::default()
    }

    /// A mock that pauses after its first event until cancelled (for stop tests).
    pub fn pausing_after_first_event() -> Self {
        Self {
            pause_after_first_event: true,
        }
    }
}

impl RunExecutor for MockExecutor {
    fn start(&self, spec: &RunSpec, sink: &RunEventSink) -> RunState {
        // Step 1: a trace echoing the intent.
        sink.emit(RunEventKind::Trace, format!("starting: {}", spec.intent));

        if self.pause_after_first_event {
            // Wait (bounded) for a cooperative cancel so a stop test is
            // deterministic. If no stop arrives within the bound, fall through
            // and complete normally (so the test fails loudly rather than hangs).
            let mut waited = std::time::Duration::ZERO;
            let step = std::time::Duration::from_millis(10);
            let bound = std::time::Duration::from_secs(5);
            while !sink.is_cancelled() && waited < bound {
                std::thread::sleep(step);
                waited += step;
            }
        }

        if sink.is_cancelled() {
            return RunState::Stopped;
        }

        // Fold in any steering injected via redirect (deliverable 5).
        for instruction in sink.drain_redirects() {
            sink.emit(RunEventKind::Trace, format!("redirected: {instruction}"));
        }

        // Step 2: an obligation.
        sink.emit(
            RunEventKind::Obligation,
            "verify changes before applying".to_string(),
        );
        if sink.is_cancelled() {
            return RunState::Stopped;
        }

        // Step 3: a diff.
        sink.emit(
            RunEventKind::Diff,
            "--- a/file\n+++ b/file\n@@ +mock change".to_string(),
        );

        RunState::Done
    }
}

/// Per-run mutable state held inside the registry: the durable record, the
/// broadcast sender for live subscribers, the cooperative cancel flag, and any
/// pending redirect instructions.
struct LiveRun {
    record: RunRecord,
    sender: broadcast::Sender<RunEvent>,
    cancel: Arc<AtomicBool>,
    redirects: Vec<String>,
}

/// Shared registry internals, behind an `Arc` so the registry is a cheap
/// cloneable handle the HTTP layer can share across requests.
struct RunInner {
    runs: Mutex<BTreeMap<String, LiveRun>>,
    executor: Arc<dyn RunExecutor>,
    tiers: ActionTierTable,
}

/// Append an event to a run's backlog and broadcast it. Shared by the sink and
/// the registry's own status emissions so sequence numbering stays monotonic
/// across both.
fn emit_event(inner: &Arc<RunInner>, run_id: &str, kind: RunEventKind, body: String) {
    let mut runs = inner.runs.lock().unwrap_or_else(|p| p.into_inner());
    let Some(live) = runs.get_mut(run_id) else {
        return;
    };
    let seq = live.record.events.len() as u64;
    let event = RunEvent {
        seq,
        kind,
        body,
        at_ms: now_ms(),
    };
    live.record.events.push(event.clone());
    // A send error just means no live subscribers; the backlog already has it.
    let _ = live.sender.send(event);
}

/// Set a run's state, recording it on the record and emitting a `Status` event
/// so stream-only subscribers observe the transition.
fn set_state(inner: &Arc<RunInner>, run_id: &str, state: RunState) {
    {
        let mut runs = inner.runs.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(live) = runs.get_mut(run_id) {
            live.record.state = state;
            if state != RunState::AwaitingAuthorization {
                live.record.authorization = None;
            }
        }
    }
    emit_event(inner, run_id, RunEventKind::Status, state_tag(state).to_string());
}

fn state_tag(state: RunState) -> &'static str {
    match state {
        RunState::Submitted => "submitted",
        RunState::AwaitingAuthorization => "awaiting_authorization",
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Stopped => "stopped",
        RunState::Failed => "failed",
    }
}

/// The error type for run-control operations that can fail because a run is
/// unknown or in the wrong state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RunError {
    /// No run with that id.
    NotFound,
    /// The control is not valid in the run's current state (e.g. approving a run
    /// that is not awaiting authorization).
    InvalidState {
        run_id: String,
        state: RunState,
        action: &'static str,
    },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunError::NotFound => write!(f, "run not found"),
            RunError::InvalidState {
                run_id,
                state,
                action,
            } => write!(
                f,
                "cannot {action} run {run_id} in state {}",
                state_tag(*state)
            ),
        }
    }
}

impl std::error::Error for RunError {}

/// The run channel: submit runs, subscribe to their event streams, fetch their
/// records, and drive the in-flight controls (approve / redirect / stop).
///
/// Cloneable handle (the state is `Arc`-backed) so the control router can share
/// one registry across handlers.
#[derive(Clone)]
pub struct RunRegistry {
    inner: Arc<RunInner>,
}

impl RunRegistry {
    /// Build a registry over an executor. The action-tier table is seeded from
    /// `agent_binding` (see [`ActionTierTable::default`]).
    pub fn new(executor: Arc<dyn RunExecutor>) -> Self {
        Self {
            inner: Arc::new(RunInner {
                runs: Mutex::new(BTreeMap::new()),
                executor,
                tiers: ActionTierTable::default(),
            }),
        }
    }

    /// Submit a run. The run is classified to its action tier and authorized
    /// (deliverable 4): a tier-1 (or pre-authorized) run goes straight to
    /// `Running` and the executor is dispatched on a blocking task; a tier-2/3
    /// run with no authorization is parked in `AwaitingAuthorization` and the
    /// gated action is NOT started until [`approve`](Self::approve).
    ///
    /// Returns the run id immediately (the run proceeds asynchronously).
    pub fn submit(&self, spec: RunSpec) -> String {
        let run_id = new_run_id();
        let (sender, _rx) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let decision = authorize_tier(&self.inner.tiers, &spec.action_tier, spec.human_authorized);

        let record = RunRecord {
            run_id: run_id.clone(),
            spec,
            state: RunState::Submitted,
            authorization: Some(decision),
            events: Vec::new(),
            submitted_at_ms: now_ms(),
        };
        {
            let mut runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
            runs.insert(
                run_id.clone(),
                LiveRun {
                    record,
                    sender,
                    cancel: Arc::new(AtomicBool::new(false)),
                    redirects: Vec::new(),
                },
            );
        }
        // Emit the initial Submitted status so a subscriber that attaches at t0
        // sees the lifecycle from the start.
        emit_event(
            &self.inner,
            &run_id,
            RunEventKind::Status,
            state_tag(RunState::Submitted).to_string(),
        );

        match decision {
            AuthorizationDecision::Immediate => self.dispatch(&run_id),
            AuthorizationDecision::HoldForApproval => {
                // HOLD: the gated action does not run until an explicit approve.
                set_state(&self.inner, &run_id, RunState::AwaitingAuthorization);
            }
        }
        run_id
    }

    /// Approve a held run (deliverable 5): release a run that is
    /// `AwaitingAuthorization` and dispatch its (previously gated) action. An
    /// approve on a run in any other state is an `InvalidState` error (idempotent
    /// re-approval is not silently swallowed, so a double-approve is visible).
    pub fn approve(&self, run_id: &str) -> Result<(), RunError> {
        self.expect_state(run_id, RunState::AwaitingAuthorization, "approve")?;
        // The human has now authorized: dispatch the held action.
        self.dispatch(run_id);
        Ok(())
    }

    /// Inject a new instruction into an in-flight (or held) run (deliverable 5).
    /// The instruction is queued; the executor picks it up via
    /// [`RunEventSink::drain_redirects`] at its next safe point. Redirecting a
    /// terminal run is an `InvalidState` error.
    ///
    /// Redirect DEPTH (full re-planning / interruption semantics) is intentionally
    /// shallow here: we queue the steering text and emit a Trace so the wiring is
    /// real and testable; a richer "interrupt and re-plan" contract is a
    /// follow-up (see `IMPLEMENTATION.md`).
    pub fn redirect(&self, run_id: &str, instruction: impl Into<String>) -> Result<(), RunError> {
        let instruction = instruction.into();
        {
            let mut runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
            let live = runs.get_mut(run_id).ok_or(RunError::NotFound)?;
            if live.record.state.is_terminal() {
                return Err(RunError::InvalidState {
                    run_id: run_id.to_string(),
                    state: live.record.state,
                    action: "redirect",
                });
            }
            live.redirects.push(instruction.clone());
        }
        emit_event(
            &self.inner,
            run_id,
            RunEventKind::Trace,
            format!("redirect queued: {instruction}"),
        );
        Ok(())
    }

    /// Stop an in-flight run (deliverable 5): set its cooperative cancel flag so
    /// the executor returns `Stopped` at its next checkpoint. Stopping a held run
    /// (`AwaitingAuthorization`) cancels it without ever running the gated action.
    /// Stopping a terminal run is an `InvalidState` error.
    ///
    /// This is cooperative: a well-behaved executor (like [`MockExecutor`]) polls
    /// [`RunEventSink::is_cancelled`] and returns promptly. For a held run there
    /// is no executor in flight, so stop transitions it straight to `Stopped`.
    pub fn stop(&self, run_id: &str) -> Result<(), RunError> {
        let was_held;
        {
            let mut runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
            let live = runs.get_mut(run_id).ok_or(RunError::NotFound)?;
            if live.record.state.is_terminal() {
                return Err(RunError::InvalidState {
                    run_id: run_id.to_string(),
                    state: live.record.state,
                    action: "stop",
                });
            }
            live.cancel.store(true, Ordering::SeqCst);
            was_held = live.record.state == RunState::AwaitingAuthorization;
        }
        if was_held {
            // No executor is running for a held run: mark it Stopped directly so
            // the gated action never executes.
            set_state(&self.inner, run_id, RunState::Stopped);
        }
        // For a Running run, the executor observes the flag and returns Stopped,
        // and the dispatch completion records the terminal state.
        Ok(())
    }

    /// Snapshot a run's record (state + full event backlog), or `None` if unknown.
    pub fn record(&self, run_id: &str) -> Option<RunRecord> {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(run_id)
            .map(|live| live.record.clone())
    }

    /// List all run records (ordered by id).
    pub fn list(&self) -> Vec<RunRecord> {
        self.inner
            .runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(|live| live.record.clone())
            .collect()
    }

    /// Subscribe to a run's live event stream plus its current backlog. Returns
    /// `(backlog, receiver)`: the backlog is every event so far (so an SSE
    /// consumer can replay history before tailing), and the receiver yields
    /// events emitted after the subscription. `None` if the run is unknown.
    pub fn subscribe(&self, run_id: &str) -> Option<(Vec<RunEvent>, broadcast::Receiver<RunEvent>)> {
        let runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
        let live = runs.get(run_id)?;
        Some((live.record.events.clone(), live.sender.subscribe()))
    }

    /// Dispatch a run's executor on a blocking task and record the terminal state
    /// it returns. Sets the run `Running` first (emitting the status), then runs
    /// the executor with a [`RunEventSink`], then records the terminal state.
    fn dispatch(&self, run_id: &str) {
        set_state(&self.inner, run_id, RunState::Running);
        let inner = Arc::clone(&self.inner);
        let run_id = run_id.to_string();
        let sink = RunEventSink {
            run_id: run_id.clone(),
            inner: Arc::clone(&inner),
        };
        let executor = Arc::clone(&inner.executor);
        // The executor is synchronous/blocking (a real one spawns a child
        // process); run it on the blocking pool so it never stalls the async
        // reactor that serves SSE.
        tokio::task::spawn_blocking(move || {
            let spec = {
                let runs = inner.runs.lock().unwrap_or_else(|p| p.into_inner());
                match runs.get(&run_id) {
                    Some(live) => live.record.spec.clone(),
                    None => return,
                }
            };
            let terminal = executor.start(&spec, &sink);
            set_state(&inner, &run_id, terminal);
        });
    }

    /// Assert a run is in `expected`, returning the appropriate [`RunError`] if
    /// it is unknown or in another state.
    fn expect_state(
        &self,
        run_id: &str,
        expected: RunState,
        action: &'static str,
    ) -> Result<(), RunError> {
        let runs = self.inner.runs.lock().unwrap_or_else(|p| p.into_inner());
        let live = runs.get(run_id).ok_or(RunError::NotFound)?;
        if live.record.state == expected {
            Ok(())
        } else {
            Err(RunError::InvalidState {
                run_id: run_id.to_string(),
                state: live.record.state,
                action,
            })
        }
    }
}

/// Generate an opaque run id. CSPRNG-backed hex, prefixed for readability.
fn new_run_id() -> String {
    match crate::pairing::random_token_hex(12) {
        Ok(hex) => format!("run_{hex}"),
        // A CSPRNG failure is astronomically unlikely; fall back to a timestamp
        // so submission never panics. (Run ids are not security tokens; the
        // device token already gated the request.)
        Err(_) => format!("run_{}", now_ms()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{TIER_THREE, TIER_TWO};
    use std::time::Duration;

    /// Poll a predicate until it holds or a bound elapses, yielding to the tokio
    /// runtime between checks so spawned blocking tasks make progress. Returns
    /// the final value (so the caller can assert with a useful message).
    async fn wait_until<F>(registry: &RunRegistry, run_id: &str, mut done: F) -> Option<RunRecord>
    where
        F: FnMut(&RunRecord) -> bool,
    {
        let bound = Duration::from_secs(10);
        let step = Duration::from_millis(10);
        let mut waited = Duration::ZERO;
        loop {
            if let Some(record) = registry.record(run_id) {
                if done(&record) {
                    return Some(record);
                }
            }
            if waited >= bound {
                return registry.record(run_id);
            }
            tokio::time::sleep(step).await;
            waited += step;
        }
    }

    #[tokio::test]
    async fn tier_one_run_streams_trace_obligation_diff_to_completion() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        // Subscribe is per-run, so submit first, then immediately read the record
        // and subscribe; the backlog covers anything emitted before we attach.
        let run_id = registry.submit(RunSpec::tier_one("explain main.rs"));

        let record = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run record exists");
        assert_eq!(record.state, RunState::Done, "a tier-1 run completes");

        // The three first-class stream kinds all appear in order.
        let kinds: Vec<RunEventKind> = record
            .events
            .iter()
            .map(|event| event.kind)
            .filter(|kind| *kind != RunEventKind::Status)
            .collect();
        assert_eq!(
            kinds,
            vec![
                RunEventKind::Trace,
                RunEventKind::Obligation,
                RunEventKind::Diff
            ],
            "the run streams Trace, Obligation, Diff"
        );
        // Sequence numbers are monotonic and contiguous.
        for (index, event) in record.events.iter().enumerate() {
            assert_eq!(event.seq, index as u64, "event seq is contiguous");
        }
        // The lifecycle reached Running then Done via Status events.
        let status_bodies: Vec<&str> = record
            .events
            .iter()
            .filter(|e| e.kind == RunEventKind::Status)
            .map(|e| e.body.as_str())
            .collect();
        assert!(status_bodies.contains(&"running"));
        assert!(status_bodies.contains(&"done"));
    }

    #[tokio::test]
    async fn live_subscriber_receives_streamed_events() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        let run_id = registry.submit(RunSpec::tier_one("trace me"));
        let (_backlog, mut rx) = registry.subscribe(&run_id).expect("run exists");

        // Collect events off the broadcast channel until the channel sees Done.
        let mut saw_diff = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
                Ok(Ok(event)) => {
                    if event.kind == RunEventKind::Diff {
                        saw_diff = true;
                    }
                    if event.kind == RunEventKind::Status && event.body == "done" {
                        break;
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(_))) => continue,
                Ok(Err(broadcast::error::RecvError::Closed)) => break,
                Err(_timeout) => continue,
            }
        }
        assert!(saw_diff, "a live subscriber receives the streamed Diff event");
    }

    #[tokio::test]
    async fn tier_three_run_holds_until_approved_and_does_not_run_gated_action() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        let run_id = registry.submit(RunSpec::at_tier("commit and push", TIER_THREE));

        // It parks in AwaitingAuthorization and emits NO executor events (no
        // Trace/Obligation/Diff) -- the gated action has not run.
        let held = wait_until(&registry, &run_id, |r| {
            r.state == RunState::AwaitingAuthorization
        })
        .await
        .expect("run exists");
        assert_eq!(held.state, RunState::AwaitingAuthorization);
        let executor_events = held
            .events
            .iter()
            .filter(|e| e.kind != RunEventKind::Status)
            .count();
        assert_eq!(
            executor_events, 0,
            "a held tier-3 run must NOT execute its gated action before approval"
        );

        // Approve releases it; now it runs to completion.
        registry.approve(&run_id).expect("approve a held run");
        let done = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run exists");
        assert_eq!(done.state, RunState::Done, "an approved run completes");
        assert!(
            done.events.iter().any(|e| e.kind == RunEventKind::Diff),
            "the gated action runs only after approval"
        );

        // Re-approving a now-terminal run is an InvalidState error.
        assert!(matches!(
            registry.approve(&run_id),
            Err(RunError::InvalidState { .. })
        ));
    }

    #[tokio::test]
    async fn tier_three_with_upfront_authorization_runs_immediately() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        let mut spec = RunSpec::at_tier("commit", TIER_TWO);
        spec.human_authorized = true;
        let run_id = registry.submit(spec);
        let done = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run exists");
        assert_eq!(
            done.state,
            RunState::Done,
            "a pre-authorized consequential run does not hold"
        );
    }

    #[tokio::test]
    async fn stop_halts_an_in_flight_run() {
        // A pausing mock holds after its first event until cancelled, so the stop
        // lands while the run is genuinely in flight.
        let registry = RunRegistry::new(Arc::new(MockExecutor::pausing_after_first_event()));
        let run_id = registry.submit(RunSpec::tier_one("long task"));

        // Wait until it is Running (the first Trace has been emitted).
        wait_until(&registry, &run_id, |r| r.state == RunState::Running)
            .await
            .expect("run exists");

        // Stop it; the cooperative cancel makes the executor return Stopped.
        registry.stop(&run_id).expect("stop an in-flight run");
        let stopped = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run exists");
        assert_eq!(
            stopped.state,
            RunState::Stopped,
            "stop halts the in-flight run cooperatively"
        );
        // It stopped before emitting the Diff (the last scripted step).
        assert!(
            !stopped.events.iter().any(|e| e.kind == RunEventKind::Diff),
            "a stopped run does not complete its remaining work"
        );

        // Stopping an already-terminal run is an InvalidState error.
        assert!(matches!(
            registry.stop(&run_id),
            Err(RunError::InvalidState { .. })
        ));
    }

    #[tokio::test]
    async fn stop_a_held_run_never_runs_the_gated_action() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        let run_id = registry.submit(RunSpec::at_tier("delete prod", TIER_THREE));
        wait_until(&registry, &run_id, |r| {
            r.state == RunState::AwaitingAuthorization
        })
        .await
        .expect("run exists");

        registry.stop(&run_id).expect("stop a held run");
        let stopped = registry.record(&run_id).expect("run exists");
        assert_eq!(stopped.state, RunState::Stopped);
        assert_eq!(
            stopped
                .events
                .iter()
                .filter(|e| e.kind != RunEventKind::Status)
                .count(),
            0,
            "stopping a held run must never run the gated action"
        );
    }

    #[tokio::test]
    async fn redirect_injects_an_instruction_picked_up_by_the_executor() {
        // Pause after the first event so the redirect is queued before the
        // executor drains redirects (which the mock does right after the pause).
        let registry = RunRegistry::new(Arc::new(MockExecutor::pausing_after_first_event()));
        let run_id = registry.submit(RunSpec::tier_one("initial plan"));
        wait_until(&registry, &run_id, |r| r.state == RunState::Running)
            .await
            .expect("run exists");

        registry
            .redirect(&run_id, "use the other module")
            .expect("redirect an in-flight run");
        // Release the pause by NOT cancelling: the mock waits up to its bound,
        // but we want it to proceed, so stop the wait by... letting it time out
        // is slow. Instead, the mock proceeds once cancelled is false AND the
        // bound elapses; to keep the test fast we assert the redirect is queued
        // and then visible as a Trace once drained. Cancel is NOT set, so the
        // mock completes after its (bounded) wait.
        let done = wait_until(&registry, &run_id, |r| r.state.is_terminal())
            .await
            .expect("run exists");
        assert_eq!(done.state, RunState::Done);
        assert!(
            done.events.iter().any(|e| {
                e.kind == RunEventKind::Trace && e.body.contains("redirected: use the other module")
            }),
            "the executor picks up the injected redirect"
        );

        // Redirecting a terminal run is an InvalidState error.
        assert!(matches!(
            registry.redirect(&run_id, "too late"),
            Err(RunError::InvalidState { .. })
        ));
    }

    #[tokio::test]
    async fn controls_on_unknown_run_are_not_found() {
        let registry = RunRegistry::new(Arc::new(MockExecutor::new()));
        assert_eq!(registry.approve("run_nope"), Err(RunError::NotFound));
        assert_eq!(registry.stop("run_nope"), Err(RunError::NotFound));
        assert_eq!(registry.redirect("run_nope", "x"), Err(RunError::NotFound));
        assert!(registry.record("run_nope").is_none());
        assert!(registry.subscribe("run_nope").is_none());
    }
}
