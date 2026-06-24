use crate::coordination::{write_record, WriteRecordInput};
use crate::event_log::{append_transition_from_store, load_events};
use crate::memory::{
    encode_memory, list_memory_documents_since, load_memory_document, memory_document_node_id,
    MemoryDocumentState, MemoryWriteInput,
};
use crate::skill_pack::{get_skill_pack, skill_pack_node_id, SkillPackGetInput, SkillPackState};
use crate::writing_style::{summarize_style_receipts_for_fitness, STYLE_RECEIPTS_FIELD};
use crate::{HarnessRuntimeError, RuntimeResult};
use rustyred_thg_affordances::{
    affordance_node_id, record_invocation, AffordanceGraphStore, InvocationRecordRequest,
};
use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, GraphStore, GraphStoreError,
    GraphStoreResult, GraphTransaction, GraphWriteResult, NeighborHit, NeighborQuery, NodeQuery,
    NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use theorem_harness_core::{stable_value_hash, EventState, RunState, TransitionInput};

pub const COMPOUND_CONFIG_NODE_LABEL: &str = "CompoundEngineeringConfig";
pub const COMPOUND_STATE_NODE_LABEL: &str = "CompoundEngineeringState";
pub const COMPOUND_ROOM_ID: &str = "compound-engineering";
/// Tag stamped on every compound-engineering capture (alongside `cluster:<key>`).
/// The read-only `list_compound_captures` reader keys on this to separate compound
/// captures from other `MemoryDocument` rows in the same tenant.
pub const COMPOUND_CAPTURE_TAG: &str = "compound-engineering";
const COMPOUND_CLUSTER_TAG_PREFIX: &str = "cluster:";
/// Outcome `kind` values produced by `OutcomeClass::encode_kind`, the only kinds a
/// compound capture can carry. Used to validate the `outcome` filter.
const COMPOUND_OUTCOME_KINDS: &[&str] = &["solution", "postmortem", "feedback"];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CompoundConfig {
    pub schema_version: u64,
    pub capture_step_floor: usize,
    pub advisory_promotion_run_count: u64,
    pub decay_window_runs: u64,
    pub shadow_benchmark_gate_required: bool,
    pub canonical_demotes_on_hard_axis_regression: bool,
}

impl Default for CompoundConfig {
    fn default() -> Self {
        Self {
            schema_version: 1,
            capture_step_floor: 6,
            advisory_promotion_run_count: 3,
            decay_window_runs: 12,
            shadow_benchmark_gate_required: true,
            canonical_demotes_on_hard_axis_regression: true,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompoundHookReceipt {
    pub run_id: String,
    pub skipped_replay: bool,
    pub config_hash: String,
    pub cluster_key: String,
    pub captured_doc_id: String,
    pub used_pack_hashes: Vec<String>,
    pub used_memory_doc_ids: Vec<String>,
    #[serde(default)]
    pub retrieval_attributions: Vec<RetrievalAttribution>,
    #[serde(default)]
    pub recorded_affordance_receipts: Vec<Value>,
    pub promotion_proposals: Vec<Value>,
    pub demotions: Vec<Value>,
    pub decayed_items: Vec<Value>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RetrievalAttribution {
    pub artifact_id: String,
    pub query: String,
    pub query_signature: String,
    pub seed_signature: String,
    #[serde(default)]
    pub memory_doc_ids: Vec<String>,
    #[serde(default)]
    pub expansion_depth: Option<u64>,
    #[serde(default)]
    pub gap_frontier_choices: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OutcomeSignal {
    pub run_id: String,
    pub outcome: String,
    pub signal: String,
    pub config_hash: String,
    pub cluster_key: String,
    pub run_counter: u64,
    #[serde(default)]
    pub gate_axes: Value,
}

impl OutcomeSignal {
    fn from_outcome(
        run_id: &str,
        outcome: &OutcomeClass,
        gate_axes: Value,
        config_hash: &str,
        cluster_key: &str,
        run_counter: u64,
    ) -> Self {
        Self {
            run_id: run_id.to_string(),
            outcome: outcome.as_str().to_string(),
            signal: outcome.signal().to_string(),
            config_hash: config_hash.to_string(),
            cluster_key: cluster_key.to_string(),
            run_counter,
            gate_axes,
        }
    }

    pub fn is_positive(&self) -> bool {
        self.outcome == "positive"
    }

    pub fn hard_axis_regression(&self) -> bool {
        self.gate_axes
            .get("writing_engineering")
            .and_then(|value| value.get("last_hard_axis_failed"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CompoundStandingReceipt {
    pub artifact_type: String,
    pub artifact_id: String,
    pub run_count: u64,
    pub positive_count: u64,
    pub hard_axis_regressions: u64,
    pub last_outcome: String,
}

pub trait CompoundingArtifact {
    fn compound_artifact_type(&self) -> &'static str;
    fn compound_artifact_id(&self) -> String;
    fn compound_metadata_mut(&mut self) -> &mut Map<String, Value>;
}

impl CompoundingArtifact for SkillPackState {
    fn compound_artifact_type(&self) -> &'static str {
        "skill_pack"
    }

    fn compound_artifact_id(&self) -> String {
        self.pack_content_hash.clone()
    }

    fn compound_metadata_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.metadata
    }
}

pub fn apply_compound_standing<A: CompoundingArtifact>(
    artifact: &mut A,
    signal: &OutcomeSignal,
) -> CompoundStandingReceipt {
    let artifact_type = artifact.compound_artifact_type().to_string();
    let artifact_id = artifact.compound_artifact_id();
    let receipt = apply_compound_standing_to_metadata(artifact.compound_metadata_mut(), signal);
    CompoundStandingReceipt {
        artifact_type,
        artifact_id,
        ..receipt
    }
}

#[derive(Clone, Debug, Default)]
struct UsedItems {
    packs: BTreeMap<String, UsedPack>,
    memory_doc_ids: BTreeSet<String>,
    tools: BTreeSet<String>,
    retrieval_attributions: BTreeMap<String, RetrievalAttribution>,
}

#[derive(Clone, Debug, Default)]
struct UsedPack {
    pack_id: String,
    pack_content_hash: String,
}

struct CompoundAffordanceStore<'a, S: GraphStore> {
    store: &'a mut S,
}

impl<S: GraphStore> AffordanceGraphStore for CompoundAffordanceStore<'_, S> {
    fn upsert_node(&mut self, node: NodeRecord) -> GraphStoreResult<GraphWriteResult> {
        self.store.upsert_node(node)
    }

    fn upsert_edge(&mut self, edge: EdgeRecord) -> GraphStoreResult<GraphWriteResult> {
        self.store.upsert_edge(edge)
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
                GraphMutation::NodeUpsert(node) => writes.push(self.store.upsert_node(node)?),
                GraphMutation::EdgeUpsert(edge) => writes.push(self.store.upsert_edge(edge)?),
            }
        }
        let graph_version = self.store.stats().version;
        Ok(GraphTransaction {
            txn_id: graph_version,
            graph_version,
            writes,
        })
    }

    fn get_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        Ok(self.store.get_node(id).cloned())
    }

    fn get_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        Ok(self.store.get_edge(id).cloned())
    }

    fn query_nodes(&self, query: NodeQuery) -> GraphStoreResult<Vec<NodeRecord>> {
        Ok(self.store.query_nodes(query))
    }

    fn neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        Ok(self.store.neighbors(query))
    }

    fn snapshot(&self) -> GraphStoreResult<GraphSnapshot> {
        Ok(GraphSnapshot {
            version: self.store.stats().version,
            nodes: Vec::new(),
            edges: Vec::new(),
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum OutcomeClass {
    Positive,
    Negative,
    Mixed,
    Neutral,
}

impl OutcomeClass {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Mixed => "mixed",
            Self::Neutral => "neutral",
        }
    }

    fn encode_kind(&self) -> &'static str {
        match self {
            Self::Positive => "solution",
            Self::Negative => "postmortem",
            Self::Mixed | Self::Neutral => "feedback",
        }
    }

    fn signal(&self) -> &'static str {
        match self {
            Self::Positive => "pinned",
            Self::Negative => "contradicted",
            Self::Mixed | Self::Neutral => "cited",
        }
    }

    fn outcome_value(&self) -> f32 {
        match self {
            Self::Positive => 1.0,
            Self::Mixed | Self::Neutral => 0.5,
            Self::Negative => 0.0,
        }
    }
}

pub fn compound_config_node_id(tenant_slug: &str) -> String {
    format!(
        "compound_engineering:config:{}",
        normalize_tenant(tenant_slug)
    )
}

pub fn compound_state_node_id(tenant_slug: &str) -> String {
    format!(
        "compound_engineering:state:{}",
        normalize_tenant(tenant_slug)
    )
}

pub fn persist_compound_config<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    config: CompoundConfig,
) -> RuntimeResult<String> {
    let tenant = normalize_tenant(tenant_slug);
    let payload = serde_json::to_value(&config)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    let hash = compound_config_hash(&config);
    store.upsert_node(NodeRecord::new(
        compound_config_node_id(&tenant),
        [COMPOUND_CONFIG_NODE_LABEL],
        json!({
            "tenant_slug": tenant,
            "config": payload,
            "config_hash": hash,
        }),
    ))?;
    Ok(hash)
}

pub fn load_compound_config<S: GraphStore>(
    store: &S,
    tenant_slug: &str,
) -> RuntimeResult<CompoundConfig> {
    let tenant = normalize_tenant(tenant_slug);
    let Some(node) = store.get_node(&compound_config_node_id(&tenant)) else {
        return Ok(CompoundConfig::default());
    };
    let config = node
        .properties
        .get("config")
        .cloned()
        .unwrap_or_else(|| json!(CompoundConfig::default()));
    serde_json::from_value(config)
        .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))
}

pub fn compound_config_hash(config: &CompoundConfig) -> String {
    let payload = serde_json::to_value(config).expect("compound config serializes");
    format!("sha256:{}", stable_value_hash(&payload))
}

/// Read-only consumer of the compound-engineering capture write-path (S4-TRACE-READER).
///
/// Returns the `MemoryDocument` captures `apply_run_close_hook` writes via
/// `encode_memory` (doc_id `compound:capture:<run_id>`), newest first, filtered to
/// the compound corpus by the [`COMPOUND_CAPTURE_TAG`] tag. Optional narrowing:
///
/// - `cluster_key`: keep captures whose `cluster:<key>` tag or `metadata["cluster_key"]`
///   matches exactly.
/// - `outcome`: keep captures whose `kind` equals one of `solution` / `postmortem` /
///   `feedback` (the values [`OutcomeClass::encode_kind`] emits). An `outcome` outside
///   that set matches nothing.
/// - `since`: keep captures whose `updated_at` is at or after the watermark (lexical
///   compare, matching the recall path), threaded through
///   [`list_memory_documents_since`].
///
/// This reader never mutates: it does not touch fitness and does not route through the
/// recall path (which calls `bump_recalled_compound_fitness`). It composes over
/// `list_memory_documents_since` with `include_inactive = true` so the full capture
/// history is visible (superseded/archived captures included; deleted always dropped).
pub fn list_compound_captures<S: GraphStore>(
    store: &S,
    tenant: &str,
    cluster_key: Option<&str>,
    outcome: Option<&str>,
    since: Option<&str>,
) -> RuntimeResult<Vec<MemoryDocumentState>> {
    let tenant = normalize_tenant(tenant);
    let since = since.unwrap_or("");
    let cluster_filter = cluster_key.map(str::trim).filter(|value| !value.is_empty());
    let outcome_filter = outcome
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    let documents = list_memory_documents_since(store, &tenant, since, true)
        .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))?;

    Ok(documents
        .into_iter()
        .filter(document_is_compound_capture)
        .filter(|document| match cluster_filter {
            Some(cluster) => document_matches_cluster(document, cluster),
            None => true,
        })
        .filter(|document| match outcome_filter.as_deref() {
            Some(kind) => {
                COMPOUND_OUTCOME_KINDS.contains(&kind) && document.kind.eq_ignore_ascii_case(kind)
            }
            None => true,
        })
        .collect())
}

fn document_is_compound_capture(document: &MemoryDocumentState) -> bool {
    document
        .tags
        .iter()
        .any(|tag| tag.trim() == COMPOUND_CAPTURE_TAG)
}

fn document_matches_cluster(document: &MemoryDocumentState, cluster: &str) -> bool {
    let tag_match = document.tags.iter().any(|tag| {
        tag.trim()
            .strip_prefix(COMPOUND_CLUSTER_TAG_PREFIX)
            .map(|value| value == cluster)
            .unwrap_or(false)
    });
    let metadata_match = document
        .metadata
        .get("cluster_key")
        .and_then(Value::as_str)
        .map(|value| value == cluster)
        .unwrap_or(false);
    tag_match || metadata_match
}

pub fn apply_run_close_hook<S: GraphStore>(
    store: &mut S,
    run: &RunState,
    trigger: &EventState,
) -> RuntimeResult<CompoundHookReceipt> {
    if !matches!(trigger.event_type.as_str(), "RUN.CLOSED" | "RUN.FAILED") {
        return Ok(CompoundHookReceipt::default());
    }

    let events = load_events(store, &run.run_id)?;
    if events
        .iter()
        .any(|event| event.event_type == "RUN.REPLAYED")
    {
        return Ok(CompoundHookReceipt {
            run_id: run.run_id.clone(),
            skipped_replay: true,
            ..CompoundHookReceipt::default()
        });
    }
    if events
        .iter()
        .any(|event| event.event_type == "COMPOUND.CAPTURED")
    {
        return Ok(CompoundHookReceipt {
            run_id: run.run_id.clone(),
            ..CompoundHookReceipt::default()
        });
    }

    let tenant = tenant_for_run(run);
    let config = load_compound_config(store, &tenant)?;
    let config_hash = compound_config_hash(&config);
    let run_counter = increment_run_counter(store, &tenant, trigger)?;
    let outcome = classify_outcome(run, &events);
    let cluster_key = cluster_key_for_run(run, &events, &tenant);
    let used = collect_used_items(store, &tenant, &run.run_id, &events);
    let recorded_affordance_receipts =
        record_used_affordance_outcomes(store, &tenant, run, &outcome, &used)?;

    let captured_doc_id = capture_run_if_qualifies(
        store,
        run,
        trigger,
        &events,
        &config,
        &config_hash,
        &cluster_key,
        &outcome,
        &used,
    )?;
    append_compound_event(
        store,
        run,
        "COMPOUND.CAPTURED",
        trigger,
        json!({
            "config_hash": config_hash,
            "captured": !captured_doc_id.is_empty(),
            "memory_doc_id": captured_doc_id,
            "cluster_key": cluster_key,
            "outcome": outcome.as_str(),
        }),
    )?;

    let (promotion_proposals, demotions) = apply_usage_fitness(
        store,
        &tenant,
        &run.run_id,
        &events,
        &config,
        &config_hash,
        &cluster_key,
        &outcome,
        &used,
        run_counter,
        trigger,
    )?;
    append_compound_event(
        store,
        run,
        "COMPOUND.FITNESS_APPLIED",
        trigger,
        json!({
            "config_hash": config_hash,
            "outcome": outcome.as_str(),
            "used_pack_hashes": used
                .packs
                .values()
                .map(|pack| pack.pack_content_hash.clone())
                .collect::<Vec<_>>(),
            "used_pack_ids": used
                .packs
                .values()
                .map(|pack| pack.pack_id.clone())
                .filter(|pack_id| !pack_id.is_empty())
                .collect::<Vec<_>>(),
            "used_memory_doc_ids": used.memory_doc_ids.iter().cloned().collect::<Vec<_>>(),
            "retrieval_attributions": used
                .retrieval_attributions
                .values()
                .cloned()
                .collect::<Vec<_>>(),
            "used_tools": used.tools.iter().cloned().collect::<Vec<_>>(),
            "recorded_affordance_receipts": recorded_affordance_receipts.clone(),
            "positive_reinforcement_applied": outcome == OutcomeClass::Positive,
        }),
    )?;

    append_compound_event(
        store,
        run,
        "COMPOUND.GATE_PROPOSED",
        trigger,
        json!({
            "config_hash": config_hash,
            "promotion_proposals": promotion_proposals,
            "demotions": demotions,
        }),
    )?;

    let decayed_items = apply_decay(store, &tenant, &config, &config_hash, run_counter, trigger)?;
    append_compound_event(
        store,
        run,
        "COMPOUND.DECAYED",
        trigger,
        json!({
            "config_hash": config_hash,
            "run_counter": run_counter,
            "decayed_items": decayed_items,
        }),
    )?;

    Ok(CompoundHookReceipt {
        run_id: run.run_id.clone(),
        skipped_replay: false,
        config_hash,
        cluster_key,
        captured_doc_id,
        used_pack_hashes: used
            .packs
            .values()
            .map(|pack| pack.pack_content_hash.clone())
            .collect(),
        used_memory_doc_ids: used.memory_doc_ids.into_iter().collect(),
        retrieval_attributions: used.retrieval_attributions.into_values().collect(),
        recorded_affordance_receipts,
        promotion_proposals,
        demotions,
        decayed_items,
    })
}

fn record_used_affordance_outcomes<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    run: &RunState,
    outcome: &OutcomeClass,
    used: &UsedItems,
) -> RuntimeResult<Vec<Value>> {
    let mut affordance_store = CompoundAffordanceStore { store };
    let mut receipts = Vec::new();
    let mut previous_affordance_id = None;
    for tool_id in &used.tools {
        let affordance_node = affordance_node_id(tenant, tool_id);
        if affordance_store.get_node(&affordance_node)?.is_none() {
            continue;
        }
        let result = record_invocation(
            &mut affordance_store,
            InvocationRecordRequest {
                tenant_id: tenant.to_string(),
                task_type: compound_task_type(run),
                candidate_affordance_ids: vec![tool_id.clone()],
                selected_affordance_id: tool_id.clone(),
                outcome_value: outcome.outcome_value(),
                outcome_weight: 1.0,
                outcome_label: outcome.as_str().to_string(),
                previous_affordance_id: previous_affordance_id.clone(),
                query_text: run.task.clone(),
                recorded_at_ms: None,
            },
            Some("compound-engineering"),
        )
        .map_err(thg_runtime_error)?;
        receipts.push(json!({
            "tool_id": tool_id,
            "receipt_node_id": result.receipt_node_id,
            "receipt_hash": result.receipt_hash,
            "effective_fitness": result.effective_fitness,
            "graph_version": result.graph_version,
        }));
        previous_affordance_id = Some(tool_id.clone());
    }
    Ok(receipts)
}

fn compound_task_type(run: &RunState) -> String {
    run.scope
        .get("task_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| "compound_run_close".to_string())
}

fn thg_runtime_error(error: rustyred_thg_core::ThgError) -> HarnessRuntimeError {
    HarnessRuntimeError::Store(GraphStoreError::new(error.code, error.message))
}

fn append_compound_event<S: GraphStore>(
    store: &mut S,
    run: &RunState,
    event_type: &str,
    trigger: &EventState,
    payload: Value,
) -> RuntimeResult<()> {
    append_transition_from_store(
        store,
        TransitionInput {
            run_id: run.run_id.clone(),
            event_type: event_type.to_string(),
            payload: payload.as_object().cloned().unwrap_or_default(),
            actor: "compound-engineering".to_string(),
            idempotency_key: format!("compound:{}:{event_type}", run.run_id),
            created_at: trigger.created_at.clone(),
        },
    )?;
    Ok(())
}

fn capture_run_if_qualifies<S: GraphStore>(
    store: &mut S,
    run: &RunState,
    trigger: &EventState,
    events: &[EventState],
    config: &CompoundConfig,
    config_hash: &str,
    cluster_key: &str,
    outcome: &OutcomeClass,
    used: &UsedItems,
) -> RuntimeResult<String> {
    if !run_qualifies_for_capture(events, config) {
        return Ok(String::new());
    }
    let summary = outcome_summary(run, events);
    let doc_id = format!("compound:capture:{}", run.run_id);
    let content = format!(
        "Run {} closed with {} outcome.\n\n{}\n\nCluster: {}",
        run.run_id,
        outcome.as_str(),
        summary,
        cluster_key
    );
    let memory = encode_memory(
        store,
        MemoryWriteInput {
            tenant_slug: tenant_for_run(run),
            actor_id: run.actor.clone(),
            session_id: run
                .scope
                .get("session_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            origin_surface: "compound-engineering".to_string(),
            project_slug: run
                .scope
                .get("repo")
                .and_then(Value::as_str)
                .unwrap_or("Theorem")
                .to_string(),
            doc_id: doc_id.clone(),
            kind: outcome.encode_kind().to_string(),
            title: format!("Compound capture: {}", run.task_signature),
            content,
            summary,
            tags: vec![
                COMPOUND_CAPTURE_TAG.to_string(),
                format!("{COMPOUND_CLUSTER_TAG_PREFIX}{cluster_key}"),
            ],
            links: vec![run.run_id.clone()],
            metadata: Map::from_iter([
                ("run_id".to_string(), Value::String(run.run_id.clone())),
                (
                    "cluster_key".to_string(),
                    Value::String(cluster_key.to_string()),
                ),
                (
                    "config_hash".to_string(),
                    Value::String(config_hash.to_string()),
                ),
                (
                    "used_pack_hashes".to_string(),
                    Value::Array(
                        used.packs
                            .values()
                            .map(|pack| Value::String(pack.pack_content_hash.clone()))
                            .collect(),
                    ),
                ),
                (
                    "used_pack_ids".to_string(),
                    Value::Array(
                        used.packs
                            .values()
                            .filter(|pack| !pack.pack_id.is_empty())
                            .map(|pack| Value::String(pack.pack_id.clone()))
                            .collect(),
                    ),
                ),
                (
                    "retrieval_attributions".to_string(),
                    serde_json::to_value(
                        used.retrieval_attributions
                            .values()
                            .cloned()
                            .collect::<Vec<_>>(),
                    )
                    .unwrap_or_else(|_| Value::Array(Vec::new())),
                ),
            ]),
            created_at: trigger.created_at.clone(),
            ..MemoryWriteInput::default()
        },
        crate::memory::EncodeMemoryInput {
            outcome: outcome.as_str().to_string(),
            signal: outcome.signal().to_string(),
            reason: "auto-captured from compound run-close hook".to_string(),
            event_id: trigger.event_id.clone(),
            context: json!({
                "run_id": run.run_id,
                "cluster_key": cluster_key,
                "config_hash": config_hash,
            }),
            auto_triggered: true,
        },
    )
    .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(memory.doc_id)
}

fn run_qualifies_for_capture(events: &[EventState], config: &CompoundConfig) -> bool {
    let has_outcome = events
        .iter()
        .any(|event| matches!(event.event_type.as_str(), "OUTCOME.RECORDED" | "RUN.FAILED"));
    let has_validation = events
        .iter()
        .any(|event| event.event_type.starts_with("VALIDATION."));
    let has_contribution = events.iter().any(|event| {
        matches!(
            event.event_type.as_str(),
            "SKILL.APPLIED" | "TOOL.SELECTED" | "SESSION.EVENT_RECORDED" | "CONTEXT.PACKED"
        ) || event.payload.contains_key(STYLE_RECEIPTS_FIELD)
            || event.payload.get("selected_tools").is_some()
            || event.payload.get("memory_doc_ids").is_some()
    });
    has_outcome && (has_validation || has_contribution || events.len() >= config.capture_step_floor)
}

fn apply_usage_fitness<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    run_id: &str,
    events: &[EventState],
    config: &CompoundConfig,
    config_hash: &str,
    cluster_key: &str,
    outcome: &OutcomeClass,
    used: &UsedItems,
    run_counter: u64,
    trigger: &EventState,
) -> RuntimeResult<(Vec<Value>, Vec<Value>)> {
    if *outcome == OutcomeClass::Negative
        && (!used.packs.is_empty() || !used.memory_doc_ids.is_empty())
    {
        write_negative_tension(store, tenant, run_id, config_hash, used, trigger)?;
    }

    let mut proposals = Vec::new();
    let mut demotions = Vec::new();
    for pack in used.packs.values() {
        let Ok(mut state) = get_skill_pack(
            store,
            SkillPackGetInput {
                tenant_slug: tenant.to_string(),
                pack_id: pack.pack_id.clone(),
                pack_content_hash: pack.pack_content_hash.clone(),
            },
        ) else {
            continue;
        };
        let gate_axes = gate_axes_for_pack(&pack.pack_content_hash, events);
        let signal = OutcomeSignal::from_outcome(
            run_id,
            outcome,
            gate_axes.clone(),
            config_hash,
            cluster_key,
            run_counter,
        );
        update_pack_compound_metadata(&mut state, &signal);
        let hard_axis_regression = signal.hard_axis_regression();
        if state.status == "shadow" && benchmark_gate_passed(&state, &gate_axes, config) {
            let proposal = json!({
                "pack_id": state.pack_id,
                "pack_content_hash": state.pack_content_hash,
                "from": "shadow",
                "to": "advisory",
                "reason": "benchmark gate passed",
                "run_id": run_id,
            });
            write_gate_record(store, tenant, "promotion proposal", &proposal, trigger)?;
            proposals.push(proposal);
        }
        if state.status == "advisory"
            && advisory_gate_passed(&state, config.advisory_promotion_run_count)
        {
            let proposal = json!({
                "pack_id": state.pack_id,
                "pack_content_hash": state.pack_content_hash,
                "from": "advisory",
                "to": "validated",
                "reason": "configured positive run threshold passed with no hard regressions",
                "run_id": run_id,
            });
            write_gate_record(store, tenant, "promotion proposal", &proposal, trigger)?;
            proposals.push(proposal);
        }
        if state.status == "canonical"
            && hard_axis_regression
            && config.canonical_demotes_on_hard_axis_regression
        {
            state.status = "advisory".to_string();
            let demotion = json!({
                "pack_id": state.pack_id,
                "pack_content_hash": state.pack_content_hash,
                "from": "canonical",
                "to": "advisory",
                "reason": "hard gate axis regressed",
                "run_id": run_id,
            });
            write_demotion_tension(store, tenant, &demotion, trigger)?;
            demotions.push(demotion);
        }
        upsert_skill_pack_state(store, state)?;
    }

    if *outcome == OutcomeClass::Positive {
        for doc_id in &used.memory_doc_ids {
            update_memory_positive_fitness(store, tenant, doc_id, run_id, run_counter, trigger)?;
        }
    }

    Ok((proposals, demotions))
}

fn update_pack_compound_metadata(state: &mut SkillPackState, signal: &OutcomeSignal) {
    apply_compound_standing(state, signal);
}

fn apply_compound_standing_to_metadata(
    metadata: &mut Map<String, Value>,
    signal: &OutcomeSignal,
) -> CompoundStandingReceipt {
    let mut compound = metadata
        .get("compound")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut ledger = compound
        .get("ledger")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if !ledger
        .iter()
        .any(|entry| entry.get("run_id").and_then(Value::as_str) == Some(signal.run_id.as_str()))
    {
        ledger.push(json!({
            "run_id": signal.run_id,
            "outcome": signal.outcome,
            "signal": signal.signal,
            "cluster_key": signal.cluster_key,
            "config_hash": signal.config_hash,
            "gate_axes": signal.gate_axes,
        }));
    }
    compound.insert("ledger".to_string(), Value::Array(ledger));
    compound.insert(
        "last_used_run_counter".to_string(),
        Value::Number(signal.run_counter.into()),
    );

    let mut fitness = metadata
        .get("fitness")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut compound_fitness = fitness
        .get("compound")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    increment_u64(&mut compound_fitness, "run_count");
    if signal.is_positive() {
        increment_u64(&mut compound_fitness, "positive_count");
    }
    if signal.hard_axis_regression() {
        increment_u64(&mut compound_fitness, "hard_axis_regressions");
    }
    compound_fitness.insert(
        "last_outcome".to_string(),
        Value::String(signal.outcome.clone()),
    );
    compound_fitness.insert(
        "last_run_id".to_string(),
        Value::String(signal.run_id.clone()),
    );
    compound_fitness.insert("low_fitness".to_string(), Value::Bool(false));
    compound_fitness.insert(
        "last_used_run_counter".to_string(),
        Value::Number(signal.run_counter.into()),
    );
    let run_count = compound_fitness
        .get("run_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let positive_count = compound_fitness
        .get("positive_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let hard_axis_regressions = compound_fitness
        .get("hard_axis_regressions")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    fitness.insert("compound".to_string(), Value::Object(compound_fitness));
    if let Some(axis_map) = signal.gate_axes.as_object() {
        for (axis, value) in axis_map {
            fitness.insert(axis.clone(), value.clone());
        }
    }
    metadata.insert("fitness".to_string(), Value::Object(fitness));
    metadata.insert("compound".to_string(), Value::Object(compound));
    CompoundStandingReceipt {
        run_count,
        positive_count,
        hard_axis_regressions,
        last_outcome: signal.outcome.clone(),
        ..CompoundStandingReceipt::default()
    }
}

fn gate_axes_for_pack(pack_content_hash: &str, events: &[EventState]) -> Value {
    let receipts = events
        .iter()
        .flat_map(|event| {
            event
                .payload
                .get(STYLE_RECEIPTS_FIELD)
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        })
        .filter(|receipt| {
            let hash = receipt
                .get("receipt")
                .and_then(|value| value.get("pack_hash"))
                .or_else(|| receipt.get("pack_hash"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            hash == pack_content_hash
        })
        .collect::<Vec<_>>();
    if receipts.is_empty() {
        return json!({});
    }
    json!({
        "writing_engineering": summarize_style_receipts_for_fitness(&receipts)
    })
}

fn collect_used_items<S: GraphStore>(
    store: &S,
    tenant: &str,
    run_id: &str,
    events: &[EventState],
) -> UsedItems {
    let mut used = UsedItems::default();
    for event in events {
        collect_pack_from_payload(&mut used, &event.payload);
        collect_memory_from_payload(&mut used, &event.payload);
        collect_retrieval_from_payload(&mut used, &event.payload);
        collect_tools_from_payload(&mut used, &event.payload);
    }
    for node in store
        .query_nodes(
            NodeQuery::label("SkillPackUseReceipt")
                .with_property("tenant_slug", Value::String(normalize_tenant(tenant)))
                .with_property("run_id", Value::String(run_id.to_string())),
        )
        .into_iter()
    {
        let hash = node
            .properties
            .get("pack_content_hash")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if hash.is_empty() {
            continue;
        }
        used.packs.entry(hash.clone()).or_insert(UsedPack {
            pack_id: node
                .properties
                .get("pack_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            pack_content_hash: hash,
        });
    }
    used
}

fn collect_pack_from_payload(used: &mut UsedItems, payload: &Map<String, Value>) {
    if let Some(hash) = text_value(payload.get("pack_content_hash"))
        .or_else(|| text_value(payload.get("pack_hash")))
    {
        let pack_id = text_value(payload.get("pack_id")).unwrap_or_default();
        used.packs.entry(hash.clone()).or_insert(UsedPack {
            pack_id,
            pack_content_hash: hash,
        });
    }
    for receipt in payload
        .get(STYLE_RECEIPTS_FIELD)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let Some(hash) = receipt
            .get("receipt")
            .and_then(|value| value.get("pack_hash"))
            .or_else(|| receipt.get("pack_hash"))
            .and_then(Value::as_str)
            .map(str::to_string)
        else {
            continue;
        };
        let pack_id = receipt
            .get("pack_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        used.packs.entry(hash.clone()).or_insert(UsedPack {
            pack_id,
            pack_content_hash: hash,
        });
    }
}

fn collect_memory_from_payload(used: &mut UsedItems, payload: &Map<String, Value>) {
    for key in [
        "memory_doc_id",
        "memory_doc_ids",
        "cited_memory_doc_id",
        "cited_memory_doc_ids",
        "memory_citations",
    ] {
        collect_strings(payload.get(key), &mut used.memory_doc_ids);
    }
    if let Some(context) = payload
        .get("context_pack")
        .or_else(|| payload.get("context"))
    {
        collect_strings_by_key(context, "memory_doc_ids", &mut used.memory_doc_ids);
        collect_strings_by_key(context, "cited_memory_doc_ids", &mut used.memory_doc_ids);
    }
}

fn collect_retrieval_from_payload(used: &mut UsedItems, payload: &Map<String, Value>) {
    let payload_value = Value::Object(payload.clone());
    let mut memory_doc_ids = BTreeSet::new();
    for key in [
        "memory_doc_id",
        "memory_doc_ids",
        "cited_memory_doc_id",
        "cited_memory_doc_ids",
        "memory_citations",
    ] {
        collect_strings_by_key(&payload_value, key, &mut memory_doc_ids);
    }
    if memory_doc_ids.is_empty() {
        return;
    }

    let query = first_text_by_keys(
        &payload_value,
        &[
            "retrieval_query",
            "memory_query",
            "query",
            "task_query",
            "prompt_query",
        ],
    )
    .unwrap_or_default();
    let query_signature = first_text_by_keys(&payload_value, &["query_signature"])
        .unwrap_or_else(|| signature_for_value(&json!({ "query": query })));
    let seed_signature =
        first_text_by_keys(&payload_value, &["seed_signature"]).unwrap_or_else(|| {
            first_value_by_keys(
                &payload_value,
                &["seed_report", "seed_ids", "seeds", "retrieval_seeds"],
            )
            .map(|value| signature_for_value(&value))
            .unwrap_or_else(|| {
                signature_for_value(&Value::Array(
                    memory_doc_ids.iter().cloned().map(Value::String).collect(),
                ))
            })
        });
    let expansion_depth = first_u64_by_keys(
        &payload_value,
        &["expansion_depth", "retrieval_depth", "ppr_depth", "depth"],
    );
    let mut gap_frontier_choices = BTreeSet::new();
    for key in [
        "gap_frontier_choices",
        "frontier_choices",
        "gap_frontier",
        "retrieval_frontier",
    ] {
        collect_strings_by_key(&payload_value, key, &mut gap_frontier_choices);
    }

    let memory_doc_ids = memory_doc_ids.into_iter().collect::<Vec<_>>();
    let gap_frontier_choices = gap_frontier_choices.into_iter().collect::<Vec<_>>();
    let artifact_id = first_text_by_keys(
        &payload_value,
        &[
            "retrieval_path_id",
            "retrieval_artifact_id",
            "context_artifact_id",
            "artifact_id",
        ],
    )
    .unwrap_or_else(|| {
        format!(
            "retrieval:path:{}",
            stable_value_hash(&json!({
                "query_signature": query_signature,
                "seed_signature": seed_signature,
                "memory_doc_ids": memory_doc_ids,
                "expansion_depth": expansion_depth,
                "gap_frontier_choices": gap_frontier_choices,
            }))
        )
    });
    used.retrieval_attributions
        .entry(artifact_id.clone())
        .or_insert(RetrievalAttribution {
            artifact_id,
            query,
            query_signature,
            seed_signature,
            memory_doc_ids,
            expansion_depth,
            gap_frontier_choices,
        });
}

fn collect_tools_from_payload(used: &mut UsedItems, payload: &Map<String, Value>) {
    for key in ["tool_id", "tool_name", "tools"] {
        collect_strings(payload.get(key), &mut used.tools);
    }
}

fn apply_decay<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    config: &CompoundConfig,
    config_hash: &str,
    run_counter: u64,
    _trigger: &EventState,
) -> RuntimeResult<Vec<Value>> {
    let mut decayed = Vec::new();
    if config.decay_window_runs == 0 {
        return Ok(decayed);
    }

    for node in store
        .query_nodes(
            NodeQuery::label("MemoryDocument")
                .with_property("tenant_slug", Value::String(normalize_tenant(tenant))),
        )
        .into_iter()
    {
        let Ok(mut document) = serde_json::from_value::<MemoryDocumentState>(node.properties)
        else {
            continue;
        };
        let last = compound_last_used_counter(document.fitness.as_ref());
        if last == 0 || run_counter.saturating_sub(last) < config.decay_window_runs {
            continue;
        }
        mark_memory_low_fitness(&mut document, run_counter, config_hash);
        upsert_memory_document_state(store, &document)?;
        decayed.push(json!({
            "item_type": "memory",
            "doc_id": document.doc_id,
            "reason": "compound decay window elapsed"
        }));
    }

    for node in store
        .query_nodes(
            NodeQuery::label("SkillPack")
                .with_property("tenant_slug", Value::String(normalize_tenant(tenant))),
        )
        .into_iter()
    {
        let Ok(mut state) = serde_json::from_value::<SkillPackState>(node.properties) else {
            continue;
        };
        let fitness = state.metadata.get("fitness");
        let last = fitness
            .and_then(|value| value.get("compound"))
            .and_then(|value| value.get("last_used_run_counter"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if last == 0 || run_counter.saturating_sub(last) < config.decay_window_runs {
            continue;
        }
        let mut metadata = state.metadata.clone();
        let mut fitness_map = metadata
            .get("fitness")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut compound = fitness_map
            .get("compound")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        compound.insert("low_fitness".to_string(), Value::Bool(true));
        compound.insert(
            "decayed_at_run_counter".to_string(),
            Value::Number(run_counter.into()),
        );
        compound.insert(
            "decay_config_hash".to_string(),
            Value::String(config_hash.to_string()),
        );
        fitness_map.insert("compound".to_string(), Value::Object(compound));
        metadata.insert("fitness".to_string(), Value::Object(fitness_map));
        state.metadata = metadata;
        upsert_skill_pack_state(store, state.clone())?;
        decayed.push(json!({
            "item_type": "skill_pack",
            "pack_content_hash": state.pack_content_hash,
            "reason": "compound decay window elapsed"
        }));
    }
    Ok(decayed)
}

fn update_memory_positive_fitness<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    doc_id: &str,
    run_id: &str,
    run_counter: u64,
    trigger: &EventState,
) -> RuntimeResult<()> {
    let Some(mut document) = load_memory_document(store, tenant, doc_id)
        .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))?
    else {
        return Ok(());
    };
    let mut fitness = document
        .fitness
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut compound = fitness
        .get("compound")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    increment_u64(&mut compound, "positive_count");
    increment_u64(&mut compound, "run_count");
    compound.insert("last_run_id".to_string(), Value::String(run_id.to_string()));
    compound.insert(
        "last_used_run_counter".to_string(),
        Value::Number(run_counter.into()),
    );
    compound.insert("low_fitness".to_string(), Value::Bool(false));
    fitness.insert("compound".to_string(), Value::Object(compound));
    document.fitness = Some(Value::Object(fitness));
    document.updated_at = trigger.created_at.clone();
    upsert_memory_document_state(store, &document)
}

fn mark_memory_low_fitness(
    document: &mut MemoryDocumentState,
    run_counter: u64,
    config_hash: &str,
) {
    let mut fitness = document
        .fitness
        .as_ref()
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut compound = fitness
        .get("compound")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    compound.insert("low_fitness".to_string(), Value::Bool(true));
    compound.insert(
        "decayed_at_run_counter".to_string(),
        Value::Number(run_counter.into()),
    );
    compound.insert(
        "decay_config_hash".to_string(),
        Value::String(config_hash.to_string()),
    );
    fitness.insert("compound".to_string(), Value::Object(compound));
    document.fitness = Some(Value::Object(fitness));
}

fn upsert_memory_document_state<S: GraphStore>(
    store: &mut S,
    document: &MemoryDocumentState,
) -> RuntimeResult<()> {
    let mut properties = serde_json::to_value(document)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    insert_search_text(&mut properties, document);
    store.upsert_node(NodeRecord::new(
        memory_document_node_id(&document.tenant_slug, &document.doc_id),
        ["HarnessMemory", "MemoryDocument"],
        properties,
    ))?;
    Ok(())
}

fn upsert_skill_pack_state<S: GraphStore>(
    store: &mut S,
    state: SkillPackState,
) -> RuntimeResult<()> {
    let properties = serde_json::to_value(&state)
        .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    store.upsert_node(NodeRecord::new(
        skill_pack_node_id(&state.tenant_slug, &state.pack_content_hash),
        ["CapabilityPack", "SkillPack"],
        properties,
    ))?;
    Ok(())
}

fn increment_run_counter<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    trigger: &EventState,
) -> RuntimeResult<u64> {
    let node_id = compound_state_node_id(tenant);
    let mut properties = store
        .get_node(&node_id)
        .map(|node| node.properties.clone())
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let next = properties
        .get("run_counter")
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    properties.insert(
        "tenant_slug".to_string(),
        Value::String(normalize_tenant(tenant)),
    );
    properties.insert("run_counter".to_string(), Value::Number(next.into()));
    properties.insert(
        "updated_at".to_string(),
        Value::String(trigger.created_at.clone()),
    );
    store.upsert_node(NodeRecord::new(
        node_id,
        [COMPOUND_STATE_NODE_LABEL],
        Value::Object(properties),
    ))?;
    Ok(next)
}

fn write_negative_tension<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    run_id: &str,
    config_hash: &str,
    used: &UsedItems,
    trigger: &EventState,
) -> RuntimeResult<()> {
    let summary =
        format!("Negative compound outcome for run {run_id}; used items flagged for review.");
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: COMPOUND_ROOM_ID.to_string(),
            actor_id: "compound-engineering".to_string(),
            record_id: format!("compound:tension:{run_id}"),
            record_type: "tension".to_string(),
            title: "Compound review tension".to_string(),
            summary,
            body: "A failed run does not localize blame to any one used item. The items are named for review only; no fitness decrement was applied.".to_string(),
            metadata: Map::from_iter([
                ("run_id".to_string(), Value::String(run_id.to_string())),
                ("config_hash".to_string(), Value::String(config_hash.to_string())),
                (
                    "used_pack_hashes".to_string(),
                    Value::Array(
                        used.packs
                            .values()
                            .map(|pack| Value::String(pack.pack_content_hash.clone()))
                            .collect(),
                    ),
                ),
                (
                    "used_memory_doc_ids".to_string(),
                    Value::Array(used.memory_doc_ids.iter().cloned().map(Value::String).collect()),
                ),
            ]),
            created_at: trigger.created_at.clone(),
        },
    )
    .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(())
}

fn write_gate_record<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    title: &str,
    payload: &Value,
    trigger: &EventState,
) -> RuntimeResult<()> {
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: COMPOUND_ROOM_ID.to_string(),
            actor_id: "compound-engineering".to_string(),
            record_id: format!(
                "compound:gate:{}:{}",
                payload
                    .get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or("run"),
                payload
                    .get("pack_content_hash")
                    .and_then(Value::as_str)
                    .unwrap_or("pack")
            ),
            record_type: "event".to_string(),
            title: title.to_string(),
            summary: payload.to_string(),
            metadata: Map::from_iter([("proposal".to_string(), payload.clone())]),
            created_at: trigger.created_at.clone(),
            ..WriteRecordInput::default()
        },
    )
    .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(())
}

fn write_demotion_tension<S: GraphStore>(
    store: &mut S,
    tenant: &str,
    payload: &Value,
    trigger: &EventState,
) -> RuntimeResult<()> {
    write_record(
        store,
        WriteRecordInput {
            tenant_slug: tenant.to_string(),
            room_id: COMPOUND_ROOM_ID.to_string(),
            actor_id: "compound-engineering".to_string(),
            record_id: format!(
                "compound:demotion:{}:{}",
                payload
                    .get("run_id")
                    .and_then(Value::as_str)
                    .unwrap_or("run"),
                payload
                    .get("pack_content_hash")
                    .and_then(Value::as_str)
                    .unwrap_or("pack")
            ),
            record_type: "tension".to_string(),
            title: "Compound canonical demotion".to_string(),
            summary: payload.to_string(),
            metadata: Map::from_iter([("demotion".to_string(), payload.clone())]),
            created_at: trigger.created_at.clone(),
            ..WriteRecordInput::default()
        },
    )
    .map_err(|error| HarnessRuntimeError::Serialization(error.to_string()))?;
    Ok(())
}

fn benchmark_gate_passed(
    state: &SkillPackState,
    gate_axes: &Value,
    config: &CompoundConfig,
) -> bool {
    if !config.shadow_benchmark_gate_required {
        return true;
    }
    state
        .metadata
        .get("benchmark_gate_passed")
        .and_then(Value::as_bool)
        .unwrap_or(false)
        || state
            .metadata
            .get("compound")
            .and_then(|value| value.get("benchmark_gate_passed"))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        || gate_axes
            .get("writing_engineering")
            .and_then(|value| value.get("style_receipt_count"))
            .and_then(Value::as_u64)
            .map(|count| count > 0)
            .unwrap_or(false)
}

fn advisory_gate_passed(state: &SkillPackState, threshold: u64) -> bool {
    let compound = state
        .metadata
        .get("fitness")
        .and_then(|value| value.get("compound"));
    compound
        .and_then(|value| value.get("positive_count"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        >= threshold
        && compound
            .and_then(|value| value.get("hard_axis_regressions"))
            .and_then(Value::as_u64)
            .unwrap_or(0)
            == 0
}

fn classify_outcome(run: &RunState, events: &[EventState]) -> OutcomeClass {
    if events.iter().any(|event| event.event_type == "RUN.FAILED") {
        return OutcomeClass::Negative;
    }
    let payload = events
        .iter()
        .rev()
        .find(|event| event.event_type == "OUTCOME.RECORDED")
        .map(|event| Value::Object(event.payload.clone()))
        .or_else(|| run.outcome.clone().map(Value::Object))
        .unwrap_or(Value::Null);
    if let Some(outcome) = payload.get("outcome").and_then(Value::as_str) {
        return match outcome.trim().to_ascii_lowercase().as_str() {
            "positive" => OutcomeClass::Positive,
            "negative" => OutcomeClass::Negative,
            "mixed" => OutcomeClass::Mixed,
            _ => OutcomeClass::Neutral,
        };
    }
    let accepted = payload.get("accepted").and_then(Value::as_bool);
    let tests_passed = payload.get("tests_passed").and_then(Value::as_bool);
    match (accepted, tests_passed) {
        (Some(true), Some(true)) => OutcomeClass::Positive,
        (Some(false), _) | (_, Some(false)) => OutcomeClass::Negative,
        (Some(true), None) => OutcomeClass::Mixed,
        _ => OutcomeClass::Neutral,
    }
}

fn outcome_summary(run: &RunState, events: &[EventState]) -> String {
    events
        .iter()
        .rev()
        .find_map(|event| {
            event
                .payload
                .get("summary")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    event
                        .payload
                        .get("message")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
        })
        .or_else(|| {
            run.outcome
                .as_ref()
                .and_then(|payload| payload.get("summary"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| "No outcome summary recorded.".to_string())
}

fn cluster_key_for_run(run: &RunState, events: &[EventState], tenant: &str) -> String {
    let task_type = events
        .iter()
        .find_map(|event| {
            event
                .payload
                .get("task_type")
                .or_else(|| event.payload.get("taskType"))
                .and_then(Value::as_str)
        })
        .unwrap_or("task");
    let surface = run
        .scope
        .get("surface")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            run.scope
                .get("agent_host")
                .and_then(Value::as_str)
                .unwrap_or("harness")
        });
    let intent = if run.task_signature.trim().is_empty() {
        run.scope
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
    } else {
        &run.task_signature
    };
    let terms = normalize_terms(intent);
    format!(
        "{}:{}:{}:{}",
        normalize_tenant(tenant),
        normalize_token(surface),
        normalize_token(task_type),
        terms.join("-")
    )
}

fn tenant_for_run(run: &RunState) -> String {
    run.scope
        .get("tenant_slug")
        .or_else(|| run.scope.get("tenant"))
        .and_then(Value::as_str)
        .map(normalize_tenant)
        .unwrap_or_else(|| "default".to_string())
}

fn insert_search_text(properties: &mut Value, document: &MemoryDocumentState) {
    let text = [
        document.title.as_str(),
        document.summary.as_str(),
        document.content.as_str(),
        &document.tags.join(" "),
    ]
    .into_iter()
    .filter(|part| !part.trim().is_empty())
    .collect::<Vec<_>>()
    .join("\n");
    if let Some(map) = properties.as_object_mut() {
        map.insert("search_text".to_string(), Value::String(text));
    }
}

fn increment_u64(map: &mut Map<String, Value>, key: &str) {
    let next = map.get(key).and_then(Value::as_u64).unwrap_or(0) + 1;
    map.insert(key.to_string(), Value::Number(next.into()));
}

fn compound_last_used_counter(fitness: Option<&Value>) -> u64 {
    fitness
        .and_then(|value| value.get("compound"))
        .and_then(|value| value.get("last_used_run_counter"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
}

fn collect_strings(value: Option<&Value>, output: &mut BTreeSet<String>) {
    match value {
        Some(Value::String(text)) => {
            let text = text.trim();
            if !text.is_empty() {
                output.insert(text.to_string());
            }
        }
        Some(Value::Array(items)) => {
            for item in items {
                collect_strings(Some(item), output);
            }
        }
        Some(Value::Object(map)) => {
            for item in map.values() {
                collect_strings(Some(item), output);
            }
        }
        _ => {}
    }
}

fn collect_strings_by_key(value: &Value, key: &str, output: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            if let Some(found) = map.get(key) {
                collect_strings(Some(found), output);
            }
            for item in map.values() {
                collect_strings_by_key(item, key, output);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_strings_by_key(item, key, output);
            }
        }
        _ => {}
    }
}

fn first_text_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(text) = text_value(map.get(*key)) {
                    return Some(text);
                }
            }
            map.values().find_map(|item| first_text_by_keys(item, keys))
        }
        Value::Array(items) => items.iter().find_map(|item| first_text_by_keys(item, keys)),
        _ => None,
    }
}

fn first_u64_by_keys(value: &Value, keys: &[&str]) -> Option<u64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(number) = map.get(*key).and_then(Value::as_u64) {
                    return Some(number);
                }
            }
            map.values().find_map(|item| first_u64_by_keys(item, keys))
        }
        Value::Array(items) => items.iter().find_map(|item| first_u64_by_keys(item, keys)),
        _ => None,
    }
}

fn first_value_by_keys(value: &Value, keys: &[&str]) -> Option<Value> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    return Some(value.clone());
                }
            }
            map.values()
                .find_map(|item| first_value_by_keys(item, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|item| first_value_by_keys(item, keys)),
        _ => None,
    }
}

fn signature_for_value(value: &Value) -> String {
    format!("sha256:{}", stable_value_hash(value))
}

fn text_value(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn normalize_terms(value: &str) -> Vec<String> {
    let mut terms = value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(normalize_token)
        .filter(|term| term.len() > 2)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    terms.truncate(8);
    terms
}

fn normalize_token(value: &str) -> String {
    value
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect()
}

fn normalize_tenant(value: &str) -> String {
    let value = value.trim().to_ascii_lowercase();
    if value.is_empty() {
        "default".to_string()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordination::{read_records_for_room, CoordinationRecordState};
    use crate::event_log::{append_transition_from_store, load_events};
    use crate::memory::{create_memory_document, recall_memory, RecallMemoryInput};
    use crate::skill_pack::{
        apply_skill_pack, get_skill_pack, publish_skill_pack, SkillPackApplyInput,
        SkillPackGetInput, SkillPackPublishInput,
    };
    use prose_check::{pack_hash, writing_engineering_pack_payload};
    use rustyred_thg_affordances::{
        affordance_node_id, task_type_node_id, upsert_affordance, Affordance,
        INVOCATION_RECEIPT_LABEL, PRODUCED_OUTCOME, SERVED_TASK,
    };
    use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery};
    use serde_json::json;
    use theorem_harness_core::TransitionInput;

    const TS: &str = "2026-06-08T00:00:00Z";

    #[test]
    fn run_close_hook_appends_compound_events_and_replay_does_not_double_capture() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);

        close_successful_run(
            &mut store,
            "run-compound-close",
            "Encode writing engineering",
            "Patch done. Tests pass.",
            &[],
            &[],
        );

        let events = load_events(&store, "run-compound-close").unwrap();
        let compound = compound_event_types(&events);
        assert_eq!(
            compound,
            vec![
                "COMPOUND.CAPTURED",
                "COMPOUND.FITNESS_APPLIED",
                "COMPOUND.GATE_PROPOSED",
                "COMPOUND.DECAYED"
            ]
        );
        assert!(events
            .iter()
            .any(|event| event.event_type == "COMPOUND.CAPTURED"
                && event.payload["captured"] == json!(true)));

        append_transition_from_store(
            &mut store,
            transition(
                "run-compound-close",
                "RUN.REPLAYED",
                json!({ "source_run_id": "run-compound-close" }),
            ),
        )
        .unwrap();
        let replayed_events = load_events(&store, "run-compound-close").unwrap();
        assert_eq!(compound_event_types(&replayed_events).len(), 4);
    }

    #[test]
    fn auto_capture_writes_one_memory_and_trivial_run_writes_zero() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);

        close_successful_run(
            &mut store,
            "run-capture-positive",
            "Encode a pack",
            "Patch done. Tests pass.",
            &[],
            &[],
        );
        let hits = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "default".to_string(),
                query: "run-capture-positive".to_string(),
                limit: 10,
                hydrate: true,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].kind, "solution");
        assert_eq!(
            hits[0].document.as_ref().unwrap().fitness.as_ref().unwrap()["auto_triggered"],
            json!(true)
        );

        append_transition_from_store(
            &mut store,
            transition(
                "run-trivial",
                "RUN.CREATED",
                json!({
                    "task": "trivial",
                    "actor": "codex",
                    "scope": { "tenant_slug": "default", "agent_host": "codex" }
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            &mut store,
            transition(
                "run-trivial",
                "RUN.FAILED",
                json!({ "error_code": "noop", "message": "trivial" }),
            ),
        )
        .unwrap();
        let trivial_hits = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "default".to_string(),
                query: "run-trivial".to_string(),
                limit: 10,
                include_low_fitness: true,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(trivial_hits.is_empty());
    }

    #[test]
    fn positive_run_updates_pack_and_memory_negative_run_writes_tension_without_decrement() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: "default".to_string(),
                doc_id: "doc-used".to_string(),
                kind: "solution".to_string(),
                title: "Used memory".to_string(),
                content: "Reusable run context.".to_string(),
                fitness: Some(json!({
                    "compound": {
                        "run_count": 0,
                        "positive_count": 0,
                        "last_used_run_counter": 0,
                        "low_fitness": false
                    }
                })),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        close_successful_run(
            &mut store,
            "run-positive",
            "Encode writing engineering",
            "Patch done. Tests pass.",
            &[],
            &["doc-used"],
        );

        let pack = get_skill_pack(
            &store,
            SkillPackGetInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                ..SkillPackGetInput::default()
            },
        )
        .unwrap();
        assert_eq!(
            pack.metadata["fitness"]["compound"]["positive_count"],
            json!(1)
        );
        assert_eq!(
            pack.metadata["fitness"]["writing_engineering"]["style_receipt_count"],
            json!(1)
        );
        let memory = load_memory_document(&store, "default", "doc-used")
            .unwrap()
            .unwrap();
        assert_eq!(
            memory.fitness.unwrap()["compound"]["positive_count"],
            json!(1)
        );

        apply_skill_pack(
            &mut store,
            SkillPackApplyInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                actor_id: "codex".to_string(),
                run_id: "run-negative".to_string(),
                task: "failed task".to_string(),
                receipt_id: "receipt-negative".to_string(),
                ..SkillPackApplyInput::default()
            },
        )
        .unwrap();
        append_transition_from_store(
            &mut store,
            transition(
                "run-negative",
                "RUN.CREATED",
                json!({
                    "task": "failed task",
                    "actor": "codex",
                    "scope": { "tenant_slug": "default", "agent_host": "codex" }
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            &mut store,
            transition(
                "run-negative",
                "RUN.FAILED",
                json!({ "error_code": "test_failed", "message": "tests failed" }),
            ),
        )
        .unwrap();

        let after_negative = get_skill_pack(
            &store,
            SkillPackGetInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                ..SkillPackGetInput::default()
            },
        )
        .unwrap();
        assert_eq!(
            after_negative.metadata["fitness"]["compound"]["positive_count"],
            json!(1)
        );
        let tensions = compound_tensions(&store);
        assert!(tensions
            .iter()
            .any(|record| record.summary.contains("run-negative")));
    }

    #[test]
    fn gate_proposal_decay_cluster_and_config_hash_are_data_driven() {
        let mut store = InMemoryGraphStore::new();
        persist_compound_config(
            &mut store,
            "default",
            CompoundConfig {
                capture_step_floor: 4,
                decay_window_runs: 1,
                ..CompoundConfig::default()
            },
        )
        .unwrap();
        publish_writing_pack(&mut store, "shadow", true);
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: "default".to_string(),
                doc_id: "doc-decay".to_string(),
                kind: "solution".to_string(),
                title: "Decay candidate".to_string(),
                content: "Recall should rehearse this memory.".to_string(),
                fitness: Some(json!({
                    "compound": {
                        "last_used_run_counter": 0,
                        "low_fitness": false
                    }
                })),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        close_successful_run(
            &mut store,
            "run-cluster-a",
            "Encode Django plugin pack",
            "Patch done. Tests pass.",
            &[],
            &["doc-decay"],
        );
        close_successful_run(
            &mut store,
            "run-cluster-b",
            "Encode Django plugin pack",
            "Patch done. Tests pass.",
            &[],
            &[],
        );
        close_successful_run(
            &mut store,
            "run-cluster-c",
            "Fix unrelated browser job",
            "Patch done. Tests pass.",
            &[],
            &[],
        );

        let captured_a = compound_event(&store, "run-cluster-a", "COMPOUND.CAPTURED");
        let captured_b = compound_event(&store, "run-cluster-b", "COMPOUND.CAPTURED");
        let captured_c = compound_event(&store, "run-cluster-c", "COMPOUND.CAPTURED");
        assert_eq!(
            captured_a.payload["cluster_key"],
            captured_b.payload["cluster_key"]
        );
        assert_ne!(
            captured_a.payload["cluster_key"],
            captured_c.payload["cluster_key"]
        );

        let gate = compound_event(&store, "run-cluster-a", "COMPOUND.GATE_PROPOSED");
        assert!(!gate.payload["promotion_proposals"]
            .as_array()
            .unwrap()
            .is_empty());

        let default_hits = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "default".to_string(),
                query: "Decay candidate".to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert!(default_hits.is_empty());
        let low_hits = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "default".to_string(),
                query: "Decay candidate".to_string(),
                include_low_fitness: true,
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(low_hits.len(), 1);
        let reheard_hits = recall_memory(
            &mut store,
            RecallMemoryInput {
                tenant_slug: "default".to_string(),
                query: "Decay candidate".to_string(),
                limit: 10,
                ..RecallMemoryInput::default()
            },
        )
        .unwrap();
        assert_eq!(reheard_hits.len(), 1);

        let old_hash = captured_a.payload["config_hash"]
            .as_str()
            .unwrap()
            .to_string();
        persist_compound_config(
            &mut store,
            "default",
            CompoundConfig {
                capture_step_floor: 9,
                decay_window_runs: 1,
                ..CompoundConfig::default()
            },
        )
        .unwrap();
        close_successful_run(
            &mut store,
            "run-config-changed",
            "Encode Django plugin pack",
            "Patch done. Tests pass.",
            &[],
            &[],
        );
        let new_hash = compound_event(&store, "run-config-changed", "COMPOUND.CAPTURED").payload
            ["config_hash"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(old_hash, new_hash);
    }

    #[test]
    fn canonical_hard_axis_regression_demotes_and_records_tension() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "canonical", true);

        close_successful_run(
            &mut store,
            "run-canonical-regression",
            "Encode writing engineering",
            "The runtime module changed.",
            &["rustyred-web/src/lib.rs"],
            &[],
        );

        let pack = get_skill_pack(
            &store,
            SkillPackGetInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                ..SkillPackGetInput::default()
            },
        )
        .unwrap();
        assert_eq!(pack.status, "advisory");
        let gate = compound_event(&store, "run-canonical-regression", "COMPOUND.GATE_PROPOSED");
        assert!(!gate.payload["demotions"].as_array().unwrap().is_empty());
        assert!(compound_tensions(&store)
            .iter()
            .any(|record| record.title == "Compound canonical demotion"));
    }

    #[test]
    fn list_compound_captures_filters_by_compound_tag_cluster_outcome_and_since() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);

        // Two positive (solution) captures in cluster A (same task signature ->
        // same cluster_key), one positive in cluster B, and one failed run that
        // qualifies for capture as a postmortem.
        close_successful_run_at(
            &mut store,
            "run-alpha-1",
            "Encode Django plugin pack",
            "Patch done. Tests pass.",
            "2026-06-08T00:00:00Z",
        );
        close_successful_run_at(
            &mut store,
            "run-alpha-2",
            "Encode Django plugin pack",
            "Patch done. Tests pass.",
            "2026-06-08T01:00:00Z",
        );
        close_successful_run_at(
            &mut store,
            "run-bravo-1",
            "Fix unrelated browser job",
            "Patch done. Tests pass.",
            "2026-06-08T02:00:00Z",
        );
        close_failed_qualifying_run_at(
            &mut store,
            "run-charlie-fail",
            "Encode Django plugin pack",
            "Tests failed hard.",
            "2026-06-08T03:00:00Z",
        );

        // A non-compound MemoryDocument in the same tenant: must never appear.
        create_memory_document(
            &mut store,
            MemoryWriteInput {
                tenant_slug: "default".to_string(),
                doc_id: "doc-plain".to_string(),
                kind: "solution".to_string(),
                title: "Plain memory".to_string(),
                content: "Not a compound capture.".to_string(),
                tags: vec!["unrelated".to_string()],
                created_at: "2026-06-08T04:00:00Z".to_string(),
                ..MemoryWriteInput::default()
            },
        )
        .unwrap();

        // (a) all compound captures: the four hook-written docs, newest first, and
        // not the plain doc.
        let all = list_compound_captures(&store, "default", None, None, None).unwrap();
        let all_ids = all
            .iter()
            .map(|document| document.doc_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            all_ids,
            vec![
                "compound:capture:run-charlie-fail",
                "compound:capture:run-bravo-1",
                "compound:capture:run-alpha-2",
                "compound:capture:run-alpha-1",
            ]
        );
        assert!(all
            .iter()
            .all(|document| document.tags.iter().any(|tag| tag == COMPOUND_CAPTURE_TAG)));
        assert!(!all_ids.contains(&"doc-plain"));

        // (b) a specific cluster_key: the two alpha runs plus the failed charlie run
        // share the Django cluster; the browser run does not.
        let cluster_key = all
            .iter()
            .find(|document| document.doc_id == "compound:capture:run-alpha-1")
            .and_then(|document| document.metadata.get("cluster_key"))
            .and_then(Value::as_str)
            .unwrap()
            .to_string();
        let by_cluster =
            list_compound_captures(&store, "default", Some(&cluster_key), None, None).unwrap();
        let mut by_cluster_ids = by_cluster
            .iter()
            .map(|document| document.doc_id.clone())
            .collect::<Vec<_>>();
        by_cluster_ids.sort();
        assert_eq!(
            by_cluster_ids,
            vec![
                "compound:capture:run-alpha-1".to_string(),
                "compound:capture:run-alpha-2".to_string(),
                "compound:capture:run-charlie-fail".to_string(),
            ]
        );
        // Cluster filter also matches the `cluster:<key>` tag, independent of metadata.
        assert!(by_cluster
            .iter()
            .all(|document| document.tags.contains(&format!("cluster:{cluster_key}"))));

        // (c) a specific outcome kind: only the failed run is a postmortem; the
        // positive runs are solutions.
        let postmortems =
            list_compound_captures(&store, "default", None, Some("postmortem"), None).unwrap();
        assert_eq!(postmortems.len(), 1);
        assert_eq!(postmortems[0].doc_id, "compound:capture:run-charlie-fail");
        assert_eq!(postmortems[0].kind, "postmortem");

        let solutions =
            list_compound_captures(&store, "default", None, Some("solution"), None).unwrap();
        assert_eq!(solutions.len(), 3);
        assert!(solutions.iter().all(|document| document.kind == "solution"));

        // An outcome outside the encode-kind set matches nothing.
        assert!(
            list_compound_captures(&store, "default", None, Some("not-a-kind"), None)
                .unwrap()
                .is_empty()
        );

        // (d) a since-watermark: only captures with updated_at >= the watermark.
        let since =
            list_compound_captures(&store, "default", None, None, Some("2026-06-08T02:00:00Z"))
                .unwrap();
        let since_ids = since
            .iter()
            .map(|document| document.doc_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            since_ids,
            vec![
                "compound:capture:run-charlie-fail",
                "compound:capture:run-bravo-1",
            ]
        );
    }

    #[test]
    fn list_compound_captures_is_read_only_and_does_not_change_fitness() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);
        close_successful_run(
            &mut store,
            "run-readonly",
            "Encode writing engineering",
            "Patch done. Tests pass.",
            &[],
            &[],
        );

        let before = load_memory_document(&store, "default", "compound:capture:run-readonly")
            .unwrap()
            .unwrap();
        let before_fitness = before.fitness.clone();
        let before_updated_at = before.updated_at.clone();

        // Read repeatedly: a recall path would bump compound fitness; this reader
        // must not.
        for _ in 0..3 {
            let captures = list_compound_captures(&store, "default", None, None, None).unwrap();
            assert!(captures
                .iter()
                .any(|document| document.doc_id == "compound:capture:run-readonly"));
        }

        let after = load_memory_document(&store, "default", "compound:capture:run-readonly")
            .unwrap()
            .unwrap();
        assert_eq!(after.fitness, before_fitness);
        assert_eq!(after.updated_at, before_updated_at);
        assert_eq!(after, before);
    }

    #[test]
    fn generic_standing_update_core_records_outcome_signal() {
        #[derive(Default)]
        struct TestArtifact {
            metadata: Map<String, Value>,
        }

        impl CompoundingArtifact for TestArtifact {
            fn compound_artifact_type(&self) -> &'static str {
                "calibration_model"
            }

            fn compound_artifact_id(&self) -> String {
                "calibration:v1".to_string()
            }

            fn compound_metadata_mut(&mut self) -> &mut Map<String, Value> {
                &mut self.metadata
            }
        }

        let mut artifact = TestArtifact::default();
        let signal = OutcomeSignal {
            run_id: "run-signal".to_string(),
            outcome: "positive".to_string(),
            signal: "pinned".to_string(),
            config_hash: "cfg:1".to_string(),
            cluster_key: "cluster:a".to_string(),
            run_counter: 7,
            gate_axes: json!({
                "calibration": { "brier": 0.12 },
                "writing_engineering": { "last_hard_axis_failed": true }
            }),
        };

        let receipt = apply_compound_standing(&mut artifact, &signal);

        assert_eq!(receipt.artifact_type, "calibration_model");
        assert_eq!(receipt.artifact_id, "calibration:v1");
        assert_eq!(receipt.run_count, 1);
        assert_eq!(receipt.positive_count, 1);
        assert_eq!(receipt.hard_axis_regressions, 1);
        assert_eq!(
            artifact.metadata["fitness"]["compound"]["last_used_run_counter"],
            json!(7)
        );
        assert_eq!(
            artifact.metadata["fitness"]["calibration"]["brier"],
            json!(0.12)
        );
        assert_eq!(
            artifact.metadata["compound"]["ledger"][0]["signal"],
            json!("pinned")
        );
    }

    #[test]
    fn run_close_receipt_records_retrieval_path_attribution() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "shadow", true);

        drive_run_preamble(
            &mut store,
            "run-retrieval-attribution",
            "Use similar prior memory for a fix",
        );
        append_transition_from_store(
            &mut store,
            transition(
                "run-retrieval-attribution",
                "SESSION.EVENT_RECORDED",
                json!({
                    "event_subtype": "memory_retrieval",
                    "retrieval_path_id": "retrieval:similar-fix",
                    "retrieval_query": "similar prior memory for failing wake route",
                    "seed_signature": "seed:repo-main",
                    "expansion_depth": 2,
                    "gap_frontier_choices": ["coordination_push", "receiver_wake"],
                    "memory_doc_ids": ["doc-a", "doc-b"]
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            &mut store,
            transition(
                "run-retrieval-attribution",
                "OUTCOME.RECORDED",
                json!({
                    "accepted": true,
                    "tests_passed": true,
                    "manual_override": true,
                    "validator_results": [],
                    "files_changed": [],
                    "summary": "accepted"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            &mut store,
            transition(
                "run-retrieval-attribution",
                "RUN.CLOSED",
                json!({
                    "summary": "closed",
                    "closed_by": "codex",
                    "source_identifiers": []
                }),
            ),
        )
        .unwrap();

        let fitness = compound_event(
            &store,
            "run-retrieval-attribution",
            "COMPOUND.FITNESS_APPLIED",
        );
        let attributions = fitness.payload["retrieval_attributions"]
            .as_array()
            .unwrap();
        assert_eq!(attributions.len(), 1);
        assert_eq!(
            attributions[0]["artifact_id"],
            json!("retrieval:similar-fix")
        );
        assert_eq!(
            attributions[0]["query"],
            json!("similar prior memory for failing wake route")
        );
        assert_eq!(attributions[0]["seed_signature"], json!("seed:repo-main"));
        assert_eq!(attributions[0]["memory_doc_ids"], json!(["doc-a", "doc-b"]));

        let capture = load_memory_document(
            &store,
            "default",
            "compound:capture:run-retrieval-attribution",
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            capture.metadata["retrieval_attributions"][0]["expansion_depth"],
            json!(2)
        );
    }

    #[test]
    fn run_close_hook_records_registered_tool_affordance_outcomes() {
        let mut store = InMemoryGraphStore::new();
        upsert_affordance(
            &mut store,
            Affordance {
                tenant_id: "default".to_string(),
                affordance_id: "apply_patch".to_string(),
                server_id: "codex-tools".to_string(),
                tool_name: "apply_patch".to_string(),
                family: "developer_tool".to_string(),
                label: "apply_patch".to_string(),
                description: "Apply a source patch".to_string(),
                input_schema: json!({ "type": "object" }),
                permissions: vec!["graph:write".to_string()],
                cost: json!({}),
                writeback_policy: "write".to_string(),
                tags: vec!["compound".to_string()],
                embedding: None,
                fitness: 0.5,
                version: 1,
                created_at_ms: 1,
                manifest_version: 1,
            },
            Some("test"),
        )
        .unwrap();

        close_successful_run(
            &mut store,
            "run-affordance-close",
            "Patch the runtime close hook",
            "Patch done. Tests pass.",
            &[],
            &[],
        );

        let fitness = compound_event(&store, "run-affordance-close", "COMPOUND.FITNESS_APPLIED");
        let recorded = fitness.payload["recorded_affordance_receipts"]
            .as_array()
            .expect("recorded affordance receipts");
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0]["tool_id"], json!("apply_patch"));

        let receipt_node_id = recorded[0]["receipt_node_id"]
            .as_str()
            .expect("receipt node id");
        let receipt = store
            .get_node(receipt_node_id)
            .expect("invocation receipt node");
        assert!(receipt
            .labels
            .iter()
            .any(|label| label == INVOCATION_RECEIPT_LABEL));
        assert_eq!(receipt.properties["outcome_label"], json!("positive"));
        assert_eq!(receipt.properties["outcome_value"], json!(1.0));
        assert_eq!(receipt.properties["task_type"], json!("compound_run_close"));

        let affordance_node = affordance_node_id("default", "apply_patch");
        let task_type_node = task_type_node_id("default", "compound_run_close");
        assert!(store
            .neighbors(NeighborQuery::out(&affordance_node).with_edge_type(SERVED_TASK))
            .iter()
            .any(|hit| hit.node_id == task_type_node));
        assert!(store
            .neighbors(NeighborQuery::out(&affordance_node).with_edge_type(PRODUCED_OUTCOME))
            .iter()
            .any(|hit| hit.node_id == receipt_node_id));
    }

    #[test]
    fn run_created_registry_status_drives_next_close_receipt_action() {
        let mut store = InMemoryGraphStore::new();
        publish_writing_pack(&mut store, "advisory", true);

        close_successful_run_for_tenant(
            &mut store,
            "Travis-Gilbert",
            "run-status-bridge",
            "Encode writing engineering",
            "Patch done. Tests pass.",
            &["must-preserve-id"],
            &[],
        );

        let run = crate::event_log::load_run(&store, "run-status-bridge")
            .unwrap()
            .unwrap();
        assert_eq!(run.scope["writing_engineering_status"], json!("advisory"));
        assert_eq!(
            run.scope["writing_engineering_origin_tenant"],
            json!("default")
        );

        let close = load_events(&store, "run-status-bridge")
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == "RUN.CLOSED")
            .unwrap();
        let receipt = &close.payload[STYLE_RECEIPTS_FIELD][0];
        assert_eq!(receipt["pack_status"], json!("advisory"));
        assert_eq!(receipt["action"], json!("advisory_context"));
    }

    fn publish_writing_pack(store: &mut InMemoryGraphStore, status: &str, benchmark_passed: bool) {
        let mut pack = writing_engineering_pack_payload(None);
        pack["metadata"]["benchmark_gate_passed"] = json!(benchmark_passed);
        publish_skill_pack(
            store,
            SkillPackPublishInput {
                tenant_slug: "default".to_string(),
                pack_content_hash: pack_hash(),
                status: status.to_string(),
                pack,
                ..SkillPackPublishInput::default()
            },
        )
        .unwrap();
    }

    fn close_successful_run(
        store: &mut InMemoryGraphStore,
        run_id: &str,
        task: &str,
        close_summary: &str,
        source_identifiers: &[&str],
        memory_doc_ids: &[&str],
    ) {
        close_successful_run_for_tenant(
            store,
            "default",
            run_id,
            task,
            close_summary,
            source_identifiers,
            memory_doc_ids,
        );
    }

    fn close_successful_run_for_tenant(
        store: &mut InMemoryGraphStore,
        tenant_slug: &str,
        run_id: &str,
        task: &str,
        close_summary: &str,
        source_identifiers: &[&str],
        memory_doc_ids: &[&str],
    ) {
        append_transition_from_store(
            store,
            transition(
                run_id,
                "RUN.CREATED",
                json!({
                    "task": task,
                    "actor": "codex",
                    "scope": {
                        "tenant_slug": tenant_slug,
                        "repo": "Theorem",
                        "agent_host": "codex"
                    }
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "HOST.OBSERVED",
                json!({
                    "repo": "Theorem",
                    "branch": "main",
                    "commit_sha": "abc123",
                    "cwd": "/repo/Theorem"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(run_id, "TASK.RESOLVED", json!({ "task_signature": task })),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "PROFILE.SELECTED",
                json!({
                    "profile_id": "codex",
                    "profile_version": "1",
                    "policy_hash": "policy:1"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "TOOLKIT.COMPILED",
                json!({
                    "selected_tools": ["apply_patch", "cargo test"],
                    "selected_plugins": [],
                    "excluded_tools": [],
                    "permission_reasons": {},
                    "tool_permission_requirements": {},
                    "policy_receipts": []
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.PLANNED",
                json!({
                    "budget_tokens": 4000,
                    "plan_hash": "plan:1",
                    "candidate_token_count": 1200
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.PACKED",
                json!({
                    "artifact_id": "ctx:1",
                    "capsule_tokens": 1000,
                    "budget_tokens": 4000,
                    "included_atom_count": 2,
                    "excluded_atom_count": 0,
                    "token_ledger": {},
                    "memory_doc_ids": memory_doc_ids
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.INJECTED",
                json!({
                    "artifact_id": "ctx:1",
                    "adapter": "codex",
                    "target": "active_context"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "AGENT.ACTING",
                json!({
                    "adapter": "codex",
                    "started_at": TS
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "SESSION.EVENT_RECORDED",
                json!({
                    "event_subtype": "tool_invocation",
                    "tool_id": "apply_patch"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "OUTCOME.RECORDED",
                json!({
                    "accepted": true,
                    "tests_passed": true,
                    "manual_override": true,
                    "validator_results": [],
                    "files_changed": [],
                    "summary": "accepted"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "RUN.CLOSED",
                json!({
                    "summary": close_summary,
                    "closed_by": "codex",
                    "source_identifiers": source_identifiers
                }),
            ),
        )
        .unwrap();
    }

    fn transition(run_id: &str, event_type: &str, payload: Value) -> TransitionInput {
        transition_at(run_id, event_type, payload, TS)
    }

    fn transition_at(
        run_id: &str,
        event_type: &str,
        payload: Value,
        created_at: &str,
    ) -> TransitionInput {
        TransitionInput {
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: payload.as_object().cloned().unwrap_or_default(),
            actor: "codex".to_string(),
            idempotency_key: format!("{run_id}:{event_type}"),
            created_at: created_at.to_string(),
        }
    }

    /// Drive a run through the full guard-valid preamble (RUN.CREATED through
    /// AGENT.ACTING, including the PROFILE/TOOLKIT/CONTEXT phases the state machine
    /// requires), up to but not including the terminal outcome/close events. The
    /// packed context makes the run qualify for capture. Early events are stamped at
    /// `TS`; ordering is by sequence, so a later terminal timestamp is fine. This
    /// mirrors the event sequence in `close_successful_run_for_tenant`.
    fn drive_run_preamble(store: &mut InMemoryGraphStore, run_id: &str, task: &str) {
        append_transition_from_store(
            store,
            transition(
                run_id,
                "RUN.CREATED",
                json!({
                    "task": task,
                    "actor": "codex",
                    "scope": {
                        "tenant_slug": "default",
                        "repo": "Theorem",
                        "agent_host": "codex"
                    }
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "HOST.OBSERVED",
                json!({
                    "repo": "Theorem",
                    "branch": "main",
                    "commit_sha": "abc123",
                    "cwd": "/repo/Theorem"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(run_id, "TASK.RESOLVED", json!({ "task_signature": task })),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "PROFILE.SELECTED",
                json!({
                    "profile_id": "codex",
                    "profile_version": "1",
                    "policy_hash": "policy:1"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "TOOLKIT.COMPILED",
                json!({
                    "selected_tools": ["apply_patch", "cargo test"],
                    "selected_plugins": [],
                    "excluded_tools": [],
                    "permission_reasons": {},
                    "tool_permission_requirements": {},
                    "policy_receipts": []
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.PLANNED",
                json!({
                    "budget_tokens": 4000,
                    "plan_hash": "plan:1",
                    "candidate_token_count": 1200
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.PACKED",
                json!({
                    "artifact_id": "ctx:1",
                    "capsule_tokens": 1000,
                    "budget_tokens": 4000,
                    "included_atom_count": 2,
                    "excluded_atom_count": 0,
                    "token_ledger": {},
                    "memory_doc_ids": []
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "CONTEXT.INJECTED",
                json!({
                    "artifact_id": "ctx:1",
                    "adapter": "codex",
                    "target": "active_context"
                }),
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition(
                run_id,
                "AGENT.ACTING",
                json!({
                    "adapter": "codex",
                    "started_at": TS
                }),
            ),
        )
        .unwrap();
    }

    /// Drive the full positive run-close capture path, stamping the closing
    /// transition (and therefore the capture's `updated_at`) at `closed_at` so the
    /// `since` watermark can be exercised.
    fn close_successful_run_at(
        store: &mut InMemoryGraphStore,
        run_id: &str,
        task: &str,
        close_summary: &str,
        closed_at: &str,
    ) {
        drive_run_preamble(store, run_id, task);
        append_transition_from_store(
            store,
            transition_at(
                run_id,
                "OUTCOME.RECORDED",
                json!({
                    "accepted": true,
                    "tests_passed": true,
                    "manual_override": true,
                    "validator_results": [],
                    "files_changed": [],
                    "summary": "accepted"
                }),
                closed_at,
            ),
        )
        .unwrap();
        append_transition_from_store(
            store,
            transition_at(
                run_id,
                "RUN.CLOSED",
                json!({
                    "summary": close_summary,
                    "closed_by": "codex",
                    "source_identifiers": []
                }),
                closed_at,
            ),
        )
        .unwrap();
    }

    /// Drive a failed run rich enough to qualify for capture (it carries a packed
    /// context contribution plus a RUN.FAILED outcome), producing a `postmortem`
    /// capture. The closing RUN.FAILED transition is stamped at `closed_at`.
    fn close_failed_qualifying_run_at(
        store: &mut InMemoryGraphStore,
        run_id: &str,
        task: &str,
        close_summary: &str,
        closed_at: &str,
    ) {
        drive_run_preamble(store, run_id, task);
        append_transition_from_store(
            store,
            transition_at(
                run_id,
                "RUN.FAILED",
                json!({
                    "error_code": "test_failed",
                    "message": close_summary,
                    "summary": close_summary
                }),
                closed_at,
            ),
        )
        .unwrap();
    }

    fn compound_event_types(events: &[EventState]) -> Vec<&str> {
        events
            .iter()
            .filter(|event| event.event_type.starts_with("COMPOUND."))
            .map(|event| event.event_type.as_str())
            .collect()
    }

    fn compound_event(store: &InMemoryGraphStore, run_id: &str, event_type: &str) -> EventState {
        load_events(store, run_id)
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == event_type)
            .unwrap()
    }

    fn compound_tensions(store: &InMemoryGraphStore) -> Vec<CoordinationRecordState> {
        read_records_for_room(
            store,
            "default",
            COMPOUND_ROOM_ID,
            &["tension".to_string()],
            20,
        )
        .unwrap()
    }
}
