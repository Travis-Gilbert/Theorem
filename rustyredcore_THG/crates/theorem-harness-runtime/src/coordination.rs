use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use theorem_harness_core::stable_value_hash;

pub type CoordinationResult<T> = Result<T, CoordinationError>;

const DEFAULT_TENANT: &str = "default";
const DEFAULT_ROOM: &str = "room:ungrouped";
const DEFAULT_MODE: &str = "collaborating";
const DEFAULT_PRESENCE_TTL_SECONDS: u64 = 60;
const INTENT_STATUSES: &[&str] = &["working", "paused", "done"];
const MESSAGE_URGENCIES: &[&str] = &["info", "ask", "block"];

#[derive(Clone, Debug, PartialEq)]
pub enum CoordinationError {
    Store(GraphStoreError),
    Serialization(String),
    Deserialization(String),
    InvalidInput { field: String, message: String },
}

impl fmt::Display for CoordinationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::InvalidInput { field, message } => {
                write!(f, "invalid coordination input {field}: {message}")
            }
        }
    }
}

impl Error for CoordinationError {}

impl From<GraphStoreError> for CoordinationError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct JoinRoomInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub lane: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize, Default)]
pub struct WriteIntentInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub claimed_files: Vec<String>,
    #[serde(default)]
    pub expected_completion: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PresenceInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default = "default_presence_ttl")]
    pub ttl_seconds: u64,
    #[serde(default)]
    pub refreshed_at: String,
    #[serde(default)]
    pub expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct WriteMessageInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub urgency: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub created_at: String,
}

impl Default for PresenceInput {
    fn default() -> Self {
        Self {
            tenant_slug: String::new(),
            actor_id: String::new(),
            session_id: String::new(),
            surface: String::new(),
            status: String::new(),
            worktree: String::new(),
            branch: String::new(),
            head: String::new(),
            changed_files: Vec::new(),
            ttl_seconds: DEFAULT_PRESENCE_TTL_SECONDS,
            refreshed_at: String::new(),
            expires_at: String::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinationRoomMember {
    pub tenant_slug: String,
    pub room_id: String,
    pub actor_id: String,
    pub status: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub lane: String,
    #[serde(default)]
    pub joined_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinationRoomState {
    pub tenant_slug: String,
    pub room_id: String,
    pub status: String,
    pub mode: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub members: BTreeMap<String, CoordinationRoomMember>,
    #[serde(default)]
    pub last_packet_at: String,
    #[serde(default)]
    pub last_packet_doc_id: String,
    #[serde(default)]
    pub degraded: bool,
    #[serde(default)]
    pub degraded_reason: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinationIntentState {
    pub tenant_slug: String,
    pub room_id: String,
    pub actor_id: String,
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub claimed_files: Vec<String>,
    #[serde(default)]
    pub expected_completion: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub task: String,
    #[serde(default)]
    pub started_at: String,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinationPresenceState {
    pub tenant_slug: String,
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub surface: String,
    pub status: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub head: String,
    #[serde(default)]
    pub changed_files: Vec<String>,
    #[serde(default)]
    pub refreshed_at: String,
    #[serde(default)]
    pub expires_at: String,
    pub ttl_seconds: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CoordinationMessageState {
    pub tenant_slug: String,
    pub room_id: String,
    pub message_id: String,
    pub actor_id: String,
    pub urgency: String,
    pub message: String,
    #[serde(default)]
    pub mentions: Vec<String>,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub consumed_by: Vec<String>,
    #[serde(default)]
    pub created_at: String,
}

pub fn join_room<S: GraphStore>(
    store: &mut S,
    input: JoinRoomInput,
) -> CoordinationResult<CoordinationRoomState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let actor_id = require_text("actor_id", &input.actor_id)?;
    let room_id = resolve_room_id(
        &input.room_id,
        &input.repo,
        &input.branch,
        &input.task,
        &input.session_id,
    );
    let now = timestamp_or_now(&input.updated_at);

    let mut state = load_room(store, &tenant_slug, &room_id)?
        .unwrap_or_else(|| empty_room_state(&tenant_slug, &room_id, &now));
    let existing = state.members.get(&actor_id);
    let joined_at = existing
        .map(|member| member.joined_at.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| now.clone());

    let member = CoordinationRoomMember {
        tenant_slug: tenant_slug.clone(),
        room_id: room_id.clone(),
        actor_id: actor_id.clone(),
        status: "joined".to_string(),
        session_id: choose(
            &input.session_id,
            existing.map(|member| member.session_id.as_str()),
        ),
        surface: choose(
            &input.surface,
            existing.map(|member| member.surface.as_str()),
        ),
        repo: choose(&input.repo, existing.map(|member| member.repo.as_str())),
        branch: choose(&input.branch, existing.map(|member| member.branch.as_str())),
        task: choose(&input.task, existing.map(|member| member.task.as_str())),
        worktree: choose(
            &input.worktree,
            existing.map(|member| member.worktree.as_str()),
        ),
        head: choose(&input.head, existing.map(|member| member.head.as_str())),
        changed_files: choose_files(
            &input.changed_files,
            existing.map(|member| member.changed_files.as_slice()),
        ),
        lane: choose(&input.lane, existing.map(|member| member.lane.as_str())),
        joined_at,
        updated_at: now.clone(),
    };

    state.status = "active".to_string();
    state.mode = DEFAULT_MODE.to_string();
    state.repo = choose(&input.repo, Some(state.repo.as_str()));
    state.branch = choose(&input.branch, Some(state.branch.as_str()));
    state.task = choose(&input.task, Some(state.task.as_str()));
    state.updated_at = now;
    state.members.insert(actor_id, member);
    persist_room_state(store, &state)?;
    Ok(state)
}

pub fn room_status<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
) -> CoordinationResult<CoordinationRoomState> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let room_id = normalize_room_id(room_id);
    Ok(load_room(store, &tenant_slug, &room_id)?
        .unwrap_or_else(|| empty_room_state(&tenant_slug, &room_id, "")))
}

pub fn write_intent<S: GraphStore>(
    store: &mut S,
    input: WriteIntentInput,
) -> CoordinationResult<CoordinationIntentState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let room_id = normalize_room_id(&input.room_id);
    let actor_id = require_text("actor_id", &input.actor_id)?;
    let summary = require_text("summary", &input.summary)?;
    let status = normalize_status(&input.status)?;
    let now = timestamp_or_now(&input.updated_at);

    if load_room(store, &tenant_slug, &room_id)?.is_none() {
        persist_room_state(store, &empty_room_state(&tenant_slug, &room_id, &now))?;
    }

    let prior = load_intent(store, &tenant_slug, &room_id, &actor_id)?;
    let started_at = prior
        .as_ref()
        .map(|intent| intent.started_at.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| now.clone());

    let intent = CoordinationIntentState {
        tenant_slug,
        room_id,
        actor_id,
        status,
        summary,
        claimed_files: normalize_files(&input.claimed_files),
        expected_completion: input.expected_completion.trim().to_string(),
        repo: input.repo.trim().to_string(),
        branch: input.branch.trim().to_string(),
        task: input.task.trim().to_string(),
        started_at,
        updated_at: now,
    };
    persist_intent_state(store, &intent)?;
    Ok(intent)
}

pub fn read_intents_for_room<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
    statuses: &[String],
) -> CoordinationResult<Vec<CoordinationIntentState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let room_id = normalize_room_id(room_id);
    let status_filter = statuses
        .iter()
        .map(|status| status.trim().to_lowercase())
        .filter(|status| !status.is_empty())
        .collect::<BTreeSet<_>>();

    let mut intents = store
        .query_nodes(
            NodeQuery::label("CoordinationIntent")
                .with_property("tenant_slug", Value::String(tenant_slug))
                .with_property("room_id", Value::String(room_id)),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<CoordinationIntentState>(node.properties)
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .filter_map(|result| match result {
            Ok(intent) => {
                if status_filter.is_empty() || status_filter.contains(&intent.status) {
                    Some(Ok(intent))
                } else {
                    None
                }
            }
            Err(error) => Some(Err(error)),
        })
        .collect::<CoordinationResult<Vec<_>>>()?;
    intents.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.actor_id.cmp(&right.actor_id))
    });
    Ok(intents)
}

pub fn heartbeat_presence<S: GraphStore>(
    store: &mut S,
    input: PresenceInput,
) -> CoordinationResult<CoordinationPresenceState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let actor_id = require_text("actor_id", &input.actor_id)?;
    let ttl_seconds = input.ttl_seconds.max(1);
    let refreshed_at = timestamp_or_now(&input.refreshed_at);
    let expires_at = if input.expires_at.trim().is_empty() {
        refreshed_at.clone()
    } else {
        input.expires_at.trim().to_string()
    };
    let record = CoordinationPresenceState {
        tenant_slug,
        actor_id,
        session_id: input.session_id.trim().to_string(),
        surface: input.surface.trim().to_string(),
        status: if input.status.trim().is_empty() {
            "active".to_string()
        } else {
            input.status.trim().to_string()
        },
        worktree: input.worktree.trim().to_string(),
        branch: input.branch.trim().to_string(),
        head: input.head.trim().to_string(),
        changed_files: normalize_files(&input.changed_files),
        refreshed_at,
        expires_at,
        ttl_seconds,
    };
    persist_presence_state(store, &record)?;
    Ok(record)
}

pub fn end_presence<S: GraphStore>(
    store: &mut S,
    mut input: PresenceInput,
) -> CoordinationResult<CoordinationPresenceState> {
    input.status = "inactive".to_string();
    input.ttl_seconds = 1;
    heartbeat_presence(store, input)
}

pub fn load_presence<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    actor_id: &str,
) -> CoordinationResult<Option<CoordinationPresenceState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let actor_id = actor_id.trim();
    if actor_id.is_empty() {
        return Ok(None);
    }
    store
        .get_node(&coordination_presence_node_id(&tenant_slug, actor_id))
        .map(|node| {
            serde_json::from_value::<CoordinationPresenceState>(node.properties.clone())
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .transpose()
}

pub fn list_presence<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
) -> CoordinationResult<Vec<CoordinationPresenceState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let mut records = store
        .query_nodes(
            NodeQuery::label("CoordinationPresence")
                .with_property("tenant_slug", Value::String(tenant_slug)),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<CoordinationPresenceState>(node.properties)
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .collect::<CoordinationResult<Vec<_>>>()?;
    records.sort_by(|left, right| {
        (left.status != "active")
            .cmp(&(right.status != "active"))
            .then_with(|| left.actor_id.cmp(&right.actor_id))
    });
    Ok(records)
}

pub fn write_message<S: GraphStore>(
    store: &mut S,
    input: WriteMessageInput,
) -> CoordinationResult<CoordinationMessageState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let room_id = normalize_room_id(&input.room_id);
    let actor_id = require_text("actor_id", &input.actor_id)?;
    let message = require_text("message", &input.message)?;
    let urgency = normalize_urgency(&input.urgency)?;
    let created_at = timestamp_or_now(&input.created_at);
    let mentions = merge_mentions(parse_mentions(&message), normalize_files(&input.mentions));
    let message_id = if input.message_id.trim().is_empty() {
        stable_message_id(&tenant_slug, &room_id, &actor_id, &message, &created_at)
    } else {
        input.message_id.trim().to_string()
    };

    if load_room(store, &tenant_slug, &room_id)?.is_none() {
        persist_room_state(
            store,
            &empty_room_state(&tenant_slug, &room_id, &created_at),
        )?;
    }

    let state = CoordinationMessageState {
        tenant_slug,
        room_id,
        message_id,
        actor_id,
        urgency,
        message,
        mentions,
        metadata: input.metadata,
        consumed_by: Vec::new(),
        created_at,
    };
    persist_message_state(store, &state)?;
    Ok(state)
}

pub fn read_messages_for_room<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
    limit: usize,
) -> CoordinationResult<Vec<CoordinationMessageState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let room_id = normalize_room_id(room_id);
    let mut messages = store
        .query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(tenant_slug))
                .with_property("room_id", Value::String(room_id)),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<CoordinationMessageState>(node.properties)
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .collect::<CoordinationResult<Vec<_>>>()?;
    messages.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.message_id.cmp(&left.message_id))
    });
    if limit > 0 {
        messages.truncate(limit);
    }
    Ok(messages)
}

pub fn read_mentions_for_actor<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    actor_id: &str,
    consume: bool,
    limit: usize,
) -> CoordinationResult<Vec<CoordinationMessageState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let actor_id = require_text("actor_id", actor_id)?;
    let mut messages = store
        .query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(tenant_slug.clone())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<CoordinationMessageState>(node.properties)
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .filter_map(|result| match result {
            Ok(message)
                if message.mentions.iter().any(|mention| mention == &actor_id)
                    && !message
                        .consumed_by
                        .iter()
                        .any(|consumer| consumer == &actor_id) =>
            {
                Some(Ok(message))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<CoordinationResult<Vec<_>>>()?;
    messages.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.message_id.cmp(&right.message_id))
    });
    if limit > 0 {
        messages.truncate(limit);
    }
    if consume {
        for message in &messages {
            let mut consumed = message.clone();
            consumed.consumed_by = merge_mentions(consumed.consumed_by, vec![actor_id.clone()]);
            persist_message_state(store, &consumed)?;
        }
    }
    Ok(messages)
}

pub fn infer_coordination_room_id(
    repo: &str,
    branch: &str,
    task: &str,
    session_id: &str,
) -> String {
    let repo_leaf = repo.rsplit('/').next().unwrap_or(repo);
    let repo_part = slugify_room_part(repo_leaf);
    let branch_part = slugify_room_part(branch);
    let task_part = slugify_room_part(task);
    let session_part = slugify_room_part(session_id);
    if !repo_part.is_empty() && !branch_part.is_empty() {
        return format!("repo:{repo_part}:branch:{branch_part}");
    }
    if !repo_part.is_empty() && !task_part.is_empty() && task_part != "agent-session" {
        return format!("repo:{repo_part}:task:{task_part}");
    }
    if !repo_part.is_empty() {
        return format!("repo:{repo_part}");
    }
    if !session_part.is_empty() {
        return format!("session:{session_part}");
    }
    DEFAULT_ROOM.to_string()
}

pub fn coordination_room_node_id(tenant_slug: &str, room_id: &str) -> String {
    format!(
        "harness:coordination:room:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(room_id).if_empty("ungrouped")
    )
}

pub fn coordination_member_node_id(tenant_slug: &str, actor_id: &str) -> String {
    format!(
        "harness:coordination:member:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(actor_id).if_empty("unknown")
    )
}

pub fn coordination_member_edge_id(tenant_slug: &str, room_id: &str, actor_id: &str) -> String {
    format!(
        "harness:coordination:edge:member:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(room_id).if_empty("ungrouped"),
        slugify_room_part(actor_id).if_empty("unknown")
    )
}

pub fn coordination_intent_node_id(tenant_slug: &str, room_id: &str, actor_id: &str) -> String {
    format!(
        "harness:coordination:intent:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(room_id).if_empty("ungrouped"),
        slugify_room_part(actor_id).if_empty("unknown")
    )
}

pub fn coordination_presence_node_id(tenant_slug: &str, actor_id: &str) -> String {
    format!(
        "harness:coordination:presence:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(actor_id).if_empty("unknown")
    )
}

pub fn coordination_message_node_id(tenant_slug: &str, room_id: &str, message_id: &str) -> String {
    format!(
        "harness:coordination:message:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(room_id).if_empty("ungrouped"),
        slugify_room_part(message_id).if_empty("unknown")
    )
}

pub fn coordination_mention_edge_id(
    tenant_slug: &str,
    room_id: &str,
    message_id: &str,
    actor_id: &str,
) -> String {
    format!(
        "harness:coordination:edge:mention:{}:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify_room_part(room_id).if_empty("ungrouped"),
        slugify_room_part(message_id).if_empty("unknown"),
        slugify_room_part(actor_id).if_empty("unknown")
    )
}

fn persist_room_state<S: GraphStore>(
    store: &mut S,
    state: &CoordinationRoomState,
) -> CoordinationResult<()> {
    upsert_node_if_changed(store, room_node(state)?)?;
    for member in state.members.values() {
        upsert_node_if_changed(store, member_node(member)?)?;
        upsert_edge_if_changed(store, member_room_edge(member)?)?;
    }
    Ok(())
}

fn persist_intent_state<S: GraphStore>(
    store: &mut S,
    state: &CoordinationIntentState,
) -> CoordinationResult<()> {
    upsert_node_if_changed(store, intent_node(state)?)?;
    upsert_edge_if_changed(store, intent_room_edge(state)?)?;
    Ok(())
}

fn persist_presence_state<S: GraphStore>(
    store: &mut S,
    state: &CoordinationPresenceState,
) -> CoordinationResult<()> {
    upsert_node_if_changed(store, presence_node(state)?)?;
    Ok(())
}

fn persist_message_state<S: GraphStore>(
    store: &mut S,
    state: &CoordinationMessageState,
) -> CoordinationResult<()> {
    upsert_node_if_changed(store, message_node(state)?)?;
    upsert_edge_if_changed(store, message_room_edge(state)?)?;
    for actor_id in &state.mentions {
        let member = CoordinationRoomMember {
            tenant_slug: state.tenant_slug.clone(),
            room_id: state.room_id.clone(),
            actor_id: actor_id.clone(),
            status: "mentioned".to_string(),
            session_id: String::new(),
            surface: String::new(),
            repo: String::new(),
            branch: String::new(),
            task: String::new(),
            worktree: String::new(),
            head: String::new(),
            changed_files: Vec::new(),
            lane: String::new(),
            joined_at: String::new(),
            updated_at: state.created_at.clone(),
        };
        if store
            .get_node(&coordination_member_node_id(&state.tenant_slug, actor_id))
            .is_none()
        {
            upsert_node_if_changed(store, member_node(&member)?)?;
        }
        upsert_edge_if_changed(store, message_mention_edge(state, actor_id)?)?;
    }
    Ok(())
}

fn load_room<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
) -> CoordinationResult<Option<CoordinationRoomState>> {
    store
        .get_node(&coordination_room_node_id(tenant_slug, room_id))
        .map(|node| {
            serde_json::from_value::<CoordinationRoomState>(node.properties.clone())
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .transpose()
}

fn load_intent<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
    actor_id: &str,
) -> CoordinationResult<Option<CoordinationIntentState>> {
    store
        .get_node(&coordination_intent_node_id(tenant_slug, room_id, actor_id))
        .map(|node| {
            serde_json::from_value::<CoordinationIntentState>(node.properties.clone())
                .map_err(|error| CoordinationError::Deserialization(error.to_string()))
        })
        .transpose()
}

fn room_node(state: &CoordinationRoomState) -> CoordinationResult<NodeRecord> {
    let properties = serde_json::to_value(state)
        .map_err(|error| CoordinationError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        ["HarnessCoordination", "CoordinationRoom"],
        properties,
    ))
}

fn member_node(member: &CoordinationRoomMember) -> CoordinationResult<NodeRecord> {
    let properties = serde_json::to_value(member)
        .map_err(|error| CoordinationError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        coordination_member_node_id(&member.tenant_slug, &member.actor_id),
        ["HarnessCoordination", "CoordinationMember"],
        properties,
    ))
}

fn member_room_edge(member: &CoordinationRoomMember) -> CoordinationResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        coordination_member_edge_id(&member.tenant_slug, &member.room_id, &member.actor_id),
        coordination_member_node_id(&member.tenant_slug, &member.actor_id),
        "COORDINATION_MEMBER_OF",
        coordination_room_node_id(&member.tenant_slug, &member.room_id),
        json!({
            "tenant_slug": member.tenant_slug,
            "room_id": member.room_id,
            "actor_id": member.actor_id,
            "status": member.status,
            "updated_at": member.updated_at,
        }),
    ))
}

fn intent_node(state: &CoordinationIntentState) -> CoordinationResult<NodeRecord> {
    let properties = serde_json::to_value(state)
        .map_err(|error| CoordinationError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        coordination_intent_node_id(&state.tenant_slug, &state.room_id, &state.actor_id),
        ["HarnessCoordination", "CoordinationIntent"],
        properties,
    ))
}

fn intent_room_edge(state: &CoordinationIntentState) -> CoordinationResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:coordination:edge:intent:{}:{}:{}",
            normalize_tenant_slug(&state.tenant_slug),
            slugify_room_part(&state.room_id).if_empty("ungrouped"),
            slugify_room_part(&state.actor_id).if_empty("unknown")
        ),
        coordination_intent_node_id(&state.tenant_slug, &state.room_id, &state.actor_id),
        "COORDINATION_INTENT_OF",
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "room_id": state.room_id,
            "actor_id": state.actor_id,
            "status": state.status,
            "updated_at": state.updated_at,
        }),
    ))
}

fn presence_node(state: &CoordinationPresenceState) -> CoordinationResult<NodeRecord> {
    let properties = serde_json::to_value(state)
        .map_err(|error| CoordinationError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        coordination_presence_node_id(&state.tenant_slug, &state.actor_id),
        ["HarnessCoordination", "CoordinationPresence"],
        properties,
    ))
}

fn message_node(state: &CoordinationMessageState) -> CoordinationResult<NodeRecord> {
    let properties = serde_json::to_value(state)
        .map_err(|error| CoordinationError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        coordination_message_node_id(&state.tenant_slug, &state.room_id, &state.message_id),
        ["HarnessCoordination", "CoordinationMessage"],
        properties,
    ))
}

fn message_room_edge(state: &CoordinationMessageState) -> CoordinationResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:coordination:edge:message:{}:{}:{}",
            normalize_tenant_slug(&state.tenant_slug),
            slugify_room_part(&state.room_id).if_empty("ungrouped"),
            slugify_room_part(&state.message_id).if_empty("unknown")
        ),
        coordination_message_node_id(&state.tenant_slug, &state.room_id, &state.message_id),
        "COORDINATION_MESSAGE_OF",
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "room_id": state.room_id,
            "message_id": state.message_id,
            "actor_id": state.actor_id,
            "urgency": state.urgency,
            "created_at": state.created_at,
        }),
    ))
}

fn message_mention_edge(
    state: &CoordinationMessageState,
    actor_id: &str,
) -> CoordinationResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        coordination_mention_edge_id(
            &state.tenant_slug,
            &state.room_id,
            &state.message_id,
            actor_id,
        ),
        coordination_message_node_id(&state.tenant_slug, &state.room_id, &state.message_id),
        "COORDINATION_MENTIONS",
        coordination_member_node_id(&state.tenant_slug, actor_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "room_id": state.room_id,
            "message_id": state.message_id,
            "actor_id": actor_id,
            "urgency": state.urgency,
            "created_at": state.created_at,
        }),
    ))
}

fn empty_room_state(tenant_slug: &str, room_id: &str, now: &str) -> CoordinationRoomState {
    CoordinationRoomState {
        tenant_slug: normalize_tenant_slug(tenant_slug),
        room_id: normalize_room_id(room_id),
        status: "active".to_string(),
        mode: DEFAULT_MODE.to_string(),
        repo: String::new(),
        branch: String::new(),
        task: String::new(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        members: BTreeMap::new(),
        last_packet_at: String::new(),
        last_packet_doc_id: String::new(),
        degraded: false,
        degraded_reason: String::new(),
    }
}

fn normalize_status(status: &str) -> CoordinationResult<String> {
    let status = if status.trim().is_empty() {
        "working".to_string()
    } else {
        status.trim().to_lowercase()
    };
    if INTENT_STATUSES.contains(&status.as_str()) {
        Ok(status)
    } else {
        Err(CoordinationError::InvalidInput {
            field: "status".to_string(),
            message: format!("must be one of {:?}", INTENT_STATUSES),
        })
    }
}

fn normalize_urgency(urgency: &str) -> CoordinationResult<String> {
    let urgency = if urgency.trim().is_empty() {
        "info".to_string()
    } else {
        urgency.trim().to_lowercase()
    };
    if MESSAGE_URGENCIES.contains(&urgency.as_str()) {
        Ok(urgency)
    } else {
        Err(CoordinationError::InvalidInput {
            field: "urgency".to_string(),
            message: format!("must be one of {:?}", MESSAGE_URGENCIES),
        })
    }
}

fn resolve_room_id(
    room_id: &str,
    repo: &str,
    branch: &str,
    task: &str,
    session_id: &str,
) -> String {
    if !room_id.trim().is_empty() {
        return normalize_room_id(room_id);
    }
    infer_coordination_room_id(repo, branch, task, session_id)
}

fn normalize_room_id(room_id: &str) -> String {
    let room_id = room_id.trim();
    if room_id.is_empty() {
        DEFAULT_ROOM.to_string()
    } else {
        room_id.to_string()
    }
}

fn normalize_tenant_slug(tenant_slug: &str) -> String {
    let tenant_slug = tenant_slug.trim().to_lowercase();
    if tenant_slug.is_empty() {
        DEFAULT_TENANT.to_string()
    } else {
        tenant_slug
    }
}

fn require_text(field: &str, value: &str) -> CoordinationResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(CoordinationError::InvalidInput {
            field: field.to_string(),
            message: "is required".to_string(),
        })
    } else {
        Ok(value.to_string())
    }
}

fn choose(value: &str, existing: Option<&str>) -> String {
    let value = value.trim();
    if value.is_empty() {
        existing.unwrap_or("").trim().to_string()
    } else {
        value.to_string()
    }
}

fn choose_files(value: &[String], existing: Option<&[String]>) -> Vec<String> {
    let normalized = normalize_files(value);
    if normalized.is_empty() {
        existing.map(normalize_files).unwrap_or_default()
    } else {
        normalized
    }
}

fn normalize_files(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut files = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        files.push(value.to_string());
    }
    files
}

fn merge_mentions(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut merged = Vec::new();
    for value in left.into_iter().chain(right) {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        merged.push(value.to_string());
    }
    merged
}

fn parse_mentions(message: &str) -> Vec<String> {
    let mut mentions = Vec::new();
    let mut seen = BTreeSet::new();
    let bytes = message.as_bytes();
    let mut index = 0;
    let mut in_code = false;
    while index < bytes.len() {
        if bytes[index] == b'`' {
            in_code = !in_code;
            index += 1;
            continue;
        }
        if in_code || bytes[index] != b'@' {
            index += 1;
            continue;
        }
        let previous_is_word = index > 0
            && (bytes[index - 1].is_ascii_alphanumeric()
                || bytes[index - 1] == b'_'
                || bytes[index - 1] == b'-');
        if previous_is_word {
            index += 1;
            continue;
        }
        let start = index + 1;
        if start >= bytes.len() || !bytes[start].is_ascii_alphanumeric() {
            index += 1;
            continue;
        }
        let mut end = start;
        while end < bytes.len()
            && end - start < 120
            && (bytes[end].is_ascii_alphanumeric()
                || matches!(bytes[end], b'_' | b'.' | b':' | b'-'))
        {
            end += 1;
        }
        if let Some(actor_id) = message.get(start..end) {
            let actor_id = actor_id.trim();
            if !actor_id.is_empty() && seen.insert(actor_id.to_string()) {
                mentions.push(actor_id.to_string());
            }
        }
        index = end;
    }
    mentions
}

fn stable_message_id(
    tenant_slug: &str,
    room_id: &str,
    actor_id: &str,
    message: &str,
    created_at: &str,
) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": tenant_slug,
        "room_id": room_id,
        "actor_id": actor_id,
        "message": message,
        "created_at": created_at,
    }));
    format!("msg_{}", &hash[..16])
}

fn slugify_room_part(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in value.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
        if slug.len() >= 80 {
            break;
        }
    }
    slug.trim_matches('-').to_string()
}

fn timestamp_or_now(value: &str) -> String {
    let value = value.trim();
    if !value.is_empty() {
        return value.to_string();
    }
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("unix_ms:{millis}")
}

fn default_presence_ttl() -> u64 {
    DEFAULT_PRESENCE_TTL_SECONDS
}

fn upsert_node_if_changed<S: GraphStore>(store: &mut S, node: NodeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_node(&node.id)
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed<S: GraphStore>(store: &mut S, edge: EdgeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_edge(&edge.id)
        .map(|existing| {
            !existing.tombstone
                && existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.edge_type == edge.edge_type
                && existing.properties == edge.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_edge(edge)?;
    }
    Ok(())
}

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, RedCoreGraphStore, RedCoreOptions};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    const TENANT: &str = "travis-gilbert";
    const ROOM: &str = "harness-rust-port";
    const T1: &str = "2026-06-01T00:00:00+00:00";
    const T2: &str = "2026-06-01T00:01:00+00:00";

    #[test]
    fn join_room_persists_membership_and_graph_edges() {
        let mut store = InMemoryGraphStore::new();
        let state = join_room(
            &mut store,
            JoinRoomInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                room_id: ROOM.to_string(),
                repo: "Theorem".to_string(),
                branch: "main".to_string(),
                task: "Rust harness port".to_string(),
                changed_files: vec![
                    "rustyredcore_THG/crates/theorem-harness-runtime/src/coordination.rs"
                        .to_string(),
                ],
                updated_at: T1.to_string(),
                ..JoinRoomInput::default()
            },
        )
        .unwrap();

        assert_eq!(state.room_id, ROOM);
        assert_eq!(state.members.len(), 1);
        assert_eq!(state.members["codex"].joined_at, T1);
        assert!(store
            .get_node(&coordination_room_node_id(TENANT, ROOM))
            .is_some());
        assert!(store
            .get_node(&coordination_member_node_id(TENANT, "codex"))
            .is_some());
        assert!(store
            .get_edge(&coordination_member_edge_id(TENANT, ROOM, "codex"))
            .is_some());

        let loaded = room_status(&store, TENANT, ROOM).unwrap();
        assert_eq!(loaded.members["codex"].repo, "Theorem");
    }

    #[test]
    fn write_intent_replaces_live_actor_record_and_preserves_started_at() {
        let mut store = InMemoryGraphStore::new();
        let first = write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: ROOM.to_string(),
                actor_id: "codex".to_string(),
                status: "working".to_string(),
                summary: "Port coordination runtime".to_string(),
                claimed_files: vec!["src/coordination.rs".to_string()],
                updated_at: T1.to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();
        let second = write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: ROOM.to_string(),
                actor_id: "codex".to_string(),
                status: "done".to_string(),
                summary: "Coordination runtime landed".to_string(),
                claimed_files: Vec::new(),
                updated_at: T2.to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();

        assert_eq!(first.started_at, T1);
        assert_eq!(second.started_at, T1);
        assert_eq!(second.updated_at, T2);

        let all = read_intents_for_room(&store, TENANT, ROOM, &[]).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].status, "done");
        assert_eq!(all[0].summary, "Coordination runtime landed");

        let working =
            read_intents_for_room(&store, TENANT, ROOM, &["working".to_string()]).unwrap();
        assert!(working.is_empty());
        assert!(store
            .get_edge(&format!(
                "harness:coordination:edge:intent:{}:{}:{}",
                TENANT, ROOM, "codex"
            ))
            .is_some());
        assert!(store
            .get_node(&coordination_intent_node_id(TENANT, ROOM, "codex"))
            .is_some());
    }

    #[test]
    fn presence_heartbeat_and_end_are_durable_records() {
        let mut store = InMemoryGraphStore::new();
        let active = heartbeat_presence(
            &mut store,
            PresenceInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                status: "active".to_string(),
                worktree: "/repo/Theorem".to_string(),
                branch: "main".to_string(),
                head: "abc123".to_string(),
                changed_files: vec!["a.rs".to_string(), "a.rs".to_string()],
                refreshed_at: T1.to_string(),
                expires_at: T2.to_string(),
                ..PresenceInput::default()
            },
        )
        .unwrap();
        assert_eq!(active.changed_files, vec!["a.rs"]);

        let loaded = load_presence(&store, TENANT, "codex").unwrap().unwrap();
        assert_eq!(loaded.status, "active");
        assert_eq!(list_presence(&store, TENANT).unwrap().len(), 1);

        let inactive = end_presence(
            &mut store,
            PresenceInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                refreshed_at: T2.to_string(),
                ..PresenceInput::default()
            },
        )
        .unwrap();
        assert_eq!(inactive.status, "inactive");
        assert_eq!(inactive.ttl_seconds, 1);
    }

    #[test]
    fn write_message_parses_mentions_and_persists_edges() {
        let mut store = InMemoryGraphStore::new();
        let message = write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: ROOM.to_string(),
                actor_id: "codex".to_string(),
                urgency: "ask".to_string(),
                message: "@claude-code please review `@ignored` and @claude-ai".to_string(),
                mentions: vec!["claude-code".to_string(), "deepseek".to_string()],
                metadata: Map::from_iter([(
                    "commit".to_string(),
                    Value::String("abc123".to_string()),
                )]),
                created_at: T1.to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();

        assert_eq!(
            message.mentions,
            vec!["claude-code", "claude-ai", "deepseek"]
        );
        assert_eq!(message.urgency, "ask");
        assert!(store
            .get_node(&coordination_message_node_id(
                TENANT,
                ROOM,
                &message.message_id
            ))
            .is_some());
        assert!(store
            .get_edge(&coordination_mention_edge_id(
                TENANT,
                ROOM,
                &message.message_id,
                "claude-code"
            ))
            .is_some());

        let room_messages = read_messages_for_room(&store, TENANT, ROOM, 10).unwrap();
        assert_eq!(room_messages.len(), 1);
        assert_eq!(room_messages[0].metadata["commit"], "abc123");
    }

    #[test]
    fn read_mentions_can_consume_target_actor_inbox() {
        let mut store = InMemoryGraphStore::new();
        write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: ROOM.to_string(),
                actor_id: "codex".to_string(),
                message_id: "m1".to_string(),
                message: "@claude-code first".to_string(),
                created_at: T1.to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();
        write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: ROOM.to_string(),
                actor_id: "codex".to_string(),
                message_id: "m2".to_string(),
                message: "@claude-code second and @codex copied".to_string(),
                created_at: T2.to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();

        let peek = read_mentions_for_actor(&mut store, TENANT, "claude-code", false, 10).unwrap();
        assert_eq!(
            peek.iter()
                .map(|message| message.message_id.as_str())
                .collect::<Vec<_>>(),
            vec!["m1", "m2"]
        );

        let consumed =
            read_mentions_for_actor(&mut store, TENANT, "claude-code", true, 10).unwrap();
        assert_eq!(consumed.len(), 2);
        assert!(
            read_mentions_for_actor(&mut store, TENANT, "claude-code", false, 10)
                .unwrap()
                .is_empty()
        );

        let codex = read_mentions_for_actor(&mut store, TENANT, "codex", false, 10).unwrap();
        assert_eq!(codex.len(), 1);
        assert_eq!(codex[0].message_id, "m2");
    }

    #[test]
    fn redcore_reopens_coordination_room_intent_and_presence() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-harness-coordination-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            join_room(
                &mut store,
                JoinRoomInput {
                    tenant_slug: TENANT.to_string(),
                    actor_id: "codex".to_string(),
                    room_id: ROOM.to_string(),
                    updated_at: T1.to_string(),
                    ..JoinRoomInput::default()
                },
            )
            .unwrap();
            write_intent(
                &mut store,
                WriteIntentInput {
                    tenant_slug: TENANT.to_string(),
                    room_id: ROOM.to_string(),
                    actor_id: "codex".to_string(),
                    summary: "persist native coordination".to_string(),
                    updated_at: T1.to_string(),
                    ..WriteIntentInput::default()
                },
            )
            .unwrap();
            heartbeat_presence(
                &mut store,
                PresenceInput {
                    tenant_slug: TENANT.to_string(),
                    actor_id: "codex".to_string(),
                    refreshed_at: T1.to_string(),
                    expires_at: T2.to_string(),
                    ..PresenceInput::default()
                },
            )
            .unwrap();
            write_message(
                &mut store,
                WriteMessageInput {
                    tenant_slug: TENANT.to_string(),
                    room_id: ROOM.to_string(),
                    actor_id: "codex".to_string(),
                    message_id: "m-redcore".to_string(),
                    message: "@claude-code persisted message".to_string(),
                    created_at: T1.to_string(),
                    ..WriteMessageInput::default()
                },
            )
            .unwrap();
        }

        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let room = room_status(&store, TENANT, ROOM).unwrap();
            assert!(room.members.contains_key("codex"));
            let intents =
                read_intents_for_room(&store, TENANT, ROOM, &["working".to_string()]).unwrap();
            assert_eq!(intents.len(), 1);
            let presence = load_presence(&store, TENANT, "codex").unwrap().unwrap();
            assert_eq!(presence.status, "active");
            let messages = read_messages_for_room(&store, TENANT, ROOM, 10).unwrap();
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].message_id, "m-redcore");
        }

        let _ = fs::remove_dir_all(data_dir);
    }
}
