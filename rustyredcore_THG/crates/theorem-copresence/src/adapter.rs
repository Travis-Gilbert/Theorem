use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::note::{NoteIntent, NoteSnapshot};
use crate::peer::{StructuredOp, SubstratePeer};
use crate::presence::Presence;
use crate::CoResult;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SurfaceIntent {
    Structured {
        op: StructuredOp,
    },
    TextInsert {
        region_id: String,
        index: u32,
        text: String,
    },
    TextPush {
        region_id: String,
        text: String,
    },
    Presence {
        presence: Presence,
    },
    Note {
        intent: NoteIntent,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum SurfaceSnapshot {
    Generic { scope: String, graph: Value },
    Note { snapshot: NoteSnapshot },
}

pub trait SurfaceAdapter {
    fn to_peer(&mut self, peer: &mut SubstratePeer, intent: SurfaceIntent) -> CoResult<()>;

    #[allow(clippy::wrong_self_convention)]
    fn from_peer(&mut self, peer: &SubstratePeer) -> CoResult<SurfaceSnapshot>;
}

pub trait InSubstrateAdapter: SurfaceAdapter {}

pub trait InstrumentAdapter: SurfaceAdapter {}
