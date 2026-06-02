//! # theorem-harness
//!
//! The idiomatic Rust SDK surface over [`theorem_harness_core`] (the pure run
//! state machine, guards, hashes, and contracts) and [`theorem_harness_runtime`]
//! (GraphStore-backed persistence: the durable event log and run state).
//!
//! This crate is the SDK v2 source of truth. The bindings for Python and Swift
//! (UniFFI), Node (NAPI-RS), and the browser (wasm-bindgen) are GENERATED from
//! this surface; there is no hand-maintained per-language SDK contract that can
//! drift. Stabilizing this crate is the THPS-011 deliverable that the binding
//! generation (THPS-012) waits on.
//!
//! ## Surface (the THPS-011 freeze)
//!
//! - [`RunHandle`]: a run as a sequence of typed events. Start it, append
//!   transitions, read its events, resume from a sequence boundary, replay it
//!   deterministically, and cancel it.
//! - [`RunStream`]: a resumable, poll-based cursor over a run's typed events,
//!   with a typed view and a text-projection view. The synchronous core that
//!   each binding wraps into a language-native async stream.
//! - [`Session`]: a continuity handle over an AgentBinding scope. Within-session
//!   working memory is the versioned scratchpad; state published across sessions
//!   is the committed graph.
//! - [`Event`] / [`RunEventKind`]: the typed view of the canonical transition
//!   log, carrying its sequence number and post-transition state hash.
//! - [`IdempotencyToken`]: a client-provided token threaded onto every
//!   state-changing call.
//! - [`CancelToken`]: a runtime-free polled flag the run loop checks before each
//!   append.
//! - [`export_run_trace`]: a run's events exported as training rows.
//!
//! ## Runtime-neutral discipline
//!
//! This crate composes core + runtime, both synchronous and GraphStore-backed
//! (no network, no async runtime). The live async push-stream (subscribe and
//! await new events) is intentionally NOT in this crate: it belongs to the
//! binding layer (NAPI's tokio, UniFFI's RustFuture polling), so the core
//! surface never drags an async runtime across the FFI boundary. The resumable
//! primitive that IS here is [`RunHandle::events_since`]: a synchronous bounded
//! replay from a sequence number that every binding wraps into its own stream.
//!
//! ## Status of the freeze
//!
//! Real and implemented this slice: cancellation flag, idempotency token
//! threading, typed events, sessions over binding scopes, the run handle over
//! the durable event log (start / append / events / events_since / state /
//! replay / cancel), and trace export. Deferred and named, not stubbed: the live
//! async push-stream (binding layer), idempotency short-circuit ENFORCEMENT
//! (a runtime delta, THPS-003: the event log must persist the token to detect a
//! repeat), and richer SFT/preference export shaping (THPS-013).

pub mod cancel;
pub mod event;
pub mod export;
pub mod idempotency;
pub mod run;
pub mod session;
pub mod stream;

pub use cancel::CancelToken;
pub use event::{Event, RunEventKind};
pub use export::{export_run_trace, TraceRow};
pub use idempotency::IdempotencyToken;
pub use run::{RunHandle, SdkError, SdkResult};
pub use session::Session;
pub use stream::RunStream;
