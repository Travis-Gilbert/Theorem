use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adapter::{InSubstrateAdapter, SurfaceAdapter, SurfaceIntent, SurfaceSnapshot};
use crate::peer::{StructuredOp, SubstratePeer};
use crate::{CoError, CoResult};

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NoteIntent {
    SetTitle {
        title: String,
    },
    SetStatus {
        status: String,
    },
    AddSection {
        section_id: String,
    },
    InsertSectionText {
        section_id: String,
        index: u32,
        text: String,
    },
    PushSectionText {
        section_id: String,
        text: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NoteSectionSnapshot {
    pub section_id: String,
    pub body_region: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct NoteSnapshot {
    pub note_id: String,
    pub title: Option<String>,
    pub status: Option<String>,
    pub sections: Vec<NoteSectionSnapshot>,
}

#[derive(Clone, Debug)]
pub struct NoteAdapter {
    note_id: String,
}

impl NoteAdapter {
    pub fn new(note_id: impl Into<String>) -> Self {
        Self {
            note_id: note_id.into(),
        }
    }

    pub fn body_region(&self, section_id: &str) -> String {
        body_region_for(&self.note_id, section_id)
    }

    fn apply_note_intent(&self, peer: &mut SubstratePeer, intent: NoteIntent) -> CoResult<()> {
        match intent {
            NoteIntent::SetTitle { title } => {
                peer.apply_structured(StructuredOp::SetObjectProperty {
                    object_id: self.note_id.clone(),
                    labels: vec!["Note".to_string()],
                    key: "title".to_string(),
                    value: Value::String(title),
                })?;
            }
            NoteIntent::SetStatus { status } => {
                peer.apply_structured(StructuredOp::SetObjectProperty {
                    object_id: self.note_id.clone(),
                    labels: vec!["Note".to_string()],
                    key: "status".to_string(),
                    value: Value::String(status),
                })?;
            }
            NoteIntent::AddSection { section_id } => {
                let mut sections = note_sections(peer, &self.note_id);
                if !sections.iter().any(|existing| existing == &section_id) {
                    sections.push(section_id.clone());
                }
                peer.apply_structured(StructuredOp::SetObjectProperty {
                    object_id: self.note_id.clone(),
                    labels: vec!["Note".to_string()],
                    key: "sections".to_string(),
                    value: json!(sections),
                })?;
                peer.apply_structured(StructuredOp::SetObjectProperty {
                    object_id: section_node_id(&self.note_id, &section_id),
                    labels: vec!["NoteSection".to_string()],
                    key: "body_region".to_string(),
                    value: Value::String(self.body_region(&section_id)),
                })?;
                peer.apply_structured(StructuredOp::SetObjectProperty {
                    object_id: section_node_id(&self.note_id, &section_id),
                    labels: vec!["NoteSection".to_string()],
                    key: "note_id".to_string(),
                    value: Value::String(self.note_id.clone()),
                })?;
                peer.text_region(&self.body_region(&section_id))?;
            }
            NoteIntent::InsertSectionText {
                section_id,
                index,
                text,
            } => {
                peer.text_region(&self.body_region(&section_id))?
                    .insert(index, &text)?;
            }
            NoteIntent::PushSectionText { section_id, text } => {
                peer.text_region(&self.body_region(&section_id))?
                    .push(&text)?;
            }
        }
        Ok(())
    }
}

impl SurfaceAdapter for NoteAdapter {
    fn to_peer(&mut self, peer: &mut SubstratePeer, intent: SurfaceIntent) -> CoResult<()> {
        match intent {
            SurfaceIntent::Note { intent } => self.apply_note_intent(peer, intent),
            SurfaceIntent::Structured { op } => peer.apply_structured(op).map(|_| ()),
            SurfaceIntent::TextInsert {
                region_id,
                index,
                text,
            } => peer
                .text_region(&region_id)?
                .insert(index, &text)
                .map(|_| ()),
            SurfaceIntent::TextPush { region_id, text } => {
                peer.text_region(&region_id)?.push(&text).map(|_| ())
            }
            SurfaceIntent::Presence { presence } => peer.announce(presence),
        }
    }

    fn from_peer(&mut self, peer: &SubstratePeer) -> CoResult<SurfaceSnapshot> {
        let Some(note) = peer.graph_node(&self.note_id) else {
            return Err(CoError::Invalid(format!(
                "note object not found: {}",
                self.note_id
            )));
        };
        let title = note
            .properties
            .get("title")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let status = note
            .properties
            .get("status")
            .and_then(Value::as_str)
            .map(ToString::to_string);
        let sections = sections_from_value(note.properties.get("sections"))
            .into_iter()
            .map(|section_id| {
                let body_region = peer
                    .graph_node(&section_node_id(&self.note_id, &section_id))
                    .and_then(|section| {
                        section
                            .properties
                            .get("body_region")
                            .and_then(Value::as_str)
                            .map(ToString::to_string)
                    })
                    .unwrap_or_else(|| self.body_region(&section_id));
                let body = peer.text_region_contents(&body_region).unwrap_or_default();
                NoteSectionSnapshot {
                    section_id,
                    body_region,
                    body,
                }
            })
            .collect();

        Ok(SurfaceSnapshot::Note {
            snapshot: NoteSnapshot {
                note_id: self.note_id.clone(),
                title,
                status,
                sections,
            },
        })
    }
}

impl InSubstrateAdapter for NoteAdapter {}

fn note_sections(peer: &SubstratePeer, note_id: &str) -> Vec<String> {
    peer.graph_node(note_id)
        .and_then(|node| node.properties.get("sections").cloned())
        .map(|value| sections_from_value(Some(&value)))
        .unwrap_or_default()
}

fn sections_from_value(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(ToString::to_string)
        .collect()
}

fn section_node_id(note_id: &str, section_id: &str) -> String {
    format!("{note_id}:section:{section_id}")
}

fn body_region_for(note_id: &str, section_id: &str) -> String {
    format!("{note_id}:section:{section_id}:body")
}
