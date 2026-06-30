use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, Mutex, RwLock,
};
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_affordances::{theorem_grpc_timeout_ms, AffordanceGraphStore};
use rustyred_thg_core::store::RedisThgStore;
use rustyred_thg_core::{
    make_fulltext_backend, make_spatial_backend, EdgeRecord, EpistemicType, FullTextBackend,
    FullTextDesignation, GraphMutation, GraphMutationBatch, GraphRebuildReport, GraphSnapshot,
    GraphStats, GraphStore, GraphStoreError, GraphStoreResult, GraphTransaction, GraphWriteResult,
    HookDispatcher, HookDispatcherConfig, HookRegistration, HookStoreAccess, HybridScoringConfig,
    InMemoryGraphStore, MemoryDocumentQuery, NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
    PluginRegistry, RedCoreGraphStore, RedCoreOptions, RedisGraphStore, SpatialBackend,
    SpatialDesignation, VectorDesignation, VerifyReport,
};
use rustyred_thg_datawave_harness::{DatawaveIngestPlugin, INGEST_CAPABILITY_PACK};
use rustyred_thg_mcp::{
    job_archive_to_store, job_list_from_store, job_note_to_store, job_submit_to_store,
    redcore_reverse_engineer_compose_payload, AppAffordanceInvocation, HandoffDispatch, McpError,
    McpGraphBackend, McpGraphProvider, McpServerConfig,
};
use rustyred_web::{
    configured_search_providers_from_env, FetchCascade, FetchCascadeOptions, LiveFetchOptions,
    SearchProvider,
};
use serde_json::{json, Value};
use theorem_dispatch::{priority_from_harness, DispatchQueue, Job as DispatchJob};
use theorem_harness_core::{
    GroundedClaim, HeadInvocationError, Job as HarnessJob, JobSubmission, TransitionInput,
    TransitionResult,
};
use theorem_harness_runtime::{
    append_transition_from_store, load_events, load_run, ComposedAgentRuntimeError,
    HarnessRuntimeError, JobNoteInput, ProviderHeadInvoker,
};

use crate::browser_pool::{BrowserLiveSessionRecord, LiveBrowserPool, RemoteBrowserPool};
use crate::config::{Config, StorageMode};
use crate::graph_cache::GraphCacheTenant;
use crate::observability::Observability;
use crate::payload_backend::payload_backend_from_env;
use crate::tenant_router::{tenant_data_dir, tenant_key_segment, TenantId};
use crate::ttl_sweep::TtlSweepState;

const GRAPH_TRANSACTION_TTL_MS: u64 = 5 * 60 * 1000;
const DISPATCH_DATABASE_URL_ENV: &str = "THEOREM_DISPATCH_DATABASE_URL";
const MEMORY_FULLTEXT_PROPERTY: &str = "search_text";
const MEMORY_FULLTEXT_LABELS: [&str; 3] = ["MemoryAtom", "MemoryDocument", "MemoryNode"];

#[derive(Clone, Debug)]
struct GraphTransactionContext {
    tenant_id: String,
    snapshot_version: u64,
    created_at_ms: u64,
    mutations: GraphMutationBatch,
}

/// Per-tenant Phase 8 spatial indexes. Keyed by tenant_id then by
/// (label, lat_property, lon_property).
type SpatialIndexes = BTreeMap<String, BTreeMap<(String, String, String), Box<dyn SpatialBackend>>>;

/// Per-tenant Phase 5 full-text indexes. Keyed by tenant_id then by
/// (label, property).
type FullTextIndexes = BTreeMap<String, BTreeMap<(String, String), Box<dyn FullTextBackend>>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TenantEngineState {
    Cold,
    Warm,
    Hot,
}

impl TenantEngineState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::Warm => "warm",
            Self::Hot => "hot",
        }
    }

    pub fn gauge_value(self) -> u8 {
        match self {
            Self::Cold => 0,
            Self::Warm => 1,
            Self::Hot => 2,
        }
    }
}

#[derive(Clone, Debug)]
struct TenantEngineMeta {
    tenant_id: String,
    state: TenantEngineState,
    data_dir: Option<String>,
    last_accessed_ms: u64,
    opened_at_ms: Option<u64>,
}

impl TenantEngineMeta {
    fn new(
        tenant_id: impl Into<String>,
        state: TenantEngineState,
        data_dir: Option<String>,
        now_ms: u64,
    ) -> Self {
        let opened_at_ms =
            matches!(state, TenantEngineState::Warm | TenantEngineState::Hot).then_some(now_ms);
        Self {
            tenant_id: tenant_id.into(),
            state,
            data_dir,
            last_accessed_ms: now_ms,
            opened_at_ms,
        }
    }

    fn touch_hot(&mut self, now_ms: u64, data_dir: Option<String>) {
        self.state = TenantEngineState::Hot;
        self.last_accessed_ms = now_ms;
        self.opened_at_ms.get_or_insert(now_ms);
        if data_dir.is_some() {
            self.data_dir = data_dir;
        }
    }

    fn mark_warm(&mut self) {
        self.state = TenantEngineState::Warm;
    }

    fn mark_cold(&mut self) {
        self.state = TenantEngineState::Cold;
        self.opened_at_ms = None;
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct TenantEngineReport {
    pub tenant: String,
    pub state: TenantEngineState,
    pub state_value: u8,
    pub last_accessed_ms: u64,
    pub idle_ms: u64,
    pub opened_at_ms: Option<u64>,
    pub resident_memory_bytes: usize,
    pub data_dir: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct TenantLifecycleSweepReport {
    pub cooled_to_warm: usize,
    pub cooled_to_cold: usize,
    pub warm_pool_size: usize,
}

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub observability: Observability,
    /// TTL background sweep counters + cancel flag. Cloned across the
    /// spawned sweep task and the diagnostics handler. Always present
    /// (even when no sweep task is spawned, e.g. in tests that don't
    /// exercise the loop) so accessor methods can read counters
    /// without an Option check.
    pub ttl_sweep: Arc<TtlSweepState>,
    redcore_stores: Arc<Mutex<BTreeMap<String, Arc<RedCoreTenantExecutor>>>>,
    tenant_engines: Arc<Mutex<BTreeMap<String, TenantEngineMeta>>>,
    graph_caches: Arc<Mutex<BTreeMap<String, Arc<GraphCacheTenant>>>>,
    graph_transactions: Arc<Mutex<BTreeMap<String, GraphTransactionContext>>>,
    live_fetch_cascade: Arc<FetchCascade>,
    live_browser_pool: Arc<RwLock<Option<Arc<dyn LiveBrowserPool>>>>,
    live_browser_sessions: Arc<Mutex<BTreeMap<String, BrowserLiveSessionRecord>>>,
    search_providers: Arc<RwLock<Vec<Arc<dyn SearchProvider>>>>,
    next_graph_txn_id: Arc<AtomicU64>,
    spatial_indexes: Arc<Mutex<SpatialIndexes>>,
    fulltext_indexes: Arc<Mutex<FullTextIndexes>>,
    /// Bao-style steered-optimizer observations: per query shape and
    /// enumerated candidate, measured execution cost units. Read by the
    /// Cypher query surface before choosing among native plan candidates.
    pub plan_steering: Arc<crate::cypher::planner::PlanSteeringState>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        Self::new_with_search_providers(config, configured_search_providers_from_env())
    }

    pub fn new_with_search_providers(
        config: Config,
        search_providers: Vec<Arc<dyn SearchProvider>>,
    ) -> Self {
        let observability = Observability::new_with_log(
            config.slow_query_threshold_nanos,
            config.slow_query_capacity,
            config.slow_query_log.clone(),
        );
        let live_fetch_options = LiveFetchOptions::default();
        let live_fetch_cascade = FetchCascade::new(FetchCascadeOptions {
            user_agent: live_fetch_options.user_agent,
            timeout_seconds: live_fetch_options.timeout_seconds,
            allow_impersonate: live_fetch_options.allow_impersonate,
            rendered_endpoint: live_fetch_options.rendered_endpoint,
            respect_robots_for_escalation: live_fetch_options.respect_robots,
        })
        .expect("default live fetch cascade options must build");
        let live_browser_pool =
            RemoteBrowserPool::from_env().map(|pool| Arc::new(pool) as Arc<dyn LiveBrowserPool>);
        Self {
            config: Arc::new(config),
            observability,
            ttl_sweep: Arc::new(TtlSweepState::new()),
            redcore_stores: Arc::new(Mutex::new(BTreeMap::new())),
            tenant_engines: Arc::new(Mutex::new(BTreeMap::new())),
            graph_caches: Arc::new(Mutex::new(BTreeMap::new())),
            graph_transactions: Arc::new(Mutex::new(BTreeMap::new())),
            live_fetch_cascade: Arc::new(live_fetch_cascade),
            live_browser_pool: Arc::new(RwLock::new(live_browser_pool)),
            live_browser_sessions: Arc::new(Mutex::new(BTreeMap::new())),
            search_providers: Arc::new(RwLock::new(search_providers)),
            next_graph_txn_id: Arc::new(AtomicU64::new(1)),
            spatial_indexes: Arc::new(Mutex::new(BTreeMap::new())),
            fulltext_indexes: Arc::new(Mutex::new(BTreeMap::new())),
            plan_steering: Arc::new(crate::cypher::planner::PlanSteeringState::default()),
        }
    }

    pub fn live_browser_pool(&self) -> Option<Arc<dyn LiveBrowserPool>> {
        self.live_browser_pool
            .read()
            .ok()
            .and_then(|pool| pool.as_ref().cloned())
    }

    pub fn set_live_browser_pool(&self, pool: Option<Arc<dyn LiveBrowserPool>>) {
        if let Ok(mut live_browser_pool) = self.live_browser_pool.write() {
            *live_browser_pool = pool;
        }
    }

    pub fn live_browser_session(&self, run_id: &str) -> Option<BrowserLiveSessionRecord> {
        self.live_browser_sessions
            .lock()
            .ok()
            .and_then(|sessions| sessions.get(run_id).cloned())
    }

    pub fn upsert_live_browser_session(&self, session: BrowserLiveSessionRecord) {
        if let Ok(mut sessions) = self.live_browser_sessions.lock() {
            sessions.insert(session.run_id.clone(), session);
        }
    }

    // ===== Phase 5: full-text designation + indexing =====

    pub fn designate_fulltext_property(
        &self,
        tenant_id: &str,
        label: &str,
        property: &str,
    ) -> Result<(), StoreAccessError> {
        let store = self.tenant_graph_store(tenant_id)?;
        let designation = FullTextDesignation {
            label: label.to_string(),
            property: property.to_string(),
        };
        // §P5-A pa5.3: env switch resolves at backend construction time so
        // each `/fulltext/designate` call honors the current
        // RUSTY_RED_FULLTEXT_BACKEND setting. Default is the hand-rolled BM25;
        // `tantivy` selects the tantivy-backed impl when the feature is built.
        let mut index = make_fulltext_backend(designation)
            .map_err(|err| StoreAccessError::unsupported(err.message()))?;
        // Bulk-index any existing nodes for the label.
        let nodes = store
            .query_nodes(NodeQuery {
                label: Some(label.to_string()),
                ..NodeQuery::default()
            })
            .map_err(StoreAccessError::from)?;
        for node in nodes {
            if let Some(text) = node.properties.get(property).and_then(|v| v.as_str()) {
                index.upsert(&node.id, text);
            }
        }
        let mut indexes = self
            .fulltext_indexes
            .lock()
            .map_err(|_| StoreAccessError::internal("fulltext index lock poisoned"))?;
        indexes
            .entry(tenant_id.to_string())
            .or_default()
            .insert((label.to_string(), property.to_string()), index);
        Ok(())
    }

    pub fn maybe_index_node_fulltext(&self, tenant_id: &str, node: &NodeRecord) {
        let Ok(mut indexes) = self.fulltext_indexes.lock() else {
            return;
        };
        let Some(tenant_map) = indexes.get_mut(tenant_id) else {
            return;
        };
        for ((label, property), index) in tenant_map.iter_mut() {
            if !node.labels.iter().any(|l| l == label) {
                continue;
            }
            if let Some(text) = node.properties.get(property).and_then(|v| v.as_str()) {
                index.upsert(&node.id, text);
            } else {
                index.remove(&node.id);
            }
        }
    }

    pub fn fulltext_search(
        &self,
        tenant_id: &str,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> Result<Vec<(String, f32)>, StoreAccessError> {
        let indexes = self
            .fulltext_indexes
            .lock()
            .map_err(|_| StoreAccessError::internal("fulltext index lock poisoned"))?;
        let Some(tenant_map) = indexes.get(tenant_id) else {
            return Err(StoreAccessError::unsupported(
                "no fulltext designations for this tenant",
            ));
        };
        // If label given, search just that (label, property). Otherwise union
        // over every label that has this property indexed.
        let mut combined: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let mut matched_designation = false;
        for ((idx_label, idx_property), index) in tenant_map.iter() {
            if idx_property != property {
                continue;
            }
            if let Some(label_filter) = label {
                if idx_label != label_filter {
                    continue;
                }
            }
            matched_designation = true;
            for (id, score) in index.search(query, k) {
                let slot = combined.entry(id).or_insert(0.0);
                if score > *slot {
                    *slot = score;
                }
            }
        }
        if !matched_designation {
            return Err(StoreAccessError::unsupported(
                "no matching fulltext designation; call /fulltext/designate first",
            ));
        }
        let mut entries: Vec<(String, f32)> = combined.into_iter().collect();
        entries.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        entries.truncate(k);
        Ok(entries)
    }

    pub fn has_fulltext_designation(
        &self,
        tenant_id: &str,
        labels: &[&str],
        property: &str,
    ) -> bool {
        let Ok(indexes) = self.fulltext_indexes.lock() else {
            return false;
        };
        indexes.get(tenant_id).is_some_and(|tenant_map| {
            labels
                .iter()
                .any(|label| tenant_map.contains_key(&((*label).to_string(), property.to_string())))
        })
    }

    pub fn ensure_memory_fulltext_index(
        &self,
        tenant_id: &str,
        label: &str,
    ) -> Result<(), StoreAccessError> {
        if !is_memory_fulltext_label(label) {
            return Ok(());
        }
        if !self.has_fulltext_designation(tenant_id, &[label], MEMORY_FULLTEXT_PROPERTY) {
            self.designate_fulltext_property(tenant_id, label, MEMORY_FULLTEXT_PROPERTY)?;
            tracing::info!(
                tenant_id = %tenant_id,
                label,
                property = MEMORY_FULLTEXT_PROPERTY,
                "memory fulltext index warmed from persisted graph store"
            );
        }
        Ok(())
    }

    // ===== Phase 8: spatial designation + indexing =====

    pub fn designate_spatial_property(
        &self,
        tenant_id: &str,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        resolution: u8,
    ) -> Result<(), StoreAccessError> {
        if !(0..=15).contains(&resolution) {
            return Err(StoreAccessError::unsupported(format!(
                "spatial resolution {resolution} is outside 0..=15"
            )));
        }
        let store = self.tenant_graph_store(tenant_id)?;
        let designation = SpatialDesignation {
            label: label.to_string(),
            lat_property: lat_property.to_string(),
            lon_property: lon_property.to_string(),
            resolution,
        };
        // §P8-A pa8.3: env switch resolves at backend construction time so
        // each `/spatial/designate` call honors the current
        // RUSTY_RED_SPATIAL_BACKEND setting. Default is the H3 impl; `s2`
        // selects the S2-cell impl when the feature is built.
        let mut index: Box<dyn SpatialBackend> = make_spatial_backend(designation.clone())
            .map_err(|err| StoreAccessError::unsupported(err.message()))?;
        // Bulk-index any existing nodes for the label.
        let nodes = store
            .query_nodes(NodeQuery {
                label: Some(label.to_string()),
                ..NodeQuery::default()
            })
            .map_err(StoreAccessError::from)?;
        for node in nodes {
            if let (Some(lat), Some(lon)) = (
                node.properties.get(lat_property).and_then(|v| v.as_f64()),
                node.properties.get(lon_property).and_then(|v| v.as_f64()),
            ) {
                let _ = SpatialBackend::upsert(index.as_mut(), &node.id, lat, lon);
            }
        }
        let mut indexes = self
            .spatial_indexes
            .lock()
            .map_err(|_| StoreAccessError::internal("spatial index lock poisoned"))?;
        indexes.entry(tenant_id.to_string()).or_default().insert(
            (
                label.to_string(),
                lat_property.to_string(),
                lon_property.to_string(),
            ),
            index,
        );
        Ok(())
    }

    /// Index a node into any designations for its label whose lat+lon
    /// properties are present. Called on the write path.
    pub fn maybe_index_node_spatially(&self, tenant_id: &str, node: &NodeRecord) {
        let Ok(mut indexes) = self.spatial_indexes.lock() else {
            return;
        };
        let Some(tenant_map) = indexes.get_mut(tenant_id) else {
            return;
        };
        for ((label, lat_prop, lon_prop), index) in tenant_map.iter_mut() {
            if !node.labels.iter().any(|l| l == label) {
                continue;
            }
            let lat = node.properties.get(lat_prop).and_then(|v| v.as_f64());
            let lon = node.properties.get(lon_prop).and_then(|v| v.as_f64());
            if let (Some(lat), Some(lon)) = (lat, lon) {
                let _ = SpatialBackend::upsert(index.as_mut(), &node.id, lat, lon);
            }
        }
    }

    pub fn spatial_radius_search(
        &self,
        tenant_id: &str,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        lat: f64,
        lon: f64,
        radius_km: f64,
    ) -> Result<Vec<String>, StoreAccessError> {
        let indexes = self
            .spatial_indexes
            .lock()
            .map_err(|_| StoreAccessError::internal("spatial index lock poisoned"))?;
        let key = (
            label.to_string(),
            lat_property.to_string(),
            lon_property.to_string(),
        );
        let Some(tenant_map) = indexes.get(tenant_id) else {
            return Err(StoreAccessError::unsupported(
                "no spatial designations for this tenant",
            ));
        };
        let Some(index) = tenant_map.get(&key) else {
            return Err(StoreAccessError::unsupported(
                "spatial designation not found; call /spatial/designate first",
            ));
        };
        index
            .radius_search(lat, lon, radius_km)
            .map_err(|e| StoreAccessError::unsupported(e.message()))
    }

    pub fn spatial_bbox_search(
        &self,
        tenant_id: &str,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        min_lat: f64,
        min_lon: f64,
        max_lat: f64,
        max_lon: f64,
    ) -> Result<Vec<String>, StoreAccessError> {
        let indexes = self
            .spatial_indexes
            .lock()
            .map_err(|_| StoreAccessError::internal("spatial index lock poisoned"))?;
        let key = (
            label.to_string(),
            lat_property.to_string(),
            lon_property.to_string(),
        );
        let Some(tenant_map) = indexes.get(tenant_id) else {
            return Err(StoreAccessError::unsupported(
                "no spatial designations for this tenant",
            ));
        };
        let Some(index) = tenant_map.get(&key) else {
            return Err(StoreAccessError::unsupported(
                "spatial designation not found; call /spatial/designate first",
            ));
        };
        Ok(index.bbox_search(min_lat, min_lon, max_lat, max_lon))
    }

    pub fn begin_graph_transaction(&self, tenant_id: &str) -> Result<String, StoreAccessError> {
        self.purge_expired_graph_transactions()?;
        let store = match self.tenant_graph_store(tenant_id)? {
            TenantGraphStore::RedCore(store) => store,
            TenantGraphStore::Redis(_) => {
                return Err(StoreAccessError::unsupported(
                    "graph transactions are supported for RedCore-backed tenants only",
                ));
            }
        };
        let snapshot_version = store.stats().map_err(StoreAccessError::from)?.version;
        let tx_id = format!(
            "tx-{}",
            self.next_graph_txn_id.fetch_add(1, Ordering::Relaxed)
        );
        let context = GraphTransactionContext {
            tenant_id: tenant_id.to_string(),
            snapshot_version,
            created_at_ms: now_millis(),
            mutations: GraphMutationBatch::default(),
        };
        let mut transactions = self
            .graph_transactions
            .lock()
            .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
        transactions.insert(tx_id.clone(), context);
        Ok(tx_id)
    }

    pub fn append_graph_transaction_mutations(
        &self,
        tenant_id: &str,
        tx_id: &str,
        batch: GraphMutationBatch,
    ) -> Result<usize, StoreAccessError> {
        self.purge_expired_graph_transactions()?;
        if batch.mutations.is_empty() {
            return Err(StoreAccessError::from(GraphStoreError::new(
                "empty_graph_transaction",
                "transaction batch must include at least one mutation",
            )));
        }
        let mut transactions = self
            .graph_transactions
            .lock()
            .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
        let Some(context) = transactions.get_mut(tx_id) else {
            return Err(StoreAccessError::unsupported("graph transaction not found"));
        };
        if context.tenant_id != tenant_id {
            return Err(StoreAccessError::unsupported(
                "graph transaction tenant mismatch",
            ));
        }
        context
            .mutations
            .mutations
            .extend(batch.mutations.into_iter());
        Ok(context.mutations.mutations.len())
    }

    pub fn commit_graph_transaction(
        &self,
        tenant_id: &str,
        tx_id: &str,
    ) -> Result<GraphTransaction, StoreAccessError> {
        self.purge_expired_graph_transactions()?;
        let store = match self.tenant_graph_store(tenant_id)? {
            TenantGraphStore::RedCore(store) => store,
            TenantGraphStore::Redis(_) => {
                return Err(StoreAccessError::unsupported(
                    "graph transactions are supported for RedCore-backed tenants only",
                ));
            }
        };
        let context = {
            let transactions = self
                .graph_transactions
                .lock()
                .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
            let context = transactions.get(tx_id).ok_or_else(|| {
                StoreAccessError::unsupported("graph transaction not found or already committed")
            })?;
            if context.tenant_id != tenant_id {
                return Err(StoreAccessError::unsupported(
                    "graph transaction tenant mismatch",
                ));
            }
            context.clone()
        };
        if context.mutations.mutations.is_empty() {
            return Err(StoreAccessError::from(GraphStoreError::new(
                "empty_graph_transaction",
                "graph transactions must include at least one mutation",
            )));
        }
        let current_version = store.stats().map_err(StoreAccessError::from)?.version;
        if current_version != context.snapshot_version {
            return Err(StoreAccessError::unsupported(
                "graph transaction snapshot conflict",
            ));
        }
        let transaction = store
            .commit_batch(context.mutations)
            .map_err(StoreAccessError::from)?;
        let mut transactions = self
            .graph_transactions
            .lock()
            .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
        transactions.remove(tx_id);
        Ok(transaction)
    }

    pub fn rollback_graph_transaction(
        &self,
        tenant_id: &str,
        tx_id: &str,
    ) -> Result<(), StoreAccessError> {
        self.purge_expired_graph_transactions()?;
        let mut transactions = self
            .graph_transactions
            .lock()
            .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
        let Some(context) = transactions.get(tx_id) else {
            return Err(StoreAccessError::unsupported("graph transaction not found"));
        };
        if context.tenant_id != tenant_id {
            return Err(StoreAccessError::unsupported(
                "graph transaction tenant mismatch",
            ));
        }
        transactions.remove(tx_id);
        Ok(())
    }

    fn purge_expired_graph_transactions(&self) -> Result<(), StoreAccessError> {
        let now_ms = now_millis();
        self.purge_expired_graph_transactions_at(now_ms)
    }

    fn purge_expired_graph_transactions_at(&self, now_ms: u64) -> Result<(), StoreAccessError> {
        let mut transactions = self
            .graph_transactions
            .lock()
            .map_err(|_| StoreAccessError::internal("graph transaction store lock poisoned"))?;
        transactions.retain(|_, context| {
            now_ms.saturating_sub(context.created_at_ms) <= GRAPH_TRANSACTION_TTL_MS
        });
        Ok(())
    }

    pub fn tenant_store(&self, tenant_id: &str) -> Result<RedisThgStore, StoreAccessError> {
        self.config.validate().map_err(StoreAccessError::internal)?;
        if self.config.storage_mode != StorageMode::Redis {
            return Err(StoreAccessError::unsupported(
                "run/context state commands are available only in RUSTY_RED_MODE=redis in this slice",
            ));
        }
        RedisThgStore::new(&self.config.redis_url, self.tenant_state_key(tenant_id))
            .map_err(StoreAccessError::from)
    }

    pub fn tenant_state_key(&self, tenant_id: &str) -> String {
        match TenantId::new(tenant_id) {
            Ok(tenant_id) => format!(
                "{}:{}:state:v1",
                self.config.redis_key_prefix,
                tenant_key_segment(&tenant_id)
            ),
            Err(_) => format!("{}:tenant-invalid:state:v1", self.config.redis_key_prefix),
        }
    }

    pub fn tenant_graph_store(
        &self,
        tenant_id: &str,
    ) -> Result<TenantGraphStore, StoreAccessError> {
        self.config.validate().map_err(StoreAccessError::internal)?;
        match self.config.storage_mode {
            StorageMode::Embedded => {
                self.sweep_idle_tenant_engines_at(now_millis())?;
                Ok(TenantGraphStore::RedCore(
                    self.redcore_store_for_tenant(tenant_id)?,
                ))
            }
            StorageMode::Memory => Ok(TenantGraphStore::RedCore(
                self.memory_store_for_tenant(tenant_id)?,
            )),
            StorageMode::Redis => RedisGraphStore::tenant(
                &self.config.redis_url,
                &self.config.redis_key_prefix,
                tenant_id,
            )
            .map(TenantGraphStore::Redis)
            .map_err(StoreAccessError::from),
        }
    }

    pub fn store_ready(&self) -> Result<ReadyReport, StoreAccessError> {
        self.config.validate().map_err(StoreAccessError::internal)?;
        match self.config.storage_mode {
            StorageMode::Embedded => {
                let data_dir = PathBuf::from(&self.config.data_dir);
                RedCoreGraphStore::readiness_check(
                    &data_dir,
                    self.config.durability,
                    self.config.strict_acid,
                )
                .map_err(StoreAccessError::from)?;
                Ok(ReadyReport {
                    mode: "embedded".to_string(),
                    store: "ready".to_string(),
                    durability: self.config.durability.as_str().to_string(),
                    strict_acid: self.config.strict_acid,
                    require_volume: self.config.require_volume,
                    data_dir: Some(data_dir.display().to_string()),
                })
            }
            StorageMode::Memory => Ok(ReadyReport {
                mode: "memory".to_string(),
                store: "ready".to_string(),
                durability: "none".to_string(),
                strict_acid: false,
                require_volume: false,
                data_dir: None,
            }),
            StorageMode::Redis => {
                let key = format!("{}:__ready__:state:v1", self.config.redis_key_prefix);
                RedisThgStore::new(&self.config.redis_url, key)
                    .and_then(|store| store.ping())
                    .map_err(StoreAccessError::from)?;
                Ok(ReadyReport {
                    mode: "redis".to_string(),
                    store: "ready".to_string(),
                    durability: "redis".to_string(),
                    strict_acid: false,
                    require_volume: false,
                    data_dir: None,
                })
            }
        }
    }

    pub fn mcp_config(&self) -> McpServerConfig {
        McpServerConfig {
            name: self.config.service_name.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            default_tenant: self.config.mcp_default_tenant.clone(),
            read_only: self.config.mcp_read_only,
            allow_admin: self.config.mcp_allow_admin,
            graphql_default_surface: self.config.mcp_graphql_default_surface,
            tool_result_budget_bytes: rustyred_thg_mcp::DEFAULT_TOOL_RESULT_BUDGET_BYTES,
            tool_result_family_budgets: Default::default(),
        }
    }

    pub fn live_fetch_cascade(&self) -> Arc<FetchCascade> {
        Arc::clone(&self.live_fetch_cascade)
    }

    pub fn search_providers(&self, allowlist: &[String]) -> Vec<Arc<dyn SearchProvider>> {
        let Ok(providers) = self.search_providers.read() else {
            return Vec::new();
        };
        if allowlist.is_empty() {
            return providers.iter().cloned().collect();
        }
        let allowed: BTreeSet<String> = allowlist
            .iter()
            .map(|provider| provider.trim().to_ascii_lowercase())
            .filter(|provider| !provider.is_empty())
            .collect();
        providers
            .iter()
            .filter(|provider| allowed.contains(&provider.name().to_ascii_lowercase()))
            .cloned()
            .collect()
    }

    pub fn sweep_idle_tenant_engines(
        &self,
    ) -> Result<TenantLifecycleSweepReport, StoreAccessError> {
        self.sweep_idle_tenant_engines_at(now_millis())
    }

    pub fn sweep_idle_tenant_engines_at(
        &self,
        now_ms: u64,
    ) -> Result<TenantLifecycleSweepReport, StoreAccessError> {
        if self.config.storage_mode != StorageMode::Embedded {
            return Ok(TenantLifecycleSweepReport {
                warm_pool_size: self.config.tenant_warm_pool_size,
                ..TenantLifecycleSweepReport::default()
            });
        }

        let mut stores = self
            .redcore_stores
            .lock()
            .map_err(|_| StoreAccessError::internal("redcore tenant map lock poisoned"))?;
        let mut engines = self
            .tenant_engines
            .lock()
            .map_err(|_| StoreAccessError::internal("tenant engine state lock poisoned"))?;

        let mut idle = engines
            .iter()
            .filter(|(tenant_key, meta)| {
                stores.contains_key(*tenant_key)
                    && now_ms.saturating_sub(meta.last_accessed_ms) >= self.config.tenant_idle_ms
            })
            .map(|(tenant_key, meta)| (tenant_key.clone(), meta.last_accessed_ms))
            .collect::<Vec<_>>();
        idle.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let keep_warm = idle
            .iter()
            .take(self.config.tenant_warm_pool_size)
            .map(|(tenant_key, _)| tenant_key.clone())
            .collect::<BTreeSet<_>>();

        let mut report = TenantLifecycleSweepReport {
            warm_pool_size: self.config.tenant_warm_pool_size,
            ..TenantLifecycleSweepReport::default()
        };
        let mut cold_keys = Vec::new();
        for (tenant_key, _) in idle {
            if keep_warm.contains(&tenant_key) {
                if let Some(store) = stores.get(&tenant_key) {
                    store.clear_hot_caches();
                }
                if let Some(meta) = engines.get_mut(&tenant_key) {
                    meta.mark_warm();
                }
                report.cooled_to_warm += 1;
            } else {
                if let Some(store) = stores.get(&tenant_key) {
                    store.clear_hot_caches();
                }
                stores.remove(&tenant_key);
                cold_keys.push(tenant_key);
                report.cooled_to_cold += 1;
            }
        }
        drop(stores);

        if !cold_keys.is_empty() {
            let mut caches = self
                .graph_caches
                .lock()
                .map_err(|_| StoreAccessError::internal("graph cache tenant map lock poisoned"))?;
            for tenant_key in &cold_keys {
                caches.remove(tenant_key);
            }
        }

        for tenant_key in cold_keys {
            if let Some(meta) = engines.get_mut(&tenant_key) {
                meta.mark_cold();
            }
        }
        Ok(report)
    }

    pub fn tenant_engine_reports(&self) -> Result<Vec<TenantEngineReport>, StoreAccessError> {
        self.tenant_engine_reports_at(now_millis())
    }

    pub fn tenant_engine_reports_at(
        &self,
        now_ms: u64,
    ) -> Result<Vec<TenantEngineReport>, StoreAccessError> {
        let stores = self
            .redcore_stores
            .lock()
            .map_err(|_| StoreAccessError::internal("redcore tenant map lock poisoned"))?
            .iter()
            .map(|(tenant, store)| (tenant.clone(), store.clone()))
            .collect::<BTreeMap<_, _>>();
        let engines = self
            .tenant_engines
            .lock()
            .map_err(|_| StoreAccessError::internal("tenant engine state lock poisoned"))?;
        Ok(engines
            .iter()
            .map(|(tenant_key, meta)| {
                let resident_memory_bytes = match meta.state {
                    TenantEngineState::Cold => 0,
                    TenantEngineState::Warm | TenantEngineState::Hot => stores
                        .get(tenant_key)
                        .and_then(|store| store.cheap_stats().ok())
                        .map(|stats| stats.memory_bytes)
                        .unwrap_or(0),
                };
                TenantEngineReport {
                    tenant: meta.tenant_id.clone(),
                    state: meta.state,
                    state_value: meta.state.gauge_value(),
                    last_accessed_ms: meta.last_accessed_ms,
                    idle_ms: now_ms.saturating_sub(meta.last_accessed_ms),
                    opened_at_ms: meta.opened_at_ms,
                    resident_memory_bytes,
                    data_dir: meta.data_dir.clone(),
                }
            })
            .collect())
    }

    pub fn tenant_engine_state(
        &self,
        tenant_id: &str,
    ) -> Result<Option<TenantEngineState>, StoreAccessError> {
        let tenant_id = TenantId::new(tenant_id)
            .map_err(|error| StoreAccessError::internal(format!("invalid tenant id: {error}")))?;
        let safe_tenant = tenant_key_segment(&tenant_id);
        let engines = self
            .tenant_engines
            .lock()
            .map_err(|_| StoreAccessError::internal("tenant engine state lock poisoned"))?;
        Ok(engines.get(&safe_tenant).map(|meta| meta.state))
    }

    pub fn render_tenant_engine_prometheus(&self) -> Result<String, StoreAccessError> {
        let reports = self.tenant_engine_reports()?;
        let mut out = String::new();
        out.push_str(
            "# HELP rustyred_thg_tenant_engine_state Tenant engine state: 0=cold, 1=warm, 2=hot\n",
        );
        out.push_str("# TYPE rustyred_thg_tenant_engine_state gauge\n");
        out.push_str("# HELP rustyred_thg_tenant_engine_resident_bytes Estimated resident bytes for this tenant engine\n");
        out.push_str("# TYPE rustyred_thg_tenant_engine_resident_bytes gauge\n");
        for report in reports {
            let tenant = prometheus_label_value(&report.tenant);
            out.push_str(&format!(
                "rustyred_thg_tenant_engine_state{{tenant=\"{tenant}\",state=\"{}\"}} {}\n",
                report.state.as_str(),
                report.state_value
            ));
            out.push_str(&format!(
                "rustyred_thg_tenant_engine_resident_bytes{{tenant=\"{tenant}\",state=\"{}\"}} {}\n",
                report.state.as_str(),
                report.resident_memory_bytes
            ));
        }
        Ok(out)
    }

    pub fn tenant_graph_cache(
        &self,
        tenant_id: &str,
    ) -> Result<Arc<GraphCacheTenant>, StoreAccessError> {
        let tenant_id = TenantId::new(tenant_id)
            .map_err(|error| StoreAccessError::internal(format!("invalid tenant id: {error}")))?;
        let safe_tenant = tenant_key_segment(&tenant_id);
        let mut caches = self
            .graph_caches
            .lock()
            .map_err(|_| StoreAccessError::internal("graph cache tenant map lock poisoned"))?;
        if let Some(cache) = caches.get(&safe_tenant) {
            return Ok(cache.clone());
        }
        let cache = Arc::new(GraphCacheTenant::default());
        caches.insert(safe_tenant, cache.clone());
        Ok(cache)
    }

    fn redcore_store_for_tenant(
        &self,
        tenant_id: &str,
    ) -> Result<Arc<RedCoreTenantExecutor>, StoreAccessError> {
        let tenant_id = TenantId::new(tenant_id)
            .map_err(|error| StoreAccessError::internal(format!("invalid tenant id: {error}")))?;
        let safe_tenant = tenant_key_segment(&tenant_id);
        let data_dir = tenant_data_dir(&self.config.data_dir, &tenant_id);
        let data_dir_label = Some(data_dir.display().to_string());
        let mut stores = self
            .redcore_stores
            .lock()
            .map_err(|_| StoreAccessError::internal("redcore tenant map lock poisoned"))?;
        if let Some(store) = stores.get(&safe_tenant) {
            self.touch_tenant_engine(
                &safe_tenant,
                tenant_id.as_str(),
                data_dir_label,
                now_millis(),
            )?;
            return Ok(store.clone());
        }
        let tenant_config = self.config.tenant_config(tenant_id.as_str());
        let options = RedCoreOptions {
            durability: tenant_config.durability,
            snapshot_interval_writes: tenant_config.snapshot_interval_writes,
            strict_acid: tenant_config.strict_acid,
        };
        let mut graph_store = RedCoreGraphStore::open(data_dir, options)?;
        configure_payload_backend(&mut graph_store)?;
        let store = Arc::new(RedCoreTenantExecutor::new(
            graph_store,
            tenant_config.tenant_memory_quota_bytes,
        )?);
        if let Err(err) = store.enable_graph_hooks(
            tenant_hook_registrations(tenant_id.as_str()),
            tenant_id.as_str(),
        ) {
            eprintln!(
                "[theorem] enable graph hooks failed for {}: {}",
                tenant_id.as_str(),
                err.message
            );
        }
        stores.insert(safe_tenant, store.clone());
        self.touch_tenant_engine(
            &tenant_key_segment(&tenant_id),
            tenant_id.as_str(),
            Some(
                tenant_data_dir(&self.config.data_dir, &tenant_id)
                    .display()
                    .to_string(),
            ),
            now_millis(),
        )?;
        Ok(store)
    }

    fn memory_store_for_tenant(
        &self,
        tenant_id: &str,
    ) -> Result<Arc<RedCoreTenantExecutor>, StoreAccessError> {
        let tenant_id = TenantId::new(tenant_id)
            .map_err(|error| StoreAccessError::internal(format!("invalid tenant id: {error}")))?;
        let safe_tenant = tenant_key_segment(&tenant_id);
        let mut stores = self
            .redcore_stores
            .lock()
            .map_err(|_| StoreAccessError::internal("redcore tenant map lock poisoned"))?;
        if let Some(store) = stores.get(&safe_tenant) {
            self.touch_tenant_engine(&safe_tenant, tenant_id.as_str(), None, now_millis())?;
            return Ok(store.clone());
        }
        let tenant_config = self.config.tenant_config(tenant_id.as_str());
        let mut graph_store = RedCoreGraphStore::memory();
        configure_payload_backend(&mut graph_store)?;
        let store = Arc::new(RedCoreTenantExecutor::new(
            graph_store,
            tenant_config.tenant_memory_quota_bytes,
        )?);
        if let Err(err) = store.enable_graph_hooks(
            tenant_hook_registrations(tenant_id.as_str()),
            tenant_id.as_str(),
        ) {
            eprintln!(
                "[theorem] enable graph hooks failed for {}: {}",
                tenant_id.as_str(),
                err.message
            );
        }
        stores.insert(safe_tenant, store.clone());
        self.touch_tenant_engine(
            &tenant_key_segment(&tenant_id),
            tenant_id.as_str(),
            None,
            now_millis(),
        )?;
        Ok(store)
    }

    fn touch_tenant_engine(
        &self,
        tenant_key: &str,
        tenant_id: &str,
        data_dir: Option<String>,
        now_ms: u64,
    ) -> Result<(), StoreAccessError> {
        let mut engines = self
            .tenant_engines
            .lock()
            .map_err(|_| StoreAccessError::internal("tenant engine state lock poisoned"))?;
        engines
            .entry(tenant_key.to_string())
            .and_modify(|meta| meta.touch_hot(now_ms, data_dir.clone()))
            .or_insert_with(|| {
                TenantEngineMeta::new(tenant_id, TenantEngineState::Hot, data_dir, now_ms)
            });
        Ok(())
    }

    /// Snapshot of every RedCore tenant currently materialized in the
    /// cache. Used by the TTL sweep loop to iterate without holding the
    /// tenant-map mutex across the per-tenant purge (which itself
    /// takes that tenant's writer mutex). Returns Arc clones so the
    /// caller can keep working after the map lock drops.
    ///
    /// Tenants only appear in the cache once they've been accessed at
    /// least once (lazy creation). The sweep doesn't try to enumerate
    /// on-disk tenants that haven't been opened yet -- those have no
    /// in-memory state to sweep and their TTL nodes will be filtered
    /// at read time by the InMemory expired-node filter the moment
    /// they ARE opened. Sweep visibility for never-accessed tenants is
    /// a deliberate non-goal of TTL-04.
    pub fn iter_redcore_tenants(
        &self,
    ) -> Result<Vec<(String, Arc<RedCoreTenantExecutor>)>, StoreAccessError> {
        let stores = self
            .redcore_stores
            .lock()
            .map_err(|_| StoreAccessError::internal("redcore tenant map lock poisoned"))?;
        Ok(stores
            .iter()
            .map(|(tenant, executor)| (tenant.clone(), executor.clone()))
            .collect())
    }

    pub fn iter_graph_caches(
        &self,
    ) -> Result<Vec<(String, Arc<GraphCacheTenant>)>, StoreAccessError> {
        let caches = self
            .graph_caches
            .lock()
            .map_err(|_| StoreAccessError::internal("graph cache tenant map lock poisoned"))?;
        Ok(caches
            .iter()
            .map(|(tenant, cache)| (tenant.clone(), cache.clone()))
            .collect())
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct ReadyReport {
    pub mode: String,
    pub store: String,
    pub durability: String,
    pub strict_acid: bool,
    pub require_volume: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_dir: Option<String>,
}

#[derive(Debug)]
pub struct StoreAccessError {
    pub code: String,
    pub message: String,
}

impl StoreAccessError {
    fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: "store_mode_unsupported".to_string(),
            message: message.into(),
        }
    }

    fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "store_internal_error".to_string(),
            message: message.into(),
        }
    }

    pub fn as_payload(&self) -> serde_json::Value {
        json!({
            "error": "store_unavailable",
            "code": self.code,
            "message": self.message
        })
    }
}

impl From<redis::RedisError> for StoreAccessError {
    fn from(error: redis::RedisError) -> Self {
        Self {
            code: "redis_store_error".to_string(),
            message: error.to_string(),
        }
    }
}

impl From<GraphStoreError> for StoreAccessError {
    fn from(error: GraphStoreError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }
}

/// Whether per-tenant stores auto-attach optional web-crawl graph hooks. The
/// Item changefeed hook is product-critical and always attaches; crawl hooks
/// remain a deliberate flag flip. Truthy: `1`/`true`/`on`/`yes`.
fn graph_hooks_enabled() -> bool {
    std::env::var("THEOREM_GRAPH_HOOKS")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

/// Whether per-tenant stores auto-attach the SPEC-7 standing-pass organizer
/// engine as a post-commit hook. The engine is advisory background compute
/// (generators propose, admission disposes), so it stays a deliberate flag flip
/// like the crawl hooks. Truthy: `1`/`true`/`on`/`yes`.
fn standing_pass_enabled() -> bool {
    std::env::var("THEOREM_STANDING_PASS")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

/// The graph-level hook registrations to attach to a fresh per-tenant store.
/// Crawl hooks ride `THEOREM_GRAPH_HOOKS`; the Item changefeed (SPEC-2) rides its
/// own `THEOREM_ITEM_CHANGEFEED`; the SPEC-7 standing-pass organizer engine rides
/// `THEOREM_STANDING_PASS`, so reactive structure-making can run without enabling
/// the heavier crawl hooks. Empty (the default) makes `enable_graph_hooks` a
/// no-op. Scoped per tenant: the standing pass writes its advisory structure
/// under `tenant_id`.
fn tenant_hook_registrations(tenant_id: &str) -> Vec<HookRegistration> {
    let mut registrations = Vec::new();
    if graph_hooks_enabled() {
        registrations.extend(rustyred_web::crawl_hooks());
    }
    if crate::items_changefeed::item_changefeed_enabled() {
        registrations.push(crate::items_changefeed::changefeed_registration());
    }
    if standing_pass_enabled() {
        match rustyred_thg_adapters::standing_pass_hook(rustyred_thg_adapters::StandingPassConfig {
            tenant_id: tenant_id.to_string(),
            ..Default::default()
        }) {
            Ok(registration) => registrations.push(registration),
            Err(err) => eprintln!(
                "[theorem] standing-pass hook init failed for {tenant_id}: {}",
                err.message
            ),
        }
    }
    registrations
}

#[cfg(test)]
mod standing_pass_activation_tests {
    use super::tenant_hook_registrations;

    /// SPEC-7 runtime activation: the standing-pass organizer engine attaches as
    /// a per-tenant graph hook only behind `THEOREM_STANDING_PASS`. Asserted by
    /// the registration name so it is independent of the other hook flags.
    /// `THEOREM_STANDING_PASS` is read by no other code path, so toggling it here
    /// cannot race another test reading the same var.
    #[test]
    fn standing_pass_hook_rides_its_env_flag() {
        std::env::set_var("THEOREM_STANDING_PASS", "1");
        let enabled = tenant_hook_registrations("theorem");
        std::env::remove_var("THEOREM_STANDING_PASS");
        let disabled = tenant_hook_registrations("theorem");

        assert!(
            enabled
                .iter()
                .any(|registration| registration.name == "reflexive.standing_pass"),
            "standing-pass hook registers when THEOREM_STANDING_PASS is truthy"
        );
        assert!(
            !disabled
                .iter()
                .any(|registration| registration.name == "reflexive.standing_pass"),
            "standing-pass hook is absent when the flag is unset"
        );
    }
}

#[derive(Debug)]
pub struct RedCoreTenantExecutor {
    writer: Mutex<RedCoreGraphStore>,
    tenant_memory_quota_bytes: usize,
    /// §P6-A pa6.1: cached `(graph_version, edges)` pair. Algorithm endpoints
    /// share the underlying allocation across concurrent calls; any mutation
    /// that bumps `graph_version` triggers a rebuild on the next read.
    cached_edges: RwLock<Option<(u64, Arc<Vec<EdgeRecord>>)>>,
    /// Graph-level hook dispatcher, owned here so its worker lives as long as the
    /// tenant store. `None` unless hooks are enabled (see `enable_graph_hooks`).
    hook_dispatcher: Mutex<Option<HookDispatcher>>,
}

/// Bridges the executor's private writer mutex to the hook dispatcher worker.
/// Holds a `Weak` so the executor->dispatcher->executor reference does not form
/// a strong cycle (the dispatcher is owned by the executor it points back to).
struct ExecutorHookStore(std::sync::Weak<RedCoreTenantExecutor>);

impl HookStoreAccess for ExecutorHookStore {
    fn with_store_mut(&self, f: &mut dyn FnMut(&mut RedCoreGraphStore)) -> bool {
        match self.0.upgrade() {
            Some(executor) => executor.run_hook_batch(f),
            None => false, // executor gone: fail open
        }
    }
}

impl RedCoreTenantExecutor {
    fn new(store: RedCoreGraphStore, tenant_memory_quota_bytes: usize) -> GraphStoreResult<Self> {
        store.set_hot_cache_budget_bytes(tenant_memory_quota_bytes)?;
        Ok(Self {
            writer: Mutex::new(store),
            tenant_memory_quota_bytes,
            cached_edges: RwLock::new(None),
            hook_dispatcher: Mutex::new(None),
        })
    }

    /// Run a coalesced hook batch under the writer lock. The executor no longer
    /// keeps a second committed read mirror, so hook writes become visible
    /// through the single writer store as soon as they commit.
    fn run_hook_batch(&self, f: &mut dyn FnMut(&mut RedCoreGraphStore)) -> bool {
        let mut writer = match self.writer.lock() {
            Ok(writer) => writer,
            Err(_) => return false,
        };
        f(&mut writer);
        true
    }

    /// Attach a hook dispatcher to this tenant store: start the worker over the
    /// writer, install the emitter, and own the dispatcher for the store's
    /// lifetime. No-op when `registrations` is empty. Idempotent-ish: a second
    /// call replaces the dispatcher (the prior one's worker stops on drop).
    pub fn enable_graph_hooks(
        self: &Arc<Self>,
        registrations: Vec<HookRegistration>,
        tenant: &str,
    ) -> GraphStoreResult<()> {
        if registrations.is_empty() {
            return Ok(());
        }
        let dispatcher = HookDispatcher::start(
            ExecutorHookStore(Arc::downgrade(self)),
            registrations,
            HookDispatcherConfig::default(),
        );
        {
            let mut writer = self.lock_writer()?;
            writer.attach_hook_emitter(dispatcher.emitter());
            writer.set_hook_tenant(tenant);
        }
        if let Ok(mut guard) = self.hook_dispatcher.lock() {
            *guard = Some(dispatcher);
        }
        Ok(())
    }

    /// Block until the hook dispatcher has drained (or the timeout elapses).
    /// Returns true when drained or when no dispatcher is attached. Useful for
    /// deterministic shutdown and tests.
    pub fn quiesce_hooks(&self, timeout: std::time::Duration) -> bool {
        match self.hook_dispatcher.lock() {
            Ok(guard) => guard.as_ref().map(|d| d.quiesce(timeout)).unwrap_or(true),
            Err(_) => true,
        }
    }

    /// §P6-A pa6.1: cheap `Arc<Vec<EdgeRecord>>` clone for algorithm endpoints.
    /// Reads the cached arc when `graph_version` matches the current snapshot;
    /// rebuilds otherwise. Concurrent callers share the same allocation.
    pub fn list_edges_arc(&self) -> GraphStoreResult<Arc<Vec<EdgeRecord>>> {
        let current_version = self.stats()?.version;
        {
            let guard = self.cached_edges.read().map_err(|_| {
                GraphStoreError::new(
                    "redcore_snapshot_lock_poisoned",
                    "RedCore arc-cache lock poisoned",
                )
            })?;
            if let Some((cached_version, arc)) = guard.as_ref() {
                if *cached_version == current_version {
                    return Ok(Arc::clone(arc));
                }
            }
        }
        let edges = self.lock_writer()?.graph_snapshot().edges;
        let arc = Arc::new(edges);
        let mut guard = self.cached_edges.write().map_err(|_| {
            GraphStoreError::new(
                "redcore_snapshot_lock_poisoned",
                "RedCore arc-cache lock poisoned",
            )
        })?;
        *guard = Some((current_version, Arc::clone(&arc)));
        Ok(arc)
    }

    pub fn commit_batch(&self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        let mut writer = self.lock_writer()?;
        self.enforce_tenant_memory_quota(&writer, &batch)?;
        writer.commit_batch(batch)
    }

    pub fn upsert_node(&self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        self.commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(node)]))?
            .writes
            .into_iter()
            .next()
            .ok_or_else(|| GraphStoreError::new("redcore_missing_write", "node write vanished"))
    }

    pub fn upsert_edge(&self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        self.commit_batch(GraphMutationBatch::new([GraphMutation::EdgeUpsert(edge)]))?
            .writes
            .into_iter()
            .next()
            .ok_or_else(|| GraphStoreError::new("redcore_missing_write", "edge write vanished"))
    }

    #[cfg(test)]
    pub fn read_barrier(&self) -> GraphStoreResult<u64> {
        Ok(self.lock_writer()?.status().last_txn_id)
    }

    pub fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.lock_writer()?.get_node(id)
    }

    pub fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.lock_writer()?.get_edge(id)
    }

    pub fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.lock_writer()?.query_nodes(query)
    }

    pub fn memory_documents_by_updated_at(
        &self,
        query: MemoryDocumentQuery,
    ) -> GraphStoreResult<Vec<NodeRecord>> {
        self.lock_writer()?.memory_documents_by_updated_at(query)
    }

    pub fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.lock_writer()?.neighbors(query)
    }

    pub fn stats(&self) -> GraphStoreResult<GraphStats> {
        let mut stats = self.lock_writer()?.stats()?;
        stats.memory_quota_bytes = self.tenant_memory_quota_bytes;
        Ok(stats)
    }

    pub fn cheap_stats(&self) -> GraphStoreResult<GraphStats> {
        let mut stats = self.lock_writer()?.stats()?;
        stats.memory_quota_bytes = self.tenant_memory_quota_bytes;
        Ok(stats)
    }

    pub fn cached_edges_diagnostics(&self) -> Value {
        match self.cached_edges.read() {
            Ok(guard) => match guard.as_ref() {
                Some((version, edges)) => json!({
                    "present": true,
                    "graph_version": version,
                    "edges": edges.len(),
                    "arc_strong_count": Arc::strong_count(edges),
                    "estimated_record_struct_bytes": edges
                        .len()
                        .saturating_mul(std::mem::size_of::<EdgeRecord>()),
                }),
                None => json!({
                    "present": false,
                    "edges": 0,
                    "estimated_record_struct_bytes": 0,
                }),
            },
            Err(_) => json!({
                "present": false,
                "error": "redcore edge cache lock poisoned",
            }),
        }
    }

    pub fn hot_cache_diagnostics(&self) -> Value {
        match self
            .lock_writer()
            .and_then(|writer| writer.hot_cache_report())
        {
            Ok(report) => json!(report),
            Err(error) => json!({
                "error": {
                    "code": error.code,
                    "message": error.message,
                },
            }),
        }
    }

    pub fn archive_residency_diagnostics(&self) -> Value {
        match self.lock_writer() {
            Ok(writer) => json!(writer.archive_residency_report()),
            Err(error) => json!({
                "error": {
                    "code": error.code,
                    "message": error.message,
                },
            }),
        }
    }

    pub fn clear_hot_caches(&self) {
        if let Ok(mut guard) = self.cached_edges.write() {
            *guard = None;
        }
        if let Ok(writer) = self.lock_writer() {
            let _ = writer.clear_hot_cache();
        }
    }

    pub fn verify(&self) -> GraphStoreResult<VerifyReport> {
        self.lock_writer()?.verify()
    }

    pub fn rebuild_indexes(&self) -> GraphStoreResult<GraphRebuildReport> {
        let mut writer = self.lock_writer()?;
        writer.rebuild_indexes()
    }

    // ---- TTL surface (TTL-04) ---------------------------------------
    //
    // These methods wrap the inherent TTL methods on RedCoreGraphStore through
    // the single tenant writer, so there is no second resident read mirror to
    // keep in sync.

    /// Set or clear `_ttl_expires_at_ms` on an existing node. Routes
    /// through RedCoreGraphStore::set_node_ttl which journals the change
    /// as a NodeUpsert AOF op.
    pub fn set_node_ttl(
        &self,
        id: &str,
        expires_at_ms: Option<i64>,
    ) -> GraphStoreResult<GraphWriteResult> {
        let mut writer = self.lock_writer()?;
        writer.set_node_ttl(id, expires_at_ms)
    }

    /// Read a node regardless of TTL window. Used by admin / debug surfaces.
    pub fn get_node_including_expired(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        let writer = self.lock_writer()?;
        writer.get_node_including_expired(id)
    }

    /// Return nodes whose `_ttl_expires_at_ms <= ts_ms`, ordered by
    /// expiration. Read-only.
    pub fn nodes_expiring_before(
        &self,
        ts_ms: i64,
        limit: usize,
    ) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(self.lock_writer()?.nodes_expiring_before(ts_ms, limit))
    }

    /// Number of TTL-bearing live nodes in this tenant's graph.
    pub fn ttl_active_count(&self) -> GraphStoreResult<usize> {
        Ok(self.lock_writer()?.ttl_active_count())
    }

    /// Sweep expired nodes from this tenant's graph durably. Locks the
    /// writer and journals each expired node as a NodeDelete AOF op. Returns
    /// the count purged. Called by the background sweep task.
    pub fn purge_expired_nodes(&self) -> GraphStoreResult<usize> {
        let mut writer = self.lock_writer()?;
        writer.purge_expired_nodes()
    }

    pub fn labels(&self) -> GraphStoreResult<Vec<String>> {
        self.lock_writer()?.labels()
    }

    pub fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        self.lock_writer()?.edge_types()
    }

    pub fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        self.lock_writer()?.property_keys()
    }

    /// Phase 6: snapshot all live edges for graph-algorithm endpoints.
    /// Returns a clone of the edge vector; caller must not hold a lock.
    pub fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        Ok(self.lock_writer()?.graph_snapshot().edges)
    }

    pub fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(self.lock_writer()?.graph_snapshot())
    }

    pub fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        Ok(self.lock_writer()?.epistemic_neighbors(
            node_id,
            epistemic_types,
            min_confidence,
            max_depth,
        ))
    }

    pub fn designate_vector_property(
        &self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        let mut writer = self.lock_writer()?;
        writer.designate_vector_property(label, property_name, dimension)
    }

    pub fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        let writer = self.lock_writer()?;
        Ok(writer.vector_designations())
    }

    pub fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        let writer = self.lock_writer()?;
        writer.vector_search(label, property_name, query, k)
    }

    pub fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        let writer = self.lock_writer()?;
        writer.hybrid_search(label, property_name, query, k, graph_seeds, max_hops, alpha)
    }

    pub fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        let writer = self.lock_writer()?;
        writer.hybrid_search_with_config(
            label,
            property_name,
            query,
            k,
            graph_seeds,
            max_hops,
            config,
        )
    }

    fn lock_writer(&self) -> GraphStoreResult<std::sync::MutexGuard<'_, RedCoreGraphStore>> {
        self.writer.lock().map_err(|_| {
            GraphStoreError::new(
                "redcore_writer_lock_poisoned",
                "RedCore writer lock poisoned",
            )
        })
    }

    fn enforce_tenant_memory_quota(
        &self,
        writer: &RedCoreGraphStore,
        batch: &GraphMutationBatch,
    ) -> GraphStoreResult<()> {
        if self.tenant_memory_quota_bytes == 0 {
            return Ok(());
        }

        let mut projected_store = InMemoryGraphStore::from_snapshot(writer.graph_snapshot())?;
        for mutation in &batch.mutations {
            match mutation {
                GraphMutation::NodeUpsert(node) => {
                    projected_store.upsert_node(node.clone())?;
                }
                GraphMutation::EdgeUpsert(edge) => {
                    projected_store.upsert_edge(edge.clone())?;
                }
            }
        }

        let projected_memory = projected_store.stats().memory_bytes;
        if projected_memory > self.tenant_memory_quota_bytes {
            return Err(GraphStoreError::new(
                "tenant_memory_quota_exceeded",
                format!(
                    "tenant memory quota exceeded: projected {projected_memory} > quota {}",
                    self.tenant_memory_quota_bytes,
                ),
            ));
        }

        Ok(())
    }
}

fn configure_payload_backend(store: &mut RedCoreGraphStore) -> GraphStoreResult<()> {
    if let Some(backend) = payload_backend_from_env()? {
        store.set_payload_backend(backend);
    }
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default()
}

fn prometheus_label_value(value: &str) -> String {
    value
        .chars()
        .flat_map(|ch| match ch {
            '\\' => "\\\\".chars().collect::<Vec<_>>(),
            '"' => "\\\"".chars().collect::<Vec<_>>(),
            '\n' => "\\n".chars().collect::<Vec<_>>(),
            _ => vec![ch],
        })
        .collect()
}

#[derive(Clone)]
pub enum TenantGraphStore {
    RedCore(Arc<RedCoreTenantExecutor>),
    Redis(RedisGraphStore),
}

/// Read-only surface for the reflexive executor: lets the Cypher query
/// surface join topology with the representation/adapter sidecars through
/// the adapters crate without the tenant store becoming an adapter store.
impl rustyred_thg_adapters::ReflexiveReadStore for TenantGraphStore {
    fn read_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.get_node(id)
    }

    fn read_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.get_edge(id)
    }

    fn read_neighbors(
        &self,
        query: rustyred_thg_core::NeighborQuery,
    ) -> GraphStoreResult<Vec<rustyred_thg_core::NeighborHit>> {
        self.neighbors(query)
    }
}

impl TenantGraphStore {
    pub fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        match self {
            Self::RedCore(store) => store.upsert_node(node),
            Self::Redis(store) => store.upsert_node(node),
        }
    }

    pub fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        match self {
            Self::RedCore(store) => store.upsert_edge(edge),
            Self::Redis(store) => store.upsert_edge(edge),
        }
    }

    /// Apply a `GraphMutationBatch` in a single transaction. RedCore uses its
    /// transactional `commit_batch`. Redis loops per-mutation upserts (it has
    /// no atomic batch primitive yet); callers still get the bulk-loader's
    /// `batches:` counter, just without single-transaction semantics on Redis.
    pub fn commit_batch(
        &mut self,
        batch: rustyred_thg_core::GraphMutationBatch,
    ) -> GraphStoreResult<rustyred_thg_core::GraphTransaction> {
        match self {
            Self::RedCore(executor) => executor.commit_batch(batch),
            Self::Redis(store) => {
                let mut writes: Vec<rustyred_thg_core::GraphWriteResult> = Vec::new();
                for mutation in batch.mutations {
                    match mutation {
                        rustyred_thg_core::GraphMutation::NodeUpsert(node) => {
                            writes.push(store.upsert_node(node)?);
                        }
                        rustyred_thg_core::GraphMutation::EdgeUpsert(edge) => {
                            writes.push(store.upsert_edge(edge)?);
                        }
                    }
                }
                let graph_version = store.stats().map(|s| s.version).unwrap_or(0);
                Ok(rustyred_thg_core::GraphTransaction {
                    txn_id: writes.len() as u64,
                    graph_version,
                    writes,
                })
            }
        }
    }

    pub fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        match self {
            Self::RedCore(store) => store.get_node(id),
            Self::Redis(store) => store.get_node(id),
        }
    }

    pub fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        match self {
            Self::RedCore(store) => store.get_edge(id),
            Self::Redis(store) => store.get_edge(id),
        }
    }

    pub fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        match self {
            Self::RedCore(store) => store.query_nodes(query),
            Self::Redis(store) => store.query_nodes(query),
        }
    }

    pub fn memory_documents_by_updated_at(
        &self,
        query: MemoryDocumentQuery,
    ) -> GraphStoreResult<Vec<NodeRecord>> {
        match self {
            Self::RedCore(store) => store.memory_documents_by_updated_at(query),
            Self::Redis(store) => {
                let mut node_query = NodeQuery::label("MemoryDocument")
                    .with_property("tenant_slug", Value::String(query.tenant_slug.clone()))
                    .with_limit(10_000);
                if let Some(status) = query.status.clone() {
                    node_query = node_query.with_property("status", Value::String(status));
                }
                let since = query.since.as_deref().map(str::trim).unwrap_or_default();
                let before = query.before.as_deref().map(str::trim).unwrap_or_default();
                let mut nodes = store.query_nodes(node_query)?;
                nodes.retain(|node| {
                    let status = node
                        .properties
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    if !query.include_deleted && status == "deleted" {
                        return false;
                    }
                    let updated_at = node
                        .properties
                        .get("updated_at")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    (since.is_empty() || updated_at >= since)
                        && (before.is_empty() || updated_at < before)
                });
                nodes.sort_by(|left, right| {
                    let left_updated_at = left
                        .properties
                        .get("updated_at")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let right_updated_at = right
                        .properties
                        .get("updated_at")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    right_updated_at
                        .cmp(left_updated_at)
                        .then_with(|| right.id.cmp(&left.id))
                });
                nodes.truncate(query.limit.filter(|limit| *limit > 0).unwrap_or(100));
                Ok(nodes)
            }
        }
    }

    pub fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        match self {
            Self::RedCore(store) => store.neighbors(query),
            Self::Redis(store) => store.neighbors(query),
        }
    }

    pub fn stats(&self) -> GraphStoreResult<GraphStats> {
        match self {
            Self::RedCore(store) => store.stats(),
            Self::Redis(store) => store.stats(),
        }
    }

    pub fn verify(&self) -> GraphStoreResult<VerifyReport> {
        match self {
            Self::RedCore(store) => store.verify(),
            Self::Redis(store) => store.verify(),
        }
    }

    pub fn rebuild_indexes(&mut self) -> GraphStoreResult<GraphRebuildReport> {
        match self {
            Self::RedCore(store) => store.rebuild_indexes(),
            Self::Redis(store) => store.rebuild_indexes(),
        }
    }

    pub fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        match self {
            Self::RedCore(store) => store.graph_snapshot(),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "adapter routing requires a RedCore graph snapshot",
            )),
        }
    }

    pub fn labels(&self) -> GraphStoreResult<Vec<String>> {
        match self {
            Self::RedCore(store) => store.labels(),
            Self::Redis(store) => store.labels(),
        }
    }

    pub fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        match self {
            Self::RedCore(store) => store.edge_types(),
            Self::Redis(store) => store.edge_types(),
        }
    }

    pub fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        match self {
            Self::RedCore(store) => store.property_keys(),
            Self::Redis(store) => store.property_keys(),
        }
    }

    /// Phase 6: snapshot all live edges for graph algorithms.
    /// Redis backend is currently unsupported (would require a full scan).
    pub fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        match self {
            Self::RedCore(store) => store.list_edges(),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "graph algorithms are not supported on Redis graph stores",
            )),
        }
    }

    pub fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        match self {
            Self::RedCore(store) => store.graph_snapshot(),
            Self::Redis(_) => Err(GraphStoreError::new(
                "legacy_redis_instant_kg_unsupported",
                "instant KG requires the native RedCore graph store; RUSTY_RED_MODE=redis is a legacy compatibility path and should be changed to RUSTY_RED_MODE=embedded",
            )),
        }
    }

    /// §P6-A pa6.1: `Arc<Vec<EdgeRecord>>` snapshot used by the algorithm
    /// endpoints. The RedCore variant returns a shared, version-cached arc;
    /// callers must not mutate.
    pub fn list_edges_arc(&self) -> GraphStoreResult<Arc<Vec<EdgeRecord>>> {
        match self {
            Self::RedCore(store) => store.list_edges_arc(),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "graph algorithms are not supported on Redis graph stores",
            )),
        }
    }

    pub fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        match self {
            Self::RedCore(store) => {
                store.epistemic_neighbors(node_id, epistemic_types, min_confidence, max_depth)
            }
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "epistemic_neighbors is not supported on Redis graph stores",
            )),
        }
    }

    pub fn designate_vector_property(
        &self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        match self {
            Self::RedCore(store) => {
                store.designate_vector_property(label, property_name, dimension)
            }
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "designate_vector_property is not supported on Redis graph stores",
            )),
        }
    }

    pub fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        match self {
            Self::RedCore(store) => store.vector_designations(),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "vector_designations is not supported on Redis graph stores",
            )),
        }
    }

    pub fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        match self {
            Self::RedCore(store) => store.vector_search(label, property_name, query, k),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "vector_search is not supported on Redis graph stores",
            )),
        }
    }

    pub fn hybrid_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        alpha: f32,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        match self {
            Self::RedCore(store) => {
                store.hybrid_search(label, property_name, query, k, graph_seeds, max_hops, alpha)
            }
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "hybrid_search is not supported on Redis graph stores",
            )),
        }
    }

    pub fn hybrid_search_with_config(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
        graph_seeds: &[String],
        max_hops: usize,
        config: &HybridScoringConfig,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        match self {
            Self::RedCore(store) => store.hybrid_search_with_config(
                label,
                property_name,
                query,
                k,
                graph_seeds,
                max_hops,
                config,
            ),
            Self::Redis(_) => Err(GraphStoreError::new(
                "unsupported_operation",
                "hybrid_search is not supported on Redis graph stores",
            )),
        }
    }
}

impl McpGraphBackend for TenantGraphStore {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        TenantGraphStore::get_node(self, id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        TenantGraphStore::get_edge(self, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        TenantGraphStore::query_nodes(self, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        TenantGraphStore::neighbors(self, query)
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        TenantGraphStore::stats(self)
    }

    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        TenantGraphStore::verify(self)
    }

    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        TenantGraphStore::labels(self)
    }

    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        TenantGraphStore::edge_types(self)
    }

    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        TenantGraphStore::property_keys(self)
    }

    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        TenantGraphStore::list_edges(self)
    }

    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        TenantGraphStore::graph_snapshot(self)
    }

    fn memory_documents_by_updated_at(
        &self,
        query: MemoryDocumentQuery,
    ) -> GraphStoreResult<Vec<NodeRecord>> {
        TenantGraphStore::memory_documents_by_updated_at(self, query)
    }

    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        TenantGraphStore::upsert_node(self, node).map(|_| ())
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        TenantGraphStore::upsert_edge(self, edge).map(|_| ())
    }

    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        TenantGraphStore::vector_designations(self)
    }

    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        TenantGraphStore::designate_vector_property(self, label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        TenantGraphStore::vector_search(self, label, property_name, query, k)
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
        TenantGraphStore::hybrid_search(
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
        TenantGraphStore::hybrid_search_with_config(
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
        TenantGraphStore::epistemic_neighbors(
            self,
            node_id,
            epistemic_types,
            min_confidence,
            max_depth,
        )
    }

    /// §P6-A pa6.1: override the trait defaults to read `list_edges_arc` so
    /// concurrent algorithm endpoints share one allocation rather than each
    /// cloning the live edge vector.
    fn algo_ppr(
        &self,
        seeds: &std::collections::HashMap<String, f64>,
        alpha: f64,
        epsilon: f64,
        max_pushes: usize,
    ) -> GraphStoreResult<std::collections::HashMap<String, f64>> {
        let edges = self.list_edges_arc()?;
        let mut adjacency: std::collections::HashMap<String, Vec<(String, f64)>> =
            std::collections::HashMap::new();
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

    fn algo_components(&self, directed: bool) -> GraphStoreResult<Vec<Vec<String>>> {
        let edges = self.list_edges_arc()?;
        Ok(rustyred_thg_core::connected_components(
            edges.as_slice(),
            directed,
        ))
    }

    fn algo_pagerank(
        &self,
        damping: f64,
        max_iter: usize,
        tolerance: f64,
    ) -> GraphStoreResult<std::collections::HashMap<String, f64>> {
        let edges = self.list_edges_arc()?;
        Ok(rustyred_thg_core::pagerank(
            edges.as_slice(),
            damping,
            max_iter,
            tolerance,
        ))
    }

    fn algo_communities(&self) -> GraphStoreResult<(std::collections::HashMap<String, u64>, f64)> {
        let edges = self.list_edges_arc()?;
        Ok(rustyred_thg_core::label_propagation_communities(
            edges.as_slice(),
        ))
    }
}

#[derive(Clone)]
pub struct ProductMcpBackend {
    state: AppState,
    tenant_id: String,
    store: TenantGraphStore,
}

struct RuntimeTenantMirrorGraphStore<'a> {
    store: &'a mut TenantGraphStore,
    mirror: InMemoryGraphStore,
}

impl<'a> RuntimeTenantMirrorGraphStore<'a> {
    fn new(store: &'a mut TenantGraphStore) -> GraphStoreResult<Self> {
        let mirror = InMemoryGraphStore::from_snapshot(store.graph_snapshot()?)?;
        Ok(Self { store, mirror })
    }
}

impl GraphStore for RuntimeTenantMirrorGraphStore<'_> {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        let write = self.store.upsert_node(node.clone())?;
        GraphStore::upsert_node(&mut self.mirror, node)?;
        Ok(write)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        let write = self.store.upsert_edge(edge.clone())?;
        GraphStore::upsert_edge(&mut self.mirror, edge)?;
        Ok(write)
    }

    fn get_node(&self, id: &str) -> Option<&NodeRecord> {
        GraphStore::get_node(&self.mirror, id)
    }

    fn get_edge(&self, id: &str) -> Option<&EdgeRecord> {
        GraphStore::get_edge(&self.mirror, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> Vec<NodeRecord> {
        GraphStore::query_nodes(&self.mirror, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> Vec<NeighborHit> {
        GraphStore::neighbors(&self.mirror, query)
    }

    fn stats(&self) -> GraphStats {
        GraphStore::stats(&self.mirror)
    }

    fn verify(&self) -> VerifyReport {
        GraphStore::verify(&self.mirror)
    }

    fn rebuild_indexes(&mut self) -> GraphStoreResult<GraphRebuildReport> {
        let report = self.store.rebuild_indexes()?;
        GraphStore::rebuild_indexes(&mut self.mirror)?;
        Ok(report)
    }
}

fn mcp_harness_runtime_error(error: HarnessRuntimeError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "harness_runtime_error" })),
    }
}

fn mcp_composed_agent_runtime_error(error: ComposedAgentRuntimeError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "composed_agent_runtime_error" })),
    }
}

fn mcp_head_invocation_error(error: HeadInvocationError) -> McpError {
    McpError {
        code: -32603,
        message: error.to_string(),
        data: Some(json!({ "code": "head_invocation_error" })),
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

fn mirror_job_to_dispatch_if_configured(job: &HarnessJob) -> Result<bool, McpError> {
    let Some(database_url) = std::env::var(DISPATCH_DATABASE_URL_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    else {
        return Ok(false);
    };
    let dispatch_job = DispatchJob::from_harness(job);
    let priority = priority_from_harness(job.priority);
    let job_id = job.job_id.clone();
    let handle = std::thread::Builder::new()
        .name(format!("dispatch-mirror-{job_id}"))
        .spawn(move || -> Result<(), String> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| error.to_string())?;
            runtime.block_on(async move {
                let queue = DispatchQueue::connect(&database_url)
                    .await
                    .map_err(|error| error.to_string())?;
                // Idempotent (CREATE TABLE IF NOT EXISTS): a fresh dispatch database, or
                // one where the harness server is the first writer, still receives the row
                // instead of failing on a missing `dispatch_jobs` table.
                queue.migrate().await.map_err(|error| error.to_string())?;
                queue
                    .submit(dispatch_job, priority)
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(())
            })
        })
        .map_err(|error| McpError::internal(format!("dispatch mirror thread failed: {error}")))?;
    handle
        .join()
        .map_err(|_| McpError::internal("dispatch mirror thread panicked"))?
        .map_err(|error| McpError::internal(format!("dispatch mirror failed: {error}")))?;
    Ok(true)
}

#[derive(Clone, PartialEq, prost::Message)]
struct InvokeAppAffordanceGrpcRequest {
    #[prost(string, tag = "1")]
    tenant_id: String,
    #[prost(string, tag = "2")]
    affordance_id: String,
    #[prost(string, tag = "3")]
    actor: String,
    #[prost(string, tag = "4")]
    request_json: String,
    #[prost(bool, tag = "5")]
    dry_run: bool,
    #[prost(bool, tag = "6")]
    confirmed: bool,
    #[prost(uint64, tag = "7")]
    timeout_ms: u64,
}

#[derive(Clone, PartialEq, prost::Message)]
struct InvokeAppAffordanceGrpcResponse {
    #[prost(string, tag = "1")]
    tenant_id: String,
    #[prost(string, tag = "2")]
    affordance_id: String,
    #[prost(string, tag = "3")]
    server_id: String,
    #[prost(string, tag = "4")]
    tool_name: String,
    #[prost(string, tag = "5")]
    status: String,
    #[prost(bool, tag = "6")]
    executed: bool,
    #[prost(string, tag = "7")]
    receipt_hash: String,
    #[prost(string, tag = "8")]
    receipt_json: String,
    #[prost(string, tag = "9")]
    output_json: String,
    #[prost(string, tag = "10")]
    error_code: String,
    #[prost(string, tag = "11")]
    message: String,
    #[prost(uint64, tag = "12")]
    elapsed_ms: u64,
}

impl McpGraphBackend for ProductMcpBackend {
    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        self.store.get_node(id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        self.store.get_edge(id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        self.store.query_nodes(query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        self.store.neighbors(query)
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        self.store.stats()
    }

    fn verify(&self) -> GraphStoreResult<VerifyReport> {
        self.store.verify()
    }

    fn labels(&self) -> GraphStoreResult<Vec<String>> {
        self.store.labels()
    }

    fn edge_types(&self) -> GraphStoreResult<Vec<String>> {
        self.store.edge_types()
    }

    fn property_keys(&self) -> GraphStoreResult<Vec<String>> {
        self.store.property_keys()
    }

    fn list_edges(&self) -> GraphStoreResult<Vec<EdgeRecord>> {
        self.store.list_edges()
    }

    fn graph_snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        self.store.graph_snapshot()
    }

    fn memory_documents_by_updated_at(
        &self,
        query: MemoryDocumentQuery,
    ) -> GraphStoreResult<Vec<NodeRecord>> {
        self.store.memory_documents_by_updated_at(query)
    }

    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.store.upsert_node(node.clone())?;
        self.state.observability.record_mutation();
        self.state
            .maybe_index_node_spatially(&self.tenant_id, &node);
        self.state.maybe_index_node_fulltext(&self.tenant_id, &node);
        Ok(())
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.store.upsert_edge(edge)?;
        self.state.observability.record_mutation();
        Ok(())
    }

    fn append_harness_transition(
        &mut self,
        transition: TransitionInput,
    ) -> Result<Value, McpError> {
        let mut runtime_store = RuntimeTenantMirrorGraphStore::new(&mut self.store)?;
        append_transition_from_store(&mut runtime_store, transition)
            .map(transition_result_payload)
            .map_err(mcp_harness_runtime_error)
    }

    fn harness_run_detail(&self, run_id: &str) -> Result<Option<Value>, McpError> {
        let snapshot = self.store.graph_snapshot()?;
        let mirror = InMemoryGraphStore::from_snapshot(snapshot)?;
        match load_run(&mirror, run_id).map_err(mcp_harness_runtime_error)? {
            None => Ok(None),
            Some(run) => {
                let events = load_events(&mirror, run_id).map_err(mcp_harness_runtime_error)?;
                Ok(Some(json!({ "run": run, "events": events })))
            }
        }
    }

    fn composed_agent_run(
        &mut self,
        binding_id: String,
        task: String,
        claims: Vec<GroundedClaim>,
    ) -> Result<Value, McpError> {
        let mut runtime_store = RuntimeTenantMirrorGraphStore::new(&mut self.store)?;
        let invoker = ProviderHeadInvoker::from_env().map_err(mcp_head_invocation_error)?;
        let result = if claims.is_empty() {
            theorem_harness_runtime::run_configured_composed_agent(
                &mut runtime_store,
                &binding_id,
                &task,
                &invoker,
            )
        } else {
            theorem_harness_runtime::run_configured_composed_agent_with_claims(
                &mut runtime_store,
                &binding_id,
                &task,
                claims,
                &invoker,
            )
        }
        .map_err(mcp_composed_agent_runtime_error)?;
        serde_json::to_value(result).map_err(|error| {
            McpError::internal(format!(
                "composed_agent_run payload serialization failed: {error}"
            ))
        })
    }

    fn job_submit(
        &mut self,
        submission: JobSubmission,
        submitted_by: String,
    ) -> Result<Value, McpError> {
        let mut runtime_store = RuntimeTenantMirrorGraphStore::new(&mut self.store)?;
        let mut result = job_submit_to_store(&mut runtime_store, submission, submitted_by)?;
        let job_value = result
            .get("job")
            .cloned()
            .ok_or_else(|| McpError::internal("job_submit payload missing job"))?;
        let job = serde_json::from_value::<HarnessJob>(job_value).map_err(|error| {
            McpError::internal(format!(
                "job_submit payload could not mirror to dispatch: {error}"
            ))
        })?;
        // The board write above (`job_submit_to_store`) is canonical and already
        // committed. A dispatch-mirror failure (Postgres unreachable, schema missing,
        // transient error) must NOT fail the submit -- otherwise the job lands on the
        // board while `job_submit` reports an error, the exact reliability bug this
        // addendum fixes. Record the mirror outcome on the payload and still return Ok.
        match mirror_job_to_dispatch_if_configured(&job) {
            Ok(mirrored) => {
                if let Value::Object(map) = &mut result {
                    map.insert("dispatch_mirrored".to_string(), json!(mirrored));
                }
            }
            Err(error) => {
                if let Value::Object(map) = &mut result {
                    map.insert("dispatch_mirrored".to_string(), json!(false));
                    map.insert("dispatch_mirror_error".to_string(), json!(error.message));
                }
            }
        }
        Ok(result)
    }

    fn job_list(&self, repo: Option<String>, state: Option<String>) -> Result<Value, McpError> {
        let snapshot = self.store.graph_snapshot()?;
        let mirror = InMemoryGraphStore::from_snapshot(snapshot)?;
        job_list_from_store(&mirror, repo, state)
    }

    fn job_note(&mut self, job_id: String, input: JobNoteInput) -> Result<Value, McpError> {
        let mut runtime_store = RuntimeTenantMirrorGraphStore::new(&mut self.store)?;
        job_note_to_store(&mut runtime_store, job_id, input)
    }

    fn job_archive(
        &mut self,
        job_id: String,
        reason: String,
        actor: String,
    ) -> Result<Value, McpError> {
        let mut runtime_store = RuntimeTenantMirrorGraphStore::new(&mut self.store)?;
        job_archive_to_store(&mut runtime_store, job_id, reason, actor)
    }

    fn vector_designations(&self) -> GraphStoreResult<Vec<VectorDesignation>> {
        self.store.vector_designations()
    }

    fn designate_vector_property(
        &mut self,
        label: &str,
        property_name: &str,
        dimension: usize,
    ) -> GraphStoreResult<()> {
        self.store
            .designate_vector_property(label, property_name, dimension)
    }

    fn vector_search(
        &self,
        label: Option<&str>,
        property_name: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        self.store.vector_search(label, property_name, query, k)
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
        self.store
            .hybrid_search(label, property_name, query, k, graph_seeds, max_hops, alpha)
    }

    fn hybrid_scoring_config(&self) -> HybridScoringConfig {
        self.state
            .config
            .tenant_config(&self.tenant_id)
            .hybrid_scoring
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
        self.store.hybrid_search_with_config(
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
        self.state
            .designate_fulltext_property(&self.tenant_id, label, property)
            .map_err(|error| GraphStoreError::new(error.code, error.message))
    }

    fn fulltext_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        if property == MEMORY_FULLTEXT_PROPERTY
            && label.map(is_memory_fulltext_label).unwrap_or(true)
        {
            let label_to_warm = label.unwrap_or("MemoryAtom");
            self.state
                .ensure_memory_fulltext_index(&self.tenant_id, label_to_warm)
                .map_err(|error| GraphStoreError::new(error.code, error.message))?;
        }
        self.state
            .fulltext_search(&self.tenant_id, label, property, query, k)
            .map_err(|error| GraphStoreError::new(error.code, error.message))
    }

    fn skip_tenant_wide_recall_scan_when_indexed_empty(&self) -> bool {
        true
    }

    fn designate_spatial_property(
        &mut self,
        label: &str,
        lat_property: &str,
        lon_property: &str,
        resolution: u8,
    ) -> GraphStoreResult<()> {
        self.state
            .designate_spatial_property(
                &self.tenant_id,
                label,
                lat_property,
                lon_property,
                resolution,
            )
            .map_err(|error| GraphStoreError::new(error.code, error.message))
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
        self.state
            .spatial_radius_search(
                &self.tenant_id,
                label,
                lat_property,
                lon_property,
                lat,
                lon,
                radius_km,
            )
            .map_err(|error| GraphStoreError::new(error.code, error.message))
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
        self.state
            .spatial_bbox_search(
                &self.tenant_id,
                label,
                lat_property,
                lon_property,
                min_lat,
                min_lon,
                max_lat,
                max_lon,
            )
            .map_err(|error| GraphStoreError::new(error.code, error.message))
    }

    fn epistemic_neighbors(
        &self,
        node_id: &str,
        epistemic_types: Option<&[EpistemicType]>,
        min_confidence: Option<f64>,
        max_depth: Option<usize>,
    ) -> GraphStoreResult<Vec<(EdgeRecord, NodeRecord)>> {
        self.store
            .epistemic_neighbors(node_id, epistemic_types, min_confidence, max_depth)
    }

    fn invoke_app_affordance(
        &mut self,
        invocation: AppAffordanceInvocation,
    ) -> Result<Value, McpError> {
        let endpoint = theorem_app_affordance_grpc_endpoint()?;
        let request_json = serde_json::to_string(&invocation.request)
            .map_err(|error| McpError::invalid_params(format!("request JSON failed: {error}")))?;
        let request = InvokeAppAffordanceGrpcRequest {
            tenant_id: invocation.tenant_id,
            affordance_id: invocation.affordance_id,
            actor: invocation.actor,
            request_json,
            dry_run: invocation.dry_run,
            confirmed: invocation.confirmed,
            timeout_ms: invocation.timeout_ms,
        };
        let response = invoke_app_affordance_grpc_blocking(endpoint, request)?;
        Ok(app_affordance_response_json(response))
    }

    fn dispatch_handoff(&self, dispatch: HandoffDispatch) -> Result<(), McpError> {
        let token = std::env::var("THEOREM_HANDOFF_GITHUB_TOKEN")
            .or_else(|_| std::env::var("GITHUB_TOKEN"))
            .map_err(|_| {
                McpError::internal(
                    "session handoff dispatch requires THEOREM_HANDOFF_GITHUB_TOKEN or GITHUB_TOKEN",
                )
            })?;
        // Run the GitHub repository_dispatch POST on a dedicated thread with its own
        // current-thread runtime, so the blocking call never nests inside the server's async
        // runtime regardless of how the handler is scheduled.
        std::thread::spawn(move || -> Result<(), McpError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    McpError::internal(format!("dispatch runtime build failed: {error}"))
                })?;
            runtime.block_on(async move {
                let url = format!(
                    "https://api.github.com/repos/{}/{}/dispatches",
                    dispatch.owner, dispatch.repo
                );
                let body = json!({
                    "event_type": dispatch.event_type,
                    "client_payload": {
                        "intent": dispatch.intent,
                        "branch": dispatch.branch,
                    },
                });
                let response = reqwest::Client::new()
                    .post(&url)
                    .header("Authorization", format!("Bearer {token}"))
                    .header("Accept", "application/vnd.github+json")
                    .header("User-Agent", "theorem-harness")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .json(&body)
                    .send()
                    .await
                    .map_err(|error| {
                        McpError::internal(format!("dispatch POST failed: {error}"))
                    })?;
                let status = response.status();
                if status.is_success() {
                    Ok(())
                } else {
                    let detail = response.text().await.unwrap_or_default();
                    Err(McpError::internal(format!(
                        "dispatch rejected ({status}): {detail}"
                    )))
                }
            })
        })
        .join()
        .map_err(|_| McpError::internal("dispatch thread panicked"))?
    }

    fn invoke_datawave_ingest(
        &mut self,
        tenant: &str,
        arguments: &Value,
        operation: &str,
    ) -> Result<Value, McpError> {
        match &mut self.store {
            TenantGraphStore::RedCore(executor) => {
                let mut writer = executor
                    .writer
                    .lock()
                    .map_err(|_| McpError::internal("RedCore writer lock poisoned"))?;
                let mut registry = PluginRegistry::new();
                registry.register(DatawaveIngestPlugin);
                let command = match operation {
                    "describe" => "ingest.describe",
                    "record" => "ingest.record",
                    "batch" => "ingest.batch",
                    "lookup" => "ingest.lookup",
                    "intersect" => "ingest.intersect",
                    _ => "ingest.describe",
                };
                let output = registry
                    .execute(&mut writer, tenant, command, arguments.clone())
                    .map_err(|error| {
                        let message = format!("{}: {}", error.code, error.message);
                        if error.code.starts_with("invalid_") || error.code.starts_with("missing_")
                        {
                            McpError::invalid_params(message)
                        } else {
                            McpError::internal(message)
                        }
                    })?;
                Ok(json!({
                    "tenant": output.tenant_id,
                    "operation": operation,
                    "command": output.command,
                    "writes_graph": output.writes_graph,
                    "affordance_id": format!("rustyred_thg_datawave.{operation}"),
                    "engine": "rustyred_thg_datawave",
                    "capability_pack": INGEST_CAPABILITY_PACK,
                    "result": output.result,
                }))
            }
            TenantGraphStore::Redis(_) => Err(McpError::internal(
                "datawave ingest requires the in-process RedCore graph backend",
            )),
        }
    }

    fn invoke_reverse_engineer_compose(
        &mut self,
        tenant: &str,
        arguments: &Value,
    ) -> Result<Value, McpError> {
        match &mut self.store {
            TenantGraphStore::RedCore(executor) => {
                let mut writer = executor
                    .writer
                    .lock()
                    .map_err(|_| McpError::internal("RedCore writer lock poisoned"))?;
                redcore_reverse_engineer_compose_payload(&mut writer, tenant, arguments)
            }
            TenantGraphStore::Redis(_) => Err(McpError::internal(
                "reverse_engineer_compose requires an in-process RedCore graph backend",
            )),
        }
    }
}

fn is_memory_fulltext_label(label: &str) -> bool {
    MEMORY_FULLTEXT_LABELS.contains(&label)
}

fn theorem_app_affordance_grpc_endpoint() -> Result<String, McpError> {
    for key in [
        "THEOREM_APP_AFFORDANCE_GRPC_URL",
        "THEOREM_GRPC_URL",
        "THEOREM_SEARCH_URL",
        "THESEUS_BRIDGE_URL",
    ] {
        if let Ok(raw) = std::env::var(key) {
            let trimmed = raw.trim().trim_end_matches('/');
            if !trimmed.is_empty() {
                return Ok(normalize_grpc_endpoint(trimmed));
            }
        }
    }
    Err(McpError::internal(
        "code_search app affordance gRPC endpoint is not configured; set THEOREM_APP_AFFORDANCE_GRPC_URL or THEOREM_GRPC_URL",
    ))
}

fn normalize_grpc_endpoint(raw: &str) -> String {
    if raw.contains("://") {
        raw.to_string()
    } else {
        format!("http://{raw}")
    }
}

fn invoke_app_affordance_grpc_blocking(
    endpoint: String,
    request: InvokeAppAffordanceGrpcRequest,
) -> Result<InvokeAppAffordanceGrpcResponse, McpError> {
    std::thread::spawn(
        move || -> Result<InvokeAppAffordanceGrpcResponse, McpError> {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    McpError::internal(format!("app affordance runtime build failed: {error}"))
                })?;
            runtime.block_on(invoke_app_affordance_grpc(endpoint, request))
        },
    )
    .join()
    .map_err(|_| McpError::internal("app affordance gRPC thread panicked"))?
}

async fn invoke_app_affordance_grpc(
    endpoint: String,
    request: InvokeAppAffordanceGrpcRequest,
) -> Result<InvokeAppAffordanceGrpcResponse, McpError> {
    let timeout_ms = theorem_grpc_timeout_ms(&request.affordance_id, request.timeout_ms);
    let timeout = std::time::Duration::from_millis(timeout_ms);
    let channel = tonic::transport::Channel::from_shared(endpoint.clone())
        .map_err(|error| {
            McpError::internal(format!(
                "invalid app affordance gRPC endpoint `{endpoint}`: {error}"
            ))
        })?
        .connect_timeout(timeout)
        .timeout(timeout)
        .connect()
        .await
        .map_err(|error| {
            McpError::internal(format!(
                "app affordance gRPC connection failed for `{endpoint}`: {error}"
            ))
        })?;
    let mut client = tonic::client::Grpc::new(channel);
    client.ready().await.map_err(|error| {
        McpError::internal(format!("app affordance gRPC client not ready: {error}"))
    })?;
    let path =
        http::uri::PathAndQuery::from_static("/theorem_grpc.AppAffordanceService/InvokeAffordance");
    let response = client
        .unary(
            tonic::Request::new(request),
            path,
            tonic::codec::ProstCodec::default(),
        )
        .await
        .map_err(|error| McpError::internal(format!("app affordance gRPC call failed: {error}")))?;
    Ok(response.into_inner())
}

fn app_affordance_response_json(response: InvokeAppAffordanceGrpcResponse) -> Value {
    let receipt = parse_json_or_raw(&response.receipt_json);
    let output = parse_json_or_raw(&response.output_json);
    json!({
        "tenant_id": response.tenant_id,
        "affordance_id": response.affordance_id,
        "server_id": response.server_id,
        "tool_name": response.tool_name,
        "status": response.status,
        "executed": response.executed,
        "receipt_hash": response.receipt_hash,
        "receipt_json": response.receipt_json,
        "receipt": receipt,
        "output_json": response.output_json,
        "output": output,
        "error_code": response.error_code,
        "message": response.message,
        "elapsed_ms": response.elapsed_ms,
    })
}

fn parse_json_or_raw(raw: &str) -> Value {
    if raw.trim().is_empty() {
        Value::Null
    } else {
        serde_json::from_str(raw).unwrap_or_else(|_| json!(raw))
    }
}

impl rustyred_thg_adapters::AdapterGraphStore for TenantGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        TenantGraphStore::upsert_node(self, node)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        TenantGraphStore::upsert_edge(self, edge)
    }

    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        TenantGraphStore::commit_batch(self, batch)
    }

    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        TenantGraphStore::get_node(self, id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        TenantGraphStore::get_edge(self, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        TenantGraphStore::query_nodes(self, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        TenantGraphStore::neighbors(self, query)
    }

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        TenantGraphStore::snapshot(self)
    }

    fn stats(&self) -> GraphStoreResult<GraphStats> {
        TenantGraphStore::stats(self)
    }
}

impl AffordanceGraphStore for TenantGraphStore {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        TenantGraphStore::upsert_node(self, node)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        TenantGraphStore::upsert_edge(self, edge)
    }

    fn commit_batch(&mut self, batch: GraphMutationBatch) -> GraphStoreResult<GraphTransaction> {
        TenantGraphStore::commit_batch(self, batch)
    }

    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        TenantGraphStore::get_node(self, id)
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        TenantGraphStore::get_edge(self, id)
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        TenantGraphStore::query_nodes(self, query)
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        TenantGraphStore::neighbors(self, query)
    }

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        TenantGraphStore::graph_snapshot(self)
    }
}

impl McpGraphProvider for AppState {
    type Backend = ProductMcpBackend;

    fn backend_for_tenant(&self, tenant: &str) -> Result<Self::Backend, McpError> {
        let store = self
            .tenant_graph_store(tenant)
            .map_err(|error| McpError::internal(error.message))?;
        Ok(ProductMcpBackend {
            state: self.clone(),
            tenant_id: tenant.to_string(),
            store,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::config::{Config, StorageMode};
    use crate::tenant_router::{tenant_data_dir, tenant_key_segment, TenantId};

    use super::{
        app_affordance_response_json, normalize_grpc_endpoint, now_millis, AppState,
        InvokeAppAffordanceGrpcResponse, RedCoreTenantExecutor, TenantEngineState,
        TenantGraphStore,
    };
    use rustyred_thg_core::{
        EdgeRecord, GraphMutation, GraphMutationBatch, NeighborQuery, NodeQuery, NodeRecord,
        RedCoreDurability, RedCoreGraphStore, RedCoreOptions,
    };
    use serde_json::json;
    use std::sync::{mpsc, Arc, Barrier};
    use std::thread;
    use std::time::Duration;

    #[test]
    fn graph_hooks_refresh_executor_read_snapshot() {
        use rustyred_thg_core::{
            HookContext, HookHandler, HookOutcome, HookRegistration, MutationEvent, MutationKind,
            MutationMatcher,
        };

        fn coalesce(_event: &MutationEvent) -> Option<String> {
            Some("derive".to_string())
        }
        // On a `Trigger` upsert, write a derived node through the writer.
        let handler: HookHandler = Arc::new(|ctx: &mut HookContext, _events: &[MutationEvent]| {
            ctx.store.upsert_node(NodeRecord::new(
                "derived:1",
                ["Derived"],
                json!({ "by": "hook" }),
            ))?;
            Ok(HookOutcome::Done)
        });
        let reg = HookRegistration::new(
            "test.derive",
            MutationMatcher::any()
                .with_kinds([MutationKind::NodeUpserted])
                .with_labels(["Trigger"]),
            coalesce,
            handler,
        );

        let executor =
            Arc::new(RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap());
        executor.enable_graph_hooks(vec![reg], "tenant-x").unwrap();

        executor
            .upsert_node(NodeRecord::new("t1", ["Trigger"], json!({})))
            .unwrap();
        assert!(executor.quiesce_hooks(Duration::from_secs(10)));

        // Visible through the executor's READ snapshot, proving run_hook_batch
        // refreshed the committed mirror after the hook wrote via the writer.
        assert!(
            executor.get_node("derived:1").unwrap().is_some(),
            "hook-derived node visible via the executor read snapshot"
        );
    }

    // SPEC-2 acceptance 2 + 5: a projected-node write delivers a projected Item
    // delta on the changefeed bus (shaped by the same projection the query uses),
    // and the publishing hook obeys the hook contract (no graph writes, fail-open
    // send, runs off the writer's critical path through the dispatcher).
    #[test]
    fn item_changefeed_publishes_a_delta_for_a_projected_write() {
        // Subscribe BEFORE the write: a broadcast receiver only sees later sends.
        let mut rx = crate::items_changefeed::subscribe();
        let executor =
            Arc::new(RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap());
        executor
            .enable_graph_hooks(
                vec![crate::items_changefeed::changefeed_registration()],
                "tenant-cf",
            )
            .unwrap();

        executor
            .upsert_node(NodeRecord::new(
                "run-cf::task-9",
                ["TaskNode"],
                json!({ "goal": "watch me appear", "created_at_ms": 1, "updated_at_ms": 1 }),
            ))
            .unwrap();
        assert!(executor.quiesce_hooks(Duration::from_secs(10)));

        // Drain the shared bus and find our delta (parallel tests may interleave).
        let mut found = None;
        loop {
            match rx.try_recv() {
                Ok(delta) => {
                    if delta.id == "run-cf::task-9" {
                        found = Some(delta);
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
        let delta = found.expect("changefeed delivered a delta for the projected task write");
        assert_eq!(delta.tenant, "tenant-cf");
        let item = delta
            .item
            .expect("an upsert delta carries the projected item");
        assert_eq!(item.kind, "task");
        assert_eq!(item.title, "watch me appear");
    }

    #[test]
    fn app_affordance_endpoint_normalization_adds_scheme() {
        assert_eq!(
            normalize_grpc_endpoint("theorem-grpc.railway.internal:50071"),
            "http://theorem-grpc.railway.internal:50071"
        );
        assert_eq!(
            normalize_grpc_endpoint("http://127.0.0.1:50071"),
            "http://127.0.0.1:50071"
        );
    }

    #[test]
    fn app_affordance_timeout_budget_extends_code_ingest_deadline_only() {
        assert_eq!(
            rustyred_thg_affordances::theorem_grpc_timeout_ms("theorem_grpc.code_search.ingest", 0),
            rustyred_thg_affordances::THEOREM_GRPC_CODE_INGEST_TIMEOUT_MS
        );
        assert_eq!(
            rustyred_thg_affordances::theorem_grpc_timeout_ms(
                "theorem_grpc.code_search.ingest",
                180_000
            ),
            180_000
        );
        assert_eq!(
            rustyred_thg_affordances::theorem_grpc_timeout_ms(
                "theorem_grpc.observability.read_trace",
                180_000
            ),
            rustyred_thg_affordances::THEOREM_GRPC_TIMEOUT_MS
        );
    }

    #[test]
    fn app_affordance_response_json_parses_receipt_and_output() {
        let response = InvokeAppAffordanceGrpcResponse {
            tenant_id: "theorem".to_string(),
            affordance_id: "theorem_grpc.code_search.search".to_string(),
            server_id: "theorem_grpc".to_string(),
            tool_name: "code_search.search".to_string(),
            status: "ok".to_string(),
            executed: true,
            receipt_hash: "sha256:test".to_string(),
            receipt_json: r#"{"kind":"receipt"}"#.to_string(),
            output_json: r#"{"matches":[{"symbol":"native_code_search"}]}"#.to_string(),
            error_code: String::new(),
            message: "ok".to_string(),
            elapsed_ms: 7,
        };

        let payload = app_affordance_response_json(response);

        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["receipt"]["kind"], "receipt");
        assert_eq!(
            payload["output"]["matches"][0]["symbol"],
            "native_code_search"
        );
        assert_eq!(payload["receipt_json"], r#"{"kind":"receipt"}"#);
    }

    #[test]
    fn tenant_state_keys_use_graph_store_tenant_normalization() {
        let state = AppState::new(Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Redis,
            data_dir: "data/rusty-red".to_string(),
            require_volume: false,
            volume_available: false,
            durability: RedCoreDurability::AofEverysec,
            snapshot_interval_writes: 1_000,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "snapshot".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        });

        assert_eq!(
            state.tenant_state_key("Tenant.One!"),
            format!(
                "rusty-red:{}:state:v1",
                tenant_key_segment(&TenantId::new("Tenant.One!").unwrap())
            )
        );
    }

    #[test]
    fn instant_kg_snapshot_rejects_legacy_redis_mode() {
        let mut config = memory_config();
        config.storage_mode = StorageMode::Redis;
        config.redis_url = "redis://127.0.0.1:6379".to_string();

        let state = AppState::new(config);
        let store = state
            .tenant_graph_store("tenant-kg")
            .expect("Redis client URL should parse without connecting");
        let err = store
            .graph_snapshot()
            .expect_err("Redis mode must be diagnostic");

        assert_eq!(err.code, "legacy_redis_instant_kg_unsupported");
        assert!(err.message.contains("RUSTY_RED_MODE=embedded"));

        let mcp = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &state.mcp_config(),
            json!({
                "jsonrpc": "2.0",
                "id": "instant-kg-status",
                "method": "tools/call",
                "params": {
                    "name": "harness_kg_status",
                    "arguments": { "tenant": "tenant-kg" }
                }
            }),
        );
        assert_eq!(
            mcp["error"]["data"]["code"],
            "legacy_redis_instant_kg_unsupported"
        );
    }

    #[test]
    fn embedded_graph_store_reopens_from_configured_data_dir_without_redis() {
        let data_dir = unique_test_dir("rusty-red-product-redcore");
        let config = Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Embedded,
            data_dir: data_dir.display().to_string(),
            require_volume: false,
            volume_available: false,
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 100,
            strict_acid: true,
            concurrency: "single_writer".to_string(),
            txn_isolation: "serializable".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "not-a-redis-url".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        };
        {
            let state = AppState::new(config.clone());
            state.store_ready().unwrap();
            let mut store = state.tenant_graph_store("Tenant.One!").unwrap();
            store
                .upsert_node(NodeRecord::new(
                    "node:embedded",
                    ["Embedded"],
                    json!({ "mode": "redcore" }),
                ))
                .unwrap();
        }

        let state = AppState::new(config);
        let store = state.tenant_graph_store("Tenant.One!").unwrap();
        assert_eq!(
            store.get_node("node:embedded").unwrap().unwrap().labels,
            vec!["Embedded".to_string()]
        );

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn embedded_front_door_keeps_historical_tenant_colliders_separate_after_restart() {
        let data_dir = unique_test_dir("rusty-red-tenant-router");
        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Embedded;
        config.data_dir = data_dir.display().to_string();
        config.durability = RedCoreDurability::AofAlways;
        config.snapshot_interval_writes = 100;

        let tenant_a = TenantId::new("acme/prod").unwrap();
        let tenant_b = TenantId::new("acme.prod").unwrap();
        assert_ne!(
            tenant_data_dir(&data_dir, &tenant_a),
            tenant_data_dir(&data_dir, &tenant_b)
        );

        {
            let state = AppState::new(config.clone());
            let mut store = state.tenant_graph_store(tenant_a.as_str()).unwrap();
            store
                .upsert_node(NodeRecord::new(
                    "node:payload",
                    ["Payload"],
                    json!({ "body": "byte-identical" }),
                ))
                .unwrap();
        }

        let state = AppState::new(config);
        let tenant_a_store = state.tenant_graph_store(tenant_a.as_str()).unwrap();
        let tenant_b_store = state.tenant_graph_store(tenant_b.as_str()).unwrap();

        assert_eq!(
            tenant_a_store
                .get_node("node:payload")
                .unwrap()
                .unwrap()
                .properties["body"],
            json!("byte-identical")
        );
        assert!(
            tenant_b_store.get_node("node:payload").unwrap().is_none(),
            "historically colliding tenant must open an empty separate graph"
        );

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn embedded_tenant_lifecycle_cold_hot_cold_reopens_on_request() {
        let data_dir = unique_test_dir("rusty-red-tenant-lifecycle");
        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Embedded;
        config.data_dir = data_dir.display().to_string();
        config.durability = RedCoreDurability::AofAlways;
        config.snapshot_interval_writes = 100;
        config.tenant_idle_ms = 1;
        config.tenant_warm_pool_size = 0;
        let state = AppState::new(config);

        assert_eq!(state.tenant_engine_state("tenant-life").unwrap(), None);
        {
            let mut store = state.tenant_graph_store("tenant-life").unwrap();
            store
                .upsert_node(NodeRecord::new(
                    "node:life",
                    ["Lifecycle"],
                    json!({ "body": "survives cold suspend" }),
                ))
                .unwrap();
            assert_eq!(
                state.tenant_engine_state("tenant-life").unwrap(),
                Some(TenantEngineState::Hot)
            );
            assert_eq!(state.iter_redcore_tenants().unwrap().len(), 1);
        }

        let sweep = state
            .sweep_idle_tenant_engines_at(now_millis().saturating_add(10))
            .unwrap();
        assert_eq!(sweep.cooled_to_cold, 1);
        assert_eq!(
            state.tenant_engine_state("tenant-life").unwrap(),
            Some(TenantEngineState::Cold)
        );
        assert_eq!(state.iter_redcore_tenants().unwrap().len(), 0);
        let cold_report = state
            .tenant_engine_reports()
            .unwrap()
            .into_iter()
            .find(|report| report.tenant == "tenant-life")
            .unwrap();
        assert_eq!(cold_report.resident_memory_bytes, 0);

        let store = state.tenant_graph_store("tenant-life").unwrap();
        assert_eq!(
            store.get_node("node:life").unwrap().unwrap().properties["body"],
            json!("survives cold suspend")
        );
        assert_eq!(
            state.tenant_engine_state("tenant-life").unwrap(),
            Some(TenantEngineState::Hot)
        );

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn embedded_tenant_lifecycle_respects_warm_pool_limit() {
        let data_dir = unique_test_dir("rusty-red-tenant-warm-pool");
        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Embedded;
        config.data_dir = data_dir.display().to_string();
        config.durability = RedCoreDurability::AofAlways;
        config.snapshot_interval_writes = 100;
        config.tenant_idle_ms = 1;
        config.tenant_warm_pool_size = 1;
        let state = AppState::new(config);

        {
            let mut first = state.tenant_graph_store("tenant-warm-a").unwrap();
            first
                .upsert_node(NodeRecord::new("node:a", ["Warm"], json!({})))
                .unwrap();
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
        {
            let mut second = state.tenant_graph_store("tenant-warm-b").unwrap();
            second
                .upsert_node(NodeRecord::new("node:b", ["Warm"], json!({})))
                .unwrap();
        }

        let sweep = state
            .sweep_idle_tenant_engines_at(now_millis().saturating_add(10))
            .unwrap();
        assert_eq!(sweep.cooled_to_warm, 1);
        assert_eq!(sweep.cooled_to_cold, 1);
        assert_eq!(state.iter_redcore_tenants().unwrap().len(), 1);
        let reports = state.tenant_engine_reports().unwrap();
        assert_eq!(
            reports
                .iter()
                .filter(|report| report.state == TenantEngineState::Warm)
                .count(),
            1
        );
        assert_eq!(
            reports
                .iter()
                .filter(|report| report.state == TenantEngineState::Cold)
                .count(),
            1
        );

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn tenant_engine_prometheus_exposes_per_tenant_state() {
        let data_dir = unique_test_dir("rusty-red-tenant-metrics");
        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Embedded;
        config.data_dir = data_dir.display().to_string();
        config.durability = RedCoreDurability::AofAlways;
        config.snapshot_interval_writes = 100;
        let state = AppState::new(config);

        let _store = state.tenant_graph_store("tenant-metrics").unwrap();
        let metrics = state.render_tenant_engine_prometheus().unwrap();

        assert!(metrics.contains(
            "rustyred_thg_tenant_engine_state{tenant=\"tenant-metrics\",state=\"hot\"} 2"
        ));
        assert!(metrics.contains("rustyred_thg_tenant_engine_resident_bytes"));

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn tenant_residency_soak_tracks_active_working_sets_not_full_graphs() {
        let data_dir = unique_test_dir("rusty-red-tenant-residency-soak");
        let tenants = [
            "tenant-soak-a",
            "tenant-soak-b",
            "tenant-soak-c",
            "tenant-soak-d",
        ];
        let nodes_per_tenant = 64;
        let tenant_budget = 16 * 1024;
        let options = RedCoreOptions {
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 10_000,
            strict_acid: false,
        };
        let mut full_materialization_baseline = 0_usize;

        for tenant in tenants {
            let tenant_id = TenantId::new(tenant).unwrap();
            let tenant_dir = tenant_data_dir(&data_dir, &tenant_id);
            let mut store = RedCoreGraphStore::open(&tenant_dir, options.clone()).unwrap();
            for index in 0..nodes_per_tenant {
                store
                    .upsert_node(NodeRecord::new(
                        format!("node:{tenant}:{index:03}"),
                        ["SoakFixture"],
                        json!({
                            "tenant": tenant,
                            "rank": index,
                            "title": format!("fixture row {index}"),
                            "payload": {
                                "body": format!("tenant={tenant};index={index};{}", "x".repeat(4096)),
                            },
                        }),
                    ))
                    .unwrap();
            }
            full_materialization_baseline =
                full_materialization_baseline.saturating_add(store.stats().unwrap().memory_bytes);
            store.snapshot_now().unwrap();
        }

        let mut config = Config::default_for_tests();
        config.storage_mode = StorageMode::Embedded;
        config.data_dir = data_dir.display().to_string();
        config.durability = RedCoreDurability::AofAlways;
        config.snapshot_interval_writes = 10_000;
        config.tenant_memory_quota_bytes = tenant_budget;
        config.tenant_idle_ms = 60_000;
        config.tenant_warm_pool_size = 0;
        let idle_ms = config.tenant_idle_ms;
        let state = AppState::new(config);
        let mut warm_floor_sum = 0_usize;

        for tenant in tenants {
            let store = state.tenant_graph_store(tenant).unwrap();
            let warm_floor = store.stats().unwrap().memory_bytes;
            warm_floor_sum = warm_floor_sum.saturating_add(warm_floor);
            assert!(
                warm_floor < full_materialization_baseline / tenants.len(),
                "opening {tenant} should stay below a full-materialized tenant baseline"
            );

            let nodes = store
                .query_nodes(NodeQuery::label("SoakFixture").with_limit(nodes_per_tenant))
                .unwrap();
            assert_eq!(nodes.len(), nodes_per_tenant);
            drop(nodes);

            if let TenantGraphStore::RedCore(executor) = &store {
                let hot_cache = executor.hot_cache_diagnostics();
                assert!(
                    hot_cache["resident_bytes"].as_u64().unwrap()
                        <= hot_cache["budget_bytes"].as_u64().unwrap(),
                    "tenant {tenant} hot cache exceeded budget after scan: {hot_cache:?}"
                );
                assert!(
                    hot_cache["admissions"].as_u64().unwrap() >= nodes_per_tenant as u64,
                    "tenant {tenant} scan should admit archived records: {hot_cache:?}"
                );
            }
            drop(store);
        }

        let reports = state.tenant_engine_reports().unwrap();
        let active_reports = reports
            .iter()
            .filter(|report| report.state == TenantEngineState::Hot)
            .collect::<Vec<_>>();
        assert_eq!(active_reports.len(), tenants.len());
        let active_resident = active_reports
            .iter()
            .map(|report| report.resident_memory_bytes)
            .sum::<usize>();
        assert!(
            active_resident <= warm_floor_sum.saturating_add(tenants.len() * tenant_budget),
            "aggregate resident bytes should track warm floors plus active working sets: active={active_resident}, warm_floor={warm_floor_sum}, budget={tenant_budget}"
        );
        assert!(
            active_resident < full_materialization_baseline / 2,
            "aggregate resident bytes should stay far below full materialization: active={active_resident}, baseline={full_materialization_baseline}"
        );

        let sweep = state
            .sweep_idle_tenant_engines_at(now_millis().saturating_add(idle_ms + 1))
            .unwrap();
        assert_eq!(sweep.cooled_to_cold, tenants.len());
        assert_eq!(state.iter_redcore_tenants().unwrap().len(), 0);
        let cold_resident = state
            .tenant_engine_reports()
            .unwrap()
            .iter()
            .map(|report| report.resident_memory_bytes)
            .sum::<usize>();
        assert_eq!(cold_resident, 0);

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn embedded_readiness_rejects_missing_required_volume() {
        let state = AppState::new(Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Embedded,
            data_dir: "data/rusty-red".to_string(),
            require_volume: true,
            volume_available: false,
            durability: RedCoreDurability::AofEverysec,
            snapshot_interval_writes: 1_000,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "snapshot".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "not-a-redis-url".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        });

        let error = state.store_ready().unwrap_err();

        assert_eq!(error.code, "store_internal_error");
        assert!(error.message.contains("REQUIRE_VOLUME"));
    }

    #[test]
    fn memory_readiness_ignores_volume_requirement_and_reports_no_durability() {
        let state = AppState::new(Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Memory,
            data_dir: "data/rusty-red".to_string(),
            require_volume: true,
            volume_available: false,
            durability: RedCoreDurability::AofEverysec,
            snapshot_interval_writes: 1_000,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "snapshot".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "not-a-redis-url".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        });

        let report = state.store_ready().unwrap();

        assert_eq!(report.mode, "memory");
        assert_eq!(report.durability, "none");
        assert!(!report.require_volume);
    }

    #[test]
    fn redcore_executor_serializes_concurrent_writes_with_monotonic_txn_ids() {
        let executor =
            Arc::new(RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap());
        let start = Arc::new(Barrier::new(9));
        let handles = (0..8)
            .map(|idx| {
                let executor = executor.clone();
                let start = start.clone();
                thread::spawn(move || {
                    start.wait();
                    executor
                        .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                            NodeRecord::new(
                                format!("node:{idx}"),
                                ["Concurrent"],
                                json!({ "idx": idx }),
                            ),
                        )]))
                        .unwrap()
                        .txn_id
                })
            })
            .collect::<Vec<_>>();

        start.wait();
        let mut txn_ids = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        txn_ids.sort_unstable();

        assert_eq!(txn_ids, (1_u64..=8).collect::<Vec<_>>());
        assert_eq!(executor.stats().unwrap().nodes_total, 8);
        assert_eq!(executor.read_barrier().unwrap(), 8);
    }

    #[test]
    fn redcore_executor_publishes_only_successful_commits() {
        let executor = RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap();
        let error = executor
            .commit_batch(GraphMutationBatch::new([
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "node:a",
                    ["File"],
                    json!({ "path": "src/lib.rs" }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:missing",
                    "node:a",
                    "IMPORTS",
                    "node:missing",
                    json!({}),
                )),
            ]))
            .unwrap_err();

        assert_eq!(error.code, "missing_graph_endpoint");
        assert!(executor.get_node("node:a").unwrap().is_none());
        assert_eq!(executor.stats().unwrap().version, 0);
        assert_eq!(executor.read_barrier().unwrap(), 0);

        let transaction = executor
            .commit_batch(GraphMutationBatch::new([
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "node:a",
                    ["File"],
                    json!({ "path": "src/lib.rs" }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "node:b",
                    ["File"],
                    json!({ "path": "src/main.rs" }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:ab",
                    "node:a",
                    "IMPORTS",
                    "node:b",
                    json!({}),
                )),
            ]))
            .unwrap();

        assert_eq!(executor.read_barrier().unwrap(), transaction.txn_id);
        assert_eq!(
            executor.neighbors(NeighborQuery::out("node:a")).unwrap()[0].node_id,
            "node:b"
        );
        assert_eq!(executor.verify().unwrap().ok, true);
    }

    #[test]
    fn redcore_executor_vector_search_uses_writer_index() {
        let executor = RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap();
        executor
            .designate_vector_property("CodeSymbol", "semantic_vec", 3)
            .unwrap();
        executor
            .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                NodeRecord::new(
                    "code:symbol:format_transcript",
                    ["CodeSymbol"],
                    json!({
                        "name": "format_transcript",
                        "semantic_vec": [1.0, 0.0, 0.0],
                    }),
                ),
            )]))
            .unwrap();

        assert_eq!(executor.vector_designations().unwrap().len(), 1);
        let results = executor
            .vector_search(Some("CodeSymbol"), "semantic_vec", &[1.0, 0.0, 0.0], 1)
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "code:symbol:format_transcript");
    }

    #[test]
    fn redcore_snapshot_reads_do_not_wait_for_writer_lock() {
        let executor =
            Arc::new(RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 0).unwrap());
        executor
            .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                NodeRecord::new("node:committed", ["File"], json!({ "path": "src/lib.rs" })),
            )]))
            .unwrap();
        let _writer_guard = executor.lock_writer().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = executor.clone();

        thread::spawn(move || {
            let node = reader.get_node("node:committed").unwrap();
            tx.send(node.map(|node| node.id)).unwrap();
        });

        assert_eq!(
            rx.recv_timeout(Duration::from_millis(250)).unwrap(),
            Some("node:committed".to_string())
        );
    }

    #[test]
    fn redcore_executor_enforces_tenant_memory_quota_on_commit() {
        let executor = RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 1).unwrap();
        let error = executor
            .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                NodeRecord::new("node:oversize", ["File"], json!({ "path": "src/lib.rs" })),
            )]))
            .unwrap_err();

        assert_eq!(error.code, "tenant_memory_quota_exceeded");
    }

    #[test]
    fn redcore_executor_includes_tenant_memory_quota_in_stats() {
        let executor = RedCoreTenantExecutor::new(RedCoreGraphStore::memory(), 128).unwrap();
        let stats = executor.stats().unwrap();

        assert_eq!(stats.memory_quota_bytes, 128);
    }

    #[test]
    fn redcore_executor_uses_tenant_quota_as_archive_hot_cache_budget() {
        let data_dir = unique_test_dir("redcore-executor-hot-cache-budget");
        let options = RedCoreOptions {
            durability: RedCoreDurability::SnapshotOnly,
            snapshot_interval_writes: 1,
            strict_acid: false,
        };
        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            for index in 0..12 {
                store
                    .upsert_node(NodeRecord::new(
                        format!("node:{index:02}"),
                        ["CacheFixture"],
                        json!({
                            "body": format!("executor-cache-row-{index:02}-{}", "x".repeat(96)),
                        }),
                    ))
                    .unwrap();
            }
        }

        let executor =
            RedCoreTenantExecutor::new(RedCoreGraphStore::open(&data_dir, options).unwrap(), 1024)
                .unwrap();
        let initial = executor.hot_cache_diagnostics();
        assert_eq!(initial["enabled"], true);
        assert_eq!(initial["budget_bytes"], 1024);
        assert_eq!(initial["resident_bytes"], 0);

        for index in 0..12 {
            let node_id = format!("node:{index:02}");
            assert!(executor.get_node(&node_id).unwrap().is_some());
        }
        let report = executor.hot_cache_diagnostics();
        assert!(
            report["resident_bytes"].as_u64().unwrap() <= report["budget_bytes"].as_u64().unwrap(),
            "hot cache report should remain under budget: {report:?}"
        );
        assert!(
            report["admissions"].as_u64().unwrap() > 0,
            "archive reads should admit records into the hot cache: {report:?}"
        );
        assert!(
            executor.stats().unwrap().memory_bytes
                >= report["resident_bytes"].as_u64().unwrap() as usize,
            "stats should include hot cache resident bytes"
        );

        executor.clear_hot_caches();
        let cleared = executor.hot_cache_diagnostics();
        assert_eq!(cleared["resident_bytes"], 0);
        assert_eq!(cleared["budget_bytes"], 1024);

        std::fs::remove_dir_all(data_dir).ok();
    }

    #[test]
    fn graph_transactions_expire_after_ttl_interval() {
        let state = AppState::new(memory_config());

        let tx_id = state.begin_graph_transaction("tenant-a").unwrap();
        let mut stale_time = super::now_millis();
        stale_time += super::GRAPH_TRANSACTION_TTL_MS + 1;
        state
            .purge_expired_graph_transactions_at(stale_time)
            .expect("graph transaction expiry check");

        let error = state
            .append_graph_transaction_mutations(
                "tenant-a",
                &tx_id,
                GraphMutationBatch::new([GraphMutation::NodeUpsert(NodeRecord::new(
                    "node:ttl",
                    ["File"],
                    json!({ "path": "src/ttl.rs" }),
                ))]),
            )
            .unwrap_err();

        assert_eq!(error.code, "store_mode_unsupported");
        assert_eq!(error.message, "graph transaction not found");
    }

    #[test]
    fn graph_transaction_wrong_tenant_commit_preserves_staged_work() {
        let state = AppState::new(memory_config());
        let tx_id = state.begin_graph_transaction("tenant-a").unwrap();
        state
            .append_graph_transaction_mutations(
                "tenant-a",
                &tx_id,
                GraphMutationBatch::new([GraphMutation::NodeUpsert(NodeRecord::new(
                    "node:tenant-a",
                    ["File"],
                    json!({ "path": "src/lib.rs" }),
                ))]),
            )
            .unwrap();

        let error = state
            .commit_graph_transaction("tenant-b", &tx_id)
            .unwrap_err();
        assert_eq!(error.code, "store_mode_unsupported");
        assert_eq!(error.message, "graph transaction tenant mismatch");

        let transaction = state.commit_graph_transaction("tenant-a", &tx_id).unwrap();
        assert_eq!(transaction.writes.len(), 1);
        let store = state.tenant_graph_store("tenant-a").unwrap();
        assert!(store.get_node("node:tenant-a").unwrap().is_some());
    }

    #[test]
    fn graph_transaction_wrong_tenant_rollback_preserves_staged_work() {
        let state = AppState::new(memory_config());
        let tx_id = state.begin_graph_transaction("tenant-a").unwrap();

        let error = state
            .rollback_graph_transaction("tenant-b", &tx_id)
            .unwrap_err();
        assert_eq!(error.code, "store_mode_unsupported");
        assert_eq!(error.message, "graph transaction tenant mismatch");

        state
            .rollback_graph_transaction("tenant-a", &tx_id)
            .expect("owner tenant can still rollback after wrong-tenant attempt");
        let error = state
            .rollback_graph_transaction("tenant-a", &tx_id)
            .unwrap_err();
        assert_eq!(error.message, "graph transaction not found");
    }

    #[test]
    fn graph_transactions_do_not_survive_state_restart() {
        let config = memory_config();
        let tx_id = {
            let active_state = AppState::new(config.clone());
            active_state.begin_graph_transaction("tenant-a").unwrap()
        };
        let fresh_state = AppState::new(config);
        let error = fresh_state
            .commit_graph_transaction("tenant-a", &tx_id)
            .unwrap_err();

        assert_eq!(error.code, "store_mode_unsupported");
        assert_eq!(
            error.message,
            "graph transaction not found or already committed"
        );
    }

    #[test]
    fn mcp_config_carries_graphql_default_surface_flag() {
        let mut config = memory_config();
        config.mcp_graphql_default_surface = true;
        let state = AppState::new(config);

        assert!(state.mcp_config().graphql_default_surface);
    }

    #[test]
    fn product_mcp_backend_reaches_fulltext_spatial_and_bulk_tools() {
        let state = AppState::new(memory_config());
        let mut config = state.mcp_config();
        config.read_only = false;

        let bulk = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "bulk",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_bulk_nodes",
                    "arguments": {
                        "tenant": "tenant-mcp",
                        "nodes": [
                            {
                                "id": "place:a",
                                "labels": ["Place"],
                                "properties": {
                                    "body": "north campus library",
                                    "lat": 42.0,
                                    "lon": -83.0
                                }
                            }
                        ]
                    }
                }
            }),
        );
        assert_eq!(bulk["result"]["structuredContent"]["inserted"], 1);

        let fulltext_designate = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ft-designate",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_fulltext_designate",
                    "arguments": {
                        "tenant": "tenant-mcp",
                        "label": "Place",
                        "property": "body"
                    }
                }
            }),
        );
        assert_eq!(
            fulltext_designate["result"]["structuredContent"]["designated"]["property"],
            "body"
        );
        let fulltext_search = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "ft-search",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_fulltext_search",
                    "arguments": {
                        "tenant": "tenant-mcp",
                        "label": "Place",
                        "property": "body",
                        "query": "library"
                    }
                }
            }),
        );
        assert_eq!(
            fulltext_search["result"]["structuredContent"]["results"][0]["node_id"],
            "place:a"
        );

        let spatial_designate = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "sp-designate",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_spatial_designate",
                    "arguments": {
                        "tenant": "tenant-mcp",
                        "label": "Place",
                        "lat_property": "lat",
                        "lon_property": "lon"
                    }
                }
            }),
        );
        assert_eq!(
            spatial_designate["result"]["structuredContent"]["designated"]["label"],
            "Place"
        );
        let spatial_search = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "sp-radius",
                "method": "tools/call",
                "params": {
                    "name": "rustyred_thg_spatial_radius",
                    "arguments": {
                        "tenant": "tenant-mcp",
                        "label": "Place",
                        "lat_property": "lat",
                        "lon_property": "lon",
                        "lat": 42.0,
                        "lon": -83.0,
                        "radius_km": 1.0
                    }
                }
            }),
        );
        assert_eq!(
            spatial_search["result"]["structuredContent"]["node_ids"][0],
            "place:a"
        );
    }

    #[test]
    fn fulltext_search_returns_empty_for_designated_label_with_no_hits() {
        let state = AppState::new(memory_config());
        state
            .designate_fulltext_property("tenant-empty-ft", "MemoryAtom", "search_text")
            .expect("designation succeeds before nodes exist");

        let hits = state
            .fulltext_search(
                "tenant-empty-ft",
                Some("MemoryAtom"),
                "search_text",
                "jobintel",
                5,
            )
            .expect("empty designated index is a valid search");
        assert!(hits.is_empty());

        let error = state
            .fulltext_search(
                "tenant-empty-ft",
                Some("MemoryNode"),
                "search_text",
                "jobintel",
                5,
            )
            .unwrap_err();
        assert_eq!(error.code, "store_mode_unsupported");
        assert_eq!(
            error.message,
            "no matching fulltext designation; call /fulltext/designate first"
        );
    }

    // T9 (dispatch-mirror fix): a configured-but-unreachable Postgres dispatch mirror
    // must not fail `job_submit`. The board write is canonical; the mirror is best-effort.
    // Acceptance: submit completes cleanly (no error envelope, no panic), the job lands on
    // the board, and the failed mirror is recorded as `dispatch_mirrored:false` + an error.
    #[test]
    fn job_submit_survives_a_failing_dispatch_mirror() {
        let state = AppState::new(memory_config());
        let mut config = state.mcp_config();
        config.read_only = false;

        // An invalid dispatch URL: `DispatchQueue::connect` fails fast (URL parse error),
        // exercising the mirror-failure path without a live database. Capture results
        // before asserting so an assert panic cannot leak the process-global env var.
        std::env::set_var("THEOREM_DISPATCH_DATABASE_URL", "not-a-valid-postgres-url");
        let submit = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "submit",
                "method": "tools/call",
                "params": {
                    "name": "job_submit",
                    "arguments": {
                        "tenant": "tenant-jobs",
                        "title": "Mirror non-fatal job",
                        "repo": "Travis-Gilbert/Theorem",
                        "spec_inline": "make the dispatch mirror non-fatal",
                        "actor": "claude-code"
                    }
                }
            }),
        );
        let list = rustyred_thg_mcp::handle_mcp_request(
            &state,
            &config,
            json!({
                "jsonrpc": "2.0",
                "id": "list",
                "method": "tools/call",
                "params": {
                    "name": "job_list",
                    "arguments": { "tenant": "tenant-jobs" }
                }
            }),
        );
        std::env::remove_var("THEOREM_DISPATCH_DATABASE_URL");

        // The submit completed cleanly: a success envelope, not the pre-fix error envelope.
        assert!(
            submit.get("error").is_none(),
            "job_submit must not error when the dispatch mirror fails: {submit}"
        );
        let payload = &submit["result"]["structuredContent"]["result"];
        assert_eq!(
            payload["dispatch_mirrored"],
            json!(false),
            "mirror failure records dispatch_mirrored:false: {payload}"
        );
        assert!(
            payload["dispatch_mirror_error"]
                .as_str()
                .map(|message| !message.is_empty())
                .unwrap_or(false),
            "a failed mirror records a non-empty error note: {payload}"
        );
        let job_id = payload["job"]["job_id"]
            .as_str()
            .expect("the job committed to the board");
        assert!(!job_id.is_empty());

        // The job is on the board despite the mirror failure.
        assert!(list.get("error").is_none(), "job_list must succeed: {list}");
        assert!(
            list.to_string().contains(job_id),
            "submitted job {job_id} must appear on the board: {list}"
        );
    }

    fn memory_config() -> Config {
        Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Memory,
            data_dir: "data/rusty-red".to_string(),
            require_volume: false,
            volume_available: false,
            durability: RedCoreDurability::None,
            snapshot_interval_writes: 0,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "snapshot".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "not-a-redis-url".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        }
    }

    fn unique_test_dir(label: &str) -> std::path::PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{label}-{unique}"))
    }

    // ---- §P6-A pa6.1 algorithm-cache tests --------------------------------

    fn arc_cache_test_state() -> AppState {
        AppState::new(Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Memory,
            data_dir: "data/rusty-red".to_string(),
            require_volume: false,
            volume_available: false,
            durability: RedCoreDurability::None,
            snapshot_interval_writes: 0,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "snapshot".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: Default::default(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: rustyred_thg_core::HybridScoringConfig::default(),
            redis_url: "not-a-redis-url".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: true,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            mcp_graphql_default_surface: false,
            ttl_sweep_ms: 1000,
            tenant_idle_ms: 300_000,
            tenant_warm_pool_size: 0,
        })
    }

    #[test]
    fn list_edges_arc_returns_shared_instance() {
        let state = arc_cache_test_state();
        let mut store = state.tenant_graph_store("tenant-arc").unwrap();
        store
            .upsert_node(NodeRecord::new("a", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_node(NodeRecord::new("b", ["Doc"], json!({})))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new("e1", "a", "T", "b", json!({})))
            .unwrap();
        let first = store.list_edges_arc().unwrap();
        let second = store.list_edges_arc().unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "successive list_edges_arc calls at the same graph_version must share the allocation",
        );
        assert!(Arc::strong_count(&first) >= 2);
    }

    #[test]
    fn list_edges_arc_rebuilds_after_mutation() {
        let state = arc_cache_test_state();
        let mut store = state.tenant_graph_store("tenant-arc-bump").unwrap();
        store
            .upsert_node(NodeRecord::new("a", ["Doc"], json!({})))
            .unwrap();
        let first = store.list_edges_arc().unwrap();
        store
            .upsert_node(NodeRecord::new("b", ["Doc"], json!({})))
            .unwrap();
        let second = store.list_edges_arc().unwrap();
        assert!(
            !Arc::ptr_eq(&first, &second),
            "mutation should bump graph_version and invalidate the arc cache",
        );
    }
}
