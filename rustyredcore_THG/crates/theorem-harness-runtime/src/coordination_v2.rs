//! Task-Reference Rooms: coordination v2 (TRR-001..TRR-010).
//!
//! Coordination addressed by task, not by a guessed room name. A [`TaskRef`] is a
//! content hash over normalized task metadata, so two heads (Codex, Claude)
//! resolve the same `task_ref_id` and `canonical_room_id` even when one points at
//! the in-repo spec path and the other at a downloaded copy. Around that anchor:
//!
//! - room aliases + permissive related-event routing, so a message written to the
//!   wrong room (e.g. `room:ungrouped`) still surfaces in the canonical room with
//!   provenance (TRR-002, TRR-009);
//! - explicit actor pings with pending/seen/consumed delivery state that reach a
//!   target even when it is not subscribed to any stream, and that can target a
//!   specific `actor + branch + worktree` checkout (TRR-003, TRR-006);
//! - a deterministic structured-claim contradiction pass that writes `CONTRADICTS`
//!   edges and room-visible contradiction events (TRR-007);
//! - turn-start discovery and a room digest that compose the above so a head sees
//!   the canonical room, inbox, open pings, active/stale intents, and
//!   contradictions before it edits (TRR-004, TRR-008);
//! - a repo-local `.harness/coordination.json` manifest so a cold head finds the
//!   canonical room without guessing (TRR-005).
//!
//! This is the engine layer over [`GraphStore`]. The MCP/console transport is a
//! separate mirror (the MCP crate carries its own `McpGraphBackend` persistence,
//! the same split the stream-coordination work shipped under).

use crate::coordination::{
    empty_room_state, infer_coordination_room_id, normalize_room_id, require_actor_id,
    require_tenant_slug, slugify_room_part, timestamp_or_now, CoordinationError,
    CoordinationIntentState, CoordinationMessageState, CoordinationPresenceState,
    CoordinationResult, DEFAULT_ROOM,
};
use crate::tenant::{normalize_actor_id, normalize_tenant_slug, tenant_slug_aliases};
use rustyred_thg_core::{EdgeRecord, GraphStore, GraphStoreResult, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use theorem_harness_core::stable_value_hash;

const PING_URGENCIES: &[&str] = &["ask", "block"];
pub const PING_PENDING: &str = "pending";
pub const PING_SEEN: &str = "seen";
pub const PING_CONSUMED: &str = "consumed";
const DEFAULT_STALE_AFTER_MS: u64 = 30 * 60 * 1000;

// ---------------------------------------------------------------------------
// Store seam.
// ---------------------------------------------------------------------------

/// The narrow store surface coordination v2 runs on: owned, fallible node/edge
/// access. Blanket-impl'd for any [`GraphStore`] (the runtime + test path) and
/// adapter-impl'd in the MCP crate over `McpGraphBackend`, so the one engine
/// serves both transports without a second persistence implementation. Mirrors
/// the `MemoryGraphStore` seam pattern.
pub trait CoordinationStore {
    fn coord_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn coord_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn coord_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn coord_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()>;
    fn coord_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()>;
}

impl<S: GraphStore> CoordinationStore for S {
    fn coord_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        Ok(GraphStore::get_node(self, id).cloned())
    }
    fn coord_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        Ok(GraphStore::get_edge(self, id).cloned())
    }
    fn coord_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(GraphStore::query_nodes(self, query))
    }
    fn coord_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        GraphStore::upsert_node(self, node).map(|_| ())
    }
    fn coord_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        GraphStore::upsert_edge(self, edge).map(|_| ())
    }
}

// ---------------------------------------------------------------------------
// TRR-001: TaskRef resolution (pure).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskRefConfidence {
    Exact,
    Strong,
    Weak,
    Ambiguous,
}

impl TaskRefConfidence {
    pub fn as_str(self) -> &'static str {
        match self {
            TaskRefConfidence::Exact => "exact",
            TaskRefConfidence::Strong => "strong",
            TaskRefConfidence::Weak => "weak",
            TaskRefConfidence::Ambiguous => "ambiguous",
        }
    }
}

/// Normalized task metadata. The resolver keys on the stable fields, so path
/// variants of the same spec collapse to one identity.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskRefInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub workstream: String,
    #[serde(default)]
    pub spec_refs: Vec<String>,
    #[serde(default)]
    pub external_refs: Vec<String>,
    #[serde(default)]
    pub branch: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RoomAlias {
    pub tenant_slug: String,
    pub from_room_id: String,
    pub canonical_room_id: String,
    pub task_ref_id: String,
    pub confidence: String,
    pub reason: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TaskRef {
    pub task_ref_id: String,
    pub canonical_room_id: String,
    pub confidence: TaskRefConfidence,
    pub aliases: Vec<RoomAlias>,
    pub spec_tokens: Vec<String>,
    pub tenant_slug: String,
    pub repo: String,
    pub branch: String,
    pub workstream: String,
}

/// Resolve task metadata into the stable coordination address. Pure and
/// deterministic: identical normalized metadata yields an identical id on any
/// head.
pub fn resolve_task_ref(input: &TaskRefInput) -> TaskRef {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let repo = input.repo.trim().to_lowercase();
    let branch = input.branch.trim().to_lowercase();
    let workstream = slugify_room_part(&input.workstream);

    let mut spec_tokens: Vec<String> = input
        .spec_refs
        .iter()
        .chain(input.external_refs.iter())
        .filter_map(|path| {
            let token = spec_token(path);
            (!token.is_empty()).then_some(token)
        })
        .collect();
    spec_tokens.sort();
    spec_tokens.dedup();

    let task_ref_id = format!(
        "task_{}",
        &stable_value_hash(&json!({
            "tenant": tenant_slug,
            "repo": repo,
            "workstream": workstream,
            "spec_tokens": spec_tokens,
            "branch": branch,
        }))[..16]
    );

    let canonical_room_id = canonical_room_for(&repo, &branch, &workstream, &spec_tokens);
    let confidence = confidence_for(&repo, &branch, &workstream, &spec_tokens, &canonical_room_id);
    let aliases = derive_aliases(
        &tenant_slug,
        &task_ref_id,
        &repo,
        &branch,
        &workstream,
        &spec_tokens,
        &canonical_room_id,
        confidence,
    );

    TaskRef {
        task_ref_id,
        canonical_room_id,
        confidence,
        aliases,
        spec_tokens,
        tenant_slug,
        repo,
        branch,
        workstream,
    }
}

/// The stable spec identity shared by any path that names the same file: the
/// extension-stripped, slugified basename. `docs/plans/x/SPEC-9-foo.md` and
/// `/Users/me/Downloads/SPEC-9-foo.md` both become `spec-9-foo`.
fn spec_token(path: &str) -> String {
    let path = path.trim();
    if path.is_empty() {
        return String::new();
    }
    let leaf = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let stem = leaf.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(leaf);
    slugify_room_part(stem)
}

fn repo_leaf_slug(repo: &str) -> String {
    slugify_room_part(repo.rsplit('/').next().unwrap_or(repo))
}

fn canonical_room_for(
    repo: &str,
    branch: &str,
    workstream: &str,
    spec_tokens: &[String],
) -> String {
    if let Some(token) = spec_tokens.first() {
        return format!("task:{token}");
    }
    if !workstream.is_empty() {
        return format!("task:{workstream}");
    }
    let inferred = infer_coordination_room_id(repo, branch, "", "");
    if inferred != DEFAULT_ROOM {
        return inferred;
    }
    DEFAULT_ROOM.to_string()
}

fn confidence_for(
    repo: &str,
    branch: &str,
    workstream: &str,
    spec_tokens: &[String],
    canonical: &str,
) -> TaskRefConfidence {
    if canonical == DEFAULT_ROOM {
        return TaskRefConfidence::Ambiguous;
    }
    let has_repo = !repo.is_empty();
    let has_branch = !branch.is_empty();
    let has_spec = !spec_tokens.is_empty();
    let has_workstream = !workstream.is_empty();
    if has_spec && has_repo && has_branch && has_workstream {
        TaskRefConfidence::Exact
    } else if has_repo && (has_branch || has_spec || has_workstream) {
        TaskRefConfidence::Strong
    } else {
        TaskRefConfidence::Weak
    }
}

#[allow(clippy::too_many_arguments)]
fn derive_aliases(
    tenant_slug: &str,
    task_ref_id: &str,
    repo: &str,
    branch: &str,
    workstream: &str,
    spec_tokens: &[String],
    canonical: &str,
    confidence: TaskRefConfidence,
) -> Vec<RoomAlias> {
    let mut candidates: Vec<(String, &str)> = Vec::new();
    if !repo.is_empty() {
        candidates.push((
            infer_coordination_room_id(repo, branch, "", ""),
            "legacy-inferred-room",
        ));
        candidates.push((format!("repo:{}", repo_leaf_slug(repo)), "repo-room"));
    }
    if !branch.is_empty() {
        candidates.push((format!("branch:{}", slugify_room_part(branch)), "branch-room"));
    }
    if !workstream.is_empty() {
        candidates.push((format!("task:{workstream}"), "workstream-room"));
    }
    for token in spec_tokens {
        candidates.push((format!("spec:{token}"), "spec-room"));
    }

    let mut seen = BTreeSet::new();
    let mut aliases = Vec::new();
    for (from_room_id, reason) in candidates {
        let from_room_id = from_room_id.trim().to_string();
        if from_room_id.is_empty() || from_room_id == canonical || !seen.insert(from_room_id.clone())
        {
            continue;
        }
        aliases.push(RoomAlias {
            tenant_slug: tenant_slug.to_string(),
            from_room_id,
            canonical_room_id: canonical.to_string(),
            task_ref_id: task_ref_id.to_string(),
            confidence: confidence.as_str().to_string(),
            reason: reason.to_string(),
            created_at: String::new(),
        });
    }
    aliases
}

// ---------------------------------------------------------------------------
// TRR-002 / TRR-009: room aliases + permissive related-event routing.
// ---------------------------------------------------------------------------

/// Persist a resolved [`TaskRef`]'s aliases so an old/known room id resolves to
/// the canonical room without guessing. Idempotent.
pub fn register_task_ref<S: CoordinationStore>(
    store: &mut S,
    task_ref: &TaskRef,
    created_at: &str,
) -> CoordinationResult<()> {
    let created_at = timestamp_or_now(created_at);
    for alias in &task_ref.aliases {
        let mut alias = alias.clone();
        if alias.created_at.trim().is_empty() {
            alias.created_at = created_at.clone();
        }
        persist_room_alias(store, &alias)?;
    }
    Ok(())
}

/// Map any room id to its canonical room, following a registered alias if one
/// exists. Backfill-safe: an unaliased room id resolves to itself with no data
/// loss (TRR-009).
pub fn resolve_canonical_room<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
) -> CoordinationResult<(String, Option<RoomAlias>)> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let room_id = normalize_room_id(room_id);
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        if let Some(node) = store.coord_get_node(&room_alias_node_id(&tenant_alias, &room_id))? {
            let alias: RoomAlias = deserialize(node.properties)?;
            return Ok((alias.canonical_room_id.clone(), Some(alias)));
        }
    }
    Ok((room_id, None))
}

/// A message that landed in a non-canonical room, attached to the canonical room
/// as a related event with provenance.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RelatedEvent {
    pub tenant_slug: String,
    pub canonical_room_id: String,
    pub event_id: String,
    pub task_ref_id: String,
    pub origin_room_id: String,
    pub origin_message_id: String,
    pub actor_id: String,
    pub summary: String,
    #[serde(default)]
    pub urgency: String,
    #[serde(default)]
    pub confidence: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AttachRelatedInput {
    pub tenant_slug: String,
    pub task_ref_id: String,
    pub canonical_room_id: String,
    pub origin_room_id: String,
    pub origin_message_id: String,
    pub actor_id: String,
    pub summary: String,
    pub urgency: String,
    pub confidence: String,
    pub reason: String,
    pub created_at: String,
}

/// Attach a related event to a canonical room. Idempotent in the origin message.
pub fn attach_related_event<S: CoordinationStore>(
    store: &mut S,
    input: AttachRelatedInput,
) -> CoordinationResult<RelatedEvent> {
    let tenant_slug = require_tenant_slug(&input.tenant_slug)?;
    let canonical_room_id = normalize_room_id(&input.canonical_room_id);
    let origin_room_id = normalize_room_id(&input.origin_room_id);
    let actor_id = require_actor_id("actor_id", &input.actor_id)?;
    let created_at = timestamp_or_now(&input.created_at);
    let event_id = format!(
        "rel_{}",
        &stable_value_hash(&json!({
            "tenant": tenant_slug,
            "canonical": canonical_room_id,
            "origin_room": origin_room_id,
            "origin_message": input.origin_message_id.trim(),
            "actor": actor_id,
        }))[..16]
    );
    let event = RelatedEvent {
        tenant_slug,
        canonical_room_id,
        event_id,
        task_ref_id: input.task_ref_id.trim().to_string(),
        origin_room_id,
        origin_message_id: input.origin_message_id.trim().to_string(),
        actor_id,
        summary: input.summary.trim().to_string(),
        urgency: input.urgency.trim().to_lowercase(),
        confidence: input.confidence.trim().to_lowercase(),
        reason: input.reason.trim().to_string(),
        created_at,
    };
    persist_related_event(store, &event)?;
    Ok(event)
}

/// Route a written message to its task's canonical room if it landed elsewhere.
/// Returns the related event when routing happened, `None` when the message is
/// already in the canonical room. This is the permissive inbox behavior for
/// `room:ungrouped` and any off-canonical room.
pub fn route_message_to_task<S: CoordinationStore>(
    store: &mut S,
    message: &CoordinationMessageState,
    task: &TaskRefInput,
) -> CoordinationResult<Option<RelatedEvent>> {
    let task_ref = resolve_task_ref(task);
    let origin_room = normalize_room_id(&message.room_id);
    if origin_room == task_ref.canonical_room_id {
        return Ok(None);
    }
    let event = attach_related_event(
        store,
        AttachRelatedInput {
            tenant_slug: task_ref.tenant_slug.clone(),
            task_ref_id: task_ref.task_ref_id.clone(),
            canonical_room_id: task_ref.canonical_room_id.clone(),
            origin_room_id: origin_room,
            origin_message_id: message.message_id.clone(),
            actor_id: message.actor_id.clone(),
            summary: truncate(&message.message, 280),
            urgency: message.urgency.clone(),
            confidence: task_ref.confidence.as_str().to_string(),
            reason: "task-metadata-match".to_string(),
            created_at: message.created_at.clone(),
        },
    )?;
    Ok(Some(event))
}

/// All related events attached to a canonical room, newest first.
pub fn read_related_events<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    canonical_room_id: &str,
    limit: usize,
) -> CoordinationResult<Vec<RelatedEvent>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let canonical_room_id = normalize_room_id(canonical_room_id);
    let mut events = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationRelatedEvent")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property(
                    "canonical_room_id",
                    Value::String(canonical_room_id.clone()),
                ),
        )? {
            events.push(deserialize::<RelatedEvent>(node.properties)?);
        }
    }
    events.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.event_id.cmp(&left.event_id))
    });
    truncate_vec(&mut events, limit);
    Ok(events)
}

// ---------------------------------------------------------------------------
// TRR-003 / TRR-006: explicit pings with delivery state + checkout targeting.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorPing {
    pub tenant_slug: String,
    pub ping_id: String,
    #[serde(default)]
    pub task_ref_id: String,
    #[serde(default)]
    pub room_id: String,
    #[serde(default)]
    pub from_actor: String,
    pub target_actor: String,
    #[serde(default)]
    pub target_branch: String,
    #[serde(default)]
    pub target_worktree: String,
    pub urgency: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub event_id: String,
    pub status: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub seen_at: String,
    #[serde(default)]
    pub consumed_at: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PingInput {
    pub tenant_slug: String,
    pub task_ref_id: String,
    pub room_id: String,
    pub from_actor: String,
    pub target_actor: String,
    pub target_branch: String,
    pub target_worktree: String,
    pub urgency: String,
    pub message: String,
    pub event_id: String,
    pub created_at: String,
}

/// Create an actor ping. Only `ask`/`block` urgency produces a ping; passive
/// progress stays stream-only. The ping is an actor-targeted mailbox item: it
/// reaches the target whether or not the target subscribes to any stream.
pub fn create_ping<S: CoordinationStore>(
    store: &mut S,
    input: PingInput,
) -> CoordinationResult<ActorPing> {
    let tenant_slug = require_tenant_slug(&input.tenant_slug)?;
    let target_actor = require_actor_id("target_actor", &input.target_actor)?;
    let urgency = normalize_ping_urgency(&input.urgency)?;
    let target_branch = input.target_branch.trim().to_string();
    let target_worktree = input.target_worktree.trim().to_string();
    let created_at = timestamp_or_now(&input.created_at);
    let ping_id = format!(
        "ping_{}",
        &stable_value_hash(&json!({
            "tenant": tenant_slug,
            "target": target_actor,
            "branch": target_branch,
            "worktree": target_worktree,
            "event": input.event_id.trim(),
            "message": input.message.trim(),
        }))[..16]
    );
    let ping = ActorPing {
        tenant_slug,
        ping_id,
        task_ref_id: input.task_ref_id.trim().to_string(),
        room_id: normalize_room_id(&input.room_id),
        from_actor: normalize_actor_id(&input.from_actor),
        target_actor,
        target_branch,
        target_worktree,
        urgency,
        message: input.message.trim().to_string(),
        event_id: input.event_id.trim().to_string(),
        status: PING_PENDING.to_string(),
        created_at,
        seen_at: String::new(),
        consumed_at: String::new(),
    };
    persist_ping(store, &ping)?;
    Ok(ping)
}

/// Does this ping target the given checkout? A ping with empty branch/worktree
/// targets any checkout of the actor; a ping that named a branch/worktree only
/// matches that exact checkout (TRR-006).
pub fn ping_targets_checkout(ping: &ActorPing, actor: &str, branch: &str, worktree: &str) -> bool {
    if normalize_actor_id(&ping.target_actor) != normalize_actor_id(actor) {
        return false;
    }
    let branch_ok =
        ping.target_branch.is_empty() || ping.target_branch == branch.trim();
    let worktree_ok =
        ping.target_worktree.is_empty() || ping.target_worktree == worktree.trim();
    branch_ok && worktree_ok
}

/// Read an actor's open (not-consumed) pings. When a checkout is supplied, pings
/// that named a different branch/worktree are filtered out: a different checkout
/// sees that it is not the intended target. With `mark_seen`, pending pings
/// transition to seen.
pub fn read_open_pings_for_actor<S: CoordinationStore>(
    store: &mut S,
    tenant_slug: &str,
    target_actor: &str,
    checkout: Option<(&str, &str)>,
    mark_seen: bool,
    limit: usize,
) -> CoordinationResult<Vec<ActorPing>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let target_actor = require_actor_id("target_actor", target_actor)?;
    let mut pings: Vec<ActorPing> = query_pings(store, &tenant_slug)?
        .into_iter()
        .filter(|ping| {
            ping.status != PING_CONSUMED
                && normalize_actor_id(&ping.target_actor) == target_actor
                && checkout
                    .map(|(branch, worktree)| ping_targets_checkout(ping, &target_actor, branch, worktree))
                    .unwrap_or(true)
        })
        .collect();
    pings.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.ping_id.cmp(&right.ping_id))
    });
    truncate_vec(&mut pings, limit);
    if mark_seen {
        for ping in pings.iter_mut() {
            if ping.status == PING_PENDING {
                ping.status = PING_SEEN.to_string();
                ping.seen_at = timestamp_or_now("");
                persist_ping(store, ping)?;
            }
        }
    }
    Ok(pings)
}

/// Open pings scoped to a task/room, across actors (for the digest).
pub fn read_pending_pings_for_task<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    task_ref_id: &str,
    limit: usize,
) -> CoordinationResult<Vec<ActorPing>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let task_ref_id = task_ref_id.trim();
    let mut pings: Vec<ActorPing> = query_pings(store, &tenant_slug)?
        .into_iter()
        .filter(|ping| ping.status != PING_CONSUMED && ping.task_ref_id == task_ref_id)
        .collect();
    pings.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.ping_id.cmp(&right.ping_id))
    });
    truncate_vec(&mut pings, limit);
    Ok(pings)
}

/// Consume a ping (the target acted on it).
pub fn consume_ping<S: CoordinationStore>(
    store: &mut S,
    tenant_slug: &str,
    ping_id: &str,
) -> CoordinationResult<Option<ActorPing>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        let props = match store.coord_get_node(&ping_node_id(&tenant_alias, ping_id))? {
            Some(node) => node.properties,
            None => continue,
        };
        let mut ping: ActorPing = deserialize(props)?;
        ping.status = PING_CONSUMED.to_string();
        ping.consumed_at = timestamp_or_now("");
        persist_ping(store, &ping)?;
        return Ok(Some(ping));
    }
    Ok(None)
}

fn query_pings<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
) -> CoordinationResult<Vec<ActorPing>> {
    let mut pings = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationActorPing")
                .with_property("tenant_slug", Value::String(tenant_alias)),
        )? {
            pings.push(deserialize::<ActorPing>(node.properties)?);
        }
    }
    Ok(pings)
}

fn normalize_ping_urgency(urgency: &str) -> CoordinationResult<String> {
    let urgency = urgency.trim().to_lowercase();
    if PING_URGENCIES.contains(&urgency.as_str()) {
        Ok(urgency)
    } else {
        Err(CoordinationError::InvalidInput {
            field: "urgency".to_string(),
            message: format!("a ping requires urgency one of {PING_URGENCIES:?}"),
        })
    }
}

// ---------------------------------------------------------------------------
// TRR-007: structured-claim contradiction model.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Claim {
    pub tenant_slug: String,
    pub claim_id: String,
    pub task_ref_id: String,
    #[serde(default)]
    pub room_id: String,
    pub actor: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub superseded: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClaimInput {
    pub tenant_slug: String,
    pub task_ref_id: String,
    pub room_id: String,
    pub actor: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub created_at: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Contradiction {
    pub tenant_slug: String,
    pub contradiction_id: String,
    pub task_ref_id: String,
    #[serde(default)]
    pub room_id: String,
    pub subject: String,
    pub predicate: String,
    pub left_claim_id: String,
    pub right_claim_id: String,
    pub left_object: String,
    pub right_object: String,
    pub left_actor: String,
    pub right_actor: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub resolved: bool,
}

/// Record a structured claim and run the deterministic contradiction pass. A
/// new claim supersedes the same actor's prior claim on the same
/// subject/predicate; a conflicting object from another live claim writes a
/// `CONTRADICTS` edge and a room-visible contradiction event. Conservative v1
/// rule: same subject + same predicate + different object.
pub fn record_claim<S: CoordinationStore>(
    store: &mut S,
    input: ClaimInput,
) -> CoordinationResult<(Claim, Vec<Contradiction>)> {
    let tenant_slug = require_tenant_slug(&input.tenant_slug)?;
    let actor = require_actor_id("actor", &input.actor)?;
    let task_ref_id = require_nonempty("task_ref_id", &input.task_ref_id)?;
    let subject = require_nonempty("subject", &input.subject)?;
    let predicate = require_nonempty("predicate", &input.predicate)?;
    let object = require_nonempty("object", &input.object)?;
    let room_id = normalize_room_id(&input.room_id);
    let created_at = timestamp_or_now(&input.created_at);

    let (subject_key, predicate_key, object_key) =
        (norm_key(&subject), norm_key(&predicate), norm_key(&object));
    let claim_id = format!(
        "claim_{}",
        &stable_value_hash(&json!({
            "tenant": tenant_slug,
            "task": task_ref_id,
            "actor": actor,
            "subject": subject_key,
            "predicate": predicate_key,
            "object": object_key,
        }))[..16]
    );

    let existing = read_claims_for_task(store, &tenant_slug, &task_ref_id)?;

    // Supersede this actor's prior live claim on the same subject/predicate.
    for prior in &existing {
        if prior.superseded
            || prior.claim_id == claim_id
            || norm_key(&prior.actor) != norm_key(&actor)
            || norm_key(&prior.subject) != subject_key
            || norm_key(&prior.predicate) != predicate_key
        {
            continue;
        }
        let mut superseded = prior.clone();
        superseded.superseded = true;
        persist_claim(store, &superseded)?;
        resolve_contradictions_for_claim(store, &tenant_slug, &task_ref_id, &superseded.claim_id)?;
    }

    let claim = Claim {
        tenant_slug: tenant_slug.clone(),
        claim_id: claim_id.clone(),
        task_ref_id: task_ref_id.clone(),
        room_id: room_id.clone(),
        actor: actor.clone(),
        subject: subject.clone(),
        predicate: predicate.clone(),
        object: object.clone(),
        created_at: created_at.clone(),
        superseded: false,
    };
    persist_claim(store, &claim)?;

    // Detect against the live claim set (re-read so the supersessions above are
    // visible: a superseded prior never re-contradicts).
    let live = read_claims_for_task(store, &tenant_slug, &task_ref_id)?;
    let mut contradictions = Vec::new();
    for other in &live {
        if other.superseded
            || other.claim_id == claim_id
            || norm_key(&other.subject) != subject_key
            || norm_key(&other.predicate) != predicate_key
            || norm_key(&other.object) == object_key
        {
            continue;
        }
        let contradiction = build_contradiction(&claim, other, &room_id, &created_at);
        persist_contradiction(store, &contradiction)?;
        upsert_edge_if_changed(store, contradicts_edge(&contradiction)?)?;
        contradictions.push(contradiction);
    }
    Ok((claim, contradictions))
}

/// All claims recorded for a task.
pub fn read_claims_for_task<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    task_ref_id: &str,
) -> CoordinationResult<Vec<Claim>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let task_ref_id = task_ref_id.trim();
    let mut claims = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationClaim")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("task_ref_id", Value::String(task_ref_id.to_string())),
        )? {
            claims.push(deserialize::<Claim>(node.properties)?);
        }
    }
    claims.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.claim_id.cmp(&right.claim_id))
    });
    Ok(claims)
}

/// Open (unresolved) contradictions for a task.
pub fn read_open_contradictions<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    task_ref_id: &str,
) -> CoordinationResult<Vec<Contradiction>> {
    Ok(read_contradictions(store, tenant_slug, task_ref_id)?
        .into_iter()
        .filter(|contradiction| !contradiction.resolved)
        .collect())
}

fn read_contradictions<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    task_ref_id: &str,
) -> CoordinationResult<Vec<Contradiction>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let task_ref_id = task_ref_id.trim();
    let mut contradictions = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationContradiction")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("task_ref_id", Value::String(task_ref_id.to_string())),
        )? {
            contradictions.push(deserialize::<Contradiction>(node.properties)?);
        }
    }
    contradictions.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.contradiction_id.cmp(&left.contradiction_id))
    });
    Ok(contradictions)
}

fn resolve_contradictions_for_claim<S: CoordinationStore>(
    store: &mut S,
    tenant_slug: &str,
    task_ref_id: &str,
    claim_id: &str,
) -> CoordinationResult<()> {
    for contradiction in read_contradictions(store, tenant_slug, task_ref_id)? {
        if contradiction.resolved
            || (contradiction.left_claim_id != claim_id
                && contradiction.right_claim_id != claim_id)
        {
            continue;
        }
        let mut resolved = contradiction.clone();
        resolved.resolved = true;
        persist_contradiction(store, &resolved)?;
    }
    Ok(())
}

fn build_contradiction(
    claim: &Claim,
    other: &Claim,
    room_id: &str,
    created_at: &str,
) -> Contradiction {
    // Order claim ids deterministically so the same pair yields one id.
    let (left, right) = if claim.claim_id <= other.claim_id {
        (claim, other)
    } else {
        (other, claim)
    };
    let contradiction_id = format!(
        "contra_{}",
        &stable_value_hash(&json!({
            "tenant": claim.tenant_slug,
            "task": claim.task_ref_id,
            "subject": norm_key(&claim.subject),
            "predicate": norm_key(&claim.predicate),
            "left": left.claim_id,
            "right": right.claim_id,
        }))[..16]
    );
    Contradiction {
        tenant_slug: claim.tenant_slug.clone(),
        contradiction_id,
        task_ref_id: claim.task_ref_id.clone(),
        room_id: room_id.to_string(),
        subject: claim.subject.clone(),
        predicate: claim.predicate.clone(),
        left_claim_id: left.claim_id.clone(),
        right_claim_id: right.claim_id.clone(),
        left_object: left.object.clone(),
        right_object: right.object.clone(),
        left_actor: left.actor.clone(),
        right_actor: right.actor.clone(),
        created_at: created_at.to_string(),
        resolved: false,
    }
}

// ---------------------------------------------------------------------------
// TRR-004: turn-start discovery.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiscoveryInput {
    pub task: TaskRefInput,
    pub actor: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub stale_after_ms: u64,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TurnStartDiscovery {
    pub task_ref_id: String,
    pub canonical_room_id: String,
    pub confidence: TaskRefConfidence,
    pub related_inbox: Vec<RelatedEvent>,
    pub open_pings: Vec<ActorPing>,
    pub active_intents: Vec<CoordinationIntentState>,
    pub stale_intents: Vec<CoordinationIntentState>,
    pub contradictions: Vec<Contradiction>,
}

/// What a head should see before it edits: the canonical room, inbox messages
/// routed in from aliases, its own open pings, active vs stale intents, and open
/// contradictions for its task. A pure read: it does not change ping delivery
/// state (use `read_open_pings_for_actor`/`consume_ping` for that).
pub fn turn_start_discovery<S: CoordinationStore>(
    store: &mut S,
    input: &DiscoveryInput,
) -> CoordinationResult<TurnStartDiscovery> {
    let task_ref = resolve_task_ref(&input.task);
    let tenant_slug = task_ref.tenant_slug.clone();
    let canonical = task_ref.canonical_room_id.clone();
    let limit = if input.limit == 0 { 50 } else { input.limit };
    let stale_after_ms = if input.stale_after_ms == 0 {
        DEFAULT_STALE_AFTER_MS
    } else {
        input.stale_after_ms
    };
    let now = timestamp_or_now(&input.now);

    let related_inbox = read_related_events(store, &tenant_slug, &canonical, limit)?;
    let contradictions = read_open_contradictions(store, &tenant_slug, &task_ref.task_ref_id)?;
    let all_intents = read_intents(store, &tenant_slug, &canonical)?;
    let (active_intents, stale_intents) = split_intents(all_intents, &now, stale_after_ms);
    let open_pings = read_open_pings_for_actor(
        store,
        &tenant_slug,
        &input.actor,
        Some((&input.branch, &input.worktree)),
        false,
        limit,
    )?;

    Ok(TurnStartDiscovery {
        task_ref_id: task_ref.task_ref_id,
        canonical_room_id: canonical,
        confidence: task_ref.confidence,
        related_inbox,
        open_pings,
        active_intents,
        stale_intents,
        contradictions,
    })
}

fn split_intents(
    intents: Vec<CoordinationIntentState>,
    now: &str,
    stale_after_ms: u64,
) -> (Vec<CoordinationIntentState>, Vec<CoordinationIntentState>) {
    let mut active = Vec::new();
    let mut stale = Vec::new();
    for intent in intents {
        let stamp = if intent.updated_at.trim().is_empty() {
            intent.started_at.as_str()
        } else {
            intent.updated_at.as_str()
        };
        if is_stale(stamp, now, stale_after_ms) {
            stale.push(intent);
        } else {
            active.push(intent);
        }
    }
    (active, stale)
}

// ---------------------------------------------------------------------------
// TRR-008: room dashboard digest.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DigestInput {
    pub task: TaskRefInput,
    #[serde(default)]
    pub now: String,
    #[serde(default)]
    pub stale_after_ms: u64,
    #[serde(default)]
    pub limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ActorActivity {
    pub actor_id: String,
    pub last_seen: String,
    pub source: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RoomDigest {
    pub task_ref_id: String,
    pub canonical_room_id: String,
    pub confidence: TaskRefConfidence,
    pub aliases: Vec<RoomAlias>,
    pub active_actors: Vec<ActorActivity>,
    pub stale_actors: Vec<ActorActivity>,
    pub pending_pings: Vec<ActorPing>,
    pub related_messages: Vec<RelatedEvent>,
    pub contradictions: Vec<Contradiction>,
}

/// A room-level snapshot for a human/console dashboard: canonical room, aliases,
/// active vs stale actors, pending pings, related ungrouped messages, and open
/// contradictions.
pub fn room_digest<S: CoordinationStore>(
    store: &S,
    input: &DigestInput,
) -> CoordinationResult<RoomDigest> {
    let task_ref = resolve_task_ref(&input.task);
    let tenant_slug = task_ref.tenant_slug.clone();
    let canonical = task_ref.canonical_room_id.clone();
    let limit = if input.limit == 0 { 50 } else { input.limit };
    let stale_after_ms = if input.stale_after_ms == 0 {
        DEFAULT_STALE_AFTER_MS
    } else {
        input.stale_after_ms
    };
    let now = timestamp_or_now(&input.now);

    let mut aliases = read_room_aliases(store, &tenant_slug, &canonical)?;
    if aliases.is_empty() {
        aliases = task_ref.aliases.clone();
    }
    let (active_actors, stale_actors) = digest_actors(store, &tenant_slug, &canonical, &now, stale_after_ms)?;
    let pending_pings = read_pending_pings_for_task(store, &tenant_slug, &task_ref.task_ref_id, limit)?;
    let related_messages = read_related_events(store, &tenant_slug, &canonical, limit)?;
    let contradictions = read_open_contradictions(store, &tenant_slug, &task_ref.task_ref_id)?;

    Ok(RoomDigest {
        task_ref_id: task_ref.task_ref_id,
        canonical_room_id: canonical,
        confidence: task_ref.confidence,
        aliases,
        active_actors,
        stale_actors,
        pending_pings,
        related_messages,
        contradictions,
    })
}

fn digest_actors<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    canonical_room_id: &str,
    now: &str,
    stale_after_ms: u64,
) -> CoordinationResult<(Vec<ActorActivity>, Vec<ActorActivity>)> {
    let mut last_seen: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut consider = |actor: &str, stamp: &str, source: &str| {
        let actor = normalize_actor_id(actor);
        if actor.is_empty() {
            return;
        }
        let stamp = stamp.trim().to_string();
        let entry = last_seen
            .entry(actor)
            .or_insert_with(|| (stamp.clone(), source.to_string()));
        if stamp > entry.0 {
            *entry = (stamp, source.to_string());
        }
    };

    for intent in read_intents(store, tenant_slug, canonical_room_id)? {
        let stamp = if intent.updated_at.trim().is_empty() {
            intent.started_at.clone()
        } else {
            intent.updated_at.clone()
        };
        consider(&intent.actor_id, &stamp, "intent");
    }
    for presence in list_presence_records(store, tenant_slug)? {
        let stamp = if presence.refreshed_at.trim().is_empty() {
            presence.expires_at.clone()
        } else {
            presence.refreshed_at.clone()
        };
        consider(&presence.actor_id, &stamp, "presence");
    }

    let mut active = Vec::new();
    let mut stale = Vec::new();
    for (actor_id, (stamp, source)) in last_seen {
        let activity = ActorActivity {
            actor_id,
            last_seen: stamp.clone(),
            source,
        };
        if is_stale(&stamp, now, stale_after_ms) {
            stale.push(activity);
        } else {
            active.push(activity);
        }
    }
    Ok((active, stale))
}

fn read_room_aliases<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    canonical_room_id: &str,
) -> CoordinationResult<Vec<RoomAlias>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let canonical_room_id = normalize_room_id(canonical_room_id);
    let mut aliases = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationRoomAlias")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property(
                    "canonical_room_id",
                    Value::String(canonical_room_id.clone()),
                ),
        )? {
            aliases.push(deserialize::<RoomAlias>(node.properties)?);
        }
    }
    aliases.sort_by(|left, right| left.from_room_id.cmp(&right.from_room_id));
    Ok(aliases)
}

// ---------------------------------------------------------------------------
// TRR-005: repo-local coordination manifest.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ManifestActor {
    #[serde(default)]
    pub role: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CoordinationManifest {
    pub schema_version: u32,
    pub task_ref_id: String,
    pub canonical_room_id: String,
    pub tenant_slug: String,
    #[serde(default)]
    pub repo: String,
    #[serde(default)]
    pub branch: String,
    #[serde(default)]
    pub worktree: String,
    #[serde(default)]
    pub actors: BTreeMap<String, ManifestActor>,
    #[serde(default)]
    pub open_questions: Vec<String>,
}

impl Default for CoordinationManifest {
    fn default() -> Self {
        Self {
            schema_version: 1,
            task_ref_id: String::new(),
            canonical_room_id: String::new(),
            tenant_slug: String::new(),
            repo: String::new(),
            branch: String::new(),
            worktree: String::new(),
            actors: BTreeMap::new(),
            open_questions: Vec::new(),
        }
    }
}

/// `<worktree>/.harness/coordination.json`.
pub fn coordination_manifest_path(worktree_dir: &Path) -> PathBuf {
    worktree_dir.join(".harness").join("coordination.json")
}

/// Write (or update) the manifest. When the existing manifest is for the same
/// task, actors and open_questions merge in rather than being clobbered; a
/// different task replaces it (the new task now owns the worktree).
pub fn write_coordination_manifest(
    worktree_dir: &Path,
    manifest: &CoordinationManifest,
) -> io::Result<PathBuf> {
    let path = coordination_manifest_path(worktree_dir);
    let mut to_write = manifest.clone();
    if to_write.schema_version == 0 {
        to_write.schema_version = 1;
    }
    if let Some(existing) = read_coordination_manifest(worktree_dir)? {
        if existing.task_ref_id == to_write.task_ref_id {
            for (actor, role) in existing.actors {
                to_write.actors.entry(actor).or_insert(role);
            }
            let mut seen: BTreeSet<String> = to_write.open_questions.iter().cloned().collect();
            for question in existing.open_questions {
                if seen.insert(question.clone()) {
                    to_write.open_questions.push(question);
                }
            }
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::to_string_pretty(&to_write)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    std::fs::write(&path, body)?;
    Ok(path)
}

/// Read the manifest if present.
pub fn read_coordination_manifest(
    worktree_dir: &Path,
) -> io::Result<Option<CoordinationManifest>> {
    let path = coordination_manifest_path(worktree_dir);
    match std::fs::read_to_string(&path) {
        Ok(body) => {
            let manifest = serde_json::from_str::<CoordinationManifest>(&body)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
            Ok(Some(manifest))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

// ---------------------------------------------------------------------------
// Node / edge persistence.
// ---------------------------------------------------------------------------

fn persist_room_alias<S: CoordinationStore>(store: &mut S, alias: &RoomAlias) -> CoordinationResult<()> {
    ensure_room_exists(
        store,
        &alias.tenant_slug,
        &alias.canonical_room_id,
        &alias.created_at,
    )?;
    let node = NodeRecord::new(
        room_alias_node_id(&alias.tenant_slug, &alias.from_room_id),
        ["HarnessCoordination", "CoordinationRoomAlias"],
        serialize(alias)?,
    );
    upsert_node_if_changed(store, node)?;
    let edge = EdgeRecord::new(
        room_alias_edge_id(&alias.tenant_slug, &alias.from_room_id),
        room_alias_node_id(&alias.tenant_slug, &alias.from_room_id),
        "COORDINATION_ROOM_ALIAS_OF",
        crate::coordination::coordination_room_node_id(
            &alias.tenant_slug,
            &alias.canonical_room_id,
        ),
        json!({
            "tenant_slug": alias.tenant_slug,
            "from_room_id": alias.from_room_id,
            "canonical_room_id": alias.canonical_room_id,
            "task_ref_id": alias.task_ref_id,
        }),
    );
    upsert_edge_if_changed(store, edge)?;
    Ok(())
}

fn persist_related_event<S: CoordinationStore>(
    store: &mut S,
    event: &RelatedEvent,
) -> CoordinationResult<()> {
    ensure_room_exists(
        store,
        &event.tenant_slug,
        &event.canonical_room_id,
        &event.created_at,
    )?;
    let node = NodeRecord::new(
        related_event_node_id(&event.tenant_slug, &event.canonical_room_id, &event.event_id),
        ["HarnessCoordination", "CoordinationRelatedEvent"],
        serialize(event)?,
    );
    upsert_node_if_changed(store, node)?;
    let edge = EdgeRecord::new(
        related_event_edge_id(&event.tenant_slug, &event.canonical_room_id, &event.event_id),
        related_event_node_id(&event.tenant_slug, &event.canonical_room_id, &event.event_id),
        "COORDINATION_RELATED_OF",
        crate::coordination::coordination_room_node_id(
            &event.tenant_slug,
            &event.canonical_room_id,
        ),
        json!({
            "tenant_slug": event.tenant_slug,
            "canonical_room_id": event.canonical_room_id,
            "origin_room_id": event.origin_room_id,
            "origin_message_id": event.origin_message_id,
            "task_ref_id": event.task_ref_id,
        }),
    );
    upsert_edge_if_changed(store, edge)?;
    Ok(())
}

fn persist_ping<S: CoordinationStore>(store: &mut S, ping: &ActorPing) -> CoordinationResult<()> {
    let node = NodeRecord::new(
        ping_node_id(&ping.tenant_slug, &ping.ping_id),
        ["HarnessCoordination", "CoordinationActorPing"],
        serialize(ping)?,
    );
    upsert_node_if_changed(store, node)?;
    Ok(())
}

fn persist_claim<S: CoordinationStore>(store: &mut S, claim: &Claim) -> CoordinationResult<()> {
    let node = NodeRecord::new(
        claim_node_id(&claim.tenant_slug, &claim.claim_id),
        ["HarnessCoordination", "CoordinationClaim"],
        serialize(claim)?,
    );
    upsert_node_if_changed(store, node)?;
    Ok(())
}

fn persist_contradiction<S: CoordinationStore>(
    store: &mut S,
    contradiction: &Contradiction,
) -> CoordinationResult<()> {
    ensure_room_exists(
        store,
        &contradiction.tenant_slug,
        &contradiction.room_id,
        &contradiction.created_at,
    )?;
    let node = NodeRecord::new(
        contradiction_node_id(&contradiction.tenant_slug, &contradiction.contradiction_id),
        ["HarnessCoordination", "CoordinationContradiction"],
        serialize(contradiction)?,
    );
    upsert_node_if_changed(store, node)?;
    let edge = EdgeRecord::new(
        contradiction_edge_id(&contradiction.tenant_slug, &contradiction.contradiction_id),
        contradiction_node_id(&contradiction.tenant_slug, &contradiction.contradiction_id),
        "COORDINATION_CONTRADICTION_OF",
        crate::coordination::coordination_room_node_id(
            &contradiction.tenant_slug,
            &contradiction.room_id,
        ),
        json!({
            "tenant_slug": contradiction.tenant_slug,
            "task_ref_id": contradiction.task_ref_id,
            "contradiction_id": contradiction.contradiction_id,
            "resolved": contradiction.resolved,
        }),
    );
    upsert_edge_if_changed(store, edge)?;
    Ok(())
}

fn contradicts_edge(contradiction: &Contradiction) -> CoordinationResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        contradicts_edge_id(
            &contradiction.tenant_slug,
            &contradiction.left_claim_id,
            &contradiction.right_claim_id,
        ),
        claim_node_id(&contradiction.tenant_slug, &contradiction.left_claim_id),
        "COORDINATION_CONTRADICTS",
        claim_node_id(&contradiction.tenant_slug, &contradiction.right_claim_id),
        json!({
            "tenant_slug": contradiction.tenant_slug,
            "task_ref_id": contradiction.task_ref_id,
            "contradiction_id": contradiction.contradiction_id,
            "subject": contradiction.subject,
            "predicate": contradiction.predicate,
        }),
    ))
}

// ---------------------------------------------------------------------------
// Node / edge id helpers (mirroring the coordination.rs id conventions).
// ---------------------------------------------------------------------------

fn id_part(value: &str, fallback: &str) -> String {
    let slug = slugify_room_part(value);
    if slug.is_empty() {
        fallback.to_string()
    } else {
        slug
    }
}

fn room_alias_node_id(tenant_slug: &str, from_room_id: &str) -> String {
    format!(
        "harness:coordination:room-alias:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(from_room_id, "ungrouped")
    )
}

fn room_alias_edge_id(tenant_slug: &str, from_room_id: &str) -> String {
    format!(
        "harness:coordination:edge:room-alias:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(from_room_id, "ungrouped")
    )
}

fn related_event_node_id(tenant_slug: &str, canonical_room_id: &str, event_id: &str) -> String {
    format!(
        "harness:coordination:related-event:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(canonical_room_id, "ungrouped"),
        id_part(event_id, "unknown")
    )
}

fn related_event_edge_id(tenant_slug: &str, canonical_room_id: &str, event_id: &str) -> String {
    format!(
        "harness:coordination:edge:related-event:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(canonical_room_id, "ungrouped"),
        id_part(event_id, "unknown")
    )
}

fn ping_node_id(tenant_slug: &str, ping_id: &str) -> String {
    format!(
        "harness:coordination:actor-ping:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(ping_id, "unknown")
    )
}

fn claim_node_id(tenant_slug: &str, claim_id: &str) -> String {
    format!(
        "harness:coordination:claim:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(claim_id, "unknown")
    )
}

fn contradiction_node_id(tenant_slug: &str, contradiction_id: &str) -> String {
    format!(
        "harness:coordination:contradiction:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(contradiction_id, "unknown")
    )
}

fn contradiction_edge_id(tenant_slug: &str, contradiction_id: &str) -> String {
    format!(
        "harness:coordination:edge:contradiction:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(contradiction_id, "unknown")
    )
}

fn contradicts_edge_id(tenant_slug: &str, left_claim_id: &str, right_claim_id: &str) -> String {
    format!(
        "harness:coordination:edge:contradicts:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        id_part(left_claim_id, "unknown"),
        id_part(right_claim_id, "unknown")
    )
}

// ---------------------------------------------------------------------------
// Store helpers (over the narrow CoordinationStore seam).
// ---------------------------------------------------------------------------

fn upsert_node_if_changed<S: CoordinationStore>(
    store: &mut S,
    node: NodeRecord,
) -> CoordinationResult<()> {
    let unchanged = store
        .coord_get_node(&node.id)?
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.coord_upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed<S: CoordinationStore>(
    store: &mut S,
    edge: EdgeRecord,
) -> CoordinationResult<()> {
    let unchanged = store
        .coord_get_edge(&edge.id)?
        .map(|existing| {
            !existing.tombstone
                && existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.edge_type == edge.edge_type
                && existing.properties == edge.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.coord_upsert_edge(edge)?;
    }
    Ok(())
}

/// Ensure a room node exists so edges can point at it. Mirrors the room node
/// `coordination.rs` writes, over the narrow seam.
fn ensure_room_exists<S: CoordinationStore>(
    store: &mut S,
    tenant_slug: &str,
    room_id: &str,
    now: &str,
) -> CoordinationResult<()> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let room_id = normalize_room_id(room_id);
    let node_id = crate::coordination::coordination_room_node_id(&tenant_slug, &room_id);
    if store.coord_get_node(&node_id)?.is_some() {
        return Ok(());
    }
    let state = empty_room_state(&tenant_slug, &room_id, now);
    let node = NodeRecord::new(
        node_id,
        ["HarnessCoordination", "CoordinationRoom"],
        serialize(&state)?,
    );
    upsert_node_if_changed(store, node)
}

fn read_intents<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
    room_id: &str,
) -> CoordinationResult<Vec<CoordinationIntentState>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let room_id = normalize_room_id(room_id);
    let mut intents = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationIntent")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("room_id", Value::String(room_id.clone())),
        )? {
            intents.push(deserialize::<CoordinationIntentState>(node.properties)?);
        }
    }
    Ok(intents)
}

fn list_presence_records<S: CoordinationStore>(
    store: &S,
    tenant_slug: &str,
) -> CoordinationResult<Vec<CoordinationPresenceState>> {
    let tenant_slug = require_tenant_slug(tenant_slug)?;
    let mut records = Vec::new();
    for tenant_alias in tenant_slug_aliases(&tenant_slug) {
        for node in store.coord_query_nodes(
            NodeQuery::label("CoordinationPresence")
                .with_property("tenant_slug", Value::String(tenant_alias)),
        )? {
            records.push(deserialize::<CoordinationPresenceState>(node.properties)?);
        }
    }
    Ok(records)
}

// ---------------------------------------------------------------------------
// Small shared helpers.
// ---------------------------------------------------------------------------

fn serialize<T: Serialize>(value: &T) -> CoordinationResult<Value> {
    serde_json::to_value(value).map_err(|error| CoordinationError::Serialization(error.to_string()))
}

fn deserialize<T: for<'de> Deserialize<'de>>(value: Value) -> CoordinationResult<T> {
    serde_json::from_value(value)
        .map_err(|error| CoordinationError::Deserialization(error.to_string()))
}

fn require_nonempty(field: &str, value: &str) -> CoordinationResult<String> {
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

fn norm_key(value: &str) -> String {
    value.trim().to_lowercase()
}

fn truncate(value: &str, max_chars: usize) -> String {
    let value = value.trim();
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    value.chars().take(max_chars).collect()
}

fn truncate_vec<T>(values: &mut Vec<T>, limit: usize) {
    if limit > 0 && values.len() > limit {
        values.truncate(limit);
    }
}

/// Stale when we can prove the timestamp is older than the cutoff. Unparseable
/// timestamps are treated as fresh (conservative: never falsely stale).
fn is_stale(stamp: &str, now: &str, stale_after_ms: u64) -> bool {
    match (parse_timestamp_ms(stamp), parse_timestamp_ms(now)) {
        (Some(stamp_ms), Some(now_ms)) => now_ms.saturating_sub(stamp_ms) > stale_after_ms,
        _ => false,
    }
}

fn parse_timestamp_ms(value: &str) -> Option<u64> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Some(rest) = value.strip_prefix("unix_ms:") {
        return rest.trim().parse::<u64>().ok();
    }
    parse_rfc3339_ms(value)
}

// ponytail: coarse RFC3339 -> epoch ms. Reads the YYYY-MM-DDTHH:MM:SS prefix and
// treats it as UTC, ignoring fractional seconds and timezone offset. Good enough
// for staleness buckets; swap for `chrono` if exact offset handling is needed.
fn parse_rfc3339_ms(value: &str) -> Option<u64> {
    if value.len() < 19 {
        return None;
    }
    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: i64 = value.get(5..7)?.parse().ok()?;
    let day: i64 = value.get(8..10)?.parse().ok()?;
    let hour: i64 = value.get(11..13)?.parse().ok()?;
    let minute: i64 = value.get(14..16)?.parse().ok()?;
    let second: i64 = value.get(17..19)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    let seconds = days * 86_400 + hour * 3_600 + minute * 60 + second;
    if seconds < 0 {
        return None;
    }
    Some((seconds as u64) * 1000)
}

// Howard Hinnant's days_from_civil: days since 1970-01-01 for a civil date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::{
        read_messages_for_room, write_intent, write_message, WriteIntentInput, WriteMessageInput,
    };

    fn msg(
        room_id: &str,
        actor_id: &str,
        message_id: &str,
        text: &str,
        urgency: &str,
        created_at: &str,
    ) -> CoordinationMessageState {
        CoordinationMessageState {
            tenant_slug: TENANT.to_string(),
            room_id: room_id.to_string(),
            message_id: message_id.to_string(),
            actor_id: actor_id.to_string(),
            urgency: urgency.to_string(),
            delivery: "passive".to_string(),
            message: text.to_string(),
            mentions: Vec::new(),
            metadata: serde_json::Map::new(),
            consumed_by: Vec::new(),
            created_at: created_at.to_string(),
        }
    }
    use rustyred_thg_core::{
        InMemoryGraphStore, RedCoreDurability, RedCoreGraphStore, RedCoreOptions,
    };
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    const TENANT: &str = "travis-gilbert";

    fn spec9_repo() -> TaskRefInput {
        TaskRefInput {
            tenant_slug: TENANT.to_string(),
            repo: "Travis-Gilbert/Theorem".to_string(),
            workstream: "SPEC-9 CommonPlace Desktop".to_string(),
            spec_refs: vec![
                "docs/plans/commonplace-desktop-tauri/SPEC-9-commonplace-desktop-tauri.md"
                    .to_string(),
            ],
            external_refs: Vec::new(),
            branch: "Travis-Gilbert/spec-9-commonplace-desktop-tauri".to_string(),
        }
    }

    fn spec9_downloads() -> TaskRefInput {
        TaskRefInput {
            tenant_slug: TENANT.to_string(),
            repo: "Travis-Gilbert/Theorem".to_string(),
            workstream: "SPEC-9 CommonPlace Desktop".to_string(),
            spec_refs: Vec::new(),
            external_refs: vec![
                "/Users/travisgilbert/Downloads/SPEC-9-commonplace-desktop-tauri.md".to_string(),
            ],
            branch: "Travis-Gilbert/spec-9-commonplace-desktop-tauri".to_string(),
        }
    }

    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default();
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("coord_v2_{tag}_{nanos}_{seq}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // TRR-001 ---------------------------------------------------------------
    #[test]
    fn trr_001_resolver_is_deterministic_across_heads() {
        let claude = resolve_task_ref(&spec9_downloads());
        let codex = resolve_task_ref(&spec9_repo());
        assert_eq!(claude.task_ref_id, codex.task_ref_id);
        assert_eq!(claude.canonical_room_id, codex.canonical_room_id);
        assert_eq!(claude.confidence, codex.confidence);
        assert_eq!(claude.confidence, TaskRefConfidence::Exact);
        assert_eq!(
            claude.canonical_room_id,
            "task:spec-9-commonplace-desktop-tauri"
        );
        // Ambiguous when nothing addressable is supplied.
        let empty = resolve_task_ref(&TaskRefInput {
            tenant_slug: TENANT.to_string(),
            ..Default::default()
        });
        assert_eq!(empty.confidence, TaskRefConfidence::Ambiguous);
        assert_eq!(empty.canonical_room_id, DEFAULT_ROOM);
    }

    // TRR-002 ---------------------------------------------------------------
    #[test]
    fn trr_002_ungrouped_message_routes_to_canonical_with_provenance() {
        let mut store = InMemoryGraphStore::new();
        let message = write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: "room:ungrouped".to_string(),
                actor_id: "claude-code".to_string(),
                message: "Handoff: D3 frontend export ready for review".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();

        let routed = route_message_to_task(&mut store, &message, &spec9_repo())
            .unwrap()
            .expect("off-canonical message should route");

        let canonical = resolve_task_ref(&spec9_repo()).canonical_room_id;
        let inbox = read_related_events(&store, TENANT, &canonical, 10).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].origin_room_id, "room:ungrouped");
        assert_eq!(inbox[0].origin_message_id, message.message_id);
        assert_eq!(inbox[0].actor_id, "claude-code");
        assert_eq!(inbox[0].reason, "task-metadata-match");
        assert!(inbox[0].summary.contains("D3 frontend export"));
        assert_eq!(routed.canonical_room_id, canonical);

        // A message already in the canonical room does not double-route.
        let in_room = msg(
            &canonical,
            "codex",
            "msg_inroom",
            "already here",
            "info",
            "2026-06-21T00:01:00+00:00",
        );
        let none = route_message_to_task(&mut store, &in_room, &spec9_repo()).unwrap();
        assert!(none.is_none());
    }

    // TRR-003 ---------------------------------------------------------------
    #[test]
    fn trr_003_ask_creates_ping_reachable_without_subscription() {
        let mut store = InMemoryGraphStore::new();
        let ping = create_ping(
            &mut store,
            PingInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: "task_demo".to_string(),
                room_id: "task:demo".to_string(),
                from_actor: "claude-code".to_string(),
                target_actor: "codex".to_string(),
                urgency: "ask".to_string(),
                message: "Need the port decision".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
                ..PingInput::default()
            },
        )
        .unwrap();
        assert_eq!(ping.status, PING_PENDING);

        // The target reads it with no stream subscription anywhere in the store.
        let open = read_open_pings_for_actor(&mut store, TENANT, "codex", None, true, 10).unwrap();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].ping_id, ping.ping_id);
        assert_eq!(open[0].status, PING_SEEN);

        let consumed = consume_ping(&mut store, TENANT, &ping.ping_id).unwrap().unwrap();
        assert_eq!(consumed.status, PING_CONSUMED);
        let after = read_open_pings_for_actor(&mut store, TENANT, "codex", None, false, 10).unwrap();
        assert!(after.is_empty());

        // info urgency is not a ping.
        let err = create_ping(
            &mut store,
            PingInput {
                tenant_slug: TENANT.to_string(),
                target_actor: "codex".to_string(),
                urgency: "info".to_string(),
                message: "fyi".to_string(),
                ..PingInput::default()
            },
        );
        assert!(err.is_err());
    }

    // TRR-006 ---------------------------------------------------------------
    #[test]
    fn trr_006_ping_targets_specific_checkout() {
        let mut store = InMemoryGraphStore::new();
        create_ping(
            &mut store,
            PingInput {
                tenant_slug: TENANT.to_string(),
                target_actor: "codex".to_string(),
                target_branch: "branch-a".to_string(),
                target_worktree: "/work/a".to_string(),
                urgency: "block".to_string(),
                message: "blocked on you".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
                ..PingInput::default()
            },
        )
        .unwrap();

        let intended =
            read_open_pings_for_actor(&mut store, TENANT, "codex", Some(("branch-a", "/work/a")), false, 10)
                .unwrap();
        assert_eq!(intended.len(), 1);

        let other_checkout =
            read_open_pings_for_actor(&mut store, TENANT, "codex", Some(("branch-b", "/work/b")), false, 10)
                .unwrap();
        assert!(other_checkout.is_empty(), "a different checkout is not the target");
    }

    // TRR-007 ---------------------------------------------------------------
    #[test]
    fn trr_007_contradiction_writes_edge_and_event_and_resolves_on_supersede() {
        let mut store = InMemoryGraphStore::new();
        let task = "task_ports";
        let (codex_claim, c0) = record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task.to_string(),
                room_id: "task:ports".to_string(),
                actor: "codex".to_string(),
                subject: "Tauri dev port".to_string(),
                predicate: "value".to_string(),
                object: "1420".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
            },
        )
        .unwrap();
        assert!(c0.is_empty());

        let (cc_claim, c1) = record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task.to_string(),
                room_id: "task:ports".to_string(),
                actor: "claude-code".to_string(),
                subject: "Tauri dev port".to_string(),
                predicate: "value".to_string(),
                object: "3000".to_string(),
                created_at: "2026-06-21T00:01:00+00:00".to_string(),
            },
        )
        .unwrap();
        assert_eq!(c1.len(), 1);

        let edge_id = contradicts_edge_id(TENANT, &codex_claim.claim_id, &cc_claim.claim_id);
        let alt_id = contradicts_edge_id(TENANT, &cc_claim.claim_id, &codex_claim.claim_id);
        assert!(
            store.get_edge(&edge_id).is_some() || store.get_edge(&alt_id).is_some(),
            "a CONTRADICTS edge must exist between the two claims"
        );
        assert_eq!(read_open_contradictions(&store, TENANT, task).unwrap().len(), 1);

        // codex changes its mind to 3000: its old claim is superseded and the
        // contradiction resolves; no re-alert.
        record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task.to_string(),
                room_id: "task:ports".to_string(),
                actor: "codex".to_string(),
                subject: "Tauri dev port".to_string(),
                predicate: "value".to_string(),
                object: "3000".to_string(),
                created_at: "2026-06-21T00:02:00+00:00".to_string(),
            },
        )
        .unwrap();
        assert!(read_open_contradictions(&store, TENANT, task).unwrap().is_empty());
    }

    // TRR-004 ---------------------------------------------------------------
    #[test]
    fn trr_004_turn_start_discovery_surfaces_every_bucket() {
        let mut store = InMemoryGraphStore::new();
        let task = spec9_repo();
        let task_ref = resolve_task_ref(&task);
        let canonical = task_ref.canonical_room_id.clone();

        // related inbox message
        route_message_to_task(
            &mut store,
            &msg(
                "room:ungrouped",
                "claude-code",
                "msg1",
                "handoff via ungrouped",
                "info",
                "2026-06-21T00:00:00+00:00",
            ),
            &task,
        )
        .unwrap();
        // open ping for codex
        create_ping(
            &mut store,
            PingInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                target_actor: "codex".to_string(),
                urgency: "ask".to_string(),
                message: "look here".to_string(),
                created_at: "2026-06-21T00:01:00+00:00".to_string(),
                ..PingInput::default()
            },
        )
        .unwrap();
        // active intent (recent) + stale intent (old)
        write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: canonical.clone(),
                actor_id: "claude-code".to_string(),
                status: "working".to_string(),
                summary: "editing frontend".to_string(),
                updated_at: "2026-06-21T12:00:00+00:00".to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();
        write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: canonical.clone(),
                actor_id: "codex".to_string(),
                status: "working".to_string(),
                summary: "old work".to_string(),
                updated_at: "2026-06-20T00:00:00+00:00".to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();
        // contradiction
        record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                actor: "codex".to_string(),
                subject: "port".to_string(),
                predicate: "value".to_string(),
                object: "1420".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
            },
        )
        .unwrap();
        record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                actor: "claude-code".to_string(),
                subject: "port".to_string(),
                predicate: "value".to_string(),
                object: "3000".to_string(),
                created_at: "2026-06-21T00:01:00+00:00".to_string(),
            },
        )
        .unwrap();

        let discovery = turn_start_discovery(
            &mut store,
            &DiscoveryInput {
                task,
                actor: "codex".to_string(),
                now: "2026-06-21T12:30:00+00:00".to_string(),
                stale_after_ms: 60 * 60 * 1000,
                ..DiscoveryInput::default()
            },
        )
        .unwrap();

        assert_eq!(discovery.canonical_room_id, canonical);
        assert_eq!(discovery.related_inbox.len(), 1);
        assert_eq!(discovery.open_pings.len(), 1);
        assert_eq!(discovery.active_intents.len(), 1);
        assert_eq!(discovery.active_intents[0].actor_id, "claude-code");
        assert_eq!(discovery.stale_intents.len(), 1);
        assert_eq!(discovery.stale_intents[0].actor_id, "codex");
        assert_eq!(discovery.contradictions.len(), 1);
    }

    // TRR-005 ---------------------------------------------------------------
    #[test]
    fn trr_005_manifest_roundtrips_and_merges_without_clobber() {
        let dir = unique_temp_dir("manifest");
        let task_ref = resolve_task_ref(&spec9_repo());
        let mut manifest = CoordinationManifest {
            task_ref_id: task_ref.task_ref_id.clone(),
            canonical_room_id: task_ref.canonical_room_id.clone(),
            tenant_slug: TENANT.to_string(),
            repo: "Travis-Gilbert/Theorem".to_string(),
            branch: task_ref.branch.clone(),
            worktree: dir.to_string_lossy().to_string(),
            ..Default::default()
        };
        manifest
            .actors
            .insert("codex".to_string(), ManifestActor { role: "primary".to_string() });
        write_coordination_manifest(&dir, &manifest).unwrap();

        // A later head reads the canonical room without guessing.
        let read = read_coordination_manifest(&dir).unwrap().unwrap();
        assert_eq!(read.canonical_room_id, task_ref.canonical_room_id);
        assert_eq!(read.task_ref_id, task_ref.task_ref_id);

        // Updating with a new actor merges, keeping the prior actor.
        let mut update = CoordinationManifest {
            task_ref_id: task_ref.task_ref_id.clone(),
            canonical_room_id: task_ref.canonical_room_id.clone(),
            tenant_slug: TENANT.to_string(),
            ..Default::default()
        };
        update.actors.insert(
            "claude-code".to_string(),
            ManifestActor { role: "frontend".to_string() },
        );
        write_coordination_manifest(&dir, &update).unwrap();
        let merged = read_coordination_manifest(&dir).unwrap().unwrap();
        assert!(merged.actors.contains_key("codex"));
        assert!(merged.actors.contains_key("claude-code"));

        std::fs::remove_dir_all(&dir).ok();
    }

    // TRR-008 ---------------------------------------------------------------
    #[test]
    fn trr_008_digest_displays_all_sections() {
        let mut store = InMemoryGraphStore::new();
        let task = spec9_repo();
        let task_ref = resolve_task_ref(&task);
        let canonical = task_ref.canonical_room_id.clone();
        register_task_ref(&mut store, &task_ref, "2026-06-21T00:00:00+00:00").unwrap();

        route_message_to_task(
            &mut store,
            &msg(
                "room:ungrouped",
                "claude-code",
                "msg1",
                "ungrouped handoff",
                "info",
                "2026-06-21T00:00:00+00:00",
            ),
            &task,
        )
        .unwrap();
        create_ping(
            &mut store,
            PingInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                target_actor: "codex".to_string(),
                urgency: "block".to_string(),
                message: "blocked".to_string(),
                created_at: "2026-06-21T00:01:00+00:00".to_string(),
                ..PingInput::default()
            },
        )
        .unwrap();
        write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: canonical.clone(),
                actor_id: "claude-code".to_string(),
                status: "working".to_string(),
                summary: "fresh".to_string(),
                updated_at: "2026-06-21T12:00:00+00:00".to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();
        write_intent(
            &mut store,
            WriteIntentInput {
                tenant_slug: TENANT.to_string(),
                room_id: canonical.clone(),
                actor_id: "stale-bot".to_string(),
                status: "working".to_string(),
                summary: "old".to_string(),
                updated_at: "2026-06-20T00:00:00+00:00".to_string(),
                ..WriteIntentInput::default()
            },
        )
        .unwrap();
        record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                actor: "codex".to_string(),
                subject: "port".to_string(),
                predicate: "value".to_string(),
                object: "1420".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
            },
        )
        .unwrap();
        record_claim(
            &mut store,
            ClaimInput {
                tenant_slug: TENANT.to_string(),
                task_ref_id: task_ref.task_ref_id.clone(),
                room_id: canonical.clone(),
                actor: "claude-code".to_string(),
                subject: "port".to_string(),
                predicate: "value".to_string(),
                object: "3000".to_string(),
                created_at: "2026-06-21T00:01:00+00:00".to_string(),
            },
        )
        .unwrap();

        let digest = room_digest(
            &store,
            &DigestInput {
                task,
                now: "2026-06-21T12:30:00+00:00".to_string(),
                stale_after_ms: 60 * 60 * 1000,
                ..DigestInput::default()
            },
        )
        .unwrap();

        assert_eq!(digest.canonical_room_id, canonical);
        assert!(!digest.aliases.is_empty(), "aliases should be registered");
        assert!(digest.active_actors.iter().any(|a| a.actor_id == "claude-code"));
        assert!(digest.stale_actors.iter().any(|a| a.actor_id == "stale-bot"));
        assert_eq!(digest.pending_pings.len(), 1);
        assert_eq!(digest.related_messages.len(), 1);
        assert_eq!(digest.contradictions.len(), 1);
    }

    // TRR-009 ---------------------------------------------------------------
    #[test]
    fn trr_009_old_room_ids_alias_in_without_data_loss() {
        let mut store = InMemoryGraphStore::new();
        let task = spec9_repo();
        let task_ref = resolve_task_ref(&task);
        register_task_ref(&mut store, &task_ref, "2026-06-21T00:00:00+00:00").unwrap();

        // The legacy room name (repo:branch) should be a registered alias.
        let legacy_room = infer_coordination_room_id(&task_ref.repo, &task_ref.branch, "", "");
        let (canonical, alias) = resolve_canonical_room(&store, TENANT, &legacy_room).unwrap();
        assert_eq!(canonical, task_ref.canonical_room_id);
        assert!(alias.is_some());

        // A message written to the old room is still readable there (no data loss).
        write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: legacy_room.clone(),
                actor_id: "codex".to_string(),
                message: "still here".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();
        let legacy_messages = read_messages_for_room(&store, TENANT, &legacy_room, 10).unwrap();
        assert_eq!(legacy_messages.len(), 1);

        // An unaliased room resolves to itself.
        let (self_room, none) = resolve_canonical_room(&store, TENANT, "room:unknown").unwrap();
        assert_eq!(self_room, "room:unknown");
        assert!(none.is_none());
    }

    // TRR-010 ---------------------------------------------------------------
    #[test]
    fn trr_010_spec9_replay_codex_discovers_claude_handoff_before_editing() {
        let dir = unique_temp_dir("replay");
        let mut store = RedCoreGraphStore::open(&dir, RedCoreOptions::default()).unwrap();

        // Claude resolves the task and (the bug) writes a handoff to room:ungrouped.
        let task = spec9_downloads();
        let claude_ref = resolve_task_ref(&task);
        let handoff = write_message(
            &mut store,
            WriteMessageInput {
                tenant_slug: TENANT.to_string(),
                room_id: "room:ungrouped".to_string(),
                actor_id: "claude-code".to_string(),
                message: "Handoff: backend wiring for SPEC-9 is done, frontend next".to_string(),
                created_at: "2026-06-21T00:00:00+00:00".to_string(),
                ..WriteMessageInput::default()
            },
        )
        .unwrap();
        // Permissive routing attaches it to the canonical SPEC-9 room.
        route_message_to_task(&mut store, &handoff, &task).unwrap();

        // Codex, at turn start for the SPEC-9 task (resolved from the repo path),
        // discovers Claude's handoff before it edits.
        let codex_view = turn_start_discovery(
            &mut store,
            &DiscoveryInput {
                task: spec9_repo(),
                actor: "codex".to_string(),
                now: "2026-06-21T00:05:00+00:00".to_string(),
                ..DiscoveryInput::default()
            },
        )
        .unwrap();

        assert_eq!(codex_view.task_ref_id, claude_ref.task_ref_id);
        assert_eq!(codex_view.related_inbox.len(), 1);
        assert!(codex_view.related_inbox[0]
            .summary
            .contains("backend wiring for SPEC-9"));
        assert_eq!(codex_view.related_inbox[0].actor_id, "claude-code");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn trr_durable_aliases_and_related_survive_reopen() {
        let dir = unique_temp_dir("durable");
        let task = spec9_repo();
        let task_ref = resolve_task_ref(&task);
        {
            let mut store = RedCoreGraphStore::open(
                &dir,
                RedCoreOptions {
                    durability: RedCoreDurability::AofAlways,
                    ..Default::default()
                },
            )
            .unwrap();
            register_task_ref(&mut store, &task_ref, "2026-06-21T00:00:00+00:00").unwrap();
            route_message_to_task(
                &mut store,
                &msg(
                    "room:ungrouped",
                    "claude-code",
                    "m1",
                    "durable handoff",
                    "info",
                    "2026-06-21T00:00:00+00:00",
                ),
                &task,
            )
            .unwrap();
        }
        let store = RedCoreGraphStore::open(
                &dir,
                RedCoreOptions {
                    durability: RedCoreDurability::AofAlways,
                    ..Default::default()
                },
            )
            .unwrap();
        let aliases = read_room_aliases(&store, TENANT, &task_ref.canonical_room_id).unwrap();
        assert!(!aliases.is_empty());
        let inbox = read_related_events(&store, TENANT, &task_ref.canonical_room_id, 10).unwrap();
        assert_eq!(inbox.len(), 1);
        assert_eq!(inbox[0].summary, "durable handoff");
        std::fs::remove_dir_all(&dir).ok();
    }
}
