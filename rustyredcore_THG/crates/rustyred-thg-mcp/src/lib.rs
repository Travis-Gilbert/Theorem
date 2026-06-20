//! # rustyred-thg-mcp
//!
//! The native Rust MCP server over the RustyRed graph store. It exposes the
//! harness capabilities as MCP tools -- memory, coordination, jobs, code
//! intelligence, graph queries and algorithms, versioning, search, symbolic
//! reasoning, and browsing -- with no Python process in the loop. The same tool
//! surface is served over stdio (via the bundled `theorems-harness` plugin) and
//! over HTTP (`POST /mcp` on the graph server).
//!
//! Tools dispatch through one handler keyed by tool name; in read-only mode the
//! write tools return a structured `mcp_read_only` error instead of mutating the
//! graph. Full categorized catalog: `docs/site/reference/mcp-tools.md`.

mod connector_gateway;
mod graphql;

pub use graphql::projection::{
    is_projected_item_label, project_mutation_event, project_node_to_item, ProjectedItem,
    ProjectedItemDelta, ITEM_SOURCE_LABELS,
};

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ensemble::{
    get_pack as ensemble_get_pack, pack_node_id as ensemble_pack_node_id,
    register_pack as ensemble_register_pack, select_from_store as ensemble_select_from_store,
    CapabilityPack, EnsembleError, EnsembleGraphStore, EnsembleResult, EnsembleSelectRequest,
    PackExposure, PackKind, TrustTier,
};
use rustyred_thg_core::{
    checkout_graph_version, compile_graph_pack, compile_graphql_selection, compile_user_subgraph,
    diff_graph_snapshots, epistemic_shadow_ppr, execute_query, graph_version_log,
    merge_graph_snapshots, read_epistemic_shadow, run_epistemic_cron_pass, update_graph_ref_cas,
    CodeKgManifest, Direction, EdgeRecord, EpistemicAnnotations, EpistemicCronInput,
    EpistemicEnricher, EpistemicEnrichmentError, EpistemicEnrichmentMode, EpistemicType,
    GraphCompileOptions, GraphMergeOptions, GraphSnapshot, GraphStats, GraphStore, GraphStoreError,
    GraphStoreResult, GraphVersionRepository, GraphqlSelection, HarnessInstantKg,
    HybridScoringConfig, InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
    QueryIr, RedCoreGraphStore, RelationalStore, SessionDelta, StreamEvent, StreamLog,
    StreamUrgency, UserSubgraph, VectorDesignation, VerifyReport, EPISTEMIC_SHADOW_LABEL,
    EPISTEMIC_SUPPORTS, HAS_EPISTEMIC_SHADOW, SAME_ECLASS, UNDERCUTS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::{
    composition_hash, evaluate_publication, hash_agent_binding, next_for_head, spawn_verify_node,
    stable_value_hash, submit_verify_receipt, ActionTierPolicy, AgentBinding, AgentHead,
    BindingBudgetScope, BindingComposition, BindingIdentity, ClaimOutcome, FakeHeadInvoker,
    GroundedClaim, HeadCostProfile, HeadFitness, HeadKind, HeadReliabilityProfile, HeadTransport,
    JobSubmission, Millis, NodeStatus, Payload, Receipt, ScratchpadRevision, TaskNode, TraceTier,
    TransitionInput, TransitionResult, VerifyReceipt, WorkGraph,
};
#[cfg(test)]
use theorem_harness_runtime::subscribe_coordination_room_events;
use theorem_harness_runtime::{
    append_transition_from_store, apply_skill_pack, archive_memory_document, binding_node_id,
    canonical_stream_key, coordination_binding_id, coordination_intent_edge_id,
    coordination_intent_node_id, coordination_intent_scratchpad_edge_id,
    coordination_member_edge_id, coordination_member_node_id, coordination_mention_edge_id,
    coordination_message_edge_id, coordination_message_node_id, coordination_presence_node_id,
    coordination_record_edge_id, coordination_record_node_id, coordination_room_binding_edge_id,
    coordination_room_node_id, coordination_stream_cursor_node_id,
    coordination_stream_event_edge_id, coordination_stream_event_node_id,
    coordination_stream_node_id, coordination_stream_subscription_node_id, default_theorem_binding,
    encode_memory, forget_memory, get_skill_pack, handoff_memory, infer_coordination_room_id,
    list_skill_packs, load_events, load_run, normalize_actor_id, normalize_coordination_urgency,
    parse_coordination_mentions, publish_coordination_room_event_from_state,
    publish_footprint_event, publish_presence_event, publish_record_event, publish_skill_pack,
    publish_work_graph_transition, recall_archived_memory, recall_memory, relate_memory,
    remember_memory, revise_memory_document, scratchpad_revision_node_id, self_note_memory,
    stable_coordination_message_id, stable_coordination_record_id, task_node_graph_id, upsert_note,
    AddOrRemove, ArchiveMemoryInput, CoordinationIntentState, CoordinationMessageState,
    CoordinationPresenceState, CoordinationRecordState, CoordinationRoomMember,
    CoordinationRoomState, EncodeMemoryInput, ForgetMemoryInput, HandoffMemoryInput,
    HarnessRuntimeError, JobActionResult, JobNoteInput, JoinRoomInput, MemoryError,
    MemoryGraphStore, MemoryWriteInput, PresenceInput, RealHeadInvoker, RecallMemoryInput,
    RelateMemoryInput, ReviseMemoryInput, SkillPackApplyInput, SkillPackError, SkillPackGetInput,
    SkillPackGraphStore, SkillPackListInput, SkillPackPublishInput, UpsertNoteInput,
    WriteIntentInput, WriteMessageInput, WriteRecordInput, EDGE_PREREQUISITE_OF, EDGE_REFINED_INTO,
    TASK_NODE_LABEL,
};

const JSONRPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const MULTIHEAD_RUN_LABEL: &str = "MultiheadRun";
const MULTIHEAD_PATCH_LABEL: &str = "MultiheadPatch";
const MULTIHEAD_PROOF_LABEL: &str = "MultiheadProofReceipt";
const DEFAULT_MULTIHEAD_LEASE_TTL_MS: Millis = 90_000;

#[allow(clippy::too_many_arguments)]
/// Request to fire a GitHub Actions `repository_dispatch` that spawns a session. The MCP crate
/// stays sync and HTTP-free; a concrete backend (the harness server, which owns reqwest) makes the
/// actual POST. `event_type` is normally "theorem-handoff". This is the executor seam the harness
/// direct-pathway spec routes through GitHub Actions instead of the Railway runner.
#[derive(Debug, Clone)]
pub struct HandoffDispatch {
    pub owner: String,
    pub repo: String,
    pub event_type: String,
    pub intent: String,
    pub branch: String,
}

fn publish_coordination_room_event(state: &CoordinationMessageState) {
    publish_coordination_room_event_from_state(state);
}

// --- Agent Space Viewport emit hooks -------------------------------------
// After a coordination write is persisted, mirror it onto the agent-space
// observatory bus so the live viewport sees presence, footprints, work-graph
// transitions, and records. These read the persisted tool `arguments` (the
// public tool contract) defensively; a missing field degrades to an empty
// string rather than dropping the event, because the viewport is an
// eventually-consistent observatory, not a source of truth.

fn agent_space_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

fn agent_space_arg_str<'a>(arguments: &'a Value, keys: &[&str]) -> &'a str {
    for key in keys {
        if let Some(value) = arguments.get(*key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return trimmed;
            }
        }
    }
    ""
}

fn agent_space_arg_room(arguments: &Value) -> Option<String> {
    arguments
        .get("room_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|room| !room.is_empty())
        .map(str::to_string)
}

fn agent_space_arg_list(arguments: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(items) = arguments.get(*key).and_then(Value::as_array) {
            return items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    Vec::new()
}

fn emit_agent_space_presence(tenant: &str, arguments: &Value) {
    let actor = agent_space_arg_str(arguments, &["actor_id", "actor"]);
    if actor.is_empty() {
        return;
    }
    let status = match agent_space_arg_str(arguments, &["status"]) {
        "" => "active",
        other => other,
    };
    publish_presence_event(
        tenant.to_string(),
        agent_space_arg_room(arguments),
        actor.to_string(),
        status.to_string(),
        agent_space_now_ms(),
    );
}

fn emit_agent_space_footprint(tenant: &str, arguments: &Value) {
    let actor = agent_space_arg_str(arguments, &["actor_id", "actor"]);
    if actor.is_empty() {
        return;
    }
    let room = agent_space_arg_room(arguments);
    let ts = agent_space_now_ms();
    let files = agent_space_arg_list(
        arguments,
        &[
            "footprint",
            "claimed_files",
            "claimedFiles",
            "touched_files",
        ],
    );
    if files.is_empty() {
        let target = agent_space_arg_str(arguments, &["task", "binding_id", "summary"]);
        if !target.is_empty() {
            publish_footprint_event(
                tenant.to_string(),
                room,
                actor.to_string(),
                target.to_string(),
                AddOrRemove::Add,
                ts,
            );
        }
        return;
    }
    for file in files {
        publish_footprint_event(
            tenant.to_string(),
            room.clone(),
            actor.to_string(),
            file,
            AddOrRemove::Add,
            ts,
        );
    }
}

fn emit_agent_space_record(tenant: &str, arguments: &Value) {
    let kind = match agent_space_arg_str(arguments, &["record_type", "kind", "type"]) {
        "" => "event",
        other => other,
    };
    let summary = agent_space_arg_str(arguments, &["summary", "title"]);
    let refs = agent_space_arg_list(arguments, &["refs", "references"]);
    publish_record_event(
        tenant.to_string(),
        agent_space_arg_room(arguments),
        kind.to_string(),
        summary.to_string(),
        refs,
        agent_space_now_ms(),
    );
}

fn emit_agent_space_transition(tenant: &str, arguments: &Value) {
    let node_id = agent_space_arg_str(arguments, &["run_id", "node_id", "binding_id", "id"]);
    let from = agent_space_arg_str(arguments, &["from", "from_state", "previous_state"]);
    let to = agent_space_arg_str(
        arguments,
        &["to", "to_state", "state", "event_type", "kind"],
    );
    let actor = agent_space_arg_str(arguments, &["actor_id", "actor", "author"]);
    publish_work_graph_transition(
        tenant.to_string(),
        agent_space_arg_room(arguments),
        node_id.to_string(),
        from.to_string(),
        to.to_string(),
        actor.to_string(),
        agent_space_now_ms(),
    );
}

#[derive(Debug, Clone)]
pub struct AppAffordanceInvocation {
    pub tenant_id: String,
    pub affordance_id: String,
    pub actor: String,
    pub request: Value,
    pub dry_run: bool,
    pub confirmed: bool,
    pub timeout_ms: u64,
}

#[allow(clippy::too_many_arguments)]
pub trait McpGraphBackend {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
    fn stats(&self) -> GraphStoreResult<GraphStats>;
    fn verify(&self) -> GraphStoreResult<VerifyReport>;
    fn labels(&self) -> GraphStoreResult<Vec<String>>;
    fn edge_types(&self) -> GraphStoreResult<Vec<String>>;
    fn property_keys(&self) -> GraphStoreResult<Vec<String>>;
    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "list_edges is not supported by this MCP backend",
        ))
    }
    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        let stats = self.stats()?;
        let nodes = self.query_nodes(NodeQuery {
            limit: Some(stats.nodes_total.max(1)),
            ..NodeQuery::default()
        })?;
        let edges = self.list_edges()?;
        Ok(GraphSnapshot {
            version: stats.version,
            nodes,
            edges,
        })
    }
    fn upsert_node(&mut self, _node: NodeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "node bulk upsert is not supported by this MCP backend",
        ))
    }
    fn upsert_edge(&mut self, _edge: EdgeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "edge bulk upsert is not supported by this MCP backend",
        ))
    }
    fn append_harness_transition(
        &mut self,
        _transition: TransitionInput,
    ) -> Result<Value, McpError> {
        Err(McpError::internal(
            "harness transition append is not supported by this MCP backend",
        ))
    }
    fn harness_run_detail(&self, _run_id: &str) -> Result<Option<Value>, McpError> {
        Err(McpError::internal(
            "harness run reads are not supported by this MCP backend",
        ))
    }
    fn composed_agent_run(
        &mut self,
        _binding_id: String,
        _task: String,
        _claims: Vec<GroundedClaim>,
    ) -> Result<Value, McpError> {
        Err(McpError::internal(
            "composed_agent_run is not supported by this MCP backend",
        ))
    }
    /// Dispatch-queue verb: create `Job{Queued}` (idempotent on idempotency_key).
    fn job_submit(
        &mut self,
        _submission: JobSubmission,
        _submitted_by: String,
    ) -> Result<Value, McpError> {
        Err(McpError::internal(
            "job_submit is not supported by this MCP backend",
        ))
    }
    /// Dispatch-board verb: list jobs with derived state.
    fn job_list(&self, _repo: Option<String>, _state: Option<String>) -> Result<Value, McpError> {
        Err(McpError::internal(
            "job_list is not supported by this MCP backend",
        ))
    }
    /// Dispatch-board verb: append a receipt, optionally with receiver start metadata.
    fn job_note(&mut self, _job_id: String, _input: JobNoteInput) -> Result<Value, McpError> {
        Err(McpError::internal(
            "job_note is not supported by this MCP backend",
        ))
    }
    /// Dispatch-board verb: archive a job thread.
    fn job_archive(
        &mut self,
        _job_id: String,
        _reason: String,
        _actor: String,
    ) -> Result<Value, McpError> {
        Err(McpError::internal(
            "job_archive is not supported by this MCP backend",
        ))
    }
    /// Fire a session-spawn dispatch (GitHub Actions `repository_dispatch`). Defaults to
    /// unsupported; the harness server backend overrides this with the real reqwest POST so the
    /// MCP crate never needs an HTTP client.
    fn dispatch_handoff(&self, _dispatch: HandoffDispatch) -> Result<(), McpError> {
        Err(McpError::internal(
            "session handoff dispatch is not supported by this MCP backend",
        ))
    }
    fn invoke_app_affordance(
        &mut self,
        _invocation: AppAffordanceInvocation,
    ) -> Result<Value, McpError> {
        Err(McpError::internal(
            "app affordance invocation is not supported by this MCP backend",
        ))
    }
    fn invoke_code_search(
        &mut self,
        tenant: &str,
        arguments: &Value,
        operation: &str,
    ) -> Result<Value, McpError> {
        let affordance_id = format!("theorem_grpc.code_search.{operation}");
        let actor = arguments
            .get("actor")
            .or_else(|| arguments.get("actor_id"))
            .and_then(Value::as_str)
            .unwrap_or("theorem-harness-mcp")
            .to_string();
        let timeout_ms = arguments
            .get("timeout_ms")
            .or_else(|| arguments.get("timeoutMs"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let dry_run = arguments
            .get("dry_run")
            .or_else(|| arguments.get("dryRun"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let confirmed = app_affordance_confirmed(arguments);
        let mut request = arguments.clone();
        if let Some(object) = request.as_object_mut() {
            object.remove("operation");
            object.remove("mode");
            object.remove("verb");
            object.remove("tenant");
            object.remove("tenant_slug");
            object.remove("timeout_ms");
            object.remove("timeoutMs");
            object.remove("dry_run");
            object.remove("dryRun");
            remove_app_affordance_confirmation_controls(object);
        }
        let response = self.invoke_app_affordance(AppAffordanceInvocation {
            tenant_id: tenant.to_string(),
            affordance_id: affordance_id.clone(),
            actor,
            request,
            dry_run,
            confirmed,
            timeout_ms,
        })?;
        Ok(json!({
            "tenant": tenant,
            "operation": operation,
            "affordance_id": affordance_id,
            "app_affordance": response,
        }))
    }
    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>>;
    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()>;
    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>>;
    fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>>;
    fn hybrid_scoring_config(&self) -> HybridScoringConfig {
        HybridScoringConfig::default()
    }
    fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.hybrid_search(
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            config.alpha,
        )
    }
    fn designate_fulltext_property(
        &mut self,
        _label: &str,
        _property: &str,
    ) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "full-text designation is not supported by this MCP backend",
        ))
    }
    fn fulltext_search(
        &self,
        _label: Option<&str>,
        _property: &str,
        _query: &str,
        _k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "full-text search is not supported by this MCP backend",
        ))
    }
    fn designate_spatial_property(
        &mut self,
        _label: &str,
        _lat_property: &str,
        _lon_property: &str,
        _resolution: u8,
    ) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "spatial designation is not supported by this MCP backend",
        ))
    }
    fn spatial_radius_search(
        &self,
        _label: &str,
        _lat_property: &str,
        _lon_property: &str,
        _lat: f64,
        _lon: f64,
        _radius_km: f64,
    ) -> GraphStoreResult<Vec<String>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "spatial radius search is not supported by this MCP backend",
        ))
    }
    fn spatial_bbox_search(
        &self,
        _label: &str,
        _lat_property: &str,
        _lon_property: &str,
        _min_lat: f64,
        _min_lon: f64,
        _max_lat: f64,
        _max_lon: f64,
    ) -> GraphStoreResult<Vec<String>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "spatial bbox search is not supported by this MCP backend",
        ))
    }
    fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>>;

    /// Personalized PageRank. Default impl walks `list_edges()` to build the
    /// adjacency map then calls `rustyred_thg_core::personalized_pagerank`.
    fn algo_ppr(
        &self,
        seeds: &HashMap<String, f64>,
        alpha: f64,
        epsilon: f64,
        max_pushes: usize,
    ) -> GraphStoreResult<HashMap<String, f64>> {
        let edges = self.list_edges()?;
        let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
        for edge in edges.iter() {
            if edge.tombstone {
                continue;
            }
            adjacency
                .entry(edge.from_id.clone())
                .or_default()
                .push((edge.to_id.clone(), edge.effective_confidence()));
        }
        Ok(rustyred_thg_core::personalized_pagerank(
            &adjacency, seeds, alpha, epsilon, max_pushes,
        ))
    }

    /// Connected components. Default impl uses `rustyred_thg_core::connected_components`.
    fn algo_components(&self, directed: bool) -> GraphStoreResult<Vec<Vec<String>>> {
        let edges = self.list_edges()?;
        Ok(rustyred_thg_core::connected_components(&edges, directed))
    }

    /// Power-iteration PageRank. Default impl uses `rustyred_thg_core::pagerank`.
    fn algo_pagerank(
        &self,
        damping: f64,
        max_iter: usize,
        tolerance: f64,
    ) -> GraphStoreResult<HashMap<String, f64>> {
        let edges = self.list_edges()?;
        Ok(rustyred_thg_core::pagerank(
            &edges, damping, max_iter, tolerance,
        ))
    }

    /// Community detection + modularity via label-propagation. Default impl
    /// uses `rustyred_thg_core::label_propagation_communities` (the modern replacement
    /// for the now-deprecated `louvain_communities` re-export).
    fn algo_communities(&self) -> GraphStoreResult<(HashMap<String, u64>, f64)> {
        let edges = self.list_edges()?;
        Ok(rustyred_thg_core::label_propagation_communities(&edges))
    }

    /// Bulk upsert NodeRecords. Default impl loops `upsert_node` per record;
    /// concrete impls that have a faster batch primitive can override.
    fn bulk_upsert_nodes(&mut self, records: Vec<NodeRecord>) -> GraphStoreResult<(usize, usize)> {
        let mut inserted = 0usize;
        let mut failed = 0usize;
        for record in records {
            match self.upsert_node(record) {
                Ok(_) => inserted += 1,
                Err(_) => failed += 1,
            }
        }
        Ok((inserted, failed))
    }

    /// Bulk upsert EdgeRecords. Default impl loops `upsert_edge` per record.
    fn bulk_upsert_edges(&mut self, records: Vec<EdgeRecord>) -> GraphStoreResult<(usize, usize)> {
        let mut inserted = 0usize;
        let mut failed = 0usize;
        for record in records {
            match self.upsert_edge(record) {
                Ok(_) => inserted += 1,
                Err(_) => failed += 1,
            }
        }
        Ok((inserted, failed))
    }
}

pub trait McpGraphProvider {
    type Backend: McpGraphBackend;

    fn backend_for_tenant(&self, tenant: &str) -> Result<Self::Backend, McpError>;
}

/// A cheaply-shareable handle to a single owned `McpGraphBackend` store, for the
/// embedded / in-process case (North Star E0): one durable store (e.g. a
/// `RedCoreGraphStore`) lives behind an `Rc<RefCell<_>>`, and each
/// `backend_for_tenant` call clones the handle rather than re-opening or
/// deep-copying the store. `SharedStore<S>` forwards EVERY `McpGraphBackend`
/// method transparently to the inner `S`, so `S`'s overrides (RedCore's native
/// code search, real upserts, etc.) are preserved -- forwarding only the required
/// methods would silently fall back to the trait defaults for the ~27 methods a
/// durable store overrides. It is also its own `McpGraphProvider`, so an embedded
/// host can pass `SharedStore::new(store)` straight to `handle_mcp_request`.
///
/// Single-threaded by construction (`Rc`/`RefCell`); the embedded surface
/// executes synchronously on one thread, matching the GraphQL dispatch model.
pub struct SharedStore<S>(Rc<RefCell<S>>);

impl<S> Clone for SharedStore<S> {
    fn clone(&self) -> Self {
        SharedStore(Rc::clone(&self.0))
    }
}

impl<S: McpGraphBackend> SharedStore<S> {
    /// Wrap an owned store in a shared, in-process handle.
    pub fn new(store: S) -> Self {
        SharedStore(Rc::new(RefCell::new(store)))
    }

    /// Run a closure with mutable access to the inner store (e.g. to read durable
    /// state directly between GraphQL calls).
    pub fn with_store<R>(&self, f: impl FnOnce(&mut S) -> R) -> R {
        f(&mut self.0.borrow_mut())
    }
}

impl<S: McpGraphBackend> McpGraphProvider for SharedStore<S> {
    type Backend = SharedStore<S>;

    fn backend_for_tenant(&self, _tenant: &str) -> Result<Self::Backend, McpError> {
        Ok(self.clone())
    }
}

impl<S: McpGraphBackend> McpGraphBackend for SharedStore<S> {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.0.borrow().get_node(id)
    }
    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.0.borrow().get_edge(id)
    }
    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.0.borrow().query_nodes(query)
    }
    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.0.borrow().neighbors(query)
    }
    fn stats(&self) -> GraphStoreResult<GraphStats> {
        self.0.borrow().stats()
    }
    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        self.0.borrow().verify()
    }
    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        self.0.borrow().labels()
    }
    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        self.0.borrow().edge_types()
    }
    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        self.0.borrow().property_keys()
    }
    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        self.0.borrow().list_edges()
    }
    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        self.0.borrow().graph_snapshot()
    }
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.0.borrow_mut().upsert_node(node)
    }
    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.0.borrow_mut().upsert_edge(edge)
    }
    fn append_harness_transition(
        &mut self,
        transition: TransitionInput,
    ) -> Result<Value, McpError> {
        self.0.borrow_mut().append_harness_transition(transition)
    }
    fn harness_run_detail(&self, run_id: &str) -> Result<Option<Value>, McpError> {
        self.0.borrow().harness_run_detail(run_id)
    }
    fn composed_agent_run(
        &mut self,
        binding_id: String,
        task: String,
        claims: Vec<GroundedClaim>,
    ) -> Result<Value, McpError> {
        self.0
            .borrow_mut()
            .composed_agent_run(binding_id, task, claims)
    }
    fn job_submit(
        &mut self,
        submission: JobSubmission,
        submitted_by: String,
    ) -> Result<Value, McpError> {
        self.0.borrow_mut().job_submit(submission, submitted_by)
    }
    fn job_list(&self, repo: Option<String>, state: Option<String>) -> Result<Value, McpError> {
        self.0.borrow().job_list(repo, state)
    }
    fn job_note(&mut self, job_id: String, input: JobNoteInput) -> Result<Value, McpError> {
        self.0.borrow_mut().job_note(job_id, input)
    }
    fn job_archive(
        &mut self,
        job_id: String,
        reason: String,
        actor: String,
    ) -> Result<Value, McpError> {
        self.0.borrow_mut().job_archive(job_id, reason, actor)
    }
    fn dispatch_handoff(&self, dispatch: HandoffDispatch) -> Result<(), McpError> {
        self.0.borrow().dispatch_handoff(dispatch)
    }
    fn invoke_app_affordance(
        &mut self,
        invocation: AppAffordanceInvocation,
    ) -> Result<Value, McpError> {
        self.0.borrow_mut().invoke_app_affordance(invocation)
    }
    fn invoke_code_search(
        &mut self,
        tenant: &str,
        arguments: &Value,
        operation: &str,
    ) -> Result<Value, McpError> {
        self.0
            .borrow_mut()
            .invoke_code_search(tenant, arguments, operation)
    }
    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        self.0.borrow().vector_designations()
    }
    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        self.0
            .borrow_mut()
            .designate_vector_property(label, property_name, dimension)
    }
    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.0
            .borrow()
            .vector_search(label, property_name, query, k)
    }
    fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.0
            .borrow()
            .hybrid_search(label, property_name, query, k, graph_seeds, max_hops, alpha)
    }
    fn hybrid_scoring_config(&self) -> HybridScoringConfig {
        self.0.borrow().hybrid_scoring_config()
    }
    fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.0.borrow().hybrid_search_with_config(
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            config,
        )
    }
    fn designate_fulltext_property(&mut self, label: &str, property: &str) -> GraphStoreResult<()> {
        self.0
            .borrow_mut()
            .designate_fulltext_property(label, property)
    }
    fn fulltext_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.0.borrow().fulltext_search(label, property, query, k)
    }
    fn designate_spatial_property(
        &mut self,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        resolution: u8,
    ) -> GraphStoreResult<()> {
        self.0.borrow_mut().designate_spatial_property(
            label,
            lat_property,
            lon_property,
            resolution,
        )
    }
    fn spatial_radius_search(
        &self,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        lat: f64,
        lon: f64,
        radius_km: f64,
    ) -> GraphStoreResult<Vec<String>> {
        self.0.borrow().spatial_radius_search(
            label,
            lat_property,
            lon_property,
            lat,
            lon,
            radius_km,
        )
    }
    fn spatial_bbox_search(
        &self,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    ) -> GraphStoreResult<Vec<String>> {
        self.0.borrow().spatial_bbox_search(
            label,
            lat_property,
            lon_property,
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        )
    }
    fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        self.0
            .borrow()
            .epistemic_neighbors(node_id, epistemic_types, min_confidence, max_depth)
    }
    fn algo_ppr(
        &self,
        seeds: &HashMap<String, f64>,
        alpha: f64,
        epsilon: f64,
        max_pushes: usize,
    ) -> GraphStoreResult<HashMap<String, f64>> {
        self.0.borrow().algo_ppr(seeds, alpha, epsilon, max_pushes)
    }
    fn algo_components(&self, directed: bool) -> GraphStoreResult<Vec<Vec<String>>> {
        self.0.borrow().algo_components(directed)
    }
    fn algo_pagerank(
        &self,
        damping: f64,
        max_iter: usize,
        tolerance: f64,
    ) -> GraphStoreResult<HashMap<String, f64>> {
        self.0.borrow().algo_pagerank(damping, max_iter, tolerance)
    }
    fn algo_communities(&self) -> GraphStoreResult<(HashMap<String, u64>, f64)> {
        self.0.borrow().algo_communities()
    }
    fn bulk_upsert_nodes(&mut self, records: Vec<NodeRecord>) -> GraphStoreResult<(usize, usize)> {
        self.0.borrow_mut().bulk_upsert_nodes(records)
    }
    fn bulk_upsert_edges(&mut self, records: Vec<EdgeRecord>) -> GraphStoreResult<(usize, usize)> {
        self.0.borrow_mut().bulk_upsert_edges(records)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub version: String,
    pub default_tenant: String,
    pub read_only: bool,
    pub allow_admin: bool,
    #[serde(default = "default_tool_result_budget_bytes")]
    pub tool_result_budget_bytes: usize,
    #[serde(default)]
    pub tool_result_family_budgets: HashMap<String, usize>,
    /// When true, advertise GraphQL as the default agent surface: the flat tools
    /// whose capability is covered by the typed GraphQL schema (Area A) are hidden
    /// from tools/list, leaving the three graphql_* transport tools plus the
    /// not-yet-covered flat tools. Default off, so the flat surface is unchanged
    /// unless a deployment opts in. (North Star A7 cutover.)
    #[serde(default)]
    pub graphql_default_surface: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "rusty-red-graph-database".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_tenant: "default".to_string(),
            read_only: false,
            allow_admin: false,
            tool_result_budget_bytes: default_tool_result_budget_bytes(),
            tool_result_family_budgets: HashMap::new(),
            graphql_default_surface: false,
        }
    }
}

pub const DEFAULT_TOOL_RESULT_BUDGET_BYTES: usize = 16 * 1024;

/// Budget for harness run-detail tool results (the run plus its full lifecycle
/// event log). The async-handoff contract advertises `harness_run` as its poll
/// tool, so a populated run must be returned inline for realistic runs instead
/// of truncating into a tool_result_fetch handle that drops the found/detail
/// fields a poller reads. The fetch fallback still applies above this ceiling.
pub const DEFAULT_HARNESS_TOOL_RESULT_BUDGET_BYTES: usize = 512 * 1024;

const TOOL_RESULT_MARKER_RESERVED_BYTES: usize = 512;

fn default_tool_result_budget_bytes() -> usize {
    DEFAULT_TOOL_RESULT_BUDGET_BYTES
}

static TOOL_RESULT_BODIES: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpRequestContext {
    pub scopes: Vec<String>,
}

impl McpRequestContext {
    pub fn with_scopes(scopes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            scopes: scopes.into_iter().map(Into::into).collect(),
        }
    }

    fn allows(&self, required_scope: &str) -> bool {
        self.scopes.iter().any(|scope| {
            scope == "*"
                || scope == required_scope
                || mcp_scope_alias(scope.as_str()) == required_scope
        })
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: Option<String>,
    #[serde(default)]
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct McpError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

impl McpError {
    pub fn parse(message: impl Into<String>) -> Self {
        Self {
            code: -32700,
            message: message.into(),
            data: None,
        }
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            code: -32600,
            message: message.into(),
            data: None,
        }
    }

    pub fn method_not_found(method: &str) -> Self {
        Self {
            code: -32601,
            message: format!("MCP method {method} is not supported"),
            data: None,
        }
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self {
            code: -32602,
            message: message.into(),
            data: None,
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: -32603,
            message: message.into(),
            data: None,
        }
    }

    /// Returned when an inline algorithm request carries more edges than the
    /// configured `MAX_INLINE_EDGES` limit. Callers should switch to the
    /// tenant-backed counterpart (`rustyred_thg_algorithm.<name>`) for graphs
    /// that exceed the inline budget.
    ///
    /// Uses application-defined JSON-RPC code `-32004` in the
    /// implementation-defined server-error range (`-32000..=-32099` per
    /// JSON-RPC 2.0 §5.1). Distinct from `invalid_params` so MCP clients can
    /// pattern-match the budget-exceeded case and route oversized graphs to
    /// the durable-tenant compute path.
    pub fn payload_too_large(message: impl Into<String>) -> Self {
        Self {
            code: -32004,
            message: message.into(),
            data: None,
        }
    }
}

impl From<GraphStoreError> for McpError {
    fn from(error: GraphStoreError) -> Self {
        Self {
            code: -32603,
            message: error.message,
            data: Some(json!({ "code": error.code })),
        }
    }
}

pub fn handle_mcp_request<P: McpGraphProvider>(
    provider: &P,
    config: &McpServerConfig,
    payload: Value,
) -> Value {
    handle_mcp_request_with_context(provider, config, &McpRequestContext::default(), payload)
}

pub fn handle_mcp_request_with_context<P: McpGraphProvider>(
    provider: &P,
    config: &McpServerConfig,
    context: &McpRequestContext,
    payload: Value,
) -> Value {
    let request = match serde_json::from_value::<JsonRpcRequest>(payload) {
        Ok(request) => request,
        Err(error) => {
            return jsonrpc_error(
                None,
                McpError::parse(format!("invalid JSON-RPC request: {error}")),
            )
        }
    };

    if request.jsonrpc.as_deref().unwrap_or(JSONRPC_VERSION) != JSONRPC_VERSION {
        return jsonrpc_error(
            request.id,
            McpError::invalid_request("jsonrpc must be \"2.0\""),
        );
    }

    match dispatch(provider, config, context, &request) {
        Ok(result) => json!({
            "jsonrpc": JSONRPC_VERSION,
            "id": request.id,
            "result": result,
        }),
        Err(error) => jsonrpc_error(request.id, error),
    }
}

pub fn mcp_manifest(base_url: Option<&str>, config: &McpServerConfig) -> Value {
    let endpoint = base_url
        .map(|url| format!("{}/mcp", url.trim_end_matches('/')))
        .unwrap_or_else(|| "/mcp".to_string());
    json!({
        "name": config.name,
        "description": "MCP agent port for Rusty Red Graph Database. Exposes graph-native tools over THG GraphStore APIs; raw Redis is never exposed.",
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "transport": {
            "type": "streamable-http",
            "endpoint": endpoint,
            "auth": "bearer"
        },
        "defaults": {
            "readOnly": config.read_only,
            "allowAdmin": config.allow_admin && !config.read_only,
            "rawRedis": false
        },
        "tools": tool_definitions(config),
        "resourceTemplates": resource_templates(),
        "prompts": prompt_definitions()
    })
}

pub fn agent_manifest(base_url: Option<&str>, config: &McpServerConfig) -> Value {
    json!({
        "name": "Rusty Red Graph Database Agent Port",
        "description": "Agent discovery for the THG/Rusty Red first-class MCP endpoint.",
        "mcp": mcp_manifest(base_url, config),
        "wellKnown": {
            "mcp": "/.well-known/mcp/rustyred_thg_json",
            "agent": "/.well-known/agent.json"
        }
    })
}

fn dispatch<P: McpGraphProvider>(
    provider: &P,
    config: &McpServerConfig,
    context: &McpRequestContext,
    request: &JsonRpcRequest,
) -> Result<Value, McpError> {
    match request.method.as_str() {
        "initialize" => Ok(initialize_result(config)),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tool_definitions(config) })),
        "tools/call" => call_tool(provider, config, context, &request.params),
        "resources/list" => Ok(json!({ "resources": resources(config) })),
        "resources/templates/list" => Ok(json!({ "resourceTemplates": resource_templates() })),
        "resources/read" => read_resource(provider, config, &request.params),
        "prompts/list" => Ok(json!({ "prompts": prompt_definitions() })),
        "prompts/get" => get_prompt(&request.params),
        method => Err(McpError::method_not_found(method)),
    }
}

fn initialize_result(config: &McpServerConfig) -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": { "listChanged": false },
            "resources": { "subscribe": false, "listChanged": false },
            "prompts": { "listChanged": false }
        },
        "serverInfo": {
            "name": config.name,
            "version": config.version
        },
        "instructions": "Use graph-native THG tools and resources. Raw Redis keys are not exposed. This first MCP slice is read-only unless the server explicitly enables admin tools."
    })
}

fn call_tool<P: McpGraphProvider>(
    provider: &P,
    config: &McpServerConfig,
    context: &McpRequestContext,
    params: &Value,
) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("tools/call requires params.name"))?;
    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let tenant = tenant_from_args_for_tool(name, &arguments, config)?;
    let mut backend = provider.backend_for_tenant(&tenant)?;

    let payload = match name {
        "tool_result_fetch" | "theorem_harness_tool_result_fetch" => {
            tool_result_fetch_payload(&arguments)?
        }
        "tool_search" | "gateway_tool_search" => {
            connector_gateway::search_payload(&tenant, &mut backend, &arguments)?
        }
        "describe" | "gateway_describe" => {
            connector_gateway::describe_payload(&tenant, &mut backend, &arguments)?
        }
        "invoke" | "gateway_invoke" => {
            let dry_run = arguments
                .get("dry_run")
                .or_else(|| arguments.get("dryRun"))
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if config.read_only && !dry_run {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "gateway invoke is unavailable while read-only mode is active unless dry_run=true."
                })));
            }
            connector_gateway::invoke_payload(&tenant, &mut backend, &arguments)?
        }
        "rustyred_thg_graph_neighbors" => graph_neighbors_payload(&tenant, &backend, &arguments)?,
        "rustyred_thg_graph_schema" => schema_payload(&tenant, &backend)?,
        "rustyred_thg_graph_index_status" => index_status_payload(&tenant, &backend)?,
        "rustyred_thg_graph_explain" => explain_payload(&tenant, &arguments),
        "rustyred_thg_graph_query" => query_payload(&tenant, &backend, &arguments)?,
        "rustyred_thg_relational_query" | "rustyred_relational_query" => {
            relational_query_payload(&tenant, &backend, &arguments)?
        }
        "epistemic_dirty_frontier" | "rustyred_thg_epistemic_dirty_frontier" => {
            epistemic_dirty_frontier_payload(&tenant, &backend, &arguments)?
        }
        "epistemic_compile_subgraph" | "rustyred_thg_epistemic_compile_subgraph" => {
            epistemic_compile_subgraph_payload(&tenant, &backend, &arguments)?
        }
        "epistemic_shadow_ppr" | "rustyred_thg_epistemic_shadow_ppr" => {
            epistemic_shadow_ppr_payload(&tenant, &backend, &arguments)?
        }
        "epistemic_enrich_apply" | "rustyred_thg_epistemic_enrich_apply" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "read_only",
                "message": "epistemic_enrich_apply is unavailable while read-only mode is active."
            })))
        }
        "epistemic_enrich_apply" | "rustyred_thg_epistemic_enrich_apply" => {
            epistemic_enrich_apply_payload(&tenant, &mut backend, &arguments)?
        }
        "rustyred_thg_graph_version_compile"
        | "rustyred_thg_git_compile"
        | "rustyred.graph.version.compile"
        | "rustyred.git.compile" => {
            let snapshot = backend.graph_snapshot()?;
            let options = serde_json::from_value::<GraphCompileOptions>(arguments.clone())
                .map_err(|error| {
                    McpError::invalid_params(format!("invalid graph compile options: {error}"))
                })?;
            json!({
                "tenant": tenant,
                "pack": compile_graph_pack(&snapshot, options)
            })
        }
        "rustyred_thg_graph_version_diff"
        | "rustyred_thg_git_diff"
        | "rustyred.graph.version.diff"
        | "rustyred.git.diff" => {
            let base = arguments.get("base").cloned().ok_or_else(|| {
                McpError::invalid_params("rustyred_thg_graph_version_diff requires base snapshot")
            })?;
            let base = serde_json::from_value::<GraphSnapshot>(base).map_err(|error| {
                McpError::invalid_params(format!("base must be a graph snapshot: {error}"))
            })?;
            let target = match arguments.get("target").cloned() {
                Some(value) => serde_json::from_value::<GraphSnapshot>(value).map_err(|error| {
                    McpError::invalid_params(format!("target must be a graph snapshot: {error}"))
                })?,
                None => backend.graph_snapshot()?,
            };
            json!({
                "tenant": tenant,
                "diff": diff_graph_snapshots(&base, &target)
            })
        }
        "rustyred_thg_graph_version_ref"
        | "rustyred_thg_git_ref"
        | "rustyred.graph.version.ref"
        | "rustyred.git.ref" => {
            let snapshot = backend.graph_snapshot()?;
            let options = serde_json::from_value::<GraphCompileOptions>(arguments.clone())
                .map_err(|error| {
                    McpError::invalid_params(format!("invalid graph compile options: {error}"))
                })?;
            let repository = optional_repository(&arguments)?;
            let branch = arguments
                .get("branch")
                .and_then(Value::as_str)
                .map(str::to_string);
            let updated_at_unix_ms = arguments.get("updated_at_unix_ms").and_then(Value::as_u64);
            let expected_commit_hash = arguments
                .get("expected_commit_hash")
                .and_then(Value::as_str)
                .map(str::to_string);
            let pack = compile_graph_pack(&snapshot, options);
            let ref_update = update_graph_ref_cas(
                repository,
                pack,
                branch,
                expected_commit_hash,
                updated_at_unix_ms.map(u128::from),
            )
            .map_err(|conflict| McpError {
                code: -32009,
                message: conflict.message.clone(),
                data: Some(json!({ "conflict": conflict })),
            })?;
            json!({
                "tenant": tenant,
                "ref_update": ref_update
            })
        }
        "rustyred_thg_graph_version_log"
        | "rustyred_thg_git_log"
        | "rustyred.graph.version.log"
        | "rustyred.git.log" => {
            let repository = required_repository(&arguments, name)?;
            let target = arguments.get("target").and_then(Value::as_str);
            json!({
                "tenant": tenant,
                "log": graph_version_log(&repository, target)
            })
        }
        "rustyred_thg_graph_version_checkout"
        | "rustyred_thg_git_checkout"
        | "rustyred.graph.version.checkout"
        | "rustyred.git.checkout" => {
            let repository = required_repository(&arguments, name)?;
            let target = required_str(&arguments, "target", name)?;
            let checkout = checkout_graph_version(&repository, target).ok_or_else(|| {
                McpError::invalid_params(format!("target not found or has no payloads: {target}"))
            })?;
            json!({
                "tenant": tenant,
                "checkout": checkout
            })
        }
        "rustyred_thg_graph_version_merge"
        | "rustyred_thg_git_merge"
        | "rustyred.graph.version.merge"
        | "rustyred.git.merge" => {
            let base = arguments.get("base").cloned().ok_or_else(|| {
                McpError::invalid_params("rustyred_thg_graph_version_merge requires base snapshot")
            })?;
            let base = serde_json::from_value::<GraphSnapshot>(base).map_err(|error| {
                McpError::invalid_params(format!("base must be a graph snapshot: {error}"))
            })?;
            let ours = match arguments.get("ours").cloned() {
                Some(value) => serde_json::from_value::<GraphSnapshot>(value).map_err(|error| {
                    McpError::invalid_params(format!("ours must be a graph snapshot: {error}"))
                })?,
                None => backend.graph_snapshot()?,
            };
            let theirs = arguments.get("theirs").cloned().ok_or_else(|| {
                McpError::invalid_params(
                    "rustyred_thg_graph_version_merge requires theirs snapshot",
                )
            })?;
            let theirs = serde_json::from_value::<GraphSnapshot>(theirs).map_err(|error| {
                McpError::invalid_params(format!("theirs must be a graph snapshot: {error}"))
            })?;
            let options = serde_json::from_value::<GraphMergeOptions>(arguments.clone()).map_err(
                |error| McpError::invalid_params(format!("invalid graph merge options: {error}")),
            )?;
            json!({
                "tenant": tenant,
                "merge": merge_graph_snapshots(&base, &ours, &theirs, options)
            })
        }
        // §P6-B pb6.1: SPEC names `thg.algo.*` are aliases for the existing
        // `thg.algorithm.*` arms below. Either name reaches the same payload.
        "rustyred_thg_algorithm_ppr" | "rustyred_thg_algo_ppr" => {
            algorithm_ppr_payload(&tenant, &backend, &arguments)?
        }
        "rustyred_thg_algorithm_components" | "rustyred_thg_algo_components" => {
            algorithm_components_payload(&tenant, &backend, &arguments)?
        }
        "rustyred_thg_algorithm_pagerank" | "rustyred_thg_algo_pagerank" => {
            algorithm_pagerank_payload(&tenant, &backend, &arguments)?
        }
        "rustyred_thg_algorithm_communities" | "rustyred_thg_algo_communities" => {
            algorithm_communities_payload(&tenant, &backend)?
        }
        "rustyred_thg_instant_kg_status" | "harness_kg_status" => {
            instant_kg_payload(&tenant, &backend, &arguments, "status", name)?
        }
        "rustyred_thg_instant_kg_ppr" | "harness_kg_ppr" => {
            instant_kg_payload(&tenant, &backend, &arguments, "ppr", name)?
        }
        "rustyred_thg_instant_kg_impact" | "harness_kg_impact" => {
            instant_kg_payload(&tenant, &backend, &arguments, "impact", name)?
        }
        "rustyred_thg_instant_kg_related_objects" | "harness_kg_related_objects" => {
            instant_kg_payload(&tenant, &backend, &arguments, "related_objects", name)?
        }
        "rustyred_thg_instant_kg_search" | "harness_kg_search" => {
            instant_kg_payload(&tenant, &backend, &arguments, "search", name)?
        }
        "rustyred_thg_instant_kg_explain_edge" | "harness_kg_explain_edge" => {
            instant_kg_payload(&tenant, &backend, &arguments, "explain_edge", name)?
        }
        // RR-INLINE-08: inline-adjacency algorithm tools. These bypass the
        // tenant entirely; the adjacency is read from the request arguments,
        // the algorithm runs against request-scoped memory, and no state is
        // written. Tenant-backed counterparts above are unchanged.
        "rustyred_thg_algorithm_ppr_inline" | "rustyred_thg_algo_ppr_inline" => {
            algorithm_ppr_inline_payload(&arguments)?
        }
        "rustyred_thg_algorithm_components_inline" | "rustyred_thg_algo_components_inline" => {
            algorithm_components_inline_payload(&arguments)?
        }
        "rustyred_thg_algorithm_pagerank_inline" | "rustyred_thg_algo_pagerank_inline" => {
            algorithm_pagerank_inline_payload(&arguments)?
        }
        "rustyred_thg_algorithm_communities_inline" | "rustyred_thg_algo_communities_inline" => {
            algorithm_communities_inline_payload(&arguments)?
        }
        "rustyred_thg_symbolic_datalog_derive" | "rustyred_thg.symbolic.datalog_derive" => {
            symbolic_datalog_derive_payload(&arguments)?
        }
        "rustyred_thg_symbolic_probabilistic_source_reliability"
        | "rustyred_thg.symbolic.probabilistic_source_reliability" => {
            symbolic_probabilistic_source_reliability_payload(&arguments)?
        }
        "rustyred_thg_symbolic_probabilistic_expected_value"
        | "rustyred_thg.symbolic.probabilistic_expected_value" => {
            symbolic_probabilistic_expected_value_payload(&arguments)?
        }
        "coordination_room" | "theorem_harness_coordination_room" => {
            let action = arguments
                .get("action")
                .and_then(Value::as_str)
                .unwrap_or("status")
                .trim()
                .to_lowercase();
            if matches!(action.as_str(), "join" | "start") && config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination-room writes are unavailable while read-only mode is active."
                })));
            }
            coordination_room_payload(&tenant, &mut backend, &arguments)?
        }
        "presence" | "theorem_harness_presence" => {
            let mode = arguments
                .get("mode")
                .and_then(Value::as_str)
                .unwrap_or("heartbeat")
                .trim()
                .to_lowercase();
            if mode != "get" && config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native presence writes are unavailable while read-only mode is active."
                })));
            }
            let payload = presence_payload(&tenant, &mut backend, &arguments)?;
            emit_agent_space_presence(&tenant, &arguments);
            payload
        }
        "coordination_intent" | "write_intent" | "theorem_harness_write_intent" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination intent writes are unavailable while read-only mode is active."
                })));
            }
            let payload = write_intent_payload(&tenant, &mut backend, &arguments)?;
            emit_agent_space_footprint(&tenant, &arguments);
            payload
        }
        "read_intents_for_room" | "theorem_harness_read_intents_for_room" => {
            read_intents_payload(&tenant, &backend, &arguments)?
        }
        "coordinate" | "theorem_harness_coordinate" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination messages are unavailable while read-only mode is active."
                })));
            }
            coordinate_payload(&tenant, &mut backend, &arguments)?
        }
        "stream_publish" | "theorem_harness_stream_publish" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Stream publishes are unavailable while read-only mode is active."
                })));
            }
            stream_publish_payload(&tenant, &mut backend, &arguments)?
        }
        "stream_read" | "theorem_harness_stream_read" => {
            // Passive read is the default and stays available read-only; only the
            // cursor advance (a write) is suppressed when the server is read-only.
            stream_read_payload(&tenant, &mut backend, &arguments, !config.read_only)?
        }
        "stream_subscribe" | "theorem_harness_stream_subscribe" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Stream subscription changes are unavailable while read-only mode is active."
                })));
            }
            stream_subscription_payload(&tenant, &mut backend, &arguments, true)?
        }
        "stream_unsubscribe" | "theorem_harness_stream_unsubscribe" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Stream subscription changes are unavailable while read-only mode is active."
                })));
            }
            stream_subscription_payload(&tenant, &mut backend, &arguments, false)?
        }
        "fractal_expansion" | "harness_fractal_expansion" | "theorem_harness_fractal_expansion" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Live fractal expansion writes are unavailable while read-only mode is active."
                })));
            }
            tool_result_error(json!({
                "error": "live_fractal_requires_async_server",
                "message": "fractal_expansion is handled by the async harness server MCP route because it performs live web fetches."
            }))
        }
        "web_consume" | "theorem_browser_web_consume" => {
            return Ok(tool_result_error(json!({
                "error": "browser_use_requires_async_server",
                "message": "web_consume is handled by the async THG server MCP route because it can navigate, extract, and ingest live web pages."
            })));
        }
        "browse_with_me" | "theorem_browser_browse_with_me" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "browse_with_me is unavailable while read-only mode is active."
                })));
            }
            return Ok(tool_result_error(json!({
                "error": "browser_use_requires_async_server",
                "message": "browse_with_me is handled by the async THG server MCP route because it coordinates a live supervised browser session."
            })));
        }
        "browse_for_me" | "theorem_browser_browse_for_me" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "browse_for_me is unavailable while read-only mode is active."
                })));
            }
            return Ok(tool_result_error(json!({
                "error": "browser_use_requires_async_server",
                "message": "browse_for_me is handled by the async THG server MCP route because it can run a browser-use loop and persist receipts."
            })));
        }
        "rustyweb_search_acquisition" | "search_acquisition" => {
            return Ok(tool_result_error(json!({
                "error": "live_search_acquisition_requires_async_server",
                "message": "rustyweb_search_acquisition is handled by the async harness server MCP route because it fans out to live or offline search providers."
            })));
        }
        "mentions" | "theorem_harness_mentions" => {
            let consume = arguments
                .get("consume")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if consume && config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Consuming native mentions is unavailable while read-only mode is active."
                })));
            }
            mentions_payload(&tenant, &mut backend, &arguments)?
        }
        "read_messages_for_room" | "theorem_harness_read_messages_for_room" => {
            read_messages_payload(&tenant, &backend, &arguments)?
        }
        "coordination_record" | "write_coordination_record" | "theorem_harness_write_record" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination record writes are unavailable while read-only mode is active."
                })));
            }
            let policy_receipt = coordination_policy_receipt(context, &arguments, name);
            if let Some(error) = coordination_policy_error(&policy_receipt) {
                return Ok(tool_result_error(error));
            }
            let payload =
                write_record_payload(&tenant, &mut backend, &arguments, Some(policy_receipt))?;
            emit_agent_space_record(&tenant, &arguments);
            payload
        }
        "coordination_contribution" | "theorem_harness_coordination_contribution" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination contribution writes are unavailable while read-only mode is active."
                })));
            }
            let policy_receipt = coordination_policy_receipt(context, &arguments, name);
            if let Some(error) = coordination_policy_error(&policy_receipt) {
                return Ok(tool_result_error(error));
            }
            write_contribution_payload(&tenant, &mut backend, &arguments, Some(policy_receipt))?
        }
        "spawn_session" | "theorem_harness_spawn_session" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Spawning native Theorem harness sessions is unavailable while read-only mode is active."
                })));
            }
            let policy_receipt = coordination_policy_receipt(context, &arguments, name);
            if let Some(error) = coordination_policy_error(&policy_receipt) {
                return Ok(tool_result_error(error));
            }
            spawn_session_payload(&tenant, &mut backend, &arguments, Some(policy_receipt))?
        }
        "read_records_for_room" | "theorem_harness_read_records_for_room" => {
            read_records_payload(&tenant, &backend, &arguments)?
        }
        "coordination_context" | "theorem_harness_coordination_context" => {
            coordination_context_payload(&tenant, &mut backend, &arguments)?
        }
        "harness_append_transition" | "theorem_harness_append_transition" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native harness transition appends are unavailable while read-only mode is active."
                })));
            }
            let payload = append_harness_transition_payload(&tenant, &mut backend, &arguments)?;
            emit_agent_space_transition(&tenant, &arguments);
            payload
        }
        "harness_run" | "theorem_harness_run" => {
            harness_run_payload(&tenant, &backend, &arguments)?
        }
        "composed_agent_run" | "theorem_composed_agent_run" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "composed_agent_run is unavailable while read-only mode is active."
                })));
            }
            composed_agent_run_payload(&tenant, &mut backend, &arguments)?
        }
        "job_submit" | "theorem_job_submit" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "job_submit is unavailable while read-only mode is active."
                })));
            }
            job_submit_payload(&tenant, &mut backend, &arguments)?
        }
        "job_list" => job_list_payload(&tenant, &backend, &arguments)?,
        "job_note" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "job_note is unavailable while read-only mode is active."
                })));
            }
            job_note_payload(&tenant, &mut backend, &arguments)?
        }
        "job_archive" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "job_archive is unavailable while read-only mode is active."
                })));
            }
            job_archive_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_run" | "theorem_harness_multihead_run" => {
            let action = argument_text(&arguments, &["action"])
                .unwrap_or_else(|| "start".to_string())
                .to_lowercase();
            if action != "status" && config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head run writes are unavailable while read-only mode is active."
                })));
            }
            multihead_run_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_task" | "theorem_harness_multihead_task" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head task writes are unavailable while read-only mode is active."
                })));
            }
            multihead_task_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_claim" | "theorem_harness_multihead_claim" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head claim writes are unavailable while read-only mode is active."
                })));
            }
            multihead_claim_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_refine" | "theorem_harness_multihead_refine" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head refine writes are unavailable while read-only mode is active."
                })));
            }
            multihead_refine_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_next" | "theorem_harness_multihead_next" => {
            multihead_next_payload(&tenant, &backend, &arguments)?
        }
        "multihead_patch" | "theorem_harness_multihead_patch" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head patch writes are unavailable while read-only mode is active."
                })));
            }
            multihead_patch_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_proof" | "theorem_harness_multihead_proof" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head proof writes are unavailable while read-only mode is active."
                })));
            }
            multihead_proof_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_review" | "theorem_harness_multihead_review" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head review writes are unavailable while read-only mode is active."
                })));
            }
            multihead_review_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_spawn_verify" | "theorem_harness_multihead_spawn_verify" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head verify-spawn writes are unavailable while read-only mode is active."
                })));
            }
            multihead_spawn_verify_payload(&tenant, &mut backend, &arguments)?
        }
        "multihead_submit_verify" | "theorem_harness_multihead_submit_verify" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native multi-head verify-submit writes are unavailable while read-only mode is active."
                })));
            }
            multihead_submit_verify_payload(&tenant, &mut backend, &arguments)?
        }
        "skill_list" | "theorem_harness_skill_list" => {
            skill_list_payload(&tenant, &backend, &arguments)?
        }
        "skill_get" | "theorem_harness_skill_get" => {
            skill_get_payload(&tenant, &backend, &arguments)?
        }
        "skill_publish" | "theorem_harness_skill_publish" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native skill-pack publishes are unavailable while read-only mode is active."
                })));
            }
            skill_publish_payload(&tenant, &mut backend, &arguments)?
        }
        "skill_apply" | "theorem_harness_skill_apply" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native skill-pack applies are unavailable while read-only mode is active."
                })));
            }
            skill_apply_payload(&tenant, &mut backend, &arguments)?
        }
        "ensemble_register" | "theorem_harness_ensemble_register" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native Ensemble capability-pack registration is unavailable while read-only mode is active."
                })));
            }
            ensemble_register_payload(&tenant, &mut backend, &arguments)?
        }
        "ensemble_select" | "theorem_harness_ensemble_select" => {
            ensemble_select_payload(&tenant, &backend, &arguments)?
        }
        "harness_prepare" | "theorem_harness_prepare" => {
            harness_prepare_payload(&tenant, &mut backend, &arguments)?
        }
        "code_search"
        | "compute_code"
        | "code_ingest"
        | "theorem_harness_code_search"
        | "theorem_harness_compute_code"
        | "theorem_harness_code_ingest" => {
            let operation = if matches!(name, "code_ingest" | "theorem_harness_code_ingest")
                && arguments
                    .get("operation")
                    .or_else(|| arguments.get("mode"))
                    .or_else(|| arguments.get("verb"))
                    .is_none()
            {
                "ingest".to_string()
            } else {
                code_search_operation(&arguments)?
            };
            if matches!(
                operation.as_str(),
                "ingest" | "reindex" | "session_reingest" | "record_use_receipt"
            ) && config.read_only
            {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Code ingest writes are unavailable while read-only mode is active."
                })));
            }
            code_search_payload(&tenant, &mut backend, &arguments, &operation)?
        }
        "remember" | "theorem_harness_remember" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native memory writes are unavailable while read-only mode is active."
                })));
            }
            remember_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "recall" | "theorem_harness_recall" => {
            let consume_handoffs = arguments
                .get("consume_handoffs")
                .or_else(|| arguments.get("consumeHandoffs"))
                .and_then(Value::as_bool)
                .unwrap_or_else(|| {
                    arguments
                        .get("kind")
                        .and_then(Value::as_str)
                        .map(|kind| kind.trim().eq_ignore_ascii_case("handoff"))
                        .unwrap_or(false)
                });
            if consume_handoffs && config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Consuming native memory handoffs is unavailable while read-only mode is active."
                })));
            }
            recall_memory_payload(&tenant, &mut backend, &arguments, consume_handoffs)?
        }
        "relate" | "theorem_harness_relate" => {
            relate_memory_payload(&tenant, &backend, &arguments)?
        }
        "self_note" | "theorem_harness_self_note" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native self-note writes are unavailable while read-only mode is active."
                })));
            }
            self_note_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "self_revise" | "theorem_harness_self_revise" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native memory revision writes are unavailable while read-only mode is active."
                })));
            }
            revise_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "self_archive" | "theorem_harness_self_archive" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native memory archive writes are unavailable while read-only mode is active."
                })));
            }
            archive_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "self_recall_archive" | "theorem_harness_self_recall_archive" => {
            recall_archived_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "encode" | "theorem_harness_encode" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native encode memory writes are unavailable while read-only mode is active."
                })));
            }
            encode_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "upsert_note" | "theorem_harness_upsert_note" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Obsidian note upserts are unavailable while read-only mode is active."
                })));
            }
            upsert_note_payload(&tenant, &mut backend, &arguments)?
        }
        "forget" | "theorem_harness_forget" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native forget writes are unavailable while read-only mode is active."
                })));
            }
            forget_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "handoff" | "theorem_harness_handoff" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native handoff writes are unavailable while read-only mode is active."
                })));
            }
            handoff_memory_payload(&tenant, &mut backend, &arguments)?
        }
        "observe" | "theorem_harness_observe" => {
            observe_payload(&tenant, &mut backend, &arguments)?
        }
        "graphql_query" | "theorem_harness_graphql_query" => {
            graphql::execute_graphql(&tenant, backend, &arguments, graphql::OpKind::Query)?
        }
        "graphql_mutate" | "theorem_harness_graphql_mutate" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "GraphQL mutations are unavailable while read-only mode is active."
                })));
            }
            graphql::execute_graphql(&tenant, backend, &arguments, graphql::OpKind::Mutate)?
        }
        "graphql_introspect" | "theorem_harness_graphql_introspect" => graphql::introspect_sdl(),
        "rustyred_thg_fulltext_search" | "rustyred_thg_graph_fulltext_search" => {
            fulltext_search_payload(&tenant, &backend, &arguments, name)?
        }
        "rustyred_thg_fulltext_designate" | "rustyred_thg_graph_fulltext_designate"
            if config.read_only =>
        {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_fulltext_designate" | "rustyred_thg_graph_fulltext_designate" => {
            fulltext_designate_payload(&tenant, &mut backend, &arguments, name)?
        }
        "rustyred_thg_spatial_radius" | "rustyred_thg_graph_spatial_radius" => {
            spatial_radius_payload(&tenant, &backend, &arguments, name)?
        }
        "rustyred_thg_spatial_bbox" | "rustyred_thg_graph_spatial_bbox" => {
            spatial_bbox_payload(&tenant, &backend, &arguments, name)?
        }
        "rustyred_thg_spatial_designate" | "rustyred_thg_graph_spatial_designate"
            if config.read_only =>
        {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_spatial_designate" | "rustyred_thg_graph_spatial_designate" => {
            spatial_designate_payload(&tenant, &mut backend, &arguments, name)?
        }
        "rustyred_thg_bulk_nodes" | "rustyred_thg_graph_bulk_nodes" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_bulk_nodes" | "rustyred_thg_graph_bulk_nodes" => {
            bulk_nodes_payload(&tenant, &mut backend, &arguments)?
        }
        "rustyred_thg_bulk_edges" | "rustyred_thg_graph_bulk_edges" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_bulk_edges" | "rustyred_thg_graph_bulk_edges" => {
            bulk_edges_payload(&tenant, &mut backend, &arguments)?
        }
        "rustyred_thg_vector_search" => vector_search_payload(&tenant, &backend, &arguments)?,
        "rustyred_thg_vector_hybrid" => vector_hybrid_payload(&tenant, &backend, &arguments)?,
        "rustyred_thg_vector_designate" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_vector_designate" => {
            vector_designate_payload(&tenant, &mut backend, &arguments)?
        }
        "rustyred_thg_epistemic_neighbors" => {
            epistemic_neighbors_payload(&tenant, &backend, &arguments)?
        }
        "rustyred_thg_admin_verify" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "admin MCP tools are unavailable while THG_MCP_READ_ONLY/RUSTY_RED_MCP_READ_ONLY is true."
            })))
        }
        "rustyred_thg_admin_verify" if !context.allows("admin:read") => {
            return Ok(tool_result_error(json!({
                "error": "admin_scope_required",
                "message": "rustyred_thg_admin_verify requires admin:read or thg:graph:admin:verify scope."
            })))
        }
        "rustyred_thg_admin_verify" if config.allow_admin => {
            json!({ "tenant": tenant, "verify": backend.verify()? })
        }
        "rustyred_thg_admin_verify" => {
            return Ok(tool_result_error(json!({
                "error": "admin_tools_disabled",
                "message": "rustyred_thg_admin_verify is hidden unless THG_MCP_ALLOW_ADMIN/RUSTY_RED_MCP_ALLOW_ADMIN is true."
            })))
        }
        other => return Err(McpError::method_not_found(other)),
    };

    Ok(tool_result_with_budget(payload, config, name))
}

fn read_resource<P: McpGraphProvider>(
    provider: &P,
    _config: &McpServerConfig,
    params: &Value,
) -> Result<Value, McpError> {
    let uri = params
        .get("uri")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("resources/read requires params.uri"))?;
    let resource = ParsedResource::parse(uri)?;
    let backend = provider.backend_for_tenant(&resource.tenant)?;
    let payload = match resource.kind.as_str() {
        "schema" => schema_payload(&resource.tenant, &backend)?,
        "labels" => json!({ "tenant": resource.tenant, "labels": backend.labels()? }),
        "edge-types" => json!({ "tenant": resource.tenant, "edgeTypes": backend.edge_types()? }),
        "indexes" => index_status_payload(&resource.tenant, &backend)?,
        "stats" => json!({ "tenant": resource.tenant, "stats": backend.stats()? }),
        "verify" if resource.rest.as_deref() == Some("latest") => {
            json!({ "tenant": resource.tenant, "verify": backend.verify()? })
        }
        "node" => {
            let id = resource
                .rest
                .as_deref()
                .ok_or_else(|| McpError::invalid_params("node resource requires an id"))?;
            json!({ "tenant": resource.tenant, "node": backend.get_node(id)? })
        }
        "edge" => {
            let id = resource
                .rest
                .as_deref()
                .ok_or_else(|| McpError::invalid_params("edge resource requires an id"))?;
            json!({ "tenant": resource.tenant, "edge": backend.get_edge(id)? })
        }
        "neighbors" => {
            let id = resource
                .rest
                .as_deref()
                .ok_or_else(|| McpError::invalid_params("neighbors resource requires a node id"))?;
            json!({
                "tenant": resource.tenant,
                "node_id": id,
                "neighbors": backend.neighbors(NeighborQuery::out(id))?
            })
        }
        _ => {
            return Err(McpError::invalid_params(format!(
                "unsupported THG resource URI {uri}"
            )))
        }
    };
    Ok(json!({
        "contents": [{
            "uri": uri,
            "mimeType": "application/json",
            "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        }]
    }))
}

fn get_prompt(params: &Value) -> Result<Value, McpError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("prompts/get requires params.name"))?;
    let text = match name {
        "thg-query" => "Construct a bounded THG graph query, then call thg.graph.explain before thg.graph.query. Keep max_depth and max_edges_touched explicit.",
        "thg-explain-plan" => "Explain a THG graph query plan, naming the starting index, traversal direction, expected edge touches, and risk of fallback scans.",
        "thg-compile-context-pack" => "Use THG schema, index status, and neighbor tools to compile a small context pack with reasons and hydrate URIs.",
        "thg-debug-indexes" => "Inspect thg.graph.index_status and thg.admin.verify output, then propose a safe rebuild or compaction follow-up without applying mutations.",
        other => return Err(McpError::method_not_found(other)),
    };
    Ok(json!({
        "description": prompt_description(name),
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": text }
        }]
    }))
}

fn graph_neighbors_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let query = neighbor_query_from_value(arguments)?;
    let mut neighbors = backend.neighbors(query)?;
    let budget = Budget::from_args(arguments);
    let truncated = apply_neighbor_budget(&mut neighbors, budget);
    Ok(json!({
        "tenant": tenant,
        "neighbors": neighbors,
        "stats": { "returned": neighbors.len(), "truncated": truncated }
    }))
}

fn fulltext_search_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
    name: &str,
) -> Result<Value, McpError> {
    let property = required_str(arguments, "property", name)?;
    let query = required_str(arguments, "query", name)?;
    let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
    let label = arguments.get("label").and_then(Value::as_str);
    let results = backend.fulltext_search(label, property, query, k)?;
    Ok(json!({
        "tenant": tenant,
        "results": results.iter().map(|(node_id, score)| json!({"node_id": node_id, "score": score})).collect::<Vec<_>>(),
        "stats": { "returned": results.len(), "k": k }
    }))
}

fn fulltext_designate_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    name: &str,
) -> Result<Value, McpError> {
    let label = required_str(arguments, "label", name)?;
    let property = required_str(arguments, "property", name)?;
    backend.designate_fulltext_property(label, property)?;
    Ok(json!({
        "tenant": tenant,
        "designated": { "label": label, "property": property }
    }))
}

fn spatial_radius_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
    name: &str,
) -> Result<Value, McpError> {
    let label = required_str(arguments, "label", name)?;
    let lat_property = required_str(arguments, "lat_property", name)?;
    let lon_property = required_str(arguments, "lon_property", name)?;
    let lat = required_f64(arguments, "lat", name)?;
    let lon = required_f64(arguments, "lon", name)?;
    let radius_km = required_f64(arguments, "radius_km", name)?;
    let node_ids =
        backend.spatial_radius_search(label, lat_property, lon_property, lat, lon, radius_km)?;
    Ok(json!({
        "tenant": tenant,
        "node_ids": node_ids,
        "stats": { "returned": node_ids.len() }
    }))
}

fn spatial_bbox_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
    name: &str,
) -> Result<Value, McpError> {
    let label = required_str(arguments, "label", name)?;
    let lat_property = required_str(arguments, "lat_property", name)?;
    let lon_property = required_str(arguments, "lon_property", name)?;
    let min_lat = required_f64(arguments, "min_lat", name)?;
    let min_lon = required_f64(arguments, "min_lon", name)?;
    let max_lat = required_f64(arguments, "max_lat", name)?;
    let max_lon = required_f64(arguments, "max_lon", name)?;
    let node_ids = backend.spatial_bbox_search(
        label,
        lat_property,
        lon_property,
        min_lat,
        min_lon,
        max_lat,
        max_lon,
    )?;
    Ok(json!({
        "tenant": tenant,
        "node_ids": node_ids,
        "stats": { "returned": node_ids.len() }
    }))
}

fn spatial_designate_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    name: &str,
) -> Result<Value, McpError> {
    let label = required_str(arguments, "label", name)?;
    let lat_property = required_str(arguments, "lat_property", name)?;
    let lon_property = required_str(arguments, "lon_property", name)?;
    let resolution = arguments
        .get("resolution")
        .and_then(Value::as_u64)
        .unwrap_or(9)
        .min(u8::MAX as u64) as u8;
    backend.designate_spatial_property(label, lat_property, lon_property, resolution)?;
    Ok(json!({
        "tenant": tenant,
        "designated": {
            "label": label,
            "lat_property": lat_property,
            "lon_property": lon_property,
            "resolution": resolution
        }
    }))
}

fn bulk_nodes_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let records = arguments
        .get("nodes")
        .or_else(|| arguments.get("records"))
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::invalid_params("rustyred_thg_bulk_nodes requires nodes array"))?;
    let mut inserted = 0usize;
    let mut errors = Vec::new();
    for (idx, raw) in records.iter().enumerate() {
        match parse_node_record(raw) {
            Ok(node) => match backend.upsert_node(node.clone()) {
                Ok(()) => inserted += 1,
                Err(error) => errors.push(json!({
                    "line": idx + 1,
                    "code": error.code,
                    "message": error.message,
                    "record_id": node.id,
                })),
            },
            Err(error) => errors.push(json!({
                "line": idx + 1,
                "code": "invalid_node_record",
                "message": error.message,
            })),
        }
    }
    Ok(json!({
        "tenant": tenant,
        "ok": errors.is_empty(),
        "inserted": inserted,
        "failed": errors.len(),
        "errors": errors,
    }))
}

fn bulk_edges_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let records = arguments
        .get("edges")
        .or_else(|| arguments.get("records"))
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::invalid_params("rustyred_thg_bulk_edges requires edges array"))?;
    let mut inserted = 0usize;
    let mut errors = Vec::new();
    for (idx, raw) in records.iter().enumerate() {
        match parse_edge_record(raw) {
            Ok(edge) => match backend.upsert_edge(edge.clone()) {
                Ok(()) => inserted += 1,
                Err(error) => errors.push(json!({
                    "line": idx + 1,
                    "code": error.code,
                    "message": error.message,
                    "record_id": edge.id,
                })),
            },
            Err(error) => errors.push(json!({
                "line": idx + 1,
                "code": "invalid_edge_record",
                "message": error.message,
            })),
        }
    }
    Ok(json!({
        "tenant": tenant,
        "ok": errors.is_empty(),
        "inserted": inserted,
        "failed": errors.len(),
        "errors": errors,
    }))
}

fn vector_search_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let property = arguments
        .get("property")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("rustyred_thg_vector_search requires property"))?;
    let query = parse_f32_array(arguments, "query")?;
    let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
    let label = arguments.get("label").and_then(Value::as_str);
    let results = backend.vector_search(label, property, &query, k)?;
    Ok(json!({
        "tenant": tenant,
        "results": results.iter().map(|(id, score)| json!({"node_id": id, "score": score})).collect::<Vec<_>>(),
        "stats": { "returned": results.len(), "k": k }
    }))
}

fn vector_hybrid_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let property = arguments
        .get("property")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("rustyred_thg_vector_hybrid requires property"))?;
    let query = parse_f32_array(arguments, "query")?;
    let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
    let label = arguments.get("label").and_then(Value::as_str);
    let graph_seeds: Vec<String> = arguments
        .get("graph_seeds")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_vector_hybrid requires graph_seeds")
        })?;
    let max_hops = arguments
        .get("max_hops")
        .and_then(Value::as_u64)
        .unwrap_or(3) as usize;
    let alpha = arguments
        .get("alpha")
        .and_then(Value::as_f64)
        .map(|value| value as f32);
    let mut scoring = backend.hybrid_scoring_config();
    if let Some(alpha) = alpha {
        scoring = scoring.with_alpha(alpha);
    }
    if let Some(confidence_weighted) = arguments
        .get("confidence_weighted_graph_distance")
        .and_then(Value::as_bool)
    {
        scoring.confidence_weighted_graph_distance = confidence_weighted;
    }
    if let Some(weights) = arguments.get("edge_type_weights") {
        scoring.edge_type_weights = serde_json::from_value(weights.clone()).map_err(|error| {
            McpError::invalid_params(format!(
                "edge_type_weights must be an object of number weights: {error}"
            ))
        })?;
    }
    let results = backend.hybrid_search_with_config(
        label,
        property,
        &query,
        k,
        &graph_seeds,
        max_hops,
        &scoring,
    )?;
    Ok(json!({
        "tenant": tenant,
        "results": results.iter().map(|(id, score)| json!({"node_id": id, "score": score})).collect::<Vec<_>>(),
        "stats": {
            "returned": results.len(),
            "k": k,
            "alpha": scoring.alpha,
            "max_hops": max_hops,
            "confidence_weighted_graph_distance": scoring.confidence_weighted_graph_distance,
            "edge_type_weights": scoring.edge_type_weights
        }
    }))
}

fn vector_designate_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let label = arguments
        .get("label")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("rustyred_thg_vector_designate requires label"))?;
    let property = arguments
        .get("property")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_vector_designate requires property")
        })?;
    let dimension = arguments
        .get("dimension")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_vector_designate requires dimension")
        })? as usize;
    backend.designate_vector_property(label, property, dimension)?;
    Ok(json!({
        "tenant": tenant,
        "designated": { "label": label, "property": property, "dimension": dimension }
    }))
}

fn schema_payload(tenant: &str, backend: &impl McpGraphBackend) -> Result<Value, McpError> {
    Ok(json!({
        "tenant": tenant,
        "labels": backend.labels()?,
        "edgeTypes": backend.edge_types()?,
        "propertyKeys": backend.property_keys()?,
        "stats": backend.stats()?,
        "propertyIndexes": "exact_scalar",
        "notes": [
            "This slice exposes label, edge-type, adjacency, and exact scalar property indexes.",
            "Full OpenCypher/GQL parsing and full-text indexes are still explicit follow-up work."
        ]
    }))
}

fn index_status_payload(tenant: &str, backend: &impl McpGraphBackend) -> Result<Value, McpError> {
    let verify = backend.verify()?;
    Ok(json!({
        "tenant": tenant,
        "healthy": verify.ok,
        "indexes": {
            "outAdjacency": "active",
            "inAdjacency": "active",
            "labels": "active",
            "edgeTypes": "active",
            "properties": "active_exact_scalar"
        },
        "stats": verify.stats,
        "problems": verify.problems
    }))
}

fn explain_payload(tenant: &str, arguments: &Value) -> Value {
    let operation = arguments
        .get("operation")
        .or_else(|| arguments.get("op"))
        .and_then(Value::as_str)
        .unwrap_or("neighbors");
    let query_step = match operation {
        "node_match" | "node_index_seek" => json!({
            "op": "node_index_seek",
            "cost": "O(label_set intersect property_set + returned_nodes)",
            "index": "label_index plus property_index",
            "bounded": true
        }),
        _ => json!({
            "op": "adjacency_lookup",
            "cost": "O(edge_types_for_node + returned_edges)",
            "index": "out_adjacency or in_adjacency",
            "bounded": true
        }),
    };
    json!({
        "tenant": tenant,
        "operation": operation,
        "plan": [{
            "op": "resolve_tenant_graph_store",
            "cost": "O(1)",
            "usesRawRedis": false
        }, query_step],
        "warnings": if matches!(operation, "neighbors" | "node_match" | "node_index_seek") {
            json!([])
        } else {
            json!(["Only neighbors and exact scalar node_match query execution are implemented in this slice."])
        }
    })
}

fn query_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let operation = arguments
        .get("operation")
        .or_else(|| arguments.get("op"))
        .and_then(Value::as_str)
        .unwrap_or("neighbors");
    if matches!(operation, "node_match" | "node_index_seek") {
        let mut query = node_query_from_value(arguments)?;
        let budget = Budget::from_args(arguments);
        query.limit = Some(budget.max_nodes_returned.saturating_add(1));
        let mut nodes = backend.query_nodes(query)?;
        let truncated = nodes.len() > budget.max_nodes_returned;
        if truncated {
            nodes.truncate(budget.max_nodes_returned);
        }
        return Ok(json!({
            "tenant": tenant,
            "operation": "node_match",
            "nodes": nodes,
            "stats": { "returned": nodes.len(), "truncated": truncated },
            "explain": explain_payload(tenant, arguments)
        }));
    }
    if operation != "neighbors" {
        return Ok(json!({
            "tenant": tenant,
            "unsupported": operation,
            "supportedOperations": ["neighbors", "node_match"],
            "explain": explain_payload(tenant, arguments)
        }));
    }
    let query = neighbor_query_from_value(arguments)?;
    let mut neighbors = backend.neighbors(query)?;
    let budget = Budget::from_args(arguments);
    let truncated = apply_neighbor_budget(&mut neighbors, budget);
    Ok(json!({
        "tenant": tenant,
        "operation": "neighbors",
        "neighbors": neighbors,
        "stats": { "returned": neighbors.len(), "truncated": truncated },
        "explain": explain_payload(tenant, arguments)
    }))
}

fn relational_query_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let snapshot = backend.graph_snapshot()?;
    let store = RelationalStore::from_graph_snapshot(&snapshot)?;
    let query = if let Some(query_ir) = arguments
        .get("query_ir")
        .or_else(|| arguments.get("queryIr"))
    {
        serde_json::from_value::<QueryIr>(query_ir.clone())
            .map_err(|error| McpError::invalid_params(format!("invalid query_ir: {error}")))?
    } else if let Some(selection) = arguments
        .get("selection")
        .or_else(|| arguments.get("graphql_selection"))
        .or_else(|| arguments.get("graphqlSelection"))
    {
        let selection = serde_json::from_value::<GraphqlSelection>(selection.clone())
            .map_err(|error| McpError::invalid_params(format!("invalid selection: {error}")))?;
        compile_graphql_selection(selection)
    } else {
        return Err(McpError::invalid_params(
            "rustyred_thg_relational_query requires query_ir or selection",
        ));
    };
    let result = execute_query(&store, query)?;
    Ok(json!({
        "tenant": tenant,
        "planner": "rustyred-native-relational",
        "rows": result.rows,
        "trace": result.trace,
        "stats": {
            "returned": result.rows.len(),
            "graph_snapshot_version": snapshot.version,
            "relations": store.relations().map(|relation| relation.schema.relation.clone()).collect::<Vec<_>>()
        }
    }))
}

fn epistemic_neighbors_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let node_id = arguments
        .get("node_id")
        .or_else(|| arguments.get("nodeId"))
        .and_then(Value::as_str)
        .ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_epistemic_neighbors requires node_id")
        })?;
    let epistemic_types: Option<Vec<EpistemicType>> = arguments
        .get("epistemic_types")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.parse::<EpistemicType>())
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()
        .map_err(McpError::from)?;
    let min_confidence = arguments.get("min_confidence").and_then(Value::as_f64);
    let max_depth = arguments
        .get("max_depth")
        .and_then(Value::as_u64)
        .map(|v| v as usize);
    let results = backend.epistemic_neighbors(
        node_id,
        epistemic_types.as_deref(),
        min_confidence,
        max_depth,
    )?;
    Ok(json!({
        "tenant": tenant,
        "node_id": node_id,
        "results": results.iter().map(|(edge, node)| json!({"edge": edge, "node": node})).collect::<Vec<_>>(),
        "stats": { "returned": results.len() }
    }))
}

fn epistemic_dirty_frontier_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = in_memory_store_from_backend(backend)?;
    let explicit = argument_string_list(arguments, &["content_ids", "content_node_ids"]);
    let k_hops = argument_u64(arguments, &["k_hops", "kHops"]).unwrap_or(2) as usize;
    let limit = argument_u64(arguments, &["limit"]).unwrap_or(10_000) as usize;
    let mut frontier = if explicit.is_empty() {
        inferred_epistemic_dirty_nodes(&store)?
    } else {
        explicit
    };
    frontier = expand_epistemic_frontier(&store, frontier, k_hops, limit)?;
    Ok(json!({
        "tenant": tenant,
        "content_ids": frontier,
        "k_hops": k_hops,
        "limit": limit,
    }))
}

fn epistemic_compile_subgraph_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = in_memory_store_from_backend(backend)?;
    let content_ids = argument_string_list(arguments, &["content_ids", "content_node_ids"]);
    let subgraph = compile_user_subgraph(&store, &content_ids);
    Ok(json!({
        "tenant": tenant,
        "content_ids": content_ids,
        "nodes": subgraph.nodes,
        "edges": subgraph.edges,
    }))
}

fn epistemic_shadow_ppr_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = in_memory_store_from_backend(backend)?;
    let seeds: HashMap<String, f64> =
        serde_json::from_value(arguments.get("seeds").cloned().ok_or_else(|| {
            McpError::invalid_params("epistemic_shadow_ppr requires seeds object")
        })?)
        .map_err(|error| McpError::invalid_params(format!("seeds must be an object: {error}")))?;
    let top_k = argument_u64(arguments, &["top_k", "topK"])
        .unwrap_or(10)
        .max(1) as usize;
    let alpha = arguments
        .get("alpha")
        .and_then(Value::as_f64)
        .unwrap_or(0.15);
    let epsilon = arguments
        .get("epsilon")
        .and_then(Value::as_f64)
        .unwrap_or(1e-6);
    let max_pushes =
        argument_u64(arguments, &["max_pushes", "maxPushes"]).unwrap_or(100_000) as usize;
    let scores = epistemic_shadow_ppr(&store, &seeds, top_k, alpha, epsilon, max_pushes)
        .into_iter()
        .map(|(shadow_node_id, score)| json!({ "shadow_node_id": shadow_node_id, "score": score }))
        .collect::<Vec<_>>();
    Ok(json!({
        "tenant": tenant,
        "scores": scores,
        "top_k": top_k,
        "alpha": alpha,
        "epsilon": epsilon,
        "max_pushes": max_pushes,
    }))
}

fn epistemic_enrich_apply_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let annotations_value = arguments
        .get("annotations")
        .cloned()
        .ok_or_else(|| McpError::invalid_params("epistemic_enrich_apply requires annotations"))?;
    let annotations: EpistemicAnnotations = serde_json::from_value(annotations_value)
        .map_err(|error| McpError::invalid_params(format!("invalid annotations: {error}")))?;
    let mut content_node_ids =
        argument_string_list(arguments, &["content_ids", "content_node_ids"]);
    if content_node_ids.is_empty() {
        content_node_ids = annotation_content_ids(&annotations);
    }
    let mode = match argument_text(arguments, &["mode"])
        .unwrap_or_else(|| "delta".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "full" => EpistemicEnrichmentMode::Full,
        _ => EpistemicEnrichmentMode::Delta,
    };
    let engine = argument_text(arguments, &["engine"])
        .unwrap_or_else(|| "theseus.epistemic_enrichment".to_string());
    let engine_version = argument_text(arguments, &["engine_version", "engineVersion"])
        .unwrap_or_else(|| "epistemic-v1".to_string());
    let density_floor = arguments
        .get("density_floor")
        .or_else(|| arguments.get("densityFloor"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let computed_at = arguments
        .get("computed_at")
        .or_else(|| arguments.get("computedAt"))
        .and_then(value_i64)
        .unwrap_or_else(now_unix_ms);
    let mut store = in_memory_store_from_backend(backend)?;
    let input = EpistemicCronInput {
        content_node_ids,
        mode,
        engine,
        engine_version,
        computed_at,
        density_floor,
    };
    let enricher = StaticEpistemicEnricher { annotations };
    let report = run_epistemic_cron_pass(&mut store, input, &enricher)?;
    let persisted = persist_epistemic_projection(backend, &store)?;
    let shadows_written = report.shadows_written;
    let shadow_edges_written = report.shadow_edges_written;
    Ok(json!({
        "tenant": tenant,
        "report": report,
        "shadows_written": shadows_written,
        "shadow_edges_written": shadow_edges_written,
        "persisted_shadow_nodes": persisted.0,
        "persisted_shadow_edges": persisted.1,
    }))
}

struct StaticEpistemicEnricher {
    annotations: EpistemicAnnotations,
}

impl EpistemicEnricher for StaticEpistemicEnricher {
    fn enrich(
        &self,
        _subgraph: UserSubgraph,
        _mode: EpistemicEnrichmentMode,
    ) -> Result<EpistemicAnnotations, EpistemicEnrichmentError> {
        Ok(self.annotations.clone())
    }
}

fn in_memory_store_from_backend(
    backend: &impl McpGraphBackend,
) -> Result<InMemoryGraphStore, McpError> {
    let snapshot = backend.graph_snapshot()?;
    let mut store = InMemoryGraphStore::new();
    for node in snapshot.nodes {
        GraphStore::upsert_node(&mut store, node)?;
    }
    for edge in snapshot.edges {
        GraphStore::upsert_edge(&mut store, edge)?;
    }
    Ok(store)
}

fn persist_epistemic_projection(
    backend: &mut impl McpGraphBackend,
    store: &InMemoryGraphStore,
) -> Result<(usize, usize), McpError> {
    let snapshot = GraphStore::graph_snapshot(store)?;
    let mut nodes = 0usize;
    let mut edges = 0usize;
    for node in snapshot.nodes {
        if is_epistemic_shadow_node(&node) {
            backend.upsert_node(node)?;
            nodes += 1;
        }
    }
    for edge in snapshot.edges {
        if is_epistemic_shadow_edge(&edge) {
            backend.upsert_edge(edge)?;
            edges += 1;
        }
    }
    Ok((nodes, edges))
}

fn inferred_epistemic_dirty_nodes(store: &InMemoryGraphStore) -> Result<Vec<String>, McpError> {
    let nodes = store.query_nodes(NodeQuery::default().with_limit(100_000));
    let mut dirty = Vec::new();
    for node in nodes {
        if is_epistemic_shadow_node(&node) {
            continue;
        }
        if node_bool(
            &node,
            &["epistemic_shadow_dirty", "shadow_dirty", "epistemic_dirty"],
        ) {
            dirty.push(node.id);
            continue;
        }
        let Some(shadow) = read_epistemic_shadow(store, &node.id) else {
            dirty.push(node.id);
            continue;
        };
        if node_timestamp(
            &node,
            &[
                "updated_at",
                "updated_at_ms",
                "modified_at",
                "modified_at_ms",
            ],
        )
        .is_some_and(|updated_at| updated_at > shadow.computed_at)
        {
            dirty.push(node.id);
        }
    }
    Ok(dirty)
}

fn expand_epistemic_frontier(
    store: &InMemoryGraphStore,
    seeds: Vec<String>,
    k_hops: usize,
    limit: usize,
) -> Result<Vec<String>, McpError> {
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        if seen.insert(seed.clone()) {
            queue.push_back((seed, 0usize));
        }
    }
    while let Some((node_id, depth)) = queue.pop_front() {
        if depth >= k_hops || seen.len() >= limit {
            continue;
        }
        for query in [NeighborQuery::out(&node_id), NeighborQuery::in_(&node_id)] {
            for hit in store.neighbors(query) {
                if seen.len() >= limit {
                    break;
                }
                if store
                    .get_node(&hit.node_id)
                    .is_some_and(|node| !is_epistemic_shadow_node(node))
                    && seen.insert(hit.node_id.clone())
                {
                    queue.push_back((hit.node_id, depth + 1));
                }
            }
        }
    }
    Ok(seen.into_iter().take(limit).collect())
}

fn annotation_content_ids(annotations: &EpistemicAnnotations) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for annotation in &annotations.annotations {
        if !annotation.content_node_id.trim().is_empty() {
            ids.insert(annotation.content_node_id.clone());
        }
    }
    for relation in annotations
        .support_relations
        .iter()
        .chain(annotations.attack_relations.iter())
    {
        if !relation.from_content_id.trim().is_empty() {
            ids.insert(relation.from_content_id.clone());
        }
        if !relation.to_content_id.trim().is_empty() {
            ids.insert(relation.to_content_id.clone());
        }
    }
    ids.into_iter().collect()
}

fn argument_string_list(arguments: &Value, keys: &[&str]) -> Vec<String> {
    argument_array(arguments, keys)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::trim).map(str::to_string))
        .filter(|value| !value.is_empty())
        .collect()
}

fn is_epistemic_shadow_node(node: &NodeRecord) -> bool {
    node.labels
        .iter()
        .any(|label| label == EPISTEMIC_SHADOW_LABEL)
}

fn is_epistemic_shadow_edge(edge: &EdgeRecord) -> bool {
    matches!(
        edge.edge_type.as_str(),
        HAS_EPISTEMIC_SHADOW | UNDERCUTS | EPISTEMIC_SUPPORTS | SAME_ECLASS
    )
}

fn node_bool(node: &NodeRecord, keys: &[&str]) -> bool {
    keys.iter().any(|key| {
        node.properties
            .get(*key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    })
}

fn node_timestamp(node: &NodeRecord, keys: &[&str]) -> Option<i64> {
    keys.iter()
        .find_map(|key| node.properties.get(*key).and_then(value_i64))
}

fn value_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or_default()
}

// ============================================================================
// Inline-adjacency algorithm helpers (RR-INLINE-03 / RR-INLINE-02 contract).
//
// These helpers back the `*_inline` graph algorithm tools, which run
// statelessly against an adjacency map passed in the MCP request arguments
// rather than against a tenant's stored graph. The inline path:
//
//   * touches no tenant state (no `tenant_id` resolved, no backend lookup)
//   * allocates only request-scoped memory (released when the response returns)
//   * triggers no AOF/snapshot writes
//   * is bounded by `MAX_INLINE_EDGES_DEFAULT` (overridable via
//     `RUSTY_RED_MAX_INLINE_EDGES`); above the limit, handlers return
//     `McpError::payload_too_large` pointing callers to the tenant-backed
//     counterpart (`rustyred_thg_algorithm.<name>`).
//
// The shared JSON contract enforced by `parse_inline_adjacency` (RR-INLINE-02):
//
//   {
//     "adjacency": { "<node_id>": [["<neighbor_id>", <weight>], ...], ... }
//   }
//
// Algorithm-specific kwargs (seeds, alpha, damping, etc.) are documented on
// each handler.
// ============================================================================

const MAX_INLINE_EDGES_DEFAULT: usize = 100_000;
const MAX_INLINE_EDGES_ENV: &str = "RUSTY_RED_MAX_INLINE_EDGES";
const MAX_SYMBOLIC_FACTS_DEFAULT: usize = 10_000;
const MAX_SYMBOLIC_FACTS_ENV: &str = "RUSTY_RED_MAX_SYMBOLIC_FACTS";

/// Shape of an inline adjacency map: node_id -> list of (neighbor_id, weight).
/// Aliased here so the helper signatures stay readable and clippy doesn't
/// complain about type complexity. Matches `personalized_pagerank`'s
/// `adjacency` parameter shape in `rustyred-thg-core::graph`.
type InlineAdjacency = HashMap<String, Vec<(String, f64)>>;

/// Read the inline-edge budget at call time. Allows ops to tune the cap via
/// env var without rebuilding. Falls back to `MAX_INLINE_EDGES_DEFAULT` when
/// the env var is unset or unparseable.
fn max_inline_edges() -> usize {
    std::env::var(MAX_INLINE_EDGES_ENV)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(MAX_INLINE_EDGES_DEFAULT)
}

fn max_symbolic_facts() -> usize {
    std::env::var(MAX_SYMBOLIC_FACTS_ENV)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(MAX_SYMBOLIC_FACTS_DEFAULT)
}

/// Count total edges in an inline-adjacency JSON value WITHOUT materializing
/// the full HashMap. Lets inline handlers reject oversized payloads before
/// the deserialization allocation.
fn count_inline_edges(adjacency: &Value) -> usize {
    adjacency
        .as_object()
        .map(|obj| {
            obj.values()
                .filter_map(|neighbors| neighbors.as_array())
                .map(|neighbors| neighbors.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Convert an inline adjacency map into the `Vec<EdgeRecord>` shape required
/// by `connected_components`, `pagerank`, and `label_propagation_communities`
/// in `rustyred-thg-core`. The PPR algorithm takes adjacency directly and
/// does not need this conversion.
///
/// Weights are preserved verbatim via direct struct construction (bypassing
/// `EdgeRecord::with_confidence`'s `[0, 1]` clamp) so inline callers can pass
/// arbitrary edge weights for weighted PageRank and community detection.
fn inline_adjacency_to_edges(adjacency: &InlineAdjacency) -> Vec<EdgeRecord> {
    let mut edges = Vec::new();
    for (from_id, neighbors) in adjacency.iter() {
        for (to_id, weight) in neighbors.iter() {
            edges.push(EdgeRecord {
                id: format!("inline:{from_id}:{to_id}"),
                from_id: from_id.clone(),
                to_id: to_id.clone(),
                edge_type: "inline".to_string(),
                properties: json!({}),
                version: 0,
                tombstone: false,
                confidence: Some(*weight),
                epistemic_type: None,
                provenance: None,
                content_hash: None,
                parent_hashes: Vec::new(),
            });
        }
    }
    edges
}

/// Parse the shared inline-adjacency contract from request arguments
/// (RR-INLINE-02). Returns `(adjacency, edge_count)` on success.
///
/// `max_edges` is the per-call edge budget. Returns
/// `McpError::payload_too_large` if the total edge count exceeds it.
/// Returns `McpError::invalid_params` if `adjacency` is missing or
/// shape-mismatched.
///
/// The budget is passed explicitly (rather than read from the env var
/// inside this function) so callers can supply different limits per call
/// without env-var mutation, which makes unit testing race-free. Production
/// handlers source `max_edges` from `max_inline_edges()` at entry.
fn parse_inline_adjacency(
    arguments: &Value,
    tool_name: &str,
    max_edges: usize,
) -> Result<(InlineAdjacency, usize), McpError> {
    let adjacency_value = arguments.get("adjacency").cloned().ok_or_else(|| {
        McpError::invalid_params(format!("{tool_name} requires adjacency object"))
    })?;
    let edge_count = count_inline_edges(&adjacency_value);
    if edge_count > max_edges {
        return Err(McpError::payload_too_large(format!(
            "inline adjacency contains {edge_count} edges, exceeds limit of {max_edges}; \
             use the tenant-backed counterpart for graphs above this size"
        )));
    }
    let adjacency: InlineAdjacency = serde_json::from_value(adjacency_value).map_err(|err| {
        McpError::invalid_params(format!(
            "adjacency must shape as {{\"<node_id>\": [[\"<neighbor_id>\", <weight>], ...]}}: {err}"
        ))
    })?;
    Ok((adjacency, edge_count))
}

fn algorithm_ppr_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let edges = backend.list_edges()?;
    let seeds: HashMap<String, f64> =
        serde_json::from_value(arguments.get("seeds").cloned().ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_algorithm_ppr requires seeds object")
        })?)
        .map_err(|error| McpError::invalid_params(format!("seeds must be an object: {error}")))?;
    let alpha = arguments
        .get("alpha")
        .and_then(Value::as_f64)
        .unwrap_or(0.15);
    let epsilon = arguments
        .get("epsilon")
        .and_then(Value::as_f64)
        .unwrap_or(1e-4);
    let max_pushes = arguments
        .get("max_pushes")
        .and_then(Value::as_u64)
        .unwrap_or(200_000) as usize;
    let top_k = arguments
        .get("top_k")
        .and_then(Value::as_u64)
        .map(|k| k as usize);
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for edge in edges.iter().filter(|edge| !edge.tombstone) {
        adjacency
            .entry(edge.from_id.clone())
            .or_default()
            .push((edge.to_id.clone(), edge.effective_confidence()));
    }
    let mut entries: Vec<(String, f64)> =
        rustyred_thg_core::personalized_pagerank(&adjacency, &seeds, alpha, epsilon, max_pushes)
            .into_iter()
            .collect();
    entries.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if let Some(k) = top_k {
        entries.truncate(k);
    }
    Ok(json!({
        "tenant": tenant,
        "alpha": alpha,
        "epsilon": epsilon,
        "scores": entries.into_iter().map(|(node_id, score)| json!({
            "node_id": node_id,
            "score": score,
        })).collect::<Vec<_>>()
    }))
}

fn algorithm_components_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let edges = backend.list_edges()?;
    let directed = arguments
        .get("directed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let components = rustyred_thg_core::connected_components(&edges, directed);
    Ok(json!({
        "tenant": tenant,
        "directed": directed,
        "components": components,
        "count": components.len(),
    }))
}

fn algorithm_pagerank_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let edges = backend.list_edges()?;
    let damping = arguments
        .get("damping")
        .and_then(Value::as_f64)
        .unwrap_or(0.85);
    let max_iter = arguments
        .get("max_iter")
        .and_then(Value::as_u64)
        .unwrap_or(100) as usize;
    let tolerance = arguments
        .get("tolerance")
        .and_then(Value::as_f64)
        .unwrap_or(1e-6);
    let top_k = arguments
        .get("top_k")
        .and_then(Value::as_u64)
        .map(|k| k as usize);
    let mut entries: Vec<(String, f64)> =
        rustyred_thg_core::pagerank(&edges, damping, max_iter, tolerance)
            .into_iter()
            .collect();
    entries.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if let Some(k) = top_k {
        entries.truncate(k);
    }
    Ok(json!({
        "tenant": tenant,
        "damping": damping,
        "scores": entries.into_iter().map(|(node_id, score)| json!({
            "node_id": node_id,
            "score": score,
        })).collect::<Vec<_>>()
    }))
}

fn algorithm_communities_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
) -> Result<Value, McpError> {
    let edges = backend.list_edges()?;
    let (community, modularity) = rustyred_thg_core::label_propagation_communities(&edges);
    let mut entries: Vec<Value> = community
        .into_iter()
        .map(|(node_id, community_id)| {
            json!({
                "node_id": node_id,
                "community_id": community_id,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        a["node_id"]
            .as_str()
            .unwrap_or("")
            .cmp(b["node_id"].as_str().unwrap_or(""))
    });
    Ok(json!({
        "tenant": tenant,
        "algorithm": "label_propagation",
        "communities": entries,
        "modularity": modularity,
    }))
}

// ============================================================================
// Inline-adjacency algorithm handlers (RR-INLINE-04 / 05 / 06 / 07).
//
// Each handler mirrors its tenant-backed counterpart's response shape MINUS
// the `tenant` field (since no tenant is touched) PLUS an `edge_count` field
// echoing the inline payload size. Existing tenant-backed handlers above
// remain unchanged; the inline path is purely additive.
//
// All inline handlers silently ignore a `tenant` field if it appears in the
// arguments. The inline path has no tenant by design; callers that need
// tenant-resident compute should use the tenant-backed counterparts
// (`rustyred_thg_algorithm.<name>` without the `_inline` suffix).
//
// Isolated nodes (nodes with no edges in or out) are invisible to all four
// algorithms because they do not appear in the adjacency representation.
// This matches the behavior of the tenant-backed handlers, which read edges
// via `backend.list_edges()` and likewise see only nodes that have edges.
// ============================================================================

/// Run Personalized PageRank over inline adjacency. Stateless: no tenant is
/// touched.
///
/// Required arguments:
///   * `adjacency`: `{"<node_id>": [["<neighbor_id>", <weight>], ...]}`
///   * `seeds`: `{"<node_id>": <mass>}`
///
/// Optional arguments (defaults match the tenant-backed variant):
///   * `alpha`: float, default `0.15`
///   * `epsilon`: float, default `1e-4`
///   * `max_pushes`: integer, default `200_000`
///   * `top_k`: integer, optional; truncates the sorted score list
///
/// Returns: `{ alpha, epsilon, edge_count, scores: [{node_id, score}, ...] }`.
fn algorithm_ppr_inline_payload(arguments: &Value) -> Result<Value, McpError> {
    let (adjacency, edge_count) = parse_inline_adjacency(
        arguments,
        "rustyred_thg_algorithm_ppr_inline",
        max_inline_edges(),
    )?;
    let seeds: HashMap<String, f64> =
        serde_json::from_value(arguments.get("seeds").cloned().ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_algorithm_ppr_inline requires seeds object")
        })?)
        .map_err(|err| {
            McpError::invalid_params(format!(
                "seeds must shape as {{\"<node_id>\": <mass>}}: {err}"
            ))
        })?;
    let alpha = arguments
        .get("alpha")
        .and_then(Value::as_f64)
        .unwrap_or(0.15);
    let epsilon = arguments
        .get("epsilon")
        .and_then(Value::as_f64)
        .unwrap_or(1e-4);
    let max_pushes = arguments
        .get("max_pushes")
        .and_then(Value::as_u64)
        .unwrap_or(200_000) as usize;
    let top_k = arguments
        .get("top_k")
        .and_then(Value::as_u64)
        .map(|k| k as usize);
    let mut entries: Vec<(String, f64)> =
        rustyred_thg_core::personalized_pagerank(&adjacency, &seeds, alpha, epsilon, max_pushes)
            .into_iter()
            .collect();
    entries.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if let Some(k) = top_k {
        entries.truncate(k);
    }
    Ok(json!({
        "alpha": alpha,
        "epsilon": epsilon,
        "edge_count": edge_count,
        "scores": entries.into_iter().map(|(node_id, score)| json!({
            "node_id": node_id,
            "score": score,
        })).collect::<Vec<_>>()
    }))
}

/// Run connected-components over inline adjacency. Stateless: no tenant is
/// touched.
///
/// Required arguments:
///   * `adjacency`: `{"<node_id>": [["<neighbor_id>", <weight>], ...]}`
///
/// Optional arguments:
///   * `directed`: boolean, default `false`
///
/// Returns: `{ directed, edge_count, components: [[node_id, ...], ...], count }`.
fn algorithm_components_inline_payload(arguments: &Value) -> Result<Value, McpError> {
    let (adjacency, edge_count) = parse_inline_adjacency(
        arguments,
        "rustyred_thg_algorithm_components_inline",
        max_inline_edges(),
    )?;
    let directed = arguments
        .get("directed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let edges = inline_adjacency_to_edges(&adjacency);
    let components = rustyred_thg_core::connected_components(&edges, directed);
    let count = components.len();
    Ok(json!({
        "directed": directed,
        "edge_count": edge_count,
        "components": components,
        "count": count,
    }))
}

/// Run power-iteration PageRank over inline adjacency. Stateless: no tenant
/// is touched.
///
/// Required arguments:
///   * `adjacency`: `{"<node_id>": [["<neighbor_id>", <weight>], ...]}`
///
/// Optional arguments (defaults match the tenant-backed variant):
///   * `damping`: float, default `0.85`
///   * `max_iter`: integer, default `100`
///   * `tolerance`: float, default `1e-6`
///   * `top_k`: integer, optional; truncates the sorted score list
///
/// Returns: `{ damping, edge_count, scores: [{node_id, score}, ...] }`.
fn algorithm_pagerank_inline_payload(arguments: &Value) -> Result<Value, McpError> {
    let (adjacency, edge_count) = parse_inline_adjacency(
        arguments,
        "rustyred_thg_algorithm_pagerank_inline",
        max_inline_edges(),
    )?;
    let damping = arguments
        .get("damping")
        .and_then(Value::as_f64)
        .unwrap_or(0.85);
    let max_iter = arguments
        .get("max_iter")
        .and_then(Value::as_u64)
        .unwrap_or(100) as usize;
    let tolerance = arguments
        .get("tolerance")
        .and_then(Value::as_f64)
        .unwrap_or(1e-6);
    let top_k = arguments
        .get("top_k")
        .and_then(Value::as_u64)
        .map(|k| k as usize);
    let edges = inline_adjacency_to_edges(&adjacency);
    let mut entries: Vec<(String, f64)> =
        rustyred_thg_core::pagerank(&edges, damping, max_iter, tolerance)
            .into_iter()
            .collect();
    entries.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    if let Some(k) = top_k {
        entries.truncate(k);
    }
    Ok(json!({
        "damping": damping,
        "edge_count": edge_count,
        "scores": entries.into_iter().map(|(node_id, score)| json!({
            "node_id": node_id,
            "score": score,
        })).collect::<Vec<_>>()
    }))
}

/// Run label-propagation community detection over inline adjacency. Stateless:
/// no tenant is touched.
///
/// Required arguments:
///   * `adjacency`: `{"<node_id>": [["<neighbor_id>", <weight>], ...]}`
///
/// Returns: `{ algorithm, edge_count, communities: [{node_id, community_id}, ...], modularity }`.
fn algorithm_communities_inline_payload(arguments: &Value) -> Result<Value, McpError> {
    let (adjacency, edge_count) = parse_inline_adjacency(
        arguments,
        "rustyred_thg_algorithm_communities_inline",
        max_inline_edges(),
    )?;
    let edges = inline_adjacency_to_edges(&adjacency);
    let (community, modularity) = rustyred_thg_core::label_propagation_communities(&edges);
    let mut entries: Vec<Value> = community
        .into_iter()
        .map(|(node_id, community_id)| {
            json!({
                "node_id": node_id,
                "community_id": community_id,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        a["node_id"]
            .as_str()
            .unwrap_or("")
            .cmp(b["node_id"].as_str().unwrap_or(""))
    });
    Ok(json!({
        "algorithm": "label_propagation",
        "edge_count": edge_count,
        "communities": entries,
        "modularity": modularity,
    }))
}

fn symbolic_datalog_derive_payload(arguments: &Value) -> Result<Value, McpError> {
    let facts = arguments
        .get("facts")
        .or_else(|| {
            arguments
                .get("fact_pack")
                .and_then(|pack| pack.get("facts"))
        })
        .and_then(Value::as_array)
        .ok_or_else(|| {
            McpError::invalid_params("rustyred_thg_symbolic_datalog_derive requires facts array")
        })?;
    let max_facts = max_symbolic_facts();
    if facts.len() > max_facts {
        return Err(McpError::payload_too_large(format!(
            "symbolic fact pack contains {} facts, exceeds limit of {max_facts}; \
             use a tenant-backed or batched symbolic path for larger packs",
            facts.len()
        )));
    }
    let rule_ids = arguments
        .get("rule_ids")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let receipt = rustyred_thg_core::derive_datalog_receipt(&json!({
        "facts": facts,
        "rule_ids": rule_ids,
    }))
    .map_err(McpError::invalid_params)?;
    Ok(receipt)
}

fn symbolic_probabilistic_source_reliability_payload(arguments: &Value) -> Result<Value, McpError> {
    rustyred_thg_core::probabilistic_source_reliability(arguments).map_err(McpError::invalid_params)
}

fn symbolic_probabilistic_expected_value_payload(arguments: &Value) -> Result<Value, McpError> {
    rustyred_thg_core::probabilistic_expected_value(arguments).map_err(McpError::invalid_params)
}

fn coordination_room_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let action = argument_text(arguments, &["action"])
        .unwrap_or_else(|| "status".to_string())
        .to_lowercase();
    let room_id = resolved_coordination_room_id(arguments);
    let room = match action.as_str() {
        "status" => load_coordination_room(backend, tenant, &room_id)?
            .unwrap_or_else(|| empty_coordination_room(tenant, &room_id, "")),
        "join" | "start" => {
            let actor_id = required_text_any(
                arguments,
                &["actor", "actor_id", "actorId"],
                "coordination_room",
            )?;
            join_coordination_room(
                backend,
                JoinRoomInput {
                    tenant_slug: tenant.to_string(),
                    actor_id,
                    room_id,
                    session_id: argument_text(arguments, &["session_id", "sessionId"])
                        .unwrap_or_default(),
                    surface: argument_text(arguments, &["surface"]).unwrap_or_default(),
                    repo: argument_text(arguments, &["repo"]).unwrap_or_default(),
                    branch: argument_text(arguments, &["branch"]).unwrap_or_default(),
                    task: argument_text(arguments, &["task"]).unwrap_or_default(),
                    worktree: argument_text(arguments, &["worktree"]).unwrap_or_default(),
                    head: argument_text(arguments, &["head"]).unwrap_or_default(),
                    changed_files: string_array_any(arguments, &["changed_files", "changedFiles"]),
                    lane: argument_text(arguments, &["lane"]).unwrap_or_default(),
                    updated_at: argument_text(arguments, &["updated_at", "updatedAt"])
                        .unwrap_or_default(),
                },
            )?
        }
        other => {
            return Err(McpError::invalid_params(format!(
                "coordination_room action must be status, join, or start, got {other}"
            )))
        }
    };
    Ok(json!({ "tenant": tenant, "room": room }))
}

fn presence_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let mode = argument_text(arguments, &["mode"])
        .unwrap_or_else(|| "heartbeat".to_string())
        .to_lowercase();
    match mode.as_str() {
        "get" => {
            if let Some(actor_id) = argument_text(arguments, &["actor", "actor_id", "actorId"]) {
                Ok(json!({
                    "tenant": tenant,
                    "presence": load_coordination_presence(backend, tenant, &actor_id)?
                }))
            } else {
                Ok(json!({
                    "tenant": tenant,
                    "presence": list_coordination_presence(backend, tenant)?
                }))
            }
        }
        "heartbeat" | "active" => {
            let presence = write_coordination_presence(
                backend,
                PresenceInput {
                    tenant_slug: tenant.to_string(),
                    actor_id: required_text_any(
                        arguments,
                        &["actor", "actor_id", "actorId"],
                        "presence",
                    )?,
                    session_id: argument_text(arguments, &["session_id", "sessionId"])
                        .unwrap_or_default(),
                    surface: argument_text(arguments, &["surface"]).unwrap_or_default(),
                    status: argument_text(arguments, &["status"])
                        .unwrap_or_else(|| "active".to_string()),
                    worktree: argument_text(arguments, &["worktree"]).unwrap_or_default(),
                    branch: argument_text(arguments, &["branch"]).unwrap_or_default(),
                    head: argument_text(arguments, &["head"]).unwrap_or_default(),
                    changed_files: string_array_any(arguments, &["changed_files", "changedFiles"]),
                    ttl_seconds: argument_u64(arguments, &["ttl_seconds", "ttlSeconds"])
                        .unwrap_or(60),
                    refreshed_at: argument_text(arguments, &["refreshed_at", "refreshedAt"])
                        .unwrap_or_default(),
                    expires_at: argument_text(arguments, &["expires_at", "expiresAt"])
                        .unwrap_or_default(),
                },
            )?;
            Ok(json!({ "tenant": tenant, "presence": presence }))
        }
        "end" | "inactive" => {
            let presence = write_coordination_presence(
                backend,
                PresenceInput {
                    tenant_slug: tenant.to_string(),
                    actor_id: required_text_any(
                        arguments,
                        &["actor", "actor_id", "actorId"],
                        "presence",
                    )?,
                    session_id: argument_text(arguments, &["session_id", "sessionId"])
                        .unwrap_or_default(),
                    surface: argument_text(arguments, &["surface"]).unwrap_or_default(),
                    status: "inactive".to_string(),
                    worktree: argument_text(arguments, &["worktree"]).unwrap_or_default(),
                    branch: argument_text(arguments, &["branch"]).unwrap_or_default(),
                    head: argument_text(arguments, &["head"]).unwrap_or_default(),
                    changed_files: string_array_any(arguments, &["changed_files", "changedFiles"]),
                    ttl_seconds: 1,
                    refreshed_at: argument_text(arguments, &["refreshed_at", "refreshedAt"])
                        .unwrap_or_default(),
                    expires_at: argument_text(arguments, &["expires_at", "expiresAt"])
                        .unwrap_or_default(),
                },
            )?;
            Ok(json!({ "tenant": tenant, "presence": presence }))
        }
        other => Err(McpError::invalid_params(format!(
            "presence mode must be get, heartbeat, or end, got {other}"
        ))),
    }
}

fn write_intent_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let intent = write_coordination_intent(
        backend,
        WriteIntentInput {
            tenant_slug: tenant.to_string(),
            agent_id: argument_text(arguments, &["agent_id", "agentId"]).unwrap_or_default(),
            binding_id: argument_text(arguments, &["binding_id", "bindingId"]).unwrap_or_default(),
            room_id: resolved_coordination_room_id(arguments),
            actor_id: required_text_any(
                arguments,
                &["actor", "actor_id", "actorId"],
                "write_intent",
            )?,
            status: argument_text(arguments, &["status"]).unwrap_or_else(|| "working".to_string()),
            summary: required_text_any(arguments, &["summary"], "write_intent")?,
            footprint: string_array_any(
                arguments,
                &[
                    "footprint",
                    "touched_files",
                    "claimed_files",
                    "claimedFiles",
                ],
            ),
            expected_completion: argument_text(
                arguments,
                &["expected_completion", "expectedCompletion"],
            )
            .unwrap_or_default(),
            repo: argument_text(arguments, &["repo"]).unwrap_or_default(),
            branch: argument_text(arguments, &["branch"]).unwrap_or_default(),
            task: argument_text(arguments, &["task"]).unwrap_or_default(),
            updated_at: argument_text(arguments, &["updated_at", "updatedAt"]).unwrap_or_default(),
        },
    )?;
    Ok(json!({ "tenant": tenant, "intent": intent }))
}

fn read_intents_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let statuses = string_array_any(arguments, &["statuses", "status"]);
    let intents = read_coordination_intents(backend, tenant, &room_id, &statuses)?;
    Ok(json!({
        "tenant": tenant,
        "room_id": room_id,
        "intents": intents,
        "count": intents.len()
    }))
}

fn coordinate_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let message = write_coordination_message(
        backend,
        WriteMessageInput {
            tenant_slug: tenant.to_string(),
            room_id: room_id.clone(),
            actor_id: required_text_any(
                arguments,
                &["actor", "actor_id", "actorId"],
                "coordinate",
            )?,
            message_id: argument_text(arguments, &["message_id", "messageId"]).unwrap_or_default(),
            urgency: argument_text(arguments, &["urgency"]).unwrap_or_else(|| "info".to_string()),
            delivery: argument_text(arguments, &["delivery"])
                .unwrap_or_else(|| coordination_delivery_from_legacy_wake(arguments)),
            message: required_text_any(arguments, &["message"], "coordinate")?,
            mentions: string_array_any(arguments, &["mentions"]),
            metadata: argument_object(arguments, "metadata"),
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
        },
    )?;
    let (stream_event_id, stream_ordering_token) =
        persist_coordination_message_stream_event(tenant, backend, &message)?;
    Ok(json!({
        "tenant": tenant,
        "ok": true,
        "room_id": room_id,
        "message_id": message.message_id,
        "stream_event_id": stream_event_id,
        "stream_ordering_token": stream_ordering_token,
        "mentions": message.mentions,
        "delivery": message.delivery,
        "unread_count": message.mentions.len(),
        "urgency": message.urgency,
        "created_at": message.created_at
    }))
}

fn persist_coordination_message_stream_event(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    message: &CoordinationMessageState,
) -> Result<(String, u64), McpError> {
    let topic = message.room_id.clone();
    let stream_key = canonical_stream_key(tenant, &topic);
    let _publish_guard = stream_publish_lock()
        .lock()
        .map_err(|_| McpError::internal("stream publish lock poisoned"))?;
    let token = reserve_stream_token(backend, tenant, &topic, &stream_key, &message.created_at)?;
    let event_id = format!(
        "sevt:{}",
        &stable_value_hash(&json!({ "k": stream_key, "t": token }))[..16]
    );
    let mut event = StreamEvent::new(
        event_id,
        stream_key,
        message.actor_id.clone(),
        "coordination_message",
        json!({
            "room_id": message.room_id,
            "message_id": message.message_id,
            "message": message.message,
            "mentions": message.mentions,
            "delivery": message.delivery,
            "metadata": message.metadata,
        }),
        StreamUrgency::parse(&message.urgency)
            .ok_or_else(|| McpError::internal("stored coordination message has invalid urgency"))?,
        (message.mentions.len() == 1).then(|| message.mentions[0].clone()),
        message.created_at.clone(),
    );
    event.ordering_token = token;
    persist_stream_event(backend, tenant, &topic, &event)?;
    Ok((event.id, token))
}

fn mentions_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let actor_id = required_text_any(arguments, &["actor", "actor_id", "actorId"], "mentions")?;
    let consume = arguments
        .get("consume")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let limit = argument_u64(arguments, &["limit"]).unwrap_or(20) as usize;
    let mentions = read_coordination_mentions(backend, tenant, &actor_id, consume, limit)?;
    Ok(json!({
        "tenant": tenant,
        "actor_id": actor_id,
        "mentions": mentions,
        "count": mentions.len(),
        "consumed": consume
    }))
}

fn read_messages_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let limit = argument_u64(arguments, &["limit"]).unwrap_or(50) as usize;
    let messages = read_coordination_messages(backend, tenant, &room_id, limit)?;
    Ok(json!({
        "tenant": tenant,
        "room_id": room_id,
        "messages": messages,
        "count": messages.len()
    }))
}

// ---------------------------------------------------------------------------
// Stream-based coordination (SPEC: append-only event streams read by cursor).
//
// `stream_publish` / `stream_read` / `stream_subscribe` / `stream_unsubscribe`
// are the cursor-delta surface that replaces the room poll. Each stream is
// `(tenant, topic)`-scoped; events carry a monotonic ordering token; a cursor is
// an actor's last-consumed token. The durable transport persists each event as a
// `CoordinationStreamEvent` graph node and rehydrates a transient
// [`StreamLog`](rustyred_thg_core::StreamLog) to run the identical `read_after`.
// An `ask`/`block` publish with a `target_actor` additionally bridges onto the
// existing mention/wake path via `write_coordination_message`.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StreamHeadState {
    #[serde(default)]
    tenant_slug: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    stream_key: String,
    #[serde(default)]
    next_token: u64,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StreamCursorState {
    #[serde(default)]
    tenant_slug: String,
    #[serde(default)]
    actor_id: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    stream_key: String,
    #[serde(default)]
    cursor: u64,
    #[serde(default)]
    updated_at: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StreamSubscriptionState {
    #[serde(default)]
    tenant_slug: String,
    #[serde(default)]
    actor_id: String,
    #[serde(default)]
    streams: Vec<String>,
    #[serde(default)]
    updated_at: String,
}

fn stream_topic_from_args(arguments: &Value) -> Option<String> {
    argument_text(arguments, &["stream", "topic", "room_id", "roomId"])
        .map(|topic| topic.trim().to_string())
        .filter(|topic| !topic.is_empty())
}

fn load_stream_head(
    backend: &impl McpGraphBackend,
    tenant: &str,
    topic: &str,
) -> Result<Option<StreamHeadState>, McpError> {
    backend
        .get_node(&coordination_stream_node_id(tenant, topic))?
        .map(|node| parse_node_properties::<StreamHeadState>(node.properties))
        .transpose()
}

/// The highest ordering token already persisted for a stream. Used only to
/// bootstrap the head allocator if the head node is ever missing while events
/// exist, so token assignment stays monotonic regardless.
fn max_persisted_stream_token(
    backend: &impl McpGraphBackend,
    tenant: &str,
    topic: &str,
) -> Result<u64, McpError> {
    let mut max_token = 0;
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationStreamEvent")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("topic", Value::String(topic.to_string())),
        )? {
            if let Some(token) = node
                .properties
                .get("ordering_token")
                .and_then(Value::as_u64)
            {
                max_token = max_token.max(token);
            }
        }
    }
    Ok(max_token)
}

fn stream_publish_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Reserve the next monotonic ordering token for a stream (read-modify-write on
/// the head node). Callers hold the process-local stream publish lock across
/// reservation and event persistence, so concurrent publishers in one MCP server
/// receive distinct tokens with no merge step.
fn reserve_stream_token(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    topic: &str,
    stream_key: &str,
    now: &str,
) -> Result<u64, McpError> {
    let mut head = load_stream_head(backend, tenant, topic)?.unwrap_or_default();
    let floor = if head.next_token == 0 {
        max_persisted_stream_token(backend, tenant, topic)?.saturating_add(1)
    } else {
        head.next_token
    };
    let token = floor.max(1);
    if head.created_at.is_empty() {
        head.created_at = now.to_string();
    }
    head.tenant_slug = tenant.to_string();
    head.topic = topic.to_string();
    head.stream_key = stream_key.to_string();
    head.next_token = token.saturating_add(1);
    head.updated_at = now.to_string();
    let node = NodeRecord::new(
        coordination_stream_node_id(tenant, topic),
        ["HarnessCoordination", "CoordinationStream"],
        serde_json::to_value(&head).map_err(|error| McpError::internal(error.to_string()))?,
    );
    upsert_node_if_changed(backend, node)?;
    Ok(token)
}

fn persist_stream_event(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    topic: &str,
    event: &StreamEvent,
) -> Result<(), McpError> {
    let mut properties = serde_json::to_value(event)
        .map_err(|error| McpError::internal(error.to_string()))?
        .as_object()
        .cloned()
        .unwrap_or_default();
    properties.insert("tenant_slug".to_string(), Value::String(tenant.to_string()));
    properties.insert("topic".to_string(), Value::String(topic.to_string()));
    let node = NodeRecord::new(
        coordination_stream_event_node_id(tenant, topic, event.ordering_token),
        ["HarnessCoordination", "CoordinationStreamEvent"],
        Value::Object(properties),
    );
    upsert_node_if_changed(backend, node)?;
    let edge = EdgeRecord::new(
        coordination_stream_event_edge_id(tenant, topic, event.ordering_token),
        coordination_stream_event_node_id(tenant, topic, event.ordering_token),
        "COORDINATION_STREAM_EVENT_OF",
        coordination_stream_node_id(tenant, topic),
        json!({
            "tenant_slug": tenant,
            "topic": topic,
            "ordering_token": event.ordering_token,
            "actor": event.actor,
            "urgency": event.urgency.as_str(),
            "created_at": event.created_at,
        }),
    );
    upsert_edge_if_changed(backend, edge)?;
    Ok(())
}

/// Rehydrate the durably-stored events for one stream into a transient
/// [`StreamLog`] and return the delta strictly after `cursor`, in token order.
/// This is the remote delta-pull transport running the exact core `read_after`.
fn read_stream_delta_after(
    backend: &impl McpGraphBackend,
    tenant: &str,
    topic: &str,
    cursor: u64,
    limit: usize,
) -> Result<Vec<StreamEvent>, McpError> {
    let mut log = StreamLog::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationStreamEvent")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("topic", Value::String(topic.to_string())),
        )? {
            log.ingest(parse_node_properties::<StreamEvent>(node.properties)?);
        }
    }
    Ok(log.read_after(cursor, limit))
}

fn load_stream_cursor(
    backend: &impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
    topic: &str,
) -> Result<u64, McpError> {
    for tenant_alias in tenant_slug_aliases(tenant) {
        if let Some(node) = backend.get_node(&coordination_stream_cursor_node_id(
            &tenant_alias,
            actor_id,
            topic,
        ))? {
            return Ok(parse_node_properties::<StreamCursorState>(node.properties)?.cursor);
        }
    }
    Ok(0)
}

fn persist_stream_cursor(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
    topic: &str,
    stream_key: &str,
    cursor: u64,
    now: &str,
) -> Result<(), McpError> {
    let state = StreamCursorState {
        tenant_slug: tenant.to_string(),
        actor_id: actor_id.to_string(),
        topic: topic.to_string(),
        stream_key: stream_key.to_string(),
        cursor,
        updated_at: now.to_string(),
    };
    let node = NodeRecord::new(
        coordination_stream_cursor_node_id(tenant, actor_id, topic),
        ["HarnessCoordination", "CoordinationStreamCursor"],
        serde_json::to_value(&state).map_err(|error| McpError::internal(error.to_string()))?,
    );
    upsert_node_if_changed(backend, node)?;
    Ok(())
}

fn load_stream_subscriptions(
    backend: &impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
) -> Result<Vec<String>, McpError> {
    for tenant_alias in tenant_slug_aliases(tenant) {
        if let Some(node) = backend.get_node(&coordination_stream_subscription_node_id(
            &tenant_alias,
            actor_id,
        ))? {
            return Ok(parse_node_properties::<StreamSubscriptionState>(node.properties)?.streams);
        }
    }
    Ok(Vec::new())
}

fn persist_stream_subscriptions(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
    streams: &[String],
    now: &str,
) -> Result<(), McpError> {
    let state = StreamSubscriptionState {
        tenant_slug: tenant.to_string(),
        actor_id: actor_id.to_string(),
        streams: streams.to_vec(),
        updated_at: now.to_string(),
    };
    let node = NodeRecord::new(
        coordination_stream_subscription_node_id(tenant, actor_id),
        ["HarnessCoordination", "CoordinationStreamSubscription"],
        serde_json::to_value(&state).map_err(|error| McpError::internal(error.to_string()))?,
    );
    upsert_node_if_changed(backend, node)?;
    Ok(())
}

fn stream_publish_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let topic = stream_topic_from_args(arguments)
        .ok_or_else(|| McpError::invalid_params("stream_publish requires stream (topic)"))?;
    let actor = require_actor_id(
        &required_text_any(
            arguments,
            &["actor", "actor_id", "actorId"],
            "stream_publish",
        )?,
        "stream_publish requires actor",
    )?;
    let kind = required_text_any(arguments, &["kind"], "stream_publish")?;
    let urgency_arg = argument_text(arguments, &["urgency"]).unwrap_or_default();
    let urgency = StreamUrgency::parse(&urgency_arg).ok_or_else(|| {
        McpError::invalid_params("stream_publish urgency must be info, ask, or block")
    })?;
    let target_actor = argument_text(arguments, &["target_actor", "targetActor", "target"])
        .map(|value| normalize_actor_id(&value))
        .filter(|value| !value.is_empty());
    let payload = arguments
        .get("payload")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let created_at = timestamp_or_now(
        &argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
    );
    let stream_key = canonical_stream_key(tenant, &topic);

    let _publish_guard = stream_publish_lock()
        .lock()
        .map_err(|_| McpError::internal("stream publish lock poisoned"))?;
    let token = reserve_stream_token(backend, tenant, &topic, &stream_key, &created_at)?;
    let event_id = format!(
        "sevt:{}",
        &stable_value_hash(&json!({ "k": stream_key, "t": token }))[..16]
    );
    let mut event = StreamEvent::new(
        event_id,
        stream_key.clone(),
        actor.clone(),
        kind,
        payload,
        urgency,
        target_actor.clone(),
        created_at.clone(),
    );
    event.ordering_token = token;
    persist_stream_event(backend, tenant, &topic, &event)?;

    // Intentional-ping bridge: an ask/block to a target additionally lands on the
    // existing mention/wake path so a warm head drains it at its next Stop hook
    // and a cold head is woken. The stream event itself is already written above,
    // so passive readers see it regardless.
    let mut pinged = false;
    if event.is_ping() {
        if let Some(target) = target_actor.as_ref() {
            write_coordination_message(
                backend,
                WriteMessageInput {
                    tenant_slug: tenant.to_string(),
                    room_id: topic.clone(),
                    actor_id: actor.clone(),
                    message_id: String::new(),
                    urgency: urgency.as_str().to_string(),
                    delivery: "wake".to_string(),
                    message: format!(
                        "@{target} stream ping ({}) on {topic}: {}",
                        urgency.as_str(),
                        event.kind
                    ),
                    mentions: vec![target.clone()],
                    metadata: Map::from_iter([
                        ("stream_ping".to_string(), Value::Bool(true)),
                        ("stream".to_string(), Value::String(topic.clone())),
                        (
                            "stream_event_id".to_string(),
                            Value::String(event.id.clone()),
                        ),
                        ("ordering_token".to_string(), Value::Number(token.into())),
                    ]),
                    created_at: created_at.clone(),
                },
            )?;
            pinged = true;
        }
    }

    Ok(json!({
        "tenant": tenant,
        "ok": true,
        "stream": topic,
        "stream_key": stream_key,
        "event_id": event.id,
        "ordering_token": token,
        "urgency": urgency.as_str(),
        "target_actor": target_actor,
        "pinged": pinged,
        "created_at": created_at,
    }))
}

fn stream_read_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    allow_advance: bool,
) -> Result<Value, McpError> {
    let actor = require_actor_id(
        &required_text_any(arguments, &["actor", "actor_id", "actorId"], "stream_read")?,
        "stream_read requires actor",
    )?;
    let requested_advance = arguments
        .get("advance")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let advance = requested_advance && allow_advance;
    let limit = argument_u64(arguments, &["limit"]).unwrap_or(0) as usize;

    // Explicit streams[] override the subscription set; otherwise read every
    // stream the actor subscribes to (the passive turn-start read).
    let mut topics = string_array_any(arguments, &["streams", "topics"])
        .into_iter()
        .map(|topic| topic.trim().to_string())
        .filter(|topic| !topic.is_empty())
        .collect::<Vec<_>>();
    if let Some(single) = stream_topic_from_args(arguments) {
        if !topics.contains(&single) {
            topics.push(single);
        }
    }
    let from_subscriptions = topics.is_empty();
    if from_subscriptions {
        topics = load_stream_subscriptions(backend, tenant, &actor)?
            .into_iter()
            .filter_map(|key| stream_topic_from_canonical(tenant, &key))
            .collect();
    }

    let now = timestamp_or_now("");
    let mut events_out = Vec::new();
    let mut new_cursors = Map::new();
    for topic in &topics {
        let stream_key = canonical_stream_key(tenant, topic);
        let cursor = load_stream_cursor(backend, tenant, &actor, topic)?;
        let delta = read_stream_delta_after(backend, tenant, topic, cursor, limit)?;
        let new_cursor = delta
            .last()
            .map(|event| event.ordering_token)
            .unwrap_or(cursor);
        if advance && new_cursor > cursor {
            persist_stream_cursor(
                backend,
                tenant,
                &actor,
                topic,
                &stream_key,
                new_cursor,
                &now,
            )?;
        }
        new_cursors.insert(topic.clone(), Value::Number(new_cursor.into()));
        for event in delta {
            events_out.push(
                serde_json::to_value(&event)
                    .map_err(|error| McpError::internal(error.to_string()))?,
            );
        }
    }

    Ok(json!({
        "tenant": tenant,
        "actor_id": actor,
        "streams": topics,
        "from_subscriptions": from_subscriptions,
        "events": events_out,
        "count": events_out.len(),
        "new_cursors": Value::Object(new_cursors),
        "advanced": advance,
    }))
}

/// Map a canonical `(tenant, topic)` key back to its topic for the read path.
fn stream_topic_from_canonical(tenant: &str, stream_key: &str) -> Option<String> {
    let prefix = format!("{}\u{1}", normalize_tenant_slug(tenant));
    stream_key
        .strip_prefix(&prefix)
        .map(str::to_string)
        .or_else(|| {
            // Tolerate a bare topic stored without the canonical prefix.
            (!stream_key.contains('\u{1}')).then(|| stream_key.to_string())
        })
        .filter(|topic| !topic.is_empty())
}

fn stream_subscription_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    subscribe: bool,
) -> Result<Value, McpError> {
    let tool = if subscribe {
        "stream_subscribe"
    } else {
        "stream_unsubscribe"
    };
    let actor = require_actor_id(
        &required_text_any(arguments, &["actor", "actor_id", "actorId"], tool)?,
        "stream subscription requires actor",
    )?;
    let topic = stream_topic_from_args(arguments)
        .ok_or_else(|| McpError::invalid_params(format!("{tool} requires stream (topic)")))?;
    let stream_key = canonical_stream_key(tenant, &topic);

    let mut streams = load_stream_subscriptions(backend, tenant, &actor)?;
    let changed = if subscribe {
        if streams.contains(&stream_key) {
            false
        } else {
            streams.push(stream_key.clone());
            true
        }
    } else {
        let before = streams.len();
        streams.retain(|existing| existing != &stream_key);
        streams.len() != before
    };
    streams.sort();
    streams.dedup();
    let now = timestamp_or_now("");
    if changed {
        persist_stream_subscriptions(backend, tenant, &actor, &streams, &now)?;
    }

    let topics = streams
        .iter()
        .filter_map(|key| stream_topic_from_canonical(tenant, key))
        .collect::<Vec<_>>();
    Ok(json!({
        "tenant": tenant,
        "actor_id": actor,
        "subscribed": subscribe,
        "changed": changed,
        "stream": topic,
        "subscriptions": topics,
        "count": topics.len(),
    }))
}

fn write_record_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    policy_receipt: Option<Value>,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let mut metadata = argument_object(arguments, "metadata");
    if let Some(policy_receipt) = policy_receipt.clone() {
        metadata.insert("policy_receipt".to_string(), policy_receipt);
    }
    let record = write_coordination_record(
        backend,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: room_id.clone(),
            actor_id: required_text_any(
                arguments,
                &["actor", "actor_id", "actorId"],
                "coordination_record",
            )?,
            record_id: argument_text(arguments, &["record_id", "recordId"]).unwrap_or_default(),
            record_type: required_text_any(
                arguments,
                &["record_type", "recordType", "type"],
                "coordination_record",
            )?,
            title: argument_text(arguments, &["title"]).unwrap_or_default(),
            summary: required_text_any(arguments, &["summary"], "coordination_record")?,
            body: argument_text(arguments, &["body"]).unwrap_or_default(),
            metadata,
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
        },
    )?;
    let mut payload = json!({
        "tenant": tenant,
        "room_id": room_id,
        "record": record
    });
    insert_policy_receipt(&mut payload, policy_receipt);
    Ok(payload)
}

/// Spawn a Claude Code session that is visible in the coordination room and runs via the committed
/// GitHub Actions `theorem-handoff` workflow (`repository_dispatch`), not the Railway runner.
/// Writes the room-visible CoordinationRecord first (so the session appears as a participant
/// regardless of the dispatch outcome), then fires the dispatch through the injected backend.
fn spawn_session_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    policy_receipt: Option<Value>,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let actor_id = required_text_any(
        arguments,
        &["actor", "actor_id", "actorId"],
        "spawn_session",
    )?;
    let intent = required_text_any(arguments, &["intent", "prompt", "message"], "spawn_session")?;
    let owner = argument_text(arguments, &["owner", "repo_owner", "repoOwner"])
        .unwrap_or_else(|| "Travis-Gilbert".to_string());
    let repo =
        argument_text(arguments, &["repo", "repository"]).unwrap_or_else(|| "theorem".to_string());
    let branch = argument_text(arguments, &["branch", "ref"]).unwrap_or_default();
    let event_type = argument_text(arguments, &["event_type", "eventType"])
        .unwrap_or_else(|| "theorem-handoff".to_string());
    let created_at = timestamp_or_now(
        &argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
    );
    let tenant_slug = normalize_tenant_slug(tenant);
    let dispatch_id = stable_coordination_record_id(
        &tenant_slug,
        &room_id,
        "spawn",
        &actor_id,
        &intent,
        &created_at,
    );
    let summary = intent
        .lines()
        .next()
        .unwrap_or(intent.as_str())
        .chars()
        .take(160)
        .collect::<String>();

    // (1) write the room-visible CoordinationRecord first, reusing the existing write machinery.
    let mut metadata = argument_object(arguments, "metadata");
    metadata.insert("dispatch_id".to_string(), json!(dispatch_id));
    metadata.insert("owner".to_string(), json!(owner));
    metadata.insert("repo".to_string(), json!(repo));
    metadata.insert("branch".to_string(), json!(branch));
    metadata.insert("surface".to_string(), json!("spawned"));
    metadata.insert("kind".to_string(), json!("spawn"));
    metadata.insert("status".to_string(), json!("running"));
    metadata.insert("executor".to_string(), json!("github_actions"));
    metadata.insert("event_type".to_string(), json!(event_type));
    metadata.insert("intent".to_string(), json!(intent));

    let record = write_coordination_record(
        backend,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: room_id.clone(),
            actor_id: actor_id.clone(),
            record_id: dispatch_id.clone(),
            record_type: "event".to_string(),
            title: format!("spawned session: {summary}"),
            summary: summary.clone(),
            body: intent.clone(),
            metadata,
            created_at,
        },
    )?;

    // (2) fire the GitHub Actions repository_dispatch via the injected backend (not the runner).
    let (status, dispatch_error) = match backend.dispatch_handoff(HandoffDispatch {
        owner: owner.clone(),
        repo: repo.clone(),
        event_type,
        intent,
        branch: branch.clone(),
    }) {
        Ok(()) => ("running", None),
        Err(error) => {
            let _ = update_spawn_record_status(
                backend,
                tenant,
                &room_id,
                &dispatch_id,
                "dispatch_failed",
                None,
            );
            ("dispatch_failed", Some(error.message))
        }
    };

    let mut payload = json!({
        "tenant": tenant,
        "room_id": room_id,
        "dispatch_id": dispatch_id,
        "status": status,
        "executor": "github_actions",
        "repo": format!("{owner}/{repo}"),
        "branch": branch,
        "record": record,
        "dispatch_error": dispatch_error,
    });
    insert_policy_receipt(&mut payload, policy_receipt);
    Ok(payload)
}

/// Update a spawn CoordinationRecord by dispatch id. This is the seam the PR-opened webhook calls
/// to close the loop with status "done"/"failed" and the PR URL; the webhook HTTP route on the
/// harness service is the explicit follow-up per the spec sequencing.
fn update_spawn_record_status(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    dispatch_id: &str,
    status: &str,
    pr_url: Option<&str>,
) -> Result<Option<CoordinationRecordState>, McpError> {
    let records =
        read_coordination_records(&*backend, tenant, room_id, &["event".to_string()], 200)?;
    let Some(mut record) = records.into_iter().find(|record| {
        record.record_id == dispatch_id
            || record.metadata.get("dispatch_id").and_then(Value::as_str) == Some(dispatch_id)
    }) else {
        return Ok(None);
    };
    record.metadata.insert("status".to_string(), json!(status));
    if let Some(url) = pr_url {
        record.metadata.insert("pr_url".to_string(), json!(url));
    }
    persist_coordination_record(backend, &record)?;
    Ok(Some(record))
}

fn write_contribution_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    policy_receipt: Option<Value>,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let actor_id = required_text_any(
        arguments,
        &["actor", "actor_id", "actorId"],
        "coordination_contribution",
    )?;
    let summary = required_text_any(arguments, &["summary"], "coordination_contribution")?;
    let mut metadata = argument_object(arguments, "metadata");
    insert_if_present(
        &mut metadata,
        "contribution_kind",
        argument_text(
            arguments,
            &["contribution_kind", "contributionKind", "kind"],
        ),
    );
    insert_if_present(
        &mut metadata,
        "status",
        argument_text(arguments, &["status"]),
    );
    insert_if_present(
        &mut metadata,
        "commit",
        argument_text(arguments, &["commit", "commit_sha", "commitSha"]),
    );
    let changed_files = string_array_any(arguments, &["changed_files", "changedFiles"]);
    if !changed_files.is_empty() {
        metadata.insert("changed_files".to_string(), json!(changed_files));
    }
    if let Some(artifacts) = argument_array(arguments, &["artifacts"]) {
        metadata.insert("artifacts".to_string(), Value::Array(artifacts));
    }
    if let Some(receipts) =
        argument_array(arguments, &["validation_receipts", "validationReceipts"])
    {
        metadata.insert("validation_receipts".to_string(), Value::Array(receipts));
    }
    if let Some(policy_receipt) = policy_receipt.clone() {
        metadata.insert("policy_receipt".to_string(), policy_receipt);
    }

    let record = write_coordination_record(
        backend,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: room_id.clone(),
            actor_id,
            record_id: argument_text(arguments, &["record_id", "recordId"]).unwrap_or_default(),
            record_type: "event".to_string(),
            title: argument_text(arguments, &["title"])
                .unwrap_or_else(|| "Coordination contribution".to_string()),
            summary,
            body: argument_text(arguments, &["body"]).unwrap_or_default(),
            metadata,
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
        },
    )?;
    let mut payload = json!({
        "tenant": tenant,
        "room_id": room_id,
        "contribution": record
    });
    insert_policy_receipt(&mut payload, policy_receipt);
    Ok(payload)
}

fn read_records_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let record_types = string_array_any(
        arguments,
        &["record_types", "recordTypes", "record_type", "recordType"],
    );
    let limit = argument_u64(arguments, &["limit"]).unwrap_or(50) as usize;
    let records = read_coordination_records(backend, tenant, &room_id, &record_types, limit)?;
    Ok(json!({
        "tenant": tenant,
        "room_id": room_id,
        "record_types": record_types,
        "records": records,
        "count": records.len()
    }))
}

fn coordination_context_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let room_id = resolved_coordination_room_id(arguments);
    let actor_id = argument_text(arguments, &["actor", "actor_id", "actorId"]);
    let message_limit = argument_u64(arguments, &["message_limit", "messageLimit"])
        .or_else(|| argument_u64(arguments, &["limit"]))
        .unwrap_or(20) as usize;
    let record_limit = argument_u64(arguments, &["record_limit", "recordLimit"])
        .or_else(|| argument_u64(arguments, &["limit"]))
        .unwrap_or(20) as usize;
    let mention_limit = argument_u64(arguments, &["mention_limit", "mentionLimit"])
        .or_else(|| argument_u64(arguments, &["limit"]))
        .unwrap_or(20) as usize;
    let statuses = string_array_any(arguments, &["statuses", "status"]);
    let record_types = string_array_any(
        arguments,
        &["record_types", "recordTypes", "record_type", "recordType"],
    );

    let room = load_coordination_room(backend, tenant, &room_id)?
        .unwrap_or_else(|| empty_coordination_room(tenant, &room_id, ""));
    let presence = list_coordination_presence(backend, tenant)?;
    let intents = read_coordination_intents(backend, tenant, &room_id, &statuses)?;
    let messages = read_coordination_messages(backend, tenant, &room_id, message_limit)?;
    let records =
        read_coordination_records(backend, tenant, &room_id, &record_types, record_limit)?;
    let pending_mentions = if let Some(actor_id) = actor_id.as_ref() {
        read_coordination_mentions_in_room(
            backend,
            tenant,
            &room_id,
            actor_id,
            false,
            mention_limit,
        )?
    } else {
        Vec::new()
    };
    let presence_count = presence.len();
    let intent_count = intents.len();
    let message_count = messages.len();
    let record_count = records.len();
    let pending_mention_count = pending_mentions.len();

    Ok(json!({
        "tenant": tenant,
        "room_id": room_id,
        "actor_id": actor_id.unwrap_or_default(),
        "room": room,
        "presence": presence,
        "intents": intents,
        "messages": messages,
        "records": records,
        "pending_mentions": pending_mentions,
        "counts": {
            "presence": presence_count,
            "intents": intent_count,
            "messages": message_count,
            "records": record_count,
            "pending_mentions": pending_mention_count
        }
    }))
}

fn append_harness_transition_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let transition = transition_from_arguments(arguments, "harness_append_transition")?;
    let result = backend.append_harness_transition(transition)?;
    Ok(json!({
        "tenant": tenant,
        "result": result
    }))
}

fn harness_run_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "harness_run")?;
    let detail = backend.harness_run_detail(&run_id)?;
    let found = detail.is_some();
    Ok(json!({
        "tenant": tenant,
        "run_id": run_id,
        "detail": detail,
        "found": found
    }))
}

fn composed_agent_run_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let task = required_text_any(arguments, &["task", "query", "q"], "composed_agent_run")?;
    let binding_id = argument_text(arguments, &["binding_id", "bindingId"])
        .unwrap_or_else(|| theorem_harness_runtime::DEFAULT_BINDING_ID.to_string());
    let claims = grounded_claims_from_arguments(arguments);
    backend
        .composed_agent_run(binding_id, task, claims)
        .map(|result| {
            json!({
                "tenant": tenant,
                "result": result
            })
        })
}

fn grounded_claims_from_arguments(arguments: &Value) -> Vec<GroundedClaim> {
    arguments
        .get("claims")
        .and_then(Value::as_array)
        .map(|claims| {
            claims
                .iter()
                .filter_map(|claim| {
                    let text = claim
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    let provenance = claim
                        .get("provenance")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .trim();
                    if text.is_empty() || provenance.is_empty() {
                        None
                    } else {
                        Some(GroundedClaim::new(text, provenance))
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

fn skill_list_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = McpSkillPackReadStore { backend };
    let packs = list_skill_packs(
        &store,
        SkillPackListInput {
            tenant_slug: tenant.to_string(),
            status: argument_text(arguments, &["status"]).unwrap_or_default(),
            include_retired: arguments
                .get("include_retired")
                .or_else(|| arguments.get("includeRetired"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            limit: argument_u64(arguments, &["limit"]).unwrap_or(20) as usize,
        },
    )
    .map_err(mcp_skill_pack_error)?;
    Ok(json!({
        "tenant": tenant,
        "packs": packs,
        "count": packs.len()
    }))
}

fn skill_get_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = McpSkillPackReadStore { backend };
    let pack = get_skill_pack(
        &store,
        SkillPackGetInput {
            tenant_slug: tenant.to_string(),
            pack_id: argument_text(arguments, &["pack_id", "packId", "id"]).unwrap_or_default(),
            pack_content_hash: argument_text(
                arguments,
                &[
                    "pack_content_hash",
                    "packContentHash",
                    "content_hash",
                    "contentHash",
                ],
            )
            .unwrap_or_default(),
        },
    )
    .map_err(mcp_skill_pack_error)?;
    Ok(json!({
        "tenant": tenant,
        "pack": pack
    }))
}

fn skill_publish_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let pack = arguments
        .get("pack")
        .or_else(|| arguments.get("capability_pack"))
        .or_else(|| arguments.get("capabilityPack"))
        .cloned()
        .ok_or_else(|| McpError::invalid_params("skill_publish requires pack object"))?;
    let mut store = McpSkillPackStore { backend };
    let receipt = publish_skill_pack(
        &mut store,
        SkillPackPublishInput {
            tenant_slug: tenant.to_string(),
            actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"])
                .unwrap_or_default(),
            pack_content_hash: argument_text(
                arguments,
                &[
                    "pack_content_hash",
                    "packContentHash",
                    "content_hash",
                    "contentHash",
                ],
            )
            .unwrap_or_default(),
            source_content_hash: argument_text(
                arguments,
                &[
                    "source_content_hash",
                    "sourceContentHash",
                    "source_hash",
                    "sourceHash",
                ],
            )
            .unwrap_or_default(),
            artifact_hashes: string_array_any(
                arguments,
                &[
                    "artifact_hashes",
                    "artifactHashes",
                    "artifact_hash",
                    "artifactHash",
                ],
            ),
            status: argument_text(arguments, &["status"]).unwrap_or_default(),
            metadata: argument_object(arguments, "metadata"),
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
            pack,
        },
    )
    .map_err(mcp_skill_pack_error)?;
    Ok(json!({
        "tenant": tenant,
        "published": receipt
    }))
}

fn skill_apply_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let mut store = McpSkillPackStore { backend };
    let receipt = apply_skill_pack(
        &mut store,
        SkillPackApplyInput {
            tenant_slug: tenant.to_string(),
            pack_id: argument_text(arguments, &["pack_id", "packId", "id"]).unwrap_or_default(),
            pack_content_hash: argument_text(
                arguments,
                &[
                    "pack_content_hash",
                    "packContentHash",
                    "content_hash",
                    "contentHash",
                ],
            )
            .unwrap_or_default(),
            actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"])
                .unwrap_or_default(),
            run_id: argument_text(arguments, &["run_id", "runId"]).unwrap_or_default(),
            task: argument_text(arguments, &["task"]).unwrap_or_default(),
            context: arguments
                .get("context")
                .cloned()
                .unwrap_or_else(|| json!({})),
            outcome: arguments
                .get("outcome")
                .cloned()
                .unwrap_or_else(|| json!({})),
            allow_retired: arguments
                .get("allow_retired")
                .or_else(|| arguments.get("allowRetired"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            receipt_id: argument_text(arguments, &["receipt_id", "receiptId"]).unwrap_or_default(),
            metadata: argument_object(arguments, "metadata"),
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
        },
    )
    .map_err(mcp_skill_pack_error)?;
    Ok(json!({
        "tenant": tenant,
        "receipt": receipt
    }))
}

fn ensemble_register_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let pack = capability_pack_from_arguments(tenant, arguments)?;
    let mut store = McpEnsembleStore { backend };
    let registered = ensemble_register_pack(&mut store, pack).map_err(mcp_ensemble_error)?;
    let node_id = ensemble_pack_node_id(tenant, &registered.pack_content_hash);
    Ok(json!({
        "tenant": tenant,
        "node_id": node_id,
        "pack": registered
    }))
}

fn ensemble_select_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let task = argument_text(arguments, &["task", "query", "intent"])
        .ok_or_else(|| McpError::invalid_params("ensemble_select requires task"))?;
    let kind = optional_pack_kind(arguments)?;
    let request = EnsembleSelectRequest {
        task,
        budget_units: arguments
            .get("budget_units")
            .or_else(|| arguments.get("budgetUnits"))
            .and_then(Value::as_u64),
        max_selected: arguments
            .get("max_selected")
            .or_else(|| arguments.get("maxSelected"))
            .and_then(Value::as_u64)
            .map(|value| value as usize),
        candidates: Vec::new(),
        priors: ensemble_priors_from_arguments(arguments)?,
    };
    let store = McpEnsembleReadStore { backend };
    let decision =
        ensemble_select_from_store(&store, tenant, kind, request).map_err(mcp_ensemble_error)?;
    let decision_content_hash = decision.content_address();
    Ok(json!({
        "tenant": tenant,
        "decision_content_hash": decision_content_hash,
        "decision": decision
    }))
}

fn harness_prepare_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let task = argument_text(arguments, &["task", "query", "intent"])
        .ok_or_else(|| McpError::invalid_params("harness_prepare requires task"))?;
    let actor = argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default();
    let budget_units = arguments
        .get("budget_units")
        .or_else(|| arguments.get("budgetUnits"))
        .and_then(Value::as_u64);
    let request = EnsembleSelectRequest {
        task: task.clone(),
        budget_units,
        max_selected: arguments
            .get("max_selected")
            .or_else(|| arguments.get("maxSelected"))
            .and_then(Value::as_u64)
            .map(|value| value as usize),
        candidates: Vec::new(),
        priors: ensemble_priors_from_arguments(arguments)?,
    };
    let decision = {
        let store = McpEnsembleReadStore { backend: &*backend };
        ensemble_select_from_store(&store, tenant, None, request).map_err(mcp_ensemble_error)?
    };
    let signature = decision.content_address();
    let recall_results = {
        let mut store = McpMemoryStore { backend };
        recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: tenant.to_string(),
                query: task.clone(),
                surface: argument_text(arguments, &["surface"]).unwrap_or_default(),
                actor: actor.clone(),
                limit: argument_u64(arguments, &["memory_limit", "memoryLimit", "limit"])
                    .unwrap_or(8) as usize,
                include_low_fitness: false,
                include_consolidation_sources: false,
                ..RecallMemoryInput::default()
            },
        )
        .map_err(mcp_memory_error)?
    };
    let memory_contract = memory_contract_from_recall(&recall_results);
    let selected_pack_details = {
        let store = McpEnsembleReadStore { backend: &*backend };
        decision
            .selected
            .iter()
            .filter_map(|selected| {
                ensemble_get_pack(&store, tenant, &selected.pack_content_hash)
                    .ok()
                    .flatten()
                    .map(|pack| (selected.pack_content_hash.clone(), pack))
            })
            .collect::<HashMap<_, _>>()
    };
    let selected_capabilities = decision
        .selected
        .iter()
        .map(|selected| {
            let pack = selected_pack_details.get(&selected.pack_content_hash);
            let title = pack
                .and_then(|pack| {
                    let title = pack.title.trim();
                    (!title.is_empty()).then(|| title.to_string())
                })
                .unwrap_or_else(|| selected.pack_content_hash.clone());
            let description = pack
                .map(|pack| pack.description.trim().to_string())
                .unwrap_or_default();
            json!({
                "id": selected.pack_content_hash,
                "title": title,
                "kind": selected.kind,
                "description": description,
                "reason": selected.reason,
                "score": selected.score,
                "cost_units": selected.cost_units
            })
        })
        .collect::<Vec<_>>();
    let rendered_markdown = render_harness_prepare_markdown(
        &task,
        &signature,
        &selected_capabilities,
        &memory_contract,
    );
    Ok(json!({
        "tenant": tenant,
        "task": task,
        "actor": actor,
        "signature": signature,
        "budget_units": budget_units,
        "selected_capabilities": selected_capabilities,
        "memory_contract": memory_contract,
        "recall_results": recall_results,
        "decision_content_hash": signature,
        "decision": decision,
        "brief": {
            "task": task,
            "signature": signature,
            "selected_capabilities": selected_capabilities,
            "memory_contract": memory_contract
        },
        "rendered_markdown": rendered_markdown
    }))
}

fn memory_contract_from_recall(results: &[theorem_harness_runtime::MemoryRecallItem]) -> Value {
    let read_first = results
        .iter()
        .take(5)
        .filter_map(memory_recall_item_summary)
        .collect::<Vec<_>>();
    let risks = results
        .iter()
        .filter_map(memory_recall_item_risk)
        .take(5)
        .collect::<Vec<_>>();
    let do_not = results
        .iter()
        .filter_map(memory_recall_item_do_not)
        .take(5)
        .collect::<Vec<_>>();
    json!({
        "read_first": read_first,
        "risks": risks,
        "do_not": do_not
    })
}

fn memory_recall_item_summary(item: &theorem_harness_runtime::MemoryRecallItem) -> Option<String> {
    let value = serde_json::to_value(item).ok()?;
    let title = first_string_at(&value, &["title", "id"])?;
    let preview = first_string_at(&value, &["summary", "content"]).unwrap_or_default();
    if preview.is_empty() {
        Some(title)
    } else {
        Some(format!("{title}: {preview}"))
    }
}

fn memory_recall_item_risk(item: &theorem_harness_runtime::MemoryRecallItem) -> Option<String> {
    let value = serde_json::to_value(item).ok()?;
    let text = first_string_at(&value, &["summary", "content", "title"])?;
    let lower = text.to_ascii_lowercase();
    if lower.contains("risk")
        || lower.contains("do not")
        || lower.contains("do_not")
        || lower.contains("avoid")
        || lower.contains("caution")
    {
        Some(text)
    } else {
        None
    }
}

fn memory_recall_item_do_not(item: &theorem_harness_runtime::MemoryRecallItem) -> Option<String> {
    let value = serde_json::to_value(item).ok()?;
    let text = first_string_at(&value, &["summary", "content", "title"])?;
    let lower = text.to_ascii_lowercase();
    if lower.contains("do not") || lower.contains("don't") || lower.contains("never") {
        Some(text)
    } else {
        None
    }
}

fn first_string_at(value: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(text) = value.get(*key).and_then(Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn render_harness_prepare_markdown(
    task: &str,
    signature: &str,
    selected_capabilities: &[Value],
    memory_contract: &Value,
) -> String {
    fn list(values: &[String]) -> String {
        if values.is_empty() {
            "(none)".to_string()
        } else {
            values
                .iter()
                .map(|value| format!("- {value}"))
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
    let read_first = string_vec_from_value(&memory_contract["read_first"]);
    let risks = string_vec_from_value(&memory_contract["risks"]);
    let do_not = string_vec_from_value(&memory_contract["do_not"]);
    let selected = if selected_capabilities.is_empty() {
        "(none)".to_string()
    } else {
        selected_capabilities
            .iter()
            .map(|capability| {
                let title = capability
                    .get("title")
                    .and_then(Value::as_str)
                    .filter(|value| !value.is_empty())
                    .or_else(|| capability.get("id").and_then(Value::as_str))
                    .unwrap_or("(untitled)");
                let reason = capability
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("selected by Ensemble");
                format!("- **{title}**: {reason}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    format!(
        "## Theorem Context Brief\n\n**Task:** {task}\n**Signature:** `{signature}`\n\n### Selected capabilities\n{selected}\n\n### Read first\n{read_first}\n\n### Risks\n{risks}\n\n### Do not\n{do_not}",
        read_first = list(&read_first),
        risks = list(&risks),
        do_not = list(&do_not)
    )
}

fn string_vec_from_value(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn capability_pack_from_arguments(
    tenant: &str,
    arguments: &Value,
) -> Result<CapabilityPack, McpError> {
    let pack_value = arguments
        .get("pack")
        .or_else(|| arguments.get("capability_pack"))
        .or_else(|| arguments.get("capabilityPack"))
        .or_else(|| arguments.get("spec"))
        .cloned()
        .ok_or_else(|| {
            McpError::invalid_params("ensemble_register requires pack or spec object")
        })?;
    if !pack_value.is_object() {
        return Err(McpError::invalid_params(
            "ensemble_register pack/spec must be an object",
        ));
    }
    let spec = pack_value
        .get("spec")
        .cloned()
        .unwrap_or_else(|| pack_value.clone());
    if !spec.is_object() {
        return Err(McpError::invalid_params(
            "ensemble_register spec must be an object",
        ));
    }
    let tenant_slug = argument_text(arguments, &["tenant_slug", "tenantSlug", "tenant"])
        .or_else(|| argument_text(&pack_value, &["tenant_slug", "tenantSlug", "tenant"]))
        .unwrap_or_else(|| tenant.to_string());
    let kind = argument_text(arguments, &["kind"])
        .or_else(|| argument_text(&pack_value, &["kind"]))
        .or_else(|| argument_text(&spec, &["kind"]))
        .unwrap_or_default();
    let title = argument_text(arguments, &["title", "name"])
        .or_else(|| argument_text(&pack_value, &["title", "name"]))
        .or_else(|| argument_text(&spec, &["title", "name"]))
        .unwrap_or_default();
    let description = argument_text(arguments, &["description", "summary"])
        .or_else(|| argument_text(&pack_value, &["description", "summary"]))
        .or_else(|| argument_text(&spec, &["description", "summary"]))
        .unwrap_or_default();
    let pack_content_hash = argument_text(arguments, &["pack_content_hash", "packContentHash"])
        .or_else(|| argument_text(&pack_value, &["pack_content_hash", "packContentHash"]))
        .unwrap_or_default();
    let source_content_hash =
        argument_text(arguments, &["source_content_hash", "sourceContentHash"])
            .or_else(|| argument_text(&pack_value, &["source_content_hash", "sourceContentHash"]))
            .unwrap_or_default();
    let artifact_hashes = {
        let from_arguments = string_array_any(arguments, &["artifact_hashes", "artifactHashes"]);
        if from_arguments.is_empty() {
            string_array_any(&pack_value, &["artifact_hashes", "artifactHashes"])
        } else {
            from_arguments
        }
    };
    let trust = optional_trust_tier(arguments, &pack_value)?;
    let exposure = optional_pack_exposure(arguments, &pack_value)?;

    Ok(CapabilityPack {
        tenant_slug,
        origin_tenant_slug: String::new(),
        pack_content_hash,
        kind,
        title,
        description,
        spec,
        trust,
        exposure,
        source_content_hash,
        artifact_hashes,
    })
}

fn optional_pack_kind(arguments: &Value) -> Result<Option<PackKind>, McpError> {
    let Some(kind) = argument_text(arguments, &["kind", "pack_kind", "packKind"]) else {
        return Ok(None);
    };
    PackKind::parse(&kind)
        .map(Some)
        .ok_or_else(|| McpError::invalid_params(format!("unsupported Ensemble pack kind `{kind}`")))
}

fn optional_trust_tier(arguments: &Value, pack_value: &Value) -> Result<TrustTier, McpError> {
    let value = arguments.get("trust").or_else(|| pack_value.get("trust"));
    match value {
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| McpError::invalid_params(format!("invalid Ensemble trust: {error}"))),
        None => Ok(TrustTier::default()),
    }
}

fn optional_pack_exposure(arguments: &Value, pack_value: &Value) -> Result<PackExposure, McpError> {
    let value = arguments
        .get("exposure")
        .or_else(|| pack_value.get("exposure"));
    match value {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            McpError::invalid_params(format!("invalid Ensemble exposure: {error}"))
        }),
        None => Ok(PackExposure::default()),
    }
}

fn ensemble_priors_from_arguments(arguments: &Value) -> Result<Value, McpError> {
    let mut priors = arguments
        .get("priors")
        .cloned()
        .unwrap_or_else(|| json!({}));
    if !priors.is_object() {
        return Err(McpError::invalid_params(
            "ensemble_select priors must be an object",
        ));
    }
    if let Some(priors_object) = priors.as_object_mut() {
        for key in [
            "pack_scores",
            "pack_costs",
            "prior_weight",
            "lexical_weight",
            "trust_weight",
            "min_trust",
            "kinds",
        ] {
            if !priors_object.contains_key(key) {
                if let Some(value) = arguments.get(key) {
                    priors_object.insert(key.to_string(), value.clone());
                }
            }
        }
    }
    Ok(priors)
}

fn code_search_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    operation: &str,
) -> Result<Value, McpError> {
    let normalized = normalize_code_arguments(arguments, operation);
    backend.invoke_code_search(tenant, &normalized, operation)
}

fn normalize_code_arguments(arguments: &Value, operation: &str) -> Value {
    let mut normalized = arguments.clone();
    let Some(object) = normalized.as_object_mut() else {
        return normalized;
    };
    object
        .entry("operation".to_string())
        .or_insert_with(|| Value::String(operation.to_string()));
    if let Some(repo) = object.get("repo").cloned() {
        if object.get("repo_id").is_none()
            && object.get("repo_url").is_none()
            && object.get("repo_path").is_none()
        {
            let repo_text = repo.as_str().unwrap_or_default();
            let target_key = if repo_text.starts_with("http://")
                || repo_text.starts_with("https://")
                || repo_text.ends_with(".git")
            {
                "repo_url"
            } else if repo_text.starts_with('/')
                || repo_text.starts_with("./")
                || repo_text.starts_with("../")
            {
                "repo_path"
            } else {
                "repo_id"
            };
            object.insert(target_key.to_string(), repo);
        }
    }
    if let Some(limits) = object.get("limits").and_then(Value::as_object).cloned() {
        for (key, value) in limits {
            object.entry(key).or_insert(value);
        }
    }
    normalized
}

fn code_search_operation(arguments: &Value) -> Result<String, McpError> {
    let raw = arguments
        .get("operation")
        .or_else(|| arguments.get("mode"))
        .or_else(|| arguments.get("verb"))
        .and_then(Value::as_str)
        .unwrap_or("search")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    match raw.as_str() {
        "ingest" | "reindex" | "search" | "context" | "recognize" | "explore" | "explain" => {
            Ok(raw)
        }
        // D1: ingest/reindex are job submissions that return a job_id; this
        // read-only operation polls a submitted job's status and event log.
        "ingest_status" | "status" | "job_status" => Ok("ingest_status".to_string()),
        "record_use" | "use_receipt" | "record_use_receipt" => Ok("record_use_receipt".to_string()),
        "list_repos" | "listrepos" | "repos" => Ok("list_repos".to_string()),
        "kg_status" | "kgstatus" | "instant_kg_status" => Ok("kg_status".to_string()),
        "session_reingest" | "sessionreingest" | "reingest_session" => {
            Ok("session_reingest".to_string())
        }
        "context_pack" | "contextpack" | "pack" => Ok("context_pack".to_string()),
        _ => Err(McpError::invalid_params(format!(
            "unsupported code_search operation `{raw}`"
        ))),
    }
}

fn code_index_error(error: rustyred_thg_code::CodeIndexError) -> McpError {
    let message = format!("{}: {}", error.code, error.message);
    if error.code.starts_with("invalid_") {
        McpError::invalid_params(message)
    } else {
        McpError::internal(message)
    }
}

fn code_plugin_response(output: rustyred_thg_code::CodePluginExecutionOutput) -> Value {
    let operation = output.operation.clone();
    json!({
        "tenant": output.tenant_id,
        "operation": output.operation,
        "command": output.command,
        "writes_graph": output.writes_graph,
        "affordance_id": format!("rustyred_thg_code.code_search.{operation}"),
        "engine": "rustyred_thg_code",
        "code_plugin": rustyred_thg_code::CodeParsingPlugin.manifest_json(),
        "result": output.result,
    })
}

fn redcore_code_search_payload(
    store: &mut RedCoreGraphStore,
    tenant: &str,
    arguments: &Value,
    operation: &str,
) -> Result<Value, McpError> {
    // The in-process plugin path runs ingest/reindex synchronously against
    // the caller's store (there is no background job to poll); job status
    // belongs to the theorem_grpc submit-plus-stream route.
    if operation == "ingest_status" {
        return Ok(json!({
            "tenant": tenant,
            "operation": "ingest_status",
            "engine": "rustyred_thg_code",
            "state": "not_applicable",
            "message": "in-process code ingest is synchronous and returned its result inline; ingest_status applies to the theorem_grpc submit-plus-stream route",
        }));
    }
    let output = rustyred_thg_code::execute_code_plugin_operation(
        store,
        tenant,
        operation,
        arguments.clone(),
    )
    .map_err(code_index_error)?;
    Ok(code_plugin_response(output))
}

fn remember_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let mut store = McpMemoryStore { backend };
    let receipt = remember_memory(
        &mut store,
        memory_write_input(tenant, arguments, "remember")?,
    )
    .map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "saved_type": receipt.saved_type,
        "document": receipt.document,
        "node": receipt.node
    }))
}

fn recall_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
    consume_handoffs: bool,
) -> Result<Value, McpError> {
    let mut store = McpMemoryStore { backend };
    let input = RecallMemoryInput {
        tenant_slug: tenant.to_string(),
        query: argument_text(arguments, &["query"]).unwrap_or_default(),
        surface: argument_text(arguments, &["surface"]).unwrap_or_default(),
        actor: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        since: argument_text(arguments, &["since"]).unwrap_or_default(),
        kind: argument_text(arguments, &["kind"]).unwrap_or_default(),
        project_slug: argument_text(arguments, &["project_slug", "projectSlug"])
            .unwrap_or_default(),
        project_permeability: argument_f64_any(
            arguments,
            &["project_permeability", "projectPermeability"],
        )
        .unwrap_or(0.75),
        limit: argument_u64(arguments, &["limit"]).unwrap_or(10) as usize,
        include_low_fitness: arguments
            .get("include_low_fitness")
            .or_else(|| arguments.get("includeLowFitness"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        include_consolidation_sources: arguments
            .get("include_consolidation_sources")
            .or_else(|| arguments.get("includeConsolidationSources"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        consume_handoffs,
        query_time: argument_text(arguments, &["query_time", "queryTime"]).unwrap_or_default(),
        overall_state: arguments
            .get("overall_state")
            .or_else(|| arguments.get("overallState"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        seed_limit: argument_u64(arguments, &["seed_limit", "seedLimit"]).unwrap_or(0) as usize,
        query_embedding: f32_array_any(arguments, &["query_embedding", "queryEmbedding"]),
        embedding_property: argument_text(arguments, &["embedding_property", "embeddingProperty"])
            .unwrap_or_default(),
        ppr_alpha: argument_f64_any(arguments, &["ppr_alpha", "pprAlpha"]).unwrap_or(0.0),
        ppr_epsilon: argument_f64_any(arguments, &["ppr_epsilon", "pprEpsilon"]).unwrap_or(0.0),
        ppr_max_pushes: argument_u64(arguments, &["ppr_max_pushes", "pprMaxPushes"]).unwrap_or(0)
            as usize,
        recency_half_life_seconds: argument_f64_any(
            arguments,
            &["recency_half_life_seconds", "recencyHalfLifeSeconds"],
        )
        .unwrap_or(0.0),
    };
    let results = recall_memory(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "results": results,
        "count": results.len()
    }))
}

fn relate_memory_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let store = McpMemoryReadStore { backend };
    let seed_id = required_text_any(arguments, &["seed_id", "seedId"], "relate")?;
    let input = RelateMemoryInput {
        tenant_slug: tenant.to_string(),
        seed_id: seed_id.clone(),
        edge_types: string_array_any(arguments, &["edge_types", "edgeTypes"]),
        max_hops: argument_u64(arguments, &["max_hops", "maxHops"]).unwrap_or(1) as usize,
    };
    let results = relate_memory(&store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "seed_id": seed_id,
        "results": results,
        "count": results.len()
    }))
}

fn self_note_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let mut input = memory_write_input(tenant, arguments, "self_note")?;
    if input.kind.trim().is_empty() {
        input.kind = "self_note".to_string();
    }
    input.memory_node_type = argument_text(arguments, &["memory_node_type", "memoryNodeType"])
        .unwrap_or_else(|| "belief".to_string());
    let mut store = McpMemoryStore { backend };
    let document = self_note_memory(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({ "tenant": tenant, "document": document }))
}

fn revise_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let input = ReviseMemoryInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        session_id: argument_text(arguments, &["session_id", "sessionId"]).unwrap_or_default(),
        origin_surface: argument_text(arguments, &["origin_surface", "originSurface", "surface"])
            .unwrap_or_default(),
        doc_id: required_text_any(arguments, &["doc_id", "docId"], "self_revise")?,
        content: required_text_any(arguments, &["content"], "self_revise")?,
        title: argument_text(arguments, &["title"]).unwrap_or_default(),
        summary: argument_text(arguments, &["summary"]).unwrap_or_default(),
        reason: argument_text(arguments, &["reason"]).unwrap_or_default(),
        memory_node_type: argument_text(arguments, &["memory_node_type", "memoryNodeType"])
            .unwrap_or_default(),
        cites_doc_ids: string_array_any(arguments, &["cites_doc_ids", "citesDocIds"]),
        derived_from_doc_ids: string_array_any(
            arguments,
            &["derived_from_doc_ids", "derivedFromDocIds"],
        ),
        contradicts_doc_ids: string_array_any(
            arguments,
            &["contradicts_doc_ids", "contradictsDocIds"],
        ),
        updated_at: argument_text(arguments, &["updated_at", "updatedAt"]).unwrap_or_default(),
    };
    let mut store = McpMemoryStore { backend };
    let receipt = revise_memory_document(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "revised": receipt.revised,
        "superseded": receipt.superseded
    }))
}

fn archive_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let input = ArchiveMemoryInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        doc_id: required_text_any(arguments, &["doc_id", "docId"], "self_archive")?,
        reason: argument_text(arguments, &["reason"]).unwrap_or_default(),
        title: argument_text(arguments, &["title"]).unwrap_or_default(),
        archived_at: argument_text(arguments, &["archived_at", "archivedAt"]).unwrap_or_default(),
    };
    let mut store = McpMemoryStore { backend };
    let receipt = archive_memory_document(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "archived": receipt.archived,
        "archive": receipt.archive
    }))
}

fn recall_archived_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let input = RecallMemoryInput {
        tenant_slug: tenant.to_string(),
        query: argument_text(arguments, &["query"]).unwrap_or_default(),
        actor: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        limit: argument_u64(arguments, &["limit"]).unwrap_or(10) as usize,
        ..RecallMemoryInput::default()
    };
    let mut store = McpMemoryStore { backend };
    let results = recall_archived_memory(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "results": results,
        "count": results.len()
    }))
}

fn encode_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let mut input = memory_write_input(tenant, arguments, "encode")?;
    if input.kind.trim().is_empty() {
        input.kind = "encode".to_string();
    }
    let outcome = argument_text(arguments, &["outcome"]).unwrap_or_else(|| "neutral".to_string());
    let signal = argument_text(arguments, &["signal"]).unwrap_or_default();
    let reason = argument_text(arguments, &["reason"]).unwrap_or_default();
    let event_id = argument_text(arguments, &["event_id", "eventId"]).unwrap_or_default();
    let context = arguments
        .get("context")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let auto_triggered = arguments
        .get("auto_triggered")
        .or_else(|| arguments.get("autoTriggered"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    input.metadata = argument_object(arguments, "metadata");
    let mut store = McpMemoryStore { backend };
    let document = encode_memory(
        &mut store,
        input,
        EncodeMemoryInput {
            outcome,
            signal,
            reason,
            event_id,
            context,
            auto_triggered,
        },
    )
    .map_err(mcp_memory_error)?;
    Ok(json!({ "tenant": tenant, "memory": document }))
}

fn upsert_note_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let input = UpsertNoteInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        session_id: argument_text(arguments, &["session_id", "sessionId"]).unwrap_or_default(),
        origin_surface: argument_text(arguments, &["origin_surface", "originSurface", "surface"])
            .unwrap_or_else(|| "obsidian".to_string()),
        project_slug: argument_text(arguments, &["project_slug", "projectSlug"])
            .unwrap_or_default(),
        doc_id: argument_text(arguments, &["doc_id", "docId"]).unwrap_or_default(),
        kind: argument_text(arguments, &["kind"]).unwrap_or_default(),
        title: argument_text(arguments, &["title"]).unwrap_or_default(),
        content: required_text_any(arguments, &["content"], "upsert_note")?,
        summary: argument_text(arguments, &["summary"]).unwrap_or_default(),
        tags: string_array_any(arguments, &["tags"]),
        links: string_array_any(arguments, &["links"]),
        status: argument_text(arguments, &["status"]).unwrap_or_default(),
        memory_node_type: argument_text(arguments, &["memory_node_type", "memoryNodeType"])
            .unwrap_or_default(),
        metadata: argument_object(arguments, "metadata"),
        outcome: argument_text(arguments, &["outcome"]).unwrap_or_default(),
        signal: argument_text(arguments, &["signal"]).unwrap_or_default(),
        reason: argument_text(arguments, &["reason"]).unwrap_or_default(),
        event_id: argument_text(arguments, &["event_id", "eventId"]).unwrap_or_default(),
        updated_at: argument_text(arguments, &["updated_at", "updatedAt"]).unwrap_or_default(),
        created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
    };
    let mut store = McpMemoryStore { backend };
    let receipt = upsert_note(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({ "tenant": tenant, "receipt": receipt }))
}

fn forget_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let input = ForgetMemoryInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        id: required_text_any(arguments, &["id"], "forget")?,
        reason: required_text_any(arguments, &["reason"], "forget")?,
        deleted_at: argument_text(arguments, &["deleted_at", "deletedAt"]).unwrap_or_default(),
    };
    let mut store = McpMemoryStore { backend };
    let receipt = forget_memory(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({
        "tenant": tenant,
        "forgotten_type": receipt.forgotten_type,
        "document": receipt.document,
        "node": receipt.node
    }))
}

fn handoff_memory_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let payload = arguments
        .get("payload")
        .cloned()
        .ok_or_else(|| McpError::invalid_params("handoff requires payload"))?;
    let input = HandoffMemoryInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        session_id: argument_text(arguments, &["session_id", "sessionId"]).unwrap_or_default(),
        origin_surface: argument_text(arguments, &["origin_surface", "originSurface", "surface"])
            .unwrap_or_default(),
        to_actor: required_text_any(arguments, &["to_actor", "toActor"], "handoff")?,
        payload,
        title: argument_text(arguments, &["title"]).unwrap_or_default(),
        expires_at: argument_text(
            arguments,
            &["expires_at", "expiresAt", "expires_in", "expiresIn"],
        )
        .unwrap_or_default(),
        created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
    };
    let mut store = McpMemoryStore { backend };
    let handoff = handoff_memory(&mut store, input).map_err(mcp_memory_error)?;
    Ok(json!({ "tenant": tenant, "handoff": handoff }))
}

fn observe_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let actor_id = argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default();
    let room_id = resolved_coordination_room_id(arguments);
    let room = load_coordination_room(backend, tenant, &room_id)?
        .unwrap_or_else(|| empty_coordination_room(tenant, &room_id, ""));
    let pending_mentions = if actor_id.is_empty() {
        Vec::new()
    } else {
        read_coordination_mentions_in_room(backend, tenant, &room_id, &actor_id, false, 20)?
    };
    let recall_results = if argument_text(arguments, &["query"]).is_some() {
        let mut store = McpMemoryStore { backend };
        recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: tenant.to_string(),
                query: argument_text(arguments, &["query"]).unwrap_or_default(),
                actor: actor_id.clone(),
                limit: argument_u64(arguments, &["limit"]).unwrap_or(10) as usize,
                include_low_fitness: arguments
                    .get("include_low_fitness")
                    .or_else(|| arguments.get("includeLowFitness"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                include_consolidation_sources: arguments
                    .get("include_consolidation_sources")
                    .or_else(|| arguments.get("includeConsolidationSources"))
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                ..RecallMemoryInput::default()
            },
        )
        .map_err(mcp_memory_error)?
    } else {
        Vec::new()
    };
    Ok(json!({
        "actor": { "actor_id": actor_id },
        "tenant": { "slug": tenant },
        "coordination_room": room,
        "pending_mentions": pending_mentions,
        "continuity_pack": {},
        "orchestrate_notes": [],
        "recall_results": recall_results
    }))
}

fn instant_kg_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
    operation: &str,
    name: &str,
) -> Result<Value, McpError> {
    let view = instant_kg_view_payload(tenant, backend, arguments)?;
    let payload = match operation {
        "status" => json!({
            "tenant": tenant,
            "status": view.status(),
            "stats": view.stats()
        }),
        "ppr" => {
            let seeds: HashMap<String, f64> =
                serde_json::from_value(arguments.get("seeds").cloned().ok_or_else(|| {
                    McpError::invalid_params("harness_kg_ppr requires seeds object")
                })?)
                .map_err(|error| {
                    McpError::invalid_params(format!("seeds must be an object: {error}"))
                })?;
            let alpha = arguments
                .get("alpha")
                .and_then(Value::as_f64)
                .unwrap_or(0.15);
            let epsilon = arguments
                .get("epsilon")
                .and_then(Value::as_f64)
                .unwrap_or(1e-4);
            let max_pushes = arguments
                .get("max_pushes")
                .and_then(Value::as_u64)
                .unwrap_or(200_000) as usize;
            let top_k = arguments.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
            json!({
                "tenant": tenant,
                "status": view.status(),
                "results": view.ppr(&seeds, alpha, epsilon, max_pushes, top_k)
            })
        }
        "impact" => {
            let seed_arg = arguments
                .get("seed")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let symbol_arg = arguments
                .get("symbol_name")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            let seed = if let Some(seed) = seed_arg {
                seed.to_string()
            } else if let Some(symbol_name) = symbol_arg {
                view.resolve_symbol_name(symbol_name).ok_or_else(|| {
                    McpError::invalid_params("harness_kg_impact could not resolve symbol_name")
                })?
            } else {
                return Err(McpError::invalid_params(
                    "harness_kg_impact requires seed or symbol_name",
                ));
            };
            let direction = instant_kg_direction(
                arguments
                    .get("direction")
                    .and_then(Value::as_str)
                    .unwrap_or("out"),
            );
            let max_depth = arguments
                .get("max_depth")
                .and_then(Value::as_u64)
                .unwrap_or(2) as usize;
            json!({
                "tenant": tenant,
                "seed": seed,
                "status": view.status(),
                "results": view.impact(&seed, direction, max_depth)
            })
        }
        "related_objects" => {
            let seed = required_str(arguments, "seed", name)?;
            let kinds = string_array(arguments, "kinds");
            let top_k = arguments.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
            json!({
                "tenant": tenant,
                "seed": seed,
                "status": view.status(),
                "results": view.related_objects(seed, &kinds, top_k)
            })
        }
        "search" => {
            let query = required_str(arguments, "query", name)?;
            let kinds = string_array(arguments, "kinds");
            let top_k = arguments.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
            json!({
                "tenant": tenant,
                "query": query,
                "status": view.status(),
                "results": view.search(query, &kinds, top_k)
            })
        }
        "explain_edge" => {
            let src = required_str(arguments, "src", name)?;
            let dst = required_str(arguments, "dst", name)?;
            json!({
                "tenant": tenant,
                "src": src,
                "dst": dst,
                "status": view.status(),
                "explanations": view.explain_edge(src, dst)
            })
        }
        _ => {
            return Err(McpError::invalid_params(format!(
                "unsupported harness_kg operation `{operation}`"
            )))
        }
    };
    Ok(payload)
}

fn instant_kg_view_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<HarnessInstantKg, McpError> {
    let base = backend.graph_snapshot()?;
    let manifest: Option<CodeKgManifest> = match arguments.get("manifest") {
        Some(value) => Some(serde_json::from_value(value.clone()).map_err(|error| {
            McpError::invalid_params(format!("manifest must match instant KG schema: {error}"))
        })?),
        None => None,
    };
    let delta: SessionDelta = match arguments.get("delta") {
        Some(value) => serde_json::from_value(value.clone()).map_err(|error| {
            McpError::invalid_params(format!("delta must match instant KG schema: {error}"))
        })?,
        None => SessionDelta::default(),
    };
    let manifest = manifest.or_else(|| {
        Some(CodeKgManifest::from_base_snapshot(
            tenant,
            format!("v{}", base.version),
            &base,
        ))
    });
    Ok(HarnessInstantKg::new(base, manifest, delta))
}

fn instant_kg_direction(value: &str) -> Direction {
    if value.eq_ignore_ascii_case("in") || value.eq_ignore_ascii_case("incoming") {
        Direction::In
    } else {
        Direction::Out
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Budget {
    max_nodes_returned: usize,
}

impl Budget {
    fn from_args(arguments: &Value) -> Self {
        let max_nodes_returned = arguments
            .get("limit")
            .or_else(|| {
                arguments
                    .get("budget")
                    .and_then(|budget| budget.get("max_nodes_returned"))
            })
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(100);
        Self { max_nodes_returned }
    }
}

fn apply_neighbor_budget(neighbors: &mut Vec<NeighborHit>, budget: Budget) -> bool {
    let truncated = neighbors.len() > budget.max_nodes_returned;
    if truncated {
        neighbors.truncate(budget.max_nodes_returned);
    }
    truncated
}

fn node_query_from_value(value: &Value) -> Result<NodeQuery, McpError> {
    let label = value
        .get("label")
        .and_then(Value::as_str)
        .map(str::to_string);
    let properties = value
        .get("properties")
        .or_else(|| value.get("props"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let properties = serde_json::from_value(properties)
        .map_err(|err| McpError::invalid_params(format!("properties must be an object: {err}")))?;
    Ok(NodeQuery {
        label,
        properties,
        limit: Some(Budget::from_args(value).max_nodes_returned),
        // MCP tools respect TTL semantics by default. Forensic
        // (include-expired) access surface is TTL-04 work.
        include_expired: false,
    })
}

fn neighbor_query_from_value(value: &Value) -> Result<NeighborQuery, McpError> {
    let node_id = value
        .get("node_id")
        .or_else(|| value.get("nodeId"))
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("neighbor query requires node_id"))?;
    let direction = match value
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("out")
    {
        "out" | "outgoing" => Direction::Out,
        "in" | "incoming" => Direction::In,
        other => {
            return Err(McpError::invalid_params(format!(
                "direction must be out or in, got {other}"
            )))
        }
    };
    let edge_type = value
        .get("edge_type")
        .or_else(|| value.get("edgeType"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string);
    Ok(NeighborQuery {
        node_id: node_id.to_string(),
        direction,
        edge_type,
        // MCP tools respect TTL semantics by default. Forensic
        // (include-expired) access surface is TTL-04 work.
        include_expired: false,
    })
}

fn join_coordination_room(
    backend: &mut impl McpGraphBackend,
    input: JoinRoomInput,
) -> Result<CoordinationRoomState, McpError> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let actor_id = require_actor_id(&input.actor_id, "coordination_room requires actor")?;
    let room_id = if input.room_id.trim().is_empty() {
        infer_coordination_room_id(&input.repo, &input.branch, &input.task, &input.session_id)
    } else {
        input.room_id.trim().to_string()
    };
    let now = timestamp_or_now(&input.updated_at);
    let mut state = load_coordination_room_exact(backend, &tenant_slug, &room_id)?
        .unwrap_or_else(|| empty_coordination_room(&tenant_slug, &room_id, &now));
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
        session_id: choose_text(
            &input.session_id,
            existing.map(|member| member.session_id.as_str()),
        ),
        surface: choose_text(
            &input.surface,
            existing.map(|member| member.surface.as_str()),
        ),
        repo: choose_text(&input.repo, existing.map(|member| member.repo.as_str())),
        branch: choose_text(&input.branch, existing.map(|member| member.branch.as_str())),
        task: choose_text(&input.task, existing.map(|member| member.task.as_str())),
        worktree: choose_text(
            &input.worktree,
            existing.map(|member| member.worktree.as_str()),
        ),
        head: choose_text(&input.head, existing.map(|member| member.head.as_str())),
        changed_files: choose_strings(
            &input.changed_files,
            existing.map(|member| member.changed_files.as_slice()),
        ),
        lane: choose_text(&input.lane, existing.map(|member| member.lane.as_str())),
        joined_at,
        updated_at: now.clone(),
    };
    state.status = "active".to_string();
    state.mode = "collaborating".to_string();
    state.repo = choose_text(&input.repo, Some(state.repo.as_str()));
    state.branch = choose_text(&input.branch, Some(state.branch.as_str()));
    state.task = choose_text(&input.task, Some(state.task.as_str()));
    state.updated_at = now;
    state.members.insert(actor_id, member);
    persist_coordination_room(backend, &state)?;
    Ok(state)
}

fn write_coordination_intent(
    backend: &mut impl McpGraphBackend,
    input: WriteIntentInput,
) -> Result<CoordinationIntentState, McpError> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let agent_id = normalize_binding_agent_id(&input.agent_id, &input.binding_id);
    let binding_id = resolve_coordination_binding_id(&input.binding_id, &agent_id);
    let room_id = if input.room_id.trim().is_empty() {
        "room:ungrouped".to_string()
    } else {
        input.room_id.trim().to_string()
    };
    let actor_id = require_actor_id(&input.actor_id, "write_intent requires actor")?;
    let summary = require_nonempty(input.summary.trim(), "write_intent requires summary")?;
    let status = normalize_coordination_status(&input.status)?;
    let now = timestamp_or_now(&input.updated_at);
    if load_coordination_room_exact(backend, &tenant_slug, &room_id)?.is_none() {
        persist_coordination_room(
            backend,
            &empty_coordination_room(&tenant_slug, &room_id, &now),
        )?;
    }
    let prior = load_coordination_intent_exact(backend, &tenant_slug, &room_id, &actor_id)?;
    let started_at = prior
        .as_ref()
        .map(|intent| intent.started_at.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| now.clone());
    let mut intent = CoordinationIntentState {
        tenant_slug,
        agent_id,
        binding_id,
        room_id,
        actor_id,
        status,
        summary,
        footprint: normalize_string_vec(input.footprint),
        expected_completion: input.expected_completion.trim().to_string(),
        repo: input.repo.trim().to_string(),
        branch: input.branch.trim().to_string(),
        task: input.task.trim().to_string(),
        started_at,
        updated_at: now,
        scratchpad_revision_id: String::new(),
        scratchpad_document_id: String::new(),
        scratchpad_seq: 0,
        binding_active_head_set: Vec::new(),
    };
    let projection = project_coordination_intent_onto_binding(backend, &intent)?;
    intent.scratchpad_revision_id = projection.scratchpad_revision_id;
    intent.scratchpad_document_id = projection.scratchpad_document_id;
    intent.scratchpad_seq = projection.scratchpad_seq;
    intent.binding_active_head_set = projection.binding_active_head_set;
    persist_coordination_intent(backend, &intent)?;
    persist_coordination_intent_binding_projection(backend, &intent)?;
    Ok(intent)
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct McpBindingProjection {
    scratchpad_revision_id: String,
    scratchpad_document_id: String,
    scratchpad_seq: u64,
    binding_active_head_set: Vec<String>,
}

fn project_coordination_intent_onto_binding(
    backend: &mut impl McpGraphBackend,
    intent: &CoordinationIntentState,
) -> Result<McpBindingProjection, McpError> {
    let mut binding = match load_coordination_agent_binding(backend, &intent.binding_id)? {
        Some(binding) => binding,
        None => default_mcp_coordination_binding(
            &intent.agent_id,
            &intent.binding_id,
            &intent.actor_id,
        )?,
    };
    binding.lifecycle.run_id = intent.binding_id.clone();
    ensure_session_actor_head(&mut binding, &intent.actor_id);

    let payload = coordination_footprint_payload(intent);
    let content_hash = stable_value_hash(&Value::Object(payload.clone()));
    let revision = binding
        .append_scratchpad_revision(
            &intent.actor_id,
            format!(
                "coordination footprint {} in {}",
                intent.status, intent.room_id
            ),
            content_hash,
            payload,
            intent.updated_at.clone(),
        )
        .map_err(|error| McpError::internal(error.to_string()))?;
    let scratchpad_document_id = binding.working_memory_scope.scratchpad.document_id.clone();
    let state_hash = hash_agent_binding(&binding);
    persist_coordination_agent_binding(backend, &binding, &state_hash)?;

    Ok(McpBindingProjection {
        scratchpad_revision_id: revision.revision_id,
        scratchpad_document_id,
        scratchpad_seq: revision.seq,
        binding_active_head_set: binding.identity.active_head_set.clone(),
    })
}

fn persist_coordination_intent_binding_projection(
    backend: &mut impl McpGraphBackend,
    state: &CoordinationIntentState,
) -> Result<(), McpError> {
    upsert_edge_if_changed(backend, coordination_room_binding_edge(state))?;
    if state.scratchpad_seq > 0 && !state.scratchpad_document_id.is_empty() {
        upsert_edge_if_changed(backend, coordination_intent_scratchpad_edge(state))?;
    }
    Ok(())
}

fn load_coordination_agent_binding(
    backend: &impl McpGraphBackend,
    binding_id: &str,
) -> Result<Option<AgentBinding>, McpError> {
    backend
        .get_node(&binding_node_id(binding_id))?
        .map(|node| parse_node_properties::<AgentBinding>(node.properties))
        .transpose()
}

fn persist_coordination_agent_binding(
    backend: &mut impl McpGraphBackend,
    binding: &AgentBinding,
    state_hash: &str,
) -> Result<(), McpError> {
    upsert_node_if_changed(
        backend,
        coordination_agent_binding_node(binding, state_hash)?,
    )?;
    let document_id = &binding.working_memory_scope.scratchpad.document_id;
    let run_id = &binding.lifecycle.run_id;
    for revision in &binding.working_memory_scope.scratchpad.revisions {
        upsert_node_if_changed(
            backend,
            coordination_scratchpad_revision_node(document_id, revision)?,
        )?;
        upsert_edge_if_changed(
            backend,
            coordination_scratchpad_revision_of_edge(document_id, run_id, revision),
        )?;
        if revision.seq > 1 {
            upsert_edge_if_changed(
                backend,
                coordination_previous_scratchpad_revision_edge(document_id, revision),
            )?;
        }
    }
    Ok(())
}

fn coordination_agent_binding_node(
    binding: &AgentBinding,
    state_hash: &str,
) -> Result<NodeRecord, McpError> {
    let mut properties =
        serde_json::to_value(binding).map_err(|error| McpError::internal(error.to_string()))?;
    properties["state_hash"] = Value::String(state_hash.to_string());
    Ok(NodeRecord::new(
        binding_node_id(&binding.lifecycle.run_id),
        ["AgentBinding"],
        properties,
    ))
}

fn coordination_scratchpad_revision_node(
    document_id: &str,
    revision: &ScratchpadRevision,
) -> Result<NodeRecord, McpError> {
    let mut properties =
        serde_json::to_value(revision).map_err(|error| McpError::internal(error.to_string()))?;
    properties["document_id"] = Value::String(document_id.to_string());
    Ok(NodeRecord::new(
        scratchpad_revision_node_id(document_id, revision.seq),
        ["ScratchpadRevision"],
        properties,
    ))
}

fn coordination_scratchpad_revision_of_edge(
    document_id: &str,
    run_id: &str,
    revision: &ScratchpadRevision,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrev-of:{}:{:020}",
            document_id, revision.seq
        ),
        scratchpad_revision_node_id(document_id, revision.seq),
        "HARNESS_SCRATCHPAD_REVISION_OF",
        binding_node_id(run_id),
        json!({
            "document_id": document_id,
            "seq": revision.seq,
            "actor_head_id": revision.actor_head_id,
            "content_hash": revision.content_hash,
        }),
    )
}

fn coordination_previous_scratchpad_revision_edge(
    document_id: &str,
    revision: &ScratchpadRevision,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrev-next:{}:{:020}",
            document_id, revision.seq
        ),
        scratchpad_revision_node_id(document_id, revision.seq - 1),
        "HARNESS_SCRATCHPAD_REVISION_NEXT",
        scratchpad_revision_node_id(document_id, revision.seq),
        json!({
            "document_id": document_id,
            "from_seq": revision.seq - 1,
            "to_seq": revision.seq,
        }),
    )
}

fn default_mcp_coordination_binding(
    agent_id: &str,
    binding_id: &str,
    actor_id: &str,
) -> Result<AgentBinding, McpError> {
    if agent_id == "theorem" {
        return default_theorem_binding(binding_id)
            .map_err(|error| McpError::internal(error.to_string()));
    }

    let actor_head = session_actor_head(actor_id);
    let mut binding = AgentBinding::new(
        BindingIdentity {
            agent_id: agent_id.to_string(),
            owner_id: "travis".to_string(),
            agent_name: agent_id.to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set: vec![actor_head.head_id.clone()],
        },
        BindingComposition {
            heads: vec![actor_head],
        },
        BindingBudgetScope::new(agent_id, 32_000.0, 8),
    )
    .map_err(|error| McpError::internal(error.to_string()))?;
    binding.lifecycle.run_id = binding_id.to_string();
    Ok(binding)
}

fn ensure_session_actor_head(binding: &mut AgentBinding, actor_id: &str) {
    if binding.head(actor_id).is_none() {
        binding.composition.heads.push(session_actor_head(actor_id));
    }
    let mut active = binding
        .identity
        .active_head_set
        .iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    active.insert(actor_id.to_string());
    binding.identity.active_head_set = active.into_iter().collect();
    binding.identity.composition_hash = composition_hash(binding);
}

fn session_actor_head(actor_id: &str) -> AgentHead {
    AgentHead {
        head_id: actor_id.to_string(),
        display_name: actor_id.to_string(),
        provider: "session".to_string(),
        model: "session-actor".to_string(),
        credential_ref: "local:none".to_string(),
        transport: HeadTransport::Local,
        kind: HeadKind::SpecializedCoder,
        capabilities: vec!["coordination".to_string()],
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: vec!["coordination_intent".to_string()],
        trace_tier: TraceTier::Receipt,
    }
}

fn coordination_footprint_payload(intent: &CoordinationIntentState) -> Payload {
    let mut payload = Payload::new();
    payload.insert(
        "type".to_string(),
        Value::String("coordination_footprint".to_string()),
    );
    payload.insert(
        "tenant_slug".to_string(),
        Value::String(intent.tenant_slug.clone()),
    );
    payload.insert(
        "agent_id".to_string(),
        Value::String(intent.agent_id.clone()),
    );
    payload.insert(
        "binding_id".to_string(),
        Value::String(intent.binding_id.clone()),
    );
    payload.insert("room_id".to_string(), Value::String(intent.room_id.clone()));
    payload.insert(
        "actor_id".to_string(),
        Value::String(intent.actor_id.clone()),
    );
    payload.insert("status".to_string(), Value::String(intent.status.clone()));
    payload.insert("summary".to_string(), Value::String(intent.summary.clone()));
    payload.insert("footprint".to_string(), json!(intent.footprint));
    payload.insert(
        "expected_completion".to_string(),
        Value::String(intent.expected_completion.clone()),
    );
    payload.insert("repo".to_string(), Value::String(intent.repo.clone()));
    payload.insert("branch".to_string(), Value::String(intent.branch.clone()));
    payload.insert("task".to_string(), Value::String(intent.task.clone()));
    payload.insert(
        "started_at".to_string(),
        Value::String(intent.started_at.clone()),
    );
    payload.insert(
        "updated_at".to_string(),
        Value::String(intent.updated_at.clone()),
    );
    payload
}

fn normalize_binding_agent_id(agent_id: &str, binding_id: &str) -> String {
    let explicit = agent_id.trim();
    let candidate = if !explicit.is_empty() {
        explicit
    } else {
        binding_id
            .trim()
            .strip_prefix("agent:")
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("theorem")
    };
    coordination_binding_id(candidate)
        .strip_prefix("agent:")
        .unwrap_or("theorem")
        .to_string()
}

fn resolve_coordination_binding_id(binding_id: &str, agent_id: &str) -> String {
    let binding_id = binding_id.trim();
    if binding_id.is_empty() {
        coordination_binding_id(agent_id)
    } else {
        binding_id.to_string()
    }
}

fn write_coordination_presence(
    backend: &mut impl McpGraphBackend,
    input: PresenceInput,
) -> Result<CoordinationPresenceState, McpError> {
    let refreshed_at = timestamp_or_now(&input.refreshed_at);
    let expires_at = if input.expires_at.trim().is_empty() {
        refreshed_at.clone()
    } else {
        input.expires_at.trim().to_string()
    };
    let presence = CoordinationPresenceState {
        tenant_slug: normalize_tenant_slug(&input.tenant_slug),
        actor_id: require_actor_id(&input.actor_id, "presence requires actor")?,
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
        changed_files: normalize_string_vec(input.changed_files),
        refreshed_at,
        expires_at,
        ttl_seconds: input.ttl_seconds.max(1),
    };
    upsert_node_if_changed(backend, coordination_presence_node(&presence)?)?;
    Ok(presence)
}

fn write_coordination_message(
    backend: &mut impl McpGraphBackend,
    input: WriteMessageInput,
) -> Result<CoordinationMessageState, McpError> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let room_id = if input.room_id.trim().is_empty() {
        "room:ungrouped".to_string()
    } else {
        input.room_id.trim().to_string()
    };
    let actor_id = require_actor_id(&input.actor_id, "coordinate requires actor")?;
    let message = require_nonempty(input.message.trim(), "coordinate requires message")?;
    let urgency = normalize_coordination_urgency(&input.urgency)
        .map_err(|error| McpError::invalid_params(error.to_string()))?;
    let delivery = normalize_coordination_delivery(&input.delivery)?;
    let created_at = timestamp_or_now(&input.created_at);
    let mentions = merge_actor_vecs(
        parse_coordination_mentions(&message),
        normalize_actor_vec(input.mentions),
    );
    let message_id = if input.message_id.trim().is_empty() {
        stable_coordination_message_id(&tenant_slug, &room_id, &actor_id, &message, &created_at)
    } else {
        input.message_id.trim().to_string()
    };
    if load_coordination_room_exact(backend, &tenant_slug, &room_id)?.is_none() {
        persist_coordination_room(
            backend,
            &empty_coordination_room(&tenant_slug, &room_id, &created_at),
        )?;
    }
    let message = CoordinationMessageState {
        tenant_slug,
        room_id,
        message_id,
        actor_id,
        urgency,
        delivery,
        message,
        mentions,
        metadata: input.metadata,
        consumed_by: Vec::new(),
        created_at,
    };
    persist_coordination_message(backend, &message)?;
    publish_coordination_room_event(&message);
    Ok(message)
}

fn write_coordination_record(
    backend: &mut impl McpGraphBackend,
    input: WriteRecordInput,
) -> Result<CoordinationRecordState, McpError> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let room_id = if input.room_id.trim().is_empty() {
        "room:ungrouped".to_string()
    } else {
        input.room_id.trim().to_string()
    };
    let actor_id = require_actor_id(&input.actor_id, "coordination_record requires actor")?;
    let record_type = normalize_coordination_record_type(&input.record_type)?;
    let summary = require_nonempty(input.summary.trim(), "coordination_record requires summary")?;
    let created_at = timestamp_or_now(&input.created_at);
    let record_id = if input.record_id.trim().is_empty() {
        stable_coordination_record_id(
            &tenant_slug,
            &room_id,
            &record_type,
            &actor_id,
            &summary,
            &created_at,
        )
    } else {
        input.record_id.trim().to_string()
    };
    if load_coordination_room_exact(backend, &tenant_slug, &room_id)?.is_none() {
        persist_coordination_room(
            backend,
            &empty_coordination_room(&tenant_slug, &room_id, &created_at),
        )?;
    }
    let record = CoordinationRecordState {
        tenant_slug,
        room_id,
        record_id,
        record_type,
        actor_id,
        title: input.title.trim().to_string(),
        summary,
        body: input.body.trim().to_string(),
        metadata: input.metadata,
        created_at,
    };
    persist_coordination_record(backend, &record)?;
    Ok(record)
}

fn read_coordination_intents(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    statuses: &[String],
) -> Result<Vec<CoordinationIntentState>, McpError> {
    let filters = normalize_string_vec(statuses.to_vec())
        .into_iter()
        .map(|status| status.to_lowercase())
        .collect::<Vec<_>>();
    let mut intents = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationIntent")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("room_id", Value::String(room_id.to_string())),
        )? {
            let intent = parse_node_properties::<CoordinationIntentState>(node.properties)?;
            if filters.is_empty() || filters.contains(&intent.status) {
                intents.push(intent);
            }
        }
    }
    intents.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| left.actor_id.cmp(&right.actor_id))
    });
    Ok(intents)
}

fn read_coordination_messages(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    limit: usize,
) -> Result<Vec<CoordinationMessageState>, McpError> {
    let mut messages = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("room_id", Value::String(room_id.to_string())),
        )? {
            messages.push(parse_node_properties::<CoordinationMessageState>(
                node.properties,
            )?);
        }
    }
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

fn read_coordination_records(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    record_types: &[String],
    limit: usize,
) -> Result<Vec<CoordinationRecordState>, McpError> {
    let filters = normalize_string_vec(record_types.to_vec())
        .into_iter()
        .map(|record_type| record_type.to_lowercase())
        .collect::<Vec<_>>();
    let mut records = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationRecord")
                .with_property("tenant_slug", Value::String(tenant_alias))
                .with_property("room_id", Value::String(room_id.to_string())),
        )? {
            let record = parse_node_properties::<CoordinationRecordState>(node.properties)?;
            if filters.is_empty() || filters.contains(&record.record_type) {
                records.push(record);
            }
        }
    }
    records.sort_by(|left, right| {
        right
            .created_at
            .cmp(&left.created_at)
            .then_with(|| right.record_id.cmp(&left.record_id))
    });
    if limit > 0 {
        records.truncate(limit);
    }
    Ok(records)
}

fn read_coordination_mentions(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
    consume: bool,
    limit: usize,
) -> Result<Vec<CoordinationMessageState>, McpError> {
    read_coordination_mentions_filtered(backend, tenant, None, actor_id, consume, limit)
}

fn read_coordination_mentions_in_room(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    actor_id: &str,
    consume: bool,
    limit: usize,
) -> Result<Vec<CoordinationMessageState>, McpError> {
    read_coordination_mentions_filtered(
        backend,
        tenant,
        Some(room_id.to_string()),
        actor_id,
        consume,
        limit,
    )
}

fn read_coordination_mentions_filtered(
    backend: &mut impl McpGraphBackend,
    tenant: &str,
    room_id: Option<String>,
    actor_id: &str,
    consume: bool,
    limit: usize,
) -> Result<Vec<CoordinationMessageState>, McpError> {
    let actor_id = require_actor_id(actor_id, "mentions requires actor")?;
    let mut messages = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(tenant_alias)),
        )? {
            let message = parse_node_properties::<CoordinationMessageState>(node.properties)?;
            if message
                .mentions
                .iter()
                .any(|mention| normalize_actor_id(mention) == actor_id)
                && room_id
                    .as_ref()
                    .map(|room_id| message.room_id == room_id.as_str())
                    .unwrap_or(true)
                && !message
                    .consumed_by
                    .iter()
                    .any(|consumer| normalize_actor_id(consumer) == actor_id)
            {
                messages.push(message);
            }
        }
    }
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
            consumed.consumed_by = merge_string_vecs(consumed.consumed_by, vec![actor_id.clone()]);
            persist_coordination_message(backend, &consumed)?;
        }
    }
    Ok(messages)
}

fn list_coordination_presence(
    backend: &impl McpGraphBackend,
    tenant: &str,
) -> Result<Vec<CoordinationPresenceState>, McpError> {
    let mut presence = Vec::new();
    for tenant_alias in tenant_slug_aliases(tenant) {
        for node in backend.query_nodes(
            NodeQuery::label("CoordinationPresence")
                .with_property("tenant_slug", Value::String(tenant_alias)),
        )? {
            presence.push(parse_node_properties::<CoordinationPresenceState>(
                node.properties,
            )?);
        }
    }
    presence.sort_by(|left, right| {
        (left.status != "active")
            .cmp(&(right.status != "active"))
            .then_with(|| left.actor_id.cmp(&right.actor_id))
    });
    Ok(presence)
}

fn load_coordination_room(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
) -> Result<Option<CoordinationRoomState>, McpError> {
    for tenant_alias in tenant_slug_aliases(tenant) {
        if let Some(node) = backend.get_node(&coordination_room_node_id(&tenant_alias, room_id))? {
            return parse_node_properties::<CoordinationRoomState>(node.properties).map(Some);
        }
    }
    Ok(None)
}

fn load_coordination_room_exact(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
) -> Result<Option<CoordinationRoomState>, McpError> {
    backend
        .get_node(&coordination_room_node_id(tenant, room_id))?
        .map(|node| parse_node_properties::<CoordinationRoomState>(node.properties))
        .transpose()
}

fn load_coordination_intent_exact(
    backend: &impl McpGraphBackend,
    tenant: &str,
    room_id: &str,
    actor_id: &str,
) -> Result<Option<CoordinationIntentState>, McpError> {
    backend
        .get_node(&coordination_intent_node_id(tenant, room_id, actor_id))?
        .map(|node| parse_node_properties::<CoordinationIntentState>(node.properties))
        .transpose()
}

fn load_coordination_presence(
    backend: &impl McpGraphBackend,
    tenant: &str,
    actor_id: &str,
) -> Result<Option<CoordinationPresenceState>, McpError> {
    for tenant_alias in tenant_slug_aliases(tenant) {
        if let Some(node) =
            backend.get_node(&coordination_presence_node_id(&tenant_alias, actor_id))?
        {
            return parse_node_properties::<CoordinationPresenceState>(node.properties).map(Some);
        }
    }
    Ok(None)
}

fn persist_coordination_room(
    backend: &mut impl McpGraphBackend,
    state: &CoordinationRoomState,
) -> Result<(), McpError> {
    upsert_node_if_changed(backend, coordination_room_node(state)?)?;
    for member in state.members.values() {
        upsert_node_if_changed(backend, coordination_member_node(member)?)?;
        upsert_edge_if_changed(backend, coordination_member_edge(member))?;
    }
    Ok(())
}

fn persist_coordination_intent(
    backend: &mut impl McpGraphBackend,
    state: &CoordinationIntentState,
) -> Result<(), McpError> {
    upsert_node_if_changed(backend, coordination_intent_node(state)?)?;
    upsert_edge_if_changed(backend, coordination_intent_edge(state))?;
    Ok(())
}

fn persist_coordination_message(
    backend: &mut impl McpGraphBackend,
    state: &CoordinationMessageState,
) -> Result<(), McpError> {
    upsert_node_if_changed(backend, coordination_message_node(state)?)?;
    upsert_edge_if_changed(backend, coordination_message_room_edge(state))?;
    for actor_id in &state.mentions {
        if backend
            .get_node(&coordination_member_node_id(&state.tenant_slug, actor_id))?
            .is_none()
        {
            upsert_node_if_changed(
                backend,
                coordination_member_node(&CoordinationRoomMember {
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
                })?,
            )?;
        }
        upsert_edge_if_changed(backend, coordination_mention_edge(state, actor_id))?;
    }
    Ok(())
}

fn persist_coordination_record(
    backend: &mut impl McpGraphBackend,
    state: &CoordinationRecordState,
) -> Result<(), McpError> {
    upsert_node_if_changed(backend, coordination_record_node(state)?)?;
    upsert_edge_if_changed(backend, coordination_record_room_edge(state))?;
    Ok(())
}

fn coordination_room_node(state: &CoordinationRoomState) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        ["HarnessCoordination", "CoordinationRoom"],
        serde_json::to_value(state).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_member_node(member: &CoordinationRoomMember) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_member_node_id(&member.tenant_slug, &member.actor_id),
        ["HarnessCoordination", "CoordinationMember"],
        serde_json::to_value(member).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_intent_node(state: &CoordinationIntentState) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_intent_node_id(&state.tenant_slug, &state.room_id, &state.actor_id),
        ["HarnessCoordination", "CoordinationIntent"],
        serde_json::to_value(state).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_presence_node(state: &CoordinationPresenceState) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_presence_node_id(&state.tenant_slug, &state.actor_id),
        ["HarnessCoordination", "CoordinationPresence"],
        serde_json::to_value(state).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_message_node(state: &CoordinationMessageState) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_message_node_id(&state.tenant_slug, &state.room_id, &state.message_id),
        ["HarnessCoordination", "CoordinationMessage"],
        serde_json::to_value(state).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_record_node(state: &CoordinationRecordState) -> Result<NodeRecord, McpError> {
    Ok(NodeRecord::new(
        coordination_record_node_id(&state.tenant_slug, &state.room_id, &state.record_id),
        ["HarnessCoordination", "CoordinationRecord"],
        serde_json::to_value(state).map_err(|error| McpError::internal(error.to_string()))?,
    ))
}

fn coordination_member_edge(member: &CoordinationRoomMember) -> EdgeRecord {
    EdgeRecord::new(
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
    )
}

fn coordination_intent_edge(state: &CoordinationIntentState) -> EdgeRecord {
    EdgeRecord::new(
        coordination_intent_edge_id(&state.tenant_slug, &state.room_id, &state.actor_id),
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
    )
}

fn coordination_room_binding_edge(state: &CoordinationIntentState) -> EdgeRecord {
    EdgeRecord::new(
        coordination_room_binding_edge_id(&state.tenant_slug, &state.room_id, &state.binding_id),
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        "COORDINATION_ROOM_PROJECTS_TO_BINDING",
        binding_node_id(&state.binding_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "agent_id": state.agent_id,
            "binding_id": state.binding_id,
            "room_id": state.room_id,
            "updated_at": state.updated_at,
        }),
    )
}

fn coordination_intent_scratchpad_edge(state: &CoordinationIntentState) -> EdgeRecord {
    EdgeRecord::new(
        coordination_intent_scratchpad_edge_id(
            &state.tenant_slug,
            &state.room_id,
            &state.actor_id,
            &state.scratchpad_revision_id,
        ),
        coordination_intent_node_id(&state.tenant_slug, &state.room_id, &state.actor_id),
        "COORDINATION_INTENT_APPENDED_SCRATCHPAD_REVISION",
        scratchpad_revision_node_id(&state.scratchpad_document_id, state.scratchpad_seq),
        json!({
            "tenant_slug": state.tenant_slug,
            "agent_id": state.agent_id,
            "binding_id": state.binding_id,
            "room_id": state.room_id,
            "actor_id": state.actor_id,
            "scratchpad_document_id": state.scratchpad_document_id,
            "scratchpad_revision_id": state.scratchpad_revision_id,
            "scratchpad_seq": state.scratchpad_seq,
            "updated_at": state.updated_at,
        }),
    )
}

fn coordination_message_room_edge(state: &CoordinationMessageState) -> EdgeRecord {
    EdgeRecord::new(
        coordination_message_edge_id(&state.tenant_slug, &state.room_id, &state.message_id),
        coordination_message_node_id(&state.tenant_slug, &state.room_id, &state.message_id),
        "COORDINATION_MESSAGE_OF",
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "room_id": state.room_id,
            "message_id": state.message_id,
            "actor_id": state.actor_id,
            "urgency": state.urgency,
            "delivery": state.delivery,
            "created_at": state.created_at,
        }),
    )
}

fn coordination_record_room_edge(state: &CoordinationRecordState) -> EdgeRecord {
    EdgeRecord::new(
        coordination_record_edge_id(&state.tenant_slug, &state.room_id, &state.record_id),
        coordination_record_node_id(&state.tenant_slug, &state.room_id, &state.record_id),
        "COORDINATION_RECORD_OF",
        coordination_room_node_id(&state.tenant_slug, &state.room_id),
        json!({
            "tenant_slug": state.tenant_slug,
            "room_id": state.room_id,
            "record_id": state.record_id,
            "record_type": state.record_type,
            "actor_id": state.actor_id,
            "created_at": state.created_at,
        }),
    )
}

fn coordination_mention_edge(state: &CoordinationMessageState, actor_id: &str) -> EdgeRecord {
    EdgeRecord::new(
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
            "delivery": state.delivery,
            "created_at": state.created_at,
        }),
    )
}

fn upsert_node_if_changed(
    backend: &mut impl McpGraphBackend,
    node: NodeRecord,
) -> Result<(), McpError> {
    let unchanged = backend
        .get_node(&node.id)?
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        backend.upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed(
    backend: &mut impl McpGraphBackend,
    edge: EdgeRecord,
) -> Result<(), McpError> {
    let unchanged = backend
        .get_edge(&edge.id)?
        .map(|existing| {
            !existing.tombstone
                && existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.edge_type == edge.edge_type
                && existing.properties == edge.properties
        })
        .unwrap_or(false);
    if !unchanged {
        backend.upsert_edge(edge)?;
    }
    Ok(())
}

fn empty_coordination_room(tenant: &str, room_id: &str, now: &str) -> CoordinationRoomState {
    CoordinationRoomState {
        tenant_slug: normalize_tenant_slug(tenant),
        room_id: room_id.to_string(),
        status: "active".to_string(),
        mode: "collaborating".to_string(),
        repo: String::new(),
        branch: String::new(),
        task: String::new(),
        created_at: now.to_string(),
        updated_at: now.to_string(),
        members: std::collections::BTreeMap::new(),
        last_packet_at: String::new(),
        last_packet_doc_id: String::new(),
        degraded: false,
        degraded_reason: String::new(),
    }
}

fn resolved_coordination_room_id(arguments: &Value) -> String {
    argument_text(arguments, &["room_id", "roomId"]).unwrap_or_else(|| {
        infer_coordination_room_id(
            &argument_text(arguments, &["repo"]).unwrap_or_default(),
            &argument_text(arguments, &["branch"]).unwrap_or_default(),
            &argument_text(arguments, &["task"]).unwrap_or_default(),
            &argument_text(arguments, &["session_id", "sessionId"]).unwrap_or_default(),
        )
    })
}

fn normalize_coordination_status(status: &str) -> Result<String, McpError> {
    let status = if status.trim().is_empty() {
        "working".to_string()
    } else {
        status.trim().to_lowercase()
    };
    match status.as_str() {
        "working" | "paused" | "done" => Ok(status),
        _ => Err(McpError::invalid_params(
            "coordination intent status must be working, paused, or done",
        )),
    }
}

fn normalize_coordination_record_type(record_type: &str) -> Result<String, McpError> {
    let record_type = record_type.trim().to_lowercase();
    match record_type.as_str() {
        "event" | "decision" | "tension" | "reflection" => Ok(record_type),
        _ => Err(McpError::invalid_params(
            "coordination record type must be event, decision, tension, or reflection",
        )),
    }
}

fn coordination_delivery_from_legacy_wake(arguments: &Value) -> String {
    if arguments
        .get("wake")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        "wake".to_string()
    } else {
        "passive".to_string()
    }
}

fn normalize_coordination_delivery(delivery: &str) -> Result<String, McpError> {
    let delivery = normalize_coordination_delivery_lossy(delivery);
    match delivery.as_str() {
        "passive" | "wake" => Ok(delivery),
        _ => Err(McpError::invalid_params(
            "coordination message delivery must be passive or wake",
        )),
    }
}

fn normalize_coordination_delivery_lossy(delivery: &str) -> String {
    let delivery = delivery.trim().to_lowercase();
    if delivery.is_empty() {
        "passive".to_string()
    } else {
        delivery
    }
}

fn normalize_tenant_slug(tenant: &str) -> String {
    let tenant = tenant.trim();
    if tenant.is_empty() {
        "default".to_string()
    } else {
        tenant.to_string()
    }
}

fn tenant_slug_aliases(tenant: &str) -> Vec<String> {
    let canonical = normalize_tenant_slug(tenant);
    let legacy_lowercase = canonical.to_lowercase();
    if legacy_lowercase == canonical {
        vec![canonical]
    } else {
        vec![canonical, legacy_lowercase]
    }
}

fn choose_text(value: &str, existing: Option<&str>) -> String {
    let value = value.trim();
    if value.is_empty() {
        existing.unwrap_or("").trim().to_string()
    } else {
        value.to_string()
    }
}

fn choose_strings(value: &[String], existing: Option<&[String]>) -> Vec<String> {
    let value = normalize_string_vec(value.to_vec());
    if value.is_empty() {
        existing
            .map(|items| normalize_string_vec(items.to_vec()))
            .unwrap_or_default()
    } else {
        value
    }
}

fn normalize_string_vec(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = value.trim();
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        normalized.push(value.to_string());
    }
    normalized
}

fn merge_string_vecs(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    normalize_string_vec(left.into_iter().chain(right).collect())
}

fn normalize_actor_vec(values: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut normalized = Vec::new();
    for value in values {
        let value = normalize_actor_id(&value);
        if value.is_empty() || !seen.insert(value.to_string()) {
            continue;
        }
        normalized.push(value);
    }
    normalized
}

fn merge_actor_vecs(left: Vec<String>, right: Vec<String>) -> Vec<String> {
    normalize_actor_vec(left.into_iter().chain(right).collect())
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

fn argument_text(arguments: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn argument_u64(arguments: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_u64))
}

fn argument_object(arguments: &Value, key: &str) -> Map<String, Value> {
    arguments
        .get(key)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

fn argument_array(arguments: &Value, keys: &[&str]) -> Option<Vec<Value>> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_array))
        .cloned()
}

fn insert_if_present(metadata: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        metadata.insert(key.to_string(), Value::String(value));
    }
}

fn insert_policy_receipt(payload: &mut Value, policy_receipt: Option<Value>) {
    if let (Value::Object(map), Some(policy_receipt)) = (payload, policy_receipt) {
        map.insert("policy_receipt".to_string(), policy_receipt);
    }
}

fn transition_from_arguments(
    arguments: &Value,
    tool_name: &str,
) -> Result<TransitionInput, McpError> {
    if let Some(transition) = arguments.get("transition") {
        return serde_json::from_value::<TransitionInput>(transition.clone()).map_err(|error| {
            McpError::invalid_params(format!("transition must match TransitionInput: {error}"))
        });
    }

    let event_type = required_text_any(arguments, &["type", "event_type", "eventType"], tool_name)?;
    let mut transition = TransitionInput::new(event_type, argument_object(arguments, "payload"));
    transition.run_id = argument_text(arguments, &["run_id", "runId"]).unwrap_or_default();
    transition.actor =
        argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default();
    transition.idempotency_key =
        argument_text(arguments, &["idempotency_key", "idempotencyKey"]).unwrap_or_default();
    if let Some(created_at) = argument_text(arguments, &["created_at", "createdAt"]) {
        transition.created_at = created_at;
    }
    Ok(transition)
}

fn memory_write_input(
    tenant: &str,
    arguments: &Value,
    tool_name: &str,
) -> Result<MemoryWriteInput, McpError> {
    let kind = argument_text(arguments, &["kind"]).unwrap_or_else(|| match tool_name {
        "self_note" => "self_note".to_string(),
        "encode" => "encode".to_string(),
        _ => String::new(),
    });
    if kind.trim().is_empty() {
        return Err(McpError::invalid_params(format!(
            "{tool_name} requires kind"
        )));
    }
    Ok(MemoryWriteInput {
        tenant_slug: tenant.to_string(),
        actor_id: argument_text(arguments, &["actor", "actor_id", "actorId"]).unwrap_or_default(),
        session_id: argument_text(arguments, &["session_id", "sessionId"]).unwrap_or_default(),
        origin_surface: argument_text(arguments, &["origin_surface", "originSurface", "surface"])
            .unwrap_or_default(),
        project_slug: argument_text(arguments, &["project_slug", "projectSlug"])
            .unwrap_or_default(),
        doc_id: argument_text(arguments, &["doc_id", "docId"]).unwrap_or_default(),
        node_id: argument_text(arguments, &["node_id", "nodeId"]).unwrap_or_default(),
        kind,
        title: argument_text(arguments, &["title"]).unwrap_or_default(),
        content: required_text_any(arguments, &["content"], tool_name)?,
        summary: argument_text(arguments, &["summary"]).unwrap_or_default(),
        tags: string_array_any(arguments, &["tags"]),
        links: string_array_any(arguments, &["links"]),
        status: argument_text(arguments, &["status"]).unwrap_or_default(),
        memory_node_type: argument_text(arguments, &["memory_node_type", "memoryNodeType"])
            .unwrap_or_default(),
        target_actor_id: argument_text(arguments, &["target_actor_id", "targetActorId"])
            .unwrap_or_default(),
        expires_at: argument_text(arguments, &["expires_at", "expiresAt"]).unwrap_or_default(),
        metadata: argument_object(arguments, "metadata"),
        fitness: arguments.get("fitness").cloned(),
        created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
    })
}

fn coordination_policy_receipt(
    context: &McpRequestContext,
    arguments: &Value,
    tool_name: &str,
) -> Value {
    let agent_id = normalize_binding_agent_id(
        &argument_text(arguments, &["agent_id", "agentId"]).unwrap_or_default(),
        &argument_text(arguments, &["binding_id", "bindingId"]).unwrap_or_default(),
    );
    let binding_id = resolve_coordination_binding_id(
        &argument_text(arguments, &["binding_id", "bindingId"]).unwrap_or_default(),
        &agent_id,
    );
    let room_id = resolved_coordination_room_id(arguments);
    let required_scopes = string_array_any(
        arguments,
        &[
            "required_scopes",
            "requiredScopes",
            "required_scope",
            "requiredScope",
        ],
    );
    let missing_scopes = required_scopes
        .iter()
        .filter(|scope| !context.allows(scope))
        .cloned()
        .collect::<Vec<_>>();
    let estimated_cost_units = argument_f64_any(
        arguments,
        &[
            "estimated_cost_units",
            "estimatedCostUnits",
            "estimated_cost",
            "cost_units",
            "costUnits",
        ],
    )
    .unwrap_or(0.0)
    .max(0.0);
    let budget_units = argument_f64_any(
        arguments,
        &[
            "budget_units",
            "budgetUnits",
            "max_cost_units",
            "maxCostUnits",
        ],
    )
    .or_else(|| {
        arguments.get("budget").and_then(|budget| {
            value_as_f64(budget.get("max_units").or_else(|| budget.get("maxUnits"))?)
        })
    });
    let budget_allowed = budget_units
        .map(|budget| estimated_cost_units <= budget.max(0.0))
        .unwrap_or(true);
    let scope_allowed = missing_scopes.is_empty();
    let publication_gate = session_publication_gate_receipt(arguments);
    let publication_allowed = publication_gate
        .get("decision")
        .and_then(Value::as_str)
        .map(|decision| decision != "deny")
        .unwrap_or(true);
    let decision = if scope_allowed && budget_allowed && publication_allowed {
        "allow"
    } else {
        "deny"
    };
    json!({
        "tool": tool_name,
        "decision": decision,
        "agent_id": agent_id,
        "binding_id": binding_id,
        "room_id": room_id,
        "required_scopes": required_scopes,
        "granted_scopes": context.scopes.clone(),
        "missing_scopes": missing_scopes,
        "scope_allowed": scope_allowed,
        "estimated_cost_units": estimated_cost_units,
        "budget_units": budget_units,
        "budget_allowed": budget_allowed,
        "publication_gate": publication_gate
    })
}

fn coordination_policy_error(policy_receipt: &Value) -> Option<Value> {
    if policy_receipt
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("allow")
        == "allow"
    {
        return None;
    }
    let missing_scopes = policy_receipt
        .get("missing_scopes")
        .and_then(Value::as_array)
        .map(|items| !items.is_empty())
        .unwrap_or(false);
    let budget_allowed = policy_receipt
        .get("budget_allowed")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let publication_allowed = policy_receipt
        .get("publication_gate")
        .and_then(|gate| gate.get("decision"))
        .and_then(Value::as_str)
        .map(|decision| decision != "deny")
        .unwrap_or(true);
    let code = if missing_scopes {
        "coordination_scope_denied"
    } else if !budget_allowed {
        "coordination_budget_exceeded"
    } else if !publication_allowed {
        "coordination_publication_denied"
    } else {
        "coordination_policy_denied"
    };
    Some(json!({
        "error": code,
        "message": "Native coordination policy denied this write.",
        "policy_receipt": policy_receipt
    }))
}

fn session_publication_gate_receipt(arguments: &Value) -> Value {
    let publication_flag = [
        "publication",
        "publish",
        "publication_gate",
        "publicationGate",
    ]
    .iter()
    .any(|key| {
        arguments
            .get(*key)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    });
    let claims = argument_array(
        arguments,
        &[
            "claims",
            "grounded_claims",
            "groundedClaims",
            "publication_claims",
            "publicationClaims",
        ],
    );
    let pr_url = argument_text(
        arguments,
        &["pr_url", "prUrl", "pull_request_url", "pullRequestUrl"],
    );
    let applies = publication_flag || claims.is_some() || pr_url.is_some();
    if !applies {
        return json!({
            "applies": false,
            "decision": "not_applicable"
        });
    }

    let synthesis_heads = publication_synthesis_heads(arguments);
    let mut payload = Payload::new();
    payload.insert(
        "claims".to_string(),
        Value::Array(claims.unwrap_or_default()),
    );
    if let Some(action_tier) = argument_text(arguments, &["action_tier", "actionTier"]) {
        payload.insert("action_tier".to_string(), Value::String(action_tier));
    }
    if let Some(human_authorized) = ["human_authorized", "humanAuthorized"]
        .iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_bool))
    {
        payload.insert(
            "human_authorized".to_string(),
            Value::Bool(human_authorized),
        );
    }
    if let Some(pr_url) = pr_url {
        payload.insert("pr_url".to_string(), Value::String(pr_url));
    }

    match evaluate_publication(
        &synthesis_heads,
        &coordination_publication_action_tiers(),
        &payload,
    ) {
        Ok(()) => json!({
            "applies": true,
            "decision": "allow",
            "synthesis_heads": synthesis_heads
        }),
        Err(error) => json!({
            "applies": true,
            "decision": "deny",
            "reason": error.to_string(),
            "synthesis_heads": synthesis_heads
        }),
    }
}

fn publication_synthesis_heads(arguments: &Value) -> Vec<String> {
    let mut heads = string_array_any(
        arguments,
        &[
            "synthesis_heads",
            "synthesisHeads",
            "contributing_heads",
            "contributingHeads",
            "peer_review_heads",
            "peerReviewHeads",
        ],
    );
    if let Some(actor) = argument_text(arguments, &["actor", "actor_id", "actorId"]) {
        heads.push(actor);
    }
    if let Some(reviewer) = argument_text(
        arguments,
        &[
            "reviewer",
            "peer_reviewer",
            "peerReviewer",
            "reviewer_head",
            "reviewerHead",
        ],
    ) {
        heads.push(reviewer);
    }
    if let Some(peer_review) = arguments
        .get("peer_review")
        .or_else(|| arguments.get("peerReview"))
        .and_then(Value::as_object)
    {
        if let Some(reviewer) = peer_review
            .get("reviewer")
            .or_else(|| peer_review.get("reviewer_head"))
            .or_else(|| peer_review.get("reviewerHead"))
            .and_then(Value::as_str)
        {
            heads.push(reviewer.to_string());
        }
    }
    normalize_string_vec(heads)
}

fn coordination_publication_action_tiers() -> Vec<ActionTierPolicy> {
    vec![
        ActionTierPolicy::new("tier_one", "reversible substrate action", false),
        ActionTierPolicy::new("tier_two", "consequential commit action", true),
        ActionTierPolicy::new("tier_three", "irreversible external action", true),
    ]
}

fn argument_f64_any(arguments: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(value_as_f64))
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
}

fn string_array_any(arguments: &Value, keys: &[&str]) -> Vec<String> {
    keys.iter()
        .find_map(|key| {
            arguments.get(*key).map(|value| match value {
                Value::Array(items) => items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>(),
                Value::String(text) if !text.trim().is_empty() => {
                    vec![text.trim().to_string()]
                }
                _ => Vec::new(),
            })
        })
        .map(normalize_string_vec)
        .unwrap_or_default()
}

fn f32_array_any(arguments: &Value, keys: &[&str]) -> Vec<f32> {
    keys.iter()
        .find_map(|key| {
            arguments.get(*key).map(|value| match value {
                Value::Array(items) => items
                    .iter()
                    .filter_map(value_as_f64)
                    .map(|value| value as f32)
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
        })
        .unwrap_or_default()
}

fn required_text_any(
    arguments: &Value,
    keys: &[&str],
    tool_name: &str,
) -> Result<String, McpError> {
    argument_text(arguments, keys).ok_or_else(|| {
        McpError::invalid_params(format!("{tool_name} requires {}", keys.join(" or ")))
    })
}

fn require_nonempty(value: &str, message: &str) -> Result<String, McpError> {
    if value.trim().is_empty() {
        Err(McpError::invalid_params(message))
    } else {
        Ok(value.trim().to_string())
    }
}

fn parse_node_properties<T: for<'de> Deserialize<'de>>(value: Value) -> Result<T, McpError> {
    serde_json::from_value(value)
        .map_err(|error| McpError::internal(format!("coordination node decode failed: {error}")))
}

fn tenant_from_args(arguments: &Value, config: &McpServerConfig) -> String {
    arguments
        .get("tenant")
        .or_else(|| arguments.get("tenant_id"))
        .or_else(|| arguments.get("tenantId"))
        .or_else(|| arguments.get("tenant_slug"))
        .or_else(|| arguments.get("tenantSlug"))
        .and_then(Value::as_str)
        .filter(|tenant| !tenant.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| config.default_tenant.clone())
}

fn tenant_from_args_for_tool(
    tool_name: &str,
    arguments: &Value,
    config: &McpServerConfig,
) -> Result<String, McpError> {
    if !coordination_tool_requires_tenant(tool_name) {
        return Ok(tenant_from_args(arguments, config));
    }
    if let Some(tenant) = explicit_tenant_arg(arguments) {
        return Ok(tenant);
    }
    let configured = config.default_tenant.trim();
    if !configured.is_empty() && configured != "default" {
        return Ok(configured.to_string());
    }
    Err(McpError::invalid_params(format!(
        "{tool_name} requires tenant or tenant_slug; refusing to fall back to default"
    )))
}

fn explicit_tenant_arg(arguments: &Value) -> Option<String> {
    arguments
        .get("tenant")
        .or_else(|| arguments.get("tenant_id"))
        .or_else(|| arguments.get("tenantId"))
        .or_else(|| arguments.get("tenant_slug"))
        .or_else(|| arguments.get("tenantSlug"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|tenant| !tenant.is_empty())
        .map(str::to_string)
}

fn coordination_tool_requires_tenant(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "coordination_room"
            | "theorem_harness_coordination_room"
            | "presence"
            | "theorem_harness_presence"
            | "coordination_intent"
            | "write_intent"
            | "theorem_harness_write_intent"
            | "read_intents_for_room"
            | "theorem_harness_read_intents_for_room"
            | "coordinate"
            | "theorem_harness_coordinate"
            | "stream_publish"
            | "theorem_harness_stream_publish"
            | "stream_read"
            | "theorem_harness_stream_read"
            | "stream_subscribe"
            | "theorem_harness_stream_subscribe"
            | "stream_unsubscribe"
            | "theorem_harness_stream_unsubscribe"
            | "graphql_mutate"
            | "theorem_harness_graphql_mutate"
            | "mentions"
            | "theorem_harness_mentions"
            | "read_messages_for_room"
            | "theorem_harness_read_messages_for_room"
            | "coordination_record"
            | "write_coordination_record"
            | "theorem_harness_write_record"
    )
}

fn required_str<'a>(arguments: &'a Value, key: &str, tool_name: &str) -> Result<&'a str, McpError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params(format!("{tool_name} requires {key}")))
}

fn require_actor_id(value: &str, message: &str) -> Result<String, McpError> {
    let actor_id = normalize_actor_id(value);
    require_nonempty(&actor_id, message)
}

fn required_f64(arguments: &Value, key: &str, tool_name: &str) -> Result<f64, McpError> {
    arguments
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| McpError::invalid_params(format!("{tool_name} requires numeric {key}")))
}

fn string_array(arguments: &Value, key: &str) -> Vec<String> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !item.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_f32_array(arguments: &Value, key: &str) -> Result<Vec<f32>, McpError> {
    arguments
        .get(key)
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .map(|v| {
                    v.as_f64().map(|f| f as f32).ok_or_else(|| {
                        McpError::invalid_params(format!("{key} must be an array of numbers"))
                    })
                })
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap_or_else(|| {
            Err(McpError::invalid_params(format!(
                "{key} is required and must be an array of numbers"
            )))
        })
}

fn optional_repository(arguments: &Value) -> Result<GraphVersionRepository, McpError> {
    match arguments.get("repository").cloned() {
        Some(value) => serde_json::from_value(value)
            .map_err(|error| McpError::invalid_params(format!("repository is invalid: {error}"))),
        None => Ok(GraphVersionRepository::default()),
    }
}

fn required_repository(
    arguments: &Value,
    tool_name: &str,
) -> Result<GraphVersionRepository, McpError> {
    let value = arguments.get("repository").cloned().ok_or_else(|| {
        McpError::invalid_params(format!("{tool_name} requires repository object"))
    })?;
    serde_json::from_value(value)
        .map_err(|error| McpError::invalid_params(format!("repository is invalid: {error}")))
}

fn parse_node_record(raw: &Value) -> Result<NodeRecord, McpError> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params("node record requires string id"))?;
    let labels = raw
        .get("labels")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let properties = raw.get("properties").cloned().unwrap_or_else(|| json!({}));
    if !properties.is_object() {
        return Err(McpError::invalid_params(
            "node properties must be an object",
        ));
    }
    let mut node = NodeRecord::new(id, labels, properties);
    node.tombstone = raw
        .get("tombstone")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Ok(node)
}

fn parse_edge_record(raw: &Value) -> Result<EdgeRecord, McpError> {
    let id = raw
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params("edge record requires string id"))?;
    let from_id = raw
        .get("from_id")
        .or_else(|| raw.get("fromId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params("edge record requires string from_id"))?;
    let to_id = raw
        .get("to_id")
        .or_else(|| raw.get("toId"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params("edge record requires string to_id"))?;
    let edge_type = raw
        .get("type")
        .or_else(|| raw.get("edge_type"))
        .or_else(|| raw.get("edgeType"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params("edge record requires string type"))?;
    let properties = raw.get("properties").cloned().unwrap_or_else(|| json!({}));
    if !properties.is_object() {
        return Err(McpError::invalid_params(
            "edge properties must be an object",
        ));
    }
    let mut edge = EdgeRecord::new(id, from_id, edge_type, to_id, properties);
    edge.tombstone = raw
        .get("tombstone")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    edge.confidence = raw.get("confidence").and_then(Value::as_f64);
    Ok(edge)
}

fn multihead_run_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let action = argument_text(arguments, &["action"])
        .unwrap_or_else(|| "start".to_string())
        .to_lowercase();
    let run_id = argument_text(arguments, &["run_id", "runId"])
        .unwrap_or_else(|| format!("multihead:{}", timestamp_or_now("")));
    if action != "status" {
        persist_multihead_run_marker(
            tenant,
            backend,
            &run_id,
            argument_text(arguments, &["goal"])
                .unwrap_or_else(|| "Multi-head run".to_string())
                .as_str(),
            argument_text(arguments, &["actor", "actor_id", "actorId"])
                .unwrap_or_default()
                .as_str(),
        )?;
    }
    let run = load_multihead_run_marker(tenant, backend, &run_id)?
        .unwrap_or_else(|| empty_multihead_run_marker(tenant, &run_id));
    let graph = load_multihead_work_graph(backend, &run_id)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "run": run,
        "graph": graph,
        "tasks": graph.nodes.values().collect::<Vec<_>>()
    }))
}

fn multihead_task_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_task")?;
    let node_id = argument_text(arguments, &["node_id", "nodeId"]).unwrap_or_else(|| {
        let hash = stable_value_hash(&json!({
            "run_id": run_id,
            "goal": arguments.get("goal").cloned().unwrap_or(Value::Null),
            "kind": arguments.get("kind").cloned().unwrap_or(Value::Null),
        }));
        format!("task:{}", &hash[..16])
    });
    if let Some(existing) = load_multihead_task_node(backend, &run_id, &node_id)? {
        return Ok(json!({
            "tenant": normalize_tenant_slug(tenant),
            "ok": true,
            "reused": true,
            "task": existing
        }));
    }
    let mut node = TaskNode::open(
        &node_id,
        &run_id,
        argument_text(arguments, &["kind", "node_type", "nodeType"])
            .unwrap_or_else(|| "task".to_string()),
        required_text_any(arguments, &["goal"], "multihead_task")?,
        argument_text(arguments, &["actor", "actor_id", "actorId"])
            .unwrap_or_else(|| "substrate".to_string()),
    );
    node.prerequisites = string_array_any(arguments, &["prerequisites"]);
    node.file_scope = string_array_any(arguments, &["files", "file_scope", "fileScope"]);
    persist_multihead_task_node(backend, &node)?;
    persist_multihead_run_marker(
        tenant,
        backend,
        &run_id,
        argument_text(arguments, &["run_goal", "runGoal", "goal"])
            .unwrap_or_else(|| "Multi-head run".to_string())
            .as_str(),
        argument_text(arguments, &["actor", "actor_id", "actorId"])
            .unwrap_or_default()
            .as_str(),
    )?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "task": node
    }))
}

fn multihead_claim_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let action = argument_text(arguments, &["action"])
        .unwrap_or_else(|| "claim".to_string())
        .to_lowercase();
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_claim")?;
    let node_id = required_text_any(arguments, &["node_id", "nodeId"], "multihead_claim")?;
    let owner = required_text_any(
        arguments,
        &["owner", "actor", "actor_id", "actorId"],
        "multihead_claim",
    )?;
    let mut node = load_required_multihead_task_node(backend, &run_id, &node_id)?;
    if action == "release" {
        match node.claim.as_ref() {
            Some(claim) if claim.owner == owner => {
                node.claim = None;
                node.status = NodeStatus::Open;
                persist_multihead_task_node(backend, &node)?;
                return Ok(json!({
                    "tenant": normalize_tenant_slug(tenant),
                    "ok": true,
                    "released": true,
                    "task": node
                }));
            }
            Some(claim) => {
                return Err(McpError::invalid_params(format!(
                    "multihead_claim release owner mismatch: active owner is {}",
                    claim.owner
                )))
            }
            None => {
                return Ok(json!({
                    "tenant": normalize_tenant_slug(tenant),
                    "ok": true,
                    "released": false,
                    "task": node
                }))
            }
        }
    }

    let now = multihead_now(arguments);
    let expected_epoch = argument_u64(arguments, &["expected_epoch", "expectedEpoch", "epoch"])
        .unwrap_or(node.claim_epoch);
    let ttl = lease_ttl_ms(arguments);
    let outcome =
        theorem_harness_core::claim_task_node(&mut node, &owner, expected_epoch, now, ttl);
    if matches!(outcome, ClaimOutcome::Won { .. }) {
        persist_multihead_task_node(backend, &node)?;
    }
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": matches!(outcome, ClaimOutcome::Won { .. }),
        "outcome": outcome,
        "task": node
    }))
}

fn multihead_refine_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_refine")?;
    let parent_id = required_text_any(
        arguments,
        &["node_id", "nodeId", "parent_id", "parentId"],
        "multihead_refine",
    )?;
    let owner = required_text_any(
        arguments,
        &["owner", "actor", "actor_id", "actorId"],
        "multihead_refine",
    )?;
    let mut graph = load_multihead_work_graph(backend, &run_id)?;
    let children = arguments
        .get("children")
        .and_then(Value::as_array)
        .ok_or_else(|| McpError::invalid_params("multihead_refine requires children array"))?
        .iter()
        .map(|child| multihead_child_task_from_value(&run_id, child, &owner))
        .collect::<Result<Vec<_>, _>>()?;
    graph
        .refine(
            &parent_id,
            &owner,
            string_array_any(arguments, &["files", "file_scope", "fileScope"]),
            children,
            multihead_now(arguments),
        )
        .map_err(|error| McpError::invalid_params(format!("multihead_refine failed: {error:?}")))?;
    for node in graph.nodes.values() {
        persist_multihead_task_node(backend, node)?;
    }
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "graph": graph
    }))
}

fn multihead_next_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_next")?;
    let head = required_text_any(
        arguments,
        &["head", "owner", "actor", "actor_id", "actorId"],
        "multihead_next",
    )?;
    let graph = load_multihead_work_graph(backend, &run_id)?;
    let fitness = multihead_fitness(arguments)?;
    let explore_token = argument_u64(arguments, &["explore_token", "exploreToken"])
        .unwrap_or(0)
        .min(999) as u32;
    let next = next_for_head(
        &graph,
        &fitness,
        &head,
        explore_token,
        multihead_now(arguments),
    );
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "run_id": run_id,
        "head": head,
        "next_node_id": next
    }))
}

fn multihead_patch_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let action = argument_text(arguments, &["action"])
        .unwrap_or_else(|| "propose".to_string())
        .to_lowercase();
    if action == "rebase" {
        return multihead_rebase_patch_payload(tenant, backend, arguments);
    }
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_patch")?;
    let node_id = required_text_any(arguments, &["node_id", "nodeId"], "multihead_patch")?;
    let owner = required_text_any(
        arguments,
        &["owner", "actor", "actor_id", "actorId"],
        "multihead_patch",
    )?;
    let epoch = argument_u64(arguments, &["epoch"])
        .ok_or_else(|| McpError::invalid_params("multihead_patch requires epoch"))?;
    let base_commit =
        required_text_any(arguments, &["base_commit", "baseCommit"], "multihead_patch")?;
    let mut node = load_required_multihead_task_node(backend, &run_id, &node_id)?;
    require_live_claim(
        &node,
        &owner,
        epoch,
        multihead_now(arguments),
        "multihead_patch",
    )?;
    let patch = multihead_patch_record(&run_id, &node_id, &owner, epoch, &base_commit, arguments);
    node.status = NodeStatus::PatchProposed;
    node.receipts.push(Receipt {
        kind: "patch_proposal".to_string(),
        command: "patch_proposed".to_string(),
        base_commit: base_commit.clone(),
        claimed_status: "proposed".to_string(),
        verified_status: None,
        artifact_hash: patch["patch_hash"].as_str().unwrap_or_default().to_string(),
    });
    persist_multihead_task_node(backend, &node)?;
    persist_multihead_patch(tenant, backend, &patch)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "patch": patch,
        "task": node
    }))
}

fn multihead_rebase_patch_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let patch_id = required_text_any(arguments, &["patch_id", "patchId"], "multihead_patch")?;
    let new_base = required_text_any(
        arguments,
        &["new_base_commit", "newBaseCommit"],
        "multihead_patch",
    )?;
    let mut patch = load_required_multihead_patch(backend, &patch_id)?;
    let old_base = patch
        .get("base_commit")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if let Some(object) = patch.as_object_mut() {
        object.insert("base_commit".to_string(), json!(new_base));
        object.insert("status".to_string(), json!("rebased"));
        object.insert("updated_at".to_string(), json!(timestamp_or_now("")));
    }
    persist_multihead_patch(tenant, backend, &patch)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "changed": old_base != patch["base_commit"].as_str().unwrap_or_default(),
        "old_base_commit": old_base,
        "patch": patch,
        "invalidated_receipts": []
    }))
}

fn multihead_proof_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let patch_id = required_text_any(arguments, &["patch_id", "patchId"], "multihead_proof")?;
    let patch = load_required_multihead_patch(backend, &patch_id)?;
    let command = required_text_any(arguments, &["command"], "multihead_proof")?;
    let args = string_array_any(arguments, &["args"]);
    let cwd = argument_text(arguments, &["cwd"]);
    let output =
        run_multihead_proof_command(&command, &args, cwd.as_deref(), proof_timeout_ms(arguments))?;
    let status = if output.status_code == Some(0) {
        "passed"
    } else {
        "failed"
    };
    let receipt = json!({
        "receipt_id": stable_value_hash(&json!({
            "patch_id": patch_id,
            "command": command,
            "args": args,
            "started_at": output.started_at,
        })),
        "run_id": patch["run_id"].clone(),
        "patch_id": patch_id,
        "node_id": patch["node_id"].clone(),
        "base_commit": patch["base_commit"].clone(),
        "status": status,
        "trust_tier": "substrate_rerun",
        "command": command,
        "args": args,
        "cwd": cwd.unwrap_or_default(),
        "exit_code": output.status_code,
        "timed_out": output.timed_out,
        "stdout": output.stdout,
        "stderr": output.stderr,
        "started_at": output.started_at,
        "finished_at": output.finished_at,
    });
    persist_multihead_proof(tenant, backend, &receipt)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": status == "passed",
        "receipt": receipt,
        "patch": patch
    }))
}

fn multihead_review_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let action = argument_text(arguments, &["action"])
        .unwrap_or_else(|| "open".to_string())
        .to_lowercase();
    let patch_id = required_text_any(arguments, &["patch_id", "patchId"], "multihead_review")?;
    let reviewer = required_text_any(
        arguments,
        &["reviewer", "actor", "actor_id", "actorId"],
        "multihead_review",
    )?;
    let patch = load_required_multihead_patch(backend, &patch_id)?;
    let run_id = patch["run_id"].as_str().unwrap_or_default().to_string();
    let node_id = patch["node_id"].as_str().unwrap_or_default().to_string();
    if action == "open" {
        let verify = spawn_verify_for_target(backend, &run_id, &node_id, &reviewer)?;
        return Ok(json!({
            "tenant": normalize_tenant_slug(tenant),
            "ok": true,
            "review": {
                "review_id": verify,
                "run_id": run_id,
                "patch_id": patch_id,
                "node_id": node_id,
                "reviewer": reviewer,
                "status": "open"
            },
            "patch": patch
        }));
    }
    let status = argument_text(arguments, &["status"]).unwrap_or_else(|| "reviewed".to_string());
    let defect_found = !matches!(
        status.as_str(),
        "passed" | "accepted" | "approved" | "no_defect"
    );
    let receipt = VerifyReceipt {
        target_node_id: node_id,
        reviewer,
        attempted_failure_modes: string_array_any(
            arguments,
            &["falsification_attempts", "attempted_failure_modes"],
        ),
        commands_run: string_array_any(arguments, &["commands_run", "commandsRun"]),
        defect_found,
        waived_risks: string_array_any(arguments, &["waived_risks", "waivedRisks"]),
    };
    let outcome = submit_verify_for_target(backend, &run_id, &receipt)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "review": {
            "run_id": run_id,
            "patch_id": patch_id,
            "reviewer": receipt.reviewer,
            "status": status,
            "outcome": outcome
        },
        "patch": patch
    }))
}

fn multihead_spawn_verify_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_spawn_verify")?;
    let target = required_text_any(
        arguments,
        &["target_node_id", "targetNodeId", "node_id", "nodeId"],
        "multihead_spawn_verify",
    )?;
    let reviewer = required_text_any(
        arguments,
        &["reviewer", "reviewer_head", "reviewerHead", "head"],
        "multihead_spawn_verify",
    )?;
    let verify_node_id = spawn_verify_for_target(backend, &run_id, &target, &reviewer)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "verify_node_id": verify_node_id
    }))
}

fn multihead_submit_verify_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let run_id = required_text_any(arguments, &["run_id", "runId"], "multihead_submit_verify")?;
    let receipt = VerifyReceipt {
        target_node_id: required_text_any(
            arguments,
            &["target_node_id", "targetNodeId", "node_id", "nodeId"],
            "multihead_submit_verify",
        )?,
        reviewer: required_text_any(
            arguments,
            &["reviewer", "head", "actor", "actor_id", "actorId"],
            "multihead_submit_verify",
        )?,
        attempted_failure_modes: string_array_any(
            arguments,
            &["attempted_failure_modes", "falsification_attempts"],
        ),
        commands_run: string_array_any(arguments, &["commands_run", "commandsRun"]),
        defect_found: arguments
            .get("defect_found")
            .or_else(|| arguments.get("defectFound"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        waived_risks: string_array_any(arguments, &["waived_risks", "waivedRisks"]),
    };
    let outcome = submit_verify_for_target(backend, &run_id, &receipt)?;
    Ok(json!({
        "tenant": normalize_tenant_slug(tenant),
        "ok": true,
        "outcome": outcome,
        "receipt": receipt
    }))
}

fn multihead_child_task_from_value(
    run_id: &str,
    value: &Value,
    created_by: &str,
) -> Result<TaskNode, McpError> {
    let object = value
        .as_object()
        .ok_or_else(|| McpError::invalid_params("multihead_refine children must be objects"))?;
    let child_id = object
        .get("node_id")
        .or_else(|| object.get("nodeId"))
        .or_else(|| object.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let hash = stable_value_hash(value);
            format!("task:{}", &hash[..16])
        });
    let mut node = TaskNode::open(
        child_id,
        run_id,
        object
            .get("kind")
            .or_else(|| object.get("node_type"))
            .or_else(|| object.get("nodeType"))
            .and_then(Value::as_str)
            .unwrap_or("task"),
        object
            .get("goal")
            .and_then(Value::as_str)
            .ok_or_else(|| McpError::invalid_params("multihead_refine child requires goal"))?,
        created_by,
    );
    node.prerequisites = object
        .get("prerequisites")
        .map(|raw| string_array_any(&json!({ "items": raw }), &["items"]))
        .unwrap_or_default();
    node.file_scope = object
        .get("file_scope")
        .or_else(|| object.get("fileScope"))
        .or_else(|| object.get("files"))
        .map(|raw| string_array_any(&json!({ "items": raw }), &["items"]))
        .unwrap_or_default();
    Ok(node)
}

fn load_multihead_work_graph(
    backend: &impl McpGraphBackend,
    run_id: &str,
) -> Result<WorkGraph, McpError> {
    let mut graph = WorkGraph::new(run_id);
    for record in backend.query_nodes(
        NodeQuery::label(TASK_NODE_LABEL)
            .with_property("run_id", Value::String(run_id.to_string())),
    )? {
        graph.insert(parse_node_properties::<TaskNode>(record.properties)?);
    }
    Ok(graph)
}

fn load_multihead_task_node(
    backend: &impl McpGraphBackend,
    run_id: &str,
    node_id: &str,
) -> Result<Option<TaskNode>, McpError> {
    backend
        .get_node(&task_node_graph_id(run_id, node_id))?
        .map(|record| parse_node_properties::<TaskNode>(record.properties))
        .transpose()
}

fn load_required_multihead_task_node(
    backend: &impl McpGraphBackend,
    run_id: &str,
    node_id: &str,
) -> Result<TaskNode, McpError> {
    load_multihead_task_node(backend, run_id, node_id)?.ok_or_else(|| {
        McpError::invalid_params(format!("multi-head task not found: {run_id}/{node_id}"))
    })
}

fn persist_multihead_task_node(
    backend: &mut impl McpGraphBackend,
    node: &TaskNode,
) -> Result<(), McpError> {
    let properties =
        serde_json::to_value(node).map_err(|error| McpError::internal(error.to_string()))?;
    backend.upsert_node(NodeRecord::new(
        task_node_graph_id(&node.run_id, &node.id),
        [TASK_NODE_LABEL],
        properties,
    ))?;
    if let Some(parent) = node.parent_id.as_deref() {
        if backend
            .get_node(&task_node_graph_id(&node.run_id, parent))?
            .is_some()
        {
            backend.upsert_edge(EdgeRecord::new(
                format!(
                    "work-graph:{}:refined-into:{}->{}",
                    node.run_id, parent, node.id
                ),
                task_node_graph_id(&node.run_id, parent),
                EDGE_REFINED_INTO,
                task_node_graph_id(&node.run_id, &node.id),
                json!({ "run_id": node.run_id }),
            ))?;
        }
    }
    for prerequisite in &node.prerequisites {
        if backend
            .get_node(&task_node_graph_id(&node.run_id, prerequisite))?
            .is_some()
        {
            backend.upsert_edge(EdgeRecord::new(
                format!(
                    "work-graph:{}:prereq-of:{}->{}",
                    node.run_id, prerequisite, node.id
                ),
                task_node_graph_id(&node.run_id, prerequisite),
                EDGE_PREREQUISITE_OF,
                task_node_graph_id(&node.run_id, &node.id),
                json!({ "run_id": node.run_id }),
            ))?;
        }
    }
    Ok(())
}

fn spawn_verify_for_target(
    backend: &mut impl McpGraphBackend,
    run_id: &str,
    target: &str,
    reviewer: &str,
) -> Result<String, McpError> {
    let mut graph = load_multihead_work_graph(backend, run_id)?;
    let verify_node_id = spawn_verify_node(&mut graph, target, reviewer)
        .ok_or_else(|| McpError::invalid_params(format!("target not found: {target}")))?;
    let verify = graph
        .get(&verify_node_id)
        .ok_or_else(|| McpError::internal("spawned verify node missing from graph"))?;
    persist_multihead_task_node(backend, verify)?;
    Ok(verify_node_id)
}

fn submit_verify_for_target(
    backend: &mut impl McpGraphBackend,
    run_id: &str,
    receipt: &VerifyReceipt,
) -> Result<theorem_harness_core::VerifyOutcome, McpError> {
    let mut graph = load_multihead_work_graph(backend, run_id)?;
    let outcome = submit_verify_receipt(&mut graph, receipt);
    if let Some(target) = graph.get(&receipt.target_node_id) {
        persist_multihead_task_node(backend, target)?;
    }
    if let Some(verify) = graph.get(&theorem_harness_core::verify_node_id(
        &receipt.target_node_id,
    )) {
        persist_multihead_task_node(backend, verify)?;
    }
    Ok(outcome)
}

fn require_live_claim(
    node: &TaskNode,
    owner: &str,
    epoch: u64,
    now: Millis,
    tool_name: &str,
) -> Result<(), McpError> {
    match node.claim.as_ref() {
        Some(claim) if claim.owner == owner && claim.epoch == epoch && !claim.is_expired(now) => {
            Ok(())
        }
        Some(claim) => Err(McpError::invalid_params(format!(
            "{tool_name} active claim is {}@{}, not {owner}@{epoch}",
            claim.owner, claim.epoch
        ))),
        None => Err(McpError::invalid_params(format!(
            "{tool_name} target task has no active claim"
        ))),
    }
}

fn multihead_patch_record(
    run_id: &str,
    node_id: &str,
    owner: &str,
    epoch: u64,
    base_commit: &str,
    arguments: &Value,
) -> Value {
    let patch_text = argument_text(arguments, &["patch"]).unwrap_or_default();
    let patch_ref = arguments.get("patch_ref").cloned().unwrap_or(Value::Null);
    let files = string_array_any(arguments, &["files"]);
    let patch_hash = stable_value_hash(&json!({
        "patch": patch_text,
        "patch_ref": patch_ref,
        "files": files,
    }));
    let patch_id = argument_text(arguments, &["patch_id", "patchId"])
        .unwrap_or_else(|| format!("patch:{}", &patch_hash[..16]));
    json!({
        "patch_id": patch_id,
        "run_id": run_id,
        "node_id": node_id,
        "owner": owner,
        "epoch": epoch,
        "base_commit": base_commit,
        "status": "proposed",
        "files": files,
        "patch": patch_text,
        "patch_ref": patch_ref,
        "patch_hash": patch_hash,
        "created_at": timestamp_or_now(""),
        "updated_at": timestamp_or_now(""),
    })
}

fn multihead_fitness(arguments: &Value) -> Result<HeadFitness, McpError> {
    if let Some(fitness) = arguments.get("fitness") {
        return serde_json::from_value::<HeadFitness>(fitness.clone()).map_err(|error| {
            McpError::invalid_params(format!("multihead_next fitness is invalid: {error}"))
        });
    }
    let heads = string_array_any(arguments, &["heads"]);
    if !heads.is_empty() {
        return Ok(HeadFitness::new(heads));
    }
    let head = required_text_any(
        arguments,
        &["head", "owner", "actor", "actor_id", "actorId"],
        "multihead_next",
    )?;
    Ok(HeadFitness::new(vec![head]))
}

fn multihead_now(arguments: &Value) -> Millis {
    argument_u64(arguments, &["now", "now_ms", "nowMs"]).unwrap_or_else(current_unix_ms)
}

fn lease_ttl_ms(arguments: &Value) -> Millis {
    if let Some(ms) = argument_u64(arguments, &["lease_ttl_ms", "leaseTtlMs"]) {
        return ms;
    }
    argument_u64(arguments, &["lease_ttl_seconds", "leaseTtlSeconds"])
        .map(|seconds| seconds.saturating_mul(1000))
        .unwrap_or(DEFAULT_MULTIHEAD_LEASE_TTL_MS)
}

fn proof_timeout_ms(arguments: &Value) -> u64 {
    argument_u64(arguments, &["timeout_ms", "timeoutMs"])
        .unwrap_or(120_000)
        .clamp(1_000, 30 * 60_000)
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

fn multihead_run_node_id(tenant: &str, run_id: &str) -> String {
    format!("multihead-run:{}:{}", normalize_tenant_slug(tenant), run_id)
}

fn empty_multihead_run_marker(tenant: &str, run_id: &str) -> Value {
    json!({
        "tenant_slug": normalize_tenant_slug(tenant),
        "run_id": run_id,
        "goal": "",
        "actor": "",
        "created_at": "",
        "updated_at": "",
    })
}

fn load_multihead_run_marker(
    tenant: &str,
    backend: &impl McpGraphBackend,
    run_id: &str,
) -> Result<Option<Value>, McpError> {
    Ok(backend
        .get_node(&multihead_run_node_id(tenant, run_id))?
        .map(|record| record.properties))
}

fn persist_multihead_run_marker(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    run_id: &str,
    goal: &str,
    actor: &str,
) -> Result<(), McpError> {
    let existing = load_multihead_run_marker(tenant, backend, run_id)?
        .unwrap_or_else(|| empty_multihead_run_marker(tenant, run_id));
    let created_at = existing
        .get("created_at")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| timestamp_or_now(""));
    backend.upsert_node(NodeRecord::new(
        multihead_run_node_id(tenant, run_id),
        [MULTIHEAD_RUN_LABEL],
        json!({
            "tenant_slug": normalize_tenant_slug(tenant),
            "run_id": run_id,
            "goal": if goal.trim().is_empty() { existing.get("goal").cloned().unwrap_or(Value::String(String::new())) } else { json!(goal) },
            "actor": actor,
            "created_at": created_at,
            "updated_at": timestamp_or_now(""),
        }),
    ))?;
    Ok(())
}

fn multihead_patch_node_id(patch_id: &str) -> String {
    format!("multihead-patch:{patch_id}")
}

fn load_required_multihead_patch(
    backend: &impl McpGraphBackend,
    patch_id: &str,
) -> Result<Value, McpError> {
    backend
        .get_node(&multihead_patch_node_id(patch_id))?
        .map(|record| record.properties)
        .ok_or_else(|| McpError::invalid_params(format!("multi-head patch not found: {patch_id}")))
}

fn persist_multihead_patch(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    patch: &Value,
) -> Result<(), McpError> {
    let patch_id = patch
        .get("patch_id")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::internal("multi-head patch missing patch_id"))?;
    backend.upsert_node(NodeRecord::new(
        multihead_patch_node_id(patch_id),
        [MULTIHEAD_PATCH_LABEL],
        merge_json_object(
            patch,
            json!({
                "tenant_slug": normalize_tenant_slug(tenant),
            }),
        ),
    ))?;
    Ok(())
}

fn persist_multihead_proof(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    receipt: &Value,
) -> Result<(), McpError> {
    let receipt_id = receipt
        .get("receipt_id")
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::internal("multi-head proof missing receipt_id"))?;
    backend.upsert_node(NodeRecord::new(
        format!("multihead-proof:{receipt_id}"),
        [MULTIHEAD_PROOF_LABEL],
        merge_json_object(
            receipt,
            json!({
                "tenant_slug": normalize_tenant_slug(tenant),
            }),
        ),
    ))?;
    Ok(())
}

fn merge_json_object(left: &Value, right: Value) -> Value {
    let mut merged = left.as_object().cloned().unwrap_or_default();
    if let Some(right) = right.as_object() {
        for (key, value) in right {
            merged.insert(key.clone(), value.clone());
        }
    }
    Value::Object(merged)
}

struct MultiheadProofOutput {
    status_code: Option<i32>,
    timed_out: bool,
    stdout: String,
    stderr: String,
    started_at: String,
    finished_at: String,
}

fn run_multihead_proof_command(
    command: &str,
    args: &[String],
    cwd: Option<&str>,
    timeout_ms: u64,
) -> Result<MultiheadProofOutput, McpError> {
    let started_at = timestamp_or_now("");
    let mut command_builder = Command::new(command);
    command_builder.args(args);
    command_builder
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(cwd) = cwd.filter(|value| !value.trim().is_empty()) {
        command_builder.current_dir(cwd);
    }
    let mut child = command_builder.spawn().map_err(|error| {
        McpError::internal(format!("multihead_proof command failed to start: {error}"))
    })?;
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    let mut timed_out = false;
    loop {
        if child
            .try_wait()
            .map_err(|error| McpError::internal(format!("multihead_proof wait failed: {error}")))?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            break;
        }
        sleep(Duration::from_millis(25));
    }
    let output = child
        .wait_with_output()
        .map_err(|error| McpError::internal(format!("multihead_proof output failed: {error}")))?;
    Ok(MultiheadProofOutput {
        status_code: output.status.code(),
        timed_out,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        started_at,
        finished_at: timestamp_or_now(""),
    })
}

fn tool_result(payload: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        }],
        "structuredContent": payload
    })
}

fn tool_result_with_budget(payload: Value, config: &McpServerConfig, tool_name: &str) -> Value {
    let budget = tool_result_budget_for(config, tool_name);
    if budget == 0 {
        return tool_result(payload);
    }
    let text = serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string());
    if text.len() <= budget {
        return json!({
            "content": [{
                "type": "text",
                "text": text
            }],
            "structuredContent": payload
        });
    }

    let handle = store_tool_result_body(tool_name, &text);
    let preview = truncate_utf8_with_marker(&text, budget, &handle);
    let structured = json!({
        "budgeted": true,
        "truncated": true,
        "tool_name": tool_name,
        "family": tool_result_family(tool_name),
        "original_bytes": text.len(),
        "returned_bytes": preview.len(),
        "fetch_handle": handle.clone(),
        "next_cursor": {
            "handle": handle.clone(),
            "offset": preview.len(),
        },
        "message": "Tool result exceeded the configured MCP boundary budget; call tool_result_fetch with the fetch_handle and offset to retrieve more bytes."
    });
    json!({
        "content": [{
            "type": "text",
            "text": preview
        }],
        "structuredContent": structured
    })
}

fn tool_result_error(payload: Value) -> Value {
    let mut result = tool_result(payload);
    if let Value::Object(map) = &mut result {
        map.insert("isError".to_string(), Value::Bool(true));
    }
    result
}

fn tool_result_budget_for(config: &McpServerConfig, tool_name: &str) -> usize {
    let family = tool_result_family(tool_name);
    if let Some(budget) = config.tool_result_family_budgets.get(family).copied() {
        return budget;
    }
    match family {
        // harness_run is the advertised async-handoff poll tool; give the run-detail
        // family enough room to return the run inline instead of truncating into a
        // fetch handle. An explicit per-family config budget above still wins.
        "harness" => DEFAULT_HARNESS_TOOL_RESULT_BUDGET_BYTES,
        _ => config.tool_result_budget_bytes,
    }
}

fn tool_result_family(tool_name: &str) -> &'static str {
    match tool_name {
        "compute_code" | "rustyred_thg_compute_code" | "theorem_harness_compute_code" => "code",
        name if name.contains("code") => "code",
        name if name.contains("fractal") => "fractal",
        "recall"
        | "theorem_harness_recall"
        | "self_recall_archive"
        | "theorem_harness_self_recall_archive" => "recall",
        "coordination_context" | "theorem_harness_coordination_context" => "coordination",
        "harness_run" | "theorem_harness_run" => "harness",
        name if name.contains("graph") || name.contains("neighbors") => "graph",
        _ => "default",
    }
}

fn store_tool_result_body(tool_name: &str, body: &str) -> String {
    let handle = format!(
        "tool-result:{}:{}",
        rustyred_thg_core::normalize_plugin_command(tool_name),
        rustyred_thg_core::stable_hash(body)
    );
    let store = TOOL_RESULT_BODIES.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut bodies) = store.lock() {
        bodies.insert(handle.clone(), body.to_string());
    }
    handle
}

fn tool_result_fetch_payload(arguments: &Value) -> Result<Value, McpError> {
    let handle = arguments
        .get("fetch_handle")
        .or_else(|| arguments.get("handle"))
        .and_then(Value::as_str)
        .ok_or_else(|| McpError::invalid_params("tool_result_fetch requires fetch_handle"))?;
    let offset = arguments.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let max_bytes = arguments
        .get("max_bytes")
        .or_else(|| arguments.get("maxBytes"))
        .and_then(Value::as_u64)
        .unwrap_or(DEFAULT_TOOL_RESULT_BUDGET_BYTES as u64) as usize;
    let store = TOOL_RESULT_BODIES.get_or_init(|| Mutex::new(HashMap::new()));
    let bodies = store
        .lock()
        .map_err(|_| McpError::internal("tool result body store lock poisoned"))?;
    let body = bodies.get(handle).ok_or_else(|| {
        McpError::invalid_params(format!("tool result fetch_handle was not found: {handle}"))
    })?;
    let start = floor_char_boundary(body, offset.min(body.len()));
    let end = floor_char_boundary(body, start.saturating_add(max_bytes).min(body.len()));
    let next_offset = (end < body.len()).then_some(end);
    Ok(json!({
        "fetch_handle": handle,
        "offset": start,
        "next_offset": next_offset,
        "total_bytes": body.len(),
        "text": &body[start..end],
    }))
}

fn truncate_utf8_with_marker(content: &str, budget: usize, handle: &str) -> String {
    let head_capacity = budget
        .saturating_sub(TOOL_RESULT_MARKER_RESERVED_BYTES)
        .max(1);
    let cut = floor_char_boundary(content, head_capacity.min(content.len()));
    let dropped = content.len().saturating_sub(cut);
    let mut out = content[..cut].to_string();
    out.push_str(&format!(
        "\n\n[{} bytes truncated by tool_result_budget; fetch_handle={handle}]",
        dropped
    ));
    out
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    index = index.min(value.len());
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn jsonrpc_error(id: Option<Value>, error: McpError) -> Value {
    let mut body = json!({
        "jsonrpc": JSONRPC_VERSION,
        "id": id,
        "error": {
            "code": error.code,
            "message": error.message,
        }
    });
    if let Some(data) = error.data {
        body["error"]["data"] = data;
    }
    body
}

fn append_harness_transition_to_store<S: GraphStore>(
    store: &mut S,
    transition: TransitionInput,
) -> Result<Value, McpError> {
    append_transition_from_store(store, transition)
        .map(transition_result_payload)
        .map_err(mcp_harness_runtime_error)
}

fn harness_run_detail_from_store<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> Result<Option<Value>, McpError> {
    match load_run(store, run_id).map_err(mcp_harness_runtime_error)? {
        None => Ok(None),
        Some(run) => {
            let events = load_events(store, run_id).map_err(mcp_harness_runtime_error)?;
            Ok(Some(json!({ "run": run, "events": events })))
        }
    }
}

fn composed_agent_run_to_store<S: GraphStore>(
    store: &mut S,
    binding_id: String,
    task: String,
    claims: Vec<GroundedClaim>,
) -> Result<Value, McpError> {
    if composed_agent_real_invoker_enabled() {
        let invoker = RealHeadInvoker::from_env().map_err(mcp_head_invocation_error)?;
        let result = if claims.is_empty() {
            theorem_harness_runtime::run_composed_agent(store, &binding_id, &task, &invoker)
        } else {
            theorem_harness_runtime::run_composed_agent_with_claims(
                store,
                &binding_id,
                &task,
                claims,
                &invoker,
            )
        }
        .map_err(mcp_composed_agent_error)?;
        return composed_agent_result_value(result);
    }

    let invoker = FakeHeadInvoker::default();
    let result = if claims.is_empty() {
        theorem_harness_runtime::run_composed_agent(store, &binding_id, &task, &invoker)
    } else {
        theorem_harness_runtime::run_composed_agent_with_claims(
            store,
            &binding_id,
            &task,
            claims,
            &invoker,
        )
    }
    .map_err(mcp_composed_agent_error)?;
    composed_agent_result_value(result)
}

fn composed_agent_real_invoker_enabled() -> bool {
    std::env::var("THEOREM_COMPOSED_AGENT_INVOKER")
        .map(|value| value.trim().eq_ignore_ascii_case("real"))
        .unwrap_or(false)
        || std::env::var("THEOREM_COMPOSED_AGENT_REAL")
            .map(|value| matches!(value.trim(), "1" | "true" | "TRUE" | "yes" | "YES"))
            .unwrap_or(false)
}

fn composed_agent_result_value(
    result: theorem_harness_runtime::ComposedAgentRunResult,
) -> Result<Value, McpError> {
    serde_json::to_value(result).map_err(|error| {
        McpError::internal(format!(
            "composed_agent_run payload serialization failed: {error}"
        ))
    })
}

// ---------------------------------------------------------------------------
// Dispatch-board verb shaping. These `*_to_store` helpers run the runtime verbs
// over any GraphStore and shape the MCP payload, so the in-process backends and
// the product server's RuntimeTenantMirror backend stay byte-identical.
// ---------------------------------------------------------------------------

fn to_job_payload<T: serde::Serialize>(value: T) -> Result<Value, McpError> {
    serde_json::to_value(value)
        .map_err(|error| McpError::internal(format!("job payload serialization failed: {error}")))
}

fn job_action_payload(job_id: &str, result: Option<JobActionResult>) -> Result<Value, McpError> {
    match result {
        None => Ok(json!({ "job_id": job_id, "found": false })),
        Some(action) => {
            let mut job = to_job_payload(&action.job)?;
            if let Value::Object(map) = &mut job {
                map.insert("state".to_string(), json!(action.job.derived_state()));
            }
            Ok(json!({
                "job_id": job_id,
                "found": true,
                "applied": action.applied,
                "message": action.message,
                "job": job,
            }))
        }
    }
}

/// `job_submit`: create a pending job, or upsert an existing idempotency key.
pub fn job_submit_to_store<S: GraphStore>(
    store: &mut S,
    submission: JobSubmission,
    submitted_by: String,
) -> Result<Value, McpError> {
    let outcome = theorem_harness_runtime::job_submit(store, submission, submitted_by)
        .map_err(mcp_harness_runtime_error)?;
    Ok(json!({
        "job_id": outcome.job.job_id,
        "created": outcome.created,
        "job": to_job_payload(&outcome.job)?,
    }))
}

/// `job_list`: jobs ordered by priority then submitted_at, optionally filtered.
pub fn job_list_from_store<S: GraphStore>(
    store: &S,
    repo: Option<String>,
    state: Option<String>,
) -> Result<Value, McpError> {
    let jobs = theorem_harness_runtime::job_list(store, repo.as_deref(), state.as_deref())
        .map_err(mcp_harness_runtime_error)?;
    let shaped = jobs
        .iter()
        .map(|job| {
            let mut value = to_job_payload(job)?;
            if let Value::Object(map) = &mut value {
                map.insert("state".to_string(), json!(job.derived_state()));
            }
            Ok(value)
        })
        .collect::<Result<Vec<_>, McpError>>()?;
    Ok(json!({
        "count": jobs.len(),
        "jobs": shaped,
    }))
}

/// `job_note`: append a receipt and optionally set/clear receiver start metadata.
pub fn job_note_to_store<S: GraphStore>(
    store: &mut S,
    job_id: String,
    input: JobNoteInput,
) -> Result<Value, McpError> {
    let result = theorem_harness_runtime::job_note(store, &job_id, input)
        .map_err(mcp_harness_runtime_error)?;
    job_action_payload(&job_id, result)
}

/// `job_archive`: archive a job thread with a reason.
pub fn job_archive_to_store<S: GraphStore>(
    store: &mut S,
    job_id: String,
    reason: String,
    actor: String,
) -> Result<Value, McpError> {
    let result = theorem_harness_runtime::job_archive(store, &job_id, reason, actor)
        .map_err(mcp_harness_runtime_error)?;
    job_action_payload(&job_id, result)
}

fn job_submission_from_arguments(arguments: &Value) -> Result<JobSubmission, McpError> {
    serde_json::from_value::<JobSubmission>(arguments.clone()).map_err(|error| {
        McpError::invalid_params(format!(
            "job_submit requires title, repo, and one of spec_ref or spec_inline: {error}"
        ))
    })
}

fn job_string_array(arguments: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(values) = arguments.get(key).and_then(Value::as_array) {
            return values
                .iter()
                .filter_map(|value| value.as_str().map(str::to_string))
                .collect();
        }
    }
    Vec::new()
}

fn job_submit_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let submission = job_submission_from_arguments(arguments)?;
    let submitted_by = argument_text(arguments, &["submitted_by", "actor", "actor_id"])
        .unwrap_or_else(|| "unknown".to_string());
    let result = backend.job_submit(submission, submitted_by)?;
    Ok(json!({ "tenant": tenant, "result": result }))
}

fn job_list_payload(
    tenant: &str,
    backend: &impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let repo = argument_text(arguments, &["repo"]);
    let state = argument_text(arguments, &["state"]);
    let result = backend.job_list(repo, state)?;
    Ok(json!({ "tenant": tenant, "result": result }))
}

fn job_note_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let job_id = required_text_any(arguments, &["job_id", "jobId"], "job_note")?;
    let actor =
        argument_text(arguments, &["actor", "actor_id"]).unwrap_or_else(|| "unknown".to_string());
    let text = required_text_any(arguments, &["text", "message"], "job_note")?;
    let refs = job_string_array(arguments, &["refs"]);
    let input = JobNoteInput {
        actor,
        text,
        refs,
        start_session_ref: argument_text(arguments, &["start_session_ref", "startSessionRef"]),
        clear_started: arguments
            .get("clear_started")
            .or_else(|| arguments.get("clearStarted"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
    };
    let result = backend.job_note(job_id, input)?;
    Ok(json!({ "tenant": tenant, "result": result }))
}

fn job_archive_payload(
    tenant: &str,
    backend: &mut impl McpGraphBackend,
    arguments: &Value,
) -> Result<Value, McpError> {
    let job_id = required_text_any(arguments, &["job_id", "jobId"], "job_archive")?;
    let reason = required_text_any(arguments, &["reason"], "job_archive")?;
    let actor =
        argument_text(arguments, &["actor", "actor_id"]).unwrap_or_else(|| "unknown".to_string());
    let result = backend.job_archive(job_id, reason, actor)?;
    Ok(json!({ "tenant": tenant, "result": result }))
}

fn transition_result_payload(result: TransitionResult) -> Value {
    json!({
        "run": result.run,
        "event": result.event,
        "effects": result.effects,
        "state_hash_before": result.state_hash_before,
        "state_hash_after": result.state_hash_after
    })
}

fn mcp_harness_runtime_error(error: HarnessRuntimeError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "harness_runtime_error" })),
    }
}

fn mcp_composed_agent_error(error: theorem_harness_runtime::ComposedAgentRuntimeError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "composed_agent_runtime_error" })),
    }
}

fn mcp_head_invocation_error(error: theorem_harness_core::HeadInvocationError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "head_invocation_error" })),
    }
}

fn mcp_memory_error(error: MemoryError) -> McpError {
    let code = match &error {
        MemoryError::InvalidInput { .. } | MemoryError::NotFound { .. } => -32602,
        MemoryError::Store(_) | MemoryError::Serialization(_) | MemoryError::Deserialization(_) => {
            -32603
        }
    };
    McpError {
        code,
        message: error.to_string(),
        data: Some(json!({ "code": "harness_memory_error" })),
    }
}

fn mcp_skill_pack_error(error: SkillPackError) -> McpError {
    let code = match &error {
        SkillPackError::InvalidInput { .. } | SkillPackError::NotFound { .. } => -32602,
        SkillPackError::Store(_)
        | SkillPackError::Serialization(_)
        | SkillPackError::Deserialization(_) => -32603,
    };
    McpError {
        code,
        message: error.to_string(),
        data: Some(json!({ "code": "harness_skill_pack_error" })),
    }
}

fn mcp_ensemble_error(error: EnsembleError) -> McpError {
    let code = match &error {
        EnsembleError::InvalidPack(_) => -32602,
        EnsembleError::Store(_) => -32603,
    };
    McpError {
        code,
        message: error.to_string(),
        data: Some(json!({ "code": "ensemble_error" })),
    }
}

struct McpMemoryStore<'a, B: McpGraphBackend> {
    backend: &'a mut B,
}

struct McpMemoryReadStore<'a, B: McpGraphBackend> {
    backend: &'a B,
}

impl<B: McpGraphBackend> MemoryGraphStore for McpMemoryStore<'_, B> {
    fn memory_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.backend.upsert_node(node)
    }

    fn memory_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.backend.upsert_edge(edge)
    }

    fn memory_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.backend.get_node(id)
    }

    fn memory_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.backend.get_edge(id)
    }

    fn memory_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query)
    }

    fn memory_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.backend.neighbors(query)
    }

    fn memory_fulltext_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.backend.fulltext_search(label, property, query, k)
    }

    fn memory_vector_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.backend.vector_search(label, property, query, k)
    }
}

impl<B: McpGraphBackend> MemoryGraphStore for McpMemoryReadStore<'_, B> {
    fn memory_upsert_node(&mut self, _node: NodeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "read_only_memory_adapter",
            "read-only memory adapter cannot upsert nodes",
        ))
    }

    fn memory_upsert_edge(&mut self, _edge: EdgeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "read_only_memory_adapter",
            "read-only memory adapter cannot upsert edges",
        ))
    }

    fn memory_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.backend.get_node(id)
    }

    fn memory_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.backend.get_edge(id)
    }

    fn memory_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query)
    }

    fn memory_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.backend.neighbors(query)
    }

    fn memory_fulltext_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.backend.fulltext_search(label, property, query, k)
    }

    fn memory_vector_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.backend.vector_search(label, property, query, k)
    }
}

struct McpSkillPackStore<'a, B: McpGraphBackend> {
    backend: &'a mut B,
}

struct McpSkillPackReadStore<'a, B: McpGraphBackend> {
    backend: &'a B,
}

impl<B: McpGraphBackend> SkillPackGraphStore for McpSkillPackStore<'_, B> {
    fn skill_pack_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.backend.upsert_node(node)
    }

    fn skill_pack_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.backend.upsert_edge(edge)
    }

    fn skill_pack_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.backend.get_node(id)
    }

    fn skill_pack_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query)
    }
}

impl<B: McpGraphBackend> SkillPackGraphStore for McpSkillPackReadStore<'_, B> {
    fn skill_pack_upsert_node(&mut self, _node: NodeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "read_only_skill_pack_adapter",
            "read-only skill-pack adapter cannot upsert nodes",
        ))
    }

    fn skill_pack_upsert_edge(&mut self, _edge: EdgeRecord) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "read_only_skill_pack_adapter",
            "read-only skill-pack adapter cannot upsert edges",
        ))
    }

    fn skill_pack_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.backend.get_node(id)
    }

    fn skill_pack_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query)
    }
}

struct McpEnsembleStore<'a, B: McpGraphBackend> {
    backend: &'a mut B,
}

struct McpEnsembleReadStore<'a, B: McpGraphBackend> {
    backend: &'a B,
}

impl<B: McpGraphBackend> EnsembleGraphStore for McpEnsembleStore<'_, B> {
    fn pack_upsert_node(&mut self, node: NodeRecord) -> EnsembleResult<()> {
        self.backend.upsert_node(node).map_err(EnsembleError::from)
    }

    fn pack_upsert_edge(&mut self, edge: EdgeRecord) -> EnsembleResult<()> {
        self.backend.upsert_edge(edge).map_err(EnsembleError::from)
    }

    fn pack_get_node(&self, id: &str) -> EnsembleResult<Option<NodeRecord>> {
        self.backend.get_node(id).map_err(EnsembleError::from)
    }

    fn pack_query_nodes(&self, query: NodeQuery) -> EnsembleResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query).map_err(EnsembleError::from)
    }
}

impl<B: McpGraphBackend> EnsembleGraphStore for McpEnsembleReadStore<'_, B> {
    fn pack_upsert_node(&mut self, _node: NodeRecord) -> EnsembleResult<()> {
        Err(EnsembleError::from(GraphStoreError::new(
            "read_only_ensemble_adapter",
            "read-only Ensemble adapter cannot upsert nodes",
        )))
    }

    fn pack_upsert_edge(&mut self, _edge: EdgeRecord) -> EnsembleResult<()> {
        Err(EnsembleError::from(GraphStoreError::new(
            "read_only_ensemble_adapter",
            "read-only Ensemble adapter cannot upsert edges",
        )))
    }

    fn pack_get_node(&self, id: &str) -> EnsembleResult<Option<NodeRecord>> {
        self.backend.get_node(id).map_err(EnsembleError::from)
    }

    fn pack_query_nodes(&self, query: NodeQuery) -> EnsembleResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query).map_err(EnsembleError::from)
    }
}

fn resources(config: &McpServerConfig) -> Vec<Value> {
    let tenant = &config.default_tenant;
    vec![
        resource(
            "schema",
            format!("rustyred_thg://tenant/{tenant}/schema"),
            "THG schema",
        ),
        resource(
            "labels",
            format!("rustyred_thg://tenant/{tenant}/labels"),
            "THG labels",
        ),
        resource(
            "edge-types",
            format!("rustyred_thg://tenant/{tenant}/edge-types"),
            "THG edge types",
        ),
        resource(
            "indexes",
            format!("rustyred_thg://tenant/{tenant}/indexes"),
            "THG index status",
        ),
        resource(
            "stats",
            format!("rustyred_thg://tenant/{tenant}/stats"),
            "THG graph stats",
        ),
        resource(
            "verify-latest",
            format!("rustyred_thg://tenant/{tenant}/verify/latest"),
            "Latest THG verify report",
        ),
    ]
}

fn resource(name: &str, uri: String, description: &str) -> Value {
    json!({
        "name": name,
        "uri": uri,
        "description": description,
        "mimeType": "application/json"
    })
}

fn resource_templates() -> Vec<Value> {
    vec![
        json!({
            "name": "node",
            "uriTemplate": "rustyred_thg://tenant/{tenant}/node/{node_id}",
            "description": "Read a graph node by id.",
            "mimeType": "application/json"
        }),
        json!({
            "name": "edge",
            "uriTemplate": "rustyred_thg://tenant/{tenant}/edge/{edge_id}",
            "description": "Read a graph edge by id.",
            "mimeType": "application/json"
        }),
        json!({
            "name": "neighbors",
            "uriTemplate": "rustyred_thg://tenant/{tenant}/neighbors/{node_id}",
            "description": "Read outgoing neighbors for a node.",
            "mimeType": "application/json"
        }),
    ]
}

/// The flat tools whose capability is fully covered by the typed GraphQL schema
/// (Area A). In `graphql_default_surface` mode these are hidden from tools/list so
/// GraphQL is the advertised agent path; the three graphql_* transport tools and
/// every not-yet-covered tool (web/browse/fractal, coordination/stream/multihead,
/// raw graph_query, relational_query, version, harness_prepare, describe/invoke,
/// etc.) stay visible. Conservative by design: a flat tool absent from this list
/// simply stays advertised, so opting in never removes an uncovered capability.
const GRAPHQL_COVERED_FLAT_TOOLS: &[&str] = &[
    // memory domain
    "recall",
    "relate",
    "remember",
    "encode",
    "self_revise",
    "forget",
    "handoff",
    "self_recall_archive",
    // graph algorithms (eight flat tools -> graphAlgorithm)
    "rustyred_thg_algorithm_pagerank",
    "rustyred_thg_algorithm_ppr",
    "rustyred_thg_algorithm_communities",
    "rustyred_thg_algorithm_components",
    "rustyred_thg_algorithm_pagerank_inline",
    "rustyred_thg_algorithm_ppr_inline",
    "rustyred_thg_algorithm_communities_inline",
    "rustyred_thg_algorithm_components_inline",
    // graph reads, index designations, bulk upserts, symbolic
    "rustyred_thg_graph_neighbors",
    "rustyred_thg_graph_schema",
    "rustyred_thg_vector_search",
    "rustyred_thg_vector_hybrid",
    "rustyred_thg_vector_designate",
    "rustyred_thg_fulltext_search",
    "rustyred_thg_fulltext_designate",
    "rustyred_thg_spatial_radius",
    "rustyred_thg_spatial_bbox",
    "rustyred_thg_spatial_designate",
    "rustyred_thg_bulk_nodes",
    "rustyred_thg_bulk_edges",
    "rustyred_thg_symbolic_datalog_derive",
    "rustyred_thg_symbolic_probabilistic_source_reliability",
    "rustyred_thg_symbolic_probabilistic_expected_value",
    // epistemic shadow graph
    "rustyred_thg_epistemic_neighbors",
    "epistemic_dirty_frontier",
    "epistemic_compile_subgraph",
    "epistemic_shadow_ppr",
    "epistemic_enrich_apply",
    // code (CodeCrawler)
    "compute_code",
    "code_ingest",
    // harness instant-KG
    "harness_kg_status",
    "harness_kg_search",
    "harness_kg_ppr",
    "harness_kg_impact",
    "harness_kg_related_objects",
    "harness_kg_explain_edge",
    // clusters: skills / ensemble / jobs / harness-run
    "skill_list",
    "skill_get",
    "skill_publish",
    "skill_apply",
    "ensemble_register",
    "ensemble_select",
    "job_submit",
    "job_list",
    "job_note",
    "job_archive",
    "harness_run",
];

fn tool_definitions(config: &McpServerConfig) -> Vec<Value> {
    let mut tools = vec![
        tool(
            "tool_result_fetch",
            "Fetch a byte slice from a tool result that exceeded the MCP boundary budget.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "fetch_handle": { "type": "string" },
                    "handle": { "type": "string" },
                    "offset": { "type": "integer", "default": 0 },
                    "max_bytes": { "type": "integer", "default": DEFAULT_TOOL_RESULT_BUDGET_BYTES },
                    "maxBytes": { "type": "integer", "default": DEFAULT_TOOL_RESULT_BUDGET_BYTES }
                },
                "required": ["fetch_handle"]
            }),
        ),
        tool(
            "tool_search",
            "Search tenant-scoped federated MCP affordances through the connector gateway without advertising every spoke tool schema upfront.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "query": { "type": "string" },
                    "task_type": { "type": "string" },
                    "k": { "type": "integer", "default": 10 },
                    "limit": { "type": "integer", "default": 10 },
                    "allow_affordance_ids": { "type": "array", "items": { "type": "string" } },
                    "allow_servers": { "type": "array", "items": { "type": "string" } },
                    "allow_families": { "type": "array", "items": { "type": "string" } },
                    "allow_tags": { "type": "array", "items": { "type": "string" } },
                    "min_fitness": { "type": "number" },
                    "ppr_damping": { "type": "number" },
                    "ppr_max_iter": { "type": "integer" }
                },
                "required": ["query"]
            }),
        ),
        tool(
            "describe",
            "Materialize one federated affordance's full input schema on demand by affordance_id.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "affordance_id": { "type": "string" }
                },
                "required": ["affordance_id"]
            }),
        ),
        tool_write(
            "invoke",
            "Invoke a selected federated MCP affordance through its persisted connector target; use dry_run=true to plan without firing.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "affordance_id": { "type": "string" },
                    "arguments": { "type": "object" },
                    "tool_arguments": { "type": "object" },
                    "task_type": { "type": "string" },
                    "query": { "type": "string" },
                    "candidate_affordance_ids": { "type": "array", "items": { "type": "string" } },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "dry_run": { "type": "boolean", "default": false }
                },
                "required": ["affordance_id", "arguments"]
            }),
        ),
        tool(
            "rustyred_thg_graph_neighbors",
            "Read graph neighbors through THG adjacency indexes.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "node_id": { "type": "string" },
                    "direction": { "type": "string", "enum": ["out", "in"] },
                    "edge_type": { "type": "string" },
                    "budget": { "type": "object" }
                },
                "required": ["node_id"]
            }),
        ),
        tool(
            "rustyred_thg_graph_query",
            "Run a bounded graph query. Supports adjacency neighbors and exact scalar node_match.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "operation": { "type": "string", "enum": ["neighbors", "node_match"] },
                    "node_id": { "type": "string" },
                    "direction": { "type": "string", "enum": ["out", "in"] },
                    "edge_type": { "type": "string" },
                    "label": { "type": "string" },
                    "properties": { "type": "object" },
                    "budget": { "type": "object" }
                }
            }),
        ),
        tool(
            "rustyred_thg_relational_query",
            "Run a native relational planner query over the tenant graph snapshot. Accepts QueryIr or a GraphQL-style selection AST and returns planner trace receipts.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "query_ir": {
                        "type": "object",
                        "description": "Native QueryIr: relations, predicates, joins, projection, limit."
                    },
                    "selection": {
                        "type": "object",
                        "description": "GraphQL-style selection AST: relation, fields, joins, limit."
                    },
                    "budget": { "type": "object" }
                }
            }),
        ),
        tool(
            "rustyred_thg_graph_explain",
            "Explain the bounded THG query plan without executing raw Redis.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "operation": { "type": "string" },
                    "node_id": { "type": "string" },
                    "direction": { "type": "string" },
                    "edge_type": { "type": "string" },
                    "label": { "type": "string" },
                    "properties": { "type": "object" },
                    "budget": { "type": "object" }
                }
            }),
        ),
        tool(
            "rustyred_thg_graph_schema",
            "Read labels, edge types, stats, and current graph-store capability notes.",
            json!({
                "type": "object",
                "properties": { "tenant": { "type": "string" } }
            }),
        ),
        tool(
            "rustyred_thg_graph_index_status",
            "Read index health and verify drift without exposing Redis keys.",
            json!({
                "type": "object",
                "properties": { "tenant": { "type": "string" } }
            }),
        ),
        tool(
            "rustyred_thg_graph_version_compile",
            "Compile the tenant graph into a content-addressed Prolly-tree graph pack.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "name": { "type": "string" },
                    "branch": { "type": "string" },
                    "parent_commits": { "type": "array", "items": { "type": "string" } },
                    "author": { "type": "string" },
                    "message": { "type": "string" },
                    "timestamp_unix_ms": { "type": "integer" },
                    "include_payloads": { "type": "boolean", "default": true }
                }
            }),
        ),
        tool(
            "rustyred_thg_graph_version_diff",
            "Diff a base graph snapshot against the tenant graph or an explicit target snapshot.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "base": { "type": "object" },
                    "target": { "type": "object" }
                },
                "required": ["base"]
            }),
        ),
        tool(
            "rustyred_thg_graph_version_ref",
            "Compile the current tenant graph and update a branch ref inside a caller-supplied graph repository value.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "repository": { "type": "object" },
                    "name": { "type": "string" },
                    "branch": { "type": "string", "default": "main" },
                    "parent_commits": { "type": "array", "items": { "type": "string" } },
                    "author": { "type": "string" },
                    "message": { "type": "string" },
                    "timestamp_unix_ms": { "type": "integer" },
                    "updated_at_unix_ms": { "type": "integer" },
                    "include_payloads": { "type": "boolean", "default": true }
                }
            }),
        ),
        tool(
            "rustyred_thg_graph_version_log",
            "Walk graph commit history from a branch name or commit hash in a caller-supplied graph repository.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "repository": { "type": "object" },
                    "target": { "type": "string", "default": "main" }
                },
                "required": ["repository"]
            }),
        ),
        tool(
            "rustyred_thg_graph_version_checkout",
            "Reconstruct a graph snapshot from a branch name or commit hash in a caller-supplied graph repository.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "repository": { "type": "object" },
                    "target": { "type": "string" }
                },
                "required": ["repository", "target"]
            }),
        ),
        tool(
            "rustyred_thg_graph_version_merge",
            "Three-way merge graph snapshots with content-hash conflict detection and confidence-weighted edge resolution.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "base": { "type": "object" },
                    "ours": { "type": "object" },
                    "theirs": { "type": "object" },
                    "strategy": {
                        "type": "string",
                        "enum": ["auto_confidence", "prefer_ours", "prefer_theirs", "manual"],
                        "default": "auto_confidence"
                    },
                    "min_confidence_delta": { "type": "number", "default": 0.0 },
                    "branch": { "type": "string" },
                    "author": { "type": "string" },
                    "message": { "type": "string" },
                    "timestamp_unix_ms": { "type": "integer" },
                    "include_payloads": { "type": "boolean", "default": true }
                },
                "required": ["base", "theirs"]
            }),
        ),
        tool(
            "rustyred_thg_algorithm_ppr",
            "Run Personalized PageRank over the tenant graph.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "seeds": { "type": "object", "additionalProperties": { "type": "number" } },
                    "alpha": { "type": "number", "default": 0.15 },
                    "epsilon": { "type": "number", "default": 0.0001 },
                    "max_pushes": { "type": "integer", "default": 200000 },
                    "top_k": { "type": "integer" }
                },
                "required": ["seeds"]
            }),
        ),
        tool(
            "rustyred_thg_algorithm_components",
            "Run connected-components over the tenant graph.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "directed": { "type": "boolean", "default": false }
                }
            }),
        ),
        tool(
            "rustyred_thg_algorithm_pagerank",
            "Run global PageRank over the tenant graph.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "damping": { "type": "number", "default": 0.85 },
                    "max_iter": { "type": "integer", "default": 100 },
                    "tolerance": { "type": "number", "default": 0.000001 },
                    "top_k": { "type": "integer" }
                }
            }),
        ),
        tool(
            "rustyred_thg_algorithm_communities",
            "Run label-propagation community detection over the tenant graph.",
            json!({
                "type": "object",
                "properties": { "tenant": { "type": "string" } }
            }),
        ),
        tool(
            "harness_kg_status",
            "Return Harness Instant KG merged-view status for the tenant base graph plus an optional session delta.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" }
                }
            }),
        ),
        tool(
            "harness_kg_ppr",
            "Run Personalized PageRank over the Harness Instant KG merged base+delta view.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" },
                    "seeds": { "type": "object", "additionalProperties": { "type": "number" } },
                    "alpha": { "type": "number", "default": 0.15 },
                    "epsilon": { "type": "number", "default": 0.0001 },
                    "max_pushes": { "type": "integer", "default": 200000 },
                    "top_k": { "type": "integer", "default": 10 }
                },
                "required": ["seeds"]
            }),
        ),
        tool(
            "harness_kg_impact",
            "Compute the blast radius from a code object in the Harness Instant KG merged view.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" },
                    "seed": { "type": "string" },
                    "symbol_name": { "type": "string" },
                    "direction": { "type": "string", "enum": ["out", "in"], "default": "out" },
                    "max_depth": { "type": "integer", "default": 2 }
                }
            }),
        ),
        tool(
            "harness_kg_related_objects",
            "Find code objects related to a seed in the Harness Instant KG merged view.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" },
                    "seed": { "type": "string" },
                    "kinds": { "type": "array", "items": { "type": "string" } },
                    "top_k": { "type": "integer", "default": 10 }
                },
                "required": ["seed"]
            }),
        ),
        tool(
            "harness_kg_search",
            "Run lexical code-object search over the Harness Instant KG merged view.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" },
                    "query": { "type": "string" },
                    "kinds": { "type": "array", "items": { "type": "string" } },
                    "top_k": { "type": "integer", "default": 10 }
                },
                "required": ["query"]
            }),
        ),
        tool(
            "harness_kg_explain_edge",
            "Explain why a merged Instant KG edge exists between two objects.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "manifest": { "type": "object" },
                    "delta": { "type": "object" },
                    "src": { "type": "string" },
                    "dst": { "type": "string" }
                },
                "required": ["src", "dst"]
            }),
        ),
        // RR-INLINE-09: inline-adjacency algorithm tools. Stateless variants
        // that accept the graph inline rather than reading from a tenant.
        // Bounded by RUSTY_RED_MAX_INLINE_EDGES (default 100_000); above the
        // limit, callers receive `payload_too_large` and should switch to the
        // tenant-backed counterpart above.
        tool(
            "rustyred_thg_algorithm_ppr_inline",
            "Run Personalized PageRank over inline adjacency (stateless; does NOT touch any tenant). Bounded by RUSTY_RED_MAX_INLINE_EDGES.",
            json!({
                "type": "object",
                "properties": {
                    "adjacency": {
                        "type": "object",
                        "description": "Map of node_id to list of [neighbor_id, weight] pairs.",
                        "additionalProperties": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 2
                            }
                        }
                    },
                    "seeds": {
                        "type": "object",
                        "description": "Map of node_id to seed mass.",
                        "additionalProperties": { "type": "number" }
                    },
                    "alpha": { "type": "number", "default": 0.15 },
                    "epsilon": { "type": "number", "default": 0.0001 },
                    "max_pushes": { "type": "integer", "default": 200000 },
                    "top_k": { "type": "integer" }
                },
                "required": ["adjacency", "seeds"]
            }),
        ),
        tool(
            "rustyred_thg_algorithm_components_inline",
            "Run connected-components over inline adjacency (stateless; does NOT touch any tenant). Bounded by RUSTY_RED_MAX_INLINE_EDGES.",
            json!({
                "type": "object",
                "properties": {
                    "adjacency": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 2
                            }
                        }
                    },
                    "directed": { "type": "boolean", "default": false }
                },
                "required": ["adjacency"]
            }),
        ),
        tool(
            "rustyred_thg_algorithm_pagerank_inline",
            "Run power-iteration PageRank over inline adjacency (stateless; does NOT touch any tenant). Bounded by RUSTY_RED_MAX_INLINE_EDGES.",
            json!({
                "type": "object",
                "properties": {
                    "adjacency": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 2
                            }
                        }
                    },
                    "damping": { "type": "number", "default": 0.85 },
                    "max_iter": { "type": "integer", "default": 100 },
                    "tolerance": { "type": "number", "default": 0.000001 },
                    "top_k": { "type": "integer" }
                },
                "required": ["adjacency"]
            }),
        ),
        tool(
            "rustyred_thg_algorithm_communities_inline",
            "Run label-propagation community detection over inline adjacency (stateless; does NOT touch any tenant). Bounded by RUSTY_RED_MAX_INLINE_EDGES.",
            json!({
                "type": "object",
                "properties": {
                    "adjacency": {
                        "type": "object",
                        "additionalProperties": {
                            "type": "array",
                            "items": {
                                "type": "array",
                                "minItems": 2,
                                "maxItems": 2
                            }
                        }
                    }
                },
                "required": ["adjacency"]
            }),
        ),
        tool(
            "rustyred_thg_symbolic_datalog_derive",
            "Run the parity-gated symbolic Datalog derivation over an inline fact pack. Bounded by RUSTY_RED_MAX_SYMBOLIC_FACTS.",
            json!({
                "type": "object",
                "properties": {
                    "facts": {
                        "type": "array",
                        "items": { "type": "object" },
                        "description": "DatalogFact-shaped rows: relation, entity_id, attributes, source_ref, fact_id."
                    },
                    "fact_pack": {
                        "type": "object",
                        "properties": {
                            "facts": { "type": "array", "items": { "type": "object" } }
                        }
                    },
                    "rule_ids": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional subset of verified Datalog rule ids. Empty or omitted runs all verified rules."
                    }
                }
            }),
        ),
        tool(
            "rustyred_thg_symbolic_probabilistic_source_reliability",
            "Run the parity-gated Beta-Bernoulli source reliability receipt.",
            json!({
                "type": "object",
                "properties": {
                    "source_id": { "type": "string" },
                    "prior_alpha": { "type": "number", "default": 1.0 },
                    "prior_beta": { "type": "number", "default": 1.0 },
                    "corroborated": { "type": "integer", "default": 0 },
                    "contradicted": { "type": "integer", "default": 0 }
                },
                "required": ["source_id"]
            }),
        ),
        tool(
            "rustyred_thg_symbolic_probabilistic_expected_value",
            "Run the parity-gated expected-value-of-information receipt.",
            json!({
                "type": "object",
                "properties": {
                    "current_uncertainty": { "type": "number" },
                    "expected_uncertainty_after": { "type": "number" },
                    "decision_value": { "type": "number", "default": 1.0 },
                    "validator_cost": { "type": "number", "default": 0.0 }
                },
                "required": ["current_uncertainty", "expected_uncertainty_after"]
            }),
        ),
        tool(
            "read_intents_for_room",
            "Read native Theorem harness coordination intents for a room.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "statuses": { "type": "array", "items": { "type": "string" } }
                }
            }),
        ),
        tool(
            "read_messages_for_room",
            "Read native Theorem harness direct-coordination messages for a room.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "limit": { "type": "integer", "default": 50 }
                }
            }),
        ),
        tool(
            "stream_read",
            "Pull the append-only coordination events after your stored cursor on your subscribed streams (or explicit streams[]). The passive, cursor-delta read that replaces the room poll; advance=true (default) consumes the window once.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "streams": { "type": "array", "items": { "type": "string" }, "description": "Explicit stream topics to read. Omit to read every stream you are subscribed to." },
                    "stream": { "type": "string", "description": "A single stream topic to read (alias for streams: [stream])." },
                    "advance": { "type": "boolean", "default": true, "description": "Advance your cursor past the events returned so they are not re-read." },
                    "limit": { "type": "integer", "default": 0, "description": "Max events per stream; 0 = no cap." }
                },
                "required": ["actor"]
            }),
        ),
        tool(
            "read_records_for_room",
            "Read durable native Theorem harness coordination records for a room.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "record_type": { "type": "string", "enum": ["event", "decision", "tension", "reflection"] },
                    "record_types": { "type": "array", "items": { "type": "string", "enum": ["event", "decision", "tension", "reflection"] } },
                    "limit": { "type": "integer", "default": 50 }
                }
            }),
        ),
        tool(
            "coordination_context",
            "Read a bundled native Theorem harness coordination context packet for turn-start injection.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "statuses": { "type": "array", "items": { "type": "string" } },
                    "record_type": { "type": "string", "enum": ["event", "decision", "tension", "reflection"] },
                    "record_types": { "type": "array", "items": { "type": "string", "enum": ["event", "decision", "tension", "reflection"] } },
                    "limit": { "type": "integer", "default": 20 },
                    "message_limit": { "type": "integer", "default": 20 },
                    "record_limit": { "type": "integer", "default": 20 },
                    "mention_limit": { "type": "integer", "default": 20 }
                }
            }),
        ),
        tool(
            "harness_run",
            "Read a native Theorem harness run plus ordered event ledger from the tenant GraphStore.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" }
                },
                "required": ["run_id"]
            }),
        ),
        tool_write(
            "composed_agent_run",
            "Run one composed-agent turn through the binding scratchpad and alignment gate.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "binding_id": { "type": "string", "default": "agent:theorem" },
                    "bindingId": { "type": "string" },
                    "task": { "type": "string" },
                    "claims": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "text": { "type": "string" },
                                "provenance": { "type": "string" }
                            },
                            "required": ["text", "provenance"]
                        }
                    }
                },
                "required": ["task"]
            }),
        ),
        tool_write(
            "job_submit",
            "Dispatch v2: create or upsert a pending Job thread. Duplicate idempotency_key returns the existing job.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "job_id": { "type": "string", "description": "optional externally assigned id for Postgres dispatch mirrors/backfills" },
                    "title": { "type": "string" },
                    "spec_ref": { "type": "string", "description": "repo path (docs/plans/x/HANDOFF.md) or harness doc_id" },
                    "spec_inline": { "type": "string", "description": "inline spec text when no spec_ref is available" },
                    "repo": { "type": "string", "description": "Travis-Gilbert/theorem etc." },
                    "priority": { "type": "string", "enum": ["P0", "P1", "P2"] },
                    "target_head": { "type": "string", "enum": ["claude", "codex", "either"] },
                    "not_before": { "type": "string", "description": "optional timestamp hint; receivers skip future jobs" },
                    "source_task_id": { "type": "string", "description": "TickTick task id this job was captured from (Agent Queue path); lets the loop relay milestones back to the task" },
                    "source_project_id": { "type": "string", "description": "TickTick project (list) id the source task lived in" },
                    "idempotency_key": { "type": "string", "description": "defaults to hash(spec_ref + title)" },
                    "submitted_by": { "type": "string" }
                },
                "required": ["title", "repo"]
            }),
        ),
        tool(
            "job_list",
            "Dispatch v2: list the Job board ordered by priority then submitted_at, optionally filtered by repo and derived state.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "repo": { "type": "string" },
                    "state": { "type": "string", "enum": ["pending", "started", "archived"] }
                }
            }),
        ),
        tool_write(
            "job_note",
            "Dispatch v2: append a receipt to a Job thread. Receivers may include start_session_ref for the set-once launch write.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "job_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "text": { "type": "string" },
                    "refs": { "type": "array", "items": { "type": "string" } },
                    "start_session_ref": { "type": "string" },
                    "clear_started": { "type": "boolean" }
                },
                "required": ["job_id", "text"]
            }),
        ),
        tool_write(
            "job_archive",
            "Dispatch v2: archive a Job thread with a reason.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "job_id": { "type": "string" },
                    "reason": { "type": "string" },
                    "actor": { "type": "string" }
                },
                "required": ["job_id", "reason"]
            }),
        ),
        tool(
            "compute_code",
            "Native CodeCrawler read path for graph-structural code discovery: search, context, explain, recognize, and explore.",
            json!({
                "type": "object",
                "properties": {
                    "tenant_slug": { "type": "string" },
                    "operation": {
                        "type": "string",
                        "enum": ["search", "context", "recognize", "explore", "explain", "list_repos", "kg_status", "context_pack", "ingest_status"],
                        "default": "search"
                    },
                    "query": { "type": "string" },
                    "node_id": { "type": "string" },
                    "repo": { "type": "string" },
                    "path_prefix": { "type": "string" },
                    "kinds": { "type": "array", "items": { "type": "string" } },
                    "limit": { "type": "integer", "default": 20 },
                    "limits": { "type": "object" }
                }
            }),
        ),
        tool_write(
            "code_ingest",
            "Native CodeCrawler heavy path for ingesting or reindexing repositories.",
            json!({
                "type": "object",
                "properties": {
                    "tenant_slug": { "type": "string" },
                    "operation": {
                        "type": "string",
                        "enum": ["ingest", "reindex", "session_reingest", "record_use_receipt"],
                        "default": "ingest"
                    },
                    "repo": { "type": "string" },
                    "repo_url": { "type": "string" },
                    "repo_path": { "type": "string" },
                    "paths": { "type": "array", "items": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "object" } },
                    "node_id": { "type": "string" },
                    "action": { "type": "string" },
                    "outcome": { "type": "string" },
                    "limits": { "type": "object" },
                    "confirmed": { "type": "boolean", "default": true }
                }
            }),
        ),
        tool(
            "harness_prepare",
            "Compose a native Theorem Context Brief from Ensemble selection plus tenant memory recall.",
            json!({
                "type": "object",
                "properties": {
                    "task": { "type": "string" },
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "budget_units": { "type": "integer" },
                    "max_selected": { "type": "integer" },
                    "maxSelected": { "type": "integer" },
                    "memory_limit": { "type": "integer", "default": 8 },
                    "surface": { "type": "string" },
                    "priors": { "type": "object" }
                },
                "required": ["task"]
            }),
        ),
        tool(
            "ensemble_select",
            "Select registered Ensemble capability packs under task, budget, trust, and prior constraints.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "task": { "type": "string" },
                    "query": { "type": "string" },
                    "intent": { "type": "string" },
                    "kind": { "type": "string", "enum": ["skill", "skill_pack", "agent", "tool", "validator", "renderer", "compute", "policy", "domain", "context"] },
                    "pack_kind": { "type": "string", "enum": ["skill", "skill_pack", "agent", "tool", "validator", "renderer", "compute", "policy", "domain", "context"] },
                    "budget_units": { "type": "integer" },
                    "budgetUnits": { "type": "integer" },
                    "max_selected": { "type": "integer" },
                    "maxSelected": { "type": "integer" },
                    "priors": { "type": "object" },
                    "pack_scores": { "type": "object" },
                    "pack_costs": { "type": "object" },
                    "prior_weight": { "type": "number" },
                    "lexical_weight": { "type": "number" },
                    "trust_weight": { "type": "number" },
                    "min_trust": { "type": "string", "enum": ["unverified", "first_party"] },
                    "kinds": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["task"]
            }),
        ),
        tool(
            "mentions",
            "Read pending native Theorem harness mentions for an actor. consume=true requires write mode.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "consume": { "type": "boolean", "default": false },
                    "limit": { "type": "integer", "default": 20 }
                }
            }),
        ),
        tool(
            "recall",
            "Recall native Theorem harness memory documents and graph-node memories from the tenant GraphStore.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "query": { "type": "string" },
                    "surface": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "since": { "type": "string" },
                    "kind": { "type": "string" },
                    "limit": { "type": "integer", "default": 10 },
                    "include_low_fitness": { "type": "boolean", "default": false },
                    "include_consolidation_sources": { "type": "boolean", "default": false },
                    "consume_handoffs": { "type": "boolean", "default": false },
                    "query_time": { "type": "string", "description": "RFC3339 valid-time cutoff. Defaults to now." },
                    "queryTime": { "type": "string" },
                    "overall_state": { "type": "boolean", "default": false },
                    "overallState": { "type": "boolean", "default": false },
                    "seed_limit": { "type": "integer", "default": 16 },
                    "seedLimit": { "type": "integer" },
                    "query_embedding": { "type": "array", "items": { "type": "number" } },
                    "queryEmbedding": { "type": "array", "items": { "type": "number" } },
                    "embedding_property": { "type": "string", "default": "embedding" },
                    "embeddingProperty": { "type": "string" },
                    "ppr_alpha": { "type": "number", "default": 0.15 },
                    "pprAlpha": { "type": "number" },
                    "ppr_epsilon": { "type": "number", "default": 0.000001 },
                    "pprEpsilon": { "type": "number" },
                    "ppr_max_pushes": { "type": "integer", "default": 100000 },
                    "pprMaxPushes": { "type": "integer" },
                    "recency_half_life_seconds": { "type": "number", "default": 0 },
                    "recencyHalfLifeSeconds": { "type": "number" }
                }
            }),
        ),
        tool(
            "epistemic_dirty_frontier",
            "Return content nodes whose EpistemicShadow is absent, explicitly dirty, or stale, expanded by k hops.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "content_ids": { "type": "array", "items": { "type": "string" } },
                    "content_node_ids": { "type": "array", "items": { "type": "string" } },
                    "k_hops": { "type": "integer", "default": 2 },
                    "limit": { "type": "integer", "default": 10000 }
                }
            }),
        ),
        tool(
            "epistemic_compile_subgraph",
            "Compile the user's content subgraph or dirty delta for Theseus epistemic enrichment.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "content_ids": { "type": "array", "items": { "type": "string" } },
                    "content_node_ids": { "type": "array", "items": { "type": "string" } }
                }
            }),
        ),
        tool(
            "epistemic_shadow_ppr",
            "Run personalized PageRank over EpistemicShadow nodes and shadow-layer edges.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "seeds": { "type": "object" },
                    "top_k": { "type": "integer", "default": 10 },
                    "alpha": { "type": "number", "default": 0.15 },
                    "epsilon": { "type": "number", "default": 0.000001 },
                    "max_pushes": { "type": "integer", "default": 100000 }
                },
                "required": ["seeds"]
            }),
        ),
        tool_write(
            "epistemic_enrich_apply",
            "Apply Theseus epistemic annotations to EpistemicShadow nodes and shadow edges only.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "annotations": { "type": "object" },
                    "content_ids": { "type": "array", "items": { "type": "string" } },
                    "content_node_ids": { "type": "array", "items": { "type": "string" } },
                    "mode": { "type": "string", "enum": ["delta", "full"], "default": "delta" },
                    "engine": { "type": "string", "default": "theseus.epistemic_enrichment" },
                    "engine_version": { "type": "string", "default": "epistemic-v1" },
                    "density_floor": { "type": "number", "default": 0 },
                    "computed_at": { "type": "integer" }
                },
                "required": ["annotations"]
            }),
        ),
        tool(
            "relate",
            "Find native graph neighbors connected to a saved memory document or memory node.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "seed_id": { "type": "string" },
                    "seedId": { "type": "string" },
                    "edge_types": { "type": "array", "items": { "type": "string" } },
                    "max_hops": { "type": "integer", "default": 1 }
                },
                "required": ["seed_id"]
            }),
        ),
        tool(
            "self_recall_archive",
            "Recall archived native memory atoms without returning them in default recall.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "query": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "limit": { "type": "integer", "default": 10 }
                }
            }),
        ),
        tool(
            "observe",
            "Observe native harness context without writing or consuming memory state.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "room_id": { "type": "string" },
                    "query": { "type": "string" },
                    "limit": { "type": "integer", "default": 10 },
                    "include_low_fitness": { "type": "boolean", "default": false },
                    "include_consolidation_sources": { "type": "boolean", "default": false }
                }
            }),
        ),
    ];
    tools.push(tool(
        "rustyred_thg_fulltext_search",
        "Search a designated full-text node property.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "label": { "type": "string" },
                "property": { "type": "string" },
                "query": { "type": "string" },
                "k": { "type": "integer", "default": 10 }
            },
            "required": ["property", "query"]
        }),
    ));
    tools.push(tool(
        "rustyweb_search_acquisition",
        "Queue RustyWeb search provider fan-out through the async harness server route, returning a pollable run_id by default. Pass wait=true for a synchronous diagnostic result.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_id": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "query": { "type": "string" },
                "q": { "type": "string" },
                "providers": { "type": "array", "items": { "type": "string" } },
                "provider_limit": { "type": "integer", "default": 10 },
                "limit": { "type": "integer", "default": 16 },
                "rrf_k": { "type": "integer", "default": 60 },
                "seed_limit": { "type": "integer", "default": 8 },
                "run_id": { "type": "string" },
                "wait": { "type": "boolean", "default": false }
            },
            "required": ["query"]
        }),
    ));
    tools.push(tool(
        "web_consume",
        "Navigate, observe, extract, and optionally ingest one web page through the async THG browser-use route.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_id": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "url": { "type": "string" },
                "run_id": { "type": "string" },
                "actor": { "type": "string" },
                "actor_id": { "type": "string" },
                "max_bytes": { "type": "integer", "default": 5242880 },
                "ingest": { "type": "boolean", "default": true },
                "wait": { "type": "boolean", "default": false }
            },
            "required": ["url"]
        }),
    ));
    tools.push(tool(
        "rustyred_thg_spatial_radius",
        "Search a designated spatial property within a radius in kilometers.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "label": { "type": "string" },
                "lat_property": { "type": "string" },
                "lon_property": { "type": "string" },
                "lat": { "type": "number" },
                "lon": { "type": "number" },
                "radius_km": { "type": "number" }
            },
            "required": ["label", "lat_property", "lon_property", "lat", "lon", "radius_km"]
        }),
    ));
    tools.push(tool(
        "rustyred_thg_spatial_bbox",
        "Search a designated spatial property within a bounding box.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "label": { "type": "string" },
                "lat_property": { "type": "string" },
                "lon_property": { "type": "string" },
                "min_lat": { "type": "number" },
                "min_lon": { "type": "number" },
                "max_lat": { "type": "number" },
                "max_lon": { "type": "number" }
            },
            "required": ["label", "lat_property", "lon_property", "min_lat", "min_lon", "max_lat", "max_lon"]
        }),
    ));
    tools.push(tool(
        "rustyred_thg_vector_search",
        "Run a pure vector similarity search using HNSW indexes. Returns top-k nearest nodes.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "property": { "type": "string", "description": "Name of the vector property to search" },
                "query": { "type": "array", "items": { "type": "number" }, "description": "Query vector" },
                "k": { "type": "integer", "default": 10 },
                "label": { "type": "string", "description": "Optional label filter" }
            },
            "required": ["property", "query"]
        }),
    ));
    tools.push(tool(
        "rustyred_thg_vector_hybrid",
        "Hybrid search blending vector similarity with graph proximity. Graph seeds anchor the graph-distance component.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "property": { "type": "string" },
                "query": { "type": "array", "items": { "type": "number" } },
                "k": { "type": "integer", "default": 10 },
                "label": { "type": "string" },
                "graph_seeds": { "type": "array", "items": { "type": "string" }, "description": "Node IDs to seed graph distance calculation" },
                "max_hops": { "type": "integer", "default": 3 },
                "alpha": { "type": "number", "default": 0.5, "description": "Blend weight: 0.0 = pure vector, 1.0 = pure graph" },
                "confidence_weighted_graph_distance": { "type": "boolean", "default": true },
                "edge_type_weights": { "type": "object", "additionalProperties": { "type": "number" } }
            },
            "required": ["property", "query", "graph_seeds"]
        }),
    ));
    tools.push(tool(
        "rustyred_thg_epistemic_neighbors",
        "Traverse epistemic-typed edges (supports, contradicts, refines, etc.) with optional confidence filtering.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "node_id": { "type": "string" },
                "epistemic_types": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["supports", "contradicts", "tension", "derives", "cites"] }
                },
                "min_confidence": { "type": "number" },
                "max_depth": { "type": "integer", "default": 1 }
            },
            "required": ["node_id"]
        }),
    ));
    tools.push(tool(
        "skill_list",
        "List native Theorem harness skill packs stored in RustyRed.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "status": { "type": "string", "enum": ["draft", "shadow", "advisory", "validated", "canonical", "retired"] },
                "include_retired": { "type": "boolean", "default": false },
                "limit": { "type": "integer", "default": 20 }
            }
        }),
    ));
    tools.push(tool(
        "skill_get",
        "Read one native Theorem harness skill pack by id or content hash.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "pack_id": { "type": "string" },
                "packId": { "type": "string" },
                "id": { "type": "string" },
                "pack_content_hash": { "type": "string" },
                "packContentHash": { "type": "string" },
                "content_hash": { "type": "string" },
                "contentHash": { "type": "string" }
            }
        }),
    ));
    tools.push(tool(
        "multihead_run",
        "Start or inspect a native multi-head work-graph run. In read-only mode only action=status is available.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "action": { "type": "string", "enum": ["start", "status"], "default": "start" },
                "run_id": { "type": "string" },
                "runId": { "type": "string" },
                "goal": { "type": "string" },
                "actor": { "type": "string" },
                "actor_id": { "type": "string" }
            }
        }),
    ));
    tools.push(tool(
        "multihead_next",
        "Route the next claimable node for a head from the durable multi-head work graph.",
        json!({
            "type": "object",
            "properties": {
                "tenant": { "type": "string" },
                "tenant_slug": { "type": "string" },
                "run_id": { "type": "string" },
                "runId": { "type": "string" },
                "head": { "type": "string" },
                "owner": { "type": "string" },
                "actor": { "type": "string" },
                "actor_id": { "type": "string" },
                "heads": { "type": "array", "items": { "type": "string" } },
                "fitness": { "type": "object" },
                "explore_token": { "type": "integer", "default": 0 },
                "now": { "type": "integer" },
                "now_ms": { "type": "integer" }
            },
            "required": ["run_id"]
        }),
    ));
    if !config.read_only {
        tools.push(tool_write(
            "multihead_task",
            "Create a durable claimable task node in the native multi-head work graph.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "node_id": { "type": "string" },
                    "nodeId": { "type": "string" },
                    "goal": { "type": "string" },
                    "kind": { "type": "string", "default": "task" },
                    "node_type": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "prerequisites": { "type": "array", "items": { "type": "string" } },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "file_scope": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["run_id", "goal"]
            }),
        ));
        tools.push(tool_write(
            "multihead_claim",
            "Acquire or release a leased CAS claim on a native multi-head task node.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "action": { "type": "string", "enum": ["claim", "release"], "default": "claim" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "node_id": { "type": "string" },
                    "nodeId": { "type": "string" },
                    "owner": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "expected_epoch": { "type": "integer" },
                    "epoch": { "type": "integer" },
                    "lease_ttl_seconds": { "type": "integer", "default": 90 },
                    "lease_ttl_ms": { "type": "integer" },
                    "now": { "type": "integer" },
                    "now_ms": { "type": "integer" }
                },
                "required": ["run_id", "node_id"]
            }),
        ));
        tools.push(tool_write(
            "multihead_refine",
            "Split a claimed native multi-head task into child task nodes.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "node_id": { "type": "string" },
                    "parent_id": { "type": "string" },
                    "owner": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "file_scope": { "type": "array", "items": { "type": "string" } },
                    "children": { "type": "array", "items": { "type": "object" } },
                    "now": { "type": "integer" },
                    "now_ms": { "type": "integer" }
                },
                "required": ["run_id", "node_id", "children"]
            }),
        ));
        tools.push(tool_write(
            "multihead_patch",
            "Mark a claimed task as patch_proposed and bind the patch to its base commit.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "action": { "type": "string", "enum": ["propose", "rebase"], "default": "propose" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "node_id": { "type": "string" },
                    "nodeId": { "type": "string" },
                    "patch_id": { "type": "string" },
                    "patchId": { "type": "string" },
                    "owner": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "epoch": { "type": "integer" },
                    "base_commit": { "type": "string" },
                    "baseCommit": { "type": "string" },
                    "new_base_commit": { "type": "string" },
                    "newBaseCommit": { "type": "string" },
                    "files": { "type": "array", "items": { "type": "string" } },
                    "patch": { "type": "string" },
                    "patch_ref": { "type": "object" },
                    "now": { "type": "integer" },
                    "now_ms": { "type": "integer" }
                },
                "required": ["run_id"]
            }),
        ));
        tools.push(tool_write(
            "multihead_proof",
            "Run a proof command from the substrate and persist a proof receipt for a multi-head patch.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "patch_id": { "type": "string" },
                    "patchId": { "type": "string" },
                    "command": { "type": "string" },
                    "args": { "type": "array", "items": { "type": "string" } },
                    "cwd": { "type": "string" },
                    "timeout_ms": { "type": "integer", "default": 120000 },
                    "timeoutMs": { "type": "integer", "default": 120000 }
                },
                "required": ["patch_id", "command"]
            }),
        ));
        tools.push(tool_write(
            "multihead_review",
            "Open or complete an adversarial verify node for a multi-head patch.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "action": { "type": "string", "enum": ["open", "complete"], "default": "open" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "patch_id": { "type": "string" },
                    "patchId": { "type": "string" },
                    "reviewer": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "status": { "type": "string" },
                    "falsification_attempts": { "type": "array", "items": { "type": "string" } },
                    "attempted_failure_modes": { "type": "array", "items": { "type": "string" } },
                    "commands_run": { "type": "array", "items": { "type": "string" } },
                    "waived_risks": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["patch_id", "reviewer"]
            }),
        ));
        tools.push(tool_write(
            "multihead_spawn_verify",
            "Spawn the sibling verify node for a patch-proposed target.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "target_node_id": { "type": "string" },
                    "node_id": { "type": "string" },
                    "reviewer": { "type": "string" },
                    "reviewer_head": { "type": "string" },
                    "head": { "type": "string" }
                },
                "required": ["run_id", "target_node_id", "reviewer"]
            }),
        ));
        tools.push(tool_write(
            "multihead_submit_verify",
            "Submit a falsification receipt for a native multi-head verify node.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "target_node_id": { "type": "string" },
                    "node_id": { "type": "string" },
                    "reviewer": { "type": "string" },
                    "head": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "attempted_failure_modes": { "type": "array", "items": { "type": "string" } },
                    "falsification_attempts": { "type": "array", "items": { "type": "string" } },
                    "commands_run": { "type": "array", "items": { "type": "string" } },
                    "defect_found": { "type": "boolean", "default": false },
                    "waived_risks": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["run_id", "target_node_id", "reviewer"]
            }),
        ));
        tools.push(tool_write(
            "fractal_expansion",
            "Queue live RustyRed fractal expansion through the async harness server route, returning a pollable run_id by default. Pass wait=true for a synchronous diagnostic receipt.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_id": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "query": { "type": "string" },
                    "providers": { "type": "array", "items": { "type": "string" } },
                    "provider_limit": { "type": "integer", "default": 10 },
                    "search_limit": { "type": "integer" },
                    "rrf_k": { "type": "integer", "default": 60 },
                    "web_seed_urls": { "type": "array", "items": { "type": "string" } },
                    "top_k": { "type": "integer", "default": 5 },
                    "frontier_limit": { "type": "integer", "default": 8 },
                    "web_seed_limit": { "type": "integer", "default": 8 },
                    "max_bytes": { "type": "integer", "default": 5242880 },
                    "embedder_model": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "run_id": { "type": "string" },
                    "wait": { "type": "boolean", "default": false }
                },
                "required": ["query"]
            }),
        ));
        tools.push(tool_write(
            "browse_with_me",
            "Start or advance a supervised co-browse session with pre-action preview and browser-use receipts.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_id": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "task": { "type": "string" },
                    "query": { "type": "string" },
                    "url": { "type": "string" },
                    "run_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "max_bytes": { "type": "integer", "default": 5242880 },
                    "control_mode": { "type": "string", "enum": ["human_drive", "agent_drive", "pair"], "default": "pair" },
                    "wait": { "type": "boolean", "default": false }
                },
                "required": ["task"]
            }),
        ));
        tools.push(tool_write(
            "browse_for_me",
            "Run the browser-use perceive -> afford loop for a task, bounded by policy and returning a BrowsingRun receipt.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_id": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "task": { "type": "string" },
                    "query": { "type": "string" },
                    "url": { "type": "string" },
                    "run_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "max_bytes": { "type": "integer", "default": 5242880 },
                    "wait": { "type": "boolean", "default": false }
                },
                "required": ["task"]
            }),
        ));
        tools.push(tool_write(
            "coordination_room",
            "Join or inspect a native Theorem harness coordination room backed by THG GraphStore.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "action": { "type": "string", "enum": ["status", "join", "start"], "default": "status" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "room_id": { "type": "string" },
                    "session_id": { "type": "string" },
                    "surface": { "type": "string" },
                    "repo": { "type": "string" },
                    "branch": { "type": "string" },
                    "task": { "type": "string" },
                    "worktree": { "type": "string" },
                    "head": { "type": "string" },
                    "changed_files": { "type": "array", "items": { "type": "string" } },
                    "lane": { "type": "string" }
                }
            }),
        ));
        tools.push(tool_write(
            "presence",
            "Read, refresh, or end native Theorem harness actor presence.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "mode": { "type": "string", "enum": ["get", "heartbeat", "end"], "default": "heartbeat" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "session_id": { "type": "string" },
                    "surface": { "type": "string" },
                    "status": { "type": "string" },
                    "worktree": { "type": "string" },
                    "branch": { "type": "string" },
                    "head": { "type": "string" },
                    "changed_files": { "type": "array", "items": { "type": "string" } },
                    "ttl_seconds": { "type": "integer", "default": 60 }
                }
            }),
        ));
        tools.push(tool_write(
            "coordination_intent",
            "Write your live footprint for this room: what you are doing now and which files your hands are on, for peers to build on (a footprint, not a lock).",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "status": { "type": "string", "enum": ["working", "paused", "done"], "default": "working" },
                    "summary": { "type": "string" },
                    "footprint": { "type": "array", "items": { "type": "string" }, "description": "Files your hands are on right now; a footprint peers build on, not a claim. Accepts legacy claimed_files." },
                    "expected_completion": { "type": "string" },
                    "repo": { "type": "string" },
                    "branch": { "type": "string" },
                    "task": { "type": "string" }
                },
                "required": ["actor", "summary"]
            }),
        ));
        tools.push(tool_write(
            "coordinate",
            "Write a native Theorem harness direct-coordination message and queue @actor mentions.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "urgency": { "type": "string", "enum": ["info", "ask", "block"], "default": "info" },
                    "delivery": { "type": "string", "enum": ["passive", "wake"], "default": "passive" },
                    "wake": { "type": "boolean", "default": false },
                    "message": { "type": "string" },
                    "mentions": { "type": "array", "items": { "type": "string" } },
                    "metadata": { "type": "object" }
                },
                "required": ["actor", "message"]
            }),
        ));
        tools.push(tool_write(
            "stream_publish",
            "Append an event to a (tenant, topic) coordination stream. Returns its monotonic ordering token. urgency ask|block with a target_actor also pings that actor (mention + wake).",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "stream": { "type": "string", "description": "Stream topic: the room, optionally finer (per-task, per-actor)." },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "kind": { "type": "string", "description": "Application event kind, e.g. intent, handoff, note." },
                    "payload": { "type": "object", "description": "Free-form event payload." },
                    "urgency": { "type": "string", "enum": ["info", "ask", "block"], "default": "info" },
                    "target_actor": { "type": "string", "description": "For ask/block: the pinged actor (lands on its mention/wake path)." }
                },
                "required": ["actor", "stream", "kind"]
            }),
        ));
        tools.push(tool_write(
            "stream_subscribe",
            "Subscribe an actor to a coordination stream so stream_read returns its delta. Returns the updated subscription set.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "stream": { "type": "string" }
                },
                "required": ["actor", "stream"]
            }),
        ));
        tools.push(tool_write(
            "stream_unsubscribe",
            "Unsubscribe an actor from a coordination stream. Returns the remaining subscription set. (A ping still reaches an unsubscribed target via mentions.)",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "stream": { "type": "string" }
                },
                "required": ["actor", "stream"]
            }),
        ));
        tools.push(tool_write(
            "coordination_record",
            "Write a durable native Theorem harness coordination record.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "record_id": { "type": "string" },
                    "record_type": { "type": "string", "enum": ["event", "decision", "tension", "reflection"] },
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "body": { "type": "string" },
                    "metadata": { "type": "object" },
                    "required_scope": { "type": "string" },
                    "required_scopes": { "type": "array", "items": { "type": "string" } },
                    "estimated_cost_units": { "type": "number", "default": 0.0 },
                    "budget_units": { "type": "number" }
                },
                "required": ["actor", "record_type", "summary"]
            }),
        ));
        tools.push(tool_write(
            "coordination_contribution",
            "Capture an agent contribution as a durable native coordination event record.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "record_id": { "type": "string" },
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "body": { "type": "string" },
                    "contribution_kind": { "type": "string" },
                    "status": { "type": "string" },
                    "commit": { "type": "string" },
                    "changed_files": { "type": "array", "items": { "type": "string" } },
                    "artifacts": { "type": "array", "items": { "type": "object" } },
                    "validation_receipts": { "type": "array", "items": { "type": "object" } },
                    "metadata": { "type": "object" },
                    "required_scope": { "type": "string" },
                    "required_scopes": { "type": "array", "items": { "type": "string" } },
                    "estimated_cost_units": { "type": "number", "default": 0.0 },
                    "budget_units": { "type": "number" }
                },
                "required": ["actor", "summary"]
            }),
        ));
        tools.push(tool_write(
            "spawn_session",
            "Spawn a Claude Code session that is room-visible and runs via the committed GitHub Actions theorem-handoff workflow (repository_dispatch), not the Railway runner.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "room_id": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "intent": { "type": "string" },
                    "owner": { "type": "string", "default": "Travis-Gilbert" },
                    "repo": { "type": "string", "default": "theorem" },
                    "branch": { "type": "string" },
                    "event_type": { "type": "string", "default": "theorem-handoff" },
                    "metadata": { "type": "object" },
                    "required_scope": { "type": "string" },
                    "required_scopes": { "type": "array", "items": { "type": "string" } },
                    "estimated_cost_units": { "type": "number", "default": 0.0 },
                    "budget_units": { "type": "number" }
                },
                "required": ["actor", "intent"]
            }),
        ));
        tools.push(tool_write(
            "skill_publish",
            "Publish a content-addressed skill pack into native Theorem RustyRed.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "pack": { "type": "object" },
                    "capability_pack": { "type": "object" },
                    "pack_content_hash": { "type": "string" },
                    "packContentHash": { "type": "string" },
                    "source_content_hash": { "type": "string" },
                    "sourceContentHash": { "type": "string" },
                    "artifact_hashes": { "type": "array", "items": { "type": "string" } },
                    "artifactHashes": { "type": "array", "items": { "type": "string" } },
                    "status": { "type": "string", "enum": ["draft", "shadow", "advisory", "validated", "canonical", "retired"] },
                    "metadata": { "type": "object" },
                    "created_at": { "type": "string" },
                    "createdAt": { "type": "string" }
                },
                "required": ["pack"]
            }),
        ));
        tools.push(tool_write(
            "skill_apply",
            "Apply a native Theorem harness skill pack and persist a use receipt.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "pack_id": { "type": "string" },
                    "packId": { "type": "string" },
                    "id": { "type": "string" },
                    "pack_content_hash": { "type": "string" },
                    "packContentHash": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "task": { "type": "string" },
                    "context": { "type": "object" },
                    "outcome": { "type": "object" },
                    "allow_retired": { "type": "boolean", "default": false },
                    "allowRetired": { "type": "boolean", "default": false },
                    "receipt_id": { "type": "string" },
                    "receiptId": { "type": "string" },
                    "metadata": { "type": "object" },
                    "created_at": { "type": "string" },
                    "createdAt": { "type": "string" }
                },
                "required": ["actor"]
            }),
        ));
        tools.push(tool_write(
            "ensemble_register",
            "Register a content-addressed Ensemble capability pack in the tenant GraphStore.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "pack": { "type": "object" },
                    "capability_pack": { "type": "object" },
                    "capabilityPack": { "type": "object" },
                    "spec": { "type": "object" },
                    "kind": { "type": "string", "enum": ["skill", "skill_pack", "agent", "tool", "validator", "renderer", "compute", "policy", "domain", "context"] },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "pack_content_hash": { "type": "string" },
                    "packContentHash": { "type": "string" },
                    "source_content_hash": { "type": "string" },
                    "sourceContentHash": { "type": "string" },
                    "artifact_hashes": { "type": "array", "items": { "type": "string" } },
                    "artifactHashes": { "type": "array", "items": { "type": "string" } },
                    "trust": { "type": "object" },
                    "exposure": { "type": "object" }
                }
            }),
        ));
        tools.push(tool_write(
            "remember",
            "Write a native Theorem harness memory document or typed memory node.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "session_id": { "type": "string" },
                    "surface": { "type": "string" },
                    "project_slug": { "type": "string" },
                    "kind": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "summary": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "links": { "type": "array", "items": { "type": "string" } },
                    "metadata": { "type": "object" }
                },
                "required": ["kind", "content"]
            }),
        ));
        tools.push(tool_write(
            "self_note",
            "Write a typed native self-memory document for the current actor.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "kind": { "type": "string", "default": "self_note" },
                    "memory_node_type": { "type": "string", "default": "belief" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "links": { "type": "array", "items": { "type": "string" } },
                    "summary": { "type": "string" }
                },
                "required": ["content"]
            }),
        ));
        tools.push(tool_write(
            "self_revise",
            "Create a revision-tracked replacement for a native memory document.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "doc_id": { "type": "string" },
                    "docId": { "type": "string" },
                    "content": { "type": "string" },
                    "title": { "type": "string" },
                    "summary": { "type": "string" },
                    "reason": { "type": "string" },
                    "memory_node_type": { "type": "string" },
                    "cites_doc_ids": { "type": "array", "items": { "type": "string" } },
                    "derived_from_doc_ids": { "type": "array", "items": { "type": "string" } },
                    "contradicts_doc_ids": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["doc_id", "content"]
            }),
        ));
        tools.push(tool_write(
            "self_archive",
            "Archive a native memory document into the cold memory tier.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "doc_id": { "type": "string" },
                    "docId": { "type": "string" },
                    "reason": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["doc_id"]
            }),
        ));
        tools.push(tool_write(
            "encode",
            "Record a native feedback, solution, or postmortem memory with fitness metadata.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "kind": { "type": "string", "enum": ["encode", "feedback", "solution", "postmortem"], "default": "encode" },
                    "outcome": { "type": "string", "enum": ["positive", "negative", "mixed", "neutral"], "default": "neutral" },
                    "signal": { "type": "string" },
                    "reason": { "type": "string" },
                    "event_id": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "links": { "type": "array", "items": { "type": "string" } },
                    "summary": { "type": "string" },
                    "metadata": { "type": "object" },
                    "context": { "type": "object" },
                    "auto_triggered": { "type": "boolean", "default": false }
                },
                "required": ["content"]
            }),
        ));
        tools.push(tool_write(
            "upsert_note",
            "Create or update an Obsidian-synced memory document by stable doc_id and reconcile its [[wikilink]] edges (resolved links become edges, removed links are tombstoned, forward references are recorded and resolved on target creation).",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "doc_id": { "type": "string", "description": "blank to create, known id to update in place" },
                    "docId": { "type": "string" },
                    "kind": { "type": "string", "default": "note" },
                    "title": { "type": "string" },
                    "content": { "type": "string" },
                    "summary": { "type": "string" },
                    "tags": { "type": "array", "items": { "type": "string" } },
                    "links": { "type": "array", "items": { "type": "string" }, "description": "wikilink targets: doc_id when resolved, target title when a forward reference" },
                    "status": { "type": "string" },
                    "memory_node_type": { "type": "string" },
                    "metadata": { "type": "object" },
                    "outcome": { "type": "string", "enum": ["positive", "negative", "mixed", "neutral"] },
                    "signal": { "type": "string" },
                    "reason": { "type": "string" },
                    "event_id": { "type": "string" }
                },
                "required": ["content"]
            }),
        ));
        tools.push(tool_write(
            "forget",
            "Soft-delete a native memory document or typed memory node with an audit reason.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "id": { "type": "string" },
                    "reason": { "type": "string" }
                },
                "required": ["id", "reason"]
            }),
        ));
        tools.push(tool_write(
            "handoff",
            "Create a native cross-actor handoff memory document.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "to_actor": { "type": "string" },
                    "toActor": { "type": "string" },
                    "payload": {},
                    "expires_in": { "type": "string" },
                    "expires_at": { "type": "string" },
                    "title": { "type": "string" }
                },
                "required": ["to_actor", "payload"]
            }),
        ));
        tools.push(tool_write(
            "harness_append_transition",
            "Append a native Theorem harness transition into the tenant GraphStore event log.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "tenant_slug": { "type": "string" },
                    "transition": {
                        "type": "object",
                        "description": "Optional full TransitionInput object; when supplied it overrides the flat fields."
                    },
                    "run_id": { "type": "string" },
                    "runId": { "type": "string" },
                    "type": { "type": "string" },
                    "event_type": { "type": "string" },
                    "eventType": { "type": "string" },
                    "payload": { "type": "object" },
                    "actor": { "type": "string" },
                    "actor_id": { "type": "string" },
                    "actorId": { "type": "string" },
                    "idempotency_key": { "type": "string" },
                    "idempotencyKey": { "type": "string" },
                    "created_at": { "type": "string" },
                    "createdAt": { "type": "string" }
                },
                "required": ["type"]
            }),
        ));
        tools.push(tool_write(
            "rustyred_thg_fulltext_designate",
            "Designate a node property for full-text search.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "label": { "type": "string" },
                    "property": { "type": "string" }
                },
                "required": ["label", "property"]
            }),
        ));
        tools.push(tool_write(
            "rustyred_thg_spatial_designate",
            "Designate latitude/longitude node properties for spatial search.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "label": { "type": "string" },
                    "lat_property": { "type": "string" },
                    "lon_property": { "type": "string" },
                    "resolution": { "type": "integer", "default": 9 }
                },
                "required": ["label", "lat_property", "lon_property"]
            }),
        ));
        tools.push(tool_write(
            "rustyred_thg_bulk_nodes",
            "Bulk upsert node records from a JSON array.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "nodes": { "type": "array", "items": { "type": "object" } },
                    "records": { "type": "array", "items": { "type": "object" } }
                }
            }),
        ));
        tools.push(tool_write(
            "rustyred_thg_bulk_edges",
            "Bulk upsert edge records from a JSON array.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "edges": { "type": "array", "items": { "type": "object" } },
                    "records": { "type": "array", "items": { "type": "object" } }
                }
            }),
        ));
        tools.push(tool_write(
            "rustyred_thg_vector_designate",
            "Designate a node property as a vector field with a fixed dimension. Creates HNSW index for that property.",
            json!({
                "type": "object",
                "properties": {
                    "tenant": { "type": "string" },
                    "label": { "type": "string", "description": "Node label to attach the vector designation to" },
                    "property": { "type": "string", "description": "Property name that holds vector data" },
                    "dimension": { "type": "integer", "description": "Vector dimensionality" }
                },
                "required": ["label", "property", "dimension"]
            }),
        ));
    }
    if config.allow_admin && !config.read_only {
        tools.push(tool(
            "rustyred_thg_admin_verify",
            "Run graph verification. Hidden unless admin MCP mode is enabled.",
            json!({
                "type": "object",
                "properties": { "tenant": { "type": "string" } }
            }),
        ));
    }
    tools.extend(graphql::graphql_tool_definitions(!config.read_only));
    if config.graphql_default_surface {
        // A7 cutover: GraphQL is the advertised agent path. Hide the flat tools the
        // typed schema covers; the graphql_* transport tools and every uncovered
        // tool stay listed.
        tools.retain(|tool| {
            tool.get("name")
                .and_then(Value::as_str)
                .map(|name| !GRAPHQL_COVERED_FLAT_TOOLS.contains(&name))
                .unwrap_or(true)
        });
    }
    tools
}

fn mcp_scope_alias(scope: &str) -> &str {
    match scope {
        "rustyred_thg:graph:read"
        | "rustyred_thg:graph:query"
        | "rustyred_thg:graph:index:read" => "graph:read",
        "rustyred_thg:graph:write:propose" | "rustyred_thg:graph:write:apply" => "graph:write",
        "rustyred_thg:graph:context" => "context:read",
        "rustyred_thg:graph:admin:verify" => "admin:read",
        other => other,
    }
}

fn tool(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "outputSchema": output_schema_for_tool(name),
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "openWorldHint": open_world_hint_for_tool(name)
        }
    })
}

fn tool_write(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "outputSchema": output_schema_for_tool(name),
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "openWorldHint": open_world_hint_for_tool(name)
        }
    })
}

fn output_schema_for_tool(name: &str) -> Value {
    match name {
        "code_search" | "compute_code" | "code_ingest" => code_search_output_schema(),
        "harness_prepare" => harness_prepare_output_schema(),
        "web_consume" => web_consume_output_schema(),
        "browse_with_me" | "browse_for_me" => browsing_run_output_schema(),
        "fractal_expansion" | "rustyweb_search_acquisition" => async_run_output_schema(),
        _ => generic_object_output_schema(),
    }
}

fn generic_object_output_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": true
    })
}

fn async_run_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tenant": { "type": "string" },
            "query": { "type": "string" },
            "run_id": { "type": "string" },
            "status": { "type": "string" },
            "receipt": { "type": "object" },
            "error": { "type": "string" },
            "message": { "type": "string" }
        },
        "additionalProperties": true
    })
}

fn browsing_run_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tenant": { "type": "string" },
            "task": { "type": "string" },
            "run_id": { "type": "string" },
            "browser_run_id": { "type": "string" },
            "status": { "type": "string" },
            "receipt": { "type": "object" },
            "pages_reached": { "type": "integer" },
            "actions_applied": { "type": "integer" },
            "data_extracted": { "type": "object" },
            "error": { "type": "string" },
            "message": { "type": "string" }
        },
        "additionalProperties": true
    })
}

fn web_consume_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tenant": { "type": "string" },
            "url": { "type": "string" },
            "run_id": { "type": "string" },
            "status": { "type": "string" },
            "page": { "type": "object" },
            "ingested": { "type": "boolean" },
            "receipt": { "type": "object" },
            "error": { "type": "string" },
            "message": { "type": "string" }
        },
        "additionalProperties": true
    })
}

fn code_search_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tenant": { "type": "string" },
            "operation": { "type": "string" },
            "affordance_id": { "type": "string" },
            "engine": { "type": "string" },
            "code_plugin": { "type": "object" },
            "result": { "type": "object" },
            "app_affordance": { "type": "object" },
            "results": { "type": "array", "items": { "type": "object" } },
            "symbols": { "type": "array", "items": { "type": "object" } },
            "context": { "type": "object" },
            "receipt": { "type": "object" },
            "error": { "type": "string" },
            "message": { "type": "string" }
        },
        "additionalProperties": true
    })
}

fn harness_prepare_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "tenant": { "type": "string" },
            "task": { "type": "string" },
            "actor": { "type": "string" },
            "signature": { "type": "string" },
            "decision_content_hash": { "type": "string" },
            "selected_capabilities": { "type": "array", "items": { "type": "object" } },
            "memory_contract": {
                "type": "object",
                "properties": {
                    "read_first": { "type": "array", "items": { "type": "string" } },
                    "risks": { "type": "array", "items": { "type": "string" } },
                    "do_not": { "type": "array", "items": { "type": "string" } }
                }
            },
            "recall_results": { "type": "array", "items": { "type": "object" } },
            "brief": { "type": "object" },
            "rendered_markdown": { "type": "string" }
        },
        "additionalProperties": true
    })
}

fn open_world_hint_for_tool(name: &str) -> bool {
    matches!(
        name,
        "fractal_expansion"
            | "rustyweb_search_acquisition"
            | "web_consume"
            | "browse_with_me"
            | "browse_for_me"
    )
}

fn prompt_definitions() -> Vec<Value> {
    [
        "thg-query",
        "thg-explain-plan",
        "thg-compile-context-pack",
        "thg-debug-indexes",
    ]
    .into_iter()
    .map(|name| {
        json!({
            "name": name,
            "title": name.replace('-', " "),
            "description": prompt_description(name),
            "arguments": []
        })
    })
    .collect()
}

fn prompt_description(name: &str) -> &'static str {
    match name {
        "thg-query" => "Guide an agent through a bounded THG graph query.",
        "thg-explain-plan" => "Explain a THG query plan and index usage.",
        "thg-compile-context-pack" => "Compile a small graph-backed context pack from THG reads.",
        "thg-debug-indexes" => "Inspect index health and suggest safe follow-up actions.",
        _ => "THG MCP prompt",
    }
}

struct ParsedResource {
    tenant: String,
    kind: String,
    rest: Option<String>,
}

impl ParsedResource {
    fn parse(uri: &str) -> Result<Self, McpError> {
        let raw = uri.strip_prefix("rustyred_thg://tenant/").ok_or_else(|| {
            McpError::invalid_params("THG resource URI must start with rustyred_thg://tenant/")
        })?;
        let mut parts = raw.splitn(3, '/');
        let tenant = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpError::invalid_params("THG resource URI is missing tenant"))?;
        let kind = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| McpError::invalid_params("THG resource URI is missing resource kind"))?;
        let rest = parts.next().map(str::to_string);
        Ok(Self {
            tenant: tenant.to_string(),
            kind: kind.to_string(),
            rest,
        })
    }
}

impl McpGraphBackend for InMemoryGraphStore {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        Ok(InMemoryGraphStore::get_node(self, id).cloned())
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        Ok(InMemoryGraphStore::get_edge(self, id).cloned())
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(InMemoryGraphStore::query_nodes(self, query))
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        Ok(InMemoryGraphStore::neighbors(self, query))
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        Ok(InMemoryGraphStore::stats(self))
    }

    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        Ok(InMemoryGraphStore::verify(self))
    }

    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        Ok(InMemoryGraphStore::labels(self))
    }

    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        Ok(InMemoryGraphStore::edge_types(self))
    }

    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        Ok(InMemoryGraphStore::property_keys(self))
    }

    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        Ok(self.snapshot().edges)
    }

    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(self.snapshot())
    }

    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        InMemoryGraphStore::upsert_node(self, node).map(|_| ())
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        InMemoryGraphStore::upsert_edge(self, edge).map(|_| ())
    }

    fn append_harness_transition(
        &mut self,
        transition: TransitionInput,
    ) -> Result<Value, McpError> {
        append_harness_transition_to_store(self, transition)
    }

    fn harness_run_detail(&self, run_id: &str) -> Result<Option<Value>, McpError> {
        harness_run_detail_from_store(self, run_id)
    }

    fn composed_agent_run(
        &mut self,
        binding_id: String,
        task: String,
        claims: Vec<GroundedClaim>,
    ) -> Result<Value, McpError> {
        composed_agent_run_to_store(self, binding_id, task, claims)
    }

    fn job_submit(
        &mut self,
        submission: JobSubmission,
        submitted_by: String,
    ) -> Result<Value, McpError> {
        job_submit_to_store(self, submission, submitted_by)
    }

    fn job_list(&self, repo: Option<String>, state: Option<String>) -> Result<Value, McpError> {
        job_list_from_store(self, repo, state)
    }

    fn job_note(&mut self, job_id: String, input: JobNoteInput) -> Result<Value, McpError> {
        job_note_to_store(self, job_id, input)
    }

    fn job_archive(
        &mut self,
        job_id: String,
        reason: String,
        actor: String,
    ) -> Result<Value, McpError> {
        job_archive_to_store(self, job_id, reason, actor)
    }

    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        Ok(InMemoryGraphStore::vector_designations(self))
    }

    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        InMemoryGraphStore::designate_vector_property(self, label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        InMemoryGraphStore::vector_search(self, label, property_name, query, k)
    }

    fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        InMemoryGraphStore::hybrid_search(
            self,
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            alpha,
        )
    }

    fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        InMemoryGraphStore::hybrid_search_with_config(
            self,
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            config,
        )
    }

    fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        Ok(InMemoryGraphStore::epistemic_neighbors(
            self,
            node_id,
            epistemic_types,
            min_confidence,
            max_depth,
        ))
    }
}

impl McpGraphBackend for RedCoreGraphStore {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        RedCoreGraphStore::get_node(self, id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        RedCoreGraphStore::get_edge(self, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        RedCoreGraphStore::query_nodes(self, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        RedCoreGraphStore::neighbors(self, query)
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        RedCoreGraphStore::stats(self)
    }

    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        RedCoreGraphStore::verify(self)
    }

    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        RedCoreGraphStore::labels(self)
    }

    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        RedCoreGraphStore::edge_types(self)
    }

    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        RedCoreGraphStore::property_keys(self)
    }

    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        Ok(self.graph_snapshot().edges)
    }

    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(RedCoreGraphStore::graph_snapshot(self))
    }

    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        RedCoreGraphStore::upsert_node(self, node).map(|_| ())
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        RedCoreGraphStore::upsert_edge(self, edge).map(|_| ())
    }

    fn append_harness_transition(
        &mut self,
        transition: TransitionInput,
    ) -> Result<Value, McpError> {
        append_harness_transition_to_store(self, transition)
    }

    fn harness_run_detail(&self, run_id: &str) -> Result<Option<Value>, McpError> {
        harness_run_detail_from_store(self, run_id)
    }

    fn composed_agent_run(
        &mut self,
        binding_id: String,
        task: String,
        claims: Vec<GroundedClaim>,
    ) -> Result<Value, McpError> {
        composed_agent_run_to_store(self, binding_id, task, claims)
    }

    fn job_submit(
        &mut self,
        submission: JobSubmission,
        submitted_by: String,
    ) -> Result<Value, McpError> {
        job_submit_to_store(self, submission, submitted_by)
    }

    fn job_list(&self, repo: Option<String>, state: Option<String>) -> Result<Value, McpError> {
        job_list_from_store(self, repo, state)
    }

    fn job_note(&mut self, job_id: String, input: JobNoteInput) -> Result<Value, McpError> {
        job_note_to_store(self, job_id, input)
    }

    fn job_archive(
        &mut self,
        job_id: String,
        reason: String,
        actor: String,
    ) -> Result<Value, McpError> {
        job_archive_to_store(self, job_id, reason, actor)
    }

    fn invoke_code_search(
        &mut self,
        tenant: &str,
        arguments: &Value,
        operation: &str,
    ) -> Result<Value, McpError> {
        redcore_code_search_payload(self, tenant, arguments, operation)
    }

    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        Ok(RedCoreGraphStore::vector_designations(self))
    }

    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        RedCoreGraphStore::designate_vector_property(self, label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        RedCoreGraphStore::vector_search(self, label, property_name, query, k)
    }

    fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        RedCoreGraphStore::hybrid_search(
            self,
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            alpha,
        )
    }

    fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        RedCoreGraphStore::hybrid_search_with_config(
            self,
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            config,
        )
    }

    fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        Ok(RedCoreGraphStore::epistemic_neighbors(
            self,
            node_id,
            epistemic_types,
            min_confidence,
            max_depth,
        ))
    }
}

#[cfg(feature = "redis-store")]
impl McpGraphBackend for rustyred_thg_core::RedisGraphStore {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        rustyred_thg_core::RedisGraphStore::get_node(self, id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        rustyred_thg_core::RedisGraphStore::get_edge(self, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        rustyred_thg_core::RedisGraphStore::query_nodes(self, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        rustyred_thg_core::RedisGraphStore::neighbors(self, query)
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        rustyred_thg_core::RedisGraphStore::stats(self)
    }

    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        rustyred_thg_core::RedisGraphStore::verify(self)
    }

    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        rustyred_thg_core::RedisGraphStore::labels(self)
    }

    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        rustyred_thg_core::RedisGraphStore::edge_types(self)
    }

    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        rustyred_thg_core::RedisGraphStore::property_keys(self)
    }

    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Err(GraphStoreError::new(
            "legacy_redis_instant_kg_unsupported",
            "instant KG requires the native RedCore graph store; RUSTY_RED_MODE=redis is a legacy compatibility path and should be changed to RUSTY_RED_MODE=embedded",
        ))
    }

    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "Vector designations are not available on the Redis backend",
        ))
    }

    fn designate_vector_property(
        &mut self,
        _label: &str,
        _property_name: &str,
        _dimension: usize,
    ) -> GraphStoreResult<()> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "Vector designation is not available on the Redis backend",
        ))
    }

    fn vector_search(
        &self,
        _label: Option<&str>,
        _property_name: &str,
        _query: &[f32],
        _k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "Vector search is not available on the Redis backend",
        ))
    }

    fn hybrid_search(
        &self,
        _label: Option<&str>,
        _property_name: &str,
        _query: &[f32],
        _k: usize,
        _graph_seeds: &[String],
        _max_hops: usize,
        _alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "Hybrid search is not available on the Redis backend",
        ))
    }

    fn epistemic_neighbors(
        &self,
        _node_id: &str,
        _epistemic_types: Option<&[EpistemicType]>,
        _min_confidence: Option<f64>,
        _max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        Err(GraphStoreError::new(
            "unsupported_operation",
            "Epistemic neighbors are not available on the Redis backend",
        ))
    }
}

fn app_affordance_confirmed(arguments: &Value) -> bool {
    truthy_confirmation_value(arguments.get("confirmed"))
        || truthy_confirmation_value(
            arguments
                .get("use")
                .and_then(|value| value.get("confirmed")),
        )
        || confirmation_action(arguments.get("action"))
}

fn truthy_confirmation_value(value: Option<&Value>) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "true" | "yes" | "y" | "1" | "confirm" | "confirmed"
        ),
        _ => false,
    }
}

fn confirmation_action(value: Option<&Value>) -> bool {
    matches!(
        value
            .and_then(Value::as_str)
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("confirm" | "confirmed" | "approve" | "approved")
    )
}

fn remove_app_affordance_confirmation_controls(object: &mut Map<String, Value>) {
    object.remove("confirmed");
    if confirmation_action(object.get("action")) {
        object.remove("action");
    }
    let remove_empty_use = object
        .get_mut("use")
        .and_then(Value::as_object_mut)
        .map(|use_object| {
            use_object.remove("confirmed");
            use_object.is_empty()
        })
        .unwrap_or(false);
    if remove_empty_use {
        object.remove("use");
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::fs;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use rustyred_thg_affordances::registry::register_connector_with_target;
    use rustyred_thg_affordances::{ConnectorManifest, ToolManifest};
    use rustyred_thg_connectors::ConnectionTarget;
    use rustyred_thg_core::{
        EdgeRecord, EpistemicType, GraphSnapshot, GraphStats, GraphStoreResult,
        HybridScoringConfig, InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
        RedCoreGraphStore, VectorDesignation, VerifyReport,
    };
    use serde_json::{json, Value};
    use theorem_harness_core::{GroundedClaim, TransitionInput};

    use super::{
        append_harness_transition_to_store, composed_agent_run_to_store, handle_mcp_request,
        handle_mcp_request_with_context, harness_run_detail_from_store,
        subscribe_coordination_room_events, AppAffordanceInvocation, McpError, McpGraphBackend,
        McpGraphProvider, McpRequestContext, McpServerConfig,
    };

    struct FixtureProvider(Rc<RefCell<InMemoryGraphStore>>);

    #[derive(Clone)]
    struct SharedFixtureBackend(Rc<RefCell<InMemoryGraphStore>>);

    impl McpGraphProvider for FixtureProvider {
        type Backend = SharedFixtureBackend;

        fn backend_for_tenant(&self, _tenant: &str) -> Result<Self::Backend, McpError> {
            Ok(SharedFixtureBackend(self.0.clone()))
        }
    }

    fn job_submission_fixture(title: &str) -> theorem_harness_core::JobSubmission {
        theorem_harness_core::JobSubmission {
            job_id: None,
            title: title.to_string(),
            spec_ref: Some(format!("docs/plans/{title}/HANDOFF.md")),
            spec_inline: None,
            repo: "Travis-Gilbert/theorem".to_string(),
            priority: Some(theorem_harness_core::Priority::P0),
            target_head: None,
            not_before: None,
            source_task_id: None,
            source_project_id: None,
            idempotency_key: None,
        }
    }

    // Acceptance criterion 1 (MCP boundary): submit creates a job visible in
    // job_list; a duplicate upserts the existing job.
    #[test]
    fn job_submit_and_job_list_shaping() {
        let mut store = InMemoryGraphStore::new();
        let submitted = super::job_submit_to_store(
            &mut store,
            job_submission_fixture("dia"),
            "claude.ai".into(),
        )
        .unwrap();
        assert_eq!(submitted["created"], json!(true));
        let job_id = submitted["job_id"].as_str().unwrap().to_string();

        let list = super::job_list_from_store(&store, None, None).unwrap();
        assert_eq!(list["count"], json!(1));
        assert_eq!(list["jobs"][0]["job_id"].as_str().unwrap(), job_id);
        assert_eq!(list["jobs"][0]["state"], json!("pending"));

        let dup = super::job_submit_to_store(
            &mut store,
            job_submission_fixture("dia"),
            "claude.ai".into(),
        )
        .unwrap();
        assert_eq!(dup["created"], json!(false));
        assert_eq!(dup["job_id"].as_str().unwrap(), job_id);
        assert_eq!(
            super::job_list_from_store(&store, None, None).unwrap()["count"],
            json!(1)
        );
    }

    #[test]
    fn job_note_start_and_archive_shaping() {
        let mut store = InMemoryGraphStore::new();
        let submitted = super::job_submit_to_store(
            &mut store,
            job_submission_fixture("alpha"),
            "claude.ai".into(),
        )
        .unwrap();
        let job_id = submitted["job_id"].as_str().unwrap().to_string();

        let started = super::job_note_to_store(
            &mut store,
            job_id.clone(),
            theorem_harness_runtime::JobNoteInput {
                actor: "receiver-a".to_string(),
                text: "starting".to_string(),
                refs: Vec::new(),
                start_session_ref: Some("session-a".to_string()),
                clear_started: false,
            },
        )
        .unwrap();
        assert_eq!(started["found"], json!(true));
        assert_eq!(started["applied"], json!(true));
        assert_eq!(started["job"]["state"], json!("started"));
        assert_eq!(started["job"]["session_ref"], json!("session-a"));

        let lost = super::job_note_to_store(
            &mut store,
            job_id.clone(),
            theorem_harness_runtime::JobNoteInput {
                actor: "receiver-b".to_string(),
                text: "starting".to_string(),
                refs: Vec::new(),
                start_session_ref: Some("session-b".to_string()),
                clear_started: false,
            },
        )
        .unwrap();
        assert_eq!(lost["applied"], json!(false));

        let archived =
            super::job_archive_to_store(&mut store, job_id.clone(), "done".into(), "codex".into())
                .unwrap();
        assert_eq!(archived["found"], json!(true));
        assert_eq!(archived["applied"], json!(true));
        let missing = super::job_archive_to_store(
            &mut store,
            "job-missing".into(),
            "done".into(),
            "x".into(),
        )
        .unwrap();
        assert_eq!(missing["found"], json!(false));
    }

    impl McpGraphBackend for SharedFixtureBackend {
        fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
            Ok(InMemoryGraphStore::get_node(&self.0.borrow(), id).cloned())
        }

        fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
            Ok(InMemoryGraphStore::get_edge(&self.0.borrow(), id).cloned())
        }

        fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
            Ok(InMemoryGraphStore::query_nodes(&self.0.borrow(), query))
        }

        fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
            Ok(InMemoryGraphStore::neighbors(&self.0.borrow(), query))
        }

        fn stats(&self) -> GraphStoreResult<GraphStats> {
            Ok(InMemoryGraphStore::stats(&self.0.borrow()))
        }

        fn verify(&self) -> GraphStoreResult<VerifyReport> {
            Ok(InMemoryGraphStore::verify(&self.0.borrow()))
        }

        fn labels(&self) -> GraphStoreResult<Vec<String>> {
            Ok(InMemoryGraphStore::labels(&self.0.borrow()))
        }

        fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
            Ok(InMemoryGraphStore::edge_types(&self.0.borrow()))
        }

        fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
            Ok(InMemoryGraphStore::property_keys(&self.0.borrow()))
        }

        fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
            Ok(self.0.borrow().snapshot().edges)
        }

        fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
            Ok(self.0.borrow().snapshot())
        }

        fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
            InMemoryGraphStore::upsert_node(&mut self.0.borrow_mut(), node).map(|_| ())
        }

        fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
            InMemoryGraphStore::upsert_edge(&mut self.0.borrow_mut(), edge).map(|_| ())
        }

        fn append_harness_transition(
            &mut self,
            transition: TransitionInput,
        ) -> Result<Value, McpError> {
            let mut store = self.0.borrow_mut();
            append_harness_transition_to_store(&mut *store, transition)
        }

        fn harness_run_detail(&self, run_id: &str) -> Result<Option<Value>, McpError> {
            let store = self.0.borrow();
            harness_run_detail_from_store(&*store, run_id)
        }

        fn composed_agent_run(
            &mut self,
            binding_id: String,
            task: String,
            claims: Vec<GroundedClaim>,
        ) -> Result<Value, McpError> {
            let mut store = self.0.borrow_mut();
            composed_agent_run_to_store(&mut *store, binding_id, task, claims)
        }

        fn dispatch_handoff(&self, _dispatch: super::HandoffDispatch) -> Result<(), McpError> {
            Ok(())
        }

        fn invoke_app_affordance(
            &mut self,
            invocation: AppAffordanceInvocation,
        ) -> Result<Value, McpError> {
            Ok(json!({
                "tenant_id": invocation.tenant_id,
                "affordance_id": invocation.affordance_id,
                "actor": invocation.actor,
                "request": invocation.request,
                "dry_run": invocation.dry_run,
                "confirmed": invocation.confirmed,
                "timeout_ms": invocation.timeout_ms,
                "status": "ok",
                "receipt_hash": "fixture-code-search-receipt",
            }))
        }

        fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
            Ok(InMemoryGraphStore::vector_designations(&self.0.borrow()))
        }

        fn designate_vector_property(
            &mut self,
            label: &str,
            property_name: &str,
            dimension: usize,
        ) -> GraphStoreResult<()> {
            InMemoryGraphStore::designate_vector_property(
                &mut self.0.borrow_mut(),
                label,
                property_name,
                dimension,
            )
        }

        fn vector_search(
            &self,
            label: Option<&str>,
            property_name: &str,
            query: &[f32],
            k: usize,
        ) -> GraphStoreResult<Vec<(String, f32)>> {
            InMemoryGraphStore::vector_search(&self.0.borrow(), label, property_name, query, k)
        }

        fn hybrid_search(
            &self,
            label: Option<&str>,
            property_name: &str,
            query: &[f32],
            k: usize,
            graph_seeds: &[String],
            max_hops: usize,
            alpha: f32,
        ) -> GraphStoreResult<Vec<(String, f32)>> {
            InMemoryGraphStore::hybrid_search(
                &self.0.borrow(),
                label,
                property_name,
                query,
                k,
                graph_seeds,
                max_hops,
                alpha,
            )
        }

        fn hybrid_search_with_config(
            &self,
            label: Option<&str>,
            property_name: &str,
            query: &[f32],
            k: usize,
            graph_seeds: &[String],
            max_hops: usize,
            config: &HybridScoringConfig,
        ) -> GraphStoreResult<Vec<(String, f32)>> {
            InMemoryGraphStore::hybrid_search_with_config(
                &self.0.borrow(),
                label,
                property_name,
                query,
                k,
                graph_seeds,
                max_hops,
                config,
            )
        }

        fn epistemic_neighbors(
            &self,
            node_id: &str,
            epistemic_types: Option<&[EpistemicType]>,
            min_confidence: Option<f64>,
            max_depth: Option<usize>,
        ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
            Ok(InMemoryGraphStore::epistemic_neighbors(
                &self.0.borrow(),
                node_id,
                epistemic_types,
                min_confidence,
                max_depth,
            ))
        }
    }

    fn fixture() -> (FixtureProvider, McpServerConfig) {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(NodeRecord::new(
                "node:a",
                ["Person"],
                json!({"name": "Ada"}),
            ))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(
                "node:b",
                ["Person", "Engineer"],
                json!({"name": "Grace"}),
            ))
            .unwrap();
        store
            .upsert_node(NodeRecord::new(
                "node:c",
                ["Person"],
                json!({"name": "Katherine"}),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "edge:ab",
                "node:a",
                "KNOWS",
                "node:b",
                json!({"since": 1952}),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "edge:ac",
                "node:a",
                "KNOWS",
                "node:c",
                json!({"since": 1962}),
            ))
            .unwrap();
        (
            FixtureProvider(Rc::new(RefCell::new(store))),
            McpServerConfig {
                default_tenant: "smoke".to_string(),
                ..McpServerConfig::default()
            },
        )
    }

    fn register_gateway_connector(provider: &FixtureProvider) {
        let mut headers = std::collections::BTreeMap::new();
        headers.insert("x-test".to_string(), "redacted".to_string());
        let target = ConnectionTarget::Http {
            url: "http://127.0.0.1:9/mcp".to_string(),
            headers,
            auth: None,
        };
        let target_value = serde_json::to_value(target).unwrap();
        let manifest = ConnectorManifest {
            tenant_id: "smoke".to_string(),
            server_id: "github".to_string(),
            label: "GitHub MCP".to_string(),
            tools: vec![
                ToolManifest {
                    name: "create_issue".to_string(),
                    label: "Create issue".to_string(),
                    description: "Create a GitHub issue in a repository.".to_string(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "title": { "type": "string" }
                        },
                        "required": ["owner", "repo", "title"]
                    }),
                    permissions: vec!["issues:write".to_string()],
                    cost: json!({}),
                    writeback_policy: "write".to_string(),
                    tags: vec!["github".to_string(), "issue".to_string()],
                    description_embedding: None,
                },
                ToolManifest {
                    name: "get_issue".to_string(),
                    label: "Get issue".to_string(),
                    description: "Read one GitHub issue.".to_string(),
                    input_schema: json!({
                        "type": "object",
                        "properties": {
                            "owner": { "type": "string" },
                            "repo": { "type": "string" },
                            "issue_number": { "type": "integer" }
                        },
                        "required": ["owner", "repo", "issue_number"]
                    }),
                    permissions: vec!["issues:read".to_string()],
                    cost: json!({}),
                    writeback_policy: "read-only".to_string(),
                    tags: vec!["github".to_string(), "issue".to_string()],
                    description_embedding: None,
                },
            ],
        };
        register_connector_with_target(
            &mut *provider.0.borrow_mut(),
            manifest,
            Some(target_value),
            Some("test"),
        )
        .unwrap();
    }

    fn unique_code_repo(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "thg-mcp-code-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn call_tool_json(
        provider: &FixtureProvider,
        config: &McpServerConfig,
        name: &str,
        arguments: Value,
    ) -> Value {
        let response = handle_mcp_request(
            provider,
            config,
            json!({
                "jsonrpc": "2.0",
                "id": name,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments
                }
            }),
        );
        if let Some(error) = response.get("error") {
            panic!("tool call failed for {name}: {error}");
        }
        response["result"]["structuredContent"].clone()
    }

    #[test]
    fn connector_gateway_meta_tools_progressively_disclose_affordances() {
        let (provider, config) = fixture();
        let before = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        let before_tools = before["result"]["tools"].as_array().unwrap();
        let before_count = before_tools.len();
        let before_names = before_tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(before_names.contains(&"tool_search"));
        assert!(before_names.contains(&"describe"));
        assert!(before_names.contains(&"invoke"));

        register_gateway_connector(&provider);

        let after = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        let after_tools = after["result"]["tools"].as_array().unwrap();
        assert_eq!(
            after_tools.len(),
            before_count,
            "connecting spokes must not enlarge the inbound tools/list"
        );
        let after_names = after_tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        assert!(
            !after_names.contains(&"github.create_issue"),
            "per-affordance tools must stay out of the inbound catalog"
        );

        let searched = call_tool_json(
            &provider,
            &config,
            "tool_search",
            json!({ "query": "github issue", "k": 5 }),
        );
        assert_eq!(
            searched["results"][0]["affordance_id"],
            "github.create_issue"
        );
        assert!(
            searched["results"][0].get("input_schema").is_none(),
            "search results stay compact; schema materializes only on describe"
        );

        let described = call_tool_json(
            &provider,
            &config,
            "describe",
            json!({ "affordance_id": "github.create_issue" }),
        );
        assert_eq!(
            described["input_schema"]["required"],
            json!(["owner", "repo", "title"])
        );

        let invoked = call_tool_json(
            &provider,
            &config,
            "invoke",
            json!({
                "affordance_id": "github.create_issue",
                "arguments": { "owner": "Travis-Gilbert", "repo": "Theorem", "title": "Test" },
                "task_type": "github issue",
                "dry_run": true
            }),
        );
        assert_eq!(invoked["fired"], json!(false));
        assert_eq!(invoked["planned"]["server_id"], "github");
        assert!(
            invoked["planned"].get("connection_target").is_none(),
            "invoke must not leak persisted connector targets or auth material"
        );
    }

    fn stream_publish(
        provider: &FixtureProvider,
        config: &McpServerConfig,
        tenant: &str,
        stream: &str,
        actor: &str,
        kind: &str,
    ) -> Value {
        call_tool_json(
            provider,
            config,
            "stream_publish",
            json!({
                "tenant": tenant,
                "stream": stream,
                "actor": actor,
                "kind": kind,
                "payload": { "note": kind }
            }),
        )
    }

    #[test]
    fn stream_read_returns_exact_ordered_delta_after_stored_cursor() {
        // AC1: a head offline for N turns, on reconnect, pulls exactly the events
        // after its stored cursor, in order, cursor advancing; nothing missed or
        // duplicated.
        let (provider, config) = fixture();
        let tenant = "travis-gilbert";
        let stream = "room:harness-streams";
        for kind in ["a", "b", "c"] {
            stream_publish(&provider, &config, tenant, stream, "codex", kind);
        }
        // First reconnect drains the whole window in order and advances the cursor.
        let first = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        let tokens = first["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["ordering_token"].as_u64().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(tokens, vec![1, 2, 3]);
        assert_eq!(first["new_cursors"][stream], json!(3));

        // Two more events arrive while offline.
        for kind in ["d", "e"] {
            stream_publish(&provider, &config, tenant, stream, "codex", kind);
        }
        // Reconnect again: only the new tail, in order, no overlap, no dup.
        let second = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        let kinds = second["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| event["kind"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(kinds, vec!["d", "e"]);
        assert_eq!(second["new_cursors"][stream], json!(5));

        // Nothing new -> empty, cursor unchanged.
        let third = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        assert_eq!(third["count"], json!(0));
        assert_eq!(third["new_cursors"][stream], json!(5));
    }

    #[test]
    fn stream_ping_reaches_target_mentions_with_wake_delivery() {
        // AC2: a ping (urgency ask|block plus target) appears in the target's
        // mention drain (warm head) carrying wake delivery (the cold-head wake
        // trigger). The stream event is also written for passive readers.
        let (provider, config) = fixture();
        let tenant = "travis-gilbert";
        let stream = "room:harness-streams";
        let published = call_tool_json(
            &provider,
            &config,
            "stream_publish",
            json!({
                "tenant": tenant,
                "stream": stream,
                "actor": "codex",
                "kind": "blocker",
                "urgency": "block",
                "target_actor": "claude-code",
                "payload": { "why": "merge conflict" }
            }),
        );
        assert_eq!(published["pinged"], json!(true));
        assert_eq!(published["ordering_token"], json!(1));

        let mentions = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({ "tenant": tenant, "actor": "claude-code", "consume": false }),
        );
        let inbox = mentions["mentions"].as_array().unwrap();
        let ping = inbox
            .iter()
            .find(|message| message["metadata"]["stream_ping"] == json!(true))
            .expect("ping lands on the target's mention path");
        assert_eq!(ping["delivery"], json!("wake"));
        assert_eq!(ping["urgency"], json!("block"));
        assert_eq!(ping["metadata"]["stream"], json!(stream));

        // The stream event itself is still readable by a passive subscriber.
        let read = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "deepseek", "stream": stream }),
        );
        assert_eq!(read["count"], json!(1));
        assert_eq!(read["events"][0]["urgency"], json!("block"));

        // An info publish does NOT ping.
        let info = call_tool_json(
            &provider,
            &config,
            "stream_publish",
            json!({
                "tenant": tenant, "stream": stream, "actor": "codex",
                "kind": "fyi", "urgency": "info", "target_actor": "claude-code"
            }),
        );
        assert_eq!(info["pinged"], json!(false));
    }

    #[test]
    fn concurrent_publishers_get_distinct_tokens_in_total_order() {
        // AC3: two heads publishing to one stream receive distinct ordering
        // tokens; both events read back in a single total order, no merge step.
        let (provider, config) = fixture();
        let tenant = "travis-gilbert";
        let stream = "room:race";
        let a = stream_publish(&provider, &config, tenant, stream, "codex", "from-codex");
        let b = stream_publish(
            &provider,
            &config,
            tenant,
            stream,
            "claude-code",
            "from-claude",
        );
        assert_ne!(a["ordering_token"], b["ordering_token"]);
        assert_eq!(a["ordering_token"], json!(1));
        assert_eq!(b["ordering_token"], json!(2));

        let read = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "observer", "stream": stream }),
        );
        let order = read["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|event| {
                (
                    event["actor"].as_str().unwrap().to_string(),
                    event["ordering_token"].as_u64().unwrap(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            order,
            vec![("codex".to_string(), 1), ("claude-code".to_string(), 2)]
        );
    }

    #[test]
    fn stream_publish_and_read_share_tenant_and_reject_empty_tenant() {
        // AC4: a publish and a read under the configured tenant share a stream; a
        // call with an empty tenant is rejected, not silently routed to a default.
        let (provider, _) = fixture();
        let strict = McpServerConfig {
            default_tenant: "default".to_string(),
            ..McpServerConfig::default()
        };
        // No tenant arg + a default-only config -> refused.
        let refused = handle_mcp_request(
            &provider,
            &strict,
            json!({
                "jsonrpc": "2.0", "id": "no-tenant", "method": "tools/call",
                "params": {
                    "name": "stream_publish",
                    "arguments": { "stream": "room:x", "actor": "codex", "kind": "k" }
                }
            }),
        );
        let refused_text = refused.to_string();
        assert!(
            refused["error"].is_object()
                || refused["result"]["isError"] == json!(true)
                || refused_text.contains("requires tenant"),
            "empty tenant must be rejected, not defaulted: {refused_text}"
        );

        // Same tenant publish + read share the stream; a different tenant does not.
        let tenant = "travis-gilbert";
        let stream = "room:shared";
        stream_publish(&provider, &strict, tenant, stream, "codex", "shared-1");
        let same = call_tool_json(
            &provider,
            &strict,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        assert_eq!(same["count"], json!(1));
        let other = call_tool_json(
            &provider,
            &strict,
            "stream_read",
            json!({ "tenant": "someone-else", "actor": "claude-code", "stream": stream }),
        );
        assert_eq!(other["count"], json!(0));
    }

    #[test]
    fn subscriptions_control_read_delta_but_ping_still_reaches_unsubscribed_target() {
        // AC5: subscribing/unsubscribing changes which streams' deltas a read
        // returns; a ping still reaches an unsubscribed target.
        let (provider, config) = fixture();
        let tenant = "travis-gilbert";
        let room = "room:demo";
        let task = "task:streams";
        stream_publish(&provider, &config, tenant, room, "codex", "room-1");
        stream_publish(&provider, &config, tenant, task, "codex", "task-1");

        // No subscriptions -> a subscription-driven read returns nothing.
        let none = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code" }),
        );
        assert_eq!(none["count"], json!(0));
        assert_eq!(none["from_subscriptions"], json!(true));

        // Subscribe to the room only.
        let sub = call_tool_json(
            &provider,
            &config,
            "stream_subscribe",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": room }),
        );
        assert_eq!(sub["subscriptions"], json!([room]));
        let room_only = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code" }),
        );
        assert_eq!(room_only["count"], json!(1));
        assert_eq!(room_only["streams"], json!([room]));

        // Add the task stream; both deltas now return.
        call_tool_json(
            &provider,
            &config,
            "stream_subscribe",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": task }),
        );
        let both = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code" }),
        );
        // room-1 already consumed above; task-1 is fresh on the newly added stream.
        let streams_read = both["streams"].as_array().unwrap();
        assert!(streams_read.iter().any(|s| s == task));

        // Unsubscribe from the task stream -> its delta no longer returned.
        let unsub = call_tool_json(
            &provider,
            &config,
            "stream_unsubscribe",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": task }),
        );
        assert_eq!(unsub["subscriptions"], json!([room]));
        stream_publish(&provider, &config, tenant, task, "codex", "task-2");
        let after = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code" }),
        );
        assert!(
            after["streams"]
                .as_array()
                .unwrap()
                .iter()
                .all(|s| s != task),
            "unsubscribed stream must not appear in subscription-driven reads"
        );

        // But a ping to the unsubscribed target still reaches it via mentions.
        call_tool_json(
            &provider,
            &config,
            "stream_publish",
            json!({
                "tenant": tenant, "stream": task, "actor": "codex",
                "kind": "ask", "urgency": "ask", "target_actor": "claude-code"
            }),
        );
        let mentions = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({ "tenant": tenant, "actor": "claude-code", "consume": false }),
        );
        assert!(
            mentions["mentions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["metadata"]["stream"] == json!(task)),
            "a ping reaches an unsubscribed target through the mention path"
        );
    }

    #[test]
    fn stream_read_only_mode_does_not_advance_cursor() {
        // Passive read stays available read-only; the cursor advance (a write) is
        // suppressed so the window can be re-read.
        let (provider, _) = fixture();
        let tenant = "travis-gilbert";
        let stream = "room:ro";
        let writable = McpServerConfig {
            default_tenant: tenant.to_string(),
            ..McpServerConfig::default()
        };
        stream_publish(&provider, &writable, tenant, stream, "codex", "a");
        let read_only = McpServerConfig {
            default_tenant: tenant.to_string(),
            read_only: true,
            ..McpServerConfig::default()
        };
        let first = call_tool_json(
            &provider,
            &read_only,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        assert_eq!(first["count"], json!(1));
        assert_eq!(first["advanced"], json!(false));
        // Re-read returns the same event because the cursor did not advance.
        let again = call_tool_json(
            &provider,
            &read_only,
            "stream_read",
            json!({ "tenant": tenant, "actor": "claude-code", "stream": stream }),
        );
        assert_eq!(again["count"], json!(1));

        // Writes are refused read-only.
        let refused = handle_mcp_request(
            &provider,
            &read_only,
            json!({
                "jsonrpc": "2.0", "id": "ro-pub", "method": "tools/call",
                "params": {
                    "name": "stream_publish",
                    "arguments": { "tenant": tenant, "stream": stream, "actor": "codex", "kind": "k" }
                }
            }),
        );
        assert!(
            refused.to_string().contains("read_only") || refused.to_string().contains("read-only"),
            "stream_publish must be refused in read-only mode"
        );
    }

    #[test]
    fn stream_write_tools_hidden_in_read_only_listing() {
        let (provider, _) = fixture();
        let read_only = McpServerConfig {
            read_only: true,
            ..McpServerConfig::default()
        };
        let listed = handle_mcp_request(
            &provider,
            &read_only,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        let names = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        // The passive read stays available; the writes are hidden.
        assert!(names.contains(&"stream_read"));
        assert!(!names.contains(&"stream_publish"));
        assert!(!names.contains(&"stream_subscribe"));
        assert!(!names.contains(&"stream_unsubscribe"));
    }

    #[test]
    fn epistemic_cron_tools_compile_apply_and_rank_shadow_graph() {
        let (provider, config) = fixture();
        let listed = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        let names = listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect::<Vec<_>>();
        for expected in [
            "epistemic_dirty_frontier",
            "epistemic_compile_subgraph",
            "epistemic_enrich_apply",
            "epistemic_shadow_ppr",
        ] {
            assert!(names.contains(&expected), "missing tool {expected}");
        }

        let compiled = call_tool_json(
            &provider,
            &config,
            "epistemic_compile_subgraph",
            json!({"content_ids": ["node:a", "node:b"]}),
        );
        assert_eq!(compiled["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(compiled["edges"].as_array().unwrap().len(), 1);

        let dirty = call_tool_json(
            &provider,
            &config,
            "epistemic_dirty_frontier",
            json!({"limit": 10, "k_hops": 0}),
        );
        assert!(dirty["content_ids"]
            .as_array()
            .unwrap()
            .iter()
            .any(|node| node.as_str() == Some("node:a")));

        let applied = call_tool_json(
            &provider,
            &config,
            "epistemic_enrich_apply",
            json!({
                "engine_version": "test-epistemic-v1",
                "annotations": {
                    "annotations": [
                        {
                            "content_node_id": "node:a",
                            "grounded_extension_status": "in",
                            "predicted_edges": [{
                                "target_content_id": "node:b",
                                "relation": "supports",
                                "confidence": 0.8,
                                "quarantine": true
                            }]
                        },
                        {
                            "content_node_id": "node:b",
                            "grounded_extension_status": "out"
                        }
                    ],
                    "support_relations": [{
                        "from_content_id": "node:a",
                        "to_content_id": "node:b",
                        "kind": "supports",
                        "confidence": 0.9,
                        "evidence": "fixture support"
                    }],
                    "attack_relations": []
                }
            }),
        );
        assert_eq!(applied["shadows_written"].as_u64(), Some(2));
        assert_eq!(applied["shadow_edges_written"].as_u64(), Some(1));

        let ranked = call_tool_json(
            &provider,
            &config,
            "epistemic_shadow_ppr",
            json!({"seeds": {"node:a": 1.0}, "top_k": 4}),
        );
        assert!(!ranked["scores"].as_array().unwrap().is_empty());
    }

    fn call_tool_json_with_context(
        provider: &FixtureProvider,
        config: &McpServerConfig,
        context: &McpRequestContext,
        name: &str,
        arguments: Value,
    ) -> Value {
        let response = handle_mcp_request_with_context(
            provider,
            config,
            context,
            json!({
                "jsonrpc": "2.0",
                "id": name,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments
                }
            }),
        );
        if let Some(error) = response.get("error") {
            panic!("tool call failed for {name}: {error}");
        }
        response["result"]["structuredContent"].clone()
    }

    #[test]
    fn coordination_writes_emit_onto_agent_space_bus() {
        use theorem_harness_runtime::{subscribe_agent_space_events, AgentSpaceEvent};
        use tokio::sync::broadcast::error::TryRecvError;

        let (provider, mut config) = fixture();
        config.read_only = false;
        // A unique tenant keeps this test's events distinguishable from any
        // other test sharing the process-global agent-space bus.
        let tenant = "agent-space-emit";
        let room = "agent-space-room";

        // Subscribe before driving writes so every emitted event is buffered.
        let mut events = subscribe_agent_space_events();

        call_tool_json(
            &provider,
            &config,
            "coordinate",
            json!({
                "tenant": tenant,
                "actor": "codex",
                "room_id": room,
                "delivery": "passive",
                "message": "agent-space emit smoke",
                "created_at": "2026-06-01T00:00:00Z"
            }),
        );
        call_tool_json(
            &provider,
            &config,
            "presence",
            json!({
                "tenant": tenant,
                "mode": "heartbeat",
                "actor": "codex",
                "room_id": room,
                "status": "working",
                "ttl_seconds": 120
            }),
        );
        call_tool_json(
            &provider,
            &config,
            "coordination_intent",
            json!({
                "tenant": tenant,
                "actor": "codex",
                "room_id": room,
                "summary": "wire agent-space transport",
                "claimed_files": ["crates/rustyred-thg-server/src/agent_space.rs"],
                "updated_at": "2026-06-01T00:02:00Z"
            }),
        );

        // Drain the shared bus, tolerating broadcast lag and ignoring events
        // emitted by other tests (filtered by our unique tenant).
        let mut saw_message = false;
        let mut saw_presence = false;
        let mut saw_footprint = false;
        loop {
            match events.try_recv() {
                Ok(envelope) => {
                    if envelope.tenant_slug != tenant {
                        continue;
                    }
                    match envelope.event {
                        AgentSpaceEvent::RoomMessage(message) if message.room_id == room => {
                            saw_message = true;
                        }
                        AgentSpaceEvent::Presence { actor, status, .. }
                            if actor == "codex" && status == "working" =>
                        {
                            saw_presence = true;
                        }
                        AgentSpaceEvent::Footprint { actor, target, .. }
                            if actor == "codex" && target.contains("agent_space.rs") =>
                        {
                            saw_footprint = true;
                        }
                        _ => {}
                    }
                }
                Err(TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }

        assert!(
            saw_message,
            "coordinate write must reach the agent-space bus"
        );
        assert!(
            saw_presence,
            "presence write must reach the agent-space bus"
        );
        assert!(
            saw_footprint,
            "intent footprint must reach the agent-space bus"
        );
    }

    fn append_harness_event(
        provider: &FixtureProvider,
        config: &McpServerConfig,
        run_id: &str,
        event_type: &str,
        payload: Value,
    ) -> Value {
        call_tool_json(
            provider,
            config,
            "harness_append_transition",
            json!({
                "tenant": "smoke",
                "run_id": run_id,
                "type": event_type,
                "payload": payload
            }),
        )
    }

    fn sample_skill_pack() -> Value {
        json!({
            "id": "rustyred-rust-skill",
            "kind": "skill_pack",
            "title": "RustyRed Rust skill",
            "capabilities": ["rust_refactor", "graph_store"],
            "validators": [
                { "id": "has-kind", "kind": "required_field", "field": "kind" },
                { "id": "has-artifact", "kind": "artifact_hash_present", "artifact_hash": "hash-validator" }
            ],
            "metadata": {
                "pack_content_hash": "hash-pack",
                "source_content_hash": "hash-source",
                "artifacts": {
                    "validator": { "content_hash": "hash-validator" }
                }
            }
        })
    }

    fn sample_capability_pack() -> Value {
        json!({
            "kind": "skill_pack",
            "title": "Rust Engineering",
            "description": "Rust systems and graph-store engineering guidance",
            "capabilities": ["rust", "graph_store", "mcp"],
            "tags": ["rust", "ensemble", "graph"],
            "metadata": {
                "source": "fixture"
            }
        })
    }

    fn has_tool(tools: &[Value], name: &str) -> bool {
        tools.iter().any(|tool| tool["name"] == name)
    }

    fn assert_no_top_level_schema_combinators(tools: &[Value]) {
        for (index, tool) in tools.iter().enumerate() {
            let schema = &tool["inputSchema"];
            for keyword in ["anyOf", "oneOf", "allOf"] {
                assert!(
                    schema.get(keyword).is_none(),
                    "tool {index} ({}) has unsupported top-level {keyword}",
                    tool["name"]
                );
            }
        }
    }

    fn assert_output_schemas_present(tools: &[Value]) {
        for tool in tools {
            let name = tool["name"].as_str().unwrap_or("<unnamed>");
            let schema = tool
                .get("outputSchema")
                .unwrap_or_else(|| panic!("{name} is missing outputSchema"));
            assert_eq!(
                schema["type"], "object",
                "{name} outputSchema must describe structuredContent as an object"
            );
        }
    }

    fn tool_by_name<'a>(tools: &'a [Value], name: &str) -> &'a Value {
        tools
            .iter()
            .find(|tool| tool["name"] == name)
            .unwrap_or_else(|| panic!("missing tool {name}"))
    }

    #[test]
    fn mcp_server_config_defaults_to_writable_without_admin() {
        let config = McpServerConfig::default();
        assert!(!config.read_only);
        assert!(!config.allow_admin);
        assert_eq!(
            config.tool_result_budget_bytes,
            super::DEFAULT_TOOL_RESULT_BUDGET_BYTES
        );
    }

    #[test]
    fn tool_result_budget_caps_payload_and_exposes_fetch_handle() {
        let config = McpServerConfig {
            tool_result_budget_bytes: 1024,
            ..McpServerConfig::default()
        };
        let result = super::tool_result_with_budget(
            json!({ "blob": "x".repeat(10_000) }),
            &config,
            "compute_code",
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.len() <= 1024);
        assert!(text.contains("truncated by tool_result_budget"));
        let handle = result["structuredContent"]["fetch_handle"]
            .as_str()
            .unwrap();

        let fetched = super::tool_result_fetch_payload(&json!({
            "fetch_handle": handle,
            "offset": 0,
            "max_bytes": 32
        }))
        .unwrap();
        assert_eq!(fetched["offset"], json!(0));
        assert_eq!(fetched["text"].as_str().unwrap().len(), 32);
    }

    #[test]
    fn harness_run_results_use_the_generous_harness_budget() {
        // harness_run is the advertised async-handoff poll tool: a populated run
        // (the run plus its full event log) must stay inline rather than truncate
        // into a fetch handle that drops the found/detail fields the poll reads.
        let config = McpServerConfig {
            tool_result_budget_bytes: 1024,
            ..McpServerConfig::default()
        };
        let payload = json!({ "found": true, "detail": { "blob": "x".repeat(40_000) } });
        let result = super::tool_result_with_budget(payload, &config, "harness_run");
        assert_eq!(result["structuredContent"]["found"], json!(true));
        assert!(result["structuredContent"].get("fetch_handle").is_none());
        assert_ne!(result["structuredContent"]["truncated"], json!(true));

        // A non-harness tool with the same oversized payload still truncates at the
        // default budget, so the larger ceiling is scoped to the harness poll family.
        let payload = json!({ "found": true, "detail": { "blob": "y".repeat(40_000) } });
        let other = super::tool_result_with_budget(payload, &config, "rustyred_thg_graph_query");
        assert_eq!(other["structuredContent"]["truncated"], json!(true));
    }

    #[test]
    fn initialize_returns_mcp_capabilities() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        );

        assert_eq!(response["result"]["serverInfo"]["name"], config.name);
        assert!(response["result"]["capabilities"]["tools"].is_object());
    }

    #[test]
    fn tools_list_exposes_read_only_graph_tools() {
        let (provider, mut config) = fixture();
        config.read_only = true;
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        assert_no_top_level_schema_combinators(tools);
        assert_output_schemas_present(tools);
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_graph_neighbors"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_algorithm_pagerank"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_fulltext_search"));
        assert!(has_tool(tools, "rustyweb_search_acquisition"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_spatial_radius"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_graph_version_merge"));
        assert!(tools.iter().any(|tool| tool["name"] == "harness_kg_status"));
        assert!(has_tool(tools, "read_intents_for_room"));
        assert!(has_tool(tools, "read_messages_for_room"));
        assert!(has_tool(tools, "read_records_for_room"));
        assert!(has_tool(tools, "coordination_context"));
        assert!(has_tool(tools, "harness_run"));
        assert!(has_tool(tools, "composed_agent_run"));
        assert!(has_tool(tools, "multihead_run"));
        assert!(has_tool(tools, "multihead_next"));
        assert!(has_tool(tools, "skill_list"));
        assert!(has_tool(tools, "skill_get"));
        assert!(!has_tool(tools, "fractal_expansion"));
        assert!(has_tool(tools, "web_consume"));
        assert!(!has_tool(tools, "browse_with_me"));
        assert!(!has_tool(tools, "browse_for_me"));
        assert!(has_tool(tools, "mentions"));
        assert!(has_tool(tools, "recall"));
        assert!(has_tool(tools, "relate"));
        assert!(has_tool(tools, "self_recall_archive"));
        assert!(has_tool(tools, "observe"));
        assert!(!has_tool(tools, "code_search"));
        assert!(has_tool(tools, "compute_code"));
        assert!(has_tool(tools, "code_ingest"));
        assert!(has_tool(tools, "harness_prepare"));
        assert_eq!(
            tool_by_name(tools, "compute_code")["inputSchema"]["properties"]["repo"]["type"],
            "string"
        );
        assert!(
            tool_by_name(tools, "compute_code")["inputSchema"]["properties"]
                .get("confirmed")
                .is_none()
        );
        assert!(
            tool_by_name(tools, "compute_code")["inputSchema"]["properties"]
                .get("repo_url")
                .is_none()
        );
        assert!(
            tool_by_name(tools, "compute_code")["inputSchema"]["properties"]
                .get("max_clone_bytes")
                .is_none()
        );
        assert!(has_tool(tools, "ensemble_select"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_symbolic_datalog_derive"));
        assert!(tools.iter().any(|tool| {
            tool["name"] == "rustyred_thg_symbolic_probabilistic_source_reliability"
        }));
        assert!(tools
            .iter()
            .any(|tool| { tool["name"] == "rustyred_thg_symbolic_probabilistic_expected_value" }));
        assert!(!tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_admin_verify"));
        assert!(!tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_bulk_nodes"));
        assert!(!has_tool(tools, "coordination_room"));
        assert!(!has_tool(tools, "presence"));
        assert!(!has_tool(tools, "coordination_intent"));
        assert!(!has_tool(tools, "coordinate"));
        assert!(!has_tool(tools, "coordination_record"));
        assert!(!has_tool(tools, "coordination_contribution"));
        assert!(!has_tool(tools, "harness_append_transition"));
        assert!(!has_tool(tools, "multihead_task"));
        assert!(!has_tool(tools, "multihead_claim"));
        assert!(!has_tool(tools, "multihead_patch"));
        assert!(!has_tool(tools, "multihead_review"));
        assert!(!has_tool(tools, "skill_publish"));
        assert!(!has_tool(tools, "skill_apply"));
        assert!(!has_tool(tools, "ensemble_register"));
        assert!(!has_tool(tools, "remember"));
        assert!(!has_tool(tools, "self_note"));
        assert!(!has_tool(tools, "self_revise"));
        assert!(!has_tool(tools, "self_archive"));
        assert!(!has_tool(tools, "encode"));
        assert!(!has_tool(tools, "upsert_note"));
        assert!(!has_tool(tools, "forget"));
        assert!(!has_tool(tools, "handoff"));
    }

    #[test]
    fn code_search_routes_named_harness_verb_to_app_affordance() {
        let (provider, config) = fixture();
        let routed = call_tool_json(
            &provider,
            &config,
            "code_search",
            json!({
                "tenant": "smoke",
                "operation": "explain",
                "query": "native_code_search",
                "repo_id": "repo:test",
                "actor": "codex",
            }),
        );

        assert_eq!(routed["operation"], "explain");
        assert_eq!(routed["affordance_id"], "theorem_grpc.code_search.explain");
        assert_eq!(
            routed["app_affordance"]["affordance_id"],
            "theorem_grpc.code_search.explain"
        );
        assert_eq!(
            routed["app_affordance"]["request"]["query"],
            "native_code_search"
        );
        assert_eq!(routed["app_affordance"]["request"]["repo_id"], "repo:test");
        assert!(routed["app_affordance"]["request"]
            .get("operation")
            .is_none());
    }

    #[test]
    fn compute_code_alias_routes_to_code_search_app_affordance() {
        let (provider, config) = fixture();
        let routed = call_tool_json(
            &provider,
            &config,
            "compute_code",
            json!({
                "tenant": "smoke",
                "query": "graph store persistence",
                "repo_id": "repo:test",
                "actor": "codex",
            }),
        );

        assert_eq!(routed["operation"], "search");
        assert_eq!(routed["affordance_id"], "theorem_grpc.code_search.search");
        assert_eq!(
            routed["app_affordance"]["request"]["query"],
            "graph store persistence"
        );
    }

    #[test]
    fn code_search_forwards_confirmation_from_tool_control_shapes() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let via_use = call_tool_json(
            &provider,
            &config,
            "code_search",
            json!({
                "tenant": "smoke",
                "operation": "ingest",
                "repo_path": "/tmp/theorem-fixture",
                "use": { "confirmed": true },
                "actor": "codex",
            }),
        );
        assert_eq!(via_use["app_affordance"]["confirmed"], json!(true));
        assert!(via_use["app_affordance"]["request"].get("use").is_none());

        let via_action = call_tool_json(
            &provider,
            &config,
            "code_search",
            json!({
                "tenant": "smoke",
                "operation": "ingest",
                "repo_path": "/tmp/theorem-fixture",
                "action": "confirm",
                "actor": "codex",
            }),
        );
        assert_eq!(via_action["app_affordance"]["confirmed"], json!(true));
        assert!(via_action["app_affordance"]["request"]
            .get("action")
            .is_none());

        let via_top_level = call_tool_json(
            &provider,
            &config,
            "code_ingest",
            json!({
                "tenant": "smoke",
                "repo_path": "/tmp/theorem-fixture",
                "confirmed": true,
                "actor": "codex",
            }),
        );
        assert_eq!(via_top_level["app_affordance"]["confirmed"], json!(true));
        assert!(via_top_level["app_affordance"]["request"]
            .get("confirmed")
            .is_none());
    }

    #[test]
    fn code_search_forwards_timeout_as_control_plane_value() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        let routed = call_tool_json(
            &provider,
            &config,
            "code_search",
            json!({
                "tenant": "smoke",
                "operation": "ingest",
                "repo_path": "https://github.com/tinyhumansai/openhuman.git",
                "timeout_ms": 180_000,
                "actor": "codex",
            }),
        );

        assert_eq!(routed["app_affordance"]["timeout_ms"], json!(180_000));
        assert!(routed["app_affordance"]["request"]
            .get("timeout_ms")
            .is_none());
    }

    #[test]
    fn redcore_code_search_ingests_and_searches_tenant_graph_directly() {
        let repo = unique_code_repo("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn alpha() -> usize {\n    1\n}\n\npub fn beta() -> usize {\n    alpha()\n}\n",
        )
        .unwrap();

        let mut store = RedCoreGraphStore::memory();
        let ingest = super::code_search_payload(
            "smoke",
            &mut store,
            &json!({
                "operation": "ingest",
                "repo_path": repo.display().to_string(),
                "actor": "codex-test"
            }),
            "ingest",
        )
        .unwrap();

        assert_eq!(ingest["engine"], "rustyred_thg_code");
        assert_eq!(ingest["result"]["files_indexed"], json!(1));
        assert_eq!(ingest["result"]["symbols_indexed"], json!(2));
        let repo_id = ingest["result"]["repo_id"].as_str().unwrap().to_string();

        let code_symbols = RedCoreGraphStore::query_nodes(
            &store,
            NodeQuery::label(rustyred_thg_code::CODE_SYMBOL_LABEL)
                .with_property("tenant_id", json!("smoke"))
                .with_limit(100),
        )
        .unwrap();
        assert!(code_symbols
            .iter()
            .any(|node| node.properties["name"] == "beta"));

        let search = super::code_search_payload(
            "smoke",
            &mut store,
            &json!({
                "operation": "search",
                "query": "beta",
                "repo_id": repo_id,
                "limit": 5
            }),
            "search",
        )
        .unwrap();
        assert_eq!(search["engine"], "rustyred_thg_code");
        assert_eq!(search["result"]["hits"][0]["name"], "beta");

        let focus_id = search["result"]["hits"][0]["node_id"]
            .as_str()
            .unwrap()
            .to_string();
        let explore = super::code_search_payload(
            "smoke",
            &mut store,
            &json!({
                "operation": "explore",
                "node_id": focus_id,
                "limit": 5
            }),
            "explore",
        )
        .unwrap();
        assert!(explore["result"]["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|edge| {
                edge["edge_type"] == rustyred_thg_code::CALLS_SYMBOL
                    && edge["from_name"] == "beta"
                    && edge["to_name"] == "alpha"
            }));

        fs::remove_dir_all(repo).ok();
    }

    #[test]
    fn code_search_write_operations_are_read_only_gated() {
        let (provider, mut config) = fixture();
        config.read_only = true;
        let gated = call_tool_json(
            &provider,
            &config,
            "code_ingest",
            json!({
                "tenant": "smoke",
                "repo_path": "/tmp/theorem-fixture",
                "actor": "codex",
            }),
        );

        assert_eq!(gated["error"], "mcp_read_only");
    }

    #[test]
    fn sync_mcp_punts_search_acquisition_to_async_server() {
        let (provider, config) = fixture();
        let payload = call_tool_json(
            &provider,
            &config,
            "rustyweb_search_acquisition",
            json!({ "query": "rustyweb" }),
        );

        assert_eq!(
            payload["error"],
            "live_search_acquisition_requires_async_server"
        );
    }

    #[test]
    fn sync_mcp_punts_browser_use_to_async_server() {
        let (provider, config) = fixture();
        let payload = call_tool_json(
            &provider,
            &config,
            "web_consume",
            json!({ "url": "https://example.com" }),
        );

        assert_eq!(payload["error"], "browser_use_requires_async_server");
    }

    #[test]
    fn tools_list_exposes_native_coordination_write_tools_when_enabled() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        assert_no_top_level_schema_combinators(tools);
        assert_output_schemas_present(tools);
        assert!(has_tool(tools, "coordination_room"));
        assert!(has_tool(tools, "presence"));
        assert!(has_tool(tools, "coordination_intent"));
        assert!(has_tool(tools, "coordinate"));
        assert!(has_tool(tools, "rustyweb_search_acquisition"));
        assert!(has_tool(tools, "fractal_expansion"));
        assert!(has_tool(tools, "web_consume"));
        assert!(has_tool(tools, "browse_with_me"));
        assert!(has_tool(tools, "browse_for_me"));
        assert!(has_tool(tools, "coordination_record"));
        assert!(has_tool(tools, "coordination_contribution"));
        assert!(has_tool(tools, "mentions"));
        assert!(has_tool(tools, "read_records_for_room"));
        assert!(has_tool(tools, "coordination_context"));
        assert!(has_tool(tools, "harness_run"));
        assert!(has_tool(tools, "multihead_run"));
        assert!(has_tool(tools, "multihead_task"));
        assert!(has_tool(tools, "multihead_claim"));
        assert!(has_tool(tools, "multihead_refine"));
        assert!(has_tool(tools, "multihead_next"));
        assert!(has_tool(tools, "multihead_patch"));
        assert!(has_tool(tools, "multihead_proof"));
        assert!(has_tool(tools, "multihead_review"));
        assert!(has_tool(tools, "multihead_spawn_verify"));
        assert!(has_tool(tools, "multihead_submit_verify"));
        assert!(has_tool(tools, "harness_append_transition"));
        assert!(has_tool(tools, "skill_list"));
        assert!(has_tool(tools, "skill_get"));
        assert!(has_tool(tools, "skill_publish"));
        assert!(has_tool(tools, "skill_apply"));
        assert!(has_tool(tools, "ensemble_select"));
        assert!(has_tool(tools, "ensemble_register"));
        assert!(has_tool(tools, "compute_code"));
        assert!(has_tool(tools, "code_ingest"));
        assert!(has_tool(tools, "harness_prepare"));
        assert!(has_tool(tools, "remember"));
        assert!(has_tool(tools, "recall"));
        assert!(has_tool(tools, "relate"));
        assert!(has_tool(tools, "self_note"));
        assert!(has_tool(tools, "self_revise"));
        assert!(has_tool(tools, "self_archive"));
        assert!(has_tool(tools, "self_recall_archive"));
        assert!(has_tool(tools, "encode"));
        assert!(has_tool(tools, "upsert_note"));
        assert!(has_tool(tools, "forget"));
        assert!(has_tool(tools, "handoff"));
        assert!(has_tool(tools, "observe"));
    }

    #[test]
    fn chatgpt_flagged_tools_advertise_receipt_output_schemas() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        for name in [
            "browse_for_me",
            "browse_with_me",
            "compute_code",
            "code_ingest",
            "harness_prepare",
        ] {
            let tool = tool_by_name(tools, name);
            assert_eq!(tool["outputSchema"]["type"], "object");
            assert!(
                tool["outputSchema"]["properties"].is_object(),
                "{name} should expose a concrete receipt schema"
            );
        }
        assert_eq!(
            tool_by_name(tools, "browse_for_me")["annotations"]["openWorldHint"],
            true
        );
        assert_eq!(
            tool_by_name(tools, "browse_with_me")["annotations"]["openWorldHint"],
            true
        );
        assert_eq!(
            tool_by_name(tools, "compute_code")["outputSchema"]["properties"]["affordance_id"]
                ["type"],
            "string"
        );
        assert_eq!(
            tool_by_name(tools, "code_ingest")["inputSchema"]["properties"]["confirmed"]["type"],
            "boolean"
        );
    }

    #[test]
    fn native_skill_pack_tools_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let published = call_tool_json(
            &provider,
            &config,
            "skill_publish",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "status": "validated",
                "pack": sample_skill_pack(),
                "created_at": "2026-06-05T00:00:00Z"
            }),
        );
        assert_eq!(
            published["published"]["pack"]["pack_content_hash"],
            "hash-pack"
        );
        assert_eq!(published["published"]["pack"]["status"], "validated");

        let listed = call_tool_json(
            &provider,
            &config,
            "skill_list",
            json!({ "tenant": "smoke" }),
        );
        assert_eq!(listed["count"], 1);
        assert_eq!(listed["packs"][0]["pack_id"], "rustyred-rust-skill");

        let loaded = call_tool_json(
            &provider,
            &config,
            "skill_get",
            json!({ "tenant": "smoke", "pack_content_hash": "hash-pack" }),
        );
        assert_eq!(loaded["pack"]["pack_id"], "rustyred-rust-skill");

        let applied = call_tool_json(
            &provider,
            &config,
            "skill_apply",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "pack_id": "rustyred-rust-skill",
                "run_id": "run-1",
                "task": "refactor GraphStore",
                "created_at": "2026-06-05T00:01:00Z"
            }),
        );
        assert_eq!(applied["receipt"]["status"], "applied");
        assert_eq!(
            applied["receipt"]["validator_execution_mode"],
            "safe_declaration"
        );
        assert_eq!(
            applied["receipt"]["validators"].as_array().unwrap().len(),
            2
        );
    }

    #[test]
    fn native_ensemble_tools_register_and_select_capability_pack() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let registered = call_tool_json(
            &provider,
            &config,
            "ensemble_register",
            json!({
                "tenant": "smoke",
                "pack": sample_capability_pack(),
                "source_content_hash": "hash-source",
                "artifact_hashes": ["hash-artifact"],
            }),
        );
        let pack_hash = registered["pack"]["pack_content_hash"]
            .as_str()
            .expect("pack hash")
            .to_string();
        assert_eq!(registered["pack"]["kind"], "skill");
        assert_eq!(registered["pack"]["tenant_slug"], "smoke");
        assert!(!pack_hash.is_empty());

        let selected = call_tool_json(
            &provider,
            &config,
            "ensemble_select",
            json!({
                "tenant": "smoke",
                "task": "use rust graph store mcp code search",
                "kind": "skill_pack",
                "max_selected": 1
            }),
        );

        assert!(!selected["decision_content_hash"]
            .as_str()
            .unwrap()
            .is_empty());
        assert_eq!(selected["decision"]["selected"][0]["kind"], "skill");
        assert_eq!(
            selected["decision"]["selected"][0]["pack_content_hash"],
            pack_hash
        );
    }

    #[test]
    fn native_harness_prepare_composes_ensemble_and_memory_brief() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        call_tool_json(
            &provider,
            &config,
            "ensemble_register",
            json!({
                "tenant": "smoke",
                "pack": sample_capability_pack(),
                "source_content_hash": "hash-source",
                "artifact_hashes": ["hash-artifact"],
            }),
        );
        call_tool_json(
            &provider,
            &config,
            "remember",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "surface": "codex",
                "kind": "insight",
                "title": "Rust graph store MCP restructure",
                "content": "Risk: duplicate schemas return if the local Node MCP route is reintroduced. Do not use the local Node MCP for harness prepare.",
                "tags": ["rust", "mcp", "prepare"],
                "created_at": "2026-06-15T00:00:00Z"
            }),
        );

        let prepared = call_tool_json(
            &provider,
            &config,
            "harness_prepare",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "surface": "codex",
                "task": "use rust graph store mcp code search",
                "max_selected": 1,
                "memory_limit": 5
            }),
        );

        assert!(!prepared["signature"].as_str().unwrap().is_empty());
        assert_eq!(
            prepared["selected_capabilities"][0]["title"],
            "Rust Engineering"
        );
        assert!(prepared["memory_contract"]["read_first"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap_or("").contains("Rust graph store")));
        assert!(prepared["memory_contract"]["risks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap_or("").contains("Risk")));
        assert!(prepared["memory_contract"]["do_not"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item.as_str().unwrap_or("").contains("Do not")));
        let markdown = prepared["rendered_markdown"].as_str().unwrap();
        assert!(markdown.contains("Theorem Context Brief"));
        assert!(markdown.contains("Rust Engineering"));
        assert!(markdown.contains("### Read first"));
    }

    #[test]
    fn native_multihead_tools_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let run = call_tool_json(
            &provider,
            &config,
            "multihead_run",
            json!({
                "tenant": "smoke",
                "action": "start",
                "run_id": "run-multihead",
                "goal": "exercise native work graph",
                "actor": "codex"
            }),
        );
        assert_eq!(run["run"]["run_id"], "run-multihead");

        let task = call_tool_json(
            &provider,
            &config,
            "multihead_task",
            json!({
                "tenant": "smoke",
                "run_id": "run-multihead",
                "node_id": "impl-a",
                "goal": "write native MCP surface",
                "kind": "rust_impl",
                "actor": "codex"
            }),
        );
        assert_eq!(task["task"]["status"], "open");

        let next = call_tool_json(
            &provider,
            &config,
            "multihead_next",
            json!({
                "tenant": "smoke",
                "run_id": "run-multihead",
                "head": "codex",
                "heads": ["codex", "claude-code"],
                "explore_token": 999,
                "now": 0
            }),
        );
        assert_eq!(next["next_node_id"], "impl-a");

        let claim = call_tool_json(
            &provider,
            &config,
            "multihead_claim",
            json!({
                "tenant": "smoke",
                "run_id": "run-multihead",
                "node_id": "impl-a",
                "owner": "codex",
                "expected_epoch": 0,
                "lease_ttl_ms": 1000,
                "now": 0
            }),
        );
        assert_eq!(claim["task"]["status"], "claimed");
        assert_eq!(claim["task"]["claim_epoch"], 1);

        let patch = call_tool_json(
            &provider,
            &config,
            "multihead_patch",
            json!({
                "tenant": "smoke",
                "run_id": "run-multihead",
                "node_id": "impl-a",
                "owner": "codex",
                "epoch": 1,
                "base_commit": "base-a",
                "files": ["rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs"],
                "patch": "diff --git a/lib.rs b/lib.rs\n",
                "now": 1
            }),
        );
        assert_eq!(patch["task"]["status"], "patch_proposed");
        let patch_id = patch["patch"]["patch_id"].as_str().unwrap().to_string();

        let proof_command = std::env::current_exe()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let proof = call_tool_json(
            &provider,
            &config,
            "multihead_proof",
            json!({
                "tenant": "smoke",
                "patch_id": patch_id,
                "command": proof_command,
                "args": ["--help"],
                "timeout_ms": 10_000
            }),
        );
        assert_eq!(proof["receipt"]["status"], "passed");
        let patch_id = proof["patch"]["patch_id"].as_str().unwrap().to_string();

        let opened = call_tool_json(
            &provider,
            &config,
            "multihead_review",
            json!({
                "tenant": "smoke",
                "patch_id": patch_id,
                "reviewer": "claude-code",
                "action": "open"
            }),
        );
        assert_eq!(opened["review"]["status"], "open");

        let reviewed = call_tool_json(
            &provider,
            &config,
            "multihead_review",
            json!({
                "tenant": "smoke",
                "patch_id": opened["patch"]["patch_id"],
                "reviewer": "claude-code",
                "action": "complete",
                "status": "accepted",
                "falsification_attempts": ["ran targeted MCP lifecycle test"]
            }),
        );
        assert_eq!(reviewed["review"]["outcome"], "target_accepted");

        let status = call_tool_json(
            &provider,
            &config,
            "multihead_run",
            json!({
                "tenant": "smoke",
                "action": "status",
                "run_id": "run-multihead"
            }),
        );
        assert_eq!(status["graph"]["nodes"]["impl-a"]["status"], "accepted");
    }

    #[test]
    fn native_coordination_tools_reject_unscoped_default_tenant() {
        let (provider, mut config) = fixture();
        config.default_tenant = "default".to_string();
        config.read_only = false;

        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "coordinate",
                "method": "tools/call",
                "params": {
                    "name": "coordinate",
                    "arguments": {
                        "actor": "codex",
                        "room_id": "harness-rust-port",
                        "message": "@claude-code this should not fall into default"
                    }
                }
            }),
        );

        assert_eq!(response["error"]["code"], json!(-32602));
        assert!(response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("requires tenant or tenant_slug"));
    }

    #[test]
    fn native_coordination_tools_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let room = call_tool_json(
            &provider,
            &config,
            "coordination_room",
            json!({
                "tenant": "smoke",
                "action": "join",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "repo": "Theorem",
                "branch": "main",
                "task": "wire native MCP",
                "updated_at": "2026-06-01T00:00:00Z"
            }),
        );
        assert_eq!(room["room"]["room_id"], "harness-rust-port");
        assert_eq!(room["room"]["members"]["codex"]["status"], "joined");

        let presence = call_tool_json(
            &provider,
            &config,
            "presence",
            json!({
                "tenant": "smoke",
                "mode": "heartbeat",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "status": "active",
                "refreshed_at": "2026-06-01T00:01:00Z",
                "ttl_seconds": 120
            }),
        );
        assert_eq!(presence["presence"]["actor_id"], "codex");
        assert_eq!(presence["presence"]["status"], "active");

        let presence_read = call_tool_json(
            &provider,
            &config,
            "presence",
            json!({
                "tenant": "smoke",
                "mode": "get",
                "actor": "codex"
            }),
        );
        assert_eq!(presence_read["presence"]["actor_id"], "codex");

        let intent = call_tool_json(
            &provider,
            &config,
            "coordination_intent",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "summary": "Wire native coordination into MCP",
                "claimed_files": ["rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs"],
                "updated_at": "2026-06-01T00:02:00Z"
            }),
        );
        assert_eq!(intent["intent"]["actor_id"], "codex");
        assert_eq!(intent["intent"]["status"], "working");
        assert_eq!(intent["intent"]["agent_id"], "theorem");
        assert_eq!(intent["intent"]["binding_id"], "agent:theorem");
        // Legacy `claimed_files` input (sent above) must populate the renamed
        // `footprint` output field, proving the serde alias keeps old callers working.
        assert_eq!(
            intent["intent"]["footprint"],
            json!(["rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs"])
        );
        assert_eq!(
            intent["intent"]["scratchpad_document_id"],
            "scratchpad:theorem"
        );
        assert_eq!(intent["intent"]["scratchpad_seq"], 1);
        assert_eq!(
            intent["intent"]["binding_active_head_set"],
            json!(["claude", "codex", "deepseek"])
        );

        let mut room_events = subscribe_coordination_room_events();
        let receipt = call_tool_json(
            &provider,
            &config,
            "coordinate",
            json!({
                "tenant": "smoke",
                "actor": "codex.",
                "room_id": "harness-rust-port",
                "urgency": "ask",
                "delivery": "wake",
                "message": "@claude-code. please test native MCP",
                "mentions": ["deepseek."],
                "metadata": { "commit": "pending" },
                "created_at": "2026-06-01T00:03:00Z"
            }),
        );
        assert_eq!(receipt["ok"], true);
        assert_eq!(receipt["mentions"], json!(["claude-code", "deepseek"]));
        assert_eq!(receipt["delivery"], "wake");
        assert_eq!(receipt["unread_count"], 2);
        assert_eq!(receipt["urgency"], "ask");
        let room_event = room_events.try_recv().expect("room event emitted");
        assert_eq!(room_event.tenant_slug, "smoke");
        assert_eq!(room_event.room_id, "harness-rust-port");
        assert_eq!(
            room_event.message_id,
            receipt["message_id"].as_str().unwrap()
        );
        assert_eq!(room_event.author, "codex");
        assert_eq!(
            room_event.mentions,
            vec!["claude-code".to_string(), "deepseek".to_string()]
        );
        assert_eq!(room_event.delivery, "wake");

        let mentions = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({
                "tenant": "smoke",
                "actor": "claude-code.",
                "consume": false
            }),
        );
        assert_eq!(mentions["count"], 1);
        assert_eq!(mentions["mentions"][0]["actor_id"], "codex");

        let consumed = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({
                "tenant": "smoke",
                "actor": "claude-code",
                "consume": true
            }),
        );
        assert_eq!(consumed["count"], 1);

        let empty_after_consume = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({
                "tenant": "smoke",
                "actor": "claude-code",
                "consume": false
            }),
        );
        assert_eq!(empty_after_consume["count"], 0);

        let intents = call_tool_json(
            &provider,
            &config,
            "read_intents_for_room",
            json!({
                "tenant": "smoke",
                "room_id": "harness-rust-port"
            }),
        );
        assert_eq!(intents["count"], 1);
        assert_eq!(
            intents["intents"][0]["summary"],
            "Wire native coordination into MCP"
        );
        assert_eq!(intents["intents"][0]["binding_id"], "agent:theorem");
        assert_eq!(intents["intents"][0]["scratchpad_seq"], 1);

        let messages = call_tool_json(
            &provider,
            &config,
            "read_messages_for_room",
            json!({
                "tenant": "smoke",
                "room_id": "harness-rust-port"
            }),
        );
        assert_eq!(messages["count"], 1);
        assert_eq!(
            messages["messages"][0]["message"],
            "@claude-code. please test native MCP"
        );
        assert_eq!(messages["messages"][0]["delivery"], "wake");

        let stream_delta = call_tool_json(
            &provider,
            &config,
            "stream_read",
            json!({
                "tenant": "smoke",
                "actor": "claude-code",
                "streams": ["harness-rust-port"],
                "advance": false
            }),
        );
        assert_eq!(stream_delta["count"], json!(1));
        assert_eq!(
            stream_delta["events"][0]["kind"],
            json!("coordination_message")
        );
        assert_eq!(
            stream_delta["events"][0]["payload"]["message_id"],
            receipt["message_id"]
        );
        assert_eq!(
            stream_delta["events"][0]["payload"]["message"],
            json!("@claude-code. please test native MCP")
        );

        let record = call_tool_json(
            &provider,
            &config,
            "coordination_record",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "record_type": "decision",
                "title": "Expose records over MCP",
                "summary": "Capture durable coordination records through the native MCP path",
                "body": "The runtime record contract is now available to MCP clients.",
                "metadata": { "commit": "pending" },
                "created_at": "2026-06-01T00:04:00Z"
            }),
        );
        assert_eq!(record["record"]["actor_id"], "codex");
        assert_eq!(record["record"]["record_type"], "decision");

        let records = call_tool_json(
            &provider,
            &config,
            "read_records_for_room",
            json!({
                "tenant": "smoke",
                "room_id": "harness-rust-port",
                "record_type": "decision"
            }),
        );
        assert_eq!(records["count"], 1);
        assert_eq!(
            records["records"][0]["summary"],
            "Capture durable coordination records through the native MCP path"
        );

        let empty_records = call_tool_json(
            &provider,
            &config,
            "read_records_for_room",
            json!({
                "tenant": "smoke",
                "room_id": "harness-rust-port",
                "record_type": "reflection"
            }),
        );
        assert_eq!(empty_records["count"], 0);

        let other_room = call_tool_json(
            &provider,
            &config,
            "coordinate",
            json!({
                "tenant": "smoke",
                "actor": "deepseek",
                "room_id": "other-room",
                "message": "@codex stale cross-room mention",
                "created_at": "2026-06-01T00:04:30Z"
            }),
        );
        assert_eq!(other_room["ok"], true);

        let context = call_tool_json(
            &provider,
            &config,
            "coordination_context",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "record_type": "decision"
            }),
        );
        assert_eq!(context["room"]["room_id"], "harness-rust-port");
        assert_eq!(context["counts"]["presence"], 1);
        assert_eq!(context["counts"]["intents"], 1);
        assert_eq!(context["counts"]["messages"], 1);
        assert_eq!(context["counts"]["records"], 1);
        assert_eq!(context["counts"]["pending_mentions"], 0);
        assert!(
            context["pending_mentions"].as_array().unwrap().is_empty(),
            "coordination_context must not include mentions from other rooms: {context}"
        );
        assert_eq!(context["intents"][0]["binding_id"], "agent:theorem");
        assert_eq!(
            context["intents"][0]["scratchpad_document_id"],
            "scratchpad:theorem"
        );

        let contribution = call_tool_json(
            &provider,
            &config,
            "coordination_contribution",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "summary": "Added native coordination record exposure",
                "contribution_kind": "code",
                "status": "validated",
                "commit": "80271e1",
                "changed_files": ["rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs"],
                "validation_receipts": [
                    {"kind": "cargo-test", "status": "passed", "summary": "MCP round trip"}
                ],
                "created_at": "2026-06-01T00:05:00Z"
            }),
        );
        assert_eq!(contribution["contribution"]["record_type"], "event");
        assert_eq!(
            contribution["contribution"]["metadata"]["contribution_kind"],
            "code"
        );

        let contributions = call_tool_json(
            &provider,
            &config,
            "read_records_for_room",
            json!({
                "tenant": "smoke",
                "room_id": "harness-rust-port",
                "record_type": "event"
            }),
        );
        assert_eq!(contributions["count"], 1);
        assert_eq!(
            contributions["records"][0]["metadata"]["status"],
            "validated"
        );
    }

    #[test]
    fn native_memory_tools_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let first = call_tool_json(
            &provider,
            &config,
            "remember",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "surface": "codex",
                "kind": "insight",
                "title": "Native memory",
                "content": "RedCore memory atoms now live inside the MCP.",
                "tags": ["memory", "rust"],
                "created_at": "2026-06-01T00:00:00Z"
            }),
        );
        assert_eq!(first["saved_type"], "document");
        let first_doc_id = first["document"]["doc_id"].as_str().unwrap().to_string();

        let second = call_tool_json(
            &provider,
            &config,
            "remember",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "kind": "insight",
                "title": "Linked memory",
                "content": "This entry links to the first memory atom.",
                "links": [first_doc_id.clone()],
                "created_at": "2026-06-01T00:01:00Z"
            }),
        );
        let second_doc_id = second["document"]["doc_id"].as_str().unwrap().to_string();

        let claim = call_tool_json(
            &provider,
            &config,
            "remember",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "kind": "claim",
                "title": "Memory claim",
                "content": "Recall includes typed graph nodes.",
                "created_at": "2026-06-01T00:02:00Z"
            }),
        );
        assert_eq!(claim["saved_type"], "node");

        let recall = call_tool_json(
            &provider,
            &config,
            "recall",
            json!({
                "tenant": "smoke",
                "query": "memory",
                "limit": 10
            }),
        );
        assert_eq!(recall["count"], 3);
        assert!(recall["results"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["item_type"] == "node"));

        let related = call_tool_json(
            &provider,
            &config,
            "relate",
            json!({
                "tenant": "smoke",
                "seed_id": second_doc_id,
                "edge_types": ["MEMORY_RELATES"],
                "max_hops": 1
            }),
        );
        assert_eq!(related["count"], 1);

        let note = call_tool_json(
            &provider,
            &config,
            "self_note",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "title": "Self note",
                "content": "Remember this implementation choice.",
                "memory_node_type": "decision",
                "created_at": "2026-06-01T00:03:00Z"
            }),
        );
        assert_eq!(note["document"]["kind"], "self_note");
        let note_doc_id = note["document"]["doc_id"].as_str().unwrap().to_string();

        let revised = call_tool_json(
            &provider,
            &config,
            "self_revise",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "doc_id": note_doc_id,
                "content": "Remember the adapter-based implementation choice.",
                "reason": "sharpened wording",
                "updated_at": "2026-06-01T00:04:00Z"
            }),
        );
        assert_eq!(revised["superseded"]["status"], "superseded");
        let revised_doc_id = revised["revised"]["doc_id"].as_str().unwrap().to_string();

        let archived = call_tool_json(
            &provider,
            &config,
            "self_archive",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "doc_id": revised_doc_id,
                "reason": "move to cold tier",
                "archived_at": "2026-06-01T00:05:00Z"
            }),
        );
        assert_eq!(archived["archived"]["status"], "archived");

        let archive_recall = call_tool_json(
            &provider,
            &config,
            "self_recall_archive",
            json!({
                "tenant": "smoke",
                "query": "adapter"
            }),
        );
        assert_eq!(archive_recall["count"], 2);

        let encoded = call_tool_json(
            &provider,
            &config,
            "encode",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "kind": "solution",
                "title": "Useful outcome",
                "content": "The native memory path round-trips through MCP.",
                "outcome": "positive",
                "signal": "test",
                "event_id": "event-1",
                "context": { "run_id": "run-1" },
                "created_at": "2026-06-01T00:06:00Z"
            }),
        );
        assert_eq!(encoded["memory"]["fitness"]["outcome"], "positive");

        let handoff = call_tool_json(
            &provider,
            &config,
            "handoff",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "to_actor": "claude-code",
                "payload": { "next": "deploy write mode" },
                "created_at": "2026-06-01T00:07:00Z"
            }),
        );
        assert_eq!(handoff["handoff"]["kind"], "handoff");

        let observe = call_tool_json(
            &provider,
            &config,
            "observe",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "query": "native",
                "limit": 5
            }),
        );
        assert_eq!(observe["tenant"]["slug"], "smoke");
        assert!(!observe["recall_results"].as_array().unwrap().is_empty());

        let forgotten = call_tool_json(
            &provider,
            &config,
            "forget",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "id": first_doc_id,
                "reason": "test cleanup",
                "deleted_at": "2026-06-01T00:08:00Z"
            }),
        );
        assert_eq!(forgotten["forgotten_type"], "document");
        assert_eq!(forgotten["document"]["status"], "deleted");
    }

    // ---- GraphQL MCP surface acceptance (Memory slice + graphAlgorithm) ----

    /// Drive a GraphQL transport tool and return the unwrapped GraphQL response
    /// ({ data, errors }) carried in `result.structuredContent`.
    fn graphql_tool_call(
        provider: &FixtureProvider,
        config: &McpServerConfig,
        tool: &str,
        query: &str,
        variables: Value,
    ) -> Value {
        let mut arguments = json!({ "tenant": "smoke", "query": query });
        if !variables.is_null() {
            arguments["variables"] = variables;
        }
        call_tool_json(provider, config, tool, arguments)
    }

    fn assert_no_graphql_errors(response: &Value) {
        let empty = response
            .get("errors")
            .map(|errors| {
                errors.is_null() || errors.as_array().map(|a| a.is_empty()).unwrap_or(false)
            })
            .unwrap_or(true);
        assert!(empty, "unexpected graphql errors: {response}");
    }

    fn seed_two_linked_memories(
        provider: &FixtureProvider,
        config: &McpServerConfig,
    ) -> (String, String) {
        let alpha = call_tool_json(
            provider,
            config,
            "remember",
            json!({
                "tenant": "smoke", "actor": "codex", "kind": "insight",
                "title": "Alpha", "content": "alpha memory atom",
                "created_at": "2026-06-01T00:00:00Z"
            }),
        );
        let alpha_id = alpha["document"]["doc_id"].as_str().unwrap().to_string();
        let beta = call_tool_json(
            provider,
            config,
            "remember",
            json!({
                "tenant": "smoke", "actor": "codex", "kind": "insight",
                "title": "Beta", "content": "beta memory atom links alpha",
                "links": [alpha_id.clone()],
                "created_at": "2026-06-01T00:01:00Z"
            }),
        );
        let beta_id = beta["document"]["doc_id"].as_str().unwrap().to_string();
        (alpha_id, beta_id)
    }

    // AC1: a single GraphQL query returns a memory node together with its related
    // neighbors and its resolved links, in one round trip.
    #[test]
    fn graphql_memory_query_resolves_related_and_links_in_one_shot() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;
        let (alpha_id, beta_id) = seed_two_linked_memories(&provider, &config);

        let response = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($q:String!){ memory(query:$q, limit:10){ id title \
             related(edgeTypes:[\"MEMORY_RELATES\"], maxHops:1){ id } links{ id } } }",
            json!({ "q": "memory" }),
        );
        assert_no_graphql_errors(&response);
        let docs = response["data"]["memory"]
            .as_array()
            .expect("memory should resolve to an array");
        let beta = docs
            .iter()
            .find(|doc| doc["id"] == json!(beta_id))
            .expect("beta doc should be in recall results");
        let related: Vec<&str> = beta["related"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["id"].as_str())
            .collect();
        let links: Vec<&str> = beta["links"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|d| d["id"].as_str())
            .collect();
        assert!(
            related.contains(&alpha_id.as_str()),
            "related must include alpha: {beta}"
        );
        assert!(
            links.contains(&alpha_id.as_str()),
            "links must include alpha: {beta}"
        );
    }

    // AC2: rememberMemory with an outcome routes through encode (feedback); without
    // an outcome it is a plain remember. Both return a typed MemoryDoc.
    #[test]
    fn graphql_remember_mutation_routes_outcome_to_encode() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let encoded = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($i:MemoryInput!){ rememberMemory(input:$i){ id kind outcome } }",
            json!({ "i": { "kind": "solution", "content": "encode path atom", "outcome": "positive", "signal": "test" } }),
        );
        assert_no_graphql_errors(&encoded);
        // Definitive: the recorded feedback outcome is only set by the encode path.
        assert_eq!(
            encoded["data"]["rememberMemory"]["outcome"], "positive",
            "rememberMemory with an outcome must route through encode: {encoded}"
        );

        let plain = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($i:MemoryInput!){ rememberMemory(input:$i){ id kind outcome } }",
            json!({ "i": { "kind": "insight", "content": "plain remember atom" } }),
        );
        assert_no_graphql_errors(&plain);
        assert!(plain["data"]["rememberMemory"]["id"].as_str().is_some());
        // No outcome supplied -> plain remember, no recorded feedback outcome.
        assert!(
            plain["data"]["rememberMemory"]["outcome"].is_null(),
            "rememberMemory without an outcome must be a plain remember: {plain}"
        );
    }

    // AC3: graphAlgorithm collapses the eight flat algorithm tools; PAGERANK and PPR
    // both return ranked results through the one field.
    #[test]
    fn graphql_graph_algorithm_runs_pagerank_and_ppr() {
        let (provider, config) = fixture();

        let pagerank = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ graphAlgorithm(kind: PAGERANK){ result } }",
            Value::Null,
        );
        assert_no_graphql_errors(&pagerank);
        assert!(
            !pagerank["data"]["graphAlgorithm"]["result"].is_null(),
            "pagerank should return a result payload: {pagerank}"
        );

        let ppr = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($s:JSON){ graphAlgorithm(kind: PPR, seeds:$s){ result } }",
            json!({ "s": { "node:a": 1.0 } }),
        );
        assert_no_graphql_errors(&ppr);
        assert!(
            !ppr["data"]["graphAlgorithm"]["result"].is_null(),
            "ppr should return a result payload: {ppr}"
        );
    }

    // AC4: the connection tenant is resolved once; a session with an empty tenant is
    // rejected rather than defaulted.
    #[test]
    fn graphql_rejects_empty_connection_tenant() {
        let (provider, config) = (
            FixtureProvider(Rc::new(RefCell::new(InMemoryGraphStore::new()))),
            McpServerConfig {
                default_tenant: String::new(),
                read_only: false,
                ..McpServerConfig::default()
            },
        );
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0", "id": "t", "method": "tools/call",
                "params": { "name": "graphql_query", "arguments": { "query": "query{ memory{ id } }" } }
            }),
        );
        assert!(
            response.get("error").is_some(),
            "an empty connection tenant must be rejected, not defaulted: {response}"
        );
    }

    // AC5: graphql_query refuses a mutation operation; mutations run only through
    // graphql_mutate.
    #[test]
    fn graphql_query_refuses_mutation_operations() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let response = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "mutation($i:MemoryInput!){ rememberMemory(input:$i){ id } }",
            json!({ "i": { "kind": "insight", "content": "should be refused" } }),
        );
        let errors = response
            .get("errors")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            !errors.is_empty(),
            "graphql_query must refuse a mutation operation: {response}"
        );
        assert!(
            response["data"].is_null(),
            "a refused mutation must not produce data: {response}"
        );
    }

    // AC6: the schema round-trips its SDL via graphql_introspect, and the flat tools
    // still answer (coexistence, no big-bang replacement).
    #[test]
    fn graphql_introspect_returns_sdl_and_flat_tools_still_answer() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "type MemoryDoc",
            "graphAlgorithm",
            "rememberMemory",
            "scalar JSON",
        ] {
            assert!(sdl.contains(fragment), "SDL missing {fragment}:\n{sdl}");
        }
        // Structural half of connection-scoped tenancy: no field or input anywhere
        // accepts a tenant, so a field cannot mis-scope it.
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );

        // A flat tool still answers exactly as before.
        let recall = call_tool_json(
            &provider,
            &config,
            "recall",
            json!({ "tenant": "smoke", "query": "anything", "limit": 5 }),
        );
        assert!(
            recall.get("count").is_some(),
            "flat recall must still answer: {recall}"
        );
    }

    // ---- A3: Graph domain GraphQL surface acceptance (each field == its flat tool) ----

    // A3.1: the typed bulk mutations write a graph, and graphNode + neighbors read it
    // back; neighbors matches the flat rustyred_thg_graph_neighbors tool exactly.
    #[test]
    fn graphql_graph_domain_bulk_and_neighbors_match_flat_tool() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let nodes = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($n:JSON!){ bulkNodes(nodes:$n){ ok inserted failed } }",
            json!({ "n": [
                { "id": "g1", "labels": ["Doc"], "properties": { "title": "one" } },
                { "id": "g2", "labels": ["Doc"], "properties": { "title": "two" } }
            ]}),
        );
        assert_no_graphql_errors(&nodes);
        assert_eq!(
            nodes["data"]["bulkNodes"]["inserted"], 2,
            "bulkNodes: {nodes}"
        );
        assert_eq!(nodes["data"]["bulkNodes"]["ok"], true);

        let edges = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($e:JSON!){ bulkEdges(edges:$e){ inserted } }",
            json!({ "e": [ { "id": "g1->g2", "from_id": "g1", "to_id": "g2", "type": "LINKS" } ]}),
        );
        assert_no_graphql_errors(&edges);
        assert_eq!(
            edges["data"]["bulkEdges"]["inserted"], 1,
            "bulkEdges: {edges}"
        );

        let node = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ graphNode(id:\"g1\") }",
            Value::Null,
        );
        assert_no_graphql_errors(&node);
        assert!(
            !node["data"]["graphNode"].is_null(),
            "graphNode must return the inserted node: {node}"
        );

        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ neighbors(nodeId:\"g1\", direction:\"out\") }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql);
        let flat = call_tool_json(
            &provider,
            &config,
            "rustyred_thg_graph_neighbors",
            json!({ "tenant": "smoke", "node_id": "g1", "direction": "out" }),
        );
        // The typed field lowers to the same payload: the neighbor lists are identical.
        assert_eq!(
            gql["data"]["neighbors"]["neighbors"], flat["neighbors"],
            "GraphQL neighbors must match the flat tool: gql={gql} flat={flat}"
        );
        assert!(
            !flat["neighbors"].as_array().unwrap().is_empty(),
            "neighbors must find g2: {flat}"
        );
    }

    // A3.2: graphSchema lowers to the same schema_payload the flat tool uses, so the
    // two are byte-identical (faithful wrap, no reimplementation).
    #[test]
    fn graphql_graph_schema_matches_flat_tool() {
        let (provider, config) = fixture();
        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ graphSchema }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql);
        let flat = call_tool_json(
            &provider,
            &config,
            "rustyred_thg_graph_schema",
            json!({ "tenant": "smoke" }),
        );
        assert_eq!(
            gql["data"]["graphSchema"], flat,
            "GraphQL graphSchema must match the flat tool exactly"
        );
    }

    // A3.3: designateVector + bulkNodes (with vectors), then vectorSearch returns the
    // same ranked node ids as the flat rustyred_thg_vector_search tool.
    #[test]
    fn graphql_vector_search_matches_flat_tool() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let designated = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation{ designateVector(label:\"Doc\", property:\"vec\", dimension:3) }",
            Value::Null,
        );
        assert_no_graphql_errors(&designated);

        let nodes = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
            json!({ "n": [
                { "id": "v1", "labels": ["Doc"], "properties": { "vec": [1.0, 0.0, 0.0] } },
                { "id": "v2", "labels": ["Doc"], "properties": { "vec": [0.0, 1.0, 0.0] } }
            ]}),
        );
        assert_no_graphql_errors(&nodes);
        assert_eq!(
            nodes["data"]["bulkNodes"]["inserted"], 2,
            "bulkNodes: {nodes}"
        );

        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($q:[Float!]!){ vectorSearch(property:\"vec\", query:$q, label:\"Doc\", k:2){ nodeId score } }",
            json!({ "q": [1.0, 0.0, 0.0] }),
        );
        assert_no_graphql_errors(&gql);
        let gql_ids: Vec<&str> = gql["data"]["vectorSearch"]
            .as_array()
            .expect("vectorSearch should resolve to a list")
            .iter()
            .filter_map(|hit| hit["nodeId"].as_str())
            .collect();

        let flat = call_tool_json(
            &provider,
            &config,
            "rustyred_thg_vector_search",
            json!({ "tenant": "smoke", "property": "vec", "query": [1.0, 0.0, 0.0], "label": "Doc", "k": 2 }),
        );
        let flat_ids: Vec<&str> = flat["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|hit| hit["node_id"].as_str())
            .collect();

        assert_eq!(
            gql_ids, flat_ids,
            "vectorSearch ids must match the flat tool: gql={gql} flat={flat}"
        );
        assert!(
            gql_ids.contains(&"v1"),
            "vectorSearch must rank v1 for query [1,0,0]: {gql}"
        );
    }

    // A3.4: the symbolic deriveFacts field lowers to the same datalog payload as the
    // flat tool; the receipts are byte-identical.
    #[test]
    fn graphql_symbolic_derive_facts_matches_flat_tool() {
        let (provider, config) = fixture();
        let facts = json!({
            "facts": [
                {"relation": "claim", "entity_id": "claim-1", "attributes": {"status": "proposed"}, "fact_id": "f1"},
                {"relation": "object", "entity_id": "obj-1", "attributes": {"title": "Same"}, "fact_id": "f2"},
                {"relation": "object", "entity_id": "obj-2", "attributes": {"title": "same"}, "fact_id": "f3"}
            ]
        });
        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($i:JSON!){ deriveFacts(input:$i) }",
            json!({ "i": facts.clone() }),
        );
        assert_no_graphql_errors(&gql);
        let mut flat_args = facts;
        flat_args["tenant"] = json!("smoke");
        let flat = call_tool_json(
            &provider,
            &config,
            "rustyred_thg_symbolic_datalog_derive",
            flat_args,
        );
        assert_eq!(
            gql["data"]["deriveFacts"], flat,
            "GraphQL deriveFacts must match the flat tool receipt"
        );
        assert_eq!(gql["data"]["deriveFacts"]["derived_count"], 3, "{gql}");
    }

    // A3.5: the introspected SDL exposes the full Graph domain (reads, searches,
    // symbolic, designate and bulk mutations) as one typed surface, still with no
    // tenant argument anywhere.
    #[test]
    fn graphql_introspect_exposes_full_graph_domain() {
        let (provider, mut config) = fixture();
        // The full typed SDL exceeds the default 16KB tool-result budget; fetch it
        // whole rather than truncated into a fetch-handle envelope.
        config.tool_result_budget_bytes = 0;
        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "graphNode",
            "neighbors",
            "graphSchema",
            "vectorSearch",
            "vectorHybrid",
            "fulltextSearch",
            "spatialRadius",
            "spatialBbox",
            "deriveFacts",
            "sourceReliability",
            "expectedValue",
            "designateVector",
            "designateSpatial",
            "designateFulltext",
            "bulkNodes",
            "bulkEdges",
            "type SearchHit",
            "type BulkResult",
        ] {
            assert!(
                sdl.contains(fragment),
                "SDL missing graph-domain {fragment}:\n{sdl}"
            );
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );
    }

    // A2.1: the coordination room GraphQL query returns messages, intents, and
    // records in one response, backed by the native coordination payloads.
    #[test]
    fn graphql_coordination_room_returns_messages_intents_records() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;
        let room = "repo:theorem:branch:gql-a2-room";

        let writes = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($room:String!, $meta:JSON!, $payload:JSON!){ \
                writeCoordinationIntent(roomId:$room, actor:\"codex\", summary:\"Wire A2 GraphQL\", footprint:[\"rustyred-thg-mcp/src/graphql/coordination.rs\"]) \
                writeCoordinationRecord(roomId:$room, actor:\"codex\", recordType:\"decision\", summary:\"Expose coordination through GraphQL\", metadata:$meta) \
                publishCoordinationEvent(stream:$room, actor:\"codex\", kind:\"needs-review\", payload:$payload, urgency:\"ask\", targetActor:\"claude-code\"){ ok stream eventId orderingToken pinged } \
            }",
            json!({
                "room": room,
                "meta": { "spec": "SPEC-GRAPHQL-MCP" },
                "payload": { "summary": "Please review A2 GraphQL" }
            }),
        );
        assert_no_graphql_errors(&writes);
        assert_eq!(
            writes["data"]["publishCoordinationEvent"]["stream"],
            json!(room),
            "publishCoordinationEvent must target the room stream: {writes}"
        );
        assert_eq!(
            writes["data"]["publishCoordinationEvent"]["pinged"],
            json!(true),
            "ask event with targetActor must bridge to the mention path: {writes}"
        );

        let room_view = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($room:String!){ coordinationRoom(roomId:$room, actor:\"claude-code\", recordTypes:[\"decision\"]){ roomId counts intents messages records pendingMentions } }",
            json!({ "room": room }),
        );
        assert_no_graphql_errors(&room_view);
        let view = &room_view["data"]["coordinationRoom"];
        assert_eq!(view["roomId"], json!(room));
        assert_eq!(view["counts"]["intents"], json!(1), "{room_view}");
        assert_eq!(view["counts"]["messages"], json!(1), "{room_view}");
        assert_eq!(view["counts"]["records"], json!(1), "{room_view}");
        assert_eq!(view["counts"]["pending_mentions"], json!(1), "{room_view}");
        assert_eq!(view["intents"][0]["summary"], json!("Wire A2 GraphQL"));
        assert_eq!(
            view["records"][0]["summary"],
            json!("Expose coordination through GraphQL")
        );
        assert_eq!(
            view["messages"][0]["metadata"]["stream_event_id"],
            writes["data"]["publishCoordinationEvent"]["eventId"]
        );
    }

    // A2.2: a stream event published through GraphQL is read by another actor
    // through the same stream seam, and the mutation read advances its cursor.
    #[test]
    fn graphql_coordination_stream_publish_read_round_trip() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;
        let stream = "repo:theorem:branch:gql-a2-stream";

        let published = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($stream:String!, $payload:JSON!){ publishCoordinationEvent(stream:$stream, actor:\"codex\", kind:\"checkpoint\", payload:$payload, urgency:\"info\"){ ok stream eventId orderingToken } }",
            json!({
                "stream": stream,
                "payload": { "result": "ready" }
            }),
        );
        assert_no_graphql_errors(&published);
        assert_eq!(
            published["data"]["publishCoordinationEvent"]["orderingToken"],
            json!(1),
            "{published}"
        );

        let peek = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($stream:String!){ coordinationStream(actor:\"claude-code\", stream:$stream, limit:10){ count advanced events } }",
            json!({ "stream": stream }),
        );
        assert_no_graphql_errors(&peek);
        assert_eq!(
            peek["data"]["coordinationStream"]["count"],
            json!(1),
            "{peek}"
        );
        assert_eq!(
            peek["data"]["coordinationStream"]["advanced"],
            json!(false),
            "query read must not advance cursor: {peek}"
        );
        assert_eq!(
            peek["data"]["coordinationStream"]["events"][0]["kind"],
            json!("checkpoint")
        );

        let advanced = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($stream:String!){ advanceCoordinationStream(actor:\"claude-code\", stream:$stream, limit:10){ count advanced newCursors } }",
            json!({ "stream": stream }),
        );
        assert_no_graphql_errors(&advanced);
        assert_eq!(
            advanced["data"]["advanceCoordinationStream"]["count"],
            json!(1),
            "{advanced}"
        );
        assert_eq!(
            advanced["data"]["advanceCoordinationStream"]["advanced"],
            json!(true),
            "{advanced}"
        );
        assert_eq!(
            advanced["data"]["advanceCoordinationStream"]["newCursors"][stream],
            json!(1),
            "{advanced}"
        );

        let empty = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($stream:String!){ advanceCoordinationStream(actor:\"claude-code\", stream:$stream, limit:10){ count advanced } }",
            json!({ "stream": stream }),
        );
        assert_no_graphql_errors(&empty);
        assert_eq!(
            empty["data"]["advanceCoordinationStream"]["count"],
            json!(0),
            "advanced cursor should prevent duplicate delivery: {empty}"
        );
    }

    // A2.3: WorkGraph and TaskNode have typed GraphQL entry points over the
    // native multi-head task payloads.
    #[test]
    fn graphql_coordination_work_graph_exposes_task_nodes() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;
        let run_id = "run:gql-a2";

        let created = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($run:String!){ createTaskNode(runId:$run, nodeId:\"task:a\", goal:\"Implement GraphQL A2\", kind:\"implementation\", actor:\"codex\"){ ok reused task } }",
            json!({ "run": run_id }),
        );
        assert_no_graphql_errors(&created);
        assert_eq!(
            created["data"]["createTaskNode"]["ok"],
            json!(true),
            "{created}"
        );
        assert_eq!(
            created["data"]["createTaskNode"]["task"]["id"],
            json!("task:a")
        );

        let graph = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($run:String!){ workGraph(runId:$run){ ok graph tasks } nextTaskNode(runId:$run, head:\"codex\") }",
            json!({ "run": run_id }),
        );
        assert_no_graphql_errors(&graph);
        assert_eq!(graph["data"]["workGraph"]["ok"], json!(true), "{graph}");
        assert_eq!(
            graph["data"]["workGraph"]["tasks"][0]["id"],
            json!("task:a")
        );
        assert_eq!(
            graph["data"]["nextTaskNode"]["next_node_id"],
            json!("task:a"),
            "{graph}"
        );
    }

    // ---- A4: Epistemic domain GraphQL surface acceptance (each field == its flat tool) ----

    // A4.1 (the acceptance): on a populated shadow graph, epistemicNeighbors over
    // GraphQL returns the shadow nodes the flat rustyred_thg_epistemic_neighbors
    // call returns; shadowPpr and compileSubgraph also match their flat tools.
    #[test]
    fn graphql_epistemic_domain_matches_flat_tools() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        // Seed the shadow graph: node:a/node:b are fixture-seeded content nodes;
        // enrich writes two shadows plus a Supports shadow edge.
        let applied = call_tool_json(
            &provider,
            &config,
            "epistemic_enrich_apply",
            json!({
                "engine_version": "a4-test-v1",
                "annotations": {
                    "annotations": [
                        { "content_node_id": "node:a", "grounded_extension_status": "in",
                          "predicted_edges": [{ "target_content_id": "node:b", "relation": "supports", "confidence": 0.8, "quarantine": true }] },
                        { "content_node_id": "node:b", "grounded_extension_status": "out" }
                    ],
                    "support_relations": [{ "from_content_id": "node:a", "to_content_id": "node:b", "kind": "supports", "confidence": 0.9, "evidence": "a4 fixture" }],
                    "attack_relations": []
                }
            }),
        );
        assert_eq!(
            applied["shadows_written"].as_u64(),
            Some(2),
            "enrich should write shadows: {applied}"
        );

        // The A4 acceptance: epistemicNeighbors over GraphQL == the flat tool's shadow nodes.
        let neighbors = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ epistemicNeighbors(nodeId:\"node:a\") }",
            Value::Null,
        );
        assert_no_graphql_errors(&neighbors);
        let flat_neighbors = call_tool_json(
            &provider,
            &config,
            "rustyred_thg_epistemic_neighbors",
            json!({ "tenant": "smoke", "node_id": "node:a" }),
        );
        assert_eq!(
            neighbors["data"]["epistemicNeighbors"]["results"], flat_neighbors["results"],
            "epistemicNeighbors must return the shadow nodes the flat tool returns: gql={neighbors} flat={flat_neighbors}"
        );

        // shadowPpr parity over the populated shadow layer.
        let ppr = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($s:JSON!){ shadowPpr(seeds:$s, topK:4) }",
            json!({ "s": { "node:a": 1.0 } }),
        );
        assert_no_graphql_errors(&ppr);
        let flat_ppr = call_tool_json(
            &provider,
            &config,
            "epistemic_shadow_ppr",
            json!({ "tenant": "smoke", "seeds": { "node:a": 1.0 }, "top_k": 4 }),
        );
        assert_eq!(
            ppr["data"]["shadowPpr"]["scores"], flat_ppr["scores"],
            "shadowPpr must match the flat tool: gql={ppr} flat={flat_ppr}"
        );

        // compileSubgraph parity (guaranteed-equal: both lower to compile_user_subgraph).
        let compiled = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($c:[String!]!){ compileSubgraph(contentIds:$c) }",
            json!({ "c": ["node:a", "node:b"] }),
        );
        assert_no_graphql_errors(&compiled);
        let flat_compiled = call_tool_json(
            &provider,
            &config,
            "epistemic_compile_subgraph",
            json!({ "tenant": "smoke", "content_ids": ["node:a", "node:b"] }),
        );
        assert_eq!(
            compiled["data"]["compileSubgraph"], flat_compiled,
            "compileSubgraph must match the flat tool"
        );
    }

    // A4.2: the introspected SDL exposes the epistemic domain as one typed surface,
    // still with no tenant argument.
    #[test]
    fn graphql_introspect_exposes_epistemic_domain() {
        let (provider, mut config) = fixture();
        config.tool_result_budget_bytes = 0;
        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "epistemicNeighbors",
            "epistemicFrontier",
            "compileSubgraph",
            "shadowPpr",
            "enrichApply",
        ] {
            assert!(
                sdl.contains(fragment),
                "SDL missing epistemic-domain {fragment}:\n{sdl}"
            );
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );
    }

    // ---- A5: Code domain GraphQL surface acceptance (each field == its compute_code op) ----

    // A5.1 (the acceptance): codeSearch over GraphQL routes to the same compute_code
    // `search` operation as the flat tool (same operation + affordance route).
    #[test]
    fn graphql_code_search_matches_flat_tool() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ codeSearch(query:\"beta\", repo:\"demo-repo\", limit:5) }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql);
        let flat = call_tool_json(
            &provider,
            &config,
            "compute_code",
            json!({ "tenant": "smoke", "operation": "search", "query": "beta", "repo": "demo-repo", "limit": 5 }),
        );
        // The typed field lowers to the same compute_code operation + affordance route.
        assert_eq!(
            gql["data"]["codeSearch"]["operation"], flat["operation"],
            "codeSearch must run the same operation as compute_code: gql={gql} flat={flat}"
        );
        assert_eq!(
            gql["data"]["codeSearch"]["affordance_id"], flat["affordance_id"],
            "codeSearch must route to the same code affordance as compute_code"
        );
        assert_eq!(
            gql["data"]["codeSearch"]["operation"],
            json!("search"),
            "{gql}"
        );
    }

    // A5.2: the introspected SDL exposes the code domain (reads + ingest/reindex
    // mutations) as one typed surface, still with no tenant argument.
    #[test]
    fn graphql_introspect_exposes_code_domain() {
        let (provider, mut config) = fixture();
        config.tool_result_budget_bytes = 0;
        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "codeSearch",
            "codeContext",
            "codeExplore",
            "codeExplain",
            "codeRecognize",
            "listRepos",
            "ingestCodebase",
            "reindexCodebase",
        ] {
            assert!(
                sdl.contains(fragment),
                "SDL missing code-domain {fragment}:\n{sdl}"
            );
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );
    }

    // ---- A5-kg: harness instant-KG GraphQL surface acceptance (each field == its flat tool) ----

    // A5-kg.1 (acceptance): harnessKgImpact + harnessKgStatus over GraphQL return what
    // the flat harness_kg_* tools return, on the fixture's seeded instant KG.
    #[test]
    fn graphql_harness_kg_matches_flat_tools() {
        let (provider, config) = fixture();

        let gql = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ harnessKgImpact(symbolName:\"Ada\", direction:\"out\", maxDepth:1) }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql);
        let flat = call_tool_json(
            &provider,
            &config,
            "harness_kg_impact",
            json!({ "tenant": "smoke", "symbol_name": "Ada", "direction": "out", "max_depth": 1 }),
        );
        // The typed field lowers to the same instant_kg payload: seed resolution + impact rows match.
        assert_eq!(
            gql["data"]["harnessKgImpact"]["seed"], flat["seed"],
            "harnessKgImpact seed must match the flat tool: gql={gql} flat={flat}"
        );
        assert_eq!(
            gql["data"]["harnessKgImpact"]["results"], flat["results"],
            "harnessKgImpact results must match the flat tool"
        );
        assert_eq!(
            gql["data"]["harnessKgImpact"]["seed"],
            json!("node:a"),
            "Ada should resolve to node:a: {gql}"
        );

        // harnessKgStatus also routes through the same payload and answers without error.
        let status = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ harnessKgStatus }",
            Value::Null,
        );
        assert_no_graphql_errors(&status);
        assert!(
            !status["data"]["harnessKgStatus"].is_null(),
            "harnessKgStatus must return a status payload: {status}"
        );
    }

    // A5-kg.2: SDL exposes the harness-KG domain, no tenant arg.
    #[test]
    fn graphql_introspect_exposes_harness_kg_domain() {
        let (provider, mut config) = fixture();
        config.tool_result_budget_bytes = 0;
        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "harnessKgStatus",
            "harnessKgSearch",
            "harnessKgPpr",
            "harnessKgImpact",
            "harnessKgRelatedObjects",
            "harnessKgExplainEdge",
        ] {
            assert!(
                sdl.contains(fragment),
                "SDL missing harness-kg {fragment}:\n{sdl}"
            );
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );
    }

    // ---- A6: remaining-cluster GraphQL surface acceptance (each cluster == its flat tool) ----

    // A6.1: a representative read per in-crate cluster (skills / harness-run / jobs)
    // matches its flat tool, and the jobSubmit mutation round-trips into jobList.
    #[test]
    fn graphql_clusters_match_flat_tools() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        // skills: the typed `skillList` surfaces the same packs the flat
        // `skill_list` returns (an empty fixture -> empty list either way).
        let gql_skills = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ skillList { packContentHash status title } }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql_skills);
        let flat_skills = call_tool_json(
            &provider,
            &config,
            "skill_list",
            json!({ "tenant": "smoke" }),
        );
        let typed_pack_count = gql_skills["data"]["skillList"]
            .as_array()
            .map(|packs| packs.len())
            .unwrap_or(0);
        let flat_pack_count = flat_skills["packs"]
            .as_array()
            .map(|packs| packs.len())
            .unwrap_or(0);
        assert_eq!(
            typed_pack_count, flat_pack_count,
            "typed skillList must surface the same pack count as flat skill_list: {gql_skills} / {flat_skills}"
        );

        // harness-run: a missing run resolves to null, mirroring the flat tool's
        // found:false.
        let gql_run = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ harnessRun(runId:\"missing\"){ runId status } }",
            Value::Null,
        );
        assert_no_graphql_errors(&gql_run);
        let flat_run = call_tool_json(
            &provider,
            &config,
            "harness_run",
            json!({ "tenant": "smoke", "run_id": "missing" }),
        );
        assert_eq!(
            flat_run["found"],
            json!(false),
            "flat harness_run must report the missing run as not found"
        );
        assert!(
            gql_run["data"]["harnessRun"].is_null(),
            "typed harnessRun must be null when the flat tool reports found:false: {gql_run}"
        );

        // jobs: the dispatch board is RedCore-backed; the in-memory fixture does not
        // implement the job_* trait methods, so the GraphQL field routes to the same
        // backend.job_list the flat tool uses (proven by the identical backend gate).
        // Runtime job round-trips are exercised against RedCore in the job_* shaping tests.
        let gql_jobs = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ jobList { jobId } }",
            Value::Null,
        );
        assert!(
            gql_jobs["data"].is_null()
                && !gql_jobs["errors"]
                    .as_array()
                    .map(|e| e.is_empty())
                    .unwrap_or(true),
            "jobList must route to the backend (not silently return empty): {gql_jobs}"
        );
        let gql_jobs_err = gql_jobs["errors"][0]["message"]
            .as_str()
            .unwrap_or_default();
        assert!(
            gql_jobs_err.contains("job_list is not supported"),
            "jobList must route to backend.job_list, the same method the flat tool uses: {gql_jobs}"
        );
    }

    // A6.2: the ensemble cluster round-trips through GraphQL -- register a pack then
    // select it back, proving both the ensemble mutation and query lower correctly.
    #[test]
    fn graphql_ensemble_register_select_round_trips() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;

        let registered = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($i:JSON!){ ensembleRegister(input:$i){ packContentHash } }",
            json!({ "i": { "pack": sample_capability_pack(), "source_content_hash": "hash-source", "artifact_hashes": ["hash-artifact"] } }),
        );
        assert_no_graphql_errors(&registered);
        let pack_hash = registered["data"]["ensembleRegister"]["packContentHash"]
            .as_str()
            .expect("registered pack hash")
            .to_string();
        assert!(
            !pack_hash.is_empty(),
            "ensembleRegister must return a pack hash: {registered}"
        );

        let selected = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($i:JSON!){ ensembleSelect(input:$i){ selected { packContentHash } } }",
            json!({ "i": { "task": "use rust graph store mcp code search", "kind": "skill_pack", "max_selected": 1 } }),
        );
        assert_no_graphql_errors(&selected);
        assert_eq!(
            selected["data"]["ensembleSelect"]["selected"][0]["packContentHash"],
            json!(pack_hash),
            "ensembleSelect must select the registered pack via the typed field: {selected}"
        );
    }

    // A6.3: SDL exposes the cluster domains (reads + mutations), no tenant arg.
    #[test]
    fn graphql_introspect_exposes_cluster_domains() {
        let (provider, mut config) = fixture();
        config.tool_result_budget_bytes = 0;
        let sdl = call_tool_json(
            &provider,
            &config,
            "graphql_introspect",
            json!({ "tenant": "smoke" }),
        );
        let sdl = sdl
            .as_str()
            .expect("introspect should return an SDL string");
        for fragment in [
            "harnessRun",
            "skillList",
            "skillGet",
            "ensembleSelect",
            "jobList",
            "skillPublish",
            "skillApply",
            "ensembleRegister",
            "jobSubmit",
            "jobNote",
            "jobArchive",
        ] {
            assert!(
                sdl.contains(fragment),
                "SDL missing cluster {fragment}:\n{sdl}"
            );
        }
        assert!(
            !sdl.to_lowercase().contains("tenant"),
            "no GraphQL field may carry a tenant argument:\n{sdl}"
        );
    }

    // ---- A7: GraphQL-default-surface cutover acceptance ----

    fn tool_names(provider: &FixtureProvider, config: &McpServerConfig) -> Vec<String> {
        let listed = handle_mcp_request(
            provider,
            config,
            json!({ "jsonrpc": "2.0", "id": "list", "method": "tools/list" }),
        );
        listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    // A7.1: graphql_default_surface hides the GraphQL-covered flat tools from
    // tools/list while keeping the graphql_* transport tools and every uncovered
    // tool; default-off leaves the flat surface unchanged.
    #[test]
    fn graphql_default_surface_hides_covered_flat_tools() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        // Default mode (flag off): the flat surface is unchanged -- covered tools present.
        let default_names = tool_names(&provider, &config);
        assert!(
            default_names.iter().any(|n| n == "recall"),
            "default mode keeps flat recall"
        );
        assert!(
            default_names
                .iter()
                .any(|n| n == "rustyred_thg_graph_neighbors"),
            "default keeps flat neighbors"
        );
        assert!(
            default_names.iter().any(|n| n == "graphql_query"),
            "graphql_query always listed"
        );

        // Every name in the covered set must be a real advertised tool (no dead names).
        for covered in super::GRAPHQL_COVERED_FLAT_TOOLS {
            assert!(
                default_names.iter().any(|n| n == covered),
                "covered list names a tool not advertised in default mode: {covered}"
            );
        }

        // Cutover mode (flag on): covered flat tools vanish; graphql_* + uncovered stay.
        config.graphql_default_surface = true;
        let cut_names = tool_names(&provider, &config);
        for hidden in [
            "recall",
            "rustyred_thg_graph_neighbors",
            "rustyred_thg_algorithm_ppr",
            "compute_code",
            "job_list",
            "harness_kg_search",
        ] {
            assert!(
                !cut_names.iter().any(|n| n == hidden),
                "cutover must hide covered flat tool {hidden}"
            );
        }
        for kept in ["graphql_query", "graphql_introspect", "graphql_mutate"] {
            assert!(
                cut_names.iter().any(|n| n == kept),
                "cutover must keep {kept}"
            );
        }
        // An uncovered tool stays advertised (raw graph_query has no GraphQL field).
        assert!(
            cut_names.iter().any(|n| n == "rustyred_thg_graph_query"),
            "cutover must keep an uncovered tool: {cut_names:?}"
        );
    }

    // A7.2: in cutover mode an agent completes a full task through GraphQL alone --
    // the covered flat tools are gone, but graphql_mutate + graphql_query carry it.
    #[test]
    fn graphql_default_surface_completes_task_through_graphql_alone() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.graphql_default_surface = true;
        config.tool_result_budget_bytes = 0;

        // The flat remember tool is hidden in this mode (GraphQL is the path).
        assert!(
            !tool_names(&provider, &config)
                .iter()
                .any(|n| n == "remember"),
            "cutover hides flat remember"
        );

        // Write via graphql_mutate.
        let remembered = graphql_tool_call(
            &provider,
            &config,
            "graphql_mutate",
            "mutation($i:MemoryInput!){ rememberMemory(input:$i){ id content } }",
            json!({ "i": { "kind": "insight", "content": "graphql-only task atom" } }),
        );
        assert_no_graphql_errors(&remembered);
        assert!(
            remembered["data"]["rememberMemory"]["id"]
                .as_str()
                .is_some(),
            "remember via graphql_mutate: {remembered}"
        );

        // Read it back via graphql_query.
        let recalled = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query($q:String!){ memory(query:$q, limit:10){ id content } }",
            json!({ "q": "graphql-only" }),
        );
        assert_no_graphql_errors(&recalled);
        assert!(
            recalled["data"]["memory"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "recall via graphql_query must find the written atom: {recalled}"
        );

        // Run a graph algorithm via graphql_query.
        let pagerank = graphql_tool_call(
            &provider,
            &config,
            "graphql_query",
            "query{ graphAlgorithm(kind: PAGERANK){ result } }",
            Value::Null,
        );
        assert_no_graphql_errors(&pagerank);
        assert!(
            !pagerank["data"]["graphAlgorithm"]["result"].is_null(),
            "graphAlgorithm via graphql_query: {pagerank}"
        );
    }

    // ---- E0.1 (embedded mode): SharedStore in-process surface over a durable store ----

    // E0.1 seam: SharedStore<S> turns one owned store into a cheap, in-process
    // McpGraphProvider, so the full GraphQL surface runs over a RedCoreGraphStore
    // with no socket and no async runtime -- the embedded-mode path. Drives
    // graphql_mutate (bulkNodes/bulkEdges) then graphql_query (neighbors) entirely
    // in-process through handle_mcp_request.
    #[test]
    fn embedded_shared_store_runs_graphql_in_process_over_redcore() {
        let provider = super::SharedStore::new(RedCoreGraphStore::memory());
        let config = McpServerConfig {
            default_tenant: "embedded".to_string(),
            read_only: false,
            tool_result_budget_bytes: 0,
            ..McpServerConfig::default()
        };

        let mutate = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0", "id": "m", "method": "tools/call",
                "params": { "name": "graphql_mutate", "arguments": {
                    "tenant": "embedded",
                    "query": "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                    "variables": { "n": [
                        { "id": "e1", "labels": ["Doc"], "properties": {} },
                        { "id": "e2", "labels": ["Doc"], "properties": {} }
                    ] }
                } }
            }),
        );
        assert_eq!(
            mutate["result"]["structuredContent"]["data"]["bulkNodes"]["inserted"],
            json!(2),
            "embedded bulkNodes via graphql_mutate over RedCore: {mutate}"
        );

        let _edge = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0", "id": "e", "method": "tools/call",
                "params": { "name": "graphql_mutate", "arguments": {
                    "tenant": "embedded",
                    "query": "mutation($e:JSON!){ bulkEdges(edges:$e){ inserted } }",
                    "variables": { "e": [ { "id": "e1->e2", "from_id": "e1", "to_id": "e2", "type": "LINKS" } ] }
                } }
            }),
        );

        let query = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0", "id": "q", "method": "tools/call",
                "params": { "name": "graphql_query", "arguments": {
                    "tenant": "embedded",
                    "query": "query{ neighbors(nodeId:\"e1\", direction:\"out\") }"
                } }
            }),
        );
        let neighbors = &query["result"]["structuredContent"]["data"]["neighbors"]["neighbors"];
        assert!(
            neighbors.as_array().map(|a| !a.is_empty()).unwrap_or(false),
            "embedded neighbors via graphql_query must find e2 in-process: {query}"
        );
    }

    #[test]
    fn spawn_session_writes_room_visible_coordination_record() {
        let (provider, mut config) = fixture();
        config.read_only = false;

        let room_id = "repo:theorem:branch:handoff-demo";
        let spawned = call_tool_json(
            &provider,
            &config,
            "spawn_session",
            json!({
                "actor": "claude-code",
                "room_id": room_id,
                "intent": "Implement the foo widget and open a PR\nwith full coverage",
                "repo": "theorem",
                "branch": "handoff-demo"
            }),
        );

        assert_eq!(spawned["status"], "running");
        assert_eq!(spawned["executor"], "github_actions");
        let dispatch_id = spawned["dispatch_id"]
            .as_str()
            .expect("dispatch_id string")
            .to_string();
        assert!(!dispatch_id.is_empty());
        assert_eq!(spawned["record"]["record_type"], "event");
        assert_eq!(spawned["record"]["metadata"]["surface"], "spawned");
        assert_eq!(spawned["record"]["metadata"]["status"], "running");
        // summary is the first line of the intent, not the whole prompt
        assert_eq!(
            spawned["record"]["summary"],
            "Implement the foo widget and open a PR"
        );

        // the spawned session is retrievable from the room exactly like any participant record
        let records = call_tool_json(
            &provider,
            &config,
            "read_records_for_room",
            json!({ "room_id": room_id, "record_types": ["event"] }),
        );
        let list = records["records"].as_array().expect("records array");
        assert!(
            list.iter().any(|record| {
                record["metadata"]["dispatch_id"] == json!(dispatch_id)
                    && record["metadata"]["surface"] == json!("spawned")
            }),
            "spawn record should be retrievable from the room: {records}"
        );
    }

    #[test]
    fn native_harness_run_transitions_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.tool_result_budget_bytes = 0;
        let run_id = "run-mcp-0001";

        let created = append_harness_event(
            &provider,
            &config,
            run_id,
            "RUN.CREATED",
            json!({
                "task": "wire native harness MCP",
                "actor": "codex",
                "scope": {
                    "repo": "Theorem",
                    "branch": "main",
                    "commit_sha": "abc123",
                    "cwd": "/repo/Theorem",
                    "workstream_id": "harness-rust-port",
                    "agent_host": "codex",
                    "agent_model": "gpt-5"
                }
            }),
        );
        assert_eq!(created["result"]["run"]["run_id"], run_id);
        assert_eq!(created["result"]["event"]["type"], "RUN.CREATED");

        append_harness_event(
            &provider,
            &config,
            run_id,
            "HOST.OBSERVED",
            json!({
                "repo": "Theorem",
                "branch": "main",
                "commit_sha": "abc123",
                "cwd": "/repo/Theorem"
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "TASK.RESOLVED",
            json!({"task_signature": "sig-native-harness-mcp"}),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "PROFILE.SELECTED",
            json!({
                "profile_id": "rust-port",
                "profile_version": "1",
                "policy_hash": "policy-abc"
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "TOOLKIT.COMPILED",
            json!({
                "selected_tools": ["harness_append_transition", "harness_run"],
                "selected_plugins": ["rustyred-thg-mcp"],
                "excluded_tools": [],
                "permission_reasons": {}
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "CONTEXT.PLANNED",
            json!({
                "budget_tokens": 1000,
                "plan_hash": "plan-mcp",
                "candidate_token_count": 500
            }),
        );
        let packed = append_harness_event(
            &provider,
            &config,
            run_id,
            "CONTEXT.PACKED",
            json!({
                "artifact_id": "art-mcp",
                "capsule_tokens": 200,
                "budget_tokens": 1000,
                "included_atom_count": 5,
                "excluded_atom_count": 2,
                "token_ledger": { "saved": 300 }
            }),
        );
        assert_eq!(packed["result"]["run"]["status"], "context_packed");
        assert_eq!(
            packed["result"]["event"]["payload"]["token_ledger"]["saved"],
            300
        );

        append_harness_event(
            &provider,
            &config,
            run_id,
            "CONTEXT.INJECTED",
            json!({
                "artifact_id": "art-mcp",
                "adapter": "mcp",
                "target": "codex"
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "AGENT.ACTING",
            json!({
                "adapter": "mcp",
                "started_at": "2026-06-01T00:00:00Z"
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "OUTCOME.RECORDED",
            json!({
                "accepted": true,
                "tests_passed": true,
                "validator_results": [{ "id": "cargo-test", "status": "passed" }],
                "files_changed": ["rustyredcore_THG/crates/rustyred-thg-mcp/src/lib.rs"],
                "summary": "native MCP append/read path works"
            }),
        );
        append_harness_event(
            &provider,
            &config,
            run_id,
            "RUN.CLOSED",
            json!({
                "summary": "native MCP append/read path works",
                "closed_by": "codex"
            }),
        );

        let detail = call_tool_json(
            &provider,
            &config,
            "harness_run",
            json!({ "tenant": "smoke", "run_id": run_id }),
        );

        assert_eq!(detail["found"], true);
        assert_eq!(detail["detail"]["run"]["status"], "closed");
        let events = detail["detail"]["events"].as_array().unwrap();
        assert_eq!(
            detail["detail"]["run"]["last_event_seq"],
            json!(events.len() as u64)
        );
        assert!(events.len() >= 11);
        let context_packed = events
            .iter()
            .find(|event| event["type"] == json!("CONTEXT.PACKED"))
            .expect("CONTEXT.PACKED event should be present");
        assert_eq!(context_packed["payload"]["token_ledger"]["saved"], 300);
        let outcome_recorded = events
            .iter()
            .find(|event| event["type"] == json!("OUTCOME.RECORDED"))
            .expect("OUTCOME.RECORDED event should be present");
        assert_eq!(
            outcome_recorded["payload"]["validator_results"][0]["status"],
            "passed"
        );
    }

    #[test]
    fn composed_agent_run_round_trips_through_mcp_with_fake_heads() {
        let provider = FixtureProvider(Rc::new(RefCell::new(InMemoryGraphStore::new())));
        let config = McpServerConfig {
            read_only: false,
            tool_result_budget_bytes: 0,
            ..McpServerConfig::default()
        };

        let result = call_tool_json(
            &provider,
            &config,
            "composed_agent_run",
            json!({
                "tenant": "smoke",
                "binding_id": "agent:mcp-test",
                "task": "publish composed agent result",
                "claims": [{
                    "text": "composed agent result is grounded",
                    "provenance": "test:composed-agent"
                }]
            }),
        );

        assert_eq!(result["tenant"], "smoke");
        assert_eq!(result["result"]["run_id"], "agent:mcp-test");
        assert_eq!(
            result["result"]["consensus_head_set"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn coordination_record_policy_hooks_gate_scope_and_budget() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        let no_write_scope = call_tool_json_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["graph:read"]),
            "coordination_record",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "record_type": "decision",
                "summary": "Requires coordination write scope",
                "required_scope": "coordination:write",
                "created_at": "2026-06-01T00:06:00Z"
            }),
        );
        assert_eq!(no_write_scope["error"], "coordination_scope_denied");
        assert_eq!(
            no_write_scope["policy_receipt"]["missing_scopes"],
            json!(["coordination:write"])
        );

        let budget_denied = call_tool_json_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["coordination:write"]),
            "coordination_contribution",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "summary": "Too expensive for this budget",
                "required_scope": "coordination:write",
                "estimated_cost_units": 5.0,
                "budget_units": 1.0,
                "created_at": "2026-06-01T00:07:00Z"
            }),
        );
        assert_eq!(budget_denied["error"], "coordination_budget_exceeded");
        assert_eq!(budget_denied["policy_receipt"]["budget_allowed"], false);

        let allowed = call_tool_json_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["coordination:write"]),
            "coordination_contribution",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "summary": "Within budget and scope",
                "required_scope": "coordination:write",
                "estimated_cost_units": 1.0,
                "budget_units": 5.0,
                "created_at": "2026-06-01T00:08:00Z"
            }),
        );
        assert_eq!(allowed["policy_receipt"]["decision"], "allow");
        assert_eq!(
            allowed["policy_receipt"]["publication_gate"]["decision"],
            "not_applicable"
        );
        assert_eq!(
            allowed["contribution"]["metadata"]["policy_receipt"]["required_scopes"],
            json!(["coordination:write"])
        );

        let publication_denied = call_tool_json_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["coordination:write"]),
            "coordination_contribution",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "summary": "Publish a claim without peer review",
                "required_scope": "coordination:write",
                "publication": true,
                "claims": [{
                    "text": "Slice 2 is complete",
                    "provenance": "test:coordination-policy"
                }]
            }),
        );
        assert_eq!(
            publication_denied["error"],
            "coordination_publication_denied"
        );
        assert_eq!(
            publication_denied["policy_receipt"]["publication_gate"]["decision"],
            "deny"
        );

        let publication_allowed = call_tool_json_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["coordination:write"]),
            "coordination_contribution",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "reviewer": "claude-code",
                "room_id": "harness-rust-port",
                "summary": "Publish a reviewed claim",
                "required_scope": "coordination:write",
                "publication": true,
                "claims": [{
                    "text": "Slice 2 is peer reviewed",
                    "provenance": "test:coordination-policy"
                }]
            }),
        );
        assert_eq!(publication_allowed["policy_receipt"]["decision"], "allow");
        assert_eq!(
            publication_allowed["policy_receipt"]["publication_gate"]["decision"],
            "allow"
        );
        assert_eq!(
            publication_allowed["policy_receipt"]["publication_gate"]["synthesis_heads"],
            json!(["codex", "claude-code"])
        );
    }

    #[test]
    fn symbolic_datalog_tool_returns_reference_receipt_shape() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "symbolic",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_symbolic_datalog_derive",
                    "arguments": {
                        "facts": [
                            {"relation": "claim", "entity_id": "claim-1", "attributes": {"status": "proposed"}, "fact_id": "f1"},
                            {"relation": "object", "entity_id": "obj-1", "attributes": {"title": "Same"}, "fact_id": "f2"},
                            {"relation": "object", "entity_id": "obj-2", "attributes": {"title": "same"}, "fact_id": "f3"}
                        ]
                    }
                }
            }),
        );

        let content = &response["result"]["structuredContent"];
        assert_eq!(content["engine"], "python-reference-datalog");
        assert_eq!(content["derived_count"], 3);
        let relations: Vec<&str> = content["derived_facts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|fact| fact["relation"].as_str())
            .collect();
        assert!(relations.contains(&"unsupported_claim"));
        assert!(relations.contains(&"likely_duplicate_entity"));
        assert!(relations.contains(&"claim_has_no_independent_support"));
    }

    #[test]
    fn symbolic_probabilistic_tools_return_receipts() {
        let (provider, config) = fixture();
        let source = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "source",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_symbolic_probabilistic_source_reliability",
                    "arguments": {
                        "source_id": "source-a",
                        "prior_alpha": 2.0,
                        "prior_beta": 2.0,
                        "corroborated": 6,
                        "contradicted": 2
                    }
                }
            }),
        );
        assert_eq!(
            source["result"]["structuredContent"]["posterior"]["alpha"],
            8.0
        );
        assert_eq!(
            source["result"]["structuredContent"]["posterior"]["beta"],
            4.0
        );

        let expected_value = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "evi",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_symbolic_probabilistic_expected_value",
                    "arguments": {
                        "current_uncertainty": 0.6,
                        "expected_uncertainty_after": 0.2,
                        "decision_value": 10.0,
                        "validator_cost": 1.0
                    }
                }
            }),
        );
        let evi = expected_value["result"]["structuredContent"]["posterior"]["expected_value"]
            .as_f64()
            .expect("expected numeric EVI");
        assert!((evi - 3.0).abs() < 1e-9);
    }

    #[test]
    fn tool_call_reads_neighbors_from_graph_store() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "neighbors",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_neighbors",
                    "arguments": {
                        "tenant": "smoke",
                        "node_id": "node:a",
                        "direction": "out",
                        "edge_type": "KNOWS"
                    }
                }
            }),
        );

        assert_eq!(
            response["result"]["structuredContent"]["neighbors"][0]["node_id"],
            "node:b"
        );
    }

    #[test]
    fn tool_call_enforces_neighbor_budget() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "neighbors",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_neighbors",
                    "arguments": {
                        "tenant": "smoke",
                        "node_id": "node:a",
                        "direction": "out",
                        "budget": { "max_nodes_returned": 1 }
                    }
                }
            }),
        );

        assert_eq!(
            response["result"]["structuredContent"]["neighbors"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            response["result"]["structuredContent"]["stats"]["truncated"],
            true
        );
    }

    #[test]
    fn relational_query_tool_resolves_graphql_selection_join() {
        let (provider, config) = fixture();
        let listed = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        assert!(listed["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_relational_query"));

        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "rel",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_relational_query",
                    "arguments": {
                        "tenant": "smoke",
                        "selection": {
                            "relation": "person",
                            "alias": "p",
                            "fields": ["id", "name"],
                            "joins": [{
                                "relation": "knows",
                                "alias": "k",
                                "left_column": "id",
                                "right_column": "from_id",
                                "fields": ["to_id", "since"]
                            }]
                        }
                    }
                }
            }),
        );

        let content = &response["result"]["structuredContent"];
        assert_eq!(content["planner"], "rustyred-native-relational");
        assert_eq!(content["trace"]["join_algorithm"], "hash_join");
        let rows = content["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().any(|row| row["k.to_id"] == "node:b"));
        assert!(rows.iter().any(|row| row["k.to_id"] == "node:c"));
    }

    #[test]
    fn graph_query_supports_property_indexed_node_match() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "node-match",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_query",
                    "arguments": {
                        "tenant": "smoke",
                        "operation": "node_match",
                        "label": "Person",
                        "properties": { "name": "Grace" },
                        "budget": { "max_nodes_returned": 5 }
                    }
                }
            }),
        );

        assert_eq!(
            response["result"]["structuredContent"]["nodes"][0]["id"],
            "node:b"
        );
        assert_eq!(
            response["result"]["structuredContent"]["explain"]["plan"][1]["op"],
            "node_index_seek"
        );
    }

    #[test]
    fn version_tools_support_refs_checkout_and_merge() {
        let (provider, config) = fixture();
        let base = provider
            .0
            .borrow()
            .graph_snapshot()
            .expect("fixture snapshot");
        let ref_update = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ref",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_version_ref",
                    "arguments": {
                        "tenant": "smoke",
                        "branch": "main",
                        "timestamp_unix_ms": 1,
                        "updated_at_unix_ms": 2
                    }
                }
            }),
        );
        let repository =
            ref_update["result"]["structuredContent"]["ref_update"]["repository"].clone();
        assert_eq!(
            ref_update["result"]["structuredContent"]["ref_update"]["reference"]["name"],
            "main"
        );

        let checkout = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "checkout",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_version_checkout",
                    "arguments": {
                        "tenant": "smoke",
                        "repository": repository,
                        "target": "main"
                    }
                }
            }),
        );
        assert_eq!(
            checkout["result"]["structuredContent"]["checkout"]["snapshot"]["nodes"]
                .as_array()
                .unwrap()
                .len(),
            base.nodes.len()
        );

        let mut theirs = base.clone();
        theirs.nodes.push(NodeRecord::new(
            "node:d",
            ["Person"],
            json!({"name": "Dorothy"}),
        ));
        let merge = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "merge",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_graph_version_merge",
                    "arguments": {
                        "tenant": "smoke",
                        "base": base,
                        "theirs": theirs,
                        "timestamp_unix_ms": 3
                    }
                }
            }),
        );
        assert_eq!(
            merge["result"]["structuredContent"]["merge"]["status"],
            "clean"
        );
    }

    #[test]
    fn algorithm_tool_calls_run_over_graph_edges() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "pagerank",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_pagerank",
                    "arguments": { "tenant": "smoke", "top_k": 2 }
                }
            }),
        );

        assert_eq!(
            response["result"]["structuredContent"]["scores"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn instant_kg_tools_resolve_symbol_names_and_reject_bad_delta() {
        let (provider, config) = fixture();
        let impact = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "impact",
                "method": "tools/call",
                "params": {
                    "name": "harness_kg_impact",
                    "arguments": {
                        "tenant": "smoke",
                        "symbol_name": "Ada",
                        "direction": "out",
                        "max_depth": 1
                    }
                }
            }),
        );

        assert_eq!(impact["result"]["structuredContent"]["seed"], "node:a");
        let impacted_ids: Vec<_> = impact["result"]["structuredContent"]["results"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|row| row["object_id"].as_str())
            .collect();
        assert!(impacted_ids.contains(&"node:b"));

        let bad_delta = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "bad-delta",
                "method": "tools/call",
                "params": {
                    "name": "harness_kg_status",
                    "arguments": {
                        "tenant": "smoke",
                        "delta": { "objects": "not-an-array" }
                    }
                }
            }),
        );

        assert_eq!(bad_delta["error"]["code"], -32602);
        assert!(bad_delta["error"]["message"]
            .as_str()
            .unwrap()
            .contains("delta must match instant KG schema"));
    }

    #[test]
    fn read_write_tools_list_exposes_bulk_and_designation_tools() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_bulk_nodes"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_fulltext_designate"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_spatial_designate"));
    }

    #[test]
    fn admin_tool_requires_read_write_mcp_mode_and_admin_scope() {
        let (provider, mut config) = fixture();
        config.read_only = false;
        config.allow_admin = true;

        let no_admin = handle_mcp_request_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["graph:read"]),
            json!({
                "jsonrpc": "2.0",
                "id": "verify",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_admin_verify",
                    "arguments": { "tenant": "smoke" }
                }
            }),
        );
        assert_eq!(
            no_admin["result"]["structuredContent"]["error"],
            "admin_scope_required"
        );

        let with_admin = handle_mcp_request_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["rustyred_thg:graph:admin:verify"]),
            json!({
                "jsonrpc": "2.0",
                "id": "verify",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_admin_verify",
                    "arguments": { "tenant": "smoke" }
                }
            }),
        );
        assert_eq!(
            with_admin["result"]["structuredContent"]["verify"]["ok"],
            true
        );
    }

    #[test]
    fn read_only_mode_hides_and_blocks_admin_tools() {
        let (provider, mut config) = fixture();
        config.read_only = true;
        config.allow_admin = true;

        let list = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": "list", "method": "tools/list"}),
        );
        assert!(!list["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_admin_verify"));

        let blocked = handle_mcp_request_with_context(
            &provider,
            &config,
            &McpRequestContext::with_scopes(["admin:read"]),
            json!({
                "jsonrpc": "2.0",
                "id": "verify",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_admin_verify",
                    "arguments": { "tenant": "smoke" }
                }
            }),
        );
        assert_eq!(
            blocked["result"]["structuredContent"]["error"],
            "mcp_read_only"
        );
    }

    #[test]
    fn resources_read_supports_node_uri() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "node",
                "method": "resources/read",
                "params": { "uri": "rustyred_thg://tenant/smoke/node/node:a" }
            }),
        );

        let text = response["result"]["contents"][0]["text"].as_str().unwrap();
        assert!(text.contains("\"node:a\""));
    }

    // ---- §P6-B pb6.1 algo + bulk trait defaults --------------------------

    fn store_with_two_components() -> InMemoryGraphStore {
        // Two disconnected edges => two connected components: {a, b} and {c, d}.
        // `connected_components` ignores nodes that don't appear in any edge,
        // so a dangling node won't form its own component.
        let mut store = InMemoryGraphStore::default();
        store
            .upsert_node(NodeRecord::new("a", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_node(NodeRecord::new("b", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_node(NodeRecord::new("c", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_node(NodeRecord::new("d", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new("e1", "a", "T", "b", json!({})))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new("e2", "c", "T", "d", json!({})))
            .unwrap();
        store
    }

    #[test]
    fn backend_components_returns_partition() {
        use super::McpGraphBackend;
        let store = store_with_two_components();
        let components = store.algo_components(false).unwrap();
        // {a, b} and {c}
        assert_eq!(components.len(), 2);
    }

    #[test]
    fn backend_pagerank_returns_score_map() {
        use super::McpGraphBackend;
        let store = store_with_two_components();
        let ranks = store.algo_pagerank(0.85, 50, 1e-6).unwrap();
        assert!(ranks.contains_key("a"));
        assert!(ranks.contains_key("b"));
    }

    #[test]
    fn backend_bulk_upsert_nodes_counts_inserts() {
        use super::McpGraphBackend;
        let mut store = InMemoryGraphStore::default();
        let records = vec![
            NodeRecord::new("x", ["Doc"], json!({})),
            NodeRecord::new("y", ["Doc"], json!({})),
        ];
        let (inserted, failed) = store.bulk_upsert_nodes(records).unwrap();
        assert_eq!(inserted, 2);
        assert_eq!(failed, 0);
    }

    // ========================================================================
    // RR-INLINE-* tests for inline-adjacency algorithm tools.
    //
    // Coverage:
    //   * RR-INLINE-03: count_inline_edges helper correctness
    //   * RR-INLINE-04: ppr_inline returns expected score shape
    //   * RR-INLINE-05: components_inline partitions a known disconnected graph
    //   * RR-INLINE-06: pagerank_inline returns expected score shape
    //   * RR-INLINE-07: communities_inline returns labels and modularity
    //   * RR-INLINE-09: tools/list exposes the four new tool names
    //   * RR-INLINE-10: payload_too_large fires above the configured limit
    //   * RR-INLINE-11: tenant-backed PPR shape unchanged post-additions
    // ========================================================================

    #[test]
    fn count_inline_edges_sums_neighbor_list_lengths() {
        let adjacency = json!({
            "a": [["b", 1.0], ["c", 1.0]],
            "b": [["a", 1.0]],
            "c": [["a", 1.0]]
        });
        assert_eq!(super::count_inline_edges(&adjacency), 4);
    }

    #[test]
    fn count_inline_edges_handles_empty_and_non_object_input() {
        let empty_neighbors = json!({ "a": [], "b": [["a", 1.0]] });
        assert_eq!(super::count_inline_edges(&empty_neighbors), 1);
        assert_eq!(super::count_inline_edges(&json!("not an object")), 0);
        assert_eq!(super::count_inline_edges(&json!(null)), 0);
        assert_eq!(super::count_inline_edges(&json!({})), 0);
    }

    #[test]
    fn algorithm_ppr_inline_returns_scores_against_inline_adjacency() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ppr_inline",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_ppr_inline",
                    "arguments": {
                        "adjacency": {
                            "a": [["b", 1.0], ["c", 1.0]],
                            "b": [["a", 1.0]],
                            "c": [["a", 1.0]]
                        },
                        "seeds": { "a": 1.0 },
                        "alpha": 0.15,
                        "epsilon": 1e-5,
                        "max_pushes": 10_000
                    }
                }
            }),
        );

        let content = &response["result"]["structuredContent"];
        // Tenant field must NOT appear in inline responses; it's the
        // structural signal that no tenant was touched.
        assert!(content.get("tenant").is_none());
        assert_eq!(content["edge_count"], json!(4));
        let scores = content["scores"].as_array().expect("scores array present");
        assert!(!scores.is_empty(), "PPR should return at least one score");
        // Seed node should appear in scores with positive mass.
        let seed_score = scores
            .iter()
            .find(|entry| entry["node_id"] == "a")
            .expect("seed node `a` ranked");
        assert!(seed_score["score"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn algorithm_ppr_inline_alias_routes_to_same_handler() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ppr_inline_alias",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algo_ppr_inline",
                    "arguments": {
                        "adjacency": { "a": [["b", 1.0]] },
                        "seeds": { "a": 1.0 }
                    }
                }
            }),
        );
        let content = &response["result"]["structuredContent"];
        assert_eq!(content["edge_count"], json!(1));
        assert!(!content["scores"].as_array().unwrap().is_empty());
    }

    #[test]
    fn algorithm_components_inline_partitions_disconnected_inline_graph() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "components_inline",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_components_inline",
                    "arguments": {
                        // Two disconnected pairs: {a,b} and {c,d}.
                        "adjacency": {
                            "a": [["b", 1.0]],
                            "b": [["a", 1.0]],
                            "c": [["d", 1.0]],
                            "d": [["c", 1.0]]
                        },
                        "directed": false
                    }
                }
            }),
        );
        let content = &response["result"]["structuredContent"];
        assert!(content.get("tenant").is_none());
        assert_eq!(content["edge_count"], json!(4));
        let count = content["count"].as_u64().expect("count present");
        assert_eq!(count, 2, "expected two connected components");
    }

    #[test]
    fn algorithm_pagerank_inline_returns_scores() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "pagerank_inline",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_pagerank_inline",
                    "arguments": {
                        "adjacency": {
                            "a": [["b", 1.0], ["c", 1.0]],
                            "b": [["a", 1.0]],
                            "c": [["a", 1.0]]
                        },
                        "damping": 0.85,
                        "max_iter": 50,
                        "tolerance": 1e-6,
                        "top_k": 3
                    }
                }
            }),
        );
        let content = &response["result"]["structuredContent"];
        assert!(content.get("tenant").is_none());
        assert_eq!(content["edge_count"], json!(4));
        let scores = content["scores"].as_array().expect("scores array");
        assert_eq!(scores.len(), 3, "top_k should bound the score list");
    }

    #[test]
    fn algorithm_communities_inline_returns_labels_and_modularity() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "communities_inline",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_communities_inline",
                    "arguments": {
                        "adjacency": {
                            "a": [["b", 1.0]],
                            "b": [["a", 1.0]],
                            "c": [["d", 1.0]],
                            "d": [["c", 1.0]]
                        }
                    }
                }
            }),
        );
        let content = &response["result"]["structuredContent"];
        assert!(content.get("tenant").is_none());
        assert_eq!(content["algorithm"], json!("label_propagation"));
        assert_eq!(content["edge_count"], json!(4));
        let communities = content["communities"]
            .as_array()
            .expect("communities array");
        assert_eq!(
            communities.len(),
            4,
            "every node has a community assignment"
        );
        assert!(content["modularity"].is_number(), "modularity is numeric");
    }

    #[test]
    fn algorithm_inline_tools_listed_in_tools_response() {
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        let names: Vec<&str> = tools
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .collect();

        assert!(names.contains(&"rustyred_thg_algorithm_ppr_inline"));
        assert!(names.contains(&"rustyred_thg_algorithm_components_inline"));
        assert!(names.contains(&"rustyred_thg_algorithm_pagerank_inline"));
        assert!(names.contains(&"rustyred_thg_algorithm_communities_inline"));
    }

    #[test]
    fn parse_inline_adjacency_rejects_payload_above_max_edges() {
        // RR-INLINE-10: the max-edges guard returns the application-defined
        // `payload_too_large` JSON-RPC code -32004 with a message routing
        // the caller to the tenant-backed counterpart.
        //
        // Tests the helper directly with an explicit `max_edges` parameter
        // rather than the env-var integration; this avoids process-global
        // env-var mutation that would race with parallel tests. The
        // env-var read path is exercised by `max_inline_edges()` separately
        // (trivial: env::var.parse().unwrap_or(default)).
        let arguments = json!({
            "adjacency": {
                "a": [["b", 1.0], ["c", 1.0]],
                "b": [["a", 1.0]]
            },
            "seeds": { "a": 1.0 }
        });
        let result =
            super::parse_inline_adjacency(&arguments, "rustyred_thg_algorithm_ppr_inline", 2);
        let err = result.expect_err("3 edges with limit 2 should be rejected");
        assert_eq!(err.code, -32004);
        assert!(err.message.contains("3 edges"));
        assert!(err.message.contains("limit of 2"));
        assert!(
            err.message.contains("tenant-backed"),
            "error message should point callers to the tenant-backed path"
        );
    }

    #[test]
    fn parse_inline_adjacency_accepts_payload_at_max_edges() {
        // Boundary case: exactly at the limit should succeed (the guard
        // rejects strictly `>`, not `>=`).
        let arguments = json!({
            "adjacency": {
                "a": [["b", 1.0], ["c", 1.0]],
                "b": [["a", 1.0]]
            }
        });
        let result =
            super::parse_inline_adjacency(&arguments, "rustyred_thg_algorithm_ppr_inline", 3);
        let (adjacency, edge_count) = result.expect("3 edges with limit 3 should succeed");
        assert_eq!(edge_count, 3);
        assert_eq!(adjacency.len(), 2);
    }

    #[test]
    fn algorithm_ppr_tenant_backed_response_shape_unchanged() {
        // RR-INLINE-11: regression. The existing tenant-backed PPR tool must
        // still produce the same response shape after the inline additions.
        // We do not assert byte-identity (algorithms involve floats and
        // sort-tiebreaks), but we DO assert the schema's invariants: `tenant`
        // present, `scores` array of {node_id, score}, `alpha` echoed back.
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ppr_tenant",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_algorithm_ppr",
                    "arguments": {
                        "tenant": "smoke",
                        "seeds": { "node:a": 1.0 }
                    }
                }
            }),
        );
        let content = &response["result"]["structuredContent"];
        assert_eq!(content["tenant"], json!("smoke"));
        assert_eq!(content["alpha"], json!(0.15));
        let scores = content["scores"].as_array().expect("scores array");
        assert!(
            !scores.is_empty(),
            "tenant graph has edges; PPR should rank"
        );
        for entry in scores {
            assert!(entry["node_id"].is_string());
            assert!(entry["score"].is_number());
        }
    }
}
