use rustyred_thg_core::{
    Direction, EdgeRecord, EpistemicType, GraphStore, GraphStoreError, GraphStoreResult,
    NeighborHit, NeighborQuery, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};
use theorem_harness_core::stable_value_hash;

pub type MemoryResult<T> = Result<T, MemoryError>;

const DEFAULT_TENANT: &str = "default";
const DEFAULT_STATUS: &str = "active";
const DOCUMENT_STATUSES: &[&str] = &["active", "superseded", "archived", "deleted"];
const NODE_MEMORY_KINDS: &[&str] = &["claim", "finding"];
const DEFAULT_LIMIT: usize = 10;
const MAX_LIMIT: usize = 50;
const MAX_GRAPH_QUERY_LIMIT: usize = 10_000;
const DEFAULT_SEED_LIMIT: usize = 16;
const DEFAULT_PPR_ALPHA: f64 = 0.15;
const DEFAULT_PPR_EPSILON: f64 = 1e-6;
const DEFAULT_PPR_MAX_PUSHES: usize = 100_000;
const DEFAULT_RECENCY_HALF_LIFE_SECONDS: f64 = 0.0;
const COMMUNITY_SUMMARY_KIND: &str = "community_summary";
const COMMUNITY_SUMMARY_EDGE: &str = "MEMORY_SUMMARIZES";

pub trait MemoryGraphStore {
    fn memory_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()>;
    fn memory_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()>;
    fn memory_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn memory_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn memory_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn memory_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
    fn memory_fulltext_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &str,
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        let _ = (label, property, query, k);
        Ok(Vec::new())
    }
    fn memory_vector_search(
        &self,
        label: Option<&str>,
        property: &str,
        query: &[f32],
        k: usize,
    ) -> GraphStoreResult<Vec<(String, f32)>> {
        let _ = (label, property, query, k);
        Ok(Vec::new())
    }
}

impl<T: GraphStore> MemoryGraphStore for T {
    fn memory_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()> {
        self.upsert_node(node).map(|_| ())
    }

    fn memory_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()> {
        self.upsert_edge(edge).map(|_| ())
    }

    fn memory_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        Ok(self.get_node(id).cloned())
    }

    fn memory_get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        Ok(self.get_edge(id).cloned())
    }

    fn memory_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(self.query_nodes(query))
    }

    fn memory_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        Ok(self.neighbors(query))
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum MemoryError {
    Store(GraphStoreError),
    Serialization(String),
    Deserialization(String),
    InvalidInput { field: String, message: String },
    NotFound { kind: String, id: String },
}

impl fmt::Display for MemoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::InvalidInput { field, message } => {
                write!(f, "invalid memory input {field}: {message}")
            }
            Self::NotFound { kind, id } => write!(f, "{kind} not found: {id}"),
        }
    }
}

impl Error for MemoryError {}

impl From<GraphStoreError> for MemoryError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct MemoryWriteInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub project_slug: String,
    #[serde(default)]
    pub doc_id: String,
    #[serde(default)]
    pub node_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub memory_node_type: String,
    #[serde(default)]
    pub target_actor_id: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub fitness: Option<Value>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RecallMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default)]
    pub since: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub limit: usize,
    #[serde(default)]
    pub include_low_fitness: bool,
    #[serde(default)]
    pub include_consolidation_sources: bool,
    #[serde(default)]
    pub consume_handoffs: bool,
    #[serde(default)]
    pub query_time: String,
    #[serde(default)]
    pub overall_state: bool,
    #[serde(default)]
    pub seed_limit: usize,
    #[serde(default)]
    pub query_embedding: Vec<f32>,
    #[serde(default)]
    pub embedding_property: String,
    #[serde(default)]
    pub ppr_alpha: f64,
    #[serde(default)]
    pub ppr_epsilon: f64,
    #[serde(default)]
    pub ppr_max_pushes: usize,
    #[serde(default)]
    pub recency_half_life_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct RelateMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub seed_id: String,
    #[serde(default)]
    pub edge_types: Vec<String>,
    #[serde(default)]
    pub max_hops: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ReviseMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub doc_id: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub memory_node_type: String,
    #[serde(default)]
    pub cites_doc_ids: Vec<String>,
    #[serde(default)]
    pub derived_from_doc_ids: Vec<String>,
    #[serde(default)]
    pub contradicts_doc_ids: Vec<String>,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ArchiveMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub doc_id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub archived_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ForgetMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub deleted_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct HandoffMemoryInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub to_actor: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct EncodeMemoryInput {
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub signal: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub context: Value,
    #[serde(default)]
    pub auto_triggered: bool,
}

/// Input for `upsert_note`: the convenience write the Obsidian sync plugin calls
/// per note. A blank `doc_id` creates a new document; a known `doc_id` updates the
/// existing one in place (the stable identity round-trips, unlike `self_revise`,
/// which mints a new id). `links` is the desired set of `[[wikilink]]` targets,
/// each either a target `doc_id` (resolved) or a target note title (forward ref).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct UpsertNoteInput {
    #[serde(default)]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub project_slug: String,
    #[serde(default)]
    pub doc_id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub memory_node_type: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub outcome: String,
    #[serde(default)]
    pub signal: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub created_at: String,
}

/// Receipt for `upsert_note`, reporting how the link set was reconciled so the
/// plugin can write back resolved ids and surface unresolved forward references.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpsertNoteReceipt {
    pub action: String,
    pub document: MemoryDocumentState,
    #[serde(default)]
    pub resolved_links: Vec<String>,
    #[serde(default)]
    pub unresolved_links: Vec<String>,
    #[serde(default)]
    pub removed_links: Vec<String>,
    #[serde(default)]
    pub reconciled_back: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryDocumentState {
    pub tenant_slug: String,
    pub doc_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub project_slug: String,
    pub status: String,
    #[serde(default)]
    pub memory_node_type: String,
    #[serde(default)]
    pub target_actor_id: String,
    #[serde(default)]
    pub expires_at: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub fitness: Option<Value>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub deleted_reason: String,
    #[serde(default)]
    pub deleted_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryNodeState {
    pub tenant_slug: String,
    pub node_id: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub project_slug: String,
    pub status: String,
    #[serde(default)]
    pub metadata: Map<String, Value>,
    #[serde(default)]
    pub fitness: Option<Value>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub deleted_reason: String,
    #[serde(default)]
    pub deleted_at: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RememberMemoryReceipt {
    pub saved_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MemoryDocumentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<MemoryNodeState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecallItem {
    pub id: String,
    pub item_type: String,
    pub kind: String,
    pub title: String,
    pub content: String,
    #[serde(default)]
    pub summary: String,
    pub status: String,
    #[serde(default)]
    pub actor_id: String,
    #[serde(default)]
    pub origin_surface: String,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub updated_at: String,
    pub score: f64,
    pub provenance: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<MemoryRecallFlag>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub rank_signals: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MemoryDocumentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<MemoryNodeState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecallFlag {
    pub kind: String,
    #[serde(default)]
    pub edge_id: String,
    #[serde(default)]
    pub edge_type: String,
    #[serde(default)]
    pub related_id: String,
    #[serde(default)]
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryRelationItem {
    pub node_id: String,
    pub id: String,
    pub item_type: String,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub summary: String,
    pub edge_id: String,
    pub edge_type: String,
    pub depth: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MemoryDocumentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<MemoryNodeState>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReviseMemoryReceipt {
    pub revised: MemoryDocumentState,
    pub superseded: MemoryDocumentState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArchiveMemoryReceipt {
    pub archived: MemoryDocumentState,
    pub archive: MemoryDocumentState,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ForgetMemoryReceipt {
    pub forgotten_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MemoryDocumentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<MemoryNodeState>,
}

pub fn remember_memory<S: MemoryGraphStore>(
    store: &mut S,
    input: MemoryWriteInput,
) -> MemoryResult<RememberMemoryReceipt> {
    let kind = normalize_kind(&input.kind, "kind")?;
    if is_node_memory_kind(&kind) {
        let node = create_memory_node(store, input)?;
        Ok(RememberMemoryReceipt {
            saved_type: "node".to_string(),
            document: None,
            node: Some(node),
        })
    } else {
        let document = create_memory_document(store, input)?;
        Ok(RememberMemoryReceipt {
            saved_type: "document".to_string(),
            document: Some(document),
            node: None,
        })
    }
}

pub fn create_memory_document<S: MemoryGraphStore>(
    store: &mut S,
    input: MemoryWriteInput,
) -> MemoryResult<MemoryDocumentState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let content = require_text("content", &input.content)?;
    let kind = normalize_kind(&input.kind, "kind")?;
    let status = normalize_status(&input.status)?;
    let actor_id = input.actor_id.trim().to_string();
    let created_at = timestamp_or_now(&input.created_at);
    let doc_id = if input.doc_id.trim().is_empty() {
        stable_document_id(&tenant_slug, &kind, &input.title, &content, &created_at)
    } else {
        input.doc_id.trim().to_string()
    };
    let document = MemoryDocumentState {
        tenant_slug,
        doc_id,
        kind,
        title: input.title.trim().to_string(),
        content,
        summary: input.summary.trim().to_string(),
        tags: normalize_strings(&input.tags),
        links: normalize_strings(&input.links),
        actor_id,
        session_id: input.session_id.trim().to_string(),
        origin_surface: input.origin_surface.trim().to_string(),
        project_slug: input.project_slug.trim().to_string(),
        status,
        memory_node_type: input.memory_node_type.trim().to_string(),
        target_actor_id: input.target_actor_id.trim().to_string(),
        expires_at: input.expires_at.trim().to_string(),
        metadata: input.metadata,
        fitness: input.fitness,
        created_at: created_at.clone(),
        updated_at: created_at,
        deleted_reason: String::new(),
        deleted_at: String::new(),
    };
    persist_memory_document(store, &document)?;
    Ok(document)
}

pub fn create_memory_node<S: MemoryGraphStore>(
    store: &mut S,
    input: MemoryWriteInput,
) -> MemoryResult<MemoryNodeState> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let content = require_text("content", &input.content)?;
    let kind = normalize_kind(&input.kind, "kind")?;
    if !is_node_memory_kind(&kind) {
        return Err(MemoryError::InvalidInput {
            field: "kind".to_string(),
            message: "memory node kind must be claim or finding".to_string(),
        });
    }
    let status = normalize_status(&input.status)?;
    let created_at = timestamp_or_now(&input.created_at);
    let node_id = if input.node_id.trim().is_empty() {
        stable_memory_node_id(&tenant_slug, &kind, &input.title, &content, &created_at)
    } else {
        input.node_id.trim().to_string()
    };
    let node = MemoryNodeState {
        tenant_slug,
        node_id,
        kind,
        title: input.title.trim().to_string(),
        content,
        tags: normalize_strings(&input.tags),
        links: normalize_strings(&input.links),
        actor_id: input.actor_id.trim().to_string(),
        session_id: input.session_id.trim().to_string(),
        origin_surface: input.origin_surface.trim().to_string(),
        project_slug: input.project_slug.trim().to_string(),
        status,
        metadata: input.metadata,
        fitness: input.fitness,
        created_at: created_at.clone(),
        updated_at: created_at,
        deleted_reason: String::new(),
        deleted_at: String::new(),
    };
    persist_memory_node(store, &node)?;
    Ok(node)
}

pub fn recall_memory<S: MemoryGraphStore>(
    store: &mut S,
    input: RecallMemoryInput,
) -> MemoryResult<Vec<MemoryRecallItem>> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let query = input.query.trim().to_string();
    let kind_filter = input.kind.trim().to_lowercase();
    let surface_filter = input.surface.trim().to_string();
    let actor_filter = input.actor.trim().to_string();
    let since = input.since.trim().to_string();
    let limit = bounded_limit(input.limit);
    let query_time = timestamp_or_now(&input.query_time);
    let mut atoms = load_recall_atoms(
        store,
        &tenant_slug,
        &kind_filter,
        &surface_filter,
        &actor_filter,
        &since,
        input.include_low_fitness,
    )?;
    let mut atom_by_graph_id = atoms
        .iter()
        .map(|atom| (atom.graph_id.clone(), atom.clone()))
        .collect::<HashMap<_, _>>();

    let mut broad_query =
        kind_filter.is_empty() && (input.overall_state || is_broad_recall_query(&query));
    if broad_query {
        ensure_community_summaries(store, &tenant_slug, &query_time)?;
        atoms = load_recall_atoms(
            store,
            &tenant_slug,
            COMMUNITY_SUMMARY_KIND,
            &surface_filter,
            &actor_filter,
            &since,
            input.include_low_fitness,
        )?;
        atom_by_graph_id = atoms
            .iter()
            .map(|atom| (atom.graph_id.clone(), atom.clone()))
            .collect::<HashMap<_, _>>();
    }

    let seed_limit = bounded_seed_limit(input.seed_limit);
    let mut seeds = if broad_query {
        seed_community_summaries(&atoms, &query, seed_limit)
    } else {
        resolve_recall_seeds(store, &atoms, &query, &input, seed_limit)?
    };
    if kind_filter.is_empty()
        && !broad_query
        && seeds.is_empty()
        && !query_has_specific_anchor(&query)
    {
        broad_query = true;
        ensure_community_summaries(store, &tenant_slug, &query_time)?;
        atoms = load_recall_atoms(
            store,
            &tenant_slug,
            COMMUNITY_SUMMARY_KIND,
            &surface_filter,
            &actor_filter,
            &since,
            input.include_low_fitness,
        )?;
        atom_by_graph_id = atoms
            .iter()
            .map(|atom| (atom.graph_id.clone(), atom.clone()))
            .collect::<HashMap<_, _>>();
        seeds = seed_community_summaries(&atoms, &query, seed_limit);
    }

    let mut results = if broad_query || seeds.is_empty() {
        lexical_recall_results(&atoms, &query)
    } else {
        ranked_ppr_recall_results(
            store,
            &atom_by_graph_id,
            &seeds,
            &query,
            &query_time,
            &input,
        )?
    };

    annotate_recall_results(store, &mut results, &tenant_slug, &query_time)?;
    results.retain(|item| item.score > 0.0 || query.is_empty() || broad_query);
    results.sort_by(compare_recall_items);
    results.truncate(limit);
    bump_recalled_compound_fitness(store, &tenant_slug, &results)?;
    bump_recall_salience(store, &tenant_slug, &results, &query_time)?;

    if input.consume_handoffs && kind_filter == "handoff" {
        for item in &results {
            if item.item_type == "document" {
                if let Some(mut document) = load_memory_document(store, &tenant_slug, &item.id)? {
                    document.status = "archived".to_string();
                    document.updated_at = timestamp_or_now("");
                    document
                        .metadata
                        .insert("consumed_as_handoff".to_string(), Value::Bool(true));
                    persist_memory_document(store, &document)?;
                }
            }
        }
    }

    Ok(results)
}

pub fn relate_memory<S: MemoryGraphStore>(
    store: &S,
    input: RelateMemoryInput,
) -> MemoryResult<Vec<MemoryRelationItem>> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let seed_id = require_text("seed_id", &input.seed_id)?;
    let seed_node_id =
        resolve_memory_graph_id(store, &tenant_slug, &seed_id)?.ok_or_else(|| {
            MemoryError::NotFound {
                kind: "memory seed".to_string(),
                id: seed_id.clone(),
            }
        })?;
    let edge_filter = normalize_strings(&input.edge_types)
        .into_iter()
        .collect::<BTreeSet<_>>();
    let max_hops = input.max_hops.clamp(1, 5);
    let mut seen = BTreeSet::from([seed_node_id.clone()]);
    let mut queue = VecDeque::from([(seed_node_id, 0usize)]);
    let mut results = Vec::new();

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_hops {
            continue;
        }
        for direction in [Direction::Out, Direction::In] {
            for hit in store.memory_neighbors(NeighborQuery {
                node_id: current.clone(),
                direction,
                edge_type: None,
                include_expired: false,
            })? {
                if !edge_filter.is_empty() && !edge_filter.contains(&hit.edge_type) {
                    continue;
                }
                if !seen.insert(hit.node_id.clone()) {
                    continue;
                }
                if let Some(item) =
                    relation_item_from_graph_node(store, &hit.node_id, &hit, depth + 1)?
                {
                    results.push(item);
                    queue.push_back((hit.node_id, depth + 1));
                }
            }
        }
    }
    Ok(results)
}

pub fn self_note_memory<S: MemoryGraphStore>(
    store: &mut S,
    mut input: MemoryWriteInput,
) -> MemoryResult<MemoryDocumentState> {
    input.kind = if input.kind.trim().is_empty() {
        "self_note".to_string()
    } else {
        input.kind
    };
    input
        .metadata
        .insert("source".to_string(), Value::String("self_note".to_string()));
    input.metadata.insert(
        "memory_node_type".to_string(),
        Value::String(if input.memory_node_type.trim().is_empty() {
            "belief".to_string()
        } else {
            input.memory_node_type.trim().to_string()
        }),
    );
    create_memory_document(store, input)
}

pub fn revise_memory_document<S: MemoryGraphStore>(
    store: &mut S,
    input: ReviseMemoryInput,
) -> MemoryResult<ReviseMemoryReceipt> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let doc_id = require_text("doc_id", &input.doc_id)?;
    let mut original = load_memory_document(store, &tenant_slug, &doc_id)?.ok_or_else(|| {
        MemoryError::NotFound {
            kind: "document".to_string(),
            id: doc_id.clone(),
        }
    })?;
    if original.status == "deleted" {
        return Err(MemoryError::InvalidInput {
            field: "doc_id".to_string(),
            message: "deleted documents cannot be revised".to_string(),
        });
    }
    let now = timestamp_or_now(&input.updated_at);
    original.status = "superseded".to_string();
    original.updated_at = now.clone();
    if !input.reason.trim().is_empty() {
        original.metadata.insert(
            "superseded_reason".to_string(),
            Value::String(input.reason.trim().to_string()),
        );
    }
    persist_memory_document(store, &original)?;

    let mut metadata = original.metadata.clone();
    metadata.insert(
        "revises_doc_id".to_string(),
        Value::String(original.doc_id.clone()),
    );
    if !input.reason.trim().is_empty() {
        metadata.insert(
            "revision_reason".to_string(),
            Value::String(input.reason.trim().to_string()),
        );
    }
    let revised = create_memory_document(
        store,
        MemoryWriteInput {
            tenant_slug: tenant_slug.clone(),
            actor_id: choose_text(&input.actor_id, Some(&original.actor_id)),
            session_id: choose_text(&input.session_id, Some(&original.session_id)),
            origin_surface: choose_text(&input.origin_surface, Some(&original.origin_surface)),
            project_slug: original.project_slug.clone(),
            kind: original.kind.clone(),
            title: choose_text(&input.title, Some(&original.title)),
            content: input.content,
            summary: choose_text(&input.summary, Some(&original.summary)),
            tags: original.tags.clone(),
            links: original.links.clone(),
            memory_node_type: choose_text(
                &input.memory_node_type,
                Some(&original.memory_node_type),
            ),
            metadata,
            created_at: now,
            ..MemoryWriteInput::default()
        },
    )?;
    upsert_memory_edge(
        store,
        &tenant_slug,
        "MEMORY_SUPERSEDES",
        &memory_document_node_id(&tenant_slug, &revised.doc_id),
        &memory_document_node_id(&tenant_slug, &original.doc_id),
        json!({ "reason": input.reason, "updated_at": revised.updated_at }),
    )?;
    link_doc_id_list(
        store,
        &tenant_slug,
        &revised.doc_id,
        "MEMORY_CITES",
        &input.cites_doc_ids,
    )?;
    link_doc_id_list(
        store,
        &tenant_slug,
        &revised.doc_id,
        "MEMORY_DERIVED_FROM",
        &input.derived_from_doc_ids,
    )?;
    link_doc_id_list(
        store,
        &tenant_slug,
        &revised.doc_id,
        "MEMORY_CONTRADICTS",
        &input.contradicts_doc_ids,
    )?;
    invalidate_positive_edges_for_targets(
        store,
        &tenant_slug,
        &input.contradicts_doc_ids,
        &revised.updated_at,
    )?;

    Ok(ReviseMemoryReceipt {
        revised,
        superseded: original,
    })
}

pub fn archive_memory_document<S: MemoryGraphStore>(
    store: &mut S,
    input: ArchiveMemoryInput,
) -> MemoryResult<ArchiveMemoryReceipt> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let doc_id = require_text("doc_id", &input.doc_id)?;
    let mut archived = load_memory_document(store, &tenant_slug, &doc_id)?.ok_or_else(|| {
        MemoryError::NotFound {
            kind: "document".to_string(),
            id: doc_id.clone(),
        }
    })?;
    let now = timestamp_or_now(&input.archived_at);
    archived.status = "archived".to_string();
    archived.updated_at = now.clone();
    if !input.reason.trim().is_empty() {
        archived.metadata.insert(
            "archive_reason".to_string(),
            Value::String(input.reason.trim().to_string()),
        );
    }
    persist_memory_document(store, &archived)?;

    let archive = create_memory_document(
        store,
        MemoryWriteInput {
            tenant_slug: tenant_slug.clone(),
            actor_id: choose_text(&input.actor_id, Some(&archived.actor_id)),
            origin_surface: archived.origin_surface.clone(),
            project_slug: archived.project_slug.clone(),
            kind: "archive".to_string(),
            title: if input.title.trim().is_empty() {
                format!("Archive: {}", archived.title)
            } else {
                input.title.trim().to_string()
            },
            content: archived.content.clone(),
            summary: input.reason.trim().to_string(),
            tags: archived.tags.clone(),
            links: archived.links.clone(),
            metadata: Map::from_iter([
                (
                    "archived_doc_id".to_string(),
                    Value::String(archived.doc_id.clone()),
                ),
                (
                    "archive_reason".to_string(),
                    Value::String(input.reason.trim().to_string()),
                ),
            ]),
            status: "archived".to_string(),
            created_at: now,
            ..MemoryWriteInput::default()
        },
    )?;
    upsert_memory_edge(
        store,
        &tenant_slug,
        "MEMORY_ARCHIVED_AS",
        &memory_document_node_id(&tenant_slug, &archived.doc_id),
        &memory_document_node_id(&tenant_slug, &archive.doc_id),
        json!({ "reason": input.reason, "archived_at": archived.updated_at }),
    )?;
    Ok(ArchiveMemoryReceipt { archived, archive })
}

pub fn recall_archived_memory<S: MemoryGraphStore>(
    store: &mut S,
    mut input: RecallMemoryInput,
) -> MemoryResult<Vec<MemoryRecallItem>> {
    input.kind = input.kind.trim().to_string();
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let query = input.query.trim().to_string();
    let actor_filter = input.actor.trim().to_string();
    let limit = bounded_limit(input.limit);
    let mut results = load_memory_documents(store, &tenant_slug, true)?
        .into_iter()
        .filter(|document| document.status == "archived")
        .filter(|document| actor_filter.is_empty() || document.actor_id == actor_filter)
        .filter(|document| {
            query.is_empty()
                || score_match(
                    &query,
                    &document.title,
                    &document.content,
                    &document.summary,
                ) > 0.0
        })
        .map(recall_item_for_document)
        .collect::<Vec<_>>();
    for item in &mut results {
        item.score = score_match(&query, &item.title, &item.content, &item.summary);
    }
    results.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    results.truncate(limit);
    Ok(results)
}

pub fn encode_memory<S: MemoryGraphStore>(
    store: &mut S,
    mut input: MemoryWriteInput,
    encode: EncodeMemoryInput,
) -> MemoryResult<MemoryDocumentState> {
    let kind = normalize_encode_kind(&input.kind)?;
    input.kind = kind;
    let mut fitness = Map::new();
    fitness.insert(
        "outcome".to_string(),
        Value::String(normalize_outcome(&encode.outcome)?),
    );
    fitness.insert(
        "signal".to_string(),
        Value::String(encode.signal.trim().to_string()),
    );
    fitness.insert(
        "reason".to_string(),
        Value::String(encode.reason.trim().to_string()),
    );
    fitness.insert(
        "event_id".to_string(),
        Value::String(encode.event_id.trim().to_string()),
    );
    fitness.insert(
        "auto_triggered".to_string(),
        Value::Bool(encode.auto_triggered),
    );
    if !encode.context.is_null() {
        fitness.insert("context".to_string(), encode.context);
    }
    input.fitness = Some(Value::Object(fitness.clone()));
    input
        .metadata
        .insert("fitness".to_string(), Value::Object(fitness));
    create_memory_document(store, input)
}

/// Stable content hash for a memory document body. The Obsidian sync plugin uses
/// this as its echo gate: a note whose body already matches the graph's
/// `content_hash` is not pushed back, breaking the bidirectional sync loop.
pub fn memory_content_hash(content: &str) -> String {
    hash_prefix(&stable_value_hash(&Value::String(content.to_string())))
}

/// List a tenant's memory documents for the Obsidian mirror, newest first.
///
/// `since` filters to documents whose `updated_at` is at or after the watermark
/// (lexical compare, matching the recall path); pass the `max_updated_at` returned
/// by a previous sync for incremental pulls. Deleted documents are always omitted.
/// With `include_inactive` false only `active` documents are returned; with it true
/// `superseded` and `archived` documents are included as well.
pub fn list_memory_documents_since<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    since: &str,
    include_inactive: bool,
) -> MemoryResult<Vec<MemoryDocumentState>> {
    let since = since.trim();
    let mut documents = load_memory_documents(store, tenant_slug, include_inactive)?;
    documents.retain(|document| document.status != "deleted");
    if !since.is_empty() {
        documents.retain(|document| document.updated_at.as_str() >= since);
    }
    Ok(documents)
}

/// Create or update an Obsidian-synced memory document and reconcile its
/// `[[wikilink]]` edges in one call. This is the write path the sync plugin uses:
/// `self_revise` cannot carry a changed link set and exposes no edge removal, so
/// reconciliation has to happen server-side. Resolved links become `MEMORY_RELATES`
/// edges, removed links are tombstoned, and forward references to notes that do not
/// exist yet are recorded as unresolved and resolved when the target note appears.
pub fn upsert_note<S: MemoryGraphStore>(
    store: &mut S,
    input: UpsertNoteInput,
) -> MemoryResult<UpsertNoteReceipt> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let content = require_text("content", &input.content)?;
    let now = timestamp_or_now(&input.updated_at);
    let kind = if input.kind.trim().is_empty() {
        "note".to_string()
    } else {
        input.kind.trim().to_lowercase()
    };
    let desired_links = normalize_strings(&input.links);
    let tags = normalize_strings(&input.tags);

    let existing = if input.doc_id.trim().is_empty() {
        None
    } else {
        load_memory_document(store, &tenant_slug, input.doc_id.trim())?
    };

    let (mut document, action) = match existing {
        Some(mut document) => {
            document.kind = kind.clone();
            document.title = input.title.trim().to_string();
            document.content = content.clone();
            document.summary = input.summary.trim().to_string();
            document.tags = tags.clone();
            document.links = desired_links.clone();
            if !input.status.trim().is_empty() {
                document.status = normalize_status(&input.status)?;
            }
            if !input.memory_node_type.trim().is_empty() {
                document.memory_node_type = input.memory_node_type.trim().to_string();
            }
            if !input.origin_surface.trim().is_empty() {
                document.origin_surface = input.origin_surface.trim().to_string();
            }
            if !input.actor_id.trim().is_empty() {
                document.actor_id = input.actor_id.trim().to_string();
            }
            for (key, value) in input.metadata.clone() {
                document.metadata.insert(key, value);
            }
            document.updated_at = now.clone();
            (document, "updated")
        }
        None => {
            let created_at = timestamp_or_now(&input.created_at);
            let doc_id = if input.doc_id.trim().is_empty() {
                stable_document_id(&tenant_slug, &kind, &input.title, &content, &created_at)
            } else {
                input.doc_id.trim().to_string()
            };
            let document = MemoryDocumentState {
                tenant_slug: tenant_slug.clone(),
                doc_id,
                kind: kind.clone(),
                title: input.title.trim().to_string(),
                content: content.clone(),
                summary: input.summary.trim().to_string(),
                tags: tags.clone(),
                links: desired_links.clone(),
                actor_id: input.actor_id.trim().to_string(),
                session_id: input.session_id.trim().to_string(),
                origin_surface: if input.origin_surface.trim().is_empty() {
                    "obsidian".to_string()
                } else {
                    input.origin_surface.trim().to_string()
                },
                project_slug: input.project_slug.trim().to_string(),
                status: if input.status.trim().is_empty() {
                    DEFAULT_STATUS.to_string()
                } else {
                    normalize_status(&input.status)?
                },
                memory_node_type: input.memory_node_type.trim().to_string(),
                target_actor_id: String::new(),
                expires_at: String::new(),
                metadata: input.metadata.clone(),
                fitness: None,
                created_at: created_at.clone(),
                updated_at: now.clone(),
                deleted_reason: String::new(),
                deleted_at: String::new(),
            };
            (document, "created")
        }
    };

    if is_encode_kind(&document.kind) {
        let mut fitness = Map::new();
        fitness.insert(
            "outcome".to_string(),
            Value::String(normalize_outcome(&input.outcome)?),
        );
        fitness.insert(
            "signal".to_string(),
            Value::String(input.signal.trim().to_string()),
        );
        fitness.insert(
            "reason".to_string(),
            Value::String(input.reason.trim().to_string()),
        );
        fitness.insert(
            "event_id".to_string(),
            Value::String(input.event_id.trim().to_string()),
        );
        fitness.insert("auto_triggered".to_string(), Value::Bool(false));
        document.fitness = Some(Value::Object(fitness.clone()));
        document
            .metadata
            .insert("fitness".to_string(), Value::Object(fitness));
    }

    let source_node_id = memory_document_node_id(&tenant_slug, &document.doc_id);

    let previous_targets: Vec<(String, String)> = store
        .memory_neighbors(NeighborQuery {
            node_id: source_node_id.clone(),
            direction: Direction::Out,
            edge_type: None,
            include_expired: false,
        })?
        .into_iter()
        .filter(|hit| hit.edge_type == "MEMORY_RELATES")
        .map(|hit| (hit.edge_id, hit.node_id))
        .collect();

    let mut resolved_links = Vec::new();
    let mut unresolved_links = Vec::new();
    let mut resolved_node_ids = BTreeSet::new();
    for link in &desired_links {
        match resolve_memory_graph_id(store, &tenant_slug, link)? {
            Some(node_id) => {
                resolved_node_ids.insert(node_id);
                resolved_links.push(link.clone());
            }
            None => unresolved_links.push(link.clone()),
        }
    }

    document.metadata.insert(
        "unresolved_links".to_string(),
        Value::Array(
            unresolved_links
                .iter()
                .cloned()
                .map(Value::String)
                .collect(),
        ),
    );

    persist_memory_document(store, &document)?;

    let mut removed_links = Vec::new();
    for (edge_id, target_node_id) in previous_targets {
        if resolved_node_ids.contains(&target_node_id) {
            continue;
        }
        tombstone_memory_edge(store, &edge_id, &source_node_id, &target_node_id)?;
        removed_links.push(target_node_id);
    }

    let reconciled_back = reconcile_incoming_links(store, &tenant_slug, &document)?;

    Ok(UpsertNoteReceipt {
        action: action.to_string(),
        document,
        resolved_links,
        unresolved_links,
        removed_links,
        reconciled_back,
    })
}

/// Resolve other documents' unresolved forward references that point at `document`,
/// by matching their recorded `unresolved_links` against this note's `doc_id` or
/// title. A match upserts the previously-dangling `MEMORY_RELATES` edge and drops
/// the entry from the source document's unresolved list.
fn reconcile_incoming_links<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    document: &MemoryDocumentState,
) -> MemoryResult<Vec<String>> {
    let source_node_id = memory_document_node_id(tenant_slug, &document.doc_id);
    let doc_id_key = document.doc_id.to_lowercase();
    let title_key = document.title.trim().to_lowercase();
    let mut reconciled = Vec::new();

    for mut other in load_memory_documents(store, tenant_slug, true)? {
        if other.doc_id == document.doc_id {
            continue;
        }
        let Some(Value::Array(entries)) = other.metadata.get("unresolved_links").cloned() else {
            continue;
        };
        let mut still_unresolved = Vec::new();
        let mut matched = false;
        for entry in entries {
            let Some(text) = entry.as_str() else {
                continue;
            };
            let normalized = text.trim().to_lowercase();
            if normalized == doc_id_key || (!title_key.is_empty() && normalized == title_key) {
                matched = true;
            } else {
                still_unresolved.push(Value::String(text.to_string()));
            }
        }
        if !matched {
            continue;
        }
        let other_node_id = memory_document_node_id(tenant_slug, &other.doc_id);
        upsert_memory_edge(
            store,
            tenant_slug,
            "MEMORY_RELATES",
            &other_node_id,
            &source_node_id,
            json!({ "source": "links_reconciled", "updated_at": document.updated_at }),
        )?;
        other.metadata.insert(
            "unresolved_links".to_string(),
            Value::Array(still_unresolved),
        );
        if !other.links.iter().any(|link| link == &document.doc_id) {
            other.links.push(document.doc_id.clone());
        }
        persist_memory_document(store, &other)?;
        reconciled.push(other.doc_id.clone());
    }

    Ok(reconciled)
}

fn tombstone_memory_edge<S: MemoryGraphStore>(
    store: &mut S,
    edge_id: &str,
    from_id: &str,
    to_id: &str,
) -> MemoryResult<()> {
    let mut edge = EdgeRecord::new(
        edge_id,
        from_id,
        "MEMORY_RELATES",
        to_id,
        json!({ "removed": true }),
    );
    edge.tombstone = true;
    store.memory_upsert_edge(edge)?;
    Ok(())
}

fn is_encode_kind(kind: &str) -> bool {
    matches!(kind, "encode" | "feedback" | "solution" | "postmortem")
}

pub fn forget_memory<S: MemoryGraphStore>(
    store: &mut S,
    input: ForgetMemoryInput,
) -> MemoryResult<ForgetMemoryReceipt> {
    let tenant_slug = normalize_tenant_slug(&input.tenant_slug);
    let id = require_text("id", &input.id)?;
    let reason = require_text("reason", &input.reason)?;
    let now = timestamp_or_now(&input.deleted_at);
    if let Some(mut document) = load_memory_document(store, &tenant_slug, &id)? {
        document.status = "deleted".to_string();
        document.deleted_reason = reason;
        document.deleted_at = now.clone();
        document.updated_at = now;
        if !input.actor_id.trim().is_empty() {
            document.metadata.insert(
                "deleted_by".to_string(),
                Value::String(input.actor_id.trim().to_string()),
            );
        }
        persist_memory_document(store, &document)?;
        return Ok(ForgetMemoryReceipt {
            forgotten_type: "document".to_string(),
            document: Some(document),
            node: None,
        });
    }
    if let Some(mut node) = load_memory_node(store, &tenant_slug, &id)? {
        node.status = "deleted".to_string();
        node.deleted_reason = reason;
        node.deleted_at = now.clone();
        node.updated_at = now;
        if !input.actor_id.trim().is_empty() {
            node.metadata.insert(
                "deleted_by".to_string(),
                Value::String(input.actor_id.trim().to_string()),
            );
        }
        persist_memory_node(store, &node)?;
        return Ok(ForgetMemoryReceipt {
            forgotten_type: "node".to_string(),
            document: None,
            node: Some(node),
        });
    }
    Err(MemoryError::NotFound {
        kind: "document or node".to_string(),
        id,
    })
}

pub fn handoff_memory<S: MemoryGraphStore>(
    store: &mut S,
    input: HandoffMemoryInput,
) -> MemoryResult<MemoryDocumentState> {
    let to_actor = require_text("to_actor", &input.to_actor)?;
    let content = if let Some(raw) = input.payload.as_str() {
        raw.to_string()
    } else {
        serde_json::to_string_pretty(&input.payload)
            .map_err(|error| MemoryError::Serialization(error.to_string()))?
    };
    let payload_type = match &input.payload {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
    };
    create_memory_document(
        store,
        MemoryWriteInput {
            tenant_slug: input.tenant_slug,
            actor_id: input.actor_id,
            session_id: input.session_id,
            origin_surface: input.origin_surface,
            kind: "handoff".to_string(),
            title: if input.title.trim().is_empty() {
                format!("Handoff to {to_actor}")
            } else {
                input.title.trim().to_string()
            },
            content,
            target_actor_id: to_actor,
            expires_at: input.expires_at,
            metadata: Map::from_iter([(
                "payload_type".to_string(),
                Value::String(payload_type.to_string()),
            )]),
            created_at: input.created_at,
            ..MemoryWriteInput::default()
        },
    )
}

pub fn load_memory_document<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    doc_id: &str,
) -> MemoryResult<Option<MemoryDocumentState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let doc_id = doc_id.trim();
    if doc_id.is_empty() {
        return Ok(None);
    }
    let graph_id = if doc_id.starts_with("mem:doc:") {
        doc_id.to_string()
    } else {
        memory_document_node_id(&tenant_slug, doc_id)
    };
    store
        .memory_get_node(&graph_id)?
        .map(document_from_node)
        .transpose()
}

pub fn load_memory_node<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    node_id: &str,
) -> MemoryResult<Option<MemoryNodeState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let node_id = node_id.trim();
    if node_id.is_empty() {
        return Ok(None);
    }
    let graph_id = if node_id.starts_with("mem:node:") {
        node_id.to_string()
    } else {
        memory_node_node_id(&tenant_slug, node_id)
    };
    store
        .memory_get_node(&graph_id)?
        .map(node_from_node)
        .transpose()
}

pub fn memory_document_node_id(tenant_slug: &str, doc_id: &str) -> String {
    format!(
        "mem:doc:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify(doc_id).if_empty("unknown")
    )
}

pub fn memory_node_node_id(tenant_slug: &str, node_id: &str) -> String {
    format!(
        "mem:node:{}:{}",
        normalize_tenant_slug(tenant_slug),
        slugify(node_id).if_empty("unknown")
    )
}

pub fn memory_edge_id(tenant_slug: &str, edge_type: &str, from_id: &str, to_id: &str) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": normalize_tenant_slug(tenant_slug),
        "edge_type": edge_type,
        "from_id": from_id,
        "to_id": to_id,
    }));
    format!(
        "mem:edge:{}:{}:{}",
        normalize_tenant_slug(tenant_slug),
        edge_type.to_lowercase(),
        hash_prefix(&hash)
    )
}

fn persist_memory_document<S: MemoryGraphStore>(
    store: &mut S,
    document: &MemoryDocumentState,
) -> MemoryResult<()> {
    upsert_node_if_changed(store, memory_document_node(document)?)?;
    for link in &document.links {
        if let Some(target_id) = resolve_memory_graph_id(store, &document.tenant_slug, link)? {
            upsert_memory_edge(
                store,
                &document.tenant_slug,
                "MEMORY_RELATES",
                &memory_document_node_id(&document.tenant_slug, &document.doc_id),
                &target_id,
                json!({ "source": "links", "updated_at": document.updated_at }),
            )?;
        }
    }
    Ok(())
}

fn persist_memory_node<S: MemoryGraphStore>(
    store: &mut S,
    node: &MemoryNodeState,
) -> MemoryResult<()> {
    upsert_node_if_changed(store, memory_node_node(node)?)?;
    for link in &node.links {
        if let Some(target_id) = resolve_memory_graph_id(store, &node.tenant_slug, link)? {
            upsert_memory_edge(
                store,
                &node.tenant_slug,
                "MEMORY_RELATES",
                &memory_node_node_id(&node.tenant_slug, &node.node_id),
                &target_id,
                json!({ "source": "links", "updated_at": node.updated_at }),
            )?;
        }
    }
    Ok(())
}

fn memory_document_node(document: &MemoryDocumentState) -> MemoryResult<NodeRecord> {
    let mut properties = serde_json::to_value(document)
        .map_err(|error| MemoryError::Serialization(error.to_string()))?;
    insert_search_text(
        &mut properties,
        &document.title,
        &document.content,
        &document.summary,
        &document.tags,
    );
    Ok(NodeRecord::new(
        memory_document_node_id(&document.tenant_slug, &document.doc_id),
        ["HarnessMemory", "MemoryAtom", "MemoryDocument"],
        properties,
    ))
}

fn memory_node_node(node: &MemoryNodeState) -> MemoryResult<NodeRecord> {
    let mut properties = serde_json::to_value(node)
        .map_err(|error| MemoryError::Serialization(error.to_string()))?;
    insert_search_text(&mut properties, &node.title, &node.content, "", &node.tags);
    Ok(NodeRecord::new(
        memory_node_node_id(&node.tenant_slug, &node.node_id),
        ["HarnessMemory", "MemoryAtom", "MemoryNode"],
        properties,
    ))
}

fn document_from_node(node: NodeRecord) -> MemoryResult<MemoryDocumentState> {
    serde_json::from_value::<MemoryDocumentState>(node.properties)
        .map_err(|error| MemoryError::Deserialization(error.to_string()))
}

fn node_from_node(node: NodeRecord) -> MemoryResult<MemoryNodeState> {
    serde_json::from_value::<MemoryNodeState>(node.properties)
        .map_err(|error| MemoryError::Deserialization(error.to_string()))
}

fn load_memory_documents<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    include_inactive: bool,
) -> MemoryResult<Vec<MemoryDocumentState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let mut documents = store
        .memory_query_nodes(
            NodeQuery::label("MemoryDocument")
                .with_property("tenant_slug", Value::String(tenant_slug))
                .with_limit(MAX_GRAPH_QUERY_LIMIT),
        )?
        .into_iter()
        .map(document_from_node)
        .filter_map(|result| match result {
            Ok(document) if include_inactive || document.status == DEFAULT_STATUS => {
                Some(Ok(document))
            }
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<MemoryResult<Vec<_>>>()?;
    documents.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(documents)
}

fn load_memory_nodes<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    include_inactive: bool,
) -> MemoryResult<Vec<MemoryNodeState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let mut nodes = store
        .memory_query_nodes(
            NodeQuery::label("MemoryNode")
                .with_property("tenant_slug", Value::String(tenant_slug))
                .with_limit(MAX_GRAPH_QUERY_LIMIT),
        )?
        .into_iter()
        .map(node_from_node)
        .filter_map(|result| match result {
            Ok(node) if include_inactive || node.status == DEFAULT_STATUS => Some(Ok(node)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<MemoryResult<Vec<_>>>()?;
    nodes.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    Ok(nodes)
}

fn document_matches_recall(
    document: &MemoryDocumentState,
    query: &str,
    kind_filter: &str,
    surface_filter: &str,
    actor_filter: &str,
    since: &str,
    include_low_fitness: bool,
) -> bool {
    if document.status != DEFAULT_STATUS {
        return false;
    }
    if kind_filter == "handoff" {
        if document.kind != "handoff" {
            return false;
        }
        if !actor_filter.is_empty()
            && !document.target_actor_id.is_empty()
            && document.target_actor_id != actor_filter
        {
            return false;
        }
    } else if !kind_filter.is_empty() && document.kind != kind_filter {
        return false;
    }
    if !surface_filter.is_empty() && document.origin_surface != surface_filter {
        return false;
    }
    if !actor_filter.is_empty() && kind_filter != "handoff" && document.actor_id != actor_filter {
        return false;
    }
    if !since.is_empty() && document.updated_at.as_str() < since {
        return false;
    }
    if !include_low_fitness && is_low_fitness(document.fitness.as_ref()) {
        return false;
    }
    query.is_empty()
        || score_match(query, &document.title, &document.content, &document.summary) > 0.0
}

fn node_matches_recall(
    node: &MemoryNodeState,
    query: &str,
    kind_filter: &str,
    surface_filter: &str,
    actor_filter: &str,
    since: &str,
    include_low_fitness: bool,
) -> bool {
    if node.status != DEFAULT_STATUS {
        return false;
    }
    if !kind_filter.is_empty() && node.kind != kind_filter {
        return false;
    }
    if !surface_filter.is_empty() && node.origin_surface != surface_filter {
        return false;
    }
    if !actor_filter.is_empty() && node.actor_id != actor_filter {
        return false;
    }
    if !since.is_empty() && node.updated_at.as_str() < since {
        return false;
    }
    if !include_low_fitness && is_low_fitness(node.fitness.as_ref()) {
        return false;
    }
    query.is_empty() || score_match(query, &node.title, &node.content, "") > 0.0
}

fn recall_item_for_document(document: MemoryDocumentState) -> MemoryRecallItem {
    let mut provenance = Map::new();
    provenance.insert(
        "actor".to_string(),
        Value::String(document.actor_id.clone()),
    );
    provenance.insert(
        "surface".to_string(),
        Value::String(document.origin_surface.clone()),
    );
    provenance.insert(
        "session".to_string(),
        Value::String(document.session_id.clone()),
    );
    MemoryRecallItem {
        id: document.doc_id.clone(),
        item_type: "document".to_string(),
        kind: document.kind.clone(),
        title: document.title.clone(),
        content: document.content.clone(),
        summary: document.summary.clone(),
        status: document.status.clone(),
        actor_id: document.actor_id.clone(),
        origin_surface: document.origin_surface.clone(),
        session_id: document.session_id.clone(),
        updated_at: document.updated_at.clone(),
        score: 0.0,
        provenance,
        flags: Vec::new(),
        rank_signals: Map::new(),
        document: Some(document),
        node: None,
    }
}

fn recall_item_for_node(node: MemoryNodeState) -> MemoryRecallItem {
    let mut provenance = Map::new();
    provenance.insert("actor".to_string(), Value::String(node.actor_id.clone()));
    provenance.insert(
        "surface".to_string(),
        Value::String(node.origin_surface.clone()),
    );
    provenance.insert(
        "session".to_string(),
        Value::String(node.session_id.clone()),
    );
    MemoryRecallItem {
        id: node.node_id.clone(),
        item_type: "node".to_string(),
        kind: node.kind.clone(),
        title: node.title.clone(),
        content: node.content.clone(),
        summary: String::new(),
        status: node.status.clone(),
        actor_id: node.actor_id.clone(),
        origin_surface: node.origin_surface.clone(),
        session_id: node.session_id.clone(),
        updated_at: node.updated_at.clone(),
        score: 0.0,
        provenance,
        flags: Vec::new(),
        rank_signals: Map::new(),
        document: None,
        node: Some(node),
    }
}

#[derive(Clone)]
struct RecallAtom {
    graph_id: String,
    item: MemoryRecallItem,
    text: String,
}

#[derive(Clone, Default)]
struct SeedProfile {
    fulltext_score: f64,
    vector_score: f64,
    mass: f64,
}

fn load_recall_atoms<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    kind_filter: &str,
    surface_filter: &str,
    actor_filter: &str,
    since: &str,
    include_low_fitness: bool,
) -> MemoryResult<Vec<RecallAtom>> {
    let mut atoms = Vec::new();
    for document in load_memory_documents(store, tenant_slug, true)? {
        if !document_matches_recall(
            &document,
            "",
            kind_filter,
            surface_filter,
            actor_filter,
            since,
            include_low_fitness,
        ) {
            continue;
        }
        let graph_id = memory_document_node_id(&document.tenant_slug, &document.doc_id);
        let text = memory_document_text(&document);
        atoms.push(RecallAtom {
            graph_id,
            item: recall_item_for_document(document),
            text,
        });
    }
    for node in load_memory_nodes(store, tenant_slug, true)? {
        if !node_matches_recall(
            &node,
            "",
            kind_filter,
            surface_filter,
            actor_filter,
            since,
            include_low_fitness,
        ) {
            continue;
        }
        let graph_id = memory_node_node_id(&node.tenant_slug, &node.node_id);
        let text = memory_node_text(&node);
        atoms.push(RecallAtom {
            graph_id,
            item: recall_item_for_node(node),
            text,
        });
    }
    Ok(atoms)
}

fn memory_document_text(document: &MemoryDocumentState) -> String {
    let tags = document.tags.join(" ");
    [
        document.title.as_str(),
        document.summary.as_str(),
        document.content.as_str(),
        tags.as_str(),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n")
}

fn memory_node_text(node: &MemoryNodeState) -> String {
    let tags = node.tags.join(" ");
    [node.title.as_str(), node.content.as_str(), tags.as_str()]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn resolve_recall_seeds<S: MemoryGraphStore>(
    store: &S,
    atoms: &[RecallAtom],
    query: &str,
    input: &RecallMemoryInput,
    seed_limit: usize,
) -> MemoryResult<BTreeMap<String, SeedProfile>> {
    if query.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    let atom_ids = atoms
        .iter()
        .map(|atom| atom.graph_id.as_str())
        .collect::<BTreeSet<_>>();
    let mut fulltext = atoms
        .iter()
        .map(|atom| {
            (
                atom.graph_id.clone(),
                score_match(
                    query,
                    &atom.item.title,
                    &atom.item.content,
                    &atom.item.summary,
                ),
            )
        })
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    for (graph_id, score) in indexed_fulltext_seed_scores(store, query, seed_limit)? {
        if atom_ids.contains(graph_id.as_str()) {
            fulltext.push((graph_id, score));
        }
    }
    fulltext.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    fulltext.truncate(seed_limit);

    let identifier_query = query_has_specific_anchor(query);
    let mut vector = indexed_vector_seed_scores(store, input, seed_limit)?
        .into_iter()
        .filter(|(graph_id, _)| atom_ids.contains(graph_id.as_str()))
        .collect::<Vec<_>>();
    if vector.is_empty() {
        vector = atoms
            .iter()
            .filter_map(|atom| {
                let score = if input.query_embedding.is_empty() {
                    if identifier_query {
                        0.0
                    } else {
                        token_cosine_score(query, &atom.text)
                    }
                } else {
                    atom_embedding_score(atom, &input.query_embedding, &input.embedding_property)
                };
                (score > 0.0).then(|| (atom.graph_id.clone(), score))
            })
            .collect::<Vec<_>>();
    }
    vector.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    vector.truncate(seed_limit);

    let max_fulltext = fulltext
        .iter()
        .map(|(_, score)| *score)
        .fold(0.0_f64, f64::max)
        .max(1e-12);
    let max_vector = vector
        .iter()
        .map(|(_, score)| *score)
        .fold(0.0_f64, f64::max)
        .max(1e-12);
    let mut seeds: BTreeMap<String, SeedProfile> = BTreeMap::new();
    for (graph_id, score) in fulltext {
        seeds.entry(graph_id).or_default().fulltext_score = score / max_fulltext;
    }
    for (graph_id, score) in vector {
        seeds.entry(graph_id).or_default().vector_score = score / max_vector;
    }
    let total = seeds
        .values()
        .map(|seed| seed.fulltext_score + seed.vector_score)
        .sum::<f64>();
    if total > 0.0 {
        for seed in seeds.values_mut() {
            seed.mass = (seed.fulltext_score + seed.vector_score) / total;
        }
    }
    Ok(seeds)
}

fn seed_community_summaries(
    atoms: &[RecallAtom],
    query: &str,
    seed_limit: usize,
) -> BTreeMap<String, SeedProfile> {
    let mut seeds = resolve_summary_seeds(atoms, query, seed_limit);
    if seeds.is_empty() && !atoms.is_empty() {
        let take = atoms.len().min(seed_limit.max(1));
        let mass = 1.0 / take as f64;
        for atom in atoms.iter().take(take) {
            seeds.insert(
                atom.graph_id.clone(),
                SeedProfile {
                    fulltext_score: 1.0,
                    vector_score: 0.0,
                    mass,
                },
            );
        }
    }
    seeds
}

fn resolve_summary_seeds(
    atoms: &[RecallAtom],
    query: &str,
    seed_limit: usize,
) -> BTreeMap<String, SeedProfile> {
    let mut fulltext = atoms
        .iter()
        .map(|atom| {
            (
                atom.graph_id.clone(),
                score_match(
                    query,
                    &atom.item.title,
                    &atom.item.content,
                    &atom.item.summary,
                ),
            )
        })
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    fulltext.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    fulltext.truncate(seed_limit);
    let total = fulltext.iter().map(|(_, score)| *score).sum::<f64>();
    let mut seeds = BTreeMap::new();
    if total > 0.0 {
        for (graph_id, score) in fulltext {
            seeds.insert(
                graph_id,
                SeedProfile {
                    fulltext_score: score / total,
                    vector_score: 0.0,
                    mass: score / total,
                },
            );
        }
    }
    seeds
}

fn indexed_fulltext_seed_scores<S: MemoryGraphStore>(
    store: &S,
    query: &str,
    seed_limit: usize,
) -> MemoryResult<Vec<(String, f64)>> {
    let mut results = Vec::new();
    for label in [
        Some("MemoryAtom"),
        Some("MemoryDocument"),
        Some("MemoryNode"),
    ] {
        match store.memory_fulltext_search(label, "search_text", query, seed_limit) {
            Ok(hits) => results.extend(
                hits.into_iter()
                    .filter(|(_, score)| *score > 0.0)
                    .map(|(id, score)| (id, score as f64)),
            ),
            Err(error) if error.code == "unsupported_operation" => {}
            Err(error) => return Err(MemoryError::Store(error)),
        }
    }
    dedupe_rank_scores(results, true, seed_limit)
}

fn indexed_vector_seed_scores<S: MemoryGraphStore>(
    store: &S,
    input: &RecallMemoryInput,
    seed_limit: usize,
) -> MemoryResult<Vec<(String, f64)>> {
    if input.query_embedding.is_empty() {
        return Ok(Vec::new());
    }
    let property = if input.embedding_property.trim().is_empty() {
        "embedding"
    } else {
        input.embedding_property.trim()
    };
    let mut results = Vec::new();
    for label in [
        Some("MemoryAtom"),
        Some("MemoryDocument"),
        Some("MemoryNode"),
    ] {
        match store.memory_vector_search(label, property, &input.query_embedding, seed_limit) {
            Ok(hits) => results.extend(hits.into_iter().map(|(id, distance)| {
                let score = (1.0 - distance as f64).clamp(0.0, 1.0);
                (id, score)
            })),
            Err(error) if error.code == "unsupported_operation" => {}
            Err(error) if error.code == "dimension_mismatch" => {}
            Err(error) if error.code == "no_vector_designation" => {}
            Err(error) => return Err(MemoryError::Store(error)),
        }
    }
    dedupe_rank_scores(results, true, seed_limit)
}

fn dedupe_rank_scores(
    results: Vec<(String, f64)>,
    higher_is_better: bool,
    limit: usize,
) -> MemoryResult<Vec<(String, f64)>> {
    let mut by_id = BTreeMap::<String, f64>::new();
    for (id, score) in results {
        if !score.is_finite() {
            continue;
        }
        by_id
            .entry(id)
            .and_modify(|current| {
                if (higher_is_better && score > *current) || (!higher_is_better && score < *current)
                {
                    *current = score;
                }
            })
            .or_insert(score);
    }
    let mut results = by_id.into_iter().collect::<Vec<_>>();
    if higher_is_better {
        results.sort_by(|left, right| {
            right
                .1
                .partial_cmp(&left.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
    } else {
        results.sort_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
    }
    results.truncate(limit);
    Ok(results)
}

fn lexical_recall_results(atoms: &[RecallAtom], query: &str) -> Vec<MemoryRecallItem> {
    let identifier_query = query_has_specific_anchor(query);
    atoms
        .iter()
        .map(|atom| {
            let mut item = atom.item.clone();
            let lexical = score_match(query, &item.title, &item.content, &item.summary);
            let vector = if identifier_query {
                0.0
            } else {
                token_cosine_score(query, &atom.text)
            };
            item.score = if query.trim().is_empty() {
                1.0
            } else {
                lexical + vector
            };
            item.rank_signals
                .insert("lexical_score".to_string(), json!(lexical));
            item.rank_signals
                .insert("vector_score".to_string(), json!(vector));
            item.rank_signals
                .insert("pipeline".to_string(), json!("stage0_or_stage3"));
            item
        })
        .collect()
}

fn ranked_ppr_recall_results<S: MemoryGraphStore>(
    store: &S,
    atoms: &HashMap<String, RecallAtom>,
    seeds: &BTreeMap<String, SeedProfile>,
    query: &str,
    query_time: &str,
    input: &RecallMemoryInput,
) -> MemoryResult<Vec<MemoryRecallItem>> {
    let adjacency = memory_recall_adjacency(
        store,
        atoms,
        query_time,
        effective_recency_half_life(input.recency_half_life_seconds),
    )?;
    let seed_mass = seeds
        .iter()
        .map(|(graph_id, profile)| (graph_id.clone(), profile.mass))
        .collect::<HashMap<_, _>>();
    let ppr = rustyred_thg_core::personalized_pagerank(
        &adjacency,
        &seed_mass,
        effective_ppr_alpha(input.ppr_alpha),
        effective_ppr_epsilon(input.ppr_epsilon),
        effective_ppr_max_pushes(input.ppr_max_pushes),
    );
    let mut results = Vec::new();
    for (graph_id, atom) in atoms {
        let ppr_score = ppr.get(graph_id).copied().unwrap_or(0.0);
        let seed = seeds.get(graph_id).cloned().unwrap_or_default();
        let lexical = score_match(
            query,
            &atom.item.title,
            &atom.item.content,
            &atom.item.summary,
        );
        let vector = if input.query_embedding.is_empty() {
            if query_has_specific_anchor(query) {
                0.0
            } else {
                token_cosine_score(query, &atom.text)
            }
        } else {
            atom_embedding_score(atom, &input.query_embedding, &input.embedding_property)
        };
        let fitness_boost = fitness_rank_boost(&atom.item);
        let base = ppr_score + (0.15 * seed.fulltext_score) + (0.15 * seed.vector_score);
        let mut item = atom.item.clone();
        item.score = (base * (1.0 + fitness_boost)).max(lexical * 0.01 + vector * 0.01);
        item.rank_signals
            .insert("ppr_score".to_string(), json!(ppr_score));
        item.rank_signals
            .insert("seed_mass".to_string(), json!(seed.mass));
        item.rank_signals.insert(
            "fulltext_seed_score".to_string(),
            json!(seed.fulltext_score),
        );
        item.rank_signals
            .insert("vector_seed_score".to_string(), json!(seed.vector_score));
        item.rank_signals
            .insert("lexical_score".to_string(), json!(lexical));
        item.rank_signals
            .insert("vector_score".to_string(), json!(vector));
        item.rank_signals
            .insert("fitness_boost".to_string(), json!(fitness_boost));
        item.rank_signals
            .insert("pipeline".to_string(), json!("stage0_stage1_stage2"));
        results.push(item);
    }
    Ok(results)
}

fn memory_recall_adjacency<S: MemoryGraphStore>(
    store: &S,
    atoms: &HashMap<String, RecallAtom>,
    query_time: &str,
    recency_half_life_seconds: f64,
) -> MemoryResult<HashMap<String, Vec<(String, f64)>>> {
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for graph_id in atoms.keys() {
        adjacency.entry(graph_id.clone()).or_default();
        for hit in store.memory_neighbors(NeighborQuery::out(graph_id))? {
            if !atoms.contains_key(&hit.node_id) {
                continue;
            }
            let Some(edge) = store.memory_get_edge(&hit.edge_id)? else {
                continue;
            };
            if !edge_valid_at(&edge, query_time) || !edge_propagates(&edge) {
                continue;
            }
            let weight = edge_propagation_weight(&edge)
                * edge.effective_confidence()
                * recency_decay(&edge, query_time, recency_half_life_seconds);
            if weight > 0.0 {
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), weight));
            }
        }
    }
    Ok(adjacency)
}

fn annotate_recall_results<S: MemoryGraphStore>(
    store: &S,
    results: &mut [MemoryRecallItem],
    tenant_slug: &str,
    query_time: &str,
) -> MemoryResult<()> {
    for item in results {
        let graph_id = recall_item_graph_id(tenant_slug, item);
        let mut support_clusters = BTreeSet::new();
        for direction in [Direction::Out, Direction::In] {
            for hit in store.memory_neighbors(NeighborQuery {
                node_id: graph_id.clone(),
                direction,
                edge_type: None,
                include_expired: false,
            })? {
                let Some(edge) = store.memory_get_edge(&hit.edge_id)? else {
                    continue;
                };
                if !edge_valid_at(&edge, query_time) {
                    continue;
                }
                if edge_is_contradiction_or_tension(&edge) {
                    item.flags.push(MemoryRecallFlag {
                        kind: edge_flag_kind(&edge).to_string(),
                        edge_id: edge.id.clone(),
                        edge_type: edge.edge_type.clone(),
                        related_id: memory_external_id(&hit.node_id),
                        message: format!(
                            "{} edge connected to recalled memory",
                            edge.edge_type.to_lowercase()
                        ),
                    });
                } else if edge_propagates(&edge) {
                    support_clusters.insert(edge_source_cluster(&edge, &hit.node_id));
                }
            }
        }
        if support_clusters.len() == 1 {
            item.flags.push(MemoryRecallFlag {
                kind: "narrow_source_support".to_string(),
                edge_id: String::new(),
                edge_type: String::new(),
                related_id: support_clusters.into_iter().next().unwrap_or_default(),
                message: "support derives from one source cluster".to_string(),
            });
        }
    }
    Ok(())
}

fn compare_recall_items(left: &MemoryRecallItem, right: &MemoryRecallItem) -> std::cmp::Ordering {
    right
        .score
        .partial_cmp(&left.score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| right.updated_at.cmp(&left.updated_at))
        .then_with(|| left.id.cmp(&right.id))
}

fn recall_item_graph_id(tenant_slug: &str, item: &MemoryRecallItem) -> String {
    if item.item_type == "node" {
        memory_node_node_id(tenant_slug, &item.id)
    } else {
        memory_document_node_id(tenant_slug, &item.id)
    }
}

fn memory_external_id(graph_id: &str) -> String {
    graph_id.rsplit(':').next().unwrap_or(graph_id).to_string()
}

fn is_broad_recall_query(query: &str) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let broad_markers = [
        "overall",
        "whole state",
        "current state",
        "big picture",
        "summarize",
        "summary",
        "what do we know",
        "how are things",
        "what is going on",
    ];
    broad_markers.iter().any(|marker| query.contains(marker))
}

fn query_has_specific_anchor(query: &str) -> bool {
    let query = query.trim();
    if query.contains('"') || query.contains(':') || query.contains('-') || query.contains('/') {
        return true;
    }
    let generic = BTreeSet::from([
        "about", "all", "are", "current", "going", "how", "know", "memory", "overall", "state",
        "status", "stuff", "summary", "things", "what", "where",
    ]);
    query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_lowercase)
        .any(|token| token.len() >= 3 && !generic.contains(token.as_str()))
}

fn token_cosine_score(query: &str, text: &str) -> f64 {
    let q = token_counts(query);
    let t = token_counts(text);
    if q.is_empty() || t.is_empty() {
        return 0.0;
    }
    let dot = q
        .iter()
        .filter_map(|(token, left)| t.get(token).map(|right| left * right))
        .sum::<f64>();
    let q_norm = q.values().map(|v| v * v).sum::<f64>().sqrt();
    let t_norm = t.values().map(|v| v * v).sum::<f64>().sqrt();
    if q_norm <= 0.0 || t_norm <= 0.0 {
        0.0
    } else {
        dot / (q_norm * t_norm)
    }
}

fn token_counts(text: &str) -> BTreeMap<String, f64> {
    let mut counts = BTreeMap::new();
    for token in text
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|token| token.len() > 1)
    {
        *counts.entry(token).or_insert(0.0) += 1.0;
    }
    counts
}

fn atom_embedding_score(atom: &RecallAtom, query_embedding: &[f32], property: &str) -> f64 {
    let property = if property.trim().is_empty() {
        "embedding"
    } else {
        property.trim()
    };
    let embedding = atom
        .item
        .document
        .as_ref()
        .and_then(|document| embedding_from_metadata(&document.metadata, property))
        .or_else(|| {
            atom.item
                .node
                .as_ref()
                .and_then(|node| embedding_from_metadata(&node.metadata, property))
        });
    let Some(embedding) = embedding else {
        return 0.0;
    };
    cosine_similarity_f32(query_embedding, &embedding).max(0.0)
}

fn embedding_from_metadata(metadata: &Map<String, Value>, property: &str) -> Option<Vec<f32>> {
    metadata
        .get(property)
        .or_else(|| metadata.get("embedding"))
        .or_else(|| metadata.get("vector"))
        .and_then(float_array)
}

fn float_array(value: &Value) -> Option<Vec<f32>> {
    let values = value.as_array()?;
    let mut result = Vec::with_capacity(values.len());
    for value in values {
        result.push(value.as_f64()? as f32);
    }
    Some(result)
}

fn cosine_similarity_f32(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() {
        return 0.0;
    }
    let dot = left
        .iter()
        .zip(right)
        .map(|(l, r)| (*l as f64) * (*r as f64))
        .sum::<f64>();
    let left_norm = left
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    let right_norm = right
        .iter()
        .map(|v| (*v as f64) * (*v as f64))
        .sum::<f64>()
        .sqrt();
    if left_norm <= 0.0 || right_norm <= 0.0 {
        0.0
    } else {
        dot / (left_norm * right_norm)
    }
}

fn edge_propagates(edge: &EdgeRecord) -> bool {
    !edge_is_contradiction_or_tension(edge)
        && matches!(
            normalized_edge_type(edge).as_str(),
            "memory_relates"
                | "memory_supports"
                | "supports"
                | "memory_derived_from"
                | "derives"
                | "derived_from"
                | "memory_cites"
                | "cites"
                | "memory_supercedes"
                | "memory_supersedes"
                | "memory_summarizes"
        )
}

fn edge_propagation_weight(edge: &EdgeRecord) -> f64 {
    match normalized_edge_type(edge).as_str() {
        "memory_supports" | "supports" => 1.0,
        "memory_derived_from" | "derives" | "derived_from" => 0.9,
        "memory_cites" | "cites" => 0.75,
        "memory_summarizes" => 0.7,
        "memory_relates" => 0.65,
        "memory_supercedes" | "memory_supersedes" => 0.4,
        _ => 0.0,
    }
}

fn edge_is_contradiction_or_tension(edge: &EdgeRecord) -> bool {
    matches!(
        edge.epistemic_type,
        Some(EpistemicType::Contradicts) | Some(EpistemicType::Tension)
    ) || matches!(
        normalized_edge_type(edge).as_str(),
        "memory_contradicts" | "contradicts" | "memory_tension" | "tension"
    )
}

fn edge_flag_kind(edge: &EdgeRecord) -> &'static str {
    if matches!(edge.epistemic_type, Some(EpistemicType::Tension))
        || normalized_edge_type(edge).contains("tension")
    {
        "tension"
    } else {
        "contradiction"
    }
}

fn normalized_edge_type(edge: &EdgeRecord) -> String {
    edge.edge_type.trim().to_lowercase()
}

fn edge_valid_at(edge: &EdgeRecord, query_time: &str) -> bool {
    let valid_at = edge_property_text(edge, &["valid_at", "validAt"]);
    let invalid_at = edge_property_text(edge, &["invalid_at", "invalidAt"]);
    if let Some(valid_at) = valid_at.as_deref() {
        if !valid_at.is_empty() && valid_at > query_time {
            return false;
        }
    }
    if let Some(invalid_at) = invalid_at.as_deref() {
        if !invalid_at.is_empty() && invalid_at <= query_time {
            return false;
        }
    }
    true
}

fn edge_property_text(edge: &EdgeRecord, names: &[&str]) -> Option<String> {
    let object = edge.properties.as_object()?;
    for name in names {
        if let Some(value) = object.get(*name).and_then(Value::as_str) {
            return Some(value.trim().to_string());
        }
    }
    None
}

fn edge_property_f64(edge: &EdgeRecord, name: &str) -> Option<f64> {
    edge.properties.as_object()?.get(name)?.as_f64()
}

fn recency_decay(edge: &EdgeRecord, query_time: &str, half_life_seconds: f64) -> f64 {
    if half_life_seconds <= 0.0 {
        return 1.0;
    }
    let Some(valid_at) = edge_property_text(edge, &["valid_at", "validAt"]) else {
        return 1.0;
    };
    let Some(age_seconds) = iso8601_age_seconds(&valid_at, query_time) else {
        return 1.0;
    };
    0.5_f64.powf((age_seconds.max(0.0)) / half_life_seconds)
}

fn iso8601_age_seconds(start: &str, end: &str) -> Option<f64> {
    let start = parse_simple_rfc3339_seconds(start)?;
    let end = parse_simple_rfc3339_seconds(end)?;
    Some((end - start) as f64)
}

fn parse_simple_rfc3339_seconds(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z').unwrap_or(value);
    let (date, time) = value.split_once('T')?;
    let mut date_parts = date.split('-').map(|part| part.parse::<i64>().ok());
    let year = date_parts.next()??;
    let month = date_parts.next()??;
    let day = date_parts.next()??;
    let mut time_parts = time.split(':').map(|part| part.parse::<i64>().ok());
    let hour = time_parts.next()??;
    let minute = time_parts.next()??;
    let second = time_parts.next().and_then(|part| part).unwrap_or(0);
    Some(days_from_civil(year, month, day)? * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let month_adjusted = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * month_adjusted + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

fn fitness_rank_boost(item: &MemoryRecallItem) -> f64 {
    let fitness = item
        .document
        .as_ref()
        .and_then(|document| document.fitness.as_ref())
        .or_else(|| item.node.as_ref().and_then(|node| node.fitness.as_ref()));
    let Some(fitness) = fitness else {
        return 0.0;
    };
    let mut boost = 0.0;
    if fitness
        .get("outcome")
        .and_then(Value::as_str)
        .is_some_and(|outcome| outcome == "positive" || outcome == "accepted")
    {
        boost += 0.15;
    }
    if let Some(positive_count) = fitness
        .get("compound")
        .and_then(|compound| compound.get("positive_count"))
        .and_then(Value::as_f64)
    {
        boost += (positive_count * 0.02).min(0.2);
    }
    boost
}

fn edge_source_cluster(edge: &EdgeRecord, fallback: &str) -> String {
    edge.provenance
        .as_ref()
        .and_then(|provenance| provenance.source_id.clone())
        .or_else(|| {
            edge.properties
                .get("source_cluster")
                .or_else(|| edge.properties.get("source"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| fallback.to_string())
}

fn bounded_seed_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_SEED_LIMIT
    } else {
        limit.clamp(1, MAX_LIMIT)
    }
}

fn effective_ppr_alpha(alpha: f64) -> f64 {
    if alpha > 0.0 {
        alpha.clamp(0.01, 0.99)
    } else {
        DEFAULT_PPR_ALPHA
    }
}

fn effective_ppr_epsilon(epsilon: f64) -> f64 {
    if epsilon > 0.0 {
        epsilon
    } else {
        DEFAULT_PPR_EPSILON
    }
}

fn effective_ppr_max_pushes(max_pushes: usize) -> usize {
    if max_pushes == 0 {
        DEFAULT_PPR_MAX_PUSHES
    } else {
        max_pushes
    }
}

fn effective_recency_half_life(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        DEFAULT_RECENCY_HALF_LIFE_SECONDS
    }
}

fn ensure_community_summaries<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    query_time: &str,
) -> MemoryResult<()> {
    let existing = load_memory_documents(store, tenant_slug, true)?
        .into_iter()
        .any(|document| {
            document.kind == COMMUNITY_SUMMARY_KIND && document.status == DEFAULT_STATUS
        });
    if existing {
        return Ok(());
    }
    recompute_memory_community_summaries(store, tenant_slug, query_time).map(|_| ())
}

pub fn recompute_memory_community_summaries<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    query_time: &str,
) -> MemoryResult<Vec<MemoryDocumentState>> {
    let tenant_slug = normalize_tenant_slug(tenant_slug);
    let atoms = load_recall_atoms(store, &tenant_slug, "", "", "", "", true)?
        .into_iter()
        .filter(|atom| atom.item.kind != COMMUNITY_SUMMARY_KIND)
        .collect::<Vec<_>>();
    if atoms.is_empty() {
        return Ok(Vec::new());
    }
    let atom_map = atoms
        .iter()
        .map(|atom| (atom.graph_id.clone(), atom.clone()))
        .collect::<HashMap<_, _>>();
    let mut edge_map = BTreeMap::new();
    for graph_id in atom_map.keys() {
        for hit in store.memory_neighbors(NeighborQuery::out(graph_id))? {
            if !atom_map.contains_key(&hit.node_id) {
                continue;
            }
            let Some(edge) = store.memory_get_edge(&hit.edge_id)? else {
                continue;
            };
            if edge_valid_at(&edge, query_time) && edge_propagates(&edge) {
                edge_map.insert(edge.id.clone(), edge);
            }
        }
    }
    let (labels, _) = if edge_map.is_empty() {
        (HashMap::new(), 0.0)
    } else {
        let edges = edge_map.values().cloned().collect::<Vec<_>>();
        rustyred_thg_core::label_propagation_communities(&edges)
    };
    let mut groups: BTreeMap<u64, Vec<RecallAtom>> = BTreeMap::new();
    let mut next_group = labels.values().copied().max().unwrap_or(0) + 1;
    for atom in atoms {
        let group = labels.get(&atom.graph_id).copied().unwrap_or_else(|| {
            let group = next_group;
            next_group += 1;
            group
        });
        groups.entry(group).or_default().push(atom);
    }

    let mut summaries = Vec::new();
    for (community_id, members) in &groups {
        let doc_id = format!("community_l1_{community_id}");
        let document = community_summary_document(
            &tenant_slug,
            &doc_id,
            1,
            &format!("Community {community_id}"),
            members,
            query_time,
        );
        persist_memory_document(store, &document)?;
        let summary_graph_id = memory_document_node_id(&tenant_slug, &document.doc_id);
        for member in members {
            upsert_memory_edge(
                store,
                &tenant_slug,
                COMMUNITY_SUMMARY_EDGE,
                &summary_graph_id,
                &member.graph_id,
                json!({
                    "source": "community_summary",
                    "valid_at": query_time,
                    "community_id": community_id
                }),
            )?;
        }
        summaries.push(document);
    }

    let level_one_atoms = summaries
        .iter()
        .map(|document| RecallAtom {
            graph_id: memory_document_node_id(&tenant_slug, &document.doc_id),
            item: recall_item_for_document(document.clone()),
            text: memory_document_text(document),
        })
        .collect::<Vec<_>>();
    let top = community_summary_document(
        &tenant_slug,
        "community_l2_overall",
        2,
        "Overall memory state",
        &level_one_atoms,
        query_time,
    );
    persist_memory_document(store, &top)?;
    let top_graph_id = memory_document_node_id(&tenant_slug, &top.doc_id);
    for summary in &summaries {
        upsert_memory_edge(
            store,
            &tenant_slug,
            COMMUNITY_SUMMARY_EDGE,
            &top_graph_id,
            &memory_document_node_id(&tenant_slug, &summary.doc_id),
            json!({
                "source": "community_summary",
                "valid_at": query_time,
                "level": 2
            }),
        )?;
    }
    summaries.push(top);
    Ok(summaries)
}

fn community_summary_document(
    tenant_slug: &str,
    doc_id: &str,
    level: u64,
    title: &str,
    members: &[RecallAtom],
    updated_at: &str,
) -> MemoryDocumentState {
    let member_titles = members
        .iter()
        .take(8)
        .map(|member| {
            if member.item.summary.trim().is_empty() {
                member.item.title.clone()
            } else {
                format!("{}: {}", member.item.title, member.item.summary)
            }
        })
        .collect::<Vec<_>>();
    let content = member_titles.join("\n");
    let mut metadata = Map::new();
    metadata.insert("summary_level".to_string(), json!(level));
    metadata.insert(
        "member_ids".to_string(),
        Value::Array(
            members
                .iter()
                .map(|member| Value::String(member.graph_id.clone()))
                .collect(),
        ),
    );
    MemoryDocumentState {
        tenant_slug: tenant_slug.to_string(),
        doc_id: doc_id.to_string(),
        kind: COMMUNITY_SUMMARY_KIND.to_string(),
        title: title.to_string(),
        content,
        summary: format!("{} memory atoms", members.len()),
        tags: vec!["community-summary".to_string()],
        links: Vec::new(),
        actor_id: "theorem-harness".to_string(),
        session_id: String::new(),
        origin_surface: "memory_recall_pipeline".to_string(),
        project_slug: String::new(),
        status: DEFAULT_STATUS.to_string(),
        memory_node_type: "summary".to_string(),
        target_actor_id: String::new(),
        expires_at: String::new(),
        metadata,
        fitness: None,
        created_at: updated_at.to_string(),
        updated_at: updated_at.to_string(),
        deleted_reason: String::new(),
        deleted_at: String::new(),
    }
}

fn bump_recall_salience<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    results: &[MemoryRecallItem],
    recalled_at: &str,
) -> MemoryResult<()> {
    for item in results {
        if item.item_type == "node" {
            if let Some(mut node) = load_memory_node(store, tenant_slug, &item.id)? {
                increment_salience(&mut node.metadata, recalled_at);
                persist_memory_node(store, &node)?;
            }
        } else if let Some(mut document) = load_memory_document(store, tenant_slug, &item.id)? {
            increment_salience(&mut document.metadata, recalled_at);
            persist_memory_document(store, &document)?;
        }
    }
    Ok(())
}

fn increment_salience(metadata: &mut Map<String, Value>, recalled_at: &str) {
    let count = metadata
        .get("salience")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    metadata.insert("salience".to_string(), json!(count));
    metadata.insert("last_recalled_at".to_string(), json!(recalled_at));
}

fn invalidate_positive_edges_for_targets<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    target_doc_ids: &[String],
    invalid_at: &str,
) -> MemoryResult<()> {
    for target_doc_id in normalize_strings(target_doc_ids) {
        let Some(target_graph_id) = resolve_memory_graph_id(store, tenant_slug, &target_doc_id)?
        else {
            continue;
        };
        for direction in [Direction::Out, Direction::In] {
            for hit in store.memory_neighbors(NeighborQuery {
                node_id: target_graph_id.clone(),
                direction,
                edge_type: None,
                include_expired: false,
            })? {
                let Some(mut edge) = store.memory_get_edge(&hit.edge_id)? else {
                    continue;
                };
                if !edge_propagates(&edge) {
                    continue;
                }
                set_edge_invalid_at(&mut edge, invalid_at);
                store.memory_upsert_edge(edge)?;
            }
        }
    }
    Ok(())
}

fn set_edge_invalid_at(edge: &mut EdgeRecord, invalid_at: &str) {
    let mut properties = edge.properties.as_object().cloned().unwrap_or_default();
    properties
        .entry("valid_at".to_string())
        .or_insert_with(|| Value::String(invalid_at.to_string()));
    properties.insert(
        "invalid_at".to_string(),
        Value::String(invalid_at.to_string()),
    );
    edge.properties = Value::Object(properties);
}

fn relation_item_from_graph_node<S: MemoryGraphStore>(
    store: &S,
    node_id: &str,
    hit: &NeighborHit,
    depth: usize,
) -> MemoryResult<Option<MemoryRelationItem>> {
    let Some(node) = store.memory_get_node(node_id)? else {
        return Ok(None);
    };
    if node.labels.iter().any(|label| label == "MemoryDocument") {
        let document = document_from_node(node)?;
        return Ok(Some(MemoryRelationItem {
            node_id: node_id.to_string(),
            id: document.doc_id.clone(),
            item_type: "document".to_string(),
            kind: document.kind.clone(),
            title: document.title.clone(),
            summary: document.summary.clone(),
            edge_id: hit.edge_id.clone(),
            edge_type: hit.edge_type.clone(),
            depth,
            document: Some(document),
            node: None,
        }));
    }
    if node.labels.iter().any(|label| label == "MemoryNode") {
        let memory_node = node_from_node(node)?;
        return Ok(Some(MemoryRelationItem {
            node_id: node_id.to_string(),
            id: memory_node.node_id.clone(),
            item_type: "node".to_string(),
            kind: memory_node.kind.clone(),
            title: memory_node.title.clone(),
            summary: String::new(),
            edge_id: hit.edge_id.clone(),
            edge_type: hit.edge_type.clone(),
            depth,
            document: None,
            node: Some(memory_node),
        }));
    }
    Ok(None)
}

fn resolve_memory_graph_id<S: MemoryGraphStore>(
    store: &S,
    tenant_slug: &str,
    id: &str,
) -> MemoryResult<Option<String>> {
    let id = id.trim();
    if id.is_empty() {
        return Ok(None);
    }
    if store.memory_get_node(id)?.is_some() {
        return Ok(Some(id.to_string()));
    }
    let document_id = memory_document_node_id(tenant_slug, id);
    if store.memory_get_node(&document_id)?.is_some() {
        return Ok(Some(document_id));
    }
    let node_id = memory_node_node_id(tenant_slug, id);
    if store.memory_get_node(&node_id)?.is_some() {
        return Ok(Some(node_id));
    }
    Ok(None)
}

fn link_doc_id_list<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    source_doc_id: &str,
    edge_type: &str,
    target_doc_ids: &[String],
) -> MemoryResult<()> {
    let source_id = memory_document_node_id(tenant_slug, source_doc_id);
    for target in normalize_strings(target_doc_ids) {
        if let Some(target_id) = resolve_memory_graph_id(store, tenant_slug, &target)? {
            upsert_memory_edge(
                store,
                tenant_slug,
                edge_type,
                &source_id,
                &target_id,
                json!({ "source": "self_revise" }),
            )?;
        }
    }
    Ok(())
}

fn upsert_memory_edge<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    edge_type: &str,
    from_id: &str,
    to_id: &str,
    properties: Value,
) -> MemoryResult<()> {
    if store.memory_get_node(from_id)?.is_none() || store.memory_get_node(to_id)?.is_none() {
        return Ok(());
    }
    let properties = memory_edge_properties(edge_type, properties);
    let mut edge = EdgeRecord::new(
        memory_edge_id(tenant_slug, edge_type, from_id, to_id),
        from_id,
        edge_type,
        to_id,
        properties,
    );
    if let Some(confidence) = edge_property_f64(&edge, "confidence") {
        edge = edge.with_confidence(confidence);
    }
    if let Some(epistemic_type) = epistemic_type_for_edge(edge_type) {
        edge = edge.with_epistemic_type(epistemic_type);
    }
    upsert_edge_if_changed(store, edge)?;
    Ok(())
}

fn memory_edge_properties(edge_type: &str, properties: Value) -> Value {
    let mut map = properties.as_object().cloned().unwrap_or_default();
    let valid_at = map
        .get("valid_at")
        .or_else(|| map.get("validAt"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            map.get("updated_at")
                .or_else(|| map.get("updatedAt"))
                .or_else(|| map.get("created_at"))
                .or_else(|| map.get("createdAt"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| timestamp_or_now(""));
    map.insert("valid_at".to_string(), Value::String(valid_at));
    map.insert(
        "edge_semantics".to_string(),
        Value::String(edge_type.to_string()),
    );
    Value::Object(map)
}

fn epistemic_type_for_edge(edge_type: &str) -> Option<EpistemicType> {
    match edge_type.trim().to_lowercase().as_str() {
        "memory_supports" | "supports" => Some(EpistemicType::Supports),
        "memory_contradicts" | "contradicts" => Some(EpistemicType::Contradicts),
        "memory_tension" | "tension" => Some(EpistemicType::Tension),
        "memory_derived_from" | "derives" | "derived_from" => Some(EpistemicType::Derives),
        "memory_cites" | "cites" => Some(EpistemicType::Cites),
        _ => None,
    }
}

fn upsert_node_if_changed<S: MemoryGraphStore>(
    store: &mut S,
    node: NodeRecord,
) -> GraphStoreResult<()> {
    let unchanged = store
        .memory_get_node(&node.id)?
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.memory_upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed<S: MemoryGraphStore>(
    store: &mut S,
    edge: EdgeRecord,
) -> GraphStoreResult<()> {
    let unchanged = store.memory_get_node(&edge.from_id)?.is_some()
        && store.memory_get_node(&edge.to_id)?.is_some();
    if unchanged {
        store.memory_upsert_edge(edge)?;
    }
    Ok(())
}

fn insert_search_text(
    properties: &mut Value,
    title: &str,
    content: &str,
    summary: &str,
    tags: &[String],
) {
    let text = [title, summary, content, &tags.join(" ")]
        .into_iter()
        .filter(|part| !part.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    if let Value::Object(map) = properties {
        map.insert("search_text".to_string(), Value::String(text));
    }
}

pub fn normalize_tenant_slug(value: &str) -> String {
    let value = value.trim().to_lowercase();
    if value.is_empty() {
        DEFAULT_TENANT.to_string()
    } else {
        value
    }
}

fn normalize_kind(value: &str, field: &str) -> MemoryResult<String> {
    let value = value.trim().to_lowercase();
    if value.is_empty() {
        return Err(MemoryError::InvalidInput {
            field: field.to_string(),
            message: "is required".to_string(),
        });
    }
    Ok(value)
}

fn normalize_encode_kind(value: &str) -> MemoryResult<String> {
    let value = if value.trim().is_empty() {
        "encode".to_string()
    } else {
        value.trim().to_lowercase()
    };
    if matches!(
        value.as_str(),
        "encode" | "feedback" | "solution" | "postmortem"
    ) {
        Ok(value)
    } else {
        Err(MemoryError::InvalidInput {
            field: "kind".to_string(),
            message: "must be encode, feedback, solution, or postmortem".to_string(),
        })
    }
}

fn normalize_outcome(value: &str) -> MemoryResult<String> {
    let value = if value.trim().is_empty() {
        "neutral".to_string()
    } else {
        value.trim().to_lowercase()
    };
    if matches!(
        value.as_str(),
        "positive" | "negative" | "mixed" | "neutral"
    ) {
        Ok(value)
    } else {
        Err(MemoryError::InvalidInput {
            field: "outcome".to_string(),
            message: "must be positive, negative, mixed, or neutral".to_string(),
        })
    }
}

fn normalize_status(value: &str) -> MemoryResult<String> {
    let value = if value.trim().is_empty() {
        DEFAULT_STATUS.to_string()
    } else {
        value.trim().to_lowercase()
    };
    if DOCUMENT_STATUSES.contains(&value.as_str()) {
        Ok(value)
    } else {
        Err(MemoryError::InvalidInput {
            field: "status".to_string(),
            message: format!("must be one of {:?}", DOCUMENT_STATUSES),
        })
    }
}

fn normalize_strings(values: &[String]) -> Vec<String> {
    let mut seen = BTreeSet::new();
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

fn bounded_limit(limit: usize) -> usize {
    if limit == 0 {
        DEFAULT_LIMIT
    } else {
        limit.clamp(1, MAX_LIMIT)
    }
}

fn is_node_memory_kind(kind: &str) -> bool {
    NODE_MEMORY_KINDS.contains(&kind)
}

fn is_low_fitness(fitness: Option<&Value>) -> bool {
    let Some(fitness) = fitness else {
        return false;
    };
    fitness
        .get("outcome")
        .and_then(Value::as_str)
        .map(|outcome| outcome == "negative")
        .unwrap_or(false)
        || fitness
            .get("compound")
            .and_then(|value| value.get("low_fitness"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
}

fn bump_recalled_compound_fitness<S: MemoryGraphStore>(
    store: &mut S,
    tenant_slug: &str,
    items: &[MemoryRecallItem],
) -> MemoryResult<()> {
    for item in items {
        match item.item_type.as_str() {
            "document" => {
                let Some(mut document) = load_memory_document(store, tenant_slug, &item.id)? else {
                    continue;
                };
                if clear_compound_low_fitness(document.fitness.as_mut()) {
                    persist_memory_document(store, &document)?;
                }
            }
            "node" => {
                let Some(mut node) = load_memory_node(store, tenant_slug, &item.id)? else {
                    continue;
                };
                if clear_compound_low_fitness(node.fitness.as_mut()) {
                    persist_memory_node(store, &node)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn clear_compound_low_fitness(fitness: Option<&mut Value>) -> bool {
    let Some(Value::Object(fitness)) = fitness else {
        return false;
    };
    let Some(Value::Object(compound)) = fitness.get_mut("compound") else {
        return false;
    };
    if compound
        .get("low_fitness")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        compound.insert("low_fitness".to_string(), Value::Bool(false));
        compound.insert("rehearsed_by_recall".to_string(), Value::Bool(true));
        return true;
    }
    false
}

fn require_text(field: &str, value: &str) -> MemoryResult<String> {
    let value = value.trim();
    if value.is_empty() {
        Err(MemoryError::InvalidInput {
            field: field.to_string(),
            message: "is required".to_string(),
        })
    } else {
        Ok(value.to_string())
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

fn score_match(query: &str, title: &str, content: &str, summary: &str) -> f64 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 1.0;
    }
    let haystack = format!("{title}\n{summary}\n{content}").to_lowercase();
    let mut score = 0.0;
    for token in query.split_whitespace() {
        if haystack.contains(token) {
            score += 1.0;
        }
    }
    score
}

fn stable_document_id(
    tenant_slug: &str,
    kind: &str,
    title: &str,
    content: &str,
    created_at: &str,
) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": tenant_slug,
        "kind": kind,
        "title": title.trim(),
        "content": content,
        "created_at": created_at,
    }));
    format!("doc_{}", hash_prefix(&hash))
}

fn stable_memory_node_id(
    tenant_slug: &str,
    kind: &str,
    title: &str,
    content: &str,
    created_at: &str,
) -> String {
    let hash = stable_value_hash(&json!({
        "tenant_slug": tenant_slug,
        "kind": kind,
        "title": title.trim(),
        "content": content,
        "created_at": created_at,
    }));
    format!("node_{}", hash_prefix(&hash))
}

fn hash_prefix(hash: &str) -> String {
    hash.chars().take(16).collect()
}

fn slugify(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for character in value.trim().to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
            previous_dash = false;
        } else if !previous_dash {
            slug.push('-');
            previous_dash = true;
        }
        if slug.len() >= 96 {
            break;
        }
    }
    slug.trim_matches('-').to_string()
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

trait IfEmpty {
    fn if_empty(self, fallback: &str) -> String;
}

impl IfEmpty for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.to_string()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, RedCoreGraphStore, RedCoreOptions};
    use std::fs;

    const TENANT: &str = "travis-gilbert";
    const T1: &str = "2026-06-01T00:00:00Z";
    const T2: &str = "2026-06-01T00:01:00Z";
    const T3: &str = "2026-06-01T00:02:00Z";

    #[test]
    fn recall_stage0_seeds_named_entities_from_union() {
        let mut store = InMemoryGraphStore::new();
        let alpha = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Alpha project".to_string(),
                content: "First anchor.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let beta = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Beta project".to_string(),
                content: "Second anchor.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        let results = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha Beta".to_string(),
                query_time: T2.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        let alpha_seed = results.iter().find(|item| item.id == alpha.doc_id).unwrap();
        let beta_seed = results.iter().find(|item| item.id == beta.doc_id).unwrap();
        assert!(alpha_seed.rank_signals["seed_mass"].as_f64().unwrap() > 0.0);
        assert!(beta_seed.rank_signals["seed_mass"].as_f64().unwrap() > 0.0);
    }

    #[test]
    fn recall_stage1_masks_edges_by_half_open_valid_time() {
        let mut store = InMemoryGraphStore::new();
        let seed = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Alpha seed".to_string(),
                content: "Anchor only.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let fact = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Temporal fact".to_string(),
                content: "Only reachable through valid support.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        upsert_memory_edge(
            &mut store,
            TENANT,
            "MEMORY_SUPPORTS",
            &memory_document_node_id(TENANT, &seed.doc_id),
            &memory_document_node_id(TENANT, &fact.doc_id),
            json!({ "valid_at": T1, "invalid_at": T3, "source": "fixture" }),
        )
        .unwrap();

        let before = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha".to_string(),
                query_time: T2.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(before.iter().any(|item| item.id == fact.doc_id));

        let after = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha".to_string(),
                query_time: T3.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(!after.iter().any(|item| item.id == fact.doc_id));
    }

    #[test]
    fn recall_stage2_flags_contradictions_without_propagating_them() {
        let mut store = InMemoryGraphStore::new();
        let seed = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Alpha claim".to_string(),
                content: "Anchor claim.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let contradicted = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Remote claim".to_string(),
                content: "This should not receive corroborating mass.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        upsert_memory_edge(
            &mut store,
            TENANT,
            "MEMORY_CONTRADICTS",
            &memory_document_node_id(TENANT, &seed.doc_id),
            &memory_document_node_id(TENANT, &contradicted.doc_id),
            json!({ "valid_at": T1, "source": "fixture" }),
        )
        .unwrap();

        let results = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha".to_string(),
                query_time: T2.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        let seed_result = results.iter().find(|item| item.id == seed.doc_id).unwrap();
        assert!(seed_result
            .flags
            .iter()
            .any(|flag| flag.kind == "contradiction"));
        assert!(!results.iter().any(|item| item.id == contradicted.doc_id));
    }

    #[test]
    fn recall_stage2_adds_trust_flag_for_single_source_cluster_support() {
        let mut store = InMemoryGraphStore::new();
        let seed = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Alpha source".to_string(),
                content: "Anchor source.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let supported = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Supported claim".to_string(),
                content: "Reached through one source cluster.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        upsert_memory_edge(
            &mut store,
            TENANT,
            "MEMORY_SUPPORTS",
            &memory_document_node_id(TENANT, &seed.doc_id),
            &memory_document_node_id(TENANT, &supported.doc_id),
            json!({ "valid_at": T1, "source_cluster": "cluster-a", "confidence": 0.9 }),
        )
        .unwrap();

        let results = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha".to_string(),
                query_time: T2.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        let supported_result = results
            .iter()
            .find(|item| item.id == supported.doc_id)
            .unwrap();
        assert!(supported_result
            .flags
            .iter()
            .any(|flag| flag.kind == "narrow_source_support"));
    }

    #[test]
    fn recall_stage3_routes_broad_queries_to_two_level_summaries() {
        let mut store = InMemoryGraphStore::new();
        let first = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Budget thread".to_string(),
                content: "Finance lane details.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let second = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Deploy thread".to_string(),
                content: "Infrastructure lane details.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        upsert_memory_edge(
            &mut store,
            TENANT,
            "MEMORY_RELATES",
            &memory_document_node_id(TENANT, &first.doc_id),
            &memory_document_node_id(TENANT, &second.doc_id),
            json!({ "valid_at": T1, "source": "fixture" }),
        )
        .unwrap();

        let results = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "overall state".to_string(),
                query_time: T2.to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(!results.is_empty());
        assert!(results
            .iter()
            .all(|item| item.kind == COMMUNITY_SUMMARY_KIND));
        let levels = results
            .iter()
            .filter_map(|item| {
                item.document
                    .as_ref()
                    .and_then(|document| document.metadata["summary_level"].as_u64())
            })
            .collect::<BTreeSet<_>>();
        assert!(levels.contains(&1));
        assert!(levels.contains(&2));
    }

    #[test]
    fn recall_bumps_salience_as_rehearsal() {
        let mut store = InMemoryGraphStore::new();
        let document = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Alpha rehearsal".to_string(),
                content: "Recall should bump salience.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "Alpha".to_string(),
                query_time: T2.to_string(),
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        let loaded = load_memory_document(&store, TENANT, &document.doc_id)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.metadata["salience"], json!(1));
        assert_eq!(loaded.metadata["last_recalled_at"], json!(T2));
    }

    #[test]
    fn contradiction_revision_invalidates_positive_edges_additively() {
        let mut store = InMemoryGraphStore::new();
        let original = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Original".to_string(),
                content: "Old framing.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let target = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Target".to_string(),
                content: "Superseded target.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let dependent = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "claim".to_string(),
                title: "Dependent".to_string(),
                content: "Depends on target.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let target_graph_id = memory_document_node_id(TENANT, &target.doc_id);
        let dependent_graph_id = memory_document_node_id(TENANT, &dependent.doc_id);
        upsert_memory_edge(
            &mut store,
            TENANT,
            "MEMORY_SUPPORTS",
            &target_graph_id,
            &dependent_graph_id,
            json!({ "valid_at": T1, "source": "fixture" }),
        )
        .unwrap();
        let support_edge_id = memory_edge_id(
            TENANT,
            "MEMORY_SUPPORTS",
            &target_graph_id,
            &dependent_graph_id,
        );

        revise_memory_document(
            &mut store,
            ReviseMemoryInput {
                tenant_slug: TENANT.to_string(),
                doc_id: original.doc_id,
                content: "New framing contradicts target.".to_string(),
                contradicts_doc_ids: vec![target.doc_id],
                updated_at: T2.to_string(),
                ..ReviseMemoryInput::default()
            },
        )
        .unwrap();

        let edge = store.get_edge(&support_edge_id).unwrap();
        assert_eq!(edge.properties["valid_at"], json!(T1));
        assert_eq!(edge.properties["invalid_at"], json!(T2));
        assert!(!edge.tombstone);
    }

    #[test]
    fn remember_recall_documents_and_nodes_with_provenance() {
        let mut store = InMemoryGraphStore::new();
        let document = remember_memory(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                origin_surface: "codex".to_string(),
                kind: "insight".to_string(),
                title: "Memory port".to_string(),
                content: "Native RedCore memory atoms are available.".to_string(),
                tags: vec!["memory".to_string(), "rust".to_string()],
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        assert_eq!(document.saved_type, "document");

        let node = remember_memory(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                kind: "claim".to_string(),
                title: "Claim".to_string(),
                content: "Recall covers memory nodes too.".to_string(),
                created_at: T2.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        assert_eq!(node.saved_type, "node");

        let results = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "memory".to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|item| item.item_type == "document"));
        assert!(results.iter().any(|item| item.item_type == "node"));
        assert_eq!(results[0].provenance["actor"], "codex");
    }

    #[test]
    fn revise_archive_forget_and_archive_recall_filter_statuses() {
        let mut store = InMemoryGraphStore::new();
        let document = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                kind: "self_note".to_string(),
                title: "Initial belief".to_string(),
                content: "Memory is still Python backed.".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        let revised = revise_memory_document(
            &mut store,
            ReviseMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                doc_id: document.doc_id.clone(),
                content: "Memory is now RedCore backed.".to_string(),
                reason: "native port landed".to_string(),
                updated_at: T2.to_string(),
                ..ReviseMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(revised.superseded.status, "superseded");

        let active = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "RedCore".to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, revised.revised.doc_id);

        let archived = archive_memory_document(
            &mut store,
            ArchiveMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                doc_id: revised.revised.doc_id.clone(),
                reason: "cold tier".to_string(),
                archived_at: T2.to_string(),
                ..ArchiveMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(archived.archived.status, "archived");
        assert!(recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                query: "RedCore".to_string(),
                ..RecallMemoryInput::default()
            },
        )
        .unwrap()
        .is_empty());
        assert_eq!(
            recall_archived_memory(
                &mut store,
                RecallMemoryInput {
                    tenant_slug: TENANT.to_string(),
                    query: "RedCore".to_string(),
                    ..RecallMemoryInput::default()
                },
            )
            .unwrap()
            .len(),
            2
        );

        let forgotten = forget_memory(
            &mut store,
            ForgetMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                id: archived.archive.doc_id,
                reason: "test delete".to_string(),
                deleted_at: T2.to_string(),
            },
        )
        .unwrap();
        assert_eq!(forgotten.forgotten_type, "document");
        assert_eq!(forgotten.document.unwrap().status, "deleted");
    }

    #[test]
    fn handoff_recall_can_consume_targeted_handoffs() {
        let mut store = InMemoryGraphStore::new();
        let handoff = handoff_memory(
            &mut store,
            HandoffMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor_id: "codex".to_string(),
                to_actor: "claude-code".to_string(),
                payload: json!({ "next": "deploy write mode" }),
                created_at: T1.to_string(),
                ..HandoffMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(handoff.kind, "handoff");

        let handoffs = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor: "claude-code".to_string(),
                kind: "handoff".to_string(),
                consume_handoffs: true,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(handoffs.len(), 1);
        assert!(recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: TENANT.to_string(),
                actor: "claude-code".to_string(),
                kind: "handoff".to_string(),
                ..RecallMemoryInput::default()
            },
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn relate_walks_memory_edges() {
        let mut store = InMemoryGraphStore::new();
        let first = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "First".to_string(),
                content: "First memory".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        let second = create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "insight".to_string(),
                title: "Second".to_string(),
                content: "Second memory".to_string(),
                links: vec![first.doc_id.clone()],
                created_at: T2.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        let related = relate_memory(
            &store,
            RelateMemoryInput {
                tenant_slug: TENANT.to_string(),
                seed_id: second.doc_id,
                edge_types: vec!["MEMORY_RELATES".to_string()],
                max_hops: 1,
            },
        )
        .unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, first.doc_id);
    }

    #[test]
    fn redcore_reopens_memory_documents_nodes_and_edges() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-harness-memory-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let doc_id;
        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let document = create_memory_document(
                &mut store,
                MemoryWriteInput {
                    tenant_slug: TENANT.to_string(),
                    actor_id: "codex".to_string(),
                    kind: "insight".to_string(),
                    title: "Persistent memory".to_string(),
                    content: "RedCore can reopen memory atoms.".to_string(),
                    created_at: T1.to_string(),
                    ..MemoryWriteInput::default()
                },
            )
            .unwrap();
            doc_id = document.doc_id.clone();
            encode_memory(
                &mut store,
                MemoryWriteInput {
                    tenant_slug: TENANT.to_string(),
                    actor_id: "codex".to_string(),
                    kind: "solution".to_string(),
                    title: "Good outcome".to_string(),
                    content: "Persist the useful lesson.".to_string(),
                    created_at: T2.to_string(),
                    ..MemoryWriteInput::default()
                },
                EncodeMemoryInput {
                    outcome: "positive".to_string(),
                    signal: "useful".to_string(),
                    reason: "test".to_string(),
                    event_id: "event-1".to_string(),
                    context: json!({ "run_id": "run-1" }),
                    auto_triggered: true,
                },
            )
            .unwrap();
        }
        {
            let mut store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let loaded = load_memory_document(&store, TENANT, &doc_id)
                .unwrap()
                .unwrap();
            assert_eq!(loaded.title, "Persistent memory");
            let recalled = recall_memory(
                &mut store,
                RecallMemoryInput {
                    tenant_slug: TENANT.to_string(),
                    query: "lesson".to_string(),
                    include_low_fitness: true,
                    ..RecallMemoryInput::default()
                },
            )
            .unwrap();
            assert_eq!(recalled.len(), 1);
            assert_eq!(
                recalled[0]
                    .document
                    .as_ref()
                    .unwrap()
                    .fitness
                    .as_ref()
                    .unwrap()["outcome"],
                "positive"
            );
        }
        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn list_memory_documents_since_filters_by_watermark() {
        let mut store = InMemoryGraphStore::new();
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "note".to_string(),
                title: "Old".to_string(),
                content: "old body".to_string(),
                created_at: T1.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: TENANT.to_string(),
                kind: "note".to_string(),
                title: "New".to_string(),
                content: "new body".to_string(),
                created_at: T2.to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        assert_eq!(
            list_memory_documents_since(&store, TENANT, "", false)
                .unwrap()
                .len(),
            2
        );
        let since = list_memory_documents_since(&store, TENANT, T2, false).unwrap();
        assert_eq!(since.len(), 1);
        assert_eq!(since[0].title, "New");
    }

    #[test]
    fn upsert_note_round_trips_doc_id_and_reconciles_links() {
        let mut store = InMemoryGraphStore::new();
        let target = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                title: "Target".to_string(),
                content: "target body".to_string(),
                created_at: T1.to_string(),
                updated_at: T1.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        assert_eq!(target.action, "created");

        let source = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                title: "Source".to_string(),
                content: "source body".to_string(),
                links: vec![target.document.doc_id.clone(), "Future".to_string()],
                created_at: T1.to_string(),
                updated_at: T2.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        assert_eq!(source.action, "created");
        assert_eq!(source.resolved_links, vec![target.document.doc_id.clone()]);
        assert_eq!(source.unresolved_links, vec!["Future".to_string()]);
        let source_doc_id = source.document.doc_id.clone();

        // Forward reference resolves when the target note is created.
        let future = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                title: "Future".to_string(),
                content: "future body".to_string(),
                updated_at: T2.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        assert!(future.reconciled_back.contains(&source_doc_id));

        // Updating by doc_id keeps the same identity (no supersede churn).
        let updated = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                doc_id: source_doc_id.clone(),
                title: "Source".to_string(),
                content: "source body v2".to_string(),
                links: vec![target.document.doc_id.clone()],
                updated_at: T2.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        assert_eq!(updated.action, "updated");
        assert_eq!(updated.document.doc_id, source_doc_id);
        assert_eq!(updated.document.content, "source body v2");

        let related = relate_memory(
            &store,
            RelateMemoryInput {
                tenant_slug: TENANT.to_string(),
                seed_id: source_doc_id.clone(),
                edge_types: vec!["MEMORY_RELATES".to_string()],
                max_hops: 1,
            },
        )
        .unwrap();
        assert!(related.iter().any(|item| item.id == target.document.doc_id));
    }

    #[test]
    fn upsert_note_removes_dropped_links() {
        let mut store = InMemoryGraphStore::new();
        let target = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                title: "A".to_string(),
                content: "a".to_string(),
                updated_at: T1.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        let source = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                title: "S".to_string(),
                content: "s".to_string(),
                links: vec![target.document.doc_id.clone()],
                updated_at: T1.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();

        let updated = upsert_note(
            &mut store,
            UpsertNoteInput {
                tenant_slug: TENANT.to_string(),
                doc_id: source.document.doc_id.clone(),
                title: "S".to_string(),
                content: "s".to_string(),
                links: Vec::new(),
                updated_at: T2.to_string(),
                ..UpsertNoteInput::default()
            },
        )
        .unwrap();
        assert_eq!(updated.removed_links.len(), 1);

        let related = relate_memory(
            &store,
            RelateMemoryInput {
                tenant_slug: TENANT.to_string(),
                seed_id: source.document.doc_id.clone(),
                edge_types: vec!["MEMORY_RELATES".to_string()],
                max_hops: 1,
            },
        )
        .unwrap();
        assert!(related.is_empty());
    }
}
