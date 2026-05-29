use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    manifest_version_compatible, now_ms, sanitize_tenant_segment, EdgeRecord, GraphMutation,
    GraphMutationBatch, GraphSnapshot, GraphStoreError, GraphStoreResult, GraphTransaction,
    GraphWriteResult, InMemoryGraphStore, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
    RedCoreGraphStore, ThgError, ThgResult, VectorDesignation,
};

pub const LORA_ADAPTER_LABEL: &str = "LoraAdapter";
pub const TENANT_LABEL: &str = "Tenant";
pub const TRAINED_ON: &str = "TRAINED_ON";
pub const DERIVED_FROM: &str = "DERIVED_FROM";
pub const SUPERSEDES: &str = "SUPERSEDES";
pub const FITNESS_SIGNAL: &str = "FITNESS_SIGNAL";
pub const SHARED_WITH: &str = "SHARED_WITH";
pub const THG_ADAPTER_SOURCE: &str = "thg-adapters";
pub const DEFAULT_MIN_FITNESS: f32 = 0.3;
pub const DEFAULT_PPR_DAMPING: f32 = 0.85;
pub const DEFAULT_PPR_MAX_PUSHES: u32 = 30;
pub const DEFAULT_SHARED_WEIGHT: f32 = 0.5;
pub const DEFAULT_THESEUS_HALF_LIFE_DAYS: f32 = 14.0;
pub const DEFAULT_FITNESS_EPSILON: f32 = 0.05;

pub trait AdapterGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult>;
    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult>;
    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction>;
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot>;
}

impl AdapterGraphStore for InMemoryGraphStore {
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

impl AdapterGraphStore for RedCoreGraphStore {
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

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LoraAdapter {
    pub adapter_id: String,
    pub tenant_id: String,
    pub base_model_sha: String,
    pub rank: u32,
    pub target_modules: Vec<String>,
    pub s3_uri: String,
    pub training_object_ids: Vec<i64>,
    pub version: u32,
    pub fitness: f32,
    pub created_at_ms: i64,
    pub manifest_version: u32,
}

impl LoraAdapter {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = sanitize_tenant_segment(&self.tenant_id);
        self.target_modules = self
            .target_modules
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
        self.fitness = self.fitness.clamp(0.0, 1.0);
        if self.created_at_ms <= 0 {
            self.created_at_ms = now_ms();
        }
        if self.manifest_version == 0 {
            self.manifest_version = 1;
        }
        self
    }

    pub fn validate(&self) -> ThgResult<()> {
        if self.adapter_id.trim().is_empty() {
            return Err(ThgError::new("invalid_adapter", "adapter_id is required"));
        }
        if self.tenant_id.trim().is_empty() {
            return Err(ThgError::new("invalid_adapter", "tenant_id is required"));
        }
        if self.base_model_sha.trim().is_empty() {
            return Err(ThgError::new(
                "invalid_adapter",
                "base_model_sha is required",
            ));
        }
        if !matches!(self.rank, 8 | 16 | 32 | 64) {
            return Err(ThgError::new(
                "invalid_adapter",
                "rank must be one of 8, 16, 32, or 64",
            ));
        }
        if self.target_modules.is_empty() {
            return Err(ThgError::new(
                "invalid_adapter",
                "target_modules must contain at least one module",
            ));
        }
        if !self.s3_uri.starts_with("s3://") {
            return Err(ThgError::new(
                "invalid_adapter",
                "s3_uri must point at an s3:// adapter artifact",
            ));
        }
        if !manifest_version_compatible(self.manifest_version) {
            return Err(ThgError::new(
                "adapter_manifest_incompatible",
                format!(
                    "adapter manifest_version {} is newer than this THG build can read",
                    self.manifest_version
                ),
            ));
        }
        Ok(())
    }

    pub fn node_id(&self) -> String {
        adapter_node_id(&self.tenant_id, &self.adapter_id)
    }

    pub fn to_node_record(&self, actor: Option<&str>, extra_properties: Value) -> NodeRecord {
        let mut properties = json!({
            "adapter_id": self.adapter_id,
            "tenant_id": self.tenant_id,
            "base_model_sha": self.base_model_sha,
            "rank": self.rank,
            "target_modules": self.target_modules,
            "s3_uri": self.s3_uri,
            "training_object_ids": self.training_object_ids,
            "version": self.version,
            "fitness": self.fitness,
            "created_at_ms": self.created_at_ms,
            "manifest_version": self.manifest_version,
            "source": THG_ADAPTER_SOURCE,
        });
        if let Some(actor) = actor.filter(|actor| !actor.trim().is_empty()) {
            properties["actor"] = json!(actor);
        }
        merge_object_properties(&mut properties, extra_properties);
        NodeRecord::new(self.node_id(), [LORA_ADAPTER_LABEL], properties)
    }

    pub fn from_node_record(node: &NodeRecord) -> ThgResult<Self> {
        if !node.labels.iter().any(|label| label == LORA_ADAPTER_LABEL) {
            return Err(ThgError::new(
                "invalid_adapter_node",
                format!("node {} is not labeled {LORA_ADAPTER_LABEL}", node.id),
            ));
        }
        let props = &node.properties;
        Ok(Self {
            adapter_id: required_string(props, "adapter_id")?,
            tenant_id: required_string(props, "tenant_id")?,
            base_model_sha: required_string(props, "base_model_sha")?,
            rank: required_u32(props, "rank")?,
            target_modules: required_string_vec(props, "target_modules")?,
            s3_uri: required_string(props, "s3_uri")?,
            training_object_ids: required_i64_vec(props, "training_object_ids")?,
            version: required_u32(props, "version")?,
            fitness: optional_f32(props, "fitness")
                .unwrap_or(0.5)
                .clamp(0.0, 1.0),
            created_at_ms: optional_i64(props, "created_at_ms").unwrap_or_default(),
            manifest_version: optional_u32(props, "manifest_version").unwrap_or(1),
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterRef {
    pub adapter: LoraAdapter,
    pub score: f32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterFindRequest {
    pub tenant_id: String,
    pub seed_node_ids: Vec<String>,
    pub k: u32,
    pub base_model_sha: Option<String>,
    pub include_superseded: bool,
    pub min_fitness: Option<f32>,
    pub ppr_damping: f32,
    pub ppr_max_iter: u32,
    pub shared_weight: Option<f32>,
}

impl AdapterFindRequest {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = sanitize_tenant_segment(&self.tenant_id);
        self.seed_node_ids = self
            .seed_node_ids
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect();
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
        self.shared_weight = self
            .shared_weight
            .map(|value| value.clamp(0.0, 1.0))
            .or(Some(DEFAULT_SHARED_WEIGHT));
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterUpsertResult {
    pub adapter: LoraAdapter,
    pub node_id: String,
    pub edge_count: usize,
    pub transaction: GraphTransaction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterFitnessRecordRequest {
    pub adapter_id: String,
    pub source_node_id: String,
    pub value: f32,
    pub weight: f32,
    pub kind: String,
    pub recorded_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterFitnessRecordResult {
    pub adapter: LoraAdapter,
    pub edge_id: String,
    pub effective_fitness: f32,
    pub transaction: GraphTransaction,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterListRequest {
    pub tenant_id: String,
    pub base_model_sha: Option<String>,
    pub min_fitness: Option<f32>,
    pub include_superseded: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdapterSupersedeResult {
    pub old_adapter: LoraAdapter,
    pub new_adapter: LoraAdapter,
    pub edge_id: String,
    pub transaction: GraphTransaction,
}

pub fn normalize_tenant_id(tenant_id: &str) -> String {
    sanitize_tenant_segment(tenant_id)
}

pub fn adapter_node_id(tenant_id: &str, adapter_id: &str) -> String {
    format!(
        "lora_adapter:{}:{}",
        sanitize_tenant_segment(tenant_id),
        adapter_id.trim()
    )
}

pub fn object_node_id(object_pk: i64) -> String {
    format!("object:{object_pk}")
}

pub fn tenant_node_id(tenant_id: &str) -> String {
    format!("tenant:{}", sanitize_tenant_segment(tenant_id))
}

pub fn adapter_vector_designation(dimension: usize) -> VectorDesignation {
    VectorDesignation {
        label: LORA_ADAPTER_LABEL.to_string(),
        property: "embedding".to_string(),
        dimension,
    }
}

pub fn edge_with_adapter_provenance(
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
    properties["source"] = json!(THG_ADAPTER_SOURCE);
    EdgeRecord::new(id, from_id, edge_type, to_id, properties).with_provenance(
        rustyred_thg_core::Provenance {
            source_id: actor.map(str::to_string),
            timestamp: Some(now_ms().to_string()),
            method: Some(THG_ADAPTER_SOURCE.to_string()),
        },
    )
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

pub fn property_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn required_string(properties: &Value, key: &str) -> ThgResult<String> {
    property_string(properties, key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| ThgError::new("invalid_adapter_node", format!("{key} is required")))
}

fn required_u32(properties: &Value, key: &str) -> ThgResult<u32> {
    optional_u32(properties, key)
        .ok_or_else(|| ThgError::new("invalid_adapter_node", format!("{key} is required")))
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

fn required_string_vec(properties: &Value, key: &str) -> ThgResult<Vec<String>> {
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
        .filter(|items| !items.is_empty())
        .ok_or_else(|| ThgError::new("invalid_adapter_node", format!("{key} is required")))
}

fn required_i64_vec(properties: &Value, key: &str) -> ThgResult<Vec<i64>> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.as_i64()
                        .or_else(|| item.as_str().and_then(|raw| raw.parse::<i64>().ok()))
                })
                .collect::<Vec<_>>()
        })
        .ok_or_else(|| ThgError::new("invalid_adapter_node", format!("{key} is required")))
}
