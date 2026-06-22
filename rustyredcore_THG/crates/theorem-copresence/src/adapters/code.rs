//! Code surface adapter (W5): copresence over a code file.
//!
//! A code file collaborates through git (W2) plus a presence layer, never a
//! character CRDT (the measured semantic-conflict and code-quality cost rules
//! it out). So this adapter carries awareness (who is on this file, at which
//! line and column) and a lightweight structural footprint, but it REFUSES to
//! move file content through a yrs text region: a text-insert/push intent is an
//! error. The file bytes flow through git and the materialized working tree;
//! copresence only says who is where.
//!
//! Plan: docs/plans/rustyred-code-workspace/W5-collaboration-adapter.md

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapter::{InSubstrateAdapter, SurfaceAdapter, SurfaceIntent, SurfaceSnapshot};
use crate::peer::{StructuredOp, SubstratePeer};
use crate::{CoError, CoResult};

/// What copresence structurally knows about a code file: its path and whether a
/// `File` node footprint exists in the peer's graph. The file BYTES are
/// intentionally absent: code is versioned by git, not CRDT-merged here.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeFileSnapshot {
    pub path: String,
    pub node_present: bool,
}

/// A copresence surface bound to a single code file.
#[derive(Clone, Debug)]
pub struct CodeSurfaceAdapter {
    path: String,
}

impl CodeSurfaceAdapter {
    pub fn new(path: impl Into<String>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &str {
        &self.path
    }

    /// The `File` node id this adapter footprints (the same `file:{path}` shape
    /// the embedded engine's `fs_write` uses, so a code presence and an imported
    /// file refer to the same node).
    pub fn file_node_id(&self) -> String {
        format!("file:{}", self.path)
    }

    /// A structural footprint op for this file: record `key = value` on the
    /// `File` node (durable metadata like who has it open or last touched it).
    /// This is structure, synced peer-to-peer by the graph CRDT; it is never
    /// file content. Pass it through `to_peer` via `SurfaceIntent::Structured`.
    pub fn footprint_op(&self, key: impl Into<String>, value: Value) -> StructuredOp {
        StructuredOp::SetObjectProperty {
            object_id: self.file_node_id(),
            labels: vec!["File".to_string(), "CodeFile".to_string()],
            key: key.into(),
            value,
        }
    }
}

impl SurfaceAdapter for CodeSurfaceAdapter {
    fn to_peer(&mut self, peer: &mut SubstratePeer, intent: SurfaceIntent) -> CoResult<()> {
        match intent {
            // Awareness: who is on this file, at file:line:col.
            SurfaceIntent::Presence { presence } => peer.announce(presence),
            // A structural footprint (a file-open marker, an edit footprint),
            // handled by the graph CRDT. This is metadata, never file content.
            SurfaceIntent::Structured { op } => {
                peer.apply_structured(op)?;
                Ok(())
            }
            // The boundary, made executable: code is NOT CRDT-merged through
            // copresence. File bytes are versioned by git (W2) and materialized
            // to the working tree (W3); they never enter a yrs text region.
            SurfaceIntent::TextInsert { .. } | SurfaceIntent::TextPush { .. } => {
                Err(CoError::Invalid(
                    "code content is not CRDT-merged through copresence; it is \
                     versioned by git and materialized to the working tree"
                        .to_string(),
                ))
            }
            SurfaceIntent::Note { .. } => Err(CoError::Invalid(
                "a note intent is not a code-surface intent".to_string(),
            )),
        }
    }

    fn from_peer(&mut self, peer: &SubstratePeer) -> CoResult<SurfaceSnapshot> {
        // The trait hands us an immutable peer (presence is drained via
        // peer.observe, which needs &mut), so from_peer reports the structural
        // footprint only. The absence of file bytes here is the point: code
        // lives in git, not the graph.
        let node_present = peer.graph_node(&self.file_node_id()).is_some();
        Ok(SurfaceSnapshot::Code {
            snapshot: CodeFileSnapshot {
                path: self.path.clone(),
                node_present,
            },
        })
    }
}

impl InSubstrateAdapter for CodeSurfaceAdapter {}
