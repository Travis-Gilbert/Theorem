//! The run surface: a run as a sequence of typed events over a GraphStore.

use rustyred_thg_core::GraphStore;
use serde_json::Value;
use theorem_harness_core::types::{Payload, RunState};
use theorem_harness_core::TransitionInput;
use theorem_harness_runtime::{
    append_transition_from_store, load_events, load_run, replay_persisted_run, HarnessRuntimeError,
    MemoryError,
};

use crate::cancel::CancelToken;
use crate::event::Event;
use crate::idempotency::IdempotencyToken;

/// Errors from the SDK run surface.
#[derive(Debug)]
pub enum SdkError {
    /// A failure in the GraphStore-backed runtime: a guard violation, an append
    /// conflict, a missing run, or a persistence error.
    Runtime(HarnessRuntimeError),
    /// A failure in the memory subsystem (remember / recall / encode).
    Memory(MemoryError),
    /// The run was cancelled; the operation refused to append without touching
    /// the store.
    Cancelled,
}

impl std::fmt::Display for SdkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SdkError::Runtime(error) => write!(f, "harness runtime error: {error:?}"),
            SdkError::Memory(error) => write!(f, "harness memory error: {error:?}"),
            SdkError::Cancelled => write!(f, "run was cancelled"),
        }
    }
}

impl std::error::Error for SdkError {}

impl From<HarnessRuntimeError> for SdkError {
    fn from(error: HarnessRuntimeError) -> Self {
        SdkError::Runtime(error)
    }
}

impl From<MemoryError> for SdkError {
    fn from(error: MemoryError) -> Self {
        SdkError::Memory(error)
    }
}

/// Result alias for the SDK run surface.
pub type SdkResult<T> = Result<T, SdkError>;

/// A run as a sequence of typed events.
///
/// The handle is store-agnostic: every method takes the `GraphStore`, so the
/// same handle drives an in-memory store in tests and a durable RedCore store in
/// production. Cancellation is a polled flag checked before each append, so a
/// long-running plan can be stopped cleanly between transitions.
#[derive(Clone, Debug)]
pub struct RunHandle {
    run_id: String,
    actor: String,
    cancel: CancelToken,
}

impl RunHandle {
    /// Start a new run by appending `RUN.CREATED`, returning its handle.
    ///
    /// `RUN.CREATED` requires `task` and `actor`; the `scope` is carried as a
    /// nested payload object for the run's initial scope.
    pub fn start<S: GraphStore>(
        store: &mut S,
        task: impl Into<String>,
        actor: impl Into<String>,
        scope: Payload,
        idempotency: IdempotencyToken,
    ) -> SdkResult<Self> {
        let actor = actor.into();
        let mut payload = Payload::new();
        payload.insert("task".to_string(), Value::String(task.into()));
        payload.insert("actor".to_string(), Value::String(actor.clone()));
        payload.insert("scope".to_string(), Value::Object(scope));
        let mut transition = TransitionInput::new("RUN.CREATED", payload);
        transition.actor = actor.clone();
        transition.idempotency_key = idempotency.into_string();
        let result = append_transition_from_store(store, transition)?;
        Ok(Self {
            run_id: result.run.run_id,
            actor,
            cancel: CancelToken::new(),
        })
    }

    /// Attach a handle to an existing run id, for resuming a run started in an
    /// earlier session or process.
    pub fn attach(run_id: impl Into<String>, actor: impl Into<String>) -> Self {
        Self {
            run_id: run_id.into(),
            actor: actor.into(),
            cancel: CancelToken::new(),
        }
    }

    /// The run id.
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    /// A cancel handle sharing this run's flag; hand it to a worker to let it
    /// request cancellation.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.handle()
    }

    /// Idempotent-retry lookup: if a non-empty `token` already produced an event
    /// on this run, return it. Empty tokens never match, so they never
    /// short-circuit. Backed by the `idempotency_key` THPS-003 persists on every
    /// event.
    fn event_for_token<S: GraphStore>(&self, store: &S, token: &str) -> SdkResult<Option<Event>> {
        if token.is_empty() {
            return Ok(None);
        }
        Ok(load_events(store, &self.run_id)?
            .into_iter()
            .find(|event| event.idempotency_key == token)
            .map(Event::new))
    }

    /// Append a transition to the run and return the typed event.
    ///
    /// Returns [`SdkError::Cancelled`] without touching the store if the run has
    /// already been cancelled.
    pub fn append<S: GraphStore>(
        &self,
        store: &mut S,
        event_type: impl Into<String>,
        payload: Payload,
        idempotency: IdempotencyToken,
    ) -> SdkResult<Event> {
        // Idempotent retry: a seen token returns its prior event without
        // re-appending, so a flaky-network retry never double-writes.
        if let Some(existing) = self.event_for_token(store, idempotency.as_str())? {
            return Ok(existing);
        }
        if self.cancel.is_cancelled() {
            return Err(SdkError::Cancelled);
        }
        let mut transition =
            TransitionInput::new(event_type, payload).with_run_id(self.run_id.clone());
        transition.actor = self.actor.clone();
        transition.idempotency_key = idempotency.into_string();
        let result = append_transition_from_store(store, transition)?;
        Ok(Event::new(result.event))
    }

    /// All events for the run, in sequence order.
    pub fn events<S: GraphStore>(&self, store: &S) -> SdkResult<Vec<Event>> {
        Ok(load_events(store, &self.run_id)?
            .into_iter()
            .map(Event::new)
            .collect())
    }

    /// Events with `seq` strictly greater than `after_seq`: the
    /// resumable-streaming primitive. A reconnecting client passes the last
    /// sequence it observed and receives a bounded replay of everything after
    /// it. The richer live push-stream is the binding layer's job; this is the
    /// synchronous core it wraps.
    pub fn events_since<S: GraphStore>(&self, store: &S, after_seq: u64) -> SdkResult<Vec<Event>> {
        Ok(load_events(store, &self.run_id)?
            .into_iter()
            .filter(|event| event.seq > after_seq)
            .map(Event::new)
            .collect())
    }

    /// The current persisted run state.
    pub fn state<S: GraphStore>(&self, store: &S) -> SdkResult<Option<RunState>> {
        Ok(load_run(store, &self.run_id)?)
    }

    /// Replay the run deterministically from its event log.
    pub fn replay<S: GraphStore>(&self, store: &S) -> SdkResult<Option<RunState>> {
        Ok(replay_persisted_run(store, &self.run_id)?)
    }

    /// Cancel the run: set the polled flag and append a clean `RUN.CANCELLED`.
    ///
    /// Unlike [`RunHandle::append`], cancellation is allowed even though it sets
    /// the flag, so a run can always be stopped. `RUN.CANCELLED` requires
    /// `reason` and `cancelled_by`; the latter is filled from the handle's
    /// actor.
    pub fn cancel<S: GraphStore>(
        &self,
        store: &mut S,
        reason: impl Into<String>,
        idempotency: IdempotencyToken,
    ) -> SdkResult<Event> {
        self.cancel.cancel();
        // Idempotent retry: a seen token returns the prior cancellation event.
        if let Some(existing) = self.event_for_token(store, idempotency.as_str())? {
            return Ok(existing);
        }
        let mut payload = Payload::new();
        payload.insert("reason".to_string(), Value::String(reason.into()));
        payload.insert(
            "cancelled_by".to_string(),
            Value::String(self.actor.clone()),
        );
        let mut transition =
            TransitionInput::new("RUN.CANCELLED", payload).with_run_id(self.run_id.clone());
        transition.actor = self.actor.clone();
        transition.idempotency_key = idempotency.into_string();
        let result = append_transition_from_store(store, transition)?;
        Ok(Event::new(result.event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::RunEventKind;
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn start_read_resume_replay_then_cancel() {
        let mut store = InMemoryGraphStore::default();

        let run = RunHandle::start(
            &mut store,
            "demo task",
            "claude-code",
            Payload::new(),
            IdempotencyToken::new("k-create"),
        )
        .expect("run starts");
        assert!(!run.run_id().is_empty());

        // The created event is present and typed.
        let all = run.events(&store).expect("events load");
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].kind(), RunEventKind::Created);
        let created_seq = all[0].seq();

        // resume-from-seq returns nothing past the latest event.
        assert!(run
            .events_since(&store, created_seq)
            .expect("events_since")
            .is_empty());
        // resume from 0 returns the created event.
        assert_eq!(run.events_since(&store, 0).expect("events_since").len(), 1);

        // The run state and a deterministic replay both reconstruct.
        assert!(run.state(&store).expect("state").is_some());
        assert!(run.replay(&store).expect("replay").is_some());

        // Cancel drives RUN.CANCELLED and flips the shared flag.
        let token = run.cancel_token();
        let cancelled = run
            .cancel(
                &mut store,
                "user stopped",
                IdempotencyToken::new("k-cancel"),
            )
            .expect("cancel appends");
        assert_eq!(cancelled.kind(), RunEventKind::Cancelled);
        assert!(token.is_cancelled());

        // After cancellation, appends are refused before the store is touched.
        let refused = run.append(
            &mut store,
            "OUTCOME.RECORDED",
            Payload::new(),
            IdempotencyToken::new("k-after"),
        );
        assert!(matches!(refused, Err(SdkError::Cancelled)));
    }

    #[test]
    fn attach_reads_an_existing_run() {
        let mut store = InMemoryGraphStore::default();
        let started = RunHandle::start(
            &mut store,
            "task",
            "codex",
            Payload::new(),
            IdempotencyToken::new("k"),
        )
        .expect("start");

        let attached = RunHandle::attach(started.run_id(), "codex");
        let events = attached.events(&store).expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind(), RunEventKind::Created);
    }

    #[test]
    fn idempotent_retry_returns_prior_event_not_a_duplicate() {
        let mut store = InMemoryGraphStore::default();
        let run = RunHandle::start(
            &mut store,
            "task",
            "claude-code",
            Payload::new(),
            IdempotencyToken::new("k-create"),
        )
        .expect("start");

        let first = run
            .cancel(&mut store, "stop", IdempotencyToken::new("k-cancel"))
            .expect("cancel");
        // A retry with the SAME token returns the prior event, not a second append.
        let retry = run
            .cancel(&mut store, "stop again", IdempotencyToken::new("k-cancel"))
            .expect("cancel retry");
        assert_eq!(first.seq(), retry.seq());

        // Exactly one cancellation event exists.
        let cancels = run
            .events(&store)
            .expect("events")
            .into_iter()
            .filter(|event| event.kind() == RunEventKind::Cancelled)
            .count();
        assert_eq!(cancels, 1);
    }
}
