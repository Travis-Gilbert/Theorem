//! Federated connector gateway for the harness MCP surface.
//!
//! Two spoke classes deliberately stay separate. Federated-MCP spokes are
//! external MCP servers whose `tools/list` catalog is registered as tenant-scoped
//! `Affordance` nodes, then re-exposed through thin search/describe/invoke
//! meta-tools here. Ingestion spokes, such as a GitHub App webhook receiver,
//! write source data into the graph through bespoke verified mappings; they do
//! not add callable tools to the consumer-facing MCP catalog.

use rustyred_thg_affordances::{
    affordance_node_id, select_affordances, Affordance, AffordanceGraphStore, AffordanceRef,
    CapabilityScope, SelectionRequest,
};
use rustyred_thg_connectors::{
    invoke_affordance as invoke_connector_affordance, ConnectorError, InvokePolicy, InvokeReport,
    InvokeRequest, PlannedInvocation,
};
use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, GraphStoreError,
    GraphStoreResult, GraphTransaction, GraphWriteResult, NeighborHit, NeighborQuery, NodeQuery,
    NodeRecord,
};
use serde_json::{json, Value};

use crate::{McpError, McpGraphBackend};

struct BackendAffordanceStore<'a, B: McpGraphBackend> {
    backend: &'a mut B,
}

impl<'a, B: McpGraphBackend> BackendAffordanceStore<'a, B> {
    fn new(backend: &'a mut B) -> Self {
        Self { backend }
    }
}

impl<B: McpGraphBackend> AffordanceGraphStore for BackendAffordanceStore<'_, B> {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        let id = node.id.clone();
        let checksum = stable_hash(json!({ "node": node }));
        self.backend.upsert_node(node)?;
        let version = self.backend.stats()?.version;
        Ok(GraphWriteResult {
            id,
            version,
            checksum,
        })
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        let id = edge.id.clone();
        let checksum = stable_hash(json!({ "edge": edge }));
        self.backend.upsert_edge(edge)?;
        let version = self.backend.stats()?.version;
        Ok(GraphWriteResult {
            id,
            version,
            checksum,
        })
    }

    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        if batch.mutations.is_empty() {
            return Err(GraphStoreError::new(
                "empty_graph_transaction",
                "graph transaction requires at least one mutation",
            ));
        }
        let mut writes = Vec::with_capacity(batch.mutations.len());
        for mutation in batch.mutations {
            match mutation {
                GraphMutation::NodeUpsert(node) => writes.push(self.upsert_node(node)?),
                GraphMutation::EdgeUpsert(edge) => writes.push(self.upsert_edge(edge)?),
            }
        }
        let graph_version = self.backend.stats()?.version;
        Ok(GraphTransaction {
            txn_id: graph_version,
            graph_version,
            writes,
        })
    }

    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.backend.get_node(id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.backend.get_edge(id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.backend.query_nodes(query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.backend.neighbors(query)
    }

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        self.backend.graph_snapshot()
    }
}

pub(crate) fn search_payload<B: McpGraphBackend>(
    tenant: &str,
    backend: &mut B,
    arguments: &Value,
) -> Result<Value, McpError> {
    let query = first_string(arguments, &["query", "q", "task", "task_type"])
        .unwrap_or_else(|| "general".to_string());
    let task_type =
        first_string(arguments, &["task_type", "taskType"]).unwrap_or_else(|| query.clone());
    let k = first_u64(arguments, &["k", "limit", "top_k", "topK"]).unwrap_or(10) as u32;
    let candidate_k = k.saturating_mul(4).max(k).max(10);
    let scope = scope_from_args(arguments);
    let store = BackendAffordanceStore::new(backend);
    let request = SelectionRequest {
        tenant_id: tenant.to_string(),
        task_type: task_type.clone(),
        k: candidate_k,
        scope,
        min_fitness: first_f64(arguments, &["min_fitness", "minFitness"]).map(|v| v as f32),
        ppr_damping: first_f64(arguments, &["ppr_damping", "pprDamping"])
            .map(|v| v as f32)
            .unwrap_or_default(),
        ppr_max_iter: first_u64(arguments, &["ppr_max_iter", "pprMaxIter"])
            .and_then(|v| u32::try_from(v).ok())
            .unwrap_or_default(),
    };
    let mut selected = select_affordances(&store, &request).map_err(mcp_affordance_error)?;
    let narrowed = lexical_narrow(&selected, &query);
    if !narrowed.is_empty() {
        selected = narrowed;
    }
    selected.truncate(k as usize);
    let candidate_affordance_ids = selected
        .iter()
        .map(|item| item.affordance.affordance_id.clone())
        .collect::<Vec<_>>();
    Ok(json!({
        "tenant": tenant,
        "query": query,
        "task_type": task_type,
        "results": selected.iter().map(compact_affordance).collect::<Vec<_>>(),
        "candidate_affordance_ids": candidate_affordance_ids,
        "ranking": "affordance-selection-ppr-plus-fitness",
        "catalog_spend": "compact-only"
    }))
}

pub(crate) fn describe_payload<B: McpGraphBackend>(
    tenant: &str,
    backend: &mut B,
    arguments: &Value,
) -> Result<Value, McpError> {
    let affordance_id = required_string(arguments, "affordance_id", "describe")?;
    let node_id = affordance_node_id(tenant, &affordance_id);
    let node = backend.get_node(&node_id)?.ok_or_else(|| {
        McpError::invalid_params(format!(
            "affordance {affordance_id} is not registered for tenant {tenant}"
        ))
    })?;
    let affordance = Affordance::from_node_record(&node).map_err(mcp_affordance_error)?;
    Ok(json!({
        "tenant": tenant,
        "affordance_id": affordance.affordance_id,
        "server_id": affordance.server_id,
        "tool_name": affordance.tool_name,
        "name": affordance.label,
        "description": affordance.description,
        "input_schema": affordance.input_schema,
        "permissions": affordance.permissions,
        "writeback_policy": affordance.writeback_policy,
        "tags": affordance.tags,
        "fitness": affordance.fitness
    }))
}

pub(crate) fn invoke_payload<B: McpGraphBackend>(
    tenant: &str,
    backend: &mut B,
    arguments: &Value,
) -> Result<Value, McpError> {
    let affordance_id = required_string(arguments, "affordance_id", "invoke")?;
    let tool_arguments = arguments
        .get("arguments")
        .or_else(|| arguments.get("tool_arguments"))
        .or_else(|| arguments.get("toolArguments"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    let task_type = first_string(arguments, &["task_type", "taskType", "query", "task"])
        .unwrap_or_else(|| "gateway-invoke".to_string());
    let candidate_affordance_ids = string_list(arguments, &["candidate_affordance_ids"])
        .into_iter()
        .chain(string_list(arguments, &["candidateAffordanceIds"]))
        .collect::<Vec<_>>();
    let candidate_affordance_ids = if candidate_affordance_ids.is_empty() {
        vec![affordance_id.clone()]
    } else {
        candidate_affordance_ids
    };
    let actor = first_string(arguments, &["actor", "actor_id", "actorId"]);
    let dry_run = first_bool(arguments, &["dry_run", "dryRun"]).unwrap_or(false);
    let policy = if dry_run {
        InvokePolicy::DryRun
    } else {
        InvokePolicy::FireAllowlist(vec![affordance_id.clone()])
    };
    let mut store = BackendAffordanceStore::new(backend);
    let report = invoke_connector_affordance(
        &mut store,
        InvokeRequest {
            tenant_id: tenant.to_string(),
            task_type: task_type.clone(),
            affordance_id,
            arguments: tool_arguments,
            candidate_affordance_ids,
        },
        &policy,
        actor.as_deref(),
    )
    .map_err(mcp_connector_error)?;
    Ok(invoke_report_payload(tenant, &task_type, report))
}

fn invoke_report_payload(tenant: &str, task_type: &str, report: InvokeReport) -> Value {
    json!({
        "tenant": tenant,
        "task_type": task_type,
        "planned": planned_invocation_payload(&report.planned),
        "fired": report.fired,
        "dry_run": !report.fired,
        "dry_run_reason": report.dry_run_reason,
        "outcome": report.outcome.map(|outcome| json!({
            "is_error": outcome.is_error,
            "text": outcome.text
        })),
        "recorded": report.recorded
    })
}

fn planned_invocation_payload(planned: &PlannedInvocation) -> Value {
    json!({
        "affordance_id": planned.affordance_id,
        "server_id": planned.server_id,
        "tool_name": planned.tool_name,
        "writeback_policy": planned.writeback_policy
    })
}

fn compact_affordance(item: &AffordanceRef) -> Value {
    let affordance = &item.affordance;
    let name = if affordance.label.trim().is_empty() {
        affordance.tool_name.as_str()
    } else {
        affordance.label.as_str()
    };
    json!({
        "affordance_id": affordance.affordance_id,
        "name": name,
        "one_line_description": one_line(&affordance.description),
        "server_id": affordance.server_id,
        "tool_name": affordance.tool_name,
        "family": affordance.family,
        "writeback_policy": affordance.writeback_policy,
        "score": item.score,
        "fitness": affordance.fitness
    })
}

fn lexical_narrow(items: &[AffordanceRef], query: &str) -> Vec<AffordanceRef> {
    let terms = query_terms(query);
    if terms.is_empty() {
        return Vec::new();
    }
    items
        .iter()
        .filter(|item| {
            let haystack = format!(
                "{} {} {} {} {}",
                item.affordance.affordance_id,
                item.affordance.label,
                item.affordance.tool_name,
                item.affordance.family,
                item.affordance.description
            )
            .to_ascii_lowercase();
            terms.iter().any(|term| haystack.contains(term))
        })
        .cloned()
        .collect()
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() > 2)
        .map(str::to_ascii_lowercase)
        .collect()
}

fn one_line(description: &str) -> String {
    let mut line = description.lines().next().unwrap_or("").trim().to_string();
    if line.len() > 180 {
        line.truncate(177);
        line.push_str("...");
    }
    line
}

fn scope_from_args(arguments: &Value) -> CapabilityScope {
    CapabilityScope {
        agent_id: first_string(arguments, &["agent_id", "actor", "actor_id"])
            .unwrap_or_else(|| "gateway-consumer".to_string()),
        allow_affordance_ids: string_list(arguments, &["allow_affordance_ids", "affordance_ids"]),
        allow_servers: string_list(arguments, &["allow_servers", "server_ids", "servers"]),
        allow_families: string_list(arguments, &["allow_families", "families"]),
        allow_tags: string_list(arguments, &["allow_tags", "tags"]),
    }
}

fn string_list(arguments: &Value, keys: &[&str]) -> Vec<String> {
    for key in keys {
        if let Some(values) = arguments.get(*key).and_then(Value::as_array) {
            return values
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
        }
    }
    Vec::new()
}

fn required_string(arguments: &Value, key: &str, tool_name: &str) -> Result<String, McpError> {
    arguments
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .ok_or_else(|| McpError::invalid_params(format!("{tool_name} requires {key}")))
}

fn first_string(arguments: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        arguments
            .get(*key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    })
}

fn first_bool(arguments: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_bool))
}

fn first_u64(arguments: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_u64))
}

fn first_f64(arguments: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_f64))
}

fn mcp_affordance_error(error: rustyred_thg_core::ThgError) -> McpError {
    McpError {
        code: -32603,
        message: error.message,
        data: Some(json!({ "code": error.code })),
    }
}

fn mcp_connector_error(error: ConnectorError) -> McpError {
    let code = match &error {
        ConnectorError::Protocol(_) => "connector_protocol_error",
        ConnectorError::Rpc { .. } => "connector_rpc_error",
        ConnectorError::Transport(_) => "connector_transport_error",
        ConnectorError::Registration(_) => "connector_registration_error",
    };
    McpError {
        code: -32020,
        message: error.to_string(),
        data: Some(json!({ "code": code })),
    }
}
