use std::collections::BTreeSet;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
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
    global_coordination_room_bus().publish(RoomMessageEvent::from_state(state));
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
