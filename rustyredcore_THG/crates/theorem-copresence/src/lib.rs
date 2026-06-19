//! Headless co-presence over the RustyRed substrate.
//!
//! Structure flows through the graph CRDT and the THG executor command path,
//! free text lives in Yrs text regions, and awareness rides the working log.

pub mod adapter;
pub mod adapters;
pub mod peer;
pub mod presence;
pub mod text_region;

pub use adapter::{
    InSubstrateAdapter, InstrumentAdapter, SurfaceAdapter, SurfaceIntent, SurfaceSnapshot,
};
pub use adapters::note::{NoteAdapter, NoteIntent, NoteSectionSnapshot, NoteSnapshot};
pub use peer::{PeerConfig, PeerEvent, SharedWorkingLog, StructuredOp, SubstratePeer};
pub use presence::{CursorPos, Presence, PresenceKind};
pub use text_region::{TextRegionHandle, TextRegionUpdate};

pub type CoResult<T> = Result<T, CoError>;

#[derive(Debug, thiserror::Error)]
pub enum CoError {
    #[error("graph store error {code}: {message}")]
    GraphStore { code: String, message: String },
    #[error("executor command {command} failed with {status}: {detail}")]
    Executor {
        command: String,
        status: String,
        detail: String,
    },
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
    #[error("yrs error: {0}")]
    Yrs(String),
    #[error("lock poisoned: {0}")]
    Lock(&'static str),
    #[error("invalid copresence operation: {0}")]
    Invalid(String),
}

impl From<rustyred_thg_core::GraphStoreError> for CoError {
    fn from(error: rustyred_thg_core::GraphStoreError) -> Self {
        Self::GraphStore {
            code: error.code,
            message: error.message,
        }
    }
}
