use rustyred_thg_core::{
    Direction, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NeighborHit,
    NeighborQuery, NodeQuery, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, VecDeque};
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

pub trait MemoryGraphStore {
    fn memory_upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<()>;
    fn memory_upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<()>;
    fn memory_get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn memory_query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>>;
    fn memory_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub document: Option<MemoryDocumentState>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<MemoryNodeState>,
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
    let mut results = Vec::new();

    for document in load_memory_documents(store, &tenant_slug, true)? {
        if !document_matches_recall(
            &document,
            &query,
            &kind_filter,
            &surface_filter,
            &actor_filter,
            &since,
            input.include_low_fitness,
        ) {
            continue;
        }
        results.push(recall_item_for_document(document));
    }
    for node in load_memory_nodes(store, &tenant_slug, true)? {
        if !node_matches_recall(
            &node,
            &query,
            &kind_filter,
            &surface_filter,
            &actor_filter,
            &since,
            input.include_low_fitness,
        ) {
            continue;
        }
        results.push(recall_item_for_node(node));
    }

    for item in &mut results {
        item.score = score_match(&query, &item.title, &item.content, &item.summary);
    }
    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    results.truncate(limit);

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
        ["HarnessMemory", "MemoryDocument"],
        properties,
    ))
}

fn memory_node_node(node: &MemoryNodeState) -> MemoryResult<NodeRecord> {
    let mut properties = serde_json::to_value(node)
        .map_err(|error| MemoryError::Serialization(error.to_string()))?;
    insert_search_text(&mut properties, &node.title, &node.content, "", &node.tags);
    Ok(NodeRecord::new(
        memory_node_node_id(&node.tenant_slug, &node.node_id),
        ["HarnessMemory", "MemoryNode"],
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
        document: None,
        node: Some(node),
    }
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
    let edge = EdgeRecord::new(
        memory_edge_id(tenant_slug, edge_type, from_id, to_id),
        from_id,
        edge_type,
        to_id,
        properties,
    );
    upsert_edge_if_changed(store, edge)?;
    Ok(())
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

fn normalize_tenant_slug(value: &str) -> String {
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
}
