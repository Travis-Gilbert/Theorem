//! Coordination domain (A2): room context, stream events, and native work-graph
//! wrappers. Resolvers lower to the existing coordination and multi-head MCP
//! payload handlers through the scoped invoker; the nested live state stays JSON
//! so GraphQL remains a faithful transport facade over the canonical handlers.

// async-graphql `#[Object]` generates resolver functions whose argument count
// mirrors each GraphQL field's argument count; the lint fires on that generated
// code (not the source method), so method-level allows do not reach it. Grouping
// these into input structs would change the public GraphQL schema, so scope the
// allow to this transport-facade module.
#![allow(clippy::too_many_arguments)]

use async_graphql::{Object, Result as GqlResult, SimpleObject};
use serde_json::{json, Value};

use super::scalars::Json;
use super::{map_err, with_invoker};

#[derive(SimpleObject)]
pub struct CoordinationRoomContext {
    pub room_id: String,
    pub actor_id: String,
    pub room: Json,
    pub presence: Json,
    pub intents: Json,
    pub messages: Json,
    pub records: Json,
    pub pending_mentions: Json,
    pub counts: Json,
}

impl CoordinationRoomContext {
    fn from_value(value: Value) -> Self {
        CoordinationRoomContext {
            room_id: str_field(&value, "room_id"),
            actor_id: str_field(&value, "actor_id"),
            room: Json(value.get("room").cloned().unwrap_or(Value::Null)),
            presence: Json(value.get("presence").cloned().unwrap_or_else(|| json!([]))),
            intents: Json(value.get("intents").cloned().unwrap_or_else(|| json!([]))),
            messages: Json(value.get("messages").cloned().unwrap_or_else(|| json!([]))),
            records: Json(value.get("records").cloned().unwrap_or_else(|| json!([]))),
            pending_mentions: Json(
                value
                    .get("pending_mentions")
                    .cloned()
                    .unwrap_or_else(|| json!([])),
            ),
            counts: Json(value.get("counts").cloned().unwrap_or_else(|| json!({}))),
        }
    }
}

#[derive(SimpleObject)]
pub struct CoordinationStreamPublish {
    pub ok: bool,
    pub stream: String,
    pub stream_key: String,
    pub event_id: String,
    pub ordering_token: i64,
    pub urgency: String,
    pub target_actor: Option<String>,
    pub pinged: bool,
    pub created_at: String,
}

impl CoordinationStreamPublish {
    fn from_value(value: Value) -> Self {
        CoordinationStreamPublish {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
            stream: str_field(&value, "stream"),
            stream_key: str_field(&value, "stream_key"),
            event_id: str_field(&value, "event_id"),
            ordering_token: value
                .get("ordering_token")
                .and_then(Value::as_i64)
                .unwrap_or_default(),
            urgency: str_field(&value, "urgency"),
            target_actor: value
                .get("target_actor")
                .and_then(Value::as_str)
                .map(str::to_string),
            pinged: value
                .get("pinged")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            created_at: str_field(&value, "created_at"),
        }
    }
}

#[derive(SimpleObject)]
pub struct CoordinationStreamRead {
    pub actor_id: String,
    pub streams: Vec<String>,
    pub from_subscriptions: bool,
    pub events: Json,
    pub count: i32,
    pub new_cursors: Json,
    pub advanced: bool,
}

impl CoordinationStreamRead {
    fn from_value(value: Value) -> Self {
        CoordinationStreamRead {
            actor_id: str_field(&value, "actor_id"),
            streams: string_array(value.get("streams")),
            from_subscriptions: value
                .get("from_subscriptions")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            events: Json(value.get("events").cloned().unwrap_or_else(|| json!([]))),
            count: value.get("count").and_then(Value::as_i64).unwrap_or(0) as i32,
            new_cursors: Json(
                value
                    .get("new_cursors")
                    .cloned()
                    .unwrap_or_else(|| json!({})),
            ),
            advanced: value
                .get("advanced")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }
}

#[derive(SimpleObject)]
pub struct WorkGraphView {
    pub ok: bool,
    pub run: Json,
    pub graph: Json,
    pub tasks: Json,
}

impl WorkGraphView {
    fn from_value(value: Value) -> Self {
        WorkGraphView {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
            run: Json(value.get("run").cloned().unwrap_or(Value::Null)),
            graph: Json(value.get("graph").cloned().unwrap_or(Value::Null)),
            tasks: Json(value.get("tasks").cloned().unwrap_or_else(|| json!([]))),
        }
    }
}

#[derive(SimpleObject)]
pub struct TaskNodeWrite {
    pub ok: bool,
    pub reused: bool,
    pub task: Json,
}

impl TaskNodeWrite {
    fn from_value(value: Value) -> Self {
        TaskNodeWrite {
            ok: value.get("ok").and_then(Value::as_bool).unwrap_or(false),
            reused: value
                .get("reused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            task: Json(value.get("task").cloned().unwrap_or(Value::Null)),
        }
    }
}

#[derive(Default)]
pub struct CoordinationQuery;

#[Object]
impl CoordinationQuery {
    /// One-shot room context: room metadata, presence, intents, messages,
    /// records, pending mentions, and counts.
    #[allow(clippy::too_many_arguments)]
    async fn coordination_room(
        &self,
        room_id: String,
        actor: Option<String>,
        statuses: Option<Vec<String>>,
        record_types: Option<Vec<String>>,
        message_limit: Option<i32>,
        record_limit: Option<i32>,
        mention_limit: Option<i32>,
    ) -> GqlResult<CoordinationRoomContext> {
        let mut args = json!({ "room_id": room_id });
        insert_opt_string(&mut args, "actor", actor);
        insert_opt_strings(&mut args, "statuses", statuses);
        insert_opt_strings(&mut args, "record_types", record_types);
        insert_opt_i32(&mut args, "message_limit", message_limit);
        insert_opt_i32(&mut args, "record_limit", record_limit);
        insert_opt_i32(&mut args, "mention_limit", mention_limit);
        with_invoker(|inv| {
            Ok(CoordinationRoomContext::from_value(
                inv.coordination_context(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Read stream events without advancing the actor cursor. Cursor advancement
    /// is exposed as a mutation because it writes cursor state.
    async fn coordination_stream(
        &self,
        actor: String,
        stream: Option<String>,
        streams: Option<Vec<String>>,
        limit: Option<i32>,
    ) -> GqlResult<CoordinationStreamRead> {
        let mut args = json!({ "actor": actor, "advance": false });
        insert_opt_string(&mut args, "stream", stream);
        insert_opt_strings(&mut args, "streams", streams);
        insert_opt_i32(&mut args, "limit", limit);
        with_invoker(|inv| {
            Ok(CoordinationStreamRead::from_value(
                inv.stream_read(args.clone(), false).map_err(map_err)?,
            ))
        })
    }

    /// Inspect a native multi-head work graph run.
    async fn work_graph(&self, run_id: String) -> GqlResult<WorkGraphView> {
        let args = json!({ "run_id": run_id });
        with_invoker(|inv| {
            Ok(WorkGraphView::from_value(
                inv.work_graph(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Route the next claimable task node for a head.
    async fn next_task_node(
        &self,
        run_id: String,
        head: String,
        fitness: Option<Json>,
        explore_token: Option<i32>,
        now_ms: Option<i64>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "run_id": run_id, "head": head });
        if let Some(fitness) = fitness {
            args["fitness"] = fitness.0;
        }
        insert_opt_i32(&mut args, "explore_token", explore_token);
        insert_opt_i64(&mut args, "now_ms", now_ms);
        with_invoker(|inv| Ok(Json(inv.next_task_node(args.clone()).map_err(map_err)?)))
    }
}

#[derive(Default)]
pub struct CoordinationMutation;

#[Object]
impl CoordinationMutation {
    /// Write or update this actor's room intent.
    #[allow(clippy::too_many_arguments)]
    async fn write_coordination_intent(
        &self,
        room_id: String,
        actor: String,
        summary: String,
        status: Option<String>,
        footprint: Option<Vec<String>>,
        expected_completion: Option<String>,
        repo: Option<String>,
        branch: Option<String>,
        task: Option<String>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "room_id": room_id, "actor": actor, "summary": summary });
        insert_opt_string(&mut args, "status", status);
        insert_opt_strings(&mut args, "footprint", footprint);
        insert_opt_string(&mut args, "expected_completion", expected_completion);
        insert_opt_string(&mut args, "repo", repo);
        insert_opt_string(&mut args, "branch", branch);
        insert_opt_string(&mut args, "task", task);
        with_invoker(|inv| {
            Ok(Json(
                inv.coordination_intent(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Write a durable coordination record.
    #[allow(clippy::too_many_arguments)]
    async fn write_coordination_record(
        &self,
        room_id: String,
        actor: String,
        record_type: String,
        summary: String,
        title: Option<String>,
        body: Option<String>,
        metadata: Option<Json>,
    ) -> GqlResult<Json> {
        let mut args = json!({
            "room_id": room_id,
            "actor": actor,
            "record_type": record_type,
            "summary": summary,
        });
        insert_opt_string(&mut args, "title", title);
        insert_opt_string(&mut args, "body", body);
        if let Some(metadata) = metadata {
            args["metadata"] = metadata.0;
        }
        with_invoker(|inv| {
            Ok(Json(
                inv.coordination_record(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Publish a stream event through the durable coordination stream path.
    async fn publish_coordination_event(
        &self,
        stream: String,
        actor: String,
        kind: String,
        payload: Option<Json>,
        urgency: Option<String>,
        target_actor: Option<String>,
        created_at: Option<String>,
    ) -> GqlResult<CoordinationStreamPublish> {
        let mut args = json!({
            "stream": stream,
            "actor": actor,
            "kind": kind,
            "urgency": urgency.unwrap_or_else(|| "info".to_string()),
        });
        if let Some(payload) = payload {
            args["payload"] = payload.0;
        }
        insert_opt_string(&mut args, "target_actor", target_actor);
        insert_opt_string(&mut args, "created_at", created_at);
        with_invoker(|inv| {
            Ok(CoordinationStreamPublish::from_value(
                inv.stream_publish(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Read stream events and advance the actor cursor past the returned window.
    async fn advance_coordination_stream(
        &self,
        actor: String,
        stream: Option<String>,
        streams: Option<Vec<String>>,
        limit: Option<i32>,
    ) -> GqlResult<CoordinationStreamRead> {
        let mut args = json!({ "actor": actor, "advance": true });
        insert_opt_string(&mut args, "stream", stream);
        insert_opt_strings(&mut args, "streams", streams);
        insert_opt_i32(&mut args, "limit", limit);
        with_invoker(|inv| {
            Ok(CoordinationStreamRead::from_value(
                inv.stream_read(args.clone(), true).map_err(map_err)?,
            ))
        })
    }

    /// Create or reuse a native multi-head task node.
    #[allow(clippy::too_many_arguments)]
    async fn create_task_node(
        &self,
        run_id: String,
        goal: String,
        node_id: Option<String>,
        kind: Option<String>,
        actor: Option<String>,
        prerequisites: Option<Vec<String>>,
        file_scope: Option<Vec<String>>,
    ) -> GqlResult<TaskNodeWrite> {
        let mut args = json!({ "run_id": run_id, "goal": goal });
        insert_opt_string(&mut args, "node_id", node_id);
        insert_opt_string(&mut args, "kind", kind);
        insert_opt_string(&mut args, "actor", actor);
        insert_opt_strings(&mut args, "prerequisites", prerequisites);
        insert_opt_strings(&mut args, "file_scope", file_scope);
        with_invoker(|inv| {
            Ok(TaskNodeWrite::from_value(
                inv.task_node(args.clone()).map_err(map_err)?,
            ))
        })
    }

    /// Claim or release a native multi-head task node.
    async fn claim_task_node(
        &self,
        run_id: String,
        node_id: String,
        owner: Option<String>,
        actor: Option<String>,
        action: Option<String>,
        expected_epoch: Option<i64>,
        lease_ttl_seconds: Option<i64>,
        now_ms: Option<i64>,
    ) -> GqlResult<Json> {
        let mut args = json!({ "run_id": run_id, "node_id": node_id });
        insert_opt_string(&mut args, "owner", owner);
        insert_opt_string(&mut args, "actor", actor);
        insert_opt_string(&mut args, "action", action);
        insert_opt_i64(&mut args, "expected_epoch", expected_epoch);
        insert_opt_i64(&mut args, "lease_ttl_seconds", lease_ttl_seconds);
        insert_opt_i64(&mut args, "now_ms", now_ms);
        with_invoker(|inv| Ok(Json(inv.claim_task_node(args.clone()).map_err(map_err)?)))
    }
}

fn str_field(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn insert_opt_string(target: &mut Value, key: &str, value: Option<String>) {
    if let Some(value) = value {
        target[key] = json!(value);
    }
}

fn insert_opt_strings(target: &mut Value, key: &str, value: Option<Vec<String>>) {
    if let Some(value) = value {
        target[key] = json!(value);
    }
}

fn insert_opt_i32(target: &mut Value, key: &str, value: Option<i32>) {
    if let Some(value) = value {
        target[key] = json!(value);
    }
}

fn insert_opt_i64(target: &mut Value, key: &str, value: Option<i64>) {
    if let Some(value) = value {
        target[key] = json!(value);
    }
}
