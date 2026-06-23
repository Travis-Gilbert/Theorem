use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::broadcast;

use crate::CoordinationMessageState;

pub const DEFAULT_ROOM_BUS_CAPACITY: usize = 1024;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomMessageDelivery {
    #[default]
    Passive,
    Wake,
}

impl RoomMessageDelivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passive => "passive",
            Self::Wake => "wake",
        }
    }

    pub fn from_core(delivery: &str) -> Self {
        if delivery.trim().eq_ignore_ascii_case("wake") {
            Self::Wake
        } else {
            Self::Passive
        }
    }

    pub fn is_wake(self) -> bool {
        self == Self::Wake
    }
}

impl PartialEq<&str> for RoomMessageDelivery {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomMessageEvent {
    pub tenant_slug: String,
    pub room_id: String,
    pub message_id: String,
    pub author: String,
    pub urgency: String,
    pub message: String,
    pub mentions: Vec<String>,
    pub delivery: RoomMessageDelivery,
    pub created_at: String,
}

impl RoomMessageEvent {
    pub fn from_state(state: &CoordinationMessageState) -> Self {
        Self {
            tenant_slug: state.tenant_slug.clone(),
            room_id: state.room_id.clone(),
            message_id: state.message_id.clone(),
            author: state.actor_id.clone(),
            urgency: state.urgency.clone(),
            message: state.message.clone(),
            mentions: state.mentions.clone(),
            delivery: RoomMessageDelivery::from_core(&state.delivery),
            created_at: state.created_at.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RoomEventBus {
    sender: broadcast::Sender<RoomMessageEvent>,
}

impl RoomEventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender }
    }

    pub fn publish(&self, event: RoomMessageEvent) {
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoomMessageEvent> {
        self.sender.subscribe()
    }
}

static COORDINATION_ROOM_EVENTS: OnceLock<RoomEventBus> = OnceLock::new();

pub fn global_coordination_room_bus() -> &'static RoomEventBus {
    COORDINATION_ROOM_EVENTS.get_or_init(|| RoomEventBus::new(DEFAULT_ROOM_BUS_CAPACITY))
}

pub fn subscribe_coordination_room_events() -> broadcast::Receiver<RoomMessageEvent> {
    global_coordination_room_bus().subscribe()
}

pub fn publish_coordination_room_event_from_state(state: &CoordinationMessageState) {
    let event = RoomMessageEvent::from_state(state);
    // Existing wake/coordination path keeps its exact RoomMessageEvent channel.
    global_coordination_room_bus().publish(event.clone());
    // The agent-space observatory bus carries the same message as a typed superset event.
    publish_agent_space_room_message(event);
}

pub fn stream_event_matches(event: &RoomMessageEvent, tenant: &str, room_id: &str) -> bool {
    event.tenant_slug == tenant && event.room_id == room_id
}

pub fn wake_targets(event: &RoomMessageEvent, room_agents: &[String]) -> Vec<String> {
    if !event.delivery.is_wake() {
        return Vec::new();
    }
    let source: &[String] = if event.mentions.is_empty() {
        room_agents
    } else {
        &event.mentions
    };
    let author = event.author.trim();
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for actor in source {
        let actor = actor.trim();
        if actor.is_empty() || actor == author || !seen.insert(actor.to_string()) {
            continue;
        }
        targets.push(actor.to_string());
    }
    targets
}

// ---------------------------------------------------------------------------
// Agent Space Viewport: a live observatory event bus that is a typed superset
// of the room-message bus above. It carries presence, footprints, work-graph
// transitions, coordination records, and CRDT deltas in addition to room
// messages, each stamped with a monotonic publish sequence so a client can
// backfill from a snapshot and then tail the stream without double-applying.
//
// Design: this is an ADDITIVE sibling channel, not a re-typing of the room
// bus. The wake listener and `/v1/coordination/events` keep their exact
// `RoomMessageEvent` channel; the agent-space bus is the wider surface.
// ---------------------------------------------------------------------------

/// Add/remove discriminator for footprint edges (agent -> task or files).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AddOrRemove {
    Add,
    Remove,
}

/// CRDT delta op kinds. A graph CRDT composes per-element CRDTs: vertices and
/// edges are add/remove sets, node properties are registers (SetProp).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeltaOp {
    AddVertex,
    RemoveVertex,
    AddEdge,
    RemoveEdge,
    SetProp,
}

/// Causal metadata for a CRDT delta: a dot (`actor:counter`) and/or a version
/// vector plus a wall-clock stamp. Used by the engine to decide causal
/// stability (settled) and to order concurrent ops.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dot: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub version_vector: BTreeMap<String, u64>,
    #[serde(default)]
    pub ts_ms: u64,
}

/// One CRDT delta over the work/agent graph. This is the delta contract the
/// viewport renders: `settled=false` is a pending (concurrent) op; `conflict`
/// names the graph-CRDT conflict state the engine resolved (e.g. "tombstone",
/// "dangling_edge", "contested_property", "cycle_compensation") so the viewport
/// renders whichever resolution the engine actually applied.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrdtDelta {
    pub op: DeltaOp,
    pub element_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<Value>,
    #[serde(default)]
    pub causal: CausalMeta,
    pub actor: String,
    #[serde(default)]
    pub settled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<String>,
}

/// The full agent-space event set. Adjacently tagged (`type` + `data`) so every
/// variant -- newtype (`RoomMessage`, `CrdtDelta`) and struct -- serializes
/// uniformly for the browser client.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum AgentSpaceEvent {
    RoomMessage(RoomMessageEvent),
    Presence {
        actor: String,
        status: String,
        ts_ms: u64,
    },
    Footprint {
        actor: String,
        target: String,
        op: AddOrRemove,
        ts_ms: u64,
    },
    WorkGraphTransition {
        node_id: String,
        from: String,
        to: String,
        actor: String,
        ts_ms: u64,
    },
    Record {
        kind: String,
        summary: String,
        #[serde(default)]
        refs: Vec<String>,
        ts_ms: u64,
    },
    CrdtDelta(CrdtDelta),
}

/// Stable SSE `event:` name for a given agent-space event. The client switches
/// its SceneDirective mapping on this name.
pub fn agent_space_event_kind(event: &AgentSpaceEvent) -> &'static str {
    match event {
        AgentSpaceEvent::RoomMessage(_) => "room_message",
        AgentSpaceEvent::Presence { .. } => "presence",
        AgentSpaceEvent::Footprint { .. } => "footprint",
        AgentSpaceEvent::WorkGraphTransition { .. } => "work_graph_transition",
        AgentSpaceEvent::Record { .. } => "record",
        AgentSpaceEvent::CrdtDelta(_) => "crdt_delta",
    }
}

/// A published event plus its monotonic sequence and routing scope. The `seq`
/// is the backfill cursor mechanism: a snapshot returns the current high-water
/// `seq`, the stream stamps every frame, and the client drops `seq <= cursor`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AgentSpaceEnvelope {
    pub seq: u64,
    pub tenant_slug: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub room_id: Option<String>,
    pub event: AgentSpaceEvent,
}

/// Broadcast bus for agent-space events with a monotonic publish sequence.
pub struct AgentSpaceEventBus {
    sender: broadcast::Sender<AgentSpaceEnvelope>,
    seq: AtomicU64,
}

impl AgentSpaceEventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self {
            sender,
            seq: AtomicU64::new(0),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentSpaceEnvelope> {
        self.sender.subscribe()
    }

    /// Highest sequence published so far (0 means nothing yet).
    pub fn high_water_seq(&self) -> u64 {
        self.seq.load(Ordering::SeqCst)
    }

    /// Stamp the next monotonic sequence and broadcast. Returns the assigned
    /// seq. Sequences are 1-based so a snapshot cursor of 0 means "seen nothing".
    pub fn publish(
        &self,
        tenant_slug: impl Into<String>,
        room_id: Option<String>,
        event: AgentSpaceEvent,
    ) -> u64 {
        let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let envelope = AgentSpaceEnvelope {
            seq,
            tenant_slug: tenant_slug.into(),
            room_id,
            event,
        };
        let _ = self.sender.send(envelope);
        seq
    }
}

static AGENT_SPACE_EVENTS: OnceLock<AgentSpaceEventBus> = OnceLock::new();

pub fn global_agent_space_bus() -> &'static AgentSpaceEventBus {
    AGENT_SPACE_EVENTS.get_or_init(|| AgentSpaceEventBus::new(DEFAULT_ROOM_BUS_CAPACITY))
}

pub fn subscribe_agent_space_events() -> broadcast::Receiver<AgentSpaceEnvelope> {
    global_agent_space_bus().subscribe()
}

/// The current high-water publish sequence (the snapshot cursor).
pub fn agent_space_high_water_seq() -> u64 {
    global_agent_space_bus().high_water_seq()
}

/// Publish any agent-space event onto the global bus. Returns its seq.
pub fn publish_agent_space_event(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    event: AgentSpaceEvent,
) -> u64 {
    global_agent_space_bus().publish(tenant_slug, room_id, event)
}

/// Mirror a room message onto the agent-space bus (called automatically when a
/// coordination room message is published).
pub fn publish_agent_space_room_message(event: RoomMessageEvent) -> u64 {
    let tenant = event.tenant_slug.clone();
    let room = Some(event.room_id.clone());
    publish_agent_space_event(tenant, room, AgentSpaceEvent::RoomMessage(event))
}

pub fn publish_presence_event(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    actor: impl Into<String>,
    status: impl Into<String>,
    ts_ms: u64,
) -> u64 {
    publish_agent_space_event(
        tenant_slug,
        room_id,
        AgentSpaceEvent::Presence {
            actor: actor.into(),
            status: status.into(),
            ts_ms,
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn publish_footprint_event(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    actor: impl Into<String>,
    target: impl Into<String>,
    op: AddOrRemove,
    ts_ms: u64,
) -> u64 {
    publish_agent_space_event(
        tenant_slug,
        room_id,
        AgentSpaceEvent::Footprint {
            actor: actor.into(),
            target: target.into(),
            op,
            ts_ms,
        },
    )
}

#[allow(clippy::too_many_arguments)]
pub fn publish_work_graph_transition(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    node_id: impl Into<String>,
    from: impl Into<String>,
    to: impl Into<String>,
    actor: impl Into<String>,
    ts_ms: u64,
) -> u64 {
    publish_agent_space_event(
        tenant_slug,
        room_id,
        AgentSpaceEvent::WorkGraphTransition {
            node_id: node_id.into(),
            from: from.into(),
            to: to.into(),
            actor: actor.into(),
            ts_ms,
        },
    )
}

pub fn publish_record_event(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    kind: impl Into<String>,
    summary: impl Into<String>,
    refs: Vec<String>,
    ts_ms: u64,
) -> u64 {
    publish_agent_space_event(
        tenant_slug,
        room_id,
        AgentSpaceEvent::Record {
            kind: kind.into(),
            summary: summary.into(),
            refs,
            ts_ms,
        },
    )
}

/// Publish a CRDT delta onto the agent-space bus. This is the seam the graph
/// CRDT engine drives once it lands (sequencing step 4); the transport and
/// contract are ready ahead of it.
pub fn publish_crdt_delta(
    tenant_slug: impl Into<String>,
    room_id: Option<String>,
    delta: CrdtDelta,
) -> u64 {
    publish_agent_space_event(tenant_slug, room_id, AgentSpaceEvent::CrdtDelta(delta))
}

/// Tenant/room routing predicate for the SSE stream. A `None` request room
/// matches every tenant event; a `Some(room)` request matches that room plus
/// room-less (global) events such as tenant-wide presence.
pub fn agent_space_event_matches(
    envelope: &AgentSpaceEnvelope,
    tenant: &str,
    room: Option<&str>,
) -> bool {
    if envelope.tenant_slug != tenant {
        return false;
    }
    match room {
        None => true,
        Some(requested) => match envelope.room_id.as_deref() {
            None => true,
            Some(event_room) => event_room == requested,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(tenant: &str, room: &str, delivery: RoomMessageDelivery) -> RoomMessageEvent {
        RoomMessageEvent {
            tenant_slug: tenant.to_string(),
            room_id: room.to_string(),
            message_id: "m1".to_string(),
            author: "travis".to_string(),
            urgency: "info".to_string(),
            message: "wake".to_string(),
            mentions: Vec::new(),
            delivery,
            created_at: "t".to_string(),
        }
    }

    #[test]
    fn delivery_parses_lossy_to_passive_or_wake() {
        assert_eq!(
            RoomMessageDelivery::from_core(""),
            RoomMessageDelivery::Passive
        );
        assert_eq!(
            RoomMessageDelivery::from_core("WAKE"),
            RoomMessageDelivery::Wake
        );
        assert_eq!(
            RoomMessageDelivery::from_core("nonsense"),
            RoomMessageDelivery::Passive
        );
    }

    #[test]
    fn stream_match_requires_tenant_and_room() {
        let mine = event("tenant-a", "room", RoomMessageDelivery::Passive);
        let other_tenant = event("tenant-b", "room", RoomMessageDelivery::Passive);
        let other_room = event("tenant-a", "other", RoomMessageDelivery::Passive);

        assert!(stream_event_matches(&mine, "tenant-a", "room"));
        assert!(!stream_event_matches(&other_tenant, "tenant-a", "room"));
        assert!(!stream_event_matches(&other_room, "tenant-a", "room"));
    }

    #[test]
    fn wake_targets_ignore_passive_author_and_duplicates() {
        let passive = event("tenant", "room", RoomMessageDelivery::Passive);
        assert!(wake_targets(&passive, &["codex".to_string()]).is_empty());

        let mut wake = event("tenant", "room", RoomMessageDelivery::Wake);
        wake.mentions = vec![
            "codex".to_string(),
            "codex".to_string(),
            "travis".to_string(),
            "".to_string(),
        ];
        assert_eq!(wake_targets(&wake, &[]), vec!["codex".to_string()]);

        wake.mentions = Vec::new();
        assert_eq!(
            wake_targets(
                &wake,
                &[
                    "travis".to_string(),
                    "codex".to_string(),
                    "claude-code".to_string(),
                ],
            ),
            vec!["codex".to_string(), "claude-code".to_string()]
        );
    }
}

#[cfg(test)]
mod agent_space_tests {
    use super::*;

    #[test]
    fn seq_is_monotonic_and_high_water_advances() {
        let bus = AgentSpaceEventBus::new(16);
        let mut rx = bus.subscribe();
        assert_eq!(bus.high_water_seq(), 0);

        let s1 = bus.publish(
            "tenant-x",
            Some("room:a".to_string()),
            AgentSpaceEvent::Presence {
                actor: "codex".to_string(),
                status: "working".to_string(),
                ts_ms: 1,
            },
        );
        let s2 = bus.publish(
            "tenant-x",
            None,
            AgentSpaceEvent::Presence {
                actor: "claude-code".to_string(),
                status: "idle".to_string(),
                ts_ms: 2,
            },
        );

        assert_eq!((s1, s2), (1, 2));
        assert_eq!(bus.high_water_seq(), 2);

        let e1 = rx.try_recv().expect("first envelope");
        assert_eq!(e1.seq, 1);
        assert_eq!(e1.room_id.as_deref(), Some("room:a"));
        let e2 = rx.try_recv().expect("second envelope");
        assert_eq!(e2.seq, 2);
        assert!(e2.room_id.is_none());
    }

    fn envelope(tenant: &str, room: Option<&str>) -> AgentSpaceEnvelope {
        AgentSpaceEnvelope {
            seq: 1,
            tenant_slug: tenant.to_string(),
            room_id: room.map(str::to_string),
            event: AgentSpaceEvent::Presence {
                actor: "codex".to_string(),
                status: "working".to_string(),
                ts_ms: 1,
            },
        }
    }

    #[test]
    fn room_filter_matches_tenant_and_room_including_global() {
        let scoped = envelope("tenant-a", Some("room:a"));
        let global = envelope("tenant-a", None);
        let other_room = envelope("tenant-a", Some("room:b"));
        let other_tenant = envelope("tenant-b", Some("room:a"));

        // No room requested: every event for the tenant matches.
        assert!(agent_space_event_matches(&scoped, "tenant-a", None));
        assert!(agent_space_event_matches(&global, "tenant-a", None));
        // Room requested: that room plus room-less (global) events match.
        assert!(agent_space_event_matches(
            &scoped,
            "tenant-a",
            Some("room:a")
        ));
        assert!(agent_space_event_matches(
            &global,
            "tenant-a",
            Some("room:a")
        ));
        assert!(!agent_space_event_matches(
            &other_room,
            "tenant-a",
            Some("room:a")
        ));
        // Tenant always gates.
        assert!(!agent_space_event_matches(&other_tenant, "tenant-a", None));
    }

    #[test]
    fn event_kind_names_are_stable() {
        let room = RoomMessageEvent {
            tenant_slug: "t".to_string(),
            room_id: "r".to_string(),
            message_id: "m".to_string(),
            author: "codex".to_string(),
            urgency: "info".to_string(),
            message: "hi".to_string(),
            mentions: vec![],
            delivery: RoomMessageDelivery::Passive,
            created_at: "t".to_string(),
        };
        assert_eq!(
            agent_space_event_kind(&AgentSpaceEvent::RoomMessage(room)),
            "room_message"
        );
        assert_eq!(
            agent_space_event_kind(&AgentSpaceEvent::Presence {
                actor: "a".to_string(),
                status: "working".to_string(),
                ts_ms: 0
            }),
            "presence"
        );
        assert_eq!(
            agent_space_event_kind(&AgentSpaceEvent::Footprint {
                actor: "a".to_string(),
                target: "f".to_string(),
                op: AddOrRemove::Add,
                ts_ms: 0
            }),
            "footprint"
        );
        assert_eq!(
            agent_space_event_kind(&AgentSpaceEvent::WorkGraphTransition {
                node_id: "n".to_string(),
                from: "x".to_string(),
                to: "y".to_string(),
                actor: "a".to_string(),
                ts_ms: 0
            }),
            "work_graph_transition"
        );
        assert_eq!(
            agent_space_event_kind(&AgentSpaceEvent::Record {
                kind: "tension".to_string(),
                summary: "s".to_string(),
                refs: vec![],
                ts_ms: 0
            }),
            "record"
        );
    }

    #[test]
    fn crdt_delta_round_trips_adjacently_tagged() {
        let delta = CrdtDelta {
            op: DeltaOp::AddEdge,
            element_id: "edge:1".to_string(),
            field: None,
            value: None,
            causal: CausalMeta {
                dot: Some("codex:7".to_string()),
                version_vector: BTreeMap::new(),
                ts_ms: 42,
            },
            actor: "codex".to_string(),
            settled: false,
            conflict: Some("dangling_edge".to_string()),
        };
        let event = AgentSpaceEvent::CrdtDelta(delta);
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["type"], "crdt_delta");
        assert_eq!(value["data"]["op"], "add_edge");
        assert_eq!(value["data"]["conflict"], "dangling_edge");
        assert_eq!(value["data"]["settled"], false);
        let back: AgentSpaceEvent = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, event);
    }

    #[test]
    fn presence_event_serializes_with_type_and_data() {
        let event = AgentSpaceEvent::Presence {
            actor: "codex".to_string(),
            status: "working".to_string(),
            ts_ms: 7,
        };
        let value = serde_json::to_value(&event).expect("serialize");
        assert_eq!(value["type"], "presence");
        assert_eq!(value["data"]["actor"], "codex");
        assert_eq!(value["data"]["status"], "working");
        let back: AgentSpaceEvent = serde_json::from_value(value).expect("deserialize");
        assert_eq!(back, event);
    }
}
