//! theorem-receiver: the dispatch-queue receiver.
//!
//! This is the local half of the harness-local-session design (receiver note,
//! approved 2026-06-04). It is light and idle until pinged: it holds only an
//! outbound connection to the cloud harness, claims a job with a network call,
//! and spawns the locally-installed `claude` / `codex` CLI in a mapped worktree.
//!
//! What it is NOT:
//!   - It does NOT run the RustyRed engine locally: no vector index, no PPR, no
//!     BM25, no embedders (acceptance criterion 9, listener-scale footprint).
//!   - It opens NO inbound port and needs NO tunnel: outbound polling only.
//!   - It stores NO credentials and uses NO GitHub Actions, runners, PATs, or
//!     OAuth tokens. The `claude` / `codex` CLIs are already authenticated through
//!     the owner's own subscriptions; a local spawn needs no credential of its own.
//!     The harness bearer token is read from the environment at startup, not stored.
//!
//! Policy (baked here so it travels with the code):
//!   - From 2026-06-15, `claude -p` on a subscription draws from the separate,
//!     finite monthly Agent SDK credit bucket. The receiver logs a per-job usage
//!     line so the draw is measurable.
//!   - Solo use on the owner's own repos is sanctioned individual use. The moment
//!     a job belongs to another user it must execute on that user's own key; that
//!     is the shelved RunPod lane, never the personal subscription login. This
//!     receiver only claims repos present in its local worktree map.
//!
//! Bootstrap: the receiver itself is built by a hand-started session; the queue
//! cannot dispatch before the receiver exists.

pub mod client;
pub mod config;
pub mod head;
pub mod lanes;
pub mod local_exec;
pub mod receiver;
pub mod spawn;
pub mod wake;

pub use client::HarnessClient;
pub use config::ReceiverConfig;
pub use head::{adapter_for, head_adapters, HeadAdapter};
pub use lanes::detect_lanes;
pub use local_exec::{run_proof, ProofPlan, ProofReceipt, TRUST_TIER_LOCAL};
pub use receiver::{run_loop, run_loop_until, JobRunReport};
pub use spawn::{build_intent, build_spawn_plan, SpawnPlan};
pub use wake::{
    build_wake_dry_run_report, build_wake_prompt, run_wake_report_with_spawner, spawn_wake_command,
    wake_dry_run_report_json, wake_run_report_json, WakeCommandPlan, WakeDryRunReport,
    WakeLaunchFailed, WakeLedger, WakeMessage, WakeRunReport, WakeSkipped, WakeSpawnOutcome,
    WakeSpawned, DEFAULT_WAKE_MAX_PLANS, DEFAULT_WAKE_MESSAGE_LIMIT,
};

use std::fmt;

/// Every failure mode of the receiver.
#[derive(Debug)]
pub enum ReceiverError {
    Config(String),
    Io(std::io::Error),
    Http(String),
    Protocol(String),
    Json(serde_json::Error),
}

impl fmt::Display for ReceiverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Http(message) => write!(f, "http error: {message}"),
            Self::Protocol(message) => write!(f, "protocol error: {message}"),
            Self::Json(error) => write!(f, "json error: {error}"),
        }
    }
}

impl std::error::Error for ReceiverError {}

impl From<std::io::Error> for ReceiverError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for ReceiverError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<reqwest::Error> for ReceiverError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.to_string())
    }
}

pub type ReceiverResult<T> = Result<T, ReceiverError>;
