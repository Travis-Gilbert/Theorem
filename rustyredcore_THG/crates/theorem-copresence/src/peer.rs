use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use rustyred_thg_core::{
    diff_since, join_delta, ActorId, EdgeRecord, GraphMutation, GraphSnapshot, GraphStore,
    HlcClock, InMemoryGraphStore, JoinReport, NodeRecord, StampedBatch, StampedMutation,
    ThgCommand, ThgExecutor, ThgRequest, VersionVector, WorkingLog, WorkingLogEvent,
};

use crate::presence::{Presence, PRESENCE_PAYLOAD_TYPE};
use crate::text_region::{open_object_store, TextRegionHandle, TextRegionUpdate};
use crate::{CoError, CoResult};

pub type SharedWorkingLog = Arc<Mutex<WorkingLog>>;

pub const MUTATION_PAYLOAD_TYPE: &str = "copresence.structured_mutation.v1";
pub const TEXT_UPDATE_PAYLOAD_TYPE: &str = "copresence.text_update.v1";
const YRS_CLIENT_MASK_53: u64 = (1_u64 << 53) - 1;

#[derive(Clone, Debug)]
pub struct PeerConfig {
    pub actor: ActorId,
    pub scope: String,
    pub data_dir: Option<PathBuf>,
    pub working_log: Option<SharedWorkingLog>,
    pub text_client_id: Option<u64>,
}

impl PeerConfig {
    pub fn new(actor: impl Into<ActorId>, scope: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            scope: scope.into(),
            data_dir: None,
            working_log: None,
            text_client_id: None,
        }
    }

    pub fn with_data_dir(mut self, data_dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(data_dir.into());
        self
    }

    pub fn with_working_log(mut self, working_log: SharedWorkingLog) -> Self {
        self.working_log = Some(working_log);
        self
    }

    pub fn with_text_client_id(mut self, client_id: u64) -> Self {
        self.text_client_id = Some(client_id & YRS_CLIENT_MASK_53);
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "op")]
pub enum StructuredOp {
    SetObjectProperty {
        object_id: String,
        labels: Vec<String>,
        key: String,
        value: Value,
    },
    AddEdge {
        edge_id: String,
        from_id: String,
        edge_type: String,
        to_id: String,
        properties: Value,
    },
    RemoveEdge {
        edge_id: String,
        from_id: String,
        edge_type: String,
        to_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum PeerEvent {
    Presence { cursor: u64, presence: Presence },
    WorkingLog { cursor: u64, event: WorkingLogEvent },
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PeerSyncDelta {
    pub from_cursor: u64,
    pub to_cursor: u64,
    pub events: Vec<WorkingLogEvent>,
    pub applied_structured: usize,
    pub applied_text: usize,
    pub presence_seen: usize,
}

pub struct SubstratePeer {
    actor: ActorId,
    scope: String,
    clock: HlcClock,
    seen: VersionVector,
    executor: Box<dyn ThgExecutor>,
    store: InMemoryGraphStore,
    working_log: SharedWorkingLog,
    text_client_id: u64,
    doc_tree: Arc<Mutex<rustyred_thg_core::DocTree>>,
    object_store: rustyred_thg_core::DiskObjectStore,
    text_regions: BTreeMap<String, TextRegionHandle>,
}

impl SubstratePeer {
    pub fn new<E>(executor: E, config: PeerConfig) -> Self
    where
        E: ThgExecutor + 'static,
    {
        Self::try_new(executor, config).expect("failed to open substrate peer")
    }

    pub fn try_new<E>(executor: E, config: PeerConfig) -> CoResult<Self>
    where
        E: ThgExecutor + 'static,
    {
        let data_dir = config.data_dir.unwrap_or_else(|| {
            std::env::temp_dir()
                .join("theorem-copresence")
                .join(safe_segment(&config.scope))
                .join(config.actor.to_string())
        });
        let object_store = open_object_store(data_dir)?;
        let text_client_id = config
            .text_client_id
            .unwrap_or_else(|| client_id_from_actor(config.actor));
        Ok(Self {
            actor: config.actor,
            scope: config.scope,
            clock: HlcClock::new(config.actor),
            seen: VersionVector::default(),
            executor: Box::new(executor),
            store: InMemoryGraphStore::new(),
            working_log: config
                .working_log
                .unwrap_or_else(|| Arc::new(Mutex::new(WorkingLog::new()))),
            text_client_id,
            doc_tree: Arc::new(Mutex::new(rustyred_thg_core::DocTree::default())),
            object_store,
            text_regions: BTreeMap::new(),
        })
    }

    pub fn actor(&self) -> ActorId {
        self.actor
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    pub fn seen(&self) -> &VersionVector {
        &self.seen
    }

    pub fn graph_node(&self, id: &str) -> Option<NodeRecord> {
        self.store.get_node_record(id)
    }

    pub fn graph_snapshot(&self) -> CoResult<GraphSnapshot> {
        self.store.graph_snapshot().map_err(CoError::from)
    }

    pub fn apply_structured(&mut self, mutation: StructuredOp) -> CoResult<VersionVector> {
        let hlc = self.clock.now();
        let graph_mutation = self.lower_structured(mutation)?;
        let stamped = StampedMutation::new(graph_mutation.clone(), hlc);
        let batch = StampedBatch::new([stamped]);

        let mut candidate = self.store.clone();
        join_delta(&mut candidate, batch.clone());
        let executor_mutation = mutation_after_join(&candidate, &graph_mutation)?;
        self.apply_executor_mutation(&executor_mutation)?;

        self.store = candidate;
        self.seen.observe(hlc);
        self.append_mutation_event(batch)?;
        Ok(self.seen.clone())
    }

    pub fn merge_delta(&mut self, batch: StampedBatch) -> CoResult<JoinReport> {
        self.merge_delta_inner(batch, true)
    }

    pub fn merge_remote_delta(&mut self, batch: StampedBatch) -> CoResult<JoinReport> {
        self.merge_delta_inner(batch, false)
    }

    fn merge_delta_inner(
        &mut self,
        batch: StampedBatch,
        append_event: bool,
    ) -> CoResult<JoinReport> {
        for mutation in &batch.mutations {
            self.clock.observe(mutation.hlc);
            self.seen.observe(mutation.hlc);
        }
        let report = join_delta(&mut self.store, batch.clone());
        if append_event {
            self.append_mutation_event(batch)?;
        }
        Ok(report)
    }

    pub fn delta_since(&self, theirs: &VersionVector) -> StampedBatch {
        diff_since(&self.store, theirs)
    }

    pub fn text_region(&mut self, region_id: &str) -> CoResult<TextRegionHandle> {
        if let Some(handle) = self.text_regions.get(region_id) {
            return Ok(handle.clone());
        }
        let handle = TextRegionHandle::open(
            region_id,
            self.scope.clone(),
            self.text_client_id,
            self.doc_tree.clone(),
            self.object_store.clone(),
        )?;
        self.text_regions
            .insert(region_id.to_string(), handle.clone());
        Ok(handle)
    }

    pub fn text_state_vector(&mut self, region_id: &str) -> CoResult<Vec<u8>> {
        Ok(self.text_region(region_id)?.encode_state_vector())
    }

    pub fn text_update_since(
        &mut self,
        region_id: &str,
        remote_state_vector_v1: &[u8],
    ) -> CoResult<Vec<u8>> {
        self.text_region(region_id)?
            .encode_update_since(remote_state_vector_v1)
    }

    pub fn apply_text_update(&mut self, region_id: &str, update_v1: &[u8]) -> CoResult<()> {
        self.text_region(region_id)?.apply_update(update_v1)
    }

    pub fn insert_text(
        &mut self,
        region_id: &str,
        index: u32,
        text: &str,
    ) -> CoResult<TextRegionUpdate> {
        let update = self.text_region(region_id)?.insert(index, text)?;
        self.append_text_event("insert", &update, Some(index), Some(text))?;
        Ok(update)
    }

    pub fn push_text(&mut self, region_id: &str, text: &str) -> CoResult<TextRegionUpdate> {
        let update = self.text_region(region_id)?.push(text)?;
        self.append_text_event("push", &update, None, Some(text))?;
        Ok(update)
    }

    pub fn text_region_contents(&self, region_id: &str) -> Option<String> {
        self.text_regions
            .get(region_id)
            .map(TextRegionHandle::contents)
    }

    pub fn persisted_text_update(&mut self, region_id: &str) -> CoResult<Option<Vec<u8>>> {
        self.text_region(region_id)?.persisted_update()
    }

    pub fn announce(&mut self, presence: Presence) -> CoResult<()> {
        let payload = json!({
            "type": PRESENCE_PAYLOAD_TYPE,
            "presence": presence,
        });
        self.working_log
            .lock()
            .map_err(|_| CoError::Lock("working log"))?
            .append_mutation(self.scope.clone(), payload);
        Ok(())
    }

    pub fn observe(&self, since_cursor: u64) -> CoResult<Vec<PeerEvent>> {
        let events = self
            .working_log
            .lock()
            .map_err(|_| CoError::Lock("working log"))?
            .subscribe_after(since_cursor, 0);
        let mut out = Vec::new();
        for event in events {
            if let Some(presence) = event
                .payload
                .get("presence")
                .cloned()
                .filter(|_| {
                    event.payload.get("type").and_then(Value::as_str) == Some(PRESENCE_PAYLOAD_TYPE)
                })
                .and_then(|value| serde_json::from_value::<Presence>(value).ok())
            {
                if presence.scope == self.scope {
                    out.push(PeerEvent::Presence {
                        cursor: event.cursor,
                        presence,
                    });
                }
            } else {
                out.push(PeerEvent::WorkingLog {
                    cursor: event.cursor,
                    event,
                });
            }
        }
        Ok(out)
    }

    pub fn sync_after(&mut self, since_cursor: u64, limit: usize) -> CoResult<PeerSyncDelta> {
        let events = self
            .working_log
            .lock()
            .map_err(|_| CoError::Lock("working log"))?
            .subscribe_after(since_cursor, limit);
        let mut delta = PeerSyncDelta {
            from_cursor: since_cursor,
            to_cursor: events
                .last()
                .map(|event| event.cursor)
                .unwrap_or(since_cursor),
            events,
            ..PeerSyncDelta::default()
        };
        for event in delta.events.clone() {
            match self.apply_working_log_event(&event)? {
                AppliedPeerEvent::Structured(applied) => {
                    delta.applied_structured += applied;
                }
                AppliedPeerEvent::Text => {
                    delta.applied_text += 1;
                }
                AppliedPeerEvent::Presence => {
                    delta.presence_seen += 1;
                }
                AppliedPeerEvent::Ignored => {}
            }
        }
        Ok(delta)
    }

    pub fn apply_working_log_event(
        &mut self,
        event: &WorkingLogEvent,
    ) -> CoResult<AppliedPeerEvent> {
        match event.payload.get("type").and_then(Value::as_str) {
            Some(MUTATION_PAYLOAD_TYPE) => {
                if event.payload.get("scope").and_then(Value::as_str) != Some(self.scope.as_str()) {
                    return Ok(AppliedPeerEvent::Ignored);
                }
                let Some(batch_value) = event.payload.get("batch").cloned() else {
                    return Ok(AppliedPeerEvent::Ignored);
                };
                let batch = serde_json::from_value::<StampedBatch>(batch_value)?;
                let report = self.merge_remote_delta(batch)?;
                Ok(AppliedPeerEvent::Structured(report.applied))
            }
            Some(TEXT_UPDATE_PAYLOAD_TYPE) => {
                if event.payload.get("scope").and_then(Value::as_str) != Some(self.scope.as_str()) {
                    return Ok(AppliedPeerEvent::Ignored);
                }
                let Some(region_id) = event.payload.get("region_id").and_then(Value::as_str) else {
                    return Ok(AppliedPeerEvent::Ignored);
                };
                let Some(update_v1) = byte_array(event.payload.get("update_v1")) else {
                    return Ok(AppliedPeerEvent::Ignored);
                };
                self.apply_text_update(region_id, &update_v1)?;
                Ok(AppliedPeerEvent::Text)
            }
            Some(PRESENCE_PAYLOAD_TYPE) => Ok(AppliedPeerEvent::Presence),
            _ => Ok(AppliedPeerEvent::Ignored),
        }
    }

    fn lower_structured(&self, mutation: StructuredOp) -> CoResult<GraphMutation> {
        match mutation {
            StructuredOp::SetObjectProperty {
                object_id,
                labels,
                key,
                value,
            } => {
                let mut node = self
                    .store
                    .get_node_record(&object_id)
                    .unwrap_or_else(|| NodeRecord::new(object_id, labels.clone(), json!({})));
                merge_labels(&mut node.labels, labels);
                ensure_property_object(&mut node.properties).insert(key, value);
                Ok(GraphMutation::NodeUpsert(node))
            }
            StructuredOp::AddEdge {
                edge_id,
                from_id,
                edge_type,
                to_id,
                properties,
            } => Ok(GraphMutation::EdgeUpsert(EdgeRecord::new(
                edge_id, from_id, edge_type, to_id, properties,
            ))),
            StructuredOp::RemoveEdge {
                edge_id,
                from_id,
                edge_type,
                to_id,
            } => {
                let mut edge = self.store.get_edge_record(&edge_id).unwrap_or_else(|| {
                    EdgeRecord::new(edge_id, from_id, edge_type, to_id, json!({}))
                });
                edge.tombstone = true;
                Ok(GraphMutation::EdgeUpsert(edge))
            }
        }
    }

    fn apply_executor_mutation(&mut self, mutation: &GraphMutation) -> CoResult<()> {
        let (command, args) = match mutation {
            GraphMutation::NodeUpsert(node) => (
                ThgCommand::GraphNodeUpsert,
                json!({
                    "id": node.id,
                    "labels": node.labels,
                    "properties": node.properties,
                    "tombstone": node.tombstone,
                }),
            ),
            GraphMutation::EdgeUpsert(edge) => (
                ThgCommand::GraphEdgeUpsert,
                json!({
                    "id": edge.id,
                    "from_id": edge.from_id,
                    "type": edge.edge_type,
                    "to_id": edge.to_id,
                    "properties": edge.properties,
                    "tombstone": edge.tombstone,
                }),
            ),
        };
        let command_name = command.name().to_string();
        let response = self
            .executor
            .execute_request(ThgRequest::new(command.name(), args));
        if response.ok {
            Ok(())
        } else {
            Err(CoError::Executor {
                command: command_name,
                status: response.status,
                detail: response
                    .error
                    .map(|error| error.message)
                    .unwrap_or_else(|| "executor rejected mutation".to_string()),
            })
        }
    }

    fn append_mutation_event(&self, batch: StampedBatch) -> CoResult<()> {
        let payload = json!({
            "type": MUTATION_PAYLOAD_TYPE,
            "actor": self.actor,
            "scope": self.scope,
            "batch": batch,
        });
        self.working_log
            .lock()
            .map_err(|_| CoError::Lock("working log"))?
            .append_mutation(self.scope.clone(), payload);
        Ok(())
    }

    fn append_text_event(
        &self,
        op: &str,
        update: &TextRegionUpdate,
        index: Option<u32>,
        text: Option<&str>,
    ) -> CoResult<()> {
        let payload = json!({
            "type": TEXT_UPDATE_PAYLOAD_TYPE,
            "actor": self.actor,
            "scope": self.scope,
            "op": op,
            "region_id": update.region_id,
            "state_vector_v1": update.state_vector_v1,
            "update_v1": update.update_v1,
            "index": index,
            "text": text,
        });
        self.working_log
            .lock()
            .map_err(|_| CoError::Lock("working log"))?
            .append_mutation(self.scope.clone(), payload);
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppliedPeerEvent {
    Structured(usize),
    Text,
    Presence,
    Ignored,
}

fn mutation_after_join(
    store: &InMemoryGraphStore,
    original: &GraphMutation,
) -> CoResult<GraphMutation> {
    match original {
        GraphMutation::NodeUpsert(node) => store
            .get_node_record(&node.id)
            .map(GraphMutation::NodeUpsert)
            .ok_or_else(|| CoError::Invalid(format!("joined node missing: {}", node.id))),
        GraphMutation::EdgeUpsert(edge) => store
            .get_edge_record(&edge.id)
            .map(GraphMutation::EdgeUpsert)
            .ok_or_else(|| CoError::Invalid(format!("joined edge missing: {}", edge.id))),
    }
}

fn ensure_property_object(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("properties are an object")
}

fn merge_labels(target: &mut Vec<String>, incoming: Vec<String>) {
    target.extend(
        incoming
            .into_iter()
            .filter(|label| !label.trim().is_empty()),
    );
    target.sort();
    target.dedup();
}

fn client_id_from_actor(actor: ActorId) -> u64 {
    let hex = actor.to_string();
    let raw = u64::from_str_radix(&hex[..16], 16).unwrap_or(1) & YRS_CLIENT_MASK_53;
    raw.max(1)
}

fn safe_segment(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    if cleaned.is_empty() {
        "scope".to_string()
    } else {
        cleaned
    }
}

fn byte_array(value: Option<&Value>) -> Option<Vec<u8>> {
    value?
        .as_array()?
        .iter()
        .map(|item| item.as_u64().and_then(|value| u8::try_from(value).ok()))
        .collect()
}
