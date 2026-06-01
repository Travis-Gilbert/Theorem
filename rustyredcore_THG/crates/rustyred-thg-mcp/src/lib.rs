use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{
    checkout_graph_version, compile_graph_pack, diff_graph_snapshots, graph_version_log,
    merge_graph_snapshots, update_graph_ref, CodeKgManifest, Direction, EdgeRecord, EpistemicType,
    GraphCompileOptions, GraphMergeOptions, GraphSnapshot, GraphStats, GraphStore, GraphStoreError,
    GraphStoreResult, GraphVersionRepository, HarnessInstantKg, HybridScoringConfig,
    InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord, RedCoreGraphStore,
    SessionDelta, VectorDesignation, VerifyReport,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use theorem_harness_core::{TransitionInput, TransitionResult};
use theorem_harness_runtime::{
    append_transition_from_store, coordination_intent_edge_id, coordination_intent_node_id,
    coordination_member_edge_id, coordination_member_node_id, coordination_mention_edge_id,
    coordination_message_edge_id, coordination_message_node_id, coordination_presence_node_id,
    coordination_record_edge_id, coordination_record_node_id, coordination_room_node_id,
    infer_coordination_room_id, load_events, load_run, normalize_coordination_urgency,
    parse_coordination_mentions, stable_coordination_message_id, stable_coordination_record_id,
    CoordinationIntentState, CoordinationMessageState, CoordinationPresenceState,
    CoordinationRecordState, CoordinationRoomMember, CoordinationRoomState, HarnessRuntimeError,
    JoinRoomInput, PresenceInput, WriteIntentInput, WriteMessageInput, WriteRecordInput,
};

const JSONRPC_VERSION: &str = "2.0";
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct McpServerConfig {
    pub name: String,
    pub version: String,
    pub default_tenant: String,
    pub read_only: bool,
    pub allow_admin: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            name: "rusty-red-graph-database".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_tenant: "default".to_string(),
            read_only: true,
            allow_admin: false,
        }
    }
}

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
    let tenant = tenant_from_args(&arguments, config);
    let mut backend = provider.backend_for_tenant(&tenant)?;

    let payload = match name {
        "rustyred_thg_graph_neighbors" => {
            let query = neighbor_query_from_value(&arguments)?;
            let mut neighbors = backend.neighbors(query)?;
            let budget = Budget::from_args(&arguments);
            let truncated = apply_neighbor_budget(&mut neighbors, budget);
            json!({
                "tenant": tenant,
                "neighbors": neighbors,
                "stats": { "returned": neighbors.len(), "truncated": truncated }
            })
        }
        "rustyred_thg_graph_schema" => schema_payload(&tenant, &backend)?,
        "rustyred_thg_graph_index_status" => index_status_payload(&tenant, &backend)?,
        "rustyred_thg_graph_explain" => explain_payload(&tenant, &arguments),
        "rustyred_thg_graph_query" => query_payload(&tenant, &backend, &arguments)?,
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
            let pack = compile_graph_pack(&snapshot, options);
            json!({
                "tenant": tenant,
                "ref_update": update_graph_ref(repository, pack, branch, updated_at_unix_ms.map(u128::from))
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
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
            json!({
                "tenant": tenant,
                "status": view.status(),
                "stats": view.stats()
            })
        }
        "rustyred_thg_instant_kg_ppr" | "harness_kg_ppr" => {
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
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
        "rustyred_thg_instant_kg_impact" | "harness_kg_impact" => {
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
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
        "rustyred_thg_instant_kg_related_objects" | "harness_kg_related_objects" => {
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
            let seed = required_str(&arguments, "seed", name)?;
            let kinds = string_array(&arguments, "kinds");
            let top_k = arguments.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
            json!({
                "tenant": tenant,
                "seed": seed,
                "status": view.status(),
                "results": view.related_objects(seed, &kinds, top_k)
            })
        }
        "rustyred_thg_instant_kg_search" | "harness_kg_search" => {
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
            let query = required_str(&arguments, "query", name)?;
            let kinds = string_array(&arguments, "kinds");
            let top_k = arguments.get("top_k").and_then(Value::as_u64).unwrap_or(10) as usize;
            json!({
                "tenant": tenant,
                "query": query,
                "status": view.status(),
                "results": view.search(query, &kinds, top_k)
            })
        }
        "rustyred_thg_instant_kg_explain_edge" | "harness_kg_explain_edge" => {
            let view = instant_kg_view_payload(&tenant, &backend, &arguments)?;
            let src = required_str(&arguments, "src", name)?;
            let dst = required_str(&arguments, "dst", name)?;
            json!({
                "tenant": tenant,
                "src": src,
                "dst": dst,
                "status": view.status(),
                "explanations": view.explain_edge(src, dst)
            })
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
            presence_payload(&tenant, &mut backend, &arguments)?
        }
        "coordination_intent" | "write_intent" | "theorem_harness_write_intent" => {
            if config.read_only {
                return Ok(tool_result_error(json!({
                    "error": "mcp_read_only",
                    "message": "Native coordination intent writes are unavailable while read-only mode is active."
                })));
            }
            write_intent_payload(&tenant, &mut backend, &arguments)?
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
            write_record_payload(&tenant, &mut backend, &arguments, Some(policy_receipt))?
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
            append_harness_transition_payload(&tenant, &mut backend, &arguments)?
        }
        "harness_run" | "theorem_harness_run" => {
            harness_run_payload(&tenant, &backend, &arguments)?
        }
        "rustyred_thg_fulltext_search" | "rustyred_thg_graph_fulltext_search" => {
            let property = required_str(&arguments, "property", name)?;
            let query = required_str(&arguments, "query", name)?;
            let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
            let label = arguments.get("label").and_then(Value::as_str);
            let results = backend.fulltext_search(label, property, query, k)?;
            json!({
                "tenant": tenant,
                "results": results.iter().map(|(node_id, score)| json!({"node_id": node_id, "score": score})).collect::<Vec<_>>(),
                "stats": { "returned": results.len(), "k": k }
            })
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
            let label = required_str(&arguments, "label", name)?;
            let property = required_str(&arguments, "property", name)?;
            backend.designate_fulltext_property(label, property)?;
            json!({
                "tenant": tenant,
                "designated": { "label": label, "property": property }
            })
        }
        "rustyred_thg_spatial_radius" | "rustyred_thg_graph_spatial_radius" => {
            let label = required_str(&arguments, "label", name)?;
            let lat_property = required_str(&arguments, "lat_property", name)?;
            let lon_property = required_str(&arguments, "lon_property", name)?;
            let lat = required_f64(&arguments, "lat", name)?;
            let lon = required_f64(&arguments, "lon", name)?;
            let radius_km = required_f64(&arguments, "radius_km", name)?;
            let node_ids = backend.spatial_radius_search(
                label,
                lat_property,
                lon_property,
                lat,
                lon,
                radius_km,
            )?;
            json!({
                "tenant": tenant,
                "node_ids": node_ids,
                "stats": { "returned": node_ids.len() }
            })
        }
        "rustyred_thg_spatial_bbox" | "rustyred_thg_graph_spatial_bbox" => {
            let label = required_str(&arguments, "label", name)?;
            let lat_property = required_str(&arguments, "lat_property", name)?;
            let lon_property = required_str(&arguments, "lon_property", name)?;
            let min_lat = required_f64(&arguments, "min_lat", name)?;
            let min_lon = required_f64(&arguments, "min_lon", name)?;
            let max_lat = required_f64(&arguments, "max_lat", name)?;
            let max_lon = required_f64(&arguments, "max_lon", name)?;
            let node_ids = backend.spatial_bbox_search(
                label,
                lat_property,
                lon_property,
                min_lat,
                min_lon,
                max_lat,
                max_lon,
            )?;
            json!({
                "tenant": tenant,
                "node_ids": node_ids,
                "stats": { "returned": node_ids.len() }
            })
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
            let label = required_str(&arguments, "label", name)?;
            let lat_property = required_str(&arguments, "lat_property", name)?;
            let lon_property = required_str(&arguments, "lon_property", name)?;
            let resolution = arguments
                .get("resolution")
                .and_then(Value::as_u64)
                .unwrap_or(9)
                .min(u8::MAX as u64) as u8;
            backend.designate_spatial_property(label, lat_property, lon_property, resolution)?;
            json!({
                "tenant": tenant,
                "designated": {
                    "label": label,
                    "lat_property": lat_property,
                    "lon_property": lon_property,
                    "resolution": resolution
                }
            })
        }
        "rustyred_thg_bulk_nodes" | "rustyred_thg_graph_bulk_nodes" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_bulk_nodes" | "rustyred_thg_graph_bulk_nodes" => {
            let records = arguments
                .get("nodes")
                .or_else(|| arguments.get("records"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpError::invalid_params("rustyred_thg_bulk_nodes requires nodes array")
                })?;
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
            json!({
                "tenant": tenant,
                "ok": errors.is_empty(),
                "inserted": inserted,
                "failed": errors.len(),
                "errors": errors,
            })
        }
        "rustyred_thg_bulk_edges" | "rustyred_thg_graph_bulk_edges" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_bulk_edges" | "rustyred_thg_graph_bulk_edges" => {
            let records = arguments
                .get("edges")
                .or_else(|| arguments.get("records"))
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    McpError::invalid_params("rustyred_thg_bulk_edges requires edges array")
                })?;
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
            json!({
                "tenant": tenant,
                "ok": errors.is_empty(),
                "inserted": inserted,
                "failed": errors.len(),
                "errors": errors,
            })
        }
        "rustyred_thg_vector_search" => {
            let property = arguments
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    McpError::invalid_params("rustyred_thg_vector_search requires property")
                })?;
            let query = parse_f32_array(&arguments, "query")?;
            let k = arguments.get("k").and_then(Value::as_u64).unwrap_or(10) as usize;
            let label = arguments.get("label").and_then(Value::as_str);
            let results = backend.vector_search(label, property, &query, k)?;
            json!({
                "tenant": tenant,
                "results": results.iter().map(|(id, score)| json!({"node_id": id, "score": score})).collect::<Vec<_>>(),
                "stats": { "returned": results.len(), "k": k }
            })
        }
        "rustyred_thg_vector_hybrid" => {
            let property = arguments
                .get("property")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    McpError::invalid_params("rustyred_thg_vector_hybrid requires property")
                })?;
            let query = parse_f32_array(&arguments, "query")?;
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
                scoring.edge_type_weights =
                    serde_json::from_value(weights.clone()).map_err(|error| {
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
            json!({
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
            })
        }
        "rustyred_thg_vector_designate" if config.read_only => {
            return Ok(tool_result_error(json!({
                "error": "mcp_read_only",
                "message": "Write tools are unavailable while read-only mode is active."
            })))
        }
        "rustyred_thg_vector_designate" => {
            let label = arguments
                .get("label")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    McpError::invalid_params("rustyred_thg_vector_designate requires label")
                })?;
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
            json!({
                "tenant": tenant,
                "designated": { "label": label, "property": property, "dimension": dimension }
            })
        }
        "rustyred_thg_epistemic_neighbors" => {
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
            json!({
                "tenant": tenant,
                "node_id": node_id,
                "results": results.iter().map(|(edge, node)| json!({"edge": edge, "node": node})).collect::<Vec<_>>(),
                "stats": { "returned": results.len() }
            })
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

    Ok(tool_result(payload))
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
            room_id: resolved_coordination_room_id(arguments),
            actor_id: required_text_any(
                arguments,
                &["actor", "actor_id", "actorId"],
                "write_intent",
            )?,
            status: argument_text(arguments, &["status"]).unwrap_or_else(|| "working".to_string()),
            summary: required_text_any(arguments, &["summary"], "write_intent")?,
            claimed_files: string_array_any(arguments, &["claimed_files", "claimedFiles"]),
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
            message: required_text_any(arguments, &["message"], "coordinate")?,
            mentions: string_array_any(arguments, &["mentions"]),
            metadata: argument_object(arguments, "metadata"),
            created_at: argument_text(arguments, &["created_at", "createdAt"]).unwrap_or_default(),
        },
    )?;
    Ok(json!({
        "tenant": tenant,
        "ok": true,
        "room_id": room_id,
        "message_id": message.message_id,
        "mentions": message.mentions,
        "unread_count": message.mentions.len(),
        "urgency": message.urgency,
        "created_at": message.created_at
    }))
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
        read_coordination_mentions(backend, tenant, actor_id, false, mention_limit)?
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
    let actor_id = require_nonempty(input.actor_id.trim(), "coordination_room requires actor")?;
    let room_id = if input.room_id.trim().is_empty() {
        infer_coordination_room_id(&input.repo, &input.branch, &input.task, &input.session_id)
    } else {
        input.room_id.trim().to_string()
    };
    let now = timestamp_or_now(&input.updated_at);
    let mut state = load_coordination_room(backend, &tenant_slug, &room_id)?
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
    let room_id = if input.room_id.trim().is_empty() {
        "room:ungrouped".to_string()
    } else {
        input.room_id.trim().to_string()
    };
    let actor_id = require_nonempty(input.actor_id.trim(), "write_intent requires actor")?;
    let summary = require_nonempty(input.summary.trim(), "write_intent requires summary")?;
    let status = normalize_coordination_status(&input.status)?;
    let now = timestamp_or_now(&input.updated_at);
    if load_coordination_room(backend, &tenant_slug, &room_id)?.is_none() {
        persist_coordination_room(
            backend,
            &empty_coordination_room(&tenant_slug, &room_id, &now),
        )?;
    }
    let prior = load_coordination_intent(backend, &tenant_slug, &room_id, &actor_id)?;
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
        claimed_files: normalize_string_vec(input.claimed_files),
        expected_completion: input.expected_completion.trim().to_string(),
        repo: input.repo.trim().to_string(),
        branch: input.branch.trim().to_string(),
        task: input.task.trim().to_string(),
        started_at,
        updated_at: now,
    };
    persist_coordination_intent(backend, &intent)?;
    Ok(intent)
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
        actor_id: require_nonempty(input.actor_id.trim(), "presence requires actor")?,
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
    let actor_id = require_nonempty(input.actor_id.trim(), "coordinate requires actor")?;
    let message = require_nonempty(input.message.trim(), "coordinate requires message")?;
    let urgency = normalize_coordination_urgency(&input.urgency)
        .map_err(|error| McpError::invalid_params(error.to_string()))?;
    let created_at = timestamp_or_now(&input.created_at);
    let mentions = merge_string_vecs(
        parse_coordination_mentions(&message),
        normalize_string_vec(input.mentions),
    );
    let message_id = if input.message_id.trim().is_empty() {
        stable_coordination_message_id(&tenant_slug, &room_id, &actor_id, &message, &created_at)
    } else {
        input.message_id.trim().to_string()
    };
    if load_coordination_room(backend, &tenant_slug, &room_id)?.is_none() {
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
        message,
        mentions,
        metadata: input.metadata,
        consumed_by: Vec::new(),
        created_at,
    };
    persist_coordination_message(backend, &message)?;
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
    let actor_id = require_nonempty(input.actor_id.trim(), "coordination_record requires actor")?;
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
    if load_coordination_room(backend, &tenant_slug, &room_id)?.is_none() {
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
    let mut intents = backend
        .query_nodes(
            NodeQuery::label("CoordinationIntent")
                .with_property("tenant_slug", Value::String(normalize_tenant_slug(tenant)))
                .with_property("room_id", Value::String(room_id.to_string())),
        )?
        .into_iter()
        .map(|node| parse_node_properties::<CoordinationIntentState>(node.properties))
        .filter_map(|result| match result {
            Ok(intent) if filters.is_empty() || filters.contains(&intent.status) => {
                Some(Ok(intent))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    let mut messages = backend
        .query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(normalize_tenant_slug(tenant)))
                .with_property("room_id", Value::String(room_id.to_string())),
        )?
        .into_iter()
        .map(|node| parse_node_properties::<CoordinationMessageState>(node.properties))
        .collect::<Result<Vec<_>, _>>()?;
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
    let mut records = backend
        .query_nodes(
            NodeQuery::label("CoordinationRecord")
                .with_property("tenant_slug", Value::String(normalize_tenant_slug(tenant)))
                .with_property("room_id", Value::String(room_id.to_string())),
        )?
        .into_iter()
        .map(|node| parse_node_properties::<CoordinationRecordState>(node.properties))
        .filter_map(|result| match result {
            Ok(record) if filters.is_empty() || filters.contains(&record.record_type) => {
                Some(Ok(record))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
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
    let actor_id = require_nonempty(actor_id.trim(), "mentions requires actor")?;
    let mut messages = backend
        .query_nodes(
            NodeQuery::label("CoordinationMessage")
                .with_property("tenant_slug", Value::String(normalize_tenant_slug(tenant))),
        )?
        .into_iter()
        .map(|node| parse_node_properties::<CoordinationMessageState>(node.properties))
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
        .collect::<Result<Vec<_>, _>>()?;
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
    let mut presence = backend
        .query_nodes(
            NodeQuery::label("CoordinationPresence")
                .with_property("tenant_slug", Value::String(normalize_tenant_slug(tenant))),
        )?
        .into_iter()
        .map(|node| parse_node_properties::<CoordinationPresenceState>(node.properties))
        .collect::<Result<Vec<_>, _>>()?;
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
    backend
        .get_node(&coordination_room_node_id(tenant, room_id))?
        .map(|node| parse_node_properties::<CoordinationRoomState>(node.properties))
        .transpose()
}

fn load_coordination_intent(
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
    backend
        .get_node(&coordination_presence_node_id(tenant, actor_id))?
        .map(|node| parse_node_properties::<CoordinationPresenceState>(node.properties))
        .transpose()
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

fn normalize_tenant_slug(tenant: &str) -> String {
    let tenant = tenant.trim().to_lowercase();
    if tenant.is_empty() {
        "default".to_string()
    } else {
        tenant
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

fn coordination_policy_receipt(
    context: &McpRequestContext,
    arguments: &Value,
    tool_name: &str,
) -> Value {
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
    let decision = if scope_allowed && budget_allowed {
        "allow"
    } else {
        "deny"
    };
    json!({
        "tool": tool_name,
        "decision": decision,
        "required_scopes": required_scopes,
        "granted_scopes": context.scopes.clone(),
        "missing_scopes": missing_scopes,
        "scope_allowed": scope_allowed,
        "estimated_cost_units": estimated_cost_units,
        "budget_units": budget_units,
        "budget_allowed": budget_allowed
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
    let code = if missing_scopes {
        "coordination_scope_denied"
    } else if !budget_allowed {
        "coordination_budget_exceeded"
    } else {
        "coordination_policy_denied"
    };
    Some(json!({
        "error": code,
        "message": "Native coordination policy denied this write.",
        "policy_receipt": policy_receipt
    }))
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

fn required_str<'a>(arguments: &'a Value, key: &str, tool_name: &str) -> Result<&'a str, McpError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| McpError::invalid_params(format!("{tool_name} requires {key}")))
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

fn tool_result(payload: Value) -> Value {
    json!({
        "content": [{
            "type": "text",
            "text": serde_json::to_string_pretty(&payload).unwrap_or_else(|_| payload.to_string())
        }],
        "structuredContent": payload
    })
}

fn tool_result_error(payload: Value) -> Value {
    let mut result = tool_result(payload);
    if let Value::Object(map) = &mut result {
        map.insert("isError".to_string(), Value::Bool(true));
    }
    result
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

fn tool_definitions(config: &McpServerConfig) -> Vec<Value> {
    let mut tools = vec![
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
                "anyOf": [
                    { "required": ["run_id"] },
                    { "required": ["runId"] }
                ]
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
    if !config.read_only {
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
            "Write this actor's native Theorem harness room intent.",
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
                    "claimed_files": { "type": "array", "items": { "type": "string" } },
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
                    "message": { "type": "string" },
                    "mentions": { "type": "array", "items": { "type": "string" } },
                    "metadata": { "type": "object" }
                },
                "required": ["actor", "message"]
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
                "anyOf": [
                    { "required": ["transition"] },
                    { "required": ["type"] },
                    { "required": ["event_type"] },
                    { "required": ["eventType"] }
                ]
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
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "openWorldHint": false
        }
    })
}

fn tool_write(name: &str, description: &str, input_schema: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": input_schema,
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "openWorldHint": false
        }
    })
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

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use rustyred_thg_core::{
        EdgeRecord, EpistemicType, GraphSnapshot, GraphStats, GraphStoreResult,
        HybridScoringConfig, InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
        VectorDesignation, VerifyReport,
    };
    use serde_json::{json, Value};
    use theorem_harness_core::TransitionInput;

    use super::{
        append_harness_transition_to_store, handle_mcp_request, handle_mcp_request_with_context,
        harness_run_detail_from_store, McpError, McpGraphBackend, McpGraphProvider,
        McpRequestContext, McpServerConfig,
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

    fn has_tool(tools: &[Value], name: &str) -> bool {
        tools.iter().any(|tool| tool["name"] == name)
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
        let (provider, config) = fixture();
        let response = handle_mcp_request(
            &provider,
            &config,
            json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        );

        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_graph_neighbors"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_algorithm_pagerank"));
        assert!(tools
            .iter()
            .any(|tool| tool["name"] == "rustyred_thg_fulltext_search"));
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
        assert!(has_tool(tools, "mentions"));
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
        assert!(has_tool(tools, "coordination_room"));
        assert!(has_tool(tools, "presence"));
        assert!(has_tool(tools, "coordination_intent"));
        assert!(has_tool(tools, "coordinate"));
        assert!(has_tool(tools, "coordination_record"));
        assert!(has_tool(tools, "coordination_contribution"));
        assert!(has_tool(tools, "mentions"));
        assert!(has_tool(tools, "read_records_for_room"));
        assert!(has_tool(tools, "coordination_context"));
        assert!(has_tool(tools, "harness_run"));
        assert!(has_tool(tools, "harness_append_transition"));
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

        let receipt = call_tool_json(
            &provider,
            &config,
            "coordinate",
            json!({
                "tenant": "smoke",
                "actor": "codex",
                "room_id": "harness-rust-port",
                "urgency": "ask",
                "message": "@claude-code please test native MCP",
                "metadata": { "commit": "pending" },
                "created_at": "2026-06-01T00:03:00Z"
            }),
        );
        assert_eq!(receipt["ok"], true);
        assert_eq!(receipt["mentions"], json!(["claude-code"]));
        assert_eq!(receipt["unread_count"], 1);
        assert_eq!(receipt["urgency"], "ask");

        let mentions = call_tool_json(
            &provider,
            &config,
            "mentions",
            json!({
                "tenant": "smoke",
                "actor": "claude-code",
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
            "@claude-code please test native MCP"
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
    fn native_harness_run_transitions_round_trip_through_mcp() {
        let (provider, mut config) = fixture();
        config.read_only = false;
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
        assert_eq!(detail["detail"]["run"]["last_event_seq"], 11);
        assert_eq!(detail["detail"]["events"].as_array().unwrap().len(), 11);
        assert_eq!(detail["detail"]["events"][6]["type"], "CONTEXT.PACKED");
        assert_eq!(
            detail["detail"]["events"][6]["payload"]["token_ledger"]["saved"],
            300
        );
        assert_eq!(
            detail["detail"]["events"][9]["payload"]["validator_results"][0]["status"],
            "passed"
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
            allowed["contribution"]["metadata"]["policy_receipt"]["required_scopes"],
            json!(["coordination:write"])
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
