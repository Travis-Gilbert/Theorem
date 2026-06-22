use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::ActorId;

use crate::adapter::{InSubstrateAdapter, SurfaceAdapter, SurfaceIntent, SurfaceSnapshot};
use crate::peer::{PeerEvent, StructuredOp, SubstratePeer};
use crate::presence::{CursorPos, Presence, PresenceKind};
use crate::{CoError, CoResult};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FileRange {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

impl FileRange {
    pub fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Self {
        Self {
            start_line,
            start_col,
            end_line,
            end_col,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum CodeIntent {
    AnnouncePresence {
        line: u32,
        col: u32,
        label: String,
        kind: PresenceKind,
        pending_edit: Option<FileRange>,
    },
    SetPendingEdit {
        range: FileRange,
        summary: Option<String>,
    },
    ClearPendingEdit,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeContentStrategy {
    GitMergeOnly,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodePresenceSnapshot {
    pub actor: ActorId,
    pub path: String,
    pub line: u32,
    pub col: u32,
    pub label: String,
    pub kind: PresenceKind,
    pub cursor: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeEditFootprint {
    pub actor: ActorId,
    pub path: String,
    pub range: FileRange,
    pub summary: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeSnapshot {
    pub path: String,
    pub presences: Vec<CodePresenceSnapshot>,
    pub edit_footprints: Vec<CodeEditFootprint>,
    pub content_strategy: CodeContentStrategy,
}

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

    fn announce_presence(
        &self,
        peer: &mut SubstratePeer,
        line: u32,
        col: u32,
        label: String,
        kind: PresenceKind,
        pending_edit: Option<FileRange>,
    ) -> CoResult<()> {
        peer.announce(Presence {
            actor: peer.actor(),
            scope: peer.scope().to_string(),
            focus_region: Some(file_node_id(&self.path)),
            cursor: Some(CursorPos::FilePosition {
                path: self.path.clone(),
                line,
                col,
            }),
            label,
            kind,
        })?;
        if let Some(range) = pending_edit {
            self.set_pending_edit(peer, range, None)?;
        }
        Ok(())
    }

    fn set_pending_edit(
        &self,
        peer: &mut SubstratePeer,
        range: FileRange,
        summary: Option<String>,
    ) -> CoResult<()> {
        peer.apply_structured(StructuredOp::SetObjectProperty {
            object_id: edit_node_id(peer.actor(), &self.path),
            labels: vec!["CodeEditFootprint".to_string()],
            key: "footprint".to_string(),
            value: json!({
                "active": true,
                "actor": peer.actor(),
                "path": self.path,
                "range": range,
                "summary": summary,
            }),
        })?;
        Ok(())
    }

    fn clear_pending_edit(&self, peer: &mut SubstratePeer) -> CoResult<()> {
        peer.apply_structured(StructuredOp::SetObjectProperty {
            object_id: edit_node_id(peer.actor(), &self.path),
            labels: vec!["CodeEditFootprint".to_string()],
            key: "footprint".to_string(),
            value: json!({
                "active": false,
                "actor": peer.actor(),
                "path": self.path,
            }),
        })?;
        Ok(())
    }

    fn snapshot(&self, peer: &SubstratePeer) -> CoResult<CodeSnapshot> {
        let mut presences_by_actor: BTreeMap<ActorId, CodePresenceSnapshot> = BTreeMap::new();
        for event in peer.observe(0)? {
            let PeerEvent::Presence { cursor, presence } = event else {
                continue;
            };
            let Some(CursorPos::FilePosition { path, line, col }) = presence.cursor.clone() else {
                continue;
            };
            if path != self.path {
                continue;
            }
            let snapshot = CodePresenceSnapshot {
                actor: presence.actor,
                path,
                line,
                col,
                label: presence.label,
                kind: presence.kind,
                cursor,
            };
            let replace = presences_by_actor
                .get(&snapshot.actor)
                .map(|existing| existing.cursor < snapshot.cursor)
                .unwrap_or(true);
            if replace {
                presences_by_actor.insert(snapshot.actor, snapshot);
            }
        }
        let mut presences: Vec<_> = presences_by_actor.into_values().collect();
        presences.sort_by_key(|presence| presence.cursor);

        let mut edit_footprints = Vec::new();
        for node in peer.graph_snapshot()?.nodes {
            if !node.labels.iter().any(|label| label == "CodeEditFootprint") {
                continue;
            }
            let Some(value) = node.properties.get("footprint") else {
                continue;
            };
            if !value
                .get("active")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                continue;
            }
            if value.get("path").and_then(Value::as_str) != Some(self.path.as_str()) {
                continue;
            }
            edit_footprints.push(serde_json::from_value::<CodeEditFootprint>(value.clone())?);
        }
        edit_footprints.sort_by(|a, b| a.actor.cmp(&b.actor).then_with(|| a.path.cmp(&b.path)));

        Ok(CodeSnapshot {
            path: self.path.clone(),
            presences,
            edit_footprints,
            content_strategy: CodeContentStrategy::GitMergeOnly,
        })
    }
}

impl SurfaceAdapter for CodeSurfaceAdapter {
    fn to_peer(&mut self, peer: &mut SubstratePeer, intent: SurfaceIntent) -> CoResult<()> {
        match intent {
            SurfaceIntent::Code { intent } => match intent {
                CodeIntent::AnnouncePresence {
                    line,
                    col,
                    label,
                    kind,
                    pending_edit,
                } => self.announce_presence(peer, line, col, label, kind, pending_edit),
                CodeIntent::SetPendingEdit { range, summary } => {
                    self.set_pending_edit(peer, range, summary)
                }
                CodeIntent::ClearPendingEdit => self.clear_pending_edit(peer),
            },
            SurfaceIntent::Presence { presence } => match presence.cursor {
                Some(CursorPos::FilePosition { ref path, .. }) if path == &self.path => {
                    peer.announce(presence)
                }
                _ => Err(CoError::Invalid(format!(
                    "code presence must target file path {}",
                    self.path
                ))),
            },
            SurfaceIntent::TextInsert { .. } | SurfaceIntent::TextPush { .. } => {
                Err(CoError::Invalid(
                    "code adapter does not merge file bytes through text regions".to_string(),
                ))
            }
            SurfaceIntent::Structured { .. } | SurfaceIntent::Note { .. } => Err(CoError::Invalid(
                "code adapter accepts only code presence and edit-footprint intents".to_string(),
            )),
        }
    }

    fn from_peer(&mut self, peer: &SubstratePeer) -> CoResult<SurfaceSnapshot> {
        Ok(SurfaceSnapshot::Code {
            snapshot: self.snapshot(peer)?,
        })
    }
}

impl InSubstrateAdapter for CodeSurfaceAdapter {}

fn file_node_id(path: &str) -> String {
    format!("file:{path}")
}

fn edit_node_id(actor: ActorId, path: &str) -> String {
    format!("code_edit:{actor}:{path}")
}
