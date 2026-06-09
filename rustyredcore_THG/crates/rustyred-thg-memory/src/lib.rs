use std::collections::{BTreeMap, BTreeSet, HashMap};

use rustyred_thg_core::{
    now_ms, personalized_pagerank, stable_hash, EdgeRecord, EpistemicType, GraphStore,
    GraphStoreError, GraphStoreResult, NeighborQuery, NodeQuery, NodeRecord, PluginCapability,
    PluginCapabilityKind, PluginOperationContext, PluginOperationRegistration, PluginRegistry,
    RustyRedPlugin,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const MEMORY_DOCUMENT_LABEL: &str = "MemoryDocument";
pub const MEMORY_NODE_LABEL: &str = "MemoryNode";
pub const MEMORY_SUMMARY_LABEL: &str = "MemorySummary";
pub const HARNESS_MEMORY_LABEL: &str = "HarnessMemory";
pub const DERIVED_FROM: &str = "DERIVED_FROM";
pub const SUPPORTS: &str = "supports";
pub const MEMORY_PLUGIN_SOURCE: &str = "rustyred_thg_memory";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MemoryRecallInput {
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default, alias = "tenant_slug")]
    pub tenant_slug: String,
    #[serde(default)]
    pub query: String,
    #[serde(default)]
    pub seeds: Vec<String>,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
    #[serde(default)]
    pub edge_type_weights: BTreeMap<String, f64>,
    #[serde(default)]
    pub as_of_ms: Option<i64>,
    #[serde(default = "default_budget_tokens")]
    pub budget_tokens: i64,
    #[serde(default = "default_true")]
    pub bump_activation: bool,
}

impl Default for MemoryRecallInput {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            tenant_slug: String::new(),
            query: String::new(),
            seeds: Vec::new(),
            top_k: default_top_k(),
            edge_type_weights: BTreeMap::new(),
            as_of_ms: None,
            budget_tokens: default_budget_tokens(),
            bump_activation: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RankedMemory {
    pub id: String,
    pub graph_id: String,
    pub title: String,
    pub summary: String,
    pub content_preview: String,
    pub score: f64,
    pub activation: f64,
    pub fitness: f64,
    pub recency: f64,
    pub estimated_tokens: i64,
    pub provenance: Map<String, Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct RankedMemories {
    pub tenant_id: String,
    pub total_candidates: usize,
    pub returned: usize,
    pub budget_tokens: i64,
    pub estimated_tokens: i64,
    pub memories: Vec<RankedMemory>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ConsolidateInput {
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default, alias = "tenant_slug")]
    pub tenant_slug: String,
    #[serde(default)]
    pub actor: String,
    #[serde(default = "default_max_groups")]
    pub max_groups: usize,
    #[serde(default)]
    pub now_ms: Option<i64>,
}

impl Default for ConsolidateInput {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            tenant_slug: String::new(),
            actor: String::new(),
            max_groups: default_max_groups(),
            now_ms: None,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ConsolidateOutput {
    pub tenant_id: String,
    pub groups_merged: usize,
    pub source_nodes_archived: usize,
    pub summary_nodes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DecayInput {
    #[serde(default)]
    pub tenant_id: String,
    #[serde(default, alias = "tenant_slug")]
    pub tenant_slug: String,
    #[serde(default)]
    pub now_ms: Option<i64>,
    #[serde(default = "default_inactive_after_ms")]
    pub inactive_after_ms: i64,
    #[serde(default = "default_activation_threshold")]
    pub activation_threshold: f64,
}

impl Default for DecayInput {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            tenant_slug: String::new(),
            now_ms: None,
            inactive_after_ms: default_inactive_after_ms(),
            activation_threshold: default_activation_threshold(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct DecayOutput {
    pub tenant_id: String,
    pub demoted: usize,
    pub archive_nodes: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BeliefRevisionOutput {
    pub invalidated_edge_id: String,
    pub invalid_at_ms: i64,
    pub flagged_dependents: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct MemoryPlugin;

pub fn builtin_memory_plugin_registry() -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    registry.register(MemoryPlugin);
    registry
}

impl RustyRedPlugin for MemoryPlugin {
    fn name(&self) -> &'static str {
        "rustyred-thg-memory"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![
            PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: "memory.recall".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: "memory.consolidate".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: "memory.decay".to_string(),
            },
        ]
    }

    fn operations(&self) -> Vec<PluginOperationRegistration> {
        vec![
            PluginOperationRegistration {
                operation: "recall",
                command: "memory.recall",
                aliases: &["rustyred.thg.memory.recall"],
                summary: "Rank tenant memories with PPR-seeded retrieval under a token budget.",
                writes_graph: true,
                handler: handle_recall_operation,
            },
            PluginOperationRegistration {
                operation: "consolidate",
                command: "memory.consolidate",
                aliases: &["rustyred.thg.memory.consolidate"],
                summary: "Merge duplicate memory atoms into summary nodes with provenance edges.",
                writes_graph: true,
                handler: handle_consolidate_operation,
            },
            PluginOperationRegistration {
                operation: "decay",
                command: "memory.decay",
                aliases: &["rustyred.thg.memory.decay"],
                summary: "Demote stale low-activation memories to the archive tier.",
                writes_graph: true,
                handler: handle_decay_operation,
            },
        ]
    }
}

pub fn recall<S: GraphStore>(
    store: &mut S,
    input: MemoryRecallInput,
) -> GraphStoreResult<RankedMemories> {
    let tenant_id = normalized_tenant(&input);
    let as_of_ms = input.as_of_ms.unwrap_or_else(now_ms);
    let top_k = input.top_k.max(1);
    let budget_tokens = input.budget_tokens.max(1);
    let nodes = memory_nodes(store, &tenant_id, true)?
        .into_iter()
        .filter(|node| node_visible_for_recall(node, as_of_ms))
        .collect::<Vec<_>>();
    let id_set = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let seeds = recall_seeds(&nodes, &id_set, &input);
    let adjacency = memory_adjacency(store, &id_set, as_of_ms, &input.edge_type_weights)?;
    let ppr = if seeds.is_empty() {
        HashMap::new()
    } else {
        personalized_pagerank(&adjacency, &seeds, 0.15, 1e-5, 20_000)
    };

    let mut ranked = nodes
        .iter()
        .map(|node| {
            ranked_memory(
                node,
                &input.query,
                ppr.get(&node.id).copied().unwrap_or(0.0),
            )
        })
        .collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.graph_id.cmp(&right.graph_id))
    });

    let mut selected = Vec::new();
    let mut estimated_tokens = 0;
    for item in ranked.into_iter() {
        if selected.len() >= top_k {
            break;
        }
        if estimated_tokens + item.estimated_tokens > budget_tokens && !selected.is_empty() {
            break;
        }
        estimated_tokens += item.estimated_tokens;
        selected.push(item);
    }

    if input.bump_activation {
        for item in &selected {
            if let Some(mut node) = store.get_node(&item.graph_id).cloned() {
                set_prop_f64(&mut node.properties, "activation", item.activation + 1.0);
                set_prop_i64(&mut node.properties, "last_accessed_ms", as_of_ms);
                store.upsert_node(node)?;
            }
        }
    }

    Ok(RankedMemories {
        tenant_id,
        total_candidates: nodes.len(),
        returned: selected.len(),
        budget_tokens,
        estimated_tokens,
        memories: selected,
    })
}

pub fn consolidate<S: GraphStore>(
    store: &mut S,
    input: ConsolidateInput,
) -> GraphStoreResult<ConsolidateOutput> {
    let tenant_id = normalized_tenant_pair(&input.tenant_id, &input.tenant_slug);
    let now = input.now_ms.unwrap_or_else(now_ms);
    let mut groups: BTreeMap<String, Vec<NodeRecord>> = BTreeMap::new();
    for node in memory_nodes(store, &tenant_id, false)? {
        if prop_bool(&node.properties, "archive_tier") {
            continue;
        }
        groups
            .entry(consolidation_key(&node))
            .or_default()
            .push(node);
    }

    let mut output = ConsolidateOutput {
        tenant_id: tenant_id.clone(),
        ..ConsolidateOutput::default()
    };
    for (_key, group) in groups
        .into_iter()
        .filter(|(_, group)| group.len() > 1)
        .take(input.max_groups.max(1))
    {
        let summary_id = summary_node_id(&tenant_id, &group);
        if store.get_node(&summary_id).is_some() {
            continue;
        }
        let title = group
            .iter()
            .find_map(|node| prop_str(&node.properties, "title"))
            .unwrap_or_else(|| "Consolidated memory".to_string());
        let summary = group
            .iter()
            .filter_map(|node| {
                prop_str(&node.properties, "summary")
                    .or_else(|| prop_str(&node.properties, "content"))
            })
            .take(3)
            .collect::<Vec<_>>()
            .join("\n");
        let summary_node = NodeRecord::new(
            summary_id.clone(),
            [HARNESS_MEMORY_LABEL, MEMORY_SUMMARY_LABEL],
            json!({
                "tenant_id": tenant_id,
                "tenant_slug": tenant_id,
                "title": title,
                "summary": summary,
                "status": "active",
                "kind": "summary",
                "source": MEMORY_PLUGIN_SOURCE,
                "created_at_ms": now,
                "updated_at_ms": now,
                "activation": 1.0,
                "writeback_policy": "summary_node",
            }),
        );
        store.upsert_node(summary_node)?;
        for mut source in group {
            set_prop_bool(&mut source.properties, "archive_tier", true);
            set_prop_str(&mut source.properties, "status", "archived");
            set_prop_i64(&mut source.properties, "archived_at_ms", now);
            store.upsert_node(source.clone())?;
            store.upsert_edge(EdgeRecord::new(
                memory_edge_id(&tenant_id, DERIVED_FROM, &summary_id, &source.id),
                summary_id.clone(),
                DERIVED_FROM,
                source.id,
                json!({ "tenant_id": tenant_id, "created_at_ms": now }),
            ))?;
            output.source_nodes_archived += 1;
        }
        output.groups_merged += 1;
        output.summary_nodes.push(summary_id);
    }
    Ok(output)
}

pub fn decay<S: GraphStore>(store: &mut S, input: DecayInput) -> GraphStoreResult<DecayOutput> {
    let tenant_id = normalized_tenant_pair(&input.tenant_id, &input.tenant_slug);
    let now = input.now_ms.unwrap_or_else(now_ms);
    let cutoff = now.saturating_sub(input.inactive_after_ms.max(1));
    let mut output = DecayOutput {
        tenant_id: tenant_id.clone(),
        ..DecayOutput::default()
    };
    for mut node in memory_nodes(store, &tenant_id, false)? {
        if prop_bool(&node.properties, "archive_tier") {
            continue;
        }
        let last_accessed = prop_i64(&node.properties, "last_accessed_ms")
            .or_else(|| prop_i64(&node.properties, "updated_at_ms"))
            .unwrap_or(0);
        let activation = prop_f64(&node.properties, "activation").unwrap_or(0.0);
        if last_accessed <= cutoff && activation <= input.activation_threshold {
            set_prop_bool(&mut node.properties, "archive_tier", true);
            set_prop_str(&mut node.properties, "status", "archived");
            set_prop_i64(&mut node.properties, "archived_at_ms", now);
            store.upsert_node(node.clone())?;
            output.demoted += 1;
            output.archive_nodes.push(node.id);
        }
    }
    Ok(output)
}

pub fn invalidate_memory_edge<S: GraphStore>(
    store: &mut S,
    edge_id: &str,
    invalid_at_ms: i64,
) -> GraphStoreResult<BeliefRevisionOutput> {
    let mut edge = store.get_edge(edge_id).cloned().ok_or_else(|| {
        GraphStoreError::new(
            "missing_memory_edge",
            format!("edge {edge_id} was not found"),
        )
    })?;
    set_prop_i64(&mut edge.properties, "invalid_at_ms", invalid_at_ms);
    edge.epistemic_type = Some(EpistemicType::Contradicts);
    store.upsert_edge(edge.clone())?;

    let mut flagged = Vec::new();
    for root in [&edge.from_id, &edge.to_id] {
        for hit in store.neighbors(NeighborQuery::in_(root).with_include_expired(true)) {
            if hit.edge_type != DERIVED_FROM && hit.edge_type != SUPPORTS {
                continue;
            }
            if let Some(mut dependent) = store.get_node(&hit.node_id).cloned() {
                set_prop_bool(&mut dependent.properties, "stale_derivation", true);
                set_prop_str(&mut dependent.properties, "stale_source_edge_id", edge_id);
                store.upsert_node(dependent)?;
                push_unique(&mut flagged, hit.node_id);
            }
        }
    }

    Ok(BeliefRevisionOutput {
        invalidated_edge_id: edge_id.to_string(),
        invalid_at_ms,
        flagged_dependents: flagged,
    })
}

fn handle_recall_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let mut input: MemoryRecallInput = serde_json::from_value(arguments)
        .map_err(|error| GraphStoreError::new("invalid_memory_recall_input", error.to_string()))?;
    if input.tenant_id.trim().is_empty() && input.tenant_slug.trim().is_empty() {
        input.tenant_id = context.tenant_id.to_string();
    }
    serde_json::to_value(recall(context.store, input)?)
        .map_err(|error| GraphStoreError::new("memory_recall_serialize", error.to_string()))
}

fn handle_consolidate_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let mut input: ConsolidateInput = serde_json::from_value(arguments).map_err(|error| {
        GraphStoreError::new("invalid_memory_consolidate_input", error.to_string())
    })?;
    if input.tenant_id.trim().is_empty() && input.tenant_slug.trim().is_empty() {
        input.tenant_id = context.tenant_id.to_string();
    }
    serde_json::to_value(consolidate(context.store, input)?)
        .map_err(|error| GraphStoreError::new("memory_consolidate_serialize", error.to_string()))
}

fn handle_decay_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let mut input: DecayInput = serde_json::from_value(arguments)
        .map_err(|error| GraphStoreError::new("invalid_memory_decay_input", error.to_string()))?;
    if input.tenant_id.trim().is_empty() && input.tenant_slug.trim().is_empty() {
        input.tenant_id = context.tenant_id.to_string();
    }
    serde_json::to_value(decay(context.store, input)?)
        .map_err(|error| GraphStoreError::new("memory_decay_serialize", error.to_string()))
}

fn memory_nodes<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    include_archived: bool,
) -> GraphStoreResult<Vec<NodeRecord>> {
    let mut nodes = Vec::new();
    for label in [
        MEMORY_DOCUMENT_LABEL,
        MEMORY_NODE_LABEL,
        MEMORY_SUMMARY_LABEL,
    ] {
        for node in store.query_nodes(
            NodeQuery::label(label)
                .with_limit(50_000)
                .with_include_expired(true),
        ) {
            if !node_matches_tenant(&node, tenant_id) {
                continue;
            }
            if !include_archived && prop_bool(&node.properties, "archive_tier") {
                continue;
            }
            if !nodes
                .iter()
                .any(|existing: &NodeRecord| existing.id == node.id)
            {
                nodes.push(node);
            }
        }
    }
    Ok(nodes)
}

fn memory_adjacency<S: GraphStore>(
    store: &S,
    ids: &BTreeSet<String>,
    as_of_ms: i64,
    weights: &BTreeMap<String, f64>,
) -> GraphStoreResult<HashMap<String, Vec<(String, f64)>>> {
    let mut adjacency = HashMap::new();
    for id in ids {
        let mut neighbors = Vec::new();
        for hit in store.neighbors(NeighborQuery::out(id).with_include_expired(true)) {
            if !ids.contains(&hit.node_id) {
                continue;
            }
            if let Some(edge) = store.get_edge(&hit.edge_id) {
                if !edge_valid_at(edge, as_of_ms) {
                    continue;
                }
            }
            let weight = weights.get(&hit.edge_type).copied().unwrap_or(1.0)
                * hit.confidence.unwrap_or(1.0).clamp(0.0, 1.0);
            if weight > 0.0 {
                neighbors.push((hit.node_id, weight));
            }
        }
        adjacency.insert(id.clone(), neighbors);
    }
    Ok(adjacency)
}

fn recall_seeds(
    nodes: &[NodeRecord],
    id_set: &BTreeSet<String>,
    input: &MemoryRecallInput,
) -> HashMap<String, f64> {
    let mut seeds = HashMap::new();
    for seed in &input.seeds {
        if id_set.contains(seed) {
            seeds.insert(seed.clone(), 1.0);
        }
    }
    if !seeds.is_empty() {
        return seeds;
    }
    for node in nodes {
        let score = lexical_score(&input.query, node);
        if score > 0.0 {
            seeds.insert(node.id.clone(), score);
        }
    }
    if seeds.is_empty() {
        for node in nodes.iter().take(8) {
            seeds.insert(node.id.clone(), 1.0);
        }
    }
    seeds
}

fn ranked_memory(node: &NodeRecord, query: &str, ppr_score: f64) -> RankedMemory {
    let activation = prop_f64(&node.properties, "activation").unwrap_or(0.0);
    let fitness = fitness_score(&node.properties);
    let recency = prop_i64(&node.properties, "updated_at_ms")
        .or_else(|| prop_i64(&node.properties, "created_at_ms"))
        .map(|value| (value.max(0) as f64 + 1.0).log10() / 16.0)
        .unwrap_or(0.0);
    let lexical = lexical_score(query, node);
    let score = ppr_score * 2.0 + lexical + activation * 0.05 + fitness * 0.2 + recency;
    let title = prop_str(&node.properties, "title").unwrap_or_else(|| node.id.clone());
    let summary = prop_str(&node.properties, "summary").unwrap_or_default();
    let content = prop_str(&node.properties, "content").unwrap_or_default();
    let estimated_tokens = prop_i64(&node.properties, "estimated_tokens")
        .unwrap_or_else(|| calibrated_tokens(&title, &summary, &content));
    let mut provenance = Map::new();
    provenance.insert("graph_id".to_string(), Value::String(node.id.clone()));
    provenance.insert(
        "source".to_string(),
        Value::String(MEMORY_PLUGIN_SOURCE.to_string()),
    );
    if let Some(source_ref) = prop_str(&node.properties, "source_ref") {
        provenance.insert("source_ref".to_string(), Value::String(source_ref));
    }
    RankedMemory {
        id: prop_str(&node.properties, "doc_id")
            .or_else(|| prop_str(&node.properties, "node_id"))
            .unwrap_or_else(|| node.id.clone()),
        graph_id: node.id.clone(),
        title,
        summary,
        content_preview: content.chars().take(1_000).collect(),
        score,
        activation,
        fitness,
        recency,
        estimated_tokens: estimated_tokens.max(1),
        provenance,
    }
}

fn node_visible_for_recall(node: &NodeRecord, as_of_ms: i64) -> bool {
    if prop_bool(&node.properties, "archive_tier") {
        return false;
    }
    if prop_str(&node.properties, "status").as_deref() == Some("archived") {
        return false;
    }
    node_valid_at(&node.properties, as_of_ms)
}

fn node_valid_at(properties: &Value, as_of_ms: i64) -> bool {
    let valid_at = prop_i64(properties, "valid_at_ms").unwrap_or(i64::MIN);
    let invalid_at = prop_i64(properties, "invalid_at_ms").unwrap_or(i64::MAX);
    valid_at <= as_of_ms && as_of_ms < invalid_at
}

fn edge_valid_at(edge: &EdgeRecord, as_of_ms: i64) -> bool {
    node_valid_at(&edge.properties, as_of_ms)
}

fn consolidation_key(node: &NodeRecord) -> String {
    prop_str(&node.properties, "source_ref")
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("source:{value}"))
        .unwrap_or_else(|| {
            let content = prop_str(&node.properties, "content")
                .or_else(|| prop_str(&node.properties, "summary"))
                .unwrap_or_else(|| node.id.clone());
            format!("content:{}", stable_hash(content))
        })
}

fn summary_node_id(tenant_id: &str, group: &[NodeRecord]) -> String {
    let ids = group
        .iter()
        .map(|node| node.id.as_str())
        .collect::<Vec<_>>();
    format!("mem:summary:{tenant_id}:{}", stable_hash(ids))
}

fn memory_edge_id(tenant_id: &str, edge_type: &str, from_id: &str, to_id: &str) -> String {
    format!(
        "mem:edge:{tenant_id}:{}",
        stable_hash(json!([edge_type, from_id, to_id]))
    )
}

fn normalized_tenant(input: &MemoryRecallInput) -> String {
    normalized_tenant_pair(&input.tenant_id, &input.tenant_slug)
}

fn normalized_tenant_pair(tenant_id: &str, tenant_slug: &str) -> String {
    let raw = if tenant_id.trim().is_empty() {
        tenant_slug
    } else {
        tenant_id
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "default".to_string()
    } else {
        trimmed.to_string()
    }
}

fn node_matches_tenant(node: &NodeRecord, tenant_id: &str) -> bool {
    prop_str(&node.properties, "tenant_id")
        .or_else(|| prop_str(&node.properties, "tenant_slug"))
        .map(|value| value == tenant_id)
        .unwrap_or(false)
}

fn lexical_score(query: &str, node: &NodeRecord) -> f64 {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 0.0;
    }
    let haystack = [
        prop_str(&node.properties, "title").unwrap_or_default(),
        prop_str(&node.properties, "summary").unwrap_or_default(),
        prop_str(&node.properties, "content").unwrap_or_default(),
    ]
    .join(" ")
    .to_lowercase();
    query
        .split_whitespace()
        .filter(|term| haystack.contains(*term))
        .count() as f64
}

fn fitness_score(properties: &Value) -> f64 {
    properties
        .get("fitness")
        .and_then(|fitness| {
            fitness
                .get("score")
                .or_else(|| fitness.get("overall"))
                .and_then(Value::as_f64)
        })
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

fn calibrated_tokens(title: &str, summary: &str, content: &str) -> i64 {
    let bytes = title.len() + summary.len() + content.len().min(1_000);
    ((bytes as i64 + 3) / 4).max(1)
}

fn prop_str(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn prop_i64(properties: &Value, key: &str) -> Option<i64> {
    properties.get(key).and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().map(|value| value as i64))
    })
}

fn prop_f64(properties: &Value, key: &str) -> Option<f64> {
    properties.get(key).and_then(Value::as_f64)
}

fn prop_bool(properties: &Value, key: &str) -> bool {
    properties
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn set_prop_str(properties: &mut Value, key: &str, value: impl Into<String>) {
    ensure_object(properties).insert(key.to_string(), Value::String(value.into()));
}

fn set_prop_i64(properties: &mut Value, key: &str, value: i64) {
    ensure_object(properties).insert(key.to_string(), Value::Number(value.into()));
}

fn set_prop_f64(properties: &mut Value, key: &str, value: f64) {
    if let Some(number) = serde_json::Number::from_f64(value) {
        ensure_object(properties).insert(key.to_string(), Value::Number(number));
    }
}

fn set_prop_bool(properties: &mut Value, key: &str, value: bool) {
    ensure_object(properties).insert(key.to_string(), Value::Bool(value));
}

fn ensure_object(properties: &mut Value) -> &mut Map<String, Value> {
    if !properties.is_object() {
        *properties = Value::Object(Map::new());
    }
    properties.as_object_mut().expect("properties is object")
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn default_top_k() -> usize {
    10
}

fn default_budget_tokens() -> i64 {
    2_000
}

fn default_max_groups() -> usize {
    32
}

fn default_inactive_after_ms() -> i64 {
    14 * 24 * 60 * 60 * 1_000
}

fn default_activation_threshold() -> f64 {
    0.1
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::InMemoryGraphStore;

    fn memory_doc(id: &str, title: &str, content: &str) -> NodeRecord {
        NodeRecord::new(
            id,
            [HARNESS_MEMORY_LABEL, MEMORY_DOCUMENT_LABEL],
            json!({
                "tenant_id": "theorem",
                "doc_id": id,
                "title": title,
                "content": content,
                "summary": content,
                "status": "active",
                "activation": 0.1,
                "fitness": { "score": 1.0 },
                "updated_at_ms": 1000,
            }),
        )
    }

    #[test]
    fn recall_uses_graph_activation_over_recency_dump() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(memory_doc(
                "mem:a",
                "Rust PPR",
                "personalized pagerank recall",
            ))
            .unwrap();
        store
            .upsert_node(memory_doc("mem:b", "Unrelated", "other content"))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "edge:ab",
                "mem:a",
                SUPPORTS,
                "mem:b",
                json!({ "tenant_id": "theorem" }),
            ))
            .unwrap();

        let result = recall(
            &mut store,
            MemoryRecallInput {
                tenant_id: "theorem".to_string(),
                query: "pagerank".to_string(),
                top_k: 2,
                budget_tokens: 100,
                ..MemoryRecallInput::default()
            },
        )
        .unwrap();

        assert_eq!(result.returned, 2);
        assert_eq!(result.memories[0].graph_id, "mem:a");
        assert!(store
            .get_node("mem:a")
            .unwrap()
            .properties
            .get("last_accessed_ms")
            .is_some());
    }

    #[test]
    fn consolidate_archives_duplicates_and_writes_summary_edges() {
        let mut store = InMemoryGraphStore::new();
        let mut first = memory_doc("mem:a", "Same", "duplicate");
        let mut second = memory_doc("mem:b", "Same", "duplicate");
        set_prop_str(&mut first.properties, "source_ref", "source-1");
        set_prop_str(&mut second.properties, "source_ref", "source-1");
        store.upsert_node(first).unwrap();
        store.upsert_node(second).unwrap();

        let output = consolidate(
            &mut store,
            ConsolidateInput {
                tenant_id: "theorem".to_string(),
                ..ConsolidateInput::default()
            },
        )
        .unwrap();

        assert_eq!(output.groups_merged, 1);
        assert_eq!(output.source_nodes_archived, 2);
        let summary = store.get_node(&output.summary_nodes[0]).unwrap();
        assert!(summary.labels.contains(&MEMORY_SUMMARY_LABEL.to_string()));
        assert_eq!(
            store.get_node("mem:a").unwrap().properties["status"].as_str(),
            Some("archived")
        );
    }

    #[test]
    fn decay_demotes_stale_low_activation_memory() {
        let mut store = InMemoryGraphStore::new();
        let mut node = memory_doc("mem:a", "Stale", "old");
        set_prop_i64(&mut node.properties, "last_accessed_ms", 10);
        set_prop_f64(&mut node.properties, "activation", 0.0);
        store.upsert_node(node).unwrap();

        let output = decay(
            &mut store,
            DecayInput {
                tenant_id: "theorem".to_string(),
                now_ms: Some(10_000),
                inactive_after_ms: 1_000,
                activation_threshold: 0.1,
                ..DecayInput::default()
            },
        )
        .unwrap();

        assert_eq!(output.demoted, 1);
        assert_eq!(
            store.get_node("mem:a").unwrap().properties["status"].as_str(),
            Some("archived")
        );
    }

    #[test]
    fn invalidating_source_edge_flags_dependent_derivations() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(memory_doc("mem:source", "Source", "root"))
            .unwrap();
        store
            .upsert_node(memory_doc("mem:derived", "Derived", "child"))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "edge:derived",
                "mem:derived",
                DERIVED_FROM,
                "mem:source",
                json!({ "tenant_id": "theorem" }),
            ))
            .unwrap();

        let output = invalidate_memory_edge(&mut store, "edge:derived", 42).unwrap();

        assert_eq!(output.flagged_dependents, vec!["mem:derived".to_string()]);
        assert_eq!(
            store.get_node("mem:derived").unwrap().properties["stale_derivation"].as_bool(),
            Some(true)
        );
        assert_eq!(
            store.get_edge("edge:derived").unwrap().properties["invalid_at_ms"].as_i64(),
            Some(42)
        );
    }
}
