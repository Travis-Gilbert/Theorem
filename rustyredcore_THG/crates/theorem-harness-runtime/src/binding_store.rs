//! GraphStore persistence for the composed-agent `AgentBinding`.
//!
//! The runtime seam for the binding kernel in `theorem-harness-core`
//! (`agent_binding.rs`): persist an `AgentBinding`, its lifecycle events, and
//! its versioned scratchpad revisions as graph nodes plus append-chain edges,
//! so a composed agent's binding survives a `RedCoreGraphStore` reopen and
//! replays deterministically. This is the sibling of `event_log.rs` (which
//! persists the single-agent `RunState`/`EventState`); it follows the same
//! idempotent upsert + contiguous-append + conflict-detection discipline.
//!
//! Storage mapping (spec Part 6): the in-process `GraphStore` is Theorem's
//! canonical durable layer. The binding node carries the full binding
//! (identity, composition, scopes, lifecycle, and the scratchpad it owns);
//! each scratchpad revision is also persisted as its own node + chain edge so
//! the versioned document is queryable without loading the whole binding.

use crate::writing_style;
use rustyred_thg_core::{
    EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;
use theorem_harness_core::{
    AgentBinding, BindingError, BindingEventState, BindingTransitionInput, BindingTransitionResult,
    ScratchpadRevision, ScratchpadRevisionRelation, apply_binding_transition,
};

pub type BindingRuntimeResult<T> = Result<T, BindingRuntimeError>;

/// Errors from persisting a binding run: kernel guard violations, store I/O,
/// (de)serialization, and append-chain integrity failures.
#[derive(Clone, Debug, PartialEq)]
pub enum BindingRuntimeError {
    Kernel(BindingError),
    Store(GraphStoreError),
    Serialization(String),
    Deserialization(String),
    EventConflict {
        event_id: String,
        run_id: String,
        seq: u64,
    },
    EventGap {
        run_id: String,
        expected_previous_seq: u64,
    },
}

impl fmt::Display for BindingRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Kernel(error) => write!(f, "{error}"),
            Self::Store(error) => write!(f, "{}: {}", error.code, error.message),
            Self::Serialization(error) => write!(f, "serialization failed: {error}"),
            Self::Deserialization(error) => write!(f, "deserialization failed: {error}"),
            Self::EventConflict {
                event_id,
                run_id,
                seq,
            } => write!(
                f,
                "binding event log conflict at {event_id} for run {run_id} seq {seq}"
            ),
            Self::EventGap {
                run_id,
                expected_previous_seq,
            } => write!(
                f,
                "binding event log gap for run {run_id}: missing previous seq {expected_previous_seq}"
            ),
        }
    }
}

impl Error for BindingRuntimeError {}

impl From<BindingError> for BindingRuntimeError {
    fn from(value: BindingError) -> Self {
        Self::Kernel(value)
    }
}

impl From<GraphStoreError> for BindingRuntimeError {
    fn from(value: GraphStoreError) -> Self {
        Self::Store(value)
    }
}

/// Apply a binding lifecycle transition through the kernel and persist the
/// resulting binding, event, and scratchpad revisions to the store.
pub fn append_binding_transition<S: GraphStore>(
    store: &mut S,
    binding: AgentBinding,
    transition: BindingTransitionInput,
) -> BindingRuntimeResult<BindingTransitionResult> {
    let transition = writing_style::enrich_binding_transition(transition);
    let result = apply_binding_transition(binding, transition)?;
    persist_binding_transition_result(store, &result)?;
    Ok(result)
}

/// Persist a `BindingTransitionResult`: the binding node (with its scratchpad
/// revisions), the binding event node, and the append-chain edges.
pub fn persist_binding_transition_result<S: GraphStore>(
    store: &mut S,
    result: &BindingTransitionResult,
) -> BindingRuntimeResult<()> {
    ensure_binding_append_position(store, &result.event)?;
    persist_binding(store, &result.binding, &result.state_hash_after)?;

    let event_node = binding_event_node(&result.event)?;
    let event_id = event_node.id.clone();
    let event_already_present = match store.get_node(&event_id) {
        Some(existing) if binding_event_matches(existing, &result.event) => true,
        Some(_) => {
            return Err(BindingRuntimeError::EventConflict {
                event_id,
                run_id: result.event.run_id.clone(),
                seq: result.event.seq,
            });
        }
        None => false,
    };
    if !event_already_present {
        upsert_node_if_changed(store, event_node)?;
    }

    upsert_edge_if_changed(store, binding_event_of_edge(&result.event)?)?;
    if result.event.seq > 1 {
        upsert_edge_if_changed(store, previous_binding_event_edge(&result.event)?)?;
    }
    Ok(())
}

pub fn persist_binding_run_result<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
    events: &[BindingEventState],
) -> BindingRuntimeResult<()> {
    let state_hash = events
        .last()
        .map(|event| event.state_hash_after.as_str())
        .unwrap_or_default();
    persist_binding(store, binding, state_hash)?;
    for event in events {
        persist_binding_event_state(store, event)?;
    }
    Ok(())
}

pub fn persist_binding_event_state<S: GraphStore>(
    store: &mut S,
    event: &BindingEventState,
) -> BindingRuntimeResult<()> {
    ensure_binding_append_position(store, event)?;
    let event_node = binding_event_node(event)?;
    let event_id = event_node.id.clone();
    let event_already_present = match store.get_node(&event_id) {
        Some(existing) if binding_event_matches(existing, event) => true,
        Some(_) => {
            return Err(BindingRuntimeError::EventConflict {
                event_id,
                run_id: event.run_id.clone(),
                seq: event.seq,
            });
        }
        None => false,
    };
    if !event_already_present {
        upsert_node_if_changed(store, event_node)?;
    }

    upsert_edge_if_changed(store, binding_event_of_edge(event)?)?;
    if event.seq > 1 {
        upsert_edge_if_changed(store, previous_binding_event_edge(event)?)?;
    }
    Ok(())
}

/// Persist a binding node plus every scratchpad revision it owns (idempotent).
/// Used both by transition persistence and directly after a scratchpad append
/// (which happens outside the lifecycle transitions).
pub fn persist_binding<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
    state_hash: &str,
) -> BindingRuntimeResult<()> {
    upsert_node_if_changed(store, binding_node(binding, state_hash)?)?;
    persist_scratchpad_revisions(store, binding)?;
    Ok(())
}

fn persist_scratchpad_revisions<S: GraphStore>(
    store: &mut S,
    binding: &AgentBinding,
) -> BindingRuntimeResult<()> {
    let document_id = &binding.working_memory_scope.scratchpad.document_id;
    let run_id = &binding.lifecycle.run_id;
    let revision_seq_by_id = binding
        .working_memory_scope
        .scratchpad
        .revisions
        .iter()
        .map(|revision| (revision.revision_id.clone(), revision.seq))
        .collect::<BTreeMap<_, _>>();
    for revision in &binding.working_memory_scope.scratchpad.revisions {
        upsert_node_if_changed(store, scratchpad_revision_node(document_id, revision)?)?;
        upsert_edge_if_changed(
            store,
            scratchpad_revision_of_edge(document_id, run_id, revision),
        )?;
        for parent_revision_id in scratchpad_parent_revision_ids(revision) {
            if let Some(parent_seq) = revision_seq_by_id.get(&parent_revision_id) {
                upsert_edge_if_changed(
                    store,
                    scratchpad_revision_parent_edge(document_id, *parent_seq, revision),
                )?;
            }
        }
        if revision.seq > 1 {
            upsert_edge_if_changed(
                store,
                previous_scratchpad_revision_edge(document_id, revision),
            )?;
        }
    }
    for relation in &binding.working_memory_scope.scratchpad.relations {
        let Some(from_seq) = revision_seq_by_id.get(&relation.from_revision_id) else {
            continue;
        };
        let Some(to_seq) = revision_seq_by_id.get(&relation.to_revision_id) else {
            continue;
        };
        upsert_edge_if_changed(
            store,
            scratchpad_revision_relation_edge(document_id, relation, *from_seq, *to_seq),
        )?;
    }
    Ok(())
}

pub fn load_binding<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> BindingRuntimeResult<Option<AgentBinding>> {
    store
        .get_node(&binding_node_id(run_id))
        .map(|node| {
            serde_json::from_value::<AgentBinding>(node.properties.clone())
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .transpose()
}

pub fn load_binding_events<S: GraphStore>(
    store: &S,
    run_id: &str,
) -> BindingRuntimeResult<Vec<BindingEventState>> {
    let mut events = store
        .query_nodes(
            NodeQuery::label("BindingEvent")
                .with_property("run_id", Value::String(run_id.to_string())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<BindingEventState>(node.properties)
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<BindingRuntimeResult<Vec<_>>>()?;
    events.sort_by_key(|event| event.seq);
    Ok(events)
}

pub fn load_scratchpad_revisions<S: GraphStore>(
    store: &S,
    document_id: &str,
) -> BindingRuntimeResult<Vec<ScratchpadRevision>> {
    let mut revisions = store
        .query_nodes(
            NodeQuery::label("ScratchpadRevision")
                .with_property("document_id", Value::String(document_id.to_string())),
        )
        .into_iter()
        .map(|node| {
            serde_json::from_value::<ScratchpadRevision>(node.properties)
                .map_err(|error| BindingRuntimeError::Deserialization(error.to_string()))
        })
        .collect::<BindingRuntimeResult<Vec<_>>>()?;
    revisions.sort_by_key(|revision| revision.seq);
    Ok(revisions)
}

/// A public-facing summary of one binding in an agent's lineage. The lineage
/// is the chain of `AgentBinding` nodes persisted across runs that share the
/// same `agent_id` (the named lineage; different `composition_hash` distinguish
/// versions). The entry surfaces the load-bearing identity facts so callers
/// can fan out reads (e.g. fetching each binding's published memory) without
/// re-deserializing the full binding node.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct BindingLineageEntry {
    pub binding_id: String,
    pub agent_id: String,
    pub agent_name: String,
    pub version: u32,
    pub composition_hash: String,
    pub created_at_ms: String,
    pub active_head_set: Vec<String>,
    pub trust_tier: String,
}

/// Walk the graph for every `AgentBinding` node whose `identity.agent_id`
/// matches `agent_id`. The chain is the named lineage; different
/// `composition_hash` values distinguish versions of the same agent. Ordering
/// is `(version, created_at_ms, binding_id)` so a strictly-increasing version
/// rules; equal versions fall back to the lifecycle creation timestamp and
/// then the durable binding node id, keeping the order deterministic. Returns
/// an empty vec when the agent has no recorded lineage.
pub fn binding_lineage<S: GraphStore>(
    store: &S,
    agent_id: &str,
) -> BindingRuntimeResult<Vec<BindingLineageEntry>> {
    let agent_id = agent_id.trim();
    if agent_id.is_empty() {
        return Ok(Vec::new());
    }
    let mut entries = store
        .query_nodes(NodeQuery::label("AgentBinding").with_limit(usize::MAX))
        .into_iter()
        .filter_map(|node| {
            // The store's property index is single-level, so `identity.agent_id`
            // is not directly queryable. Filter the AgentBinding label set in
            // memory; an agent's lineage is bounded in practice.
            let identity = node.properties.get("identity")?.as_object()?;
            let candidate = identity.get("agent_id").and_then(Value::as_str)?;
            if candidate != agent_id {
                return None;
            }
            let agent_name = identity
                .get("agent_name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let version = identity.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
            let composition_hash = identity
                .get("composition_hash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let trust_tier = identity
                .get("trust_tier")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let active_head_set = identity
                .get("active_head_set")
                .and_then(Value::as_array)
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_string))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let created_at_ms = node
                .properties
                .get("lifecycle")
                .and_then(Value::as_object)
                .and_then(|lifecycle| lifecycle.get("created_at"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            Some(BindingLineageEntry {
                binding_id: node.id.clone(),
                agent_id: agent_id.to_string(),
                agent_name,
                version,
                composition_hash,
                created_at_ms,
                active_head_set,
                trust_tier,
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| {
        a.version
            .cmp(&b.version)
            .then_with(|| a.created_at_ms.cmp(&b.created_at_ms))
            .then_with(|| a.binding_id.cmp(&b.binding_id))
    });
    Ok(entries)
}

/// Build the lineage-memory payload to thread into `MEMORY_SCOPE.MOUNTED` for
/// the binding currently being mounted. The runtime walks the agent's prior
/// bindings (every `AgentBinding` node sharing `identity.agent_id` whose
/// `composition_hash` differs from the current binding) and projects each
/// prior binding's published-substrate signal as a
/// `BindingLineageMemoryEntry`. The kernel will then append each entry as a
/// scratchpad revision before `HEADS.CONTRIBUTE` (see `apply_binding_payload`
/// in `theorem-harness-core`).
///
/// Returns the entries (possibly empty) so callers can decide whether to merge
/// them into the MOUNTED payload. An agent with no recorded prior binding
/// (cold start) yields `vec![]`, which the kernel treats as a no-op — the
/// legacy MOUNTED behaviour.
pub fn lineage_memory_for_binding<S: GraphStore>(
    store: &S,
    binding: &AgentBinding,
) -> BindingRuntimeResult<Vec<theorem_harness_core::BindingLineageMemoryEntry>> {
    let lineage = binding_lineage(store, &binding.identity.agent_id)?;
    // P2: exclude only the CURRENT binding by its node id (keyed by run_id),
    // not every prior binding sharing the same composition_hash. The prior
    // filter dropped legitimate prior runs whenever they happened to reuse
    // the same head roster -- the common case for an agent rerun -- and
    // returned an empty lineage. Same-composition prior runs must now
    // surface their lineage memory.
    let current_binding_id = binding_node_id(&binding.lifecycle.run_id);
    let mut entries = Vec::new();
    for entry in lineage {
        if entry.binding_id == current_binding_id {
            continue;
        }
        let prior_binding = match store.get_node(&entry.binding_id) {
            Some(node) => node,
            None => continue,
        };
        let run_id = prior_binding
            .properties
            .get("lifecycle")
            .and_then(Value::as_object)
            .and_then(|lifecycle| lifecycle.get("run_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if run_id.is_empty() {
            continue;
        }
        let events = load_binding_events(store, &run_id)?;
        let substrate_receipt_id = events
            .iter()
            .rev()
            .find(|event| event.event_type == "PUBLISHED_TO_SUBSTRATE")
            .and_then(|event| event.payload.get("substrate_receipt_id"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let memory_patches = events
            .iter()
            .rev()
            .find(|event| event.event_type == "MEMORY_PATCHES.PROPOSED");
        let patch_ids = memory_patches
            .and_then(|event| event.payload.get("patch_ids"))
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_string))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let published_at = memory_patches
            .map(|event| event.created_at.clone())
            .unwrap_or_default();
        // Skip a prior binding with no substrate publication and no proposed
        // memory patches: there is nothing concrete to inherit from it yet.
        if substrate_receipt_id.is_empty() && patch_ids.is_empty() {
            continue;
        }
        let summary = format!(
            "prior binding {} (v{}) published memory",
            entry.binding_id, entry.version
        );
        entries.push(theorem_harness_core::BindingLineageMemoryEntry {
            source_binding_id: entry.binding_id,
            source_composition_hash: entry.composition_hash,
            source_version: entry.version,
            summary,
            patch_ids,
            substrate_receipt_id,
            published_at,
        });
    }
    Ok(entries)
}

/// Build the `MEMORY_SCOPE.MOUNTED` payload that the runtime should hand to
/// `apply_binding_transition` for the given binding. The payload always
/// carries `scope_id` and `scratchpad_id` (the legacy contract); when prior
/// lineage memory exists, it also carries `lineage_memory` (the array the
/// kernel's MOUNTED arm projects as scratchpad Context revisions) and
/// `lineage_size` (a numeric receipt so callers can detect injection without
/// re-parsing the array).
pub fn mounted_payload_for_binding<S: GraphStore>(
    store: &S,
    binding: &AgentBinding,
) -> BindingRuntimeResult<theorem_harness_core::Payload> {
    let entries = lineage_memory_for_binding(store, binding)?;
    let mut payload = theorem_harness_core::Payload::new();
    payload.insert(
        "scope_id".to_string(),
        Value::String(binding.working_memory_scope.scope_id.clone()),
    );
    payload.insert(
        "scratchpad_id".to_string(),
        Value::String(
            binding
                .working_memory_scope
                .scratchpad
                .document_id
                .clone(),
        ),
    );
    if !entries.is_empty() {
        let lineage_array = entries
            .iter()
            .map(|entry| {
                serde_json::to_value(entry).expect(
                    "BindingLineageMemoryEntry serialization should be infallible",
                )
            })
            .collect::<Vec<_>>();
        payload.insert(
            "lineage_size".to_string(),
            Value::Number(serde_json::Number::from(entries.len())),
        );
        payload.insert("lineage_memory".to_string(), Value::Array(lineage_array));
    }
    Ok(payload)
}

pub fn binding_node_id(run_id: &str) -> String {
    format!("harness:binding:{run_id}")
}

pub fn binding_event_node_id(run_id: &str, seq: u64) -> String {
    format!("harness:bindingevent:{run_id}:{seq:020}")
}

pub fn scratchpad_revision_node_id(document_id: &str, seq: u64) -> String {
    format!("harness:scratchrev:{document_id}:{seq:020}")
}

fn binding_node(binding: &AgentBinding, state_hash: &str) -> BindingRuntimeResult<NodeRecord> {
    let mut properties = serde_json::to_value(binding)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    properties["state_hash"] = Value::String(state_hash.to_string());
    Ok(NodeRecord::new(
        binding_node_id(&binding.lifecycle.run_id),
        ["AgentBinding"],
        properties,
    ))
}

fn binding_event_node(event: &BindingEventState) -> BindingRuntimeResult<NodeRecord> {
    let properties = serde_json::to_value(event)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    Ok(NodeRecord::new(
        binding_event_node_id(&event.run_id, event.seq),
        ["BindingEvent"],
        properties,
    ))
}

fn scratchpad_revision_node(
    document_id: &str,
    revision: &ScratchpadRevision,
) -> BindingRuntimeResult<NodeRecord> {
    let mut properties = serde_json::to_value(revision)
        .map_err(|error| BindingRuntimeError::Serialization(error.to_string()))?;
    // The struct has no document_id field; the node carries it so revisions are
    // queryable per document. serde ignores the extra field on deserialization.
    properties["document_id"] = Value::String(document_id.to_string());
    Ok(NodeRecord::new(
        scratchpad_revision_node_id(document_id, revision.seq),
        ["ScratchpadRevision"],
        properties,
    ))
}

fn binding_event_of_edge(event: &BindingEventState) -> BindingRuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:edge:binding-event-of:{}:{:020}",
            event.run_id, event.seq
        ),
        binding_event_node_id(&event.run_id, event.seq),
        "HARNESS_BINDING_EVENT_OF",
        binding_node_id(&event.run_id),
        json!({
            "run_id": event.run_id,
            "seq": event.seq,
            "type": event.event_type,
            "binding_status_before": event.binding_status_before,
            "binding_status_after": event.binding_status_after,
            "state_hash_before": event.state_hash_before,
            "state_hash_after": event.state_hash_after,
        }),
    ))
}

fn previous_binding_event_edge(event: &BindingEventState) -> BindingRuntimeResult<EdgeRecord> {
    Ok(EdgeRecord::new(
        format!(
            "harness:edge:binding-event-next:{}:{:020}",
            event.run_id, event.seq
        ),
        binding_event_node_id(&event.run_id, event.seq - 1),
        "HARNESS_BINDING_EVENT_NEXT",
        binding_event_node_id(&event.run_id, event.seq),
        json!({
            "run_id": event.run_id,
            "from_seq": event.seq - 1,
            "to_seq": event.seq,
        }),
    ))
}

fn scratchpad_revision_of_edge(
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

fn previous_scratchpad_revision_edge(
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

fn scratchpad_revision_parent_edge(
    document_id: &str,
    parent_seq: u64,
    revision: &ScratchpadRevision,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrev-parent:{}:{:020}:{:020}",
            document_id, parent_seq, revision.seq
        ),
        scratchpad_revision_node_id(document_id, parent_seq),
        "HARNESS_SCRATCHPAD_REVISION_PARENT",
        scratchpad_revision_node_id(document_id, revision.seq),
        json!({
            "document_id": document_id,
            "from_seq": parent_seq,
            "to_seq": revision.seq,
            "child_revision_id": revision.revision_id,
        }),
    )
}

fn scratchpad_revision_relation_edge(
    document_id: &str,
    relation: &ScratchpadRevisionRelation,
    from_seq: u64,
    to_seq: u64,
) -> EdgeRecord {
    EdgeRecord::new(
        format!(
            "harness:edge:scratchrel:{}:{}",
            document_id, relation.relation_id
        ),
        scratchpad_revision_node_id(document_id, from_seq),
        relation.relation_kind.edge_type(),
        scratchpad_revision_node_id(document_id, to_seq),
        json!({
            "document_id": document_id,
            "relation_id": relation.relation_id,
            "from_revision_id": relation.from_revision_id,
            "to_revision_id": relation.to_revision_id,
            "relation_kind": relation.relation_kind,
            "actor_head_id": relation.actor_head_id,
            "summary": relation.summary,
            "created_at": relation.created_at,
        }),
    )
}

fn scratchpad_parent_revision_ids(revision: &ScratchpadRevision) -> Vec<String> {
    if !revision.parent_revision_ids.is_empty() {
        return revision.parent_revision_ids.clone();
    }
    if !revision.parent_revision_id.is_empty() {
        return vec![revision.parent_revision_id.clone()];
    }
    Vec::new()
}

fn ensure_binding_append_position<S: GraphStore>(
    store: &S,
    event: &BindingEventState,
) -> BindingRuntimeResult<()> {
    if event.seq <= 1 {
        return Ok(());
    }
    let previous_seq = event.seq - 1;
    if store
        .get_node(&binding_event_node_id(&event.run_id, previous_seq))
        .is_none()
    {
        return Err(BindingRuntimeError::EventGap {
            run_id: event.run_id.clone(),
            expected_previous_seq: previous_seq,
        });
    }
    Ok(())
}

fn binding_event_matches(existing: &NodeRecord, event: &BindingEventState) -> bool {
    existing.properties.get("run_id").and_then(Value::as_str) == Some(event.run_id.as_str())
        && existing.properties.get("seq").and_then(Value::as_u64) == Some(event.seq)
        && existing.properties.get("type").and_then(Value::as_str)
            == Some(event.event_type.as_str())
        && existing.properties.get("payload") == Some(&Value::Object(event.payload.clone()))
        && existing
            .properties
            .get("state_hash_before")
            .and_then(Value::as_str)
            == Some(event.state_hash_before.as_str())
        && existing
            .properties
            .get("state_hash_after")
            .and_then(Value::as_str)
            == Some(event.state_hash_after.as_str())
}

fn upsert_node_if_changed<S: GraphStore>(store: &mut S, node: NodeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_node(&node.id)
        .map(|existing| {
            !existing.tombstone
                && existing.labels == node.labels
                && existing.properties == node.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_node(node)?;
    }
    Ok(())
}

fn upsert_edge_if_changed<S: GraphStore>(store: &mut S, edge: EdgeRecord) -> GraphStoreResult<()> {
    let unchanged = store
        .get_edge(&edge.id)
        .map(|existing| {
            !existing.tombstone
                && existing.from_id == edge.from_id
                && existing.to_id == edge.to_id
                && existing.edge_type == edge.edge_type
                && existing.properties == edge.properties
        })
        .unwrap_or(false);
    if !unchanged {
        store.upsert_edge(edge)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, RedCoreGraphStore, RedCoreOptions};
    use serde_json::{Map, json};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use theorem_harness_core::{
        AgentHead, BindingBudgetScope, BindingComposition, BindingIdentity, HeadCostProfile,
        HeadKind, HeadReliabilityProfile, HeadTransport, Payload, ScratchpadRelationKind,
        ScratchpadRevisionLink, TraceTier, hash_agent_binding,
    };

    const TS: &str = "2026-06-02T00:00:00Z";

    #[test]
    fn append_binding_transition_persists_binding_event_and_edges() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let run_id = binding.lifecycle.run_id.clone();

        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();
        let probed = append_binding_transition(
            &mut store,
            resolved.binding,
            transition(
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude", "deepseek"] }),
            ),
        )
        .unwrap();

        assert!(store.get_node(&binding_node_id(&run_id)).is_some());
        assert!(store.get_node(&binding_event_node_id(&run_id, 1)).is_some());
        assert!(store.get_node(&binding_event_node_id(&run_id, 2)).is_some());
        assert!(
            store
                .get_edge(&format!("harness:edge:binding-event-of:{run_id}:{:020}", 2))
                .is_some()
        );
        assert!(
            store
                .get_edge(&format!(
                    "harness:edge:binding-event-next:{run_id}:{:020}",
                    2
                ))
                .is_some()
        );

        let events = load_binding_events(&store, &run_id).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].seq, 1);
        assert_eq!(events[1].event_type, "HEADS.PROBED");

        let loaded = load_binding(&store, &run_id).unwrap().unwrap();
        assert_eq!(loaded.lifecycle.status, probed.binding.lifecycle.status);
        assert_eq!(loaded.lifecycle.status, "heads_probed");
        assert_eq!(loaded.identity.agent_id, "theorem");
    }

    #[test]
    fn loaded_binding_hash_matches_persisted_state() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let run_id = binding.lifecycle.run_id.clone();
        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();

        let loaded = load_binding(&store, &run_id).unwrap().unwrap();
        assert_eq!(hash_agent_binding(&loaded), resolved.state_hash_after);
    }

    #[test]
    fn redcore_reopens_persisted_binding() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-binding-store-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let run_id;

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let binding = fixture_binding();
            run_id = binding.lifecycle.run_id.clone();
            let resolved = append_binding_transition(
                &mut store,
                binding,
                transition(
                    "BINDING.RESOLVED",
                    json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
                ),
            )
            .unwrap();
            append_binding_transition(
                &mut store,
                resolved.binding,
                transition("HEADS.PROBED", json!({ "probed_head_set": ["claude"] })),
            )
            .unwrap();
        }

        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let events = load_binding_events(&store, &run_id).unwrap();
            assert_eq!(events.len(), 2);
            assert_eq!(events[1].event_type, "HEADS.PROBED");
            let binding = load_binding(&store, &run_id).unwrap().unwrap();
            assert_eq!(binding.lifecycle.status, "heads_probed");
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    #[test]
    fn scratchpad_revisions_persist_and_chain() {
        let mut store = InMemoryGraphStore::new();
        let mut binding = fixture_binding();
        let document_id = binding.working_memory_scope.scratchpad.document_id.clone();
        binding
            .append_scratchpad_revision("claude", "proposal", "hash:1", Payload::new(), TS)
            .unwrap();
        binding
            .append_scratchpad_revision("deepseek", "critique", "hash:2", Payload::new(), TS)
            .unwrap();

        persist_binding(&mut store, &binding, &hash_agent_binding(&binding)).unwrap();

        let revisions = load_scratchpad_revisions(&store, &document_id).unwrap();
        assert_eq!(revisions.len(), 2);
        assert_eq!(revisions[0].seq, 1);
        assert_eq!(revisions[1].seq, 2);
        assert_eq!(revisions[1].parent_revision_id, revisions[0].revision_id);
        assert!(
            store
                .get_edge(&format!(
                    "harness:edge:scratchrev-next:{document_id}:{:020}",
                    2
                ))
                .is_some()
        );
        assert!(
            store
                .get_edge(&format!(
                    "harness:edge:scratchrev-of:{document_id}:{:020}",
                    1
                ))
                .is_some()
        );
    }

    #[test]
    fn scratchpad_dag_relations_persist_as_edges() {
        let mut store = InMemoryGraphStore::new();
        let mut binding = fixture_binding();
        let document_id = binding.working_memory_scope.scratchpad.document_id.clone();
        let proposal = binding
            .append_scratchpad_revision("claude", "proposal", "hash:1", Payload::new(), TS)
            .unwrap();
        let critique = binding
            .append_scratchpad_revision("deepseek", "critique", "hash:2", Payload::new(), TS)
            .unwrap();
        binding
            .append_scratchpad_revision_with_links(
                "claude",
                "synthesis",
                "hash:3",
                Payload::new(),
                vec![proposal.revision_id.clone(), critique.revision_id.clone()],
                vec![ScratchpadRevisionLink::new(
                    critique.revision_id,
                    ScratchpadRelationKind::Undercuts,
                    "critique undercuts proposal",
                    Payload::new(),
                )],
                TS,
            )
            .unwrap();

        persist_binding(&mut store, &binding, &hash_agent_binding(&binding)).unwrap();

        assert!(
            store
                .get_edge(&format!(
                    "harness:edge:scratchrev-parent:{document_id}:{:020}:{:020}",
                    1, 3
                ))
                .is_some()
        );
        assert!(
            store
                .get_edge(&format!(
                    "harness:edge:scratchrev-parent:{document_id}:{:020}:{:020}",
                    2, 3
                ))
                .is_some()
        );
        let relation_id = &binding.working_memory_scope.scratchpad.relations[0].relation_id;
        let relation_edge = store
            .get_edge(&format!(
                "harness:edge:scratchrel:{document_id}:{relation_id}"
            ))
            .unwrap();
        assert_eq!(relation_edge.edge_type, "HARNESS_SCRATCHPAD_UNDERCUTS");
    }

    #[test]
    fn binding_append_requires_contiguous_events() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let resolved = apply_binding_transition(
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();
        // Force a non-contiguous seq to prove the append-position guard fires.
        let mut skipped = resolved.clone();
        skipped.event.seq = 3;
        let error = persist_binding_transition_result(&mut store, &skipped).unwrap_err();
        assert_eq!(
            error,
            BindingRuntimeError::EventGap {
                run_id: skipped.event.run_id.clone(),
                expected_previous_seq: 2,
            }
        );
    }

    #[test]
    fn binding_append_rejects_conflicting_event_at_same_sequence() {
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();

        let mut conflicting = resolved.clone();
        conflicting.event.state_hash_after = "different".to_string();
        conflicting.state_hash_after = "different".to_string();
        let error = persist_binding_transition_result(&mut store, &conflicting).unwrap_err();
        assert_eq!(
            error,
            BindingRuntimeError::EventConflict {
                event_id: binding_event_node_id(&resolved.event.run_id, 1),
                run_id: resolved.event.run_id.clone(),
                seq: 1,
            }
        );
    }

    #[test]
    fn full_binding_lifecycle_persists_and_reopens() {
        let data_dir = std::env::temp_dir().join(format!(
            "theorem-binding-lifecycle-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos()
        ));
        let options = RedCoreOptions::default();
        let run_id;

        {
            let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
            let mut binding = fixture_binding();
            run_id = binding.lifecycle.run_id.clone();
            for (event, payload) in lifecycle_events() {
                binding = drive(&mut store, binding, event, payload);
            }
            assert_eq!(binding.lifecycle.status, "closed");
            assert_eq!(binding.lifecycle.last_event_seq, 16);
        }

        // Reopen the durable store and prove the whole composed-agent run is
        // intact: 16 contiguous lifecycle events, the binding at terminal
        // status, and one recorded head contribution receipt.
        {
            let store = RedCoreGraphStore::open(&data_dir, options).unwrap();
            let events = load_binding_events(&store, &run_id).unwrap();
            assert_eq!(events.len(), 16);
            assert_eq!(events.first().unwrap().event_type, "BINDING.RESOLVED");
            assert_eq!(events.last().unwrap().event_type, "RUN.CLOSED");
            let synthesis = events
                .iter()
                .find(|event| event.event_type == "DRAFTS.SYNTHESIZED")
                .unwrap();
            assert_eq!(
                synthesis.payload["style_receipts"][0]["receipt"]["pack_hash"],
                json!(prose_check::pack_hash())
            );
            for (idx, event) in events.iter().enumerate() {
                assert_eq!(event.seq, (idx as u64) + 1);
            }
            let binding = load_binding(&store, &run_id).unwrap().unwrap();
            assert_eq!(binding.lifecycle.status, "closed");
            assert_eq!(binding.trace_scope.contributions.len(), 1);
        }

        let _ = fs::remove_dir_all(data_dir);
    }

    fn drive(
        store: &mut impl GraphStore,
        binding: AgentBinding,
        event: &str,
        payload: Value,
    ) -> AgentBinding {
        append_binding_transition(store, binding, transition(event, payload))
            .unwrap()
            .binding
    }

    fn lifecycle_events() -> Vec<(&'static str, Value)> {
        vec![
            (
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
            (
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude", "deepseek"] }),
            ),
            (
                "MEMORY_SCOPE.MOUNTED",
                json!({ "scope_id": "bindingscope:theorem", "scratchpad_id": "scratchpad:theorem" }),
            ),
            (
                "CHARTER.COMPILED",
                json!({ "charter_hash": "charter:1", "stance": "grounded composed agent" }),
            ),
            (
                "CAPABILITIES.SELECTED",
                json!({ "capability_scope_hash": "cap:1", "visible_tools": ["datalog"], "callable_tools": ["datalog"] }),
            ),
            (
                "BUDGET.ALLOCATED",
                json!({ "budget_units": 25.0, "max_parallel_heads": 2 }),
            ),
            (
                "RUN.STARTED",
                json!({ "task": "answer with Theorem voice", "started_at": "2026-06-02T00:00:00Z" }),
            ),
            (
                "PRIVATE_WORK.OPENED",
                json!({ "scratchpad_revision_id": "scratchrev:1" }),
            ),
            (
                "HEADS.CONTRIBUTE",
                json!({ "head_id": "claude", "contribution_id": "contrib:1", "contribution_kind": "proposal" }),
            ),
            (
                "DRAFTS.SYNTHESIZED",
                json!({ "synthesis_id": "synth:1", "contributing_heads": ["claude", "deepseek"] }),
            ),
            (
                "PUBLICATION.PROPOSED",
                json!({ "publication_id": "pub:1", "draft_hash": "draft:1" }),
            ),
            (
                "POLICY.CHECKED",
                json!({
                    "policy_receipt_id": "policy:1",
                    "allowed": true,
                    "claims": [{ "text": "grounded", "provenance": "src:1" }]
                }),
            ),
            (
                "PUBLISHED_TO_SUBSTRATE",
                json!({ "publication_id": "pub:1", "substrate_receipt_id": "substrate:1" }),
            ),
            (
                "OUTCOME.RECORDED",
                json!({ "outcome_id": "outcome:1", "accepted": true, "summary": "published" }),
            ),
            (
                "MEMORY_PATCHES.PROPOSED",
                json!({ "patch_ids": ["patch:1"], "review_required": true }),
            ),
            (
                "RUN.CLOSED",
                json!({ "summary": "closed", "closed_by": "claude-code" }),
            ),
        ]
    }

    fn fixture_binding() -> AgentBinding {
        AgentBinding::new(
            BindingIdentity {
                agent_id: "theorem".to_string(),
                owner_id: "travis".to_string(),
                agent_name: "Theorem".to_string(),
                composition_hash: String::new(),
                version: 1,
                trust_tier: "first_party".to_string(),
                active_head_set: vec!["claude".to_string(), "deepseek".to_string()],
                agent_constitution: None,
            },
            BindingComposition {
                heads: vec![
                    head("claude", "anthropic", "claude", HeadKind::ReasoningCore),
                    head("deepseek", "deepseek", "v4", HeadKind::ReasoningCore),
                ],
            },
            BindingBudgetScope::new("theorem", 100.0, 3),
        )
        .unwrap()
    }

    fn head(head_id: &str, provider: &str, model: &str, kind: HeadKind) -> AgentHead {
        AgentHead {
            head_id: head_id.to_string(),
            display_name: head_id.to_string(),
            provider: provider.to_string(),
            model: model.to_string(),
            credential_ref: format!("credential:{head_id}"),
            transport: HeadTransport::Api,
            kind,
            capabilities: Vec::new(),
            cost_profile: HeadCostProfile::default(),
            reliability_profile: HeadReliabilityProfile::default(),
            allowed_tools: Vec::new(),
            trace_tier: TraceTier::Receipt,
        }
    }

    fn transition(event_type: &str, payload: Value) -> BindingTransitionInput {
        BindingTransitionInput::new(
            event_type,
            payload.as_object().cloned().unwrap_or_else(Map::new),
        )
        .at(TS)
    }

    // ---- S3: binding lineage + memory continuity tests ----

    fn lineage_binding(run_id: &str, version: u32, head_seed: &str) -> AgentBinding {
        // Same agent_id ("theorem"), distinct (run_id, composition) so the
        // durable binding nodes split into separate AgentBinding rows that the
        // lineage walker can sort. `head_seed` drives composition_hash drift.
        let mut binding = AgentBinding::new(
            BindingIdentity {
                agent_id: "theorem".to_string(),
                owner_id: "travis".to_string(),
                agent_name: "Theorem".to_string(),
                composition_hash: String::new(),
                version,
                trust_tier: "first_party".to_string(),
                active_head_set: vec![format!("claude-{head_seed}"), format!("deepseek-{head_seed}")],
                agent_constitution: None,
            },
            BindingComposition {
                heads: vec![
                    head(
                        &format!("claude-{head_seed}"),
                        "anthropic",
                        "claude",
                        HeadKind::ReasoningCore,
                    ),
                    head(
                        &format!("deepseek-{head_seed}"),
                        "deepseek",
                        "v4",
                        HeadKind::ReasoningCore,
                    ),
                ],
            },
            BindingBudgetScope::new("theorem", 100.0, 3),
        )
        .unwrap();
        binding.lifecycle.run_id = run_id.to_string();
        binding
    }

    /// Lifecycle event sequence (BINDING.RESOLVED through RUN.CLOSED) using
    /// the head ids that `lineage_binding(_, _, "a")` produces, so the
    /// HEADS.CONTRIBUTE / DRAFTS.SYNTHESIZED guards accept the binding.
    fn lineage_v1_lifecycle_events() -> Vec<(&'static str, Value)> {
        vec![
            (
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem:v1", "composition_hash": "ignored" }),
            ),
            (
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude-a", "deepseek-a"] }),
            ),
            (
                "MEMORY_SCOPE.MOUNTED",
                json!({ "scope_id": "bindingscope:theorem", "scratchpad_id": "scratchpad:theorem" }),
            ),
            (
                "CHARTER.COMPILED",
                json!({ "charter_hash": "charter:1", "stance": "grounded composed agent" }),
            ),
            (
                "CAPABILITIES.SELECTED",
                json!({ "capability_scope_hash": "cap:1", "visible_tools": ["datalog"], "callable_tools": ["datalog"] }),
            ),
            (
                "BUDGET.ALLOCATED",
                json!({ "budget_units": 25.0, "max_parallel_heads": 2 }),
            ),
            (
                "RUN.STARTED",
                json!({ "task": "answer with Theorem voice", "started_at": "2026-06-02T00:00:00Z" }),
            ),
            (
                "PRIVATE_WORK.OPENED",
                json!({ "scratchpad_revision_id": "scratchrev:1" }),
            ),
            (
                "HEADS.CONTRIBUTE",
                json!({ "head_id": "claude-a", "contribution_id": "contrib:1", "contribution_kind": "proposal" }),
            ),
            (
                "DRAFTS.SYNTHESIZED",
                json!({ "synthesis_id": "synth:1", "contributing_heads": ["claude-a", "deepseek-a"] }),
            ),
            (
                "PUBLICATION.PROPOSED",
                json!({ "publication_id": "pub:1", "draft_hash": "draft:1" }),
            ),
            (
                "POLICY.CHECKED",
                json!({
                    "policy_receipt_id": "policy:1",
                    "allowed": true,
                    "claims": [{ "text": "grounded", "provenance": "src:1" }]
                }),
            ),
            (
                "PUBLISHED_TO_SUBSTRATE",
                json!({ "publication_id": "pub:1", "substrate_receipt_id": "substrate:1" }),
            ),
            (
                "OUTCOME.RECORDED",
                json!({ "outcome_id": "outcome:1", "accepted": true, "summary": "published" }),
            ),
            (
                "MEMORY_PATCHES.PROPOSED",
                json!({ "patch_ids": ["patch:1"], "review_required": true }),
            ),
            (
                "RUN.CLOSED",
                json!({ "summary": "closed", "closed_by": "claude-code" }),
            ),
        ]
    }

    #[test]
    fn binding_lineage_returns_empty_for_unknown_agent() {
        let mut store = InMemoryGraphStore::new();
        // Persist one binding for a different agent_id to prove the filter is
        // doing real work (not just returning an empty graph).
        let mut binding = fixture_binding();
        binding.lifecycle.run_id = "agent:other".to_string();
        binding.identity.agent_id = "other-agent".to_string();
        persist_binding(&mut store, &binding, &hash_agent_binding(&binding)).unwrap();

        let lineage = binding_lineage(&store, "theorem").unwrap();
        assert!(
            lineage.is_empty(),
            "lineage for unknown agent must be empty, got {lineage:?}"
        );

        let empty_id = binding_lineage(&store, "").unwrap();
        assert!(empty_id.is_empty(), "lineage for blank agent_id must be empty");
    }

    #[test]
    fn binding_lineage_walks_all_versions_in_order() {
        let mut store = InMemoryGraphStore::new();
        // Persist out of order to prove the walker reorders by (version, created_at).
        for (run_id, version, seed, created_at) in [
            ("agent:theorem:v2", 2u32, "b", "2026-06-02T00:00:00Z"),
            ("agent:theorem:v1", 1u32, "a", "2026-06-01T00:00:00Z"),
            ("agent:theorem:v3", 3u32, "c", "2026-06-03T00:00:00Z"),
        ] {
            let mut binding = lineage_binding(run_id, version, seed);
            binding.lifecycle.created_at = created_at.to_string();
            persist_binding(&mut store, &binding, &hash_agent_binding(&binding)).unwrap();
        }
        let lineage = binding_lineage(&store, "theorem").unwrap();
        assert_eq!(lineage.len(), 3, "expected three lineage entries");
        assert_eq!(lineage[0].version, 1);
        assert_eq!(lineage[1].version, 2);
        assert_eq!(lineage[2].version, 3);
        assert_eq!(lineage[0].binding_id, binding_node_id("agent:theorem:v1"));
        assert_eq!(lineage[1].binding_id, binding_node_id("agent:theorem:v2"));
        assert_eq!(lineage[2].binding_id, binding_node_id("agent:theorem:v3"));
        for entry in &lineage {
            assert_eq!(entry.agent_id, "theorem");
            assert_eq!(entry.agent_name, "Theorem");
            assert_eq!(entry.trust_tier, "first_party");
            assert!(!entry.composition_hash.is_empty());
            assert!(!entry.created_at_ms.is_empty());
        }
        // All three composition hashes must be distinct (head_seed drives this);
        // otherwise the lineage cannot represent multiple binding versions.
        let hashes = lineage
            .iter()
            .map(|entry| entry.composition_hash.clone())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(hashes.len(), 3, "composition hashes must be distinct");
    }

    #[test]
    fn lineage_memory_for_binding_skips_prior_with_no_publication() {
        let mut store = InMemoryGraphStore::new();
        // Prior binding persisted via persist_binding without any BindingEvent
        // history: its substrate_receipt + patch_ids are both empty so it must
        // not appear as a lineage memory entry.
        let prior = lineage_binding("agent:theorem:cold", 1, "a");
        persist_binding(&mut store, &prior, &hash_agent_binding(&prior)).unwrap();

        let current = lineage_binding("agent:theorem:warm", 2, "b");
        let entries = lineage_memory_for_binding(&store, &current).unwrap();
        assert!(
            entries.is_empty(),
            "a prior binding with no publication signals must not surface as lineage memory; got {entries:?}"
        );
    }

    #[test]
    fn mounted_loads_agent_published_memory_from_prior_binding() {
        // End-to-end memory continuity test: bring a binding through to
        // PUBLISHED_TO_SUBSTRATE + MEMORY_PATCHES.PROPOSED, then mount a new
        // binding (same agent_id, fresh run_id and composition) and prove the
        // MOUNTED transition projects the prior binding's published memory
        // into the new binding's scratchpad as a Context revision.
        let mut store = InMemoryGraphStore::new();
        let mut v1 = lineage_binding("agent:theorem:v1", 1, "a");
        for (event, payload) in lineage_v1_lifecycle_events() {
            v1 = append_binding_transition(&mut store, v1, transition(event, payload))
                .unwrap()
                .binding;
        }
        assert_eq!(v1.lifecycle.status, "closed");

        // Pretend v1's composition hash is now "frozen"; v2 must be different.
        let mut v2 = lineage_binding("agent:theorem:v2", 2, "b");
        assert_ne!(v2.identity.composition_hash, v1.identity.composition_hash);

        let mut mounted_payload = mounted_payload_for_binding(&store, &v2).unwrap();
        let lineage_size = mounted_payload
            .get("lineage_size")
            .and_then(Value::as_u64)
            .expect("lineage_size present");
        assert_eq!(lineage_size, 1, "v2 must inherit v1's published memory");
        let lineage_array = mounted_payload
            .get("lineage_memory")
            .and_then(Value::as_array)
            .expect("lineage_memory array present");
        let entry = &lineage_array[0];
        assert_eq!(
            entry.get("source_binding_id").and_then(Value::as_str),
            Some(binding_node_id("agent:theorem:v1").as_str())
        );
        assert_eq!(
            entry.get("substrate_receipt_id").and_then(Value::as_str),
            Some("substrate:1"),
            "MOUNTED payload must carry the prior binding's PUBLISHED_TO_SUBSTRATE receipt id"
        );
        let patch_ids = entry
            .get("patch_ids")
            .and_then(Value::as_array)
            .expect("patch_ids array");
        assert_eq!(
            patch_ids.iter().filter_map(Value::as_str).collect::<Vec<_>>(),
            vec!["patch:1"],
            "MOUNTED payload must carry the prior binding's MEMORY_PATCHES.PROPOSED patch_ids"
        );

        // Walk v2 through BINDING.RESOLVED, HEADS.PROBED, MEMORY_SCOPE.MOUNTED
        // with the lineage-augmented payload. Prove the kernel arm appended a
        // scratchpad revision attributed to the synthetic lineage actor.
        v2 = append_binding_transition(
            &mut store,
            v2,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem:v2", "composition_hash": "ignored" }),
            ),
        )
        .unwrap()
        .binding;
        v2 = append_binding_transition(
            &mut store,
            v2,
            transition(
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude-b", "deepseek-b"] }),
            ),
        )
        .unwrap()
        .binding;
        // Rehydrate the lineage payload now that the binding's composition_hash
        // has been recomputed by BINDING.RESOLVED (guarantees v2 still excludes
        // itself from its own lineage).
        mounted_payload = mounted_payload_for_binding(&store, &v2).unwrap();
        let mounted_transition = BindingTransitionInput::new(
            "MEMORY_SCOPE.MOUNTED",
            mounted_payload,
        )
        .at(TS);
        let mounted = append_binding_transition(&mut store, v2, mounted_transition).unwrap();
        let lineage_revisions = mounted
            .binding
            .working_memory_scope
            .scratchpad
            .revisions
            .iter()
            .filter(|revision| revision.actor_head_id == "lineage:agent_published")
            .collect::<Vec<_>>();
        assert_eq!(
            lineage_revisions.len(),
            1,
            "MOUNTED must project one lineage memory entry as a scratchpad revision"
        );
        let revision = lineage_revisions[0];
        assert_eq!(
            revision
                .payload
                .get("kind")
                .and_then(Value::as_str),
            Some("lineage_memory")
        );
        assert_eq!(
            revision
                .payload
                .get("substrate_receipt_id")
                .and_then(Value::as_str),
            Some("substrate:1")
        );
        assert_eq!(
            revision
                .payload
                .get("source_binding_id")
                .and_then(Value::as_str),
            Some(binding_node_id("agent:theorem:v1").as_str())
        );
    }

    #[test]
    fn lineage_memory_for_binding_returns_same_composition_prior_run() {
        // P2 regression: a prior binding that shares the same head roster
        // (and therefore the same composition_hash) as the current binding
        // must still surface its published memory. The prior filter dropped
        // every same-composition prior run, which excluded the common case
        // (an agent rerun with the same heads). The exclude-by-binding-id
        // filter must only drop the current binding's own node.
        let mut store = InMemoryGraphStore::new();
        // v1 and v2 both use head_seed "a" -- identical heads, so the
        // composition_hash both bindings get from BINDING.RESOLVED matches.
        let mut v1 = lineage_binding("agent:theorem:rerun:v1", 1, "a");
        for (event, payload) in lineage_v1_lifecycle_events() {
            v1 = append_binding_transition(&mut store, v1, transition(event, payload))
                .unwrap()
                .binding;
        }
        assert_eq!(v1.lifecycle.status, "closed");

        let mut v2 = lineage_binding("agent:theorem:rerun:v2", 2, "a");
        v2 = append_binding_transition(
            &mut store,
            v2,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem:rerun:v2", "composition_hash": "ignored" }),
            ),
        )
        .unwrap()
        .binding;
        // Pin the precondition the prior filter dropped: same composition.
        assert_eq!(
            v1.identity.composition_hash, v2.identity.composition_hash,
            "test setup: v1 and v2 must share composition_hash so this test exercises the same-composition path"
        );
        assert_ne!(
            binding_node_id(&v1.lifecycle.run_id),
            binding_node_id(&v2.lifecycle.run_id),
            "test setup: v1 and v2 must be distinct binding nodes"
        );

        let entries = lineage_memory_for_binding(&store, &v2).unwrap();
        assert_eq!(
            entries.len(),
            1,
            "P2 fix: same-composition prior runs must surface their lineage; got {entries:?}"
        );
        assert_eq!(
            entries[0].source_binding_id,
            binding_node_id("agent:theorem:rerun:v1")
        );
        assert_eq!(entries[0].substrate_receipt_id, "substrate:1");
        assert_eq!(entries[0].patch_ids, vec!["patch:1".to_string()]);
    }

    #[test]
    fn lineage_memory_for_binding_still_excludes_current_binding() {
        // P2 invariant: the new exclude-by-binding-id filter must still
        // drop the current binding's own node from its own lineage, even
        // when the binding's BindingEvent history already carries
        // PUBLISHED_TO_SUBSTRATE + MEMORY_PATCHES.PROPOSED receipts. The
        // ScratchpadDocument is supposed to inherit memory from PRIOR
        // bindings, never from itself.
        let mut store = InMemoryGraphStore::new();
        let mut v1 = lineage_binding("agent:theorem:self:v1", 1, "a");
        for (event, payload) in lineage_v1_lifecycle_events() {
            v1 = append_binding_transition(&mut store, v1, transition(event, payload))
                .unwrap()
                .binding;
        }
        assert_eq!(v1.lifecycle.status, "closed");

        let entries = lineage_memory_for_binding(&store, &v1).unwrap();
        assert!(
            entries.is_empty(),
            "the current binding must never appear in its own lineage; got {entries:?}"
        );
    }

    #[test]
    fn mounted_without_lineage_memory_is_backward_compatible() {
        // No prior bindings persisted, so MOUNTED stays the legacy
        // {scope_id, scratchpad_id} payload (no lineage_size, no
        // lineage_memory) and the scratchpad gains no lineage revisions.
        let mut store = InMemoryGraphStore::new();
        let binding = fixture_binding();
        let payload = mounted_payload_for_binding(&store, &binding).unwrap();
        assert!(payload.get("scope_id").is_some());
        assert!(payload.get("scratchpad_id").is_some());
        assert!(payload.get("lineage_memory").is_none());
        assert!(payload.get("lineage_size").is_none());

        let resolved = append_binding_transition(
            &mut store,
            binding,
            transition(
                "BINDING.RESOLVED",
                json!({ "binding_id": "agent:theorem", "composition_hash": "ignored" }),
            ),
        )
        .unwrap();
        let probed = append_binding_transition(
            &mut store,
            resolved.binding,
            transition(
                "HEADS.PROBED",
                json!({ "probed_head_set": ["claude", "deepseek"] }),
            ),
        )
        .unwrap();
        let mounted = append_binding_transition(
            &mut store,
            probed.binding,
            transition(
                "MEMORY_SCOPE.MOUNTED",
                json!({
                    "scope_id": "bindingscope:theorem",
                    "scratchpad_id": "scratchpad:theorem"
                }),
            ),
        )
        .unwrap();
        let lineage_revisions = mounted
            .binding
            .working_memory_scope
            .scratchpad
            .revisions
            .iter()
            .filter(|revision| revision.actor_head_id == "lineage:agent_published")
            .count();
        assert_eq!(lineage_revisions, 0);
    }
}
