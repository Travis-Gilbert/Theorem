use serde::{Deserialize, Serialize};

use rustyred_thg_core::ActorId;

pub const PRESENCE_PAYLOAD_TYPE: &str = "copresence.presence.v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresenceKind {
    Human,
    Agent,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CursorPos {
    TextIndex { region_id: String, index: u32 },
    Object { object_id: String },
    /// A position inside a code file (the W5 code-surface cursor). Code presence
    /// addresses file:line:col, distinct from a text-region index, because a
    /// code file is NOT a yrs text region: it is versioned by git and
    /// materialized to the working tree, never CRDT-merged.
    FilePosition { path: String, line: u32, col: u32 },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Presence {
    pub actor: ActorId,
    pub scope: String,
    pub focus_region: Option<String>,
    pub cursor: Option<CursorPos>,
    pub label: String,
    pub kind: PresenceKind,
}

impl Presence {
    pub fn agent(
        actor: ActorId,
        scope: impl Into<String>,
        label: impl Into<String>,
        focus_region: Option<String>,
    ) -> Self {
        Self {
            actor,
            scope: scope.into(),
            focus_region,
            cursor: None,
            label: label.into(),
            kind: PresenceKind::Agent,
        }
    }

    /// Presence at a position inside a code file (W5). The cursor is a
    /// [`CursorPos::FilePosition`] and the focus region is the file path.
    pub fn at_code(
        actor: ActorId,
        scope: impl Into<String>,
        label: impl Into<String>,
        path: impl Into<String>,
        line: u32,
        col: u32,
        kind: PresenceKind,
    ) -> Self {
        let path = path.into();
        Self {
            actor,
            scope: scope.into(),
            focus_region: Some(path.clone()),
            cursor: Some(CursorPos::FilePosition { path, line, col }),
            label: label.into(),
            kind,
        }
    }
}
