use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{ActorId, WorkingLogEvent};
use theorem_harness_core::{
    ScratchpadAwarenessEntry, ScratchpadCrdtBacking, ScratchpadRevision, ScratchpadRevisionRelation,
};

use crate::peer::{MUTATION_PAYLOAD_TYPE, TEXT_UPDATE_PAYLOAD_TYPE};
use crate::presence::{CursorPos, Presence, PresenceKind, PRESENCE_PAYLOAD_TYPE};
use crate::text_region::TextRegionUpdate;
use crate::{CoResult, StructuredOp, SubstratePeer};

pub struct ScratchpadSession {
    peer: SubstratePeer,
    crdt: ScratchpadCrdtBacking,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ScratchpadPublishReceipt {
    pub revision_id: String,
    pub region_id: String,
    pub text_update: TextRegionUpdate,
    pub awareness: ScratchpadAwarenessEntry,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ScratchpadLiveDelta {
    pub from_cursor: u64,
    pub to_cursor: u64,
    pub events: Vec<ScratchpadLiveEvent>,
    pub awareness: Vec<ScratchpadAwarenessEntry>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum ScratchpadLiveEvent {
    Revision {
        cursor: u64,
        revision_id: String,
        actor_head_id: String,
        region_id: String,
        summary: String,
    },
    TextUpdate {
        cursor: u64,
        region_id: String,
        update_len: usize,
    },
    Awareness {
        cursor: u64,
        entry: ScratchpadAwarenessEntry,
    },
    Structured {
        cursor: u64,
        row_id: Option<String>,
    },
}

impl ScratchpadSession {
    pub fn new(peer: SubstratePeer, crdt: ScratchpadCrdtBacking) -> Self {
        Self { peer, crdt }
    }

    pub fn peer(&self) -> &SubstratePeer {
        &self.peer
    }

    pub fn peer_mut(&mut self) -> &mut SubstratePeer {
        &mut self.peer
    }

    pub fn crdt(&self) -> &ScratchpadCrdtBacking {
        &self.crdt
    }

    pub fn publish_revision(
        &mut self,
        revision: &ScratchpadRevision,
        relations: &[ScratchpadRevisionRelation],
        awareness: Option<ScratchpadAwarenessEntry>,
    ) -> CoResult<ScratchpadPublishReceipt> {
        let region_id = scratchpad_region_for_revision(revision);
        self.publish_revision_node(revision, &region_id)?;
        self.publish_revision_edges(revision, relations)?;
        let text_update = self
            .peer
            .push_text(&region_id, &revision_text_chunk(revision))?;
        let awareness = awareness.unwrap_or_else(|| ScratchpadAwarenessEntry {
            actor_head_id: revision.actor_head_id.clone(),
            region_id: region_id.clone(),
            revision_id: revision.revision_id.clone(),
            status: "writing".to_string(),
            cursor: revision.seq,
            updated_at: revision.created_at.clone(),
        });
        self.announce_awareness(&awareness)?;
        Ok(ScratchpadPublishReceipt {
            revision_id: revision.revision_id.clone(),
            region_id,
            text_update,
            awareness,
        })
    }

    pub fn subscribe_after(&mut self, cursor: u64, limit: usize) -> CoResult<ScratchpadLiveDelta> {
        let peer_delta = self.peer.sync_after(cursor, limit)?;
        let events = peer_delta
            .events
            .iter()
            .filter_map(scratchpad_live_event)
            .collect::<Vec<_>>();
        Ok(ScratchpadLiveDelta {
            from_cursor: peer_delta.from_cursor,
            to_cursor: peer_delta.to_cursor,
            events,
            awareness: self.awareness_snapshot()?,
        })
    }

    pub fn awareness_snapshot(&self) -> CoResult<Vec<ScratchpadAwarenessEntry>> {
        let mut latest: BTreeMap<String, (u64, ScratchpadAwarenessEntry)> = BTreeMap::new();
        for event in self.peer.observe(0)? {
            let crate::PeerEvent::Presence { cursor, presence } = event else {
                continue;
            };
            let Some(entry) = awareness_entry_from_presence(cursor, presence) else {
                continue;
            };
            let key = entry.actor_head_id.clone();
            let replace = latest
                .get(&key)
                .map(|(existing_cursor, _)| *existing_cursor < cursor)
                .unwrap_or(true);
            if replace {
                latest.insert(key, (cursor, entry));
            }
        }
        Ok(latest.into_values().map(|(_, entry)| entry).collect())
    }

    pub fn text_region_contents(&self, region_id: &str) -> Option<String> {
        self.peer.text_region_contents(region_id)
    }

    pub fn announce_awareness(&mut self, entry: &ScratchpadAwarenessEntry) -> CoResult<()> {
        let index = u32::try_from(entry.cursor).unwrap_or(u32::MAX);
        self.peer.announce(Presence {
            actor: ActorId::from_label(&entry.actor_head_id),
            scope: self.peer.scope().to_string(),
            focus_region: Some(entry.region_id.clone()),
            cursor: Some(CursorPos::TextIndex {
                region_id: entry.region_id.clone(),
                index,
            }),
            label: entry.actor_head_id.clone(),
            kind: PresenceKind::Agent,
        })
    }

    fn publish_revision_node(
        &mut self,
        revision: &ScratchpadRevision,
        region_id: &str,
    ) -> CoResult<()> {
        self.peer
            .apply_structured(StructuredOp::SetObjectProperty {
                object_id: revision.revision_id.clone(),
                labels: vec![
                    "ScratchpadRevision".to_string(),
                    "ScratchpadCrdtNode".to_string(),
                ],
                key: "revision".to_string(),
                value: json!({
                    "document_graph_root_id": self.crdt.graph_root_id,
                    "yrs_doc_id": self.crdt.yrs_doc_id,
                    "stream_topic": self.crdt.stream_topic,
                    "awareness_log_id": self.crdt.awareness_log_id,
                    "region_id": region_id,
                    "revision": revision,
                }),
            })?;
        self.peer
            .apply_structured(StructuredOp::SetObjectProperty {
                object_id: format!("scratchop:{}", revision.revision_id),
                labels: vec!["ScratchpadCrdtOperation".to_string()],
                key: "operation".to_string(),
                value: json!({
                    "op_kind": "upsert_revision",
                    "graph_element_id": revision.revision_id,
                    "text_region_id": region_id,
                    "content_hash": revision.content_hash,
                    "actor_head_id": revision.actor_head_id,
                }),
            })?;
        Ok(())
    }

    fn publish_revision_edges(
        &mut self,
        revision: &ScratchpadRevision,
        relations: &[ScratchpadRevisionRelation],
    ) -> CoResult<()> {
        self.ensure_node(
            &self.crdt.graph_root_id.clone(),
            "ScratchpadCrdtGraph",
            json!({
                "graph_root_id": self.crdt.graph_root_id.clone(),
                "yrs_doc_id": self.crdt.yrs_doc_id.clone(),
                "stream_topic": self.crdt.stream_topic.clone(),
            }),
        )?;
        self.peer.apply_structured(StructuredOp::AddEdge {
            edge_id: format!(
                "scratchpad_contains:{}:{}",
                self.crdt.graph_root_id, revision.revision_id
            ),
            from_id: self.crdt.graph_root_id.clone(),
            edge_type: "SCRATCHPAD_CONTAINS_REVISION".to_string(),
            to_id: revision.revision_id.clone(),
            properties: json!({
                "stream_topic": self.crdt.stream_topic,
                "region_id": scratchpad_region_for_revision(revision),
            }),
        })?;
        for parent_id in &revision.parent_revision_ids {
            self.ensure_node(
                parent_id,
                "ScratchpadRevision",
                json!({ "placeholder": true }),
            )?;
            self.peer.apply_structured(StructuredOp::AddEdge {
                edge_id: format!("scratchpad_parent:{}:{}", revision.revision_id, parent_id),
                from_id: revision.revision_id.clone(),
                edge_type: "SCRATCHPAD_PARENT".to_string(),
                to_id: parent_id.clone(),
                properties: json!({ "actor_head_id": revision.actor_head_id }),
            })?;
        }
        for relation in relations {
            self.ensure_node(
                &relation.to_revision_id,
                "ScratchpadRevision",
                json!({ "placeholder": true }),
            )?;
            self.peer.apply_structured(StructuredOp::AddEdge {
                edge_id: relation.relation_id.clone(),
                from_id: relation.from_revision_id.clone(),
                edge_type: relation.relation_kind.edge_type().to_string(),
                to_id: relation.to_revision_id.clone(),
                properties: json!({
                    "actor_head_id": relation.actor_head_id,
                    "summary": relation.summary,
                    "payload": relation.payload,
                }),
            })?;
        }
        Ok(())
    }

    fn ensure_node(&mut self, object_id: &str, label: &str, properties: Value) -> CoResult<()> {
        if self.peer.graph_node(object_id).is_some() {
            return Ok(());
        }
        self.peer
            .apply_structured(StructuredOp::SetObjectProperty {
                object_id: object_id.to_string(),
                labels: vec![label.to_string()],
                key: "node".to_string(),
                value: properties,
            })?;
        Ok(())
    }
}

fn scratchpad_live_event(event: &WorkingLogEvent) -> Option<ScratchpadLiveEvent> {
    match event.payload.get("type").and_then(Value::as_str) {
        Some(TEXT_UPDATE_PAYLOAD_TYPE) => Some(ScratchpadLiveEvent::TextUpdate {
            cursor: event.cursor,
            region_id: event
                .payload
                .get("region_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            update_len: event
                .payload
                .get("update_v1")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or_default(),
        }),
        Some(PRESENCE_PAYLOAD_TYPE) => event
            .payload
            .get("presence")
            .cloned()
            .and_then(|value| serde_json::from_value::<Presence>(value).ok())
            .and_then(|presence| awareness_entry_from_presence(event.cursor, presence))
            .map(|entry| ScratchpadLiveEvent::Awareness {
                cursor: event.cursor,
                entry,
            }),
        Some(MUTATION_PAYLOAD_TYPE) => revision_event_from_mutation(event).or_else(|| {
            Some(ScratchpadLiveEvent::Structured {
                cursor: event.cursor,
                row_id: event.row_id.clone(),
            })
        }),
        _ => None,
    }
}

fn revision_event_from_mutation(event: &WorkingLogEvent) -> Option<ScratchpadLiveEvent> {
    let mutations = event.payload.get("batch")?.get("mutations")?.as_array()?;
    for mutation in mutations {
        let mutation = mutation.get("mutation")?;
        if mutation.get("op").and_then(Value::as_str) != Some("node_upsert") {
            continue;
        }
        let node = mutation.get("record")?;
        let properties = node.get("properties")?;
        let revision_value = properties.get("revision")?.get("revision")?;
        let revision = serde_json::from_value::<ScratchpadRevision>(revision_value.clone()).ok()?;
        return Some(ScratchpadLiveEvent::Revision {
            cursor: event.cursor,
            revision_id: revision.revision_id.clone(),
            actor_head_id: revision.actor_head_id.clone(),
            region_id: scratchpad_region_for_revision(&revision),
            summary: revision.summary.clone(),
        });
    }
    None
}

fn awareness_entry_from_presence(
    cursor: u64,
    presence: Presence,
) -> Option<ScratchpadAwarenessEntry> {
    let region_id = presence
        .focus_region
        .clone()
        .or_else(|| match &presence.cursor {
            Some(CursorPos::TextIndex { region_id, .. }) => Some(region_id.clone()),
            _ => None,
        })?;
    let text_cursor = match presence.cursor {
        Some(CursorPos::TextIndex { index, .. }) => u64::from(index),
        _ => cursor,
    };
    Some(ScratchpadAwarenessEntry {
        actor_head_id: presence.label,
        region_id,
        revision_id: String::new(),
        status: "writing".to_string(),
        cursor: text_cursor,
        updated_at: String::new(),
    })
}

fn scratchpad_region_for_revision(revision: &ScratchpadRevision) -> String {
    match revision.payload.get("kind").and_then(Value::as_str) {
        Some("proposal") => "proposal",
        Some("critique") => "critique",
        Some("synthesis") => "synthesis",
        Some("verification") => "verification",
        Some("orientation") => "orientation",
        _ => "synthesis",
    }
    .to_string()
}

fn revision_text_chunk(revision: &ScratchpadRevision) -> String {
    let body = revision
        .payload
        .get("text")
        .or_else(|| revision.payload.get("content"))
        .and_then(Value::as_str)
        .unwrap_or(&revision.summary);
    format!(
        "[{}:{}:{}]\n{}\n",
        revision.seq, revision.actor_head_id, revision.summary, body
    )
}
