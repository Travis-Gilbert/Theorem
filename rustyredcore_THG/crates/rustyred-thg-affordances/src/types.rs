//! Affordance node + edge taxonomy over RustyRedCore-THG graph records.
//!
//! An Affordance is a connector tool (an MCP server's tool, or a built-in
//! symbolic engine) modeled as a first-class graph node, so the substrate can
//! learn which affordances to reach for from accumulated outcomes. This crate
//! stays above `rustyred-thg-core`: it reuses core graph records, stores, and
//! PPR, and reuses the affordance vocabulary from `theorem-harness-core`
//! (`AffordanceContract`, `AffordanceReceipt`) rather than forking it.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    manifest_version_compatible, now_ms, sanitize_tenant_segment, EdgeRecord, GraphMutation,
    GraphMutationBatch, GraphSnapshot, GraphStoreError, GraphStoreResult, GraphTransaction,
    GraphWriteResult, InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
    Provenance, RedCoreGraphStore, ThgError, ThgResult, VectorDesignation,
};
use theorem_harness_core::AffordanceContract;

// --- Labels -----------------------------------------------------------------

pub const AFFORDANCE_LABEL: &str = "Affordance";
pub const CONNECTOR_LABEL: &str = "Connector";
pub const TASK_TYPE_LABEL: &str = "TaskType";
pub const INVOCATION_RECEIPT_LABEL: &str = "InvocationReceipt";
pub const TENANT_LABEL: &str = "Tenant";

// --- Edge types -------------------------------------------------------------

/// Connector -> Affordance: the owning server offers this tool.
pub const OFFERS: &str = "OFFERS";
/// Affordance -> TaskType: this affordance served this task shape.
pub const SERVED_TASK: &str = "SERVED_TASK";
/// Affordance -> InvocationReceipt: outcome-weighted result of a call.
pub const PRODUCED_OUTCOME: &str = "PRODUCED_OUTCOME";
/// Affordance -> Affordance: commonly sequenced together in a session.
pub const SEQUENCED_WITH: &str = "SEQUENCED_WITH";

pub const THG_AFFORDANCE_SOURCE: &str = "thg-affordances";

// --- Tuning constants (mirror the adapter catalog) --------------------------

pub const DEFAULT_MIN_FITNESS: f32 = 0.3;
pub const DEFAULT_BASE_FITNESS: f32 = 0.5;
pub const DEFAULT_FITNESS_EPSILON: f32 = 0.05;
pub const DEFAULT_PPR_DAMPING: f32 = 0.85;
pub const DEFAULT_PPR_MAX_PUSHES: u32 = 30;
pub const DEFAULT_HALF_LIFE_DAYS: f32 = 30.0;
/// Base structural score floor so a freshly registered (unprimed) affordance is
/// still reachable. This is the spec's "forwarding is the fallback for an
/// affordance that has no learned prior yet."
pub const DEFAULT_COLD_START_SCORE: f64 = 0.05;

// --- Store trait ------------------------------------------------------------

/// Narrow graph interface used by the affordance registry. Mirrors the
/// adapter catalog's `AdapterGraphStore` so `RedCoreGraphStore`'s inherent
/// methods (which shadow the `GraphStore` trait) are reached cleanly. A future
/// shared `CatalogGraphStore` trait is a promotion candidate.
pub trait AffordanceGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult>;
    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult>;
    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction>;
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot>;
}

impl AffordanceGraphStore for InMemoryGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        InMemoryGraphStore::upsert_node(self, node)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        InMemoryGraphStore::upsert_edge(self, edge)
    }

    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        if batch.mutations.is_empty() {
            return Err(GraphStoreError::new(
                "empty_graph_transaction",
                "graph transaction requires at least one mutation",
            ));
        }
        let mut staged = self.clone();
        let mut writes = Vec::with_capacity(batch.mutations.len());
        for mutation in batch.mutations {
            match mutation {
                GraphMutation::NodeUpsert(node) => writes.push(staged.upsert_node(node)?),
                GraphMutation::EdgeUpsert(edge) => writes.push(staged.upsert_edge(edge)?),
            }
        }
        let graph_version = staged.stats().version;
        *self = staged;
        Ok(GraphTransaction {
            txn_id: graph_version,
            graph_version,
            writes,
        })
    }

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

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(InMemoryGraphStore::snapshot(self))
    }
}

impl AffordanceGraphStore for RedCoreGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        RedCoreGraphStore::upsert_node(self, node)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        RedCoreGraphStore::upsert_edge(self, edge)
    }

    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        RedCoreGraphStore::commit_batch(self, batch)
    }

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

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(RedCoreGraphStore::graph_snapshot(self))
    }
}

// --- Affordance node --------------------------------------------------------

/// A connector tool modeled as a graph node. The semantic `description` is
/// embedded as the `embedding` vector (caller-supplied; RustyRed has no
/// text->vector embedder). With no embedding, selection degrades to structural
/// PPR + fitness.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Affordance {
    pub affordance_id: String,
    pub tenant_id: String,
    pub server_id: String,
    pub tool_name: String,
    pub family: String,
    pub label: String,
    pub description: String,
    pub input_schema: Value,
    pub permissions: Vec<String>,
    pub cost: Value,
    pub writeback_policy: String,
    pub tags: Vec<String>,
    pub embedding: Option<Vec<f32>>,
    pub fitness: f32,
    pub version: u32,
    pub created_at_ms: i64,
    pub manifest_version: u32,
}

impl Affordance {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = sanitize_tenant_segment(&self.tenant_id);
        self.affordance_id = self.affordance_id.trim().to_string();
        self.server_id = self.server_id.trim().to_string();
        self.tool_name = self.tool_name.trim().to_string();
        self.family = self.family.trim().to_string();
        if self.family.is_empty() {
            self.family = "connector".to_string();
        }
        if self.writeback_policy.trim().is_empty() {
            self.writeback_policy = "read-only".to_string();
        }
        self.permissions = clean_strings(self.permissions);
        self.tags = clean_strings(self.tags);
        self.fitness = self.fitness.clamp(0.0, 1.0);
        if self.fitness == 0.0 {
            self.fitness = DEFAULT_BASE_FITNESS;
        }
        if self.version == 0 {
            self.version = 1;
        }
        if self.created_at_ms <= 0 {
            self.created_at_ms = now_ms();
        }
        if self.manifest_version == 0 {
            self.manifest_version = 1;
        }
        if let Some(embedding) = self.embedding.as_ref() {
            if embedding.is_empty() {
                self.embedding = None;
            }
        }
        self
    }

    pub fn validate(&self) -> ThgResult<()> {
        if self.affordance_id.trim().is_empty() {
            return Err(ThgError::new(
                "invalid_affordance",
                "affordance_id is required",
            ));
        }
        if self.tenant_id.trim().is_empty() {
            return Err(ThgError::new("invalid_affordance", "tenant_id is required"));
        }
        if self.server_id.trim().is_empty() {
            return Err(ThgError::new("invalid_affordance", "server_id is required"));
        }
        if self.tool_name.trim().is_empty() {
            return Err(ThgError::new("invalid_affordance", "tool_name is required"));
        }
        if !manifest_version_compatible(self.manifest_version) {
            return Err(ThgError::new(
                "affordance_manifest_incompatible",
                format!(
                    "affordance manifest_version {} is newer than this THG build can read",
                    self.manifest_version
                ),
            ));
        }
        Ok(())
    }

    pub fn node_id(&self) -> String {
        affordance_node_id(&self.tenant_id, &self.affordance_id)
    }

    pub fn to_node_record(&self, actor: Option<&str>, extra_properties: Value) -> NodeRecord {
        let mut properties = json!({
            "affordance_id": self.affordance_id,
            "tenant_id": self.tenant_id,
            "server_id": self.server_id,
            "tool_name": self.tool_name,
            "family": self.family,
            "label": self.label,
            "description": self.description,
            "input_schema": self.input_schema,
            "permissions": self.permissions,
            "cost": self.cost,
            "writeback_policy": self.writeback_policy,
            "tags": self.tags,
            "fitness": self.fitness,
            "version": self.version,
            "created_at_ms": self.created_at_ms,
            "manifest_version": self.manifest_version,
            "source": THG_AFFORDANCE_SOURCE,
        });
        if let Some(embedding) = self.embedding.as_ref() {
            properties["embedding"] = json!(embedding);
        }
        if let Some(actor) = actor.filter(|actor| !actor.trim().is_empty()) {
            properties["actor"] = json!(actor);
        }
        merge_object_properties(&mut properties, extra_properties);
        NodeRecord::new(self.node_id(), [AFFORDANCE_LABEL], properties)
    }

    pub fn from_node_record(node: &NodeRecord) -> ThgResult<Self> {
        if !node.labels.iter().any(|label| label == AFFORDANCE_LABEL) {
            return Err(ThgError::new(
                "invalid_affordance_node",
                format!("node {} is not labeled {AFFORDANCE_LABEL}", node.id),
            ));
        }
        let props = &node.properties;
        Ok(Self {
            affordance_id: required_string(props, "affordance_id")?,
            tenant_id: required_string(props, "tenant_id")?,
            server_id: required_string(props, "server_id")?,
            tool_name: required_string(props, "tool_name")?,
            family: optional_string(props, "family").unwrap_or_else(|| "connector".to_string()),
            label: optional_string(props, "label").unwrap_or_default(),
            description: optional_string(props, "description").unwrap_or_default(),
            input_schema: props.get("input_schema").cloned().unwrap_or_else(|| json!({})),
            permissions: optional_string_vec(props, "permissions"),
            cost: props.get("cost").cloned().unwrap_or_else(|| json!({})),
            writeback_policy: optional_string(props, "writeback_policy")
                .unwrap_or_else(|| "read-only".to_string()),
            tags: optional_string_vec(props, "tags"),
            embedding: embedding_from_properties(props),
            fitness: optional_f32(props, "fitness")
                .unwrap_or(DEFAULT_BASE_FITNESS)
                .clamp(0.0, 1.0),
            version: optional_u32(props, "version").unwrap_or(1),
            created_at_ms: optional_i64(props, "created_at_ms").unwrap_or_default(),
            manifest_version: optional_u32(props, "manifest_version").unwrap_or(1),
        })
    }

    /// Bridge a built-in `theorem-harness-core` symbolic-engine contract into a
    /// graph-node affordance, so the existing 11 affordances are first-class
    /// nodes too, not only newly connected MCP tools.
    pub fn from_contract(contract: &AffordanceContract, tenant_id: &str) -> Self {
        let tool_name = contract
            .affordance_id
            .rsplit('.')
            .next()
            .unwrap_or(&contract.affordance_id)
            .to_string();
        Self {
            affordance_id: contract.affordance_id.clone(),
            tenant_id: tenant_id.to_string(),
            server_id: contract.engine_id.clone(),
            tool_name,
            family: contract.family.clone(),
            label: contract.label.clone(),
            description: format!(
                "{} ({} -> {})",
                contract.label, contract.input_shape, contract.output_shape
            ),
            input_schema: json!({
                "input_shape": contract.input_shape,
                "output_shape": contract.output_shape,
            }),
            permissions: contract.permissions.clone(),
            cost: json!({
                "execution_surface": contract.execution_surface,
                "parity_status": contract.parity_status,
            }),
            writeback_policy: contract.writeback_policy.clone(),
            tags: contract.tags.clone(),
            embedding: None,
            fitness: DEFAULT_BASE_FITNESS,
            version: 1,
            created_at_ms: 0,
            manifest_version: 1,
        }
        .normalized()
    }
}

// --- Capability scope (the charter) -----------------------------------------

/// The capability-scope plane of the AgentBinding: the subset of affordances an
/// agent is primed for. Selection filters to this scope, then the learned prior
/// ranks within it. An unrestricted scope (all lists empty) admits everything;
/// any populated dimension imposes an intersection constraint, so a scope can
/// be `allow_servers=["github"]` AND `allow_tags=["write"]` for github write
/// tools only.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CapabilityScope {
    pub agent_id: String,
    #[serde(default)]
    pub allow_affordance_ids: Vec<String>,
    #[serde(default)]
    pub allow_servers: Vec<String>,
    #[serde(default)]
    pub allow_families: Vec<String>,
    #[serde(default)]
    pub allow_tags: Vec<String>,
}

impl CapabilityScope {
    pub fn unrestricted(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            ..Default::default()
        }
    }

    pub fn is_unrestricted(&self) -> bool {
        self.allow_affordance_ids.is_empty()
            && self.allow_servers.is_empty()
            && self.allow_families.is_empty()
            && self.allow_tags.is_empty()
    }

    /// Admits an affordance if it satisfies every populated allowlist dimension.
    pub fn admits(&self, affordance: &Affordance) -> bool {
        if self.is_unrestricted() {
            return true;
        }
        if !self.allow_affordance_ids.is_empty()
            && !self
                .allow_affordance_ids
                .iter()
                .any(|id| id == &affordance.affordance_id)
        {
            return false;
        }
        if !self.allow_servers.is_empty()
            && !self
                .allow_servers
                .iter()
                .any(|server| server == &affordance.server_id)
        {
            return false;
        }
        if !self.allow_families.is_empty()
            && !self
                .allow_families
                .iter()
                .any(|family| family == &affordance.family)
        {
            return false;
        }
        if !self.allow_tags.is_empty()
            && !self
                .allow_tags
                .iter()
                .any(|tag| affordance.tags.iter().any(|owned| owned == tag))
        {
            return false;
        }
        true
    }
}

// --- Connector manifest (registration input) --------------------------------

/// The normalized shape of an MCP server's `tools/list` response (the
/// `{name, description, input_schema}` shape the `rustyred-thg-mcp` crate
/// already emits via `tool_definitions`).
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConnectorManifest {
    pub tenant_id: String,
    pub server_id: String,
    #[serde(default)]
    pub label: String,
    pub tools: Vec<ToolManifest>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ToolManifest {
    pub name: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub cost: Value,
    #[serde(default)]
    pub writeback_policy: String,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Caller-supplied semantic embedding of `description`.
    #[serde(default)]
    pub description_embedding: Option<Vec<f32>>,
}

// --- Request / result structs -----------------------------------------------

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AffordanceUpsertResult {
    pub affordance: Affordance,
    pub node_id: String,
    pub edge_count: usize,
    pub transaction: GraphTransaction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConnectorRegisterResult {
    pub tenant_id: String,
    pub server_id: String,
    pub connector_node_id: String,
    pub affordance_node_ids: Vec<String>,
    pub transaction: GraphTransaction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AffordanceRef {
    pub affordance: Affordance,
    pub score: f32,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SelectionRequest {
    pub tenant_id: String,
    pub task_type: String,
    pub k: u32,
    #[serde(default)]
    pub scope: CapabilityScope,
    #[serde(default)]
    pub min_fitness: Option<f32>,
    #[serde(default)]
    pub ppr_damping: f32,
    #[serde(default)]
    pub ppr_max_iter: u32,
}

impl SelectionRequest {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = sanitize_tenant_segment(&self.tenant_id);
        self.task_type = self.task_type.trim().to_string();
        if self.k == 0 {
            self.k = 10;
        }
        if !(0.0..1.0).contains(&self.ppr_damping) {
            self.ppr_damping = DEFAULT_PPR_DAMPING;
        }
        if self.ppr_max_iter == 0 {
            self.ppr_max_iter = DEFAULT_PPR_MAX_PUSHES;
        }
        self.min_fitness = self
            .min_fitness
            .map(|value| value.clamp(0.0, 1.0))
            .or(Some(DEFAULT_MIN_FITNESS));
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InvocationRecordRequest {
    pub tenant_id: String,
    pub task_type: String,
    /// Candidate affordance ids that were considered at selection time.
    pub candidate_affordance_ids: Vec<String>,
    /// The affordance id the caller actually selected and invoked.
    pub selected_affordance_id: String,
    /// Outcome score in [0,1] (1 = success, 0 = failure).
    pub outcome_value: f32,
    /// Weight of this observation (number of trials / confidence).
    pub outcome_weight: f32,
    #[serde(default)]
    pub outcome_label: String,
    /// The affordance selected just before this one in the same session, for
    /// `SEQUENCED_WITH`. Optional.
    #[serde(default)]
    pub previous_affordance_id: Option<String>,
    #[serde(default)]
    pub query_text: String,
    #[serde(default)]
    pub recorded_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct InvocationRecordResult {
    pub receipt_node_id: String,
    pub receipt_hash: String,
    pub served_task_edge_id: String,
    pub produced_outcome_edge_id: String,
    pub sequenced_with_edge_id: Option<String>,
    pub effective_fitness: f32,
    pub graph_version: u64,
    pub transaction: GraphTransaction,
}

// --- Node-id helpers (tenant-scoped) ----------------------------------------

pub fn normalize_tenant_id(tenant_id: &str) -> String {
    sanitize_tenant_segment(tenant_id)
}

pub fn affordance_node_id(tenant_id: &str, affordance_id: &str) -> String {
    format!(
        "affordance:{}:{}",
        sanitize_tenant_segment(tenant_id),
        affordance_id.trim()
    )
}

pub fn connector_node_id(tenant_id: &str, server_id: &str) -> String {
    format!(
        "connector:{}:{}",
        sanitize_tenant_segment(tenant_id),
        server_id.trim()
    )
}

pub fn task_type_node_id(tenant_id: &str, task_type: &str) -> String {
    format!(
        "task_type:{}:{}",
        sanitize_tenant_segment(tenant_id),
        task_type.trim()
    )
}

pub fn invocation_receipt_node_id(tenant_id: &str, receipt_hash: &str) -> String {
    format!(
        "invocation_receipt:{}:{}",
        sanitize_tenant_segment(tenant_id),
        receipt_hash.trim()
    )
}

pub fn tenant_node_id(tenant_id: &str) -> String {
    format!("tenant:{}", sanitize_tenant_segment(tenant_id))
}

pub fn affordance_vector_designation(dimension: usize) -> VectorDesignation {
    VectorDesignation {
        label: AFFORDANCE_LABEL.to_string(),
        property: "embedding".to_string(),
        dimension,
    }
}

// --- Edge + property helpers ------------------------------------------------

pub fn edge_with_affordance_provenance(
    id: impl Into<String>,
    from_id: impl Into<String>,
    edge_type: impl Into<String>,
    to_id: impl Into<String>,
    properties: Value,
    actor: Option<&str>,
) -> EdgeRecord {
    let mut properties = properties;
    if let Some(actor) = actor.filter(|actor| !actor.trim().is_empty()) {
        properties["actor"] = json!(actor);
    }
    properties["source"] = json!(THG_AFFORDANCE_SOURCE);
    EdgeRecord::new(id, from_id, edge_type, to_id, properties).with_provenance(Provenance {
        source_id: actor.map(str::to_string),
        timestamp: Some(now_ms().to_string()),
        method: Some(THG_AFFORDANCE_SOURCE.to_string()),
    })
}

pub fn thg_error_from_store(error: GraphStoreError) -> ThgError {
    ThgError::new(error.code, error.message)
}

pub fn merge_object_properties(target: &mut Value, source: Value) {
    let (Some(target), Some(source)) = (target.as_object_mut(), source.as_object()) else {
        return;
    };
    for (key, value) in source {
        target.insert(key.clone(), value.clone());
    }
}

pub fn embedding_from_properties(properties: &Value) -> Option<Vec<f32>> {
    let array = properties.get("embedding")?.as_array()?;
    let embedding = array
        .iter()
        .filter_map(|item| {
            item.as_f64()
                .map(|value| value as f32)
                .or_else(|| item.as_str().and_then(|raw| raw.parse::<f32>().ok()))
        })
        .collect::<Vec<_>>();
    if embedding.is_empty() {
        None
    } else {
        Some(embedding)
    }
}

pub fn property_f32(properties: &Value, key: &str) -> Option<f32> {
    properties
        .get(key)
        .and_then(|value| value.as_f64().map(|raw| raw as f32))
        .or_else(|| {
            properties
                .get(key)
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<f32>().ok())
        })
}

pub fn property_i64(properties: &Value, key: &str) -> Option<i64> {
    properties.get(key).and_then(Value::as_i64).or_else(|| {
        properties
            .get(key)
            .and_then(Value::as_str)
            .and_then(|raw| raw.parse::<i64>().ok())
    })
}

fn optional_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn required_string(properties: &Value, key: &str) -> ThgResult<String> {
    optional_string(properties, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ThgError::new("invalid_affordance_node", format!("{key} is required")))
}

fn optional_u32(properties: &Value, key: &str) -> Option<u32> {
    properties
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|raw| u32::try_from(raw).ok())
        .or_else(|| {
            properties
                .get(key)
                .and_then(Value::as_str)
                .and_then(|raw| raw.parse::<u32>().ok())
        })
}

fn optional_i64(properties: &Value, key: &str) -> Option<i64> {
    property_i64(properties, key)
}

fn optional_f32(properties: &Value, key: &str) -> Option<f32> {
    property_f32(properties, key)
}

fn optional_string_vec(properties: &Value, key: &str) -> Vec<String> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    values
        .into_iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}
