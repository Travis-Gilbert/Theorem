//! Swift (UniFFI) binding over the `theorem-harness` Rust SDK.
//!
//! This is the second-language proof of the SDK v2 bet: the Node binding
//! (`apps/theorem-harness-node`, NAPI-RS) and this Swift binding (UniFFI) wrap
//! the *same* `theorem-harness` Rust SDK with *different* generators. Neither
//! re-implements harness logic; both are thin facades over the one Rust core, so
//! the Swift API cannot drift from the Node API or the core.
//!
//! UniFFI cannot export generics, so (like the Node binding) the facade picks a
//! concrete store: a durable [`RedCoreGraphStore`] opened from a data directory.
//! The harness logic is store-agnostic, so this is the only place the store type
//! appears.

use std::sync::{Arc, Mutex, MutexGuard};

use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use theorem_harness::{export_run_trace, IdempotencyToken, RunHandle, SdkError, Session};
use theorem_harness_core::types::Payload;

uniffi::setup_scaffolding!();

/// Errors surfaced across the Swift boundary.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum HarnessError {
    /// A harness operation failed; `message` carries the detail.
    #[error("{message}")]
    Failed { message: String },
}

impl HarnessError {
    fn msg(text: impl Into<String>) -> Self {
        HarnessError::Failed {
            message: text.into(),
        }
    }
}

impl From<SdkError> for HarnessError {
    fn from(error: SdkError) -> Self {
        HarnessError::msg(error.to_string())
    }
}

/// A harness bound to a durable RedCore graph store, exposed to Swift.
///
/// UniFFI objects are reference-counted and shared (`Arc`), so the store sits
/// behind a `Mutex` for the SDK's `&mut` state-changing calls.
#[derive(uniffi::Object)]
pub struct Harness {
    store: Mutex<RedCoreGraphStore>,
}

#[uniffi::export]
impl Harness {
    /// Open a harness over a durable RedCore store at `data_dir` (AOF-backed,
    /// recovered on open).
    #[uniffi::constructor]
    pub fn new(data_dir: String) -> Result<Arc<Self>, HarnessError> {
        let store = RedCoreGraphStore::open(data_dir, RedCoreOptions::default())
            .map_err(|error| HarnessError::msg(format!("{error:?}")))?;
        Ok(Arc::new(Self {
            store: Mutex::new(store),
        }))
    }

    /// Start a run and return its id. Mirrors `RunHandle::start`.
    pub fn start_run(
        &self,
        task: String,
        actor: String,
        idempotency_key: String,
    ) -> Result<String, HarnessError> {
        let mut store = self.lock()?;
        let run = RunHandle::start(
            &mut *store,
            task,
            actor,
            Payload::new(),
            IdempotencyToken::new(idempotency_key),
        )?;
        Ok(run.run_id().to_string())
    }

    /// Cancel a run. Mirrors `RunHandle::cancel`.
    pub fn cancel(
        &self,
        run_id: String,
        reason: String,
        idempotency_key: String,
    ) -> Result<(), HarnessError> {
        let mut store = self.lock()?;
        let run = RunHandle::attach(run_id, "swift");
        run.cancel(&mut *store, reason, IdempotencyToken::new(idempotency_key))?;
        Ok(())
    }

    /// All events for a run as a JSON array string. Mirrors `RunHandle::events`.
    pub fn events_json(&self, run_id: String) -> Result<String, HarnessError> {
        let store = self.lock()?;
        let run = RunHandle::attach(run_id, "swift");
        let events = run.events(&*store)?;
        serde_json::to_string(&export_run_trace(&events))
            .map_err(|error| HarnessError::msg(error.to_string()))
    }

    /// The current status of a run, or `unknown` if not found.
    pub fn run_status(&self, run_id: String) -> Result<String, HarnessError> {
        let store = self.lock()?;
        let run = RunHandle::attach(run_id, "swift");
        Ok(run
            .state(&*store)?
            .map(|state| state.status)
            .unwrap_or_else(|| "unknown".to_string()))
    }

    /// Save a durable memory for `agent_id`, returning the receipt as JSON.
    /// Mirrors `Session::remember`.
    pub fn remember(
        &self,
        agent_id: String,
        kind: String,
        title: String,
        content: String,
    ) -> Result<String, HarnessError> {
        let mut store = self.lock()?;
        let session = Session::open(agent_id);
        let receipt = session.remember(&mut *store, kind, title, content)?;
        serde_json::to_string(&receipt).map_err(|error| HarnessError::msg(error.to_string()))
    }

    /// Recall memories matching `query` for `agent_id`, as a JSON array string.
    /// Mirrors `Session::recall`.
    pub fn recall(
        &self,
        agent_id: String,
        query: String,
        limit: u32,
    ) -> Result<String, HarnessError> {
        let mut store = self.lock()?;
        let session = Session::open(agent_id);
        let hits = session.recall(&mut *store, query, limit as usize)?;
        serde_json::to_string(&hits).map_err(|error| HarnessError::msg(error.to_string()))
    }
}

impl Harness {
    fn lock(&self) -> Result<MutexGuard<'_, RedCoreGraphStore>, HarnessError> {
        self.store
            .lock()
            .map_err(|error| HarnessError::msg(error.to_string()))
    }
}
