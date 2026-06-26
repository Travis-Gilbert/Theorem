use serde::{Deserialize, Serialize};

use crate::graph_store::{GraphStoreError, GraphStoreResult};
use crate::state::stable_hash;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexKind {
    Identity,
    Ordered,
    Composite,
    Partial,
    Covering,
    FullText,
    Vector,
    SparseVector,
    MultiVector,
    GraphStructural,
    Temporal,
    Spatial,
    ColdSkip,
    LearnedRouting,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexBackend {
    RustyredCore,
    Turbovec,
    Bm25,
    Tantivy,
    H3,
    S2,
    ColdFragment,
    AdvisorShadow,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexScope {
    Global,
    Tenant,
    Project,
    Repo,
    User,
    Session,
    Run,
    Artifact,
    Domain,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexBuildStatus {
    Proposed,
    Building,
    Shadow,
    Active,
    Degraded,
    Retired,
    Failed,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexCreatedBy {
    Manual,
    Migration,
    Advisor,
    System,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct IndexManifest {
    pub id: String,
    pub name: String,
    pub kind: IndexKind,
    pub backend: IndexBackend,
    pub scope: IndexScope,
    pub target_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_edge_type: Option<String>,
    #[serde(default)]
    pub target_properties: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub predicate: Option<String>,
    #[serde(default)]
    pub sort_keys: Vec<String>,
    #[serde(default)]
    pub covering_fields: Vec<String>,
    pub version: u64,
    pub schema_hash: String,
    pub graph_version: u64,
    pub state_hash: String,
    pub build_status: IndexBuildStatus,
    pub created_by: IndexCreatedBy,
    pub hit_count: u64,
    pub miss_count: u64,
    pub avg_latency_ms: f64,
    pub avg_latency_saved_ms: f64,
    pub memory_bytes: u64,
    pub disk_bytes: u64,
    pub write_amplification: f64,
    pub quality_score: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retirement_reason: Option<String>,
}

impl IndexManifest {
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        kind: IndexKind,
        backend: IndexBackend,
        scope: IndexScope,
        target_label: impl Into<String>,
        created_by: IndexCreatedBy,
    ) -> Self {
        let mut manifest = Self {
            id: id.into(),
            name: name.into(),
            kind,
            backend,
            scope,
            target_label: target_label.into(),
            target_edge_type: None,
            target_properties: Vec::new(),
            predicate: None,
            sort_keys: Vec::new(),
            covering_fields: Vec::new(),
            version: 1,
            schema_hash: String::new(),
            graph_version: 0,
            state_hash: String::new(),
            build_status: IndexBuildStatus::Proposed,
            created_by,
            hit_count: 0,
            miss_count: 0,
            avg_latency_ms: 0.0,
            avg_latency_saved_ms: 0.0,
            memory_bytes: 0,
            disk_bytes: 0,
            write_amplification: 0.0,
            quality_score: 0.0,
            retirement_reason: None,
        };
        manifest.refresh_hashes();
        manifest
    }

    pub fn with_target_edge_type(mut self, edge_type: impl Into<String>) -> Self {
        self.target_edge_type = Some(edge_type.into());
        self.refresh_hashes();
        self
    }

    pub fn with_target_properties(
        mut self,
        properties: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.target_properties = properties.into_iter().map(Into::into).collect();
        self.refresh_hashes();
        self
    }

    pub fn with_predicate(mut self, predicate: impl Into<String>) -> Self {
        self.predicate = Some(predicate.into());
        self.refresh_hashes();
        self
    }

    pub fn with_sort_keys(
        mut self,
        sort_keys: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.sort_keys = sort_keys.into_iter().map(Into::into).collect();
        self.refresh_hashes();
        self
    }

    pub fn with_covering_fields(
        mut self,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.covering_fields = fields.into_iter().map(Into::into).collect();
        self.refresh_hashes();
        self
    }

    pub fn validate(&self) -> GraphStoreResult<()> {
        if self.id.trim().is_empty() {
            return Err(GraphStoreError::new(
                "invalid_index_manifest",
                "index manifest id is required",
            ));
        }
        if self.name.trim().is_empty() {
            return Err(GraphStoreError::new(
                "invalid_index_manifest",
                "index manifest name is required",
            ));
        }
        if self.target_label.trim().is_empty()
            && self
                .target_edge_type
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(GraphStoreError::new(
                "invalid_index_manifest",
                "index manifest requires a target label or target edge type",
            ));
        }
        Ok(())
    }

    pub fn mark_building(&mut self) {
        self.build_status = IndexBuildStatus::Building;
        self.refresh_hashes();
    }

    pub fn mark_shadow(&mut self, graph_version: u64) {
        self.build_status = IndexBuildStatus::Shadow;
        self.graph_version = graph_version;
        self.refresh_hashes();
    }

    pub fn activate(&mut self, graph_version: u64) {
        self.build_status = IndexBuildStatus::Active;
        self.graph_version = graph_version;
        self.retirement_reason = None;
        self.refresh_hashes();
    }

    pub fn degrade(&mut self) {
        self.build_status = IndexBuildStatus::Degraded;
        self.refresh_hashes();
    }

    pub fn fail(&mut self, reason: impl Into<String>) {
        self.build_status = IndexBuildStatus::Failed;
        self.retirement_reason = Some(reason.into());
        self.refresh_hashes();
    }

    pub fn retire(&mut self, reason: impl Into<String>) {
        self.build_status = IndexBuildStatus::Retired;
        self.retirement_reason = Some(reason.into());
        self.refresh_hashes();
    }

    pub fn record_hit(&mut self, latency_ms: f64, latency_saved_ms: f64) {
        let observations = self.hit_count.saturating_add(self.miss_count);
        self.avg_latency_ms = rolling_average(self.avg_latency_ms, observations, latency_ms);
        self.avg_latency_saved_ms =
            rolling_average(self.avg_latency_saved_ms, self.hit_count, latency_saved_ms);
        self.hit_count = self.hit_count.saturating_add(1);
        self.refresh_hashes();
    }

    pub fn record_miss(&mut self, latency_ms: f64) {
        let observations = self.hit_count.saturating_add(self.miss_count);
        self.avg_latency_ms = rolling_average(self.avg_latency_ms, observations, latency_ms);
        self.miss_count = self.miss_count.saturating_add(1);
        self.refresh_hashes();
    }

    pub fn refresh_hashes(&mut self) {
        self.schema_hash = self.compute_schema_hash();
        self.state_hash = self.compute_state_hash();
    }

    fn compute_schema_hash(&self) -> String {
        #[derive(Serialize)]
        struct SchemaHashInput<'a> {
            kind: &'a IndexKind,
            backend: &'a IndexBackend,
            scope: &'a IndexScope,
            target_label: &'a str,
            target_edge_type: &'a Option<String>,
            target_properties: &'a [String],
            predicate: &'a Option<String>,
            sort_keys: &'a [String],
            covering_fields: &'a [String],
        }

        stable_hash(SchemaHashInput {
            kind: &self.kind,
            backend: &self.backend,
            scope: &self.scope,
            target_label: &self.target_label,
            target_edge_type: &self.target_edge_type,
            target_properties: &self.target_properties,
            predicate: &self.predicate,
            sort_keys: &self.sort_keys,
            covering_fields: &self.covering_fields,
        })
    }

    fn compute_state_hash(&self) -> String {
        #[derive(Serialize)]
        struct StateHashInput<'a> {
            id: &'a str,
            name: &'a str,
            schema_hash: &'a str,
            version: u64,
            graph_version: u64,
            build_status: &'a IndexBuildStatus,
            created_by: &'a IndexCreatedBy,
            hit_count: u64,
            miss_count: u64,
            avg_latency_ms: f64,
            avg_latency_saved_ms: f64,
            memory_bytes: u64,
            disk_bytes: u64,
            write_amplification: f64,
            quality_score: f64,
            retirement_reason: &'a Option<String>,
        }

        stable_hash(StateHashInput {
            id: &self.id,
            name: &self.name,
            schema_hash: &self.schema_hash,
            version: self.version,
            graph_version: self.graph_version,
            build_status: &self.build_status,
            created_by: &self.created_by,
            hit_count: self.hit_count,
            miss_count: self.miss_count,
            avg_latency_ms: self.avg_latency_ms,
            avg_latency_saved_ms: self.avg_latency_saved_ms,
            memory_bytes: self.memory_bytes,
            disk_bytes: self.disk_bytes,
            write_amplification: self.write_amplification,
            quality_score: self.quality_score,
            retirement_reason: &self.retirement_reason,
        })
    }
}

fn rolling_average(current_avg: f64, current_count: u64, next: f64) -> f64 {
    if current_count == 0 {
        next
    } else {
        ((current_avg * current_count as f64) + next) / (current_count.saturating_add(1) as f64)
    }
}
