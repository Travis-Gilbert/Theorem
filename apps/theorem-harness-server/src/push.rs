//! Native push for the coordination room (docs/plans/coordination-room-push).
//!
//! The room write path emits a `RoomMessageEvent` onto an in-process broadcast
//! bus. Two consumers ride the same bus:
//!   - the app's SSE subscription (`GET /harness/rooms/:room_id/stream`), which
//!     makes the room feel real-time because the app holds the socket;
//!   - a spawn-listener task that wakes agents on `delivery = wake` messages.
//!
//! `delivery` is a first-class field on the coordination message (`delivery` on
//! `WriteMessageInput`/`CoordinationMessageState` in `theorem-harness-runtime`),
//! so every surface that writes a message - the MCP `coordinate` path and this
//! HTTP path alike - carries the tap/hold intent consistently. For v1 the bus is
//! in-process: the app posts to the
//! same harness server that streams to it, so the human side is fully connected
//! without external infrastructure. Cross-process delivery (an agent's MCP-side
//! `coordinate` waking an open app in real time) is the named follow-up and
//! would swap the in-process `broadcast` for Redis/NATS or a RustyRed pub/sub.

use std::collections::BTreeSet;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
    Json, Router,
};
use rustyred_thg_core::GraphStore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::{Stream, StreamExt};

use theorem_harness_runtime::{
    room_status, write_message, CoordinationError, CoordinationMessageState, WriteMessageInput,
};

/// Default in-process bus depth. Generous: each open app and the spawn-listener
/// is a receiver, and a slow receiver only lags (it never blocks the writer).
pub const DEFAULT_BUS_CAPACITY: usize = 1024;

/// The single send button's two behaviors, both riding the same emit. `delivery`
/// is the whole difference between leaving a note and queueing the agents.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Delivery {
    /// Tap: leave a note. Streamed to anyone watching the room; the spawn-listener
    /// ignores it, so the agent sees it next time it is live and calls `mentions`.
    #[default]
    Passive,
    /// Hold: queue the agents. The spawn-listener fires a session for the targets.
    Wake,
}

impl Delivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Delivery::Passive => "passive",
            Delivery::Wake => "wake",
        }
    }

    /// Parse the coordination message's first-class `delivery` field, defaulting
    /// to passive (the safe "left a note" reading) when empty or unrecognized.
    pub fn from_core(delivery: &str) -> Delivery {
        if delivery.trim().eq_ignore_ascii_case("wake") {
            Delivery::Wake
        } else {
            Delivery::Passive
        }
    }
}

/// The event published on the room bus when a message is written. This is both
/// the SSE payload the app renders and the trigger the spawn-listener reads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RoomMessageEvent {
    pub tenant_slug: String,
    pub room_id: String,
    pub message_id: String,
    pub author: String,
    pub urgency: String,
    pub message: String,
    pub mentions: Vec<String>,
    pub delivery: Delivery,
    pub created_at: String,
}

impl RoomMessageEvent {
    /// Project a persisted message into the bus event the consumers ride.
    pub fn from_state(state: &CoordinationMessageState) -> RoomMessageEvent {
        RoomMessageEvent {
            tenant_slug: state.tenant_slug.clone(),
            room_id: state.room_id.clone(),
            message_id: state.message_id.clone(),
            author: state.actor_id.clone(),
            urgency: state.urgency.clone(),
            message: state.message.clone(),
            mentions: state.mentions.clone(),
            delivery: Delivery::from_core(&state.delivery),
            created_at: state.created_at.clone(),
        }
    }
}

/// Request body for the room write endpoint: the single send button's payload.
/// `delivery` defaults to passive (tap) when the field is absent.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MessagePost {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub urgency: String,
    #[serde(default)]
    pub delivery: Delivery,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
}

/// Write the message durably and produce the bus event. `delivery` is set as the
/// message's first-class field (the runtime persists it on the node), so the
/// tap/hold intent survives to the cross-process consumer in a later iteration.
/// Pure over the store; the caller publishes the returned event onto the bus.
pub fn write_room_message<S: GraphStore>(
    store: &mut S,
    room_id: &str,
    post: MessagePost,
) -> Result<(CoordinationMessageState, RoomMessageEvent), CoordinationError> {
    let state = write_message(
        store,
        WriteMessageInput {
            tenant_slug: post.tenant_slug,
            room_id: room_id.to_string(),
            actor_id: post.actor_id,
            message_id: String::new(),
            urgency: post.urgency,
            delivery: post.delivery.as_str().to_string(),
            message: post.message,
            mentions: post.mentions,
            metadata: post.metadata,
            created_at: String::new(),
        },
    )?;
    let event = RoomMessageEvent::from_state(&state);
    Ok((state, event))
}

/// Who to wake for an event: the explicit mentions, or - if none are named - all
/// the supplied room agents (the "all room agents if none" rule). Passive events
/// wake no one. The author is never woken by their own message.
pub fn wake_targets(event: &RoomMessageEvent, room_agents: &[String]) -> Vec<String> {
    if event.delivery != Delivery::Wake {
        return Vec::new();
    }
    let source: &[String] = if event.mentions.is_empty() {
        room_agents
    } else {
        &event.mentions
    };
    let mut seen = BTreeSet::new();
    let mut targets = Vec::new();
    for actor in source {
        let actor = actor.trim();
        if actor.is_empty() || actor == event.author.trim() || !seen.insert(actor.to_string()) {
            continue;
        }
        targets.push(actor.to_string());
    }
    targets
}

/// Outcome of attempting to wake one actor.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SpawnOutcome {
    pub actor_id: String,
    pub dispatched: bool,
    pub detail: String,
}

/// Strategy for waking an agent on a wake-flagged message. The default impl runs
/// a configured command; tests inject a recorder. The boundary the handoff keeps
/// is coordinate-versus-execute: a message launches a run, it does not try to
/// finish inside a chat bubble.
pub trait SpawnDispatcher: Send + Sync {
    fn dispatch(&self, actor_id: &str, event: &RoomMessageEvent) -> SpawnOutcome;
}

/// Spawns the per-actor command configured via the environment, passing the wake
/// data through env vars (never interpolated into the command string, since the
/// message body is room-attacker-influenced). Resolution order:
///   `THEOREM_SPAWN_CMD_<ACTOR>` (actor uppercased, non-alphanumeric -> `_`),
///   then the generic `THEOREM_SPAWN_CMD`. With neither set the wake is a logged
///   no-op - honest about the fact that no runner is wired, rather than pretending
///   an agent woke. The command is run via `sh -c`, fire-and-forget (the child's
///   completion is the run's concern, not the listener's).
#[derive(Clone, Copy, Debug, Default)]
pub struct CommandSpawnDispatcher;

impl CommandSpawnDispatcher {
    fn command_for(actor_id: &str) -> Option<String> {
        let key = format!("THEOREM_SPAWN_CMD_{}", env_actor_key(actor_id));
        std::env::var(&key)
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("THEOREM_SPAWN_CMD")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
    }
}

fn env_actor_key(actor_id: &str) -> String {
    actor_id
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

impl SpawnDispatcher for CommandSpawnDispatcher {
    fn dispatch(&self, actor_id: &str, event: &RoomMessageEvent) -> SpawnOutcome {
        let Some(command) = Self::command_for(actor_id) else {
            return SpawnOutcome {
                actor_id: actor_id.to_string(),
                dispatched: false,
                detail: "no spawn command configured".to_string(),
            };
        };
        let spawned = std::process::Command::new("sh")
            .arg("-c")
            .arg(&command)
            .env("THEOREM_WAKE_ACTOR", actor_id)
            .env("THEOREM_WAKE_TENANT", &event.tenant_slug)
            .env("THEOREM_WAKE_ROOM", &event.room_id)
            .env("THEOREM_WAKE_MESSAGE_ID", &event.message_id)
            .env("THEOREM_WAKE_MESSAGE", &event.message)
            .env("THEOREM_WAKE_AUTHOR", &event.author)
            .spawn();
        match spawned {
            Ok(_child) => SpawnOutcome {
                actor_id: actor_id.to_string(),
                dispatched: true,
                detail: "spawned".to_string(),
            },
            Err(error) => SpawnOutcome {
                actor_id: actor_id.to_string(),
                dispatched: false,
                detail: format!("spawn failed: {error}"),
            },
        }
    }
}

/// The in-process emit. A `coordinate` write publishes one `RoomMessageEvent`;
/// every subscriber (each open app's SSE stream, the spawn-listener) gets a
/// clone. A lagging subscriber is dropped events, never a blocked writer.
#[derive(Clone)]
pub struct RoomBus {
    sender: broadcast::Sender<RoomMessageEvent>,
    spawn: Arc<dyn SpawnDispatcher>,
}

impl RoomBus {
    pub fn new(capacity: usize, spawn: Arc<dyn SpawnDispatcher>) -> Self {
        let (sender, _receiver) = broadcast::channel(capacity.max(1));
        Self { sender, spawn }
    }

    /// Convenience constructor with the default command-spawn dispatcher.
    pub fn with_command_spawn(capacity: usize) -> Self {
        Self::new(capacity, Arc::new(CommandSpawnDispatcher))
    }

    pub fn publish(&self, event: RoomMessageEvent) {
        // Err only means no receivers right now; the durable write already landed.
        let _ = self.sender.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<RoomMessageEvent> {
        self.sender.subscribe()
    }

    pub fn spawn_dispatcher(&self) -> Arc<dyn SpawnDispatcher> {
        self.spawn.clone()
    }
}

/// Router state for the push endpoints: the same durable store the read handlers
/// use, plus the bus. Generic over the store so the routes are testable against
/// an `InMemoryGraphStore` and served over a `RedCoreGraphStore`.
pub struct PushState<S: GraphStore> {
    pub store: Arc<Mutex<S>>,
    pub bus: RoomBus,
}

impl<S: GraphStore> Clone for PushState<S> {
    fn clone(&self) -> Self {
        Self {
            store: self.store.clone(),
            bus: self.bus.clone(),
        }
    }
}

/// The two push routes, sharing the store the read endpoints already hold:
///   POST /harness/rooms/:room_id/messages  -> write + emit (the send button)
///   GET  /harness/rooms/:room_id/stream    -> SSE of this room's events
pub fn push_router<S: GraphStore + Send + 'static>(state: PushState<S>) -> Router {
    Router::new()
        .route(
            "/harness/rooms/:room_id/messages",
            post(post_message_handler::<S>),
        )
        .route(
            "/harness/rooms/:room_id/stream",
            get(stream_room_handler::<S>),
        )
        .with_state(state)
}

async fn post_message_handler<S: GraphStore + Send + 'static>(
    State(state): State<PushState<S>>,
    Path(room_id): Path<String>,
    Json(post): Json<MessagePost>,
) -> Result<Json<Value>, (StatusCode, String)> {
    if post.actor_id.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "actor_id is required".to_string()));
    }
    if post.message.trim().is_empty() {
        return Err((StatusCode::BAD_REQUEST, "message is required".to_string()));
    }

    let (message, event) = {
        let mut store = state
            .store
            .lock()
            .map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "store lock".to_string()))?;
        write_room_message(&mut *store, &room_id, post).map_err(coordination_status)?
    };

    state.bus.publish(event.clone());
    Ok(Json(json!({ "message": message, "event": event })))
}

/// Query parameters for the room stream. `tenant` is REQUIRED. The SSE filter
/// must scope by tenant as well as room: the bus is a single in-process broadcast
/// shared by every tenant, so room-only filtering lets two tenants that happen to
/// share a `room_id` (e.g. the default `repo:theorem:branch:main`) cross-receive
/// each other's messages. POST writes already carry `tenant_slug`; the stream has
/// to carry it too, or the write/read sides disagree on isolation.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct StreamQuery {
    #[serde(default)]
    pub tenant: String,
}

/// The bus-event filter a stream applies: an event is delivered only when BOTH its
/// tenant and its room match the subscription. Pulled out as a pure predicate so
/// the tenant-isolation invariant is unit-testable without standing up the server.
pub fn stream_event_matches(event: &RoomMessageEvent, tenant: &str, room_id: &str) -> bool {
    event.tenant_slug == tenant && event.room_id == room_id
}

async fn stream_room_handler<S: GraphStore + Send + 'static>(
    State(state): State<PushState<S>>,
    Path(room_id): Path<String>,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, (StatusCode, String)> {
    let tenant = query.tenant.trim().to_string();
    if tenant.is_empty() {
        // Without a tenant the only available filter is room-only scoping, which
        // is exactly the cross-tenant leak. Refuse the subscription rather than
        // silently fall back to leaking.
        return Err((
            StatusCode::BAD_REQUEST,
            "tenant query parameter is required".to_string(),
        ));
    }
    let receiver = state.bus.subscribe();
    let stream = BroadcastStream::new(receiver).filter_map(move |item| match item {
        // This tenant's events in this room only; skip other tenants, other rooms,
        // and lag notifications.
        Ok(event) if stream_event_matches(&event, &tenant, &room_id) => Event::default()
            .event("room_message")
            .json_data(&event)
            .ok()
            .map(Ok),
        _ => None,
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::default()))
}

fn coordination_status(error: CoordinationError) -> (StatusCode, String) {
    match error {
        CoordinationError::InvalidInput { .. } => (StatusCode::BAD_REQUEST, error.to_string()),
        _ => (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()),
    }
}

/// The agent-wake side: a single task subscribed to the same bus. On a
/// wake-flagged message it resolves the targets (mentions, else room members)
/// and asks the dispatcher to spawn each. Because it is a task inside an
/// already-running server, the marginal always-on cost is one subscription.
pub fn spawn_wake_listener<S: GraphStore + Send + 'static>(
    bus: RoomBus,
    store: Arc<Mutex<S>>,
) -> JoinHandle<()> {
    let mut receiver = bus.subscribe();
    let spawn = bus.spawn_dispatcher();
    tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event.delivery != Delivery::Wake {
                        continue;
                    }
                    let agents = room_agent_ids(&store, &event.tenant_slug, &event.room_id);
                    for actor in wake_targets(&event, &agents) {
                        let outcome = spawn.dispatch(&actor, &event);
                        tracing::info!(
                            actor = %actor,
                            dispatched = outcome.dispatched,
                            room = %event.room_id,
                            detail = %outcome.detail,
                            "coordination wake dispatch"
                        );
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "wake listener lagged behind the bus");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

/// The room's member actor ids (the wake fallback when no one is mentioned).
/// A member with no configured spawn command is a harmless no-op, so this needs
/// no human/agent flag: only actors with a runner wired actually wake.
fn room_agent_ids<S: GraphStore>(
    store: &Arc<Mutex<S>>,
    tenant_slug: &str,
    room_id: &str,
) -> Vec<String> {
    let store = match store.lock() {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    match room_status(&*store, tenant_slug, room_id) {
        Ok(room) => room.members.keys().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;
    use std::sync::Mutex as StdMutex;
    use theorem_harness_runtime::{join_room, JoinRoomInput};

    const TENANT: &str = "travis-gilbert";
    const ROOM: &str = "repo:theorem:branch:main";

    fn post(actor: &str, message: &str, delivery: Delivery, mentions: &[&str]) -> MessagePost {
        MessagePost {
            tenant_slug: TENANT.to_string(),
            actor_id: actor.to_string(),
            message: message.to_string(),
            urgency: "info".to_string(),
            delivery,
            mentions: mentions.iter().map(|m| m.to_string()).collect(),
            metadata: Map::new(),
        }
    }

    #[test]
    fn write_sets_delivery_field_and_event() {
        let mut store = InMemoryGraphStore::new();
        let (state, event) = write_room_message(
            &mut store,
            ROOM,
            post("travis", "ship it", Delivery::Wake, &[]),
        )
        .expect("write");

        assert_eq!(state.delivery, "wake");
        assert_eq!(event.delivery, Delivery::Wake);
        assert_eq!(event.author, "travis");
        assert_eq!(event.room_id, ROOM);
        assert_eq!(event.message, "ship it");
        assert!(!event.message_id.is_empty());
    }

    #[test]
    fn tap_defaults_to_passive() {
        let mut store = InMemoryGraphStore::new();
        let (state, event) = write_room_message(
            &mut store,
            ROOM,
            post("travis", "just a note", Delivery::Passive, &[]),
        )
        .expect("write");
        assert_eq!(state.delivery, "passive");
        assert_eq!(event.delivery, Delivery::Passive);
    }

    #[test]
    fn delivery_parses_from_core_field() {
        assert_eq!(Delivery::from_core(""), Delivery::Passive);
        assert_eq!(Delivery::from_core("wake"), Delivery::Wake);
        assert_eq!(Delivery::from_core("WAKE"), Delivery::Wake);
        assert_eq!(Delivery::from_core("nonsense"), Delivery::Passive);
    }

    #[test]
    fn passive_wakes_no_one() {
        let event = RoomMessageEvent {
            tenant_slug: TENANT.to_string(),
            room_id: ROOM.to_string(),
            message_id: "m1".to_string(),
            author: "travis".to_string(),
            urgency: "info".to_string(),
            message: "@codex note".to_string(),
            mentions: vec!["codex".to_string()],
            delivery: Delivery::Passive,
            created_at: "t".to_string(),
        };
        assert!(wake_targets(&event, &["codex".to_string()]).is_empty());
    }

    #[test]
    fn wake_uses_mentions_then_room_agents_and_never_the_author() {
        let mentioned = RoomMessageEvent {
            tenant_slug: TENANT.to_string(),
            room_id: ROOM.to_string(),
            message_id: "m1".to_string(),
            author: "travis".to_string(),
            urgency: "info".to_string(),
            message: "@codex @claude-code go".to_string(),
            mentions: vec!["codex".to_string(), "claude-code".to_string()],
            delivery: Delivery::Wake,
            created_at: "t".to_string(),
        };
        assert_eq!(
            wake_targets(&mentioned, &["everyone".to_string()]),
            vec!["codex".to_string(), "claude-code".to_string()]
        );

        let unaddressed = RoomMessageEvent {
            mentions: Vec::new(),
            message: "everyone wake".to_string(),
            ..mentioned.clone()
        };
        // Room agents minus the author.
        assert_eq!(
            wake_targets(
                &unaddressed,
                &[
                    "travis".to_string(),
                    "codex".to_string(),
                    "claude-code".to_string()
                ]
            ),
            vec!["codex".to_string(), "claude-code".to_string()]
        );
    }

    #[test]
    fn unconfigured_command_dispatcher_is_an_honest_noop() {
        let event = RoomMessageEvent {
            tenant_slug: TENANT.to_string(),
            room_id: ROOM.to_string(),
            message_id: "m1".to_string(),
            author: "travis".to_string(),
            urgency: "info".to_string(),
            message: "wake".to_string(),
            mentions: vec!["actor-with-no-runner".to_string()],
            delivery: Delivery::Wake,
            created_at: "t".to_string(),
        };
        let outcome = CommandSpawnDispatcher.dispatch("actor-with-no-runner", &event);
        assert!(!outcome.dispatched);
        assert_eq!(outcome.detail, "no spawn command configured");
    }

    #[test]
    fn env_actor_key_is_a_safe_uppercase_slug() {
        assert_eq!(env_actor_key("claude-code"), "CLAUDE_CODE");
        assert_eq!(env_actor_key("codex"), "CODEX");
        assert_eq!(env_actor_key("a.b:c"), "A_B_C");
    }

    /// Recording dispatcher for the listener test: captures every wake without
    /// spawning a process.
    #[derive(Default)]
    struct RecordingDispatcher {
        calls: StdMutex<Vec<(String, RoomMessageEvent)>>,
    }

    impl RecordingDispatcher {
        fn calls(&self) -> Vec<(String, RoomMessageEvent)> {
            self.calls.lock().expect("calls lock").clone()
        }
    }

    impl SpawnDispatcher for RecordingDispatcher {
        fn dispatch(&self, actor_id: &str, event: &RoomMessageEvent) -> SpawnOutcome {
            self.calls
                .lock()
                .expect("calls lock")
                .push((actor_id.to_string(), event.clone()));
            SpawnOutcome {
                actor_id: actor_id.to_string(),
                dispatched: true,
                detail: "recorded".to_string(),
            }
        }
    }

    #[tokio::test]
    async fn listener_wakes_mentioned_agents_on_hold_and_ignores_tap() {
        let recorder = Arc::new(RecordingDispatcher::default());
        let bus = RoomBus::new(64, recorder.clone());
        let store = Arc::new(Mutex::new(InMemoryGraphStore::new()));

        // The store is consulted only for the unaddressed fallback; mentions skip it.
        let handle = spawn_wake_listener(bus.clone(), store.clone());

        let (_state, wake_event) = {
            let mut guard = store.lock().unwrap();
            write_room_message(
                &mut *guard,
                ROOM,
                post("travis", "@codex build it", Delivery::Wake, &["codex"]),
            )
            .expect("wake write")
        };
        bus.publish(wake_event);

        let (_state, tap_event) = {
            let mut guard = store.lock().unwrap();
            write_room_message(
                &mut *guard,
                ROOM,
                post(
                    "travis",
                    "@codex just a note",
                    Delivery::Passive,
                    &["codex"],
                ),
            )
            .expect("tap write")
        };
        bus.publish(tap_event);

        // Give the listener a moment to drain both events.
        for _ in 0..50 {
            if !recorder.calls().is_empty() {
                break;
            }
            tokio::task::yield_now().await;
        }
        // A second yield window to confirm the passive event did NOT add a call.
        tokio::task::yield_now().await;

        let calls = recorder.calls();
        assert_eq!(calls.len(), 1, "only the wake message dispatches");
        assert_eq!(calls[0].0, "codex");
        assert_eq!(calls[0].1.delivery, Delivery::Wake);

        handle.abort();
    }

    #[tokio::test]
    async fn listener_falls_back_to_room_members_when_unaddressed() {
        let recorder = Arc::new(RecordingDispatcher::default());
        let bus = RoomBus::new(64, recorder.clone());
        let store = Arc::new(Mutex::new(InMemoryGraphStore::new()));

        // Seed two agent members so the unaddressed wake has a fallback set.
        {
            let mut guard = store.lock().unwrap();
            for actor in ["codex", "claude-code"] {
                join_room(
                    &mut *guard,
                    JoinRoomInput {
                        tenant_slug: TENANT.to_string(),
                        actor_id: actor.to_string(),
                        room_id: ROOM.to_string(),
                        updated_at: "2026-06-04T00:00:00Z".to_string(),
                        ..JoinRoomInput::default()
                    },
                )
                .expect("join");
            }
        }

        let handle = spawn_wake_listener(bus.clone(), store.clone());

        let (_state, event) = {
            let mut guard = store.lock().unwrap();
            write_room_message(
                &mut *guard,
                ROOM,
                post("travis", "everyone wake up", Delivery::Wake, &[]),
            )
            .expect("wake write")
        };
        bus.publish(event);

        for _ in 0..100 {
            if recorder.calls().len() >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }

        let woken: BTreeSet<String> = recorder
            .calls()
            .into_iter()
            .map(|(actor, _)| actor)
            .collect();
        assert_eq!(
            woken,
            BTreeSet::from(["codex".to_string(), "claude-code".to_string()])
        );

        handle.abort();
    }

    fn event_for(tenant: &str, room: &str, message: &str) -> RoomMessageEvent {
        RoomMessageEvent {
            tenant_slug: tenant.to_string(),
            room_id: room.to_string(),
            message_id: format!("{tenant}-{message}"),
            author: "peer".to_string(),
            urgency: "info".to_string(),
            message: message.to_string(),
            mentions: Vec::new(),
            delivery: Delivery::Passive,
            created_at: "t".to_string(),
        }
    }

    #[test]
    fn stream_filter_scopes_by_tenant_not_just_room() {
        let mine = event_for(TENANT, ROOM, "for-travis");
        let other = event_for("other-tenant", ROOM, "for-other");

        // The leak: same room_id, different tenant. The fixed predicate admits
        // only the matching tenant.
        assert!(stream_event_matches(&mine, TENANT, ROOM));
        assert!(
            !stream_event_matches(&other, TENANT, ROOM),
            "an event from another tenant on the same room_id must not match"
        );
        // The room dimension still matters within a tenant.
        assert!(!stream_event_matches(&mine, TENANT, "repo:other:branch:main"));
    }

    #[tokio::test]
    async fn shared_room_does_not_leak_across_tenants_on_the_bus() {
        let bus = RoomBus::new(64, Arc::new(CommandSpawnDispatcher));
        let mut receiver = bus.subscribe();

        // Both tenants ride the same bus on the same room_id.
        let theirs = event_for("other-tenant", ROOM, "secret");
        let mine = event_for(TENANT, ROOM, "mine");
        bus.publish(theirs);
        bus.publish(mine);

        // A subscriber scoped to TENANT sees only TENANT's event, even though the
        // other tenant's message was published first on the same shared room.
        let mut delivered = Vec::new();
        for _ in 0..2 {
            if let Ok(event) = receiver.recv().await {
                if stream_event_matches(&event, TENANT, ROOM) {
                    delivered.push(event.tenant_slug.clone());
                }
            }
        }
        assert_eq!(
            delivered,
            vec![TENANT.to_string()],
            "the other tenant's message must be filtered out of this tenant's stream"
        );
    }
}
