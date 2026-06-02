//! Node.js (NAPI-RS) binding over the `theorem-harness` Rust SDK.
//!
//! This is the THPS-012 binding: a thin idiomatic shell over the stabilized Rust
//! SDK surface, so the Node API cannot drift from the core. Every method here
//! delegates straight to `theorem-harness` (`RunHandle`, `RunStream`,
//! `IdempotencyToken`); there is no harness logic in this crate, only the FFI
//! marshalling. This is what retires the plugin's hand-written JS clients: the
//! plugin calls these native methods instead of re-implementing run logic in JS.
//!
//! Slice 1 binds the run lifecycle against an in-process [`InMemoryGraphStore`].
//! The production swap to a durable `RedCoreGraphStore` (constructed from a data
//! directory) is a one-type change to the field and constructor, because the SDK
//! surface is store-agnostic by design.

use std::sync::Mutex;

use napi_derive::napi;
use rustyred_thg_core::InMemoryGraphStore;
use theorem_harness::{export_run_trace, IdempotencyToken, RunHandle, RunStream, SdkError};
use theorem_harness_core::types::Payload;

/// A harness bound to an in-process graph store, exposed to Node.
///
/// The store is held behind a `Mutex` so the JS object can be shared and called
/// from the event loop while the SDK takes `&mut` for state-changing calls.
#[napi]
pub struct Harness {
    store: Mutex<InMemoryGraphStore>,
}

#[napi]
impl Harness {
    /// Create a harness over a fresh in-process store.
    #[napi(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            store: Mutex::new(InMemoryGraphStore::default()),
        }
    }

    /// Start a run and return its id. Mirrors `RunHandle::start`.
    #[napi]
    pub fn start_run(
        &self,
        task: String,
        actor: String,
        idempotency_key: String,
    ) -> napi::Result<String> {
        let mut store = self.lock()?;
        let run = RunHandle::start(
            &mut *store,
            task,
            actor,
            Payload::new(),
            IdempotencyToken::new(idempotency_key),
        )
        .map_err(to_napi)?;
        Ok(run.run_id().to_string())
    }

    /// Cancel a run. Mirrors `RunHandle::cancel`.
    #[napi]
    pub fn cancel(
        &self,
        run_id: String,
        reason: String,
        idempotency_key: String,
    ) -> napi::Result<()> {
        let mut store = self.lock()?;
        let run = RunHandle::attach(run_id, "node");
        run.cancel(&mut *store, reason, IdempotencyToken::new(idempotency_key))
            .map_err(to_napi)?;
        Ok(())
    }

    /// All events for a run as a JSON array string (one object per event:
    /// `{run_id, seq, kind, event_type, state_hash_after, payload}`). Returning
    /// JSON keeps slice 1 free of per-field napi object marshalling.
    #[napi]
    pub fn events_json(&self, run_id: String) -> napi::Result<String> {
        let store = self.lock()?;
        let run = RunHandle::attach(run_id, "node");
        let events = run.events(&*store).map_err(to_napi)?;
        serde_json::to_string(&export_run_trace(&events))
            .map_err(|error| napi::Error::from_reason(error.to_string()))
    }

    /// Drain the text view of a run from a sequence cursor, returning the new
    /// text. Mirrors `RunStream::resume_from(..).poll_text(..)`.
    #[napi]
    pub fn poll_text(&self, run_id: String, after_seq: u32) -> napi::Result<String> {
        let store = self.lock()?;
        let run = RunHandle::attach(run_id, "node");
        let mut stream = RunStream::resume_from(&run, u64::from(after_seq));
        stream.poll_text(&*store).map_err(to_napi)
    }

    fn lock(&self) -> napi::Result<std::sync::MutexGuard<'_, InMemoryGraphStore>> {
        self.store
            .lock()
            .map_err(|error| napi::Error::from_reason(error.to_string()))
    }
}

fn to_napi(error: SdkError) -> napi::Error {
    napi::Error::from_reason(error.to_string())
}
