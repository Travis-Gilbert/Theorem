use crate::coordination::{write_record, WriteRecordInput};
use crate::event_log::{append_transition_from_store, load_events};
use crate::memory::{
    encode_memory, load_memory_document, memory_document_node_id, MemoryDocumentState,
    MemoryWriteInput,
};
use crate::skill_pack::{skill_pack_node_id, SkillPackState};
use crate::writing_style::{summarize_style_receipts_for_fitness, STYLE_RECEIPTS_FIELD};
use crate::{HarnessRuntimeError, RuntimeResult};
use rustyred_thg_core::{GraphStore, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use theorem_harness_core::{stable_value_hash, EventState, RunState, TransitionInput};

pub const COMPOUND_CONFIG_NODE_LABEL: &str = "CompoundEngineeringConfig";
pub const COMPOUND_STATE_NODE_LABEL: &str = "CompoundEngineeringState";
pub const COMPOUND_ROOM_ID: &str = "compound-engineering";

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
    pub promotion_proposals: Vec<Value>,
    pub demotions: Vec<Value>,
    pub decayed_items: Vec<Value>,
}

#[derive(Clone, Debug, Default)]
struct UsedItems {
    packs: BTreeMap<String, UsedPack>,
    memory_doc_ids: BTreeSet<String>,
    tools: BTreeSet<String>,
}

#[derive(Clone, Debug, Default)]
struct UsedPack {
    pack_id: String,
    pack_content_hash: String,
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
            "used_tools": used.tools.iter().cloned().collect::<Vec<_>>(),
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
        promotion_proposals,
        demotions,
        decayed_items,
    })
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
                "compound-engineering".to_string(),
                format!("cluster:{cluster_key}"),
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
        let Some(node) = store
            .get_node(&skill_pack_node_id(tenant, &pack.pack_content_hash))
            .cloned()
        else {
            continue;
        };
        let mut state: SkillPackState = serde_json::from_value(node.properties.clone())
            .map_err(|error| HarnessRuntimeError::Deserialization(error.to_string()))?;
        let gate_axes = gate_axes_for_pack(&pack.pack_content_hash, events);
        update_pack_compound_metadata(
            &mut state,
            run_id,
            outcome,
            &gate_axes,
            config_hash,
            cluster_key,
            run_counter,
        );
        let hard_axis_regression = gate_axes
            .get("writing_engineering")
            .and_then(|value| value.get("last_hard_axis_failed"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
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

fn update_pack_compound_metadata(
    state: &mut SkillPackState,
    run_id: &str,
    outcome: &OutcomeClass,
    gate_axes: &Value,
    config_hash: &str,
    cluster_key: &str,
    run_counter: u64,
) {
    let mut metadata = state.metadata.clone();
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
        .any(|entry| entry.get("run_id").and_then(Value::as_str) == Some(run_id))
    {
        ledger.push(json!({
            "run_id": run_id,
            "outcome": outcome.as_str(),
            "cluster_key": cluster_key,
            "config_hash": config_hash,
            "gate_axes": gate_axes,
        }));
    }
    compound.insert("ledger".to_string(), Value::Array(ledger));
    compound.insert(
        "last_used_run_counter".to_string(),
        Value::Number(run_counter.into()),
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
    if *outcome == OutcomeClass::Positive {
        increment_u64(&mut compound_fitness, "positive_count");
    }
    if gate_axes
        .get("writing_engineering")
        .and_then(|value| value.get("last_hard_axis_failed"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        increment_u64(&mut compound_fitness, "hard_axis_regressions");
    }
    compound_fitness.insert(
        "last_outcome".to_string(),
        Value::String(outcome.as_str().to_string()),
    );
    compound_fitness.insert("last_run_id".to_string(), Value::String(run_id.to_string()));
    compound_fitness.insert("low_fitness".to_string(), Value::Bool(false));
    compound_fitness.insert(
        "last_used_run_counter".to_string(),
        Value::Number(run_counter.into()),
    );
    fitness.insert("compound".to_string(), Value::Object(compound_fitness));
    if let Some(style_axes) = gate_axes.get("writing_engineering") {
        fitness.insert("writing_engineering".to_string(), style_axes.clone());
    }
    metadata.insert("fitness".to_string(), Value::Object(fitness));
    metadata.insert("compound".to_string(), Value::Object(compound));
    state.metadata = metadata;
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

fn collect_tools_from_payload(used: &mut UsedItems, payload: &Map<String, Value>) {
    for key in ["tool_id", "tool_name", "selected_tools", "tools"] {
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
    use rustyred_thg_core::InMemoryGraphStore;
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
                    "source_identifiers": source_identifiers,
                    "writing_engineering_status": "canonical"
                }),
            ),
        )
        .unwrap();
    }

    fn transition(run_id: &str, event_type: &str, payload: Value) -> TransitionInput {
        TransitionInput {
            run_id: run_id.to_string(),
            event_type: event_type.to_string(),
            payload: payload.as_object().cloned().unwrap_or_default(),
            actor: "codex".to_string(),
            idempotency_key: format!("{run_id}:{event_type}"),
            created_at: TS.to_string(),
        }
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
