//! Coordination v2 (Task-Reference Rooms) GraphQL domain. Every field lowers to
//! the single `coordination_v2` invoker op, which runs the runtime engine over
//! the `McpCoordinationStore` adapter. This adds zero new flat MCP tools: the
//! verbs ride the existing `graphql_query` / `graphql_mutate` transport.

use async_graphql::{InputObject, Object, Result as GqlResult};
use serde_json::{json, Map, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

/// Task metadata used to resolve the coordination address. The instance scope
/// is bound to the connection, never accepted as a field here.
#[derive(InputObject, Default)]
pub struct TaskRefArgs {
    pub repo: Option<String>,
    pub workstream: Option<String>,
    pub spec_refs: Option<Vec<String>>,
    pub external_refs: Option<Vec<String>>,
    pub branch: Option<String>,
}

fn task_json(task: &TaskRefArgs) -> Value {
    let mut map = Map::new();
    if let Some(repo) = &task.repo {
        map.insert("repo".into(), json!(repo));
    }
    if let Some(workstream) = &task.workstream {
        map.insert("workstream".into(), json!(workstream));
    }
    if let Some(spec_refs) = &task.spec_refs {
        map.insert("spec_refs".into(), json!(spec_refs));
    }
    if let Some(external_refs) = &task.external_refs {
        map.insert("external_refs".into(), json!(external_refs));
    }
    if let Some(branch) = &task.branch {
        map.insert("branch".into(), json!(branch));
    }
    Value::Object(map)
}

fn insert_opt(args: &mut Value, key: &str, value: Option<String>) {
    if let (Some(value), Value::Object(map)) = (value, &mut *args) {
        map.insert(key.to_string(), Value::String(value));
    }
}

fn insert_i64(args: &mut Value, key: &str, value: Option<i64>) {
    if let (Some(value), Value::Object(map)) = (value, &mut *args) {
        map.insert(key.to_string(), json!(value));
    }
}

fn call(operation: &str, args: Value) -> GqlResult<Json> {
    with_invoker(|inv| Ok(Json(inv.coordination_v2(operation, args.clone()).map_err(map_err)?)))
}

#[derive(Default)]
pub struct CoordinationV2Query;

#[Object]
impl CoordinationV2Query {
    /// Resolve task metadata to its stable coordination address: task_ref_id,
    /// canonical room, aliases, and confidence. Pure and deterministic across
    /// heads (path variants of the same spec collapse to one identity).
    async fn task_ref(&self, task: TaskRefArgs) -> GqlResult<Json> {
        call("resolve_task_ref", task_json(&task))
    }

    /// What a head should see before it edits: canonical room, related inbox
    /// messages, this actor's open pings, active vs stale intents, and open
    /// contradictions for the task.
    #[allow(clippy::too_many_arguments)]
    async fn turn_start_discovery(
        &self,
        task: TaskRefArgs,
        actor: String,
        branch: Option<String>,
        worktree: Option<String>,
        now: Option<String>,
        stale_after_ms: Option<i64>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "task": task_json(&task), "actor": actor });
        insert_opt(&mut args, "branch", branch);
        insert_opt(&mut args, "worktree", worktree);
        insert_opt(&mut args, "now", now);
        insert_i64(&mut args, "stale_after_ms", stale_after_ms);
        insert_i64(&mut args, "limit", limit.map(i64::from));
        call("turn_start_discovery", args)
    }

    /// A room-level dashboard snapshot: canonical room, aliases, active/stale
    /// actors, pending pings, related ungrouped messages, and contradictions.
    async fn room_digest(
        &self,
        task: TaskRefArgs,
        now: Option<String>,
        stale_after_ms: Option<i64>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "task": task_json(&task) });
        insert_opt(&mut args, "now", now);
        insert_i64(&mut args, "stale_after_ms", stale_after_ms);
        insert_i64(&mut args, "limit", limit.map(i64::from));
        call("room_digest", args)
    }

    /// This actor's open (not-consumed) pings, filtered to the given checkout.
    async fn open_pings(
        &self,
        actor: String,
        branch: Option<String>,
        worktree: Option<String>,
        limit: Option<i32>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "actor": actor });
        insert_opt(&mut args, "branch", branch);
        insert_opt(&mut args, "worktree", worktree);
        insert_i64(&mut args, "limit", limit.map(i64::from));
        call("open_pings", args)
    }

    /// Open (unresolved) contradictions for a task.
    async fn open_contradictions(&self, task_ref_id: String) -> GqlResult<Json> {
        call("open_contradictions", json!({ "task_ref_id": task_ref_id }))
    }

    /// Related inbox events routed into a canonical room from aliases/ungrouped.
    async fn related_events(&self, canonical_room: String, limit: Option<i32>) -> GqlResult<Json> {
        let mut args = json!({ "canonical_room": canonical_room });
        insert_i64(&mut args, "limit", limit.map(i64::from));
        call("related_events", args)
    }
}

#[derive(Default)]
pub struct CoordinationV2Mutation;

#[Object]
impl CoordinationV2Mutation {
    /// Persist a task's room aliases so an old/known room id resolves to the
    /// canonical room without guessing.
    async fn register_task_ref(
        &self,
        task: TaskRefArgs,
        created_at: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = task_json(&task);
        insert_opt(&mut args, "created_at", created_at);
        call("register_task_ref", args)
    }

    /// Route an off-canonical message into its task's canonical room as a
    /// related event with provenance.
    #[allow(clippy::too_many_arguments)]
    async fn route_message_to_task(
        &self,
        task: TaskRefArgs,
        room_id: String,
        actor: String,
        message_id: String,
        message: String,
        urgency: Option<String>,
        created_at: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = json!({
            "task": task_json(&task),
            "room_id": room_id,
            "actor": actor,
            "message_id": message_id,
            "message": message,
        });
        insert_opt(&mut args, "urgency", urgency);
        insert_opt(&mut args, "created_at", created_at);
        call("route_message", args)
    }

    /// Create an actor ping (ask/block only) with pending/seen/consumed delivery
    /// state, optionally targeting a specific branch/worktree checkout.
    #[allow(clippy::too_many_arguments)]
    async fn create_ping(
        &self,
        target_actor: String,
        urgency: String,
        task_ref_id: Option<String>,
        room_id: Option<String>,
        from_actor: Option<String>,
        target_branch: Option<String>,
        target_worktree: Option<String>,
        message: Option<String>,
        event_id: Option<String>,
        created_at: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "target_actor": target_actor, "urgency": urgency });
        insert_opt(&mut args, "task_ref_id", task_ref_id);
        insert_opt(&mut args, "room_id", room_id);
        insert_opt(&mut args, "from_actor", from_actor);
        insert_opt(&mut args, "target_branch", target_branch);
        insert_opt(&mut args, "target_worktree", target_worktree);
        insert_opt(&mut args, "message", message);
        insert_opt(&mut args, "event_id", event_id);
        insert_opt(&mut args, "created_at", created_at);
        call("create_ping", args)
    }

    /// Consume a ping (the target acted on it).
    async fn consume_ping(&self, ping_id: String) -> GqlResult<Json> {
        call("consume_ping", json!({ "ping_id": ping_id }))
    }

    /// Record a structured claim. A conflicting object from another live claim
    /// writes a CONTRADICTS edge and a room-visible contradiction event; a
    /// same-actor restatement supersedes and resolves.
    #[allow(clippy::too_many_arguments)]
    async fn record_claim(
        &self,
        task_ref_id: String,
        actor: String,
        subject: String,
        predicate: String,
        object: String,
        room_id: Option<String>,
        created_at: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = json!({
            "task_ref_id": task_ref_id,
            "actor": actor,
            "subject": subject,
            "predicate": predicate,
            "object": object,
        });
        insert_opt(&mut args, "room_id", room_id);
        insert_opt(&mut args, "created_at", created_at);
        call("record_claim", args)
    }
}
