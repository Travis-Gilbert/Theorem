use std::collections::{BTreeMap, BTreeSet, HashMap};

use rustyred_thg_core::{
    cached_single_seed_personalized_pagerank, checkout_graph_version, compile_graph_pack,
    edge_time_interval, merge_ppr_scores, node_from_content_object, node_to_content_object, now_ms,
    personalized_pagerank, read_epistemic_shadow, stable_hash, update_graph_ref, ActorId,
    ColdIndex, ColdIndexEntry, ColdObjectStore, ColdScopeEntry, ColdTierKind, EdgeRecord,
    EpistemicType, EvictionFrontier, GraphCompileOptions, GraphSnapshot, GraphStore,
    GraphStoreError, GraphStoreResult, GraphVersionRepository, InMemoryColdIndex,
    InMemoryObjectStore, NeighborQuery, NodeQuery, NodeRecord, PluginCapability,
    PluginCapabilityKind, PluginOperationContext, PluginOperationRegistration, PluginRegistry,
    RustyRedPlugin, TimeInterval,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub const MEMORY_DOCUMENT_LABEL: &str = "MemoryDocument";
pub const MEMORY_NODE_LABEL: &str = "MemoryNode";
pub const MEMORY_SUMMARY_LABEL: &str = "MemorySummary";
pub const MEMORY_PROJECT_LABEL: &str = "MemoryProject";
pub const HARNESS_MEMORY_LABEL: &str = "HarnessMemory";
pub const DERIVED_FROM: &str = "DERIVED_FROM";
pub const SUPPORTS: &str = "supports";
pub const MEMORY_IN_PROJECT: &str = "MEMORY_IN_PROJECT";
pub const MEMORY_PLUGIN_SOURCE: &str = "rustyred_thg_memory";

pub mod similarity;
pub use similarity::{
    compute_memory_similarity_edges, HashEmbedder, MemoryEmbedder, SimilarityOptions,
    SimilarityStats, MEMORY_SIMILAR,
};

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
    #[serde(default)]
    pub project_slug: String,
    #[serde(default = "default_project_permeability")]
    pub project_permeability: f64,
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
    #[serde(default)]
    pub include_epistemic: bool,
}

impl Default for MemoryRecallInput {
    fn default() -> Self {
        Self {
            tenant_id: String::new(),
            tenant_slug: String::new(),
            query: String::new(),
            seeds: Vec::new(),
            project_slug: String::new(),
            project_permeability: default_project_permeability(),
            top_k: default_top_k(),
            edge_type_weights: BTreeMap::new(),
            as_of_ms: None,
            budget_tokens: default_budget_tokens(),
            bump_activation: true,
            include_epistemic: false,
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ContradictionPolicy {
    #[serde(default)]
    pub functional_edge_types: BTreeSet<String>,
    #[serde(default)]
    pub mutually_exclusive_edge_types: BTreeMap<String, BTreeSet<String>>,
}

impl ContradictionPolicy {
    pub fn functional(edge_types: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            functional_edge_types: edge_types.into_iter().map(Into::into).collect(),
            mutually_exclusive_edge_types: BTreeMap::new(),
        }
    }

    fn is_functional(&self, edge_type: &str) -> bool {
        self.functional_edge_types.contains(edge_type)
    }

    fn is_mutually_exclusive(&self, left: &str, right: &str) -> bool {
        self.mutually_exclusive_edge_types
            .get(left)
            .map(|values| values.contains(right))
            .unwrap_or(false)
            || self
                .mutually_exclusive_edge_types
                .get(right)
                .map(|values| values.contains(left))
                .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Contradiction {
    pub existing_edge_id: String,
    pub invalidated_at_ms: i64,
    pub by_actor: ActorId,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RecallQuery {
    pub seeds: Vec<String>,
    #[serde(default)]
    pub at_ms: Option<i64>,
    #[serde(default = "default_ppr_alpha")]
    pub alpha: f64,
    #[serde(default = "default_top_k")]
    pub top_k: usize,
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
    let mut id_set = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let project_anchor = project_seed_node(&tenant_id, &input);
    if let Some(anchor) = project_anchor.as_ref() {
        id_set.insert(anchor.clone());
    }
    let seeds = recall_seeds(&nodes, &id_set, &input);
    let adjacency = memory_adjacency(store, &id_set, as_of_ms, &input.edge_type_weights)?;
    let ppr = recall_ppr(
        &adjacency,
        seeds,
        project_anchor.as_deref(),
        &tenant_id,
        store.stats().version,
    );

    let mut ranked = nodes
        .iter()
        .map(|node| {
            ranked_memory(
                node,
                &input.query,
                ppr.get(&node.id).copied().unwrap_or(0.0),
                project_rank_bonus(node, &input),
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

    if input.include_epistemic {
        for item in &mut selected {
            if let Some(readout) = read_epistemic_shadow(store, &item.graph_id) {
                if let Ok(value) = serde_json::to_value(readout) {
                    item.provenance
                        .insert("epistemic_shadow".to_string(), value);
                }
            }
        }
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

// ============================================================================
// Storage spine (cut 6): real eviction past the archive flag.
//
// `decay()` above flags a node archived but leaves it resident in RAM. The cold
// tier closes that loop: after a stale, low-activation node is durably retained
// in the content-addressed cold object store, it is EVICTED from the operating
// store so it stops occupying RAM, and a later recall rehydrates it by a keyed
// cold-index lookup. The operating store stays RAM-first; the cold tail spills
// to disk; eviction is a frontier pop, not a scan; and residency changes never
// bump the graph version, so the PPR cache stays warm for the live nodes.
// ============================================================================

/// The cold tier: a durable content-addressed object store (the cold tail's
/// home), a cold index (id -> tier/object-hash, the keyed rehydration map), and
/// the per-scope eviction frontier (the coldest-first ordered index over
/// `last_accessed_ms`). Bundled so the eviction spine threads one `&mut`.
pub struct ColdTier {
    objects: Box<dyn ColdObjectStore>,
    index: Box<dyn ColdIndex>,
    frontier: EvictionFrontier,
}

impl ColdTier {
    /// Build a cold tier over an explicit object store and cold index (disk- or
    /// Postgres-backed in production, in-memory in tests).
    pub fn new(objects: Box<dyn ColdObjectStore>, index: Box<dyn ColdIndex>) -> Self {
        Self {
            objects,
            index,
            frontier: EvictionFrontier::new(),
        }
    }

    /// An all-in-memory cold tier. For tests/scratch only -- it does not survive
    /// a restart, so it cannot satisfy the durability property; use disk- or
    /// Postgres-backed stores for a real cold tier.
    pub fn in_memory() -> Self {
        Self::new(
            Box::new(InMemoryObjectStore::new()),
            Box::new(InMemoryColdIndex::new()),
        )
    }

    pub fn frontier(&self) -> &EvictionFrontier {
        &self.frontier
    }

    pub fn frontier_mut(&mut self) -> &mut EvictionFrontier {
        &mut self.frontier
    }

    pub fn objects(&self) -> &dyn ColdObjectStore {
        self.objects.as_ref()
    }

    pub fn index(&self) -> &dyn ColdIndex {
        self.index.as_ref()
    }
}

/// Observable accounting for one `evict_decayed` pass. `frontier_ops` vs
/// `candidates_examined` is the proof that eviction is O(k log n), not an O(n)
/// walk of all memory nodes (the chunk-visit-counter analogue); `scanned_nodes`
/// is the count of nodes materialized via the full memory-node scan during
/// eviction and is 0 by construction -- the path reads the frontier and loads
/// each candidate by id.
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EvictionReport {
    pub tenant_id: String,
    pub scope: String,
    pub evicted: usize,
    pub evicted_nodes: Vec<String>,
    pub frontier_ops: usize,
    pub scanned_nodes: usize,
    pub candidates_examined: usize,
}

/// One-time bootstrap of the eviction frontier for a tenant scope. In steady
/// state the frontier is maintained incrementally by `recall_with_cold_tier`
/// (the zadd-on-access site, since recall already writes `last_accessed_ms`);
/// this O(n) pass seeds it once for a store that already holds memory nodes
/// written before the cold tier was attached. Subsequent evictions read the
/// persistent frontier in O(k log n).
pub fn seed_frontier<S: GraphStore>(
    store: &S,
    cold: &mut ColdTier,
    input: &DecayInput,
) -> GraphStoreResult<usize> {
    let tenant_id = normalized_tenant_pair(&input.tenant_id, &input.tenant_slug);
    let mut seeded = 0;
    for node in memory_nodes(store, &tenant_id, false)? {
        if prop_bool(&node.properties, "archive_tier") {
            continue;
        }
        let last_accessed = prop_i64(&node.properties, "last_accessed_ms")
            .or_else(|| prop_i64(&node.properties, "updated_at_ms"))
            .unwrap_or(0);
        cold.frontier.touch(&tenant_id, &node.id, last_accessed)?;
        seeded += 1;
    }
    Ok(seeded)
}

/// The eviction spine. For each memory node in the tenant's cold tail (coldest
/// `last_accessed_ms` at or below the staleness cutoff) whose activation is at
/// or below the threshold: flag the durable copy archived, commit its content
/// object to the cold store, record the cold-index address, then evict it from
/// the operating store and drop it from the frontier.
///
/// Candidates come from the frontier (`coldest_below`, O(k log n)), never the
/// O(n) memory-node scan. Eviction is version-neutral (`evict_node` never bumps
/// the graph version), so the cached structural priors of the live nodes stay
/// valid. This is the cut-6 superset of `decay`: same staleness/activation gate,
/// plus durable retention and real RAM reclamation.
pub fn evict_decayed<S: GraphStore>(
    store: &mut S,
    cold: &mut ColdTier,
    input: DecayInput,
) -> GraphStoreResult<EvictionReport> {
    let tenant_id = normalized_tenant_pair(&input.tenant_id, &input.tenant_slug);
    let now = input.now_ms.unwrap_or_else(now_ms);
    let cutoff = now.saturating_sub(input.inactive_after_ms.max(1));
    cold.frontier.reset_ops();
    let candidates = cold.frontier.coldest_below(&tenant_id, cutoff, 0);
    let mut report = EvictionReport {
        tenant_id: tenant_id.clone(),
        scope: tenant_id.clone(),
        candidates_examined: candidates.len(),
        ..EvictionReport::default()
    };
    for (id, _score) in candidates {
        // Point lookup by id (O(log n)), never a scan. include_expired so an
        // already-flagged-resident node is still visible here.
        let Some(node) = store.get_node_including_expired(&id).cloned() else {
            cold.frontier.forget(&tenant_id, &id);
            continue;
        };
        if !node_matches_tenant(&node, &tenant_id) || !is_memory_node(&node) {
            cold.frontier.forget(&tenant_id, &id);
            continue;
        }
        let activation = prop_f64(&node.properties, "activation").unwrap_or(0.0);
        if activation > input.activation_threshold {
            // Cold by time but hot by activation: protected. Leave it resident
            // and in the frontier; a later, colder pass re-examines it cheaply.
            continue;
        }
        // Flag the cold copy archived. The flag lives ONLY in the durable copy;
        // we never upsert it into the operating store (which would bump the
        // version). Then commit -> record -> evict -> forget.
        let mut cold_copy = node.clone();
        set_prop_bool(&mut cold_copy.properties, "archive_tier", true);
        set_prop_str(&mut cold_copy.properties, "status", "archived");
        set_prop_i64(&mut cold_copy.properties, "archived_at_ms", now);
        cold_copy.content_hash = None;
        let object = node_to_content_object(&cold_copy);
        cold.objects.put_object(&object)?;
        cold.index
            .record(ColdIndexEntry::cold(&id, &tenant_id, &object.hash))?;
        store.evict_node(&id)?;
        cold.frontier.forget(&tenant_id, &id);
        report.evicted += 1;
        report.evicted_nodes.push(id);
    }
    report.frontier_ops = cold.frontier.ops();
    Ok(report)
}

/// Rehydrate a node from the cold tier back into the operating store. The
/// natural trigger is a recall that resolves to an id absent from the operating
/// store. Reads the cold index for the id's object hash (a KEYED lookup, never a
/// repository scan), fetches the object, reconstructs the node, clears the
/// archive flags (it is being accessed again), re-admits it version-neutrally,
/// and re-touches the frontier. A Warm-tier id rehydrates its whole parked
/// scope. Returns true when something was rehydrated.
pub fn rehydrate<S: GraphStore>(
    store: &mut S,
    cold: &mut ColdTier,
    id: &str,
) -> GraphStoreResult<bool> {
    let Some(entry) = cold.index.lookup(id)? else {
        return Ok(false);
    };
    if matches!(entry.tier, ColdTierKind::Warm) {
        return unpark_scope(store, cold, &entry.scope).map(|count| count > 0);
    }
    let Some(object) = cold.objects.get_object(&entry.object_hash)? else {
        return Err(GraphStoreError::new(
            "cold_object_missing",
            format!(
                "cold index points {id} at object {} but it is absent",
                entry.object_hash
            ),
        ));
    };
    let Some(mut node) = node_from_content_object(&object) else {
        return Err(GraphStoreError::new(
            "cold_object_not_node",
            format!("cold object for {id} is not a node payload"),
        ));
    };
    // It is being accessed again: clear the archive flags so recall sees it and
    // refresh recency. Clearing content_hash forces a recompute on readmit.
    remove_prop(&mut node.properties, "archive_tier");
    set_prop_str(&mut node.properties, "status", "active");
    set_prop_i64(&mut node.properties, "last_accessed_ms", now_ms());
    node.content_hash = None;
    store.readmit_node(node)?;
    cold.index.remove(id)?;
    cold.frontier.touch(&entry.scope, id, now_ms())?;
    Ok(true)
}

/// Recall with cold-tier integration. Before ranking, any requested seed id that
/// is absent from the operating store is rehydrated from the cold tier (recall
/// is the natural rehydration trigger). After ranking, every returned memory is
/// touched in the eviction frontier (recall is the zadd site, since it already
/// writes `last_accessed_ms`). Everything else is plain `recall`.
pub fn recall_with_cold_tier<S: GraphStore>(
    store: &mut S,
    cold: &mut ColdTier,
    input: MemoryRecallInput,
) -> GraphStoreResult<RankedMemories> {
    let tenant_id = normalized_tenant(&input);
    for seed in input.seeds.clone() {
        if store.get_node(&seed).is_none() {
            rehydrate(store, cold, &seed)?;
        }
    }
    let result = recall(store, input)?;
    let touched_at = now_ms();
    for memory in &result.memories {
        cold.frontier
            .touch(&tenant_id, &memory.graph_id, touched_at)?;
    }
    Ok(result)
}

/// Park a whole scope (e.g. a repo's code KG, or a tenant subgraph) to the warm
/// tier. Compiles the scope's subgraph to a `CompiledGraphPack`, persists the
/// pack to the cold object store, records the parked scope (and each member as a
/// Warm cold-index entry so a member recall unparks the whole scope), then
/// evicts every scope node and incident edge from the operating store. The
/// scope is absent from the operating store until first access. Boundary-
/// crossing edges are parked too, so the operating store is left with no
/// dangling endpoints. Version-neutral.
pub fn park_scope<S: GraphStore>(
    store: &mut S,
    cold: &mut ColdTier,
    scope: &str,
    node_ids: &[String],
) -> GraphStoreResult<ColdScopeEntry> {
    let mut nodes = Vec::new();
    for id in node_ids {
        if let Some(node) = store.get_node_including_expired(id).cloned() {
            nodes.push(node);
        }
    }
    let mut edge_ids: BTreeSet<String> = BTreeSet::new();
    for id in node_ids {
        for hit in store.neighbors(NeighborQuery::out(id).with_include_expired(true)) {
            edge_ids.insert(hit.edge_id);
        }
        for hit in store.neighbors(NeighborQuery::in_(id).with_include_expired(true)) {
            edge_ids.insert(hit.edge_id);
        }
    }
    let mut edges = Vec::new();
    for edge_id in &edge_ids {
        if let Some(edge) = store.get_edge(edge_id).cloned() {
            edges.push(edge);
        }
    }
    let snapshot = GraphSnapshot {
        version: store.stats().version,
        nodes: nodes.clone(),
        edges: edges.clone(),
    };
    let pack = compile_graph_pack(
        &snapshot,
        GraphCompileOptions {
            name: Some(format!("warm-scope-{scope}")),
            message: Some(format!("park warm scope {scope}")),
            ..GraphCompileOptions::default()
        },
    );
    cold.objects.put_pack(&pack)?;
    let entry = ColdScopeEntry {
        scope: scope.to_string(),
        commit_hash: pack.commit.commit_hash.clone(),
        node_ids: node_ids.to_vec(),
        edge_ids: edges.iter().map(|edge| edge.id.clone()).collect(),
        parked: true,
    };
    cold.index.record_scope(entry.clone())?;
    for node in &nodes {
        cold.index.record(ColdIndexEntry {
            id: node.id.clone(),
            scope: scope.to_string(),
            tier: ColdTierKind::Warm,
            object_hash: String::new(),
            commit_hash: Some(pack.commit.commit_hash.clone()),
        })?;
    }
    for edge in &edges {
        store.evict_edge(&edge.id)?;
    }
    for node in &nodes {
        store.evict_node(&node.id)?;
        cold.frontier.forget(scope, &node.id);
    }
    Ok(entry)
}

/// Unpark a warm scope: load its parked pack and reconstruct the subgraph via
/// `checkout_graph_version` (the same reader the versioned-graph checkout round-
/// trip test asserts on), re-admit every node then every edge version-neutrally,
/// re-touch the frontier, and clear the scope's cold records. Returns the number
/// of nodes rehydrated.
pub fn unpark_scope<S: GraphStore>(
    store: &mut S,
    cold: &mut ColdTier,
    scope: &str,
) -> GraphStoreResult<usize> {
    let Some(entry) = cold.index.scope(scope)? else {
        return Ok(0);
    };
    if !entry.parked {
        return Ok(0);
    }
    let Some(pack) = cold.objects.get_pack(&entry.commit_hash)? else {
        return Err(GraphStoreError::new(
            "cold_pack_missing",
            format!(
                "warm scope {scope} points at commit {} but the pack is absent",
                entry.commit_hash
            ),
        ));
    };
    // Reuse the versioned-graph checkout reader: assemble a one-pack repository
    // and check out the parked commit to reconstruct the subgraph snapshot.
    let update = update_graph_ref(GraphVersionRepository::default(), pack, None, None);
    let checkout =
        checkout_graph_version(&update.repository, &entry.commit_hash).ok_or_else(|| {
            GraphStoreError::new(
                "cold_pack_checkout_failed",
                format!("could not check out parked commit {}", entry.commit_hash),
            )
        })?;
    let touched_at = now_ms();
    for node in &checkout.snapshot.nodes {
        store.readmit_node(node.clone())?;
        cold.frontier.touch(scope, &node.id, touched_at)?;
    }
    for edge in &checkout.snapshot.edges {
        store.readmit_edge(edge.clone())?;
    }
    for node_id in &entry.node_ids {
        cold.index.remove(node_id)?;
    }
    cold.index.remove_scope(scope)?;
    Ok(checkout.snapshot.nodes.len())
}

fn is_memory_node(node: &NodeRecord) -> bool {
    node.labels.iter().any(|label| {
        label == MEMORY_DOCUMENT_LABEL
            || label == MEMORY_NODE_LABEL
            || label == MEMORY_SUMMARY_LABEL
    })
}

fn remove_prop(properties: &mut Value, key: &str) {
    if let Some(object) = properties.as_object_mut() {
        object.remove(key);
    }
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

pub fn invalidate_on_contradiction<S: GraphStore>(
    store: &mut S,
    new_edge: &EdgeRecord,
    policy: &ContradictionPolicy,
) -> GraphStoreResult<Vec<Contradiction>> {
    let invalidated_at_ms = edge_time_interval(new_edge)
        .and_then(|interval| interval.start_ms)
        .or_else(|| prop_i64(&new_edge.properties, "valid_at_ms"))
        .unwrap_or_else(now_ms);
    let by_actor = prop_str(&new_edge.properties, "actor")
        .or_else(|| prop_str(&new_edge.properties, "actor_id"))
        .map(|actor| ActorId::from_label(&actor))
        .unwrap_or(ActorId::ZERO);
    let mut contradictions = Vec::new();
    for hit in store
        .neighbors(
            NeighborQuery::out(&new_edge.from_id)
                .with_edge_type(new_edge.edge_type.clone())
                .with_include_expired(true),
        )
        .into_iter()
    {
        if hit.edge_id == new_edge.id {
            continue;
        }
        let Some(mut existing) = store.get_edge(&hit.edge_id).cloned() else {
            continue;
        };
        if !edges_contradict(&existing, new_edge, policy) {
            continue;
        }
        set_prop_i64(&mut existing.properties, "t_end_ms", invalidated_at_ms);
        set_prop_i64(&mut existing.properties, "invalid_at_ms", invalidated_at_ms);
        set_prop_str(
            &mut existing.properties,
            "invalidated_by_edge_id",
            &new_edge.id,
        );
        store.upsert_edge(existing.clone())?;
        contradictions.push(Contradiction {
            existing_edge_id: existing.id,
            invalidated_at_ms,
            by_actor,
        });
    }
    Ok(contradictions)
}

pub fn recall_valid_time<S: GraphStore>(store: &S, q: RecallQuery) -> Vec<(String, f64)> {
    let at_ms = q.at_ms.unwrap_or_else(now_ms);
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    let mut frontier = q.seeds.clone();
    let mut seen = BTreeSet::new();
    while let Some(node_id) = frontier.pop() {
        if !seen.insert(node_id.clone()) {
            continue;
        }
        let mut neighbors = Vec::new();
        for hit in store.neighbors(NeighborQuery::out(&node_id).with_include_expired(true)) {
            let Some(edge) = store.get_edge(&hit.edge_id) else {
                continue;
            };
            if !edge_valid_at(edge, at_ms) {
                continue;
            }
            if store.get_node(&hit.node_id).is_none() {
                continue;
            }
            neighbors.push((hit.node_id.clone(), hit.confidence.unwrap_or(1.0)));
            if !seen.contains(&hit.node_id) {
                frontier.push(hit.node_id);
            }
        }
        adjacency.insert(node_id, neighbors);
        if seen.len() > 10_000 {
            break;
        }
    }
    let seeds = q
        .seeds
        .iter()
        .map(|seed| (seed.clone(), 1.0))
        .collect::<HashMap<_, _>>();
    let mut scored = personalized_pagerank(&adjacency, &seeds, q.alpha, 1e-5, 20_000)
        .into_iter()
        .collect::<Vec<_>>();
    scored.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    scored.truncate(q.top_k.max(1));
    scored
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
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
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
            let weight = weights.get(&hit.edge_type).copied().unwrap_or_else(|| {
                if hit.edge_type == MEMORY_IN_PROJECT {
                    0.85
                } else {
                    1.0
                }
            }) * hit.confidence.unwrap_or(1.0).clamp(0.0, 1.0);
            if weight > 0.0 {
                if hit.edge_type == MEMORY_IN_PROJECT {
                    adjacency
                        .entry(hit.node_id.clone())
                        .or_default()
                        .push((id.clone(), weight));
                    neighbors.push((hit.node_id, weight * 0.5));
                } else {
                    neighbors.push((hit.node_id, weight));
                }
            }
        }
        adjacency.entry(id.clone()).or_default().extend(neighbors);
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
    if seeds.is_empty() {
        for node in nodes {
            let score = lexical_score(&input.query, node);
            if score > 0.0 {
                seeds.insert(node.id.clone(), score);
            }
        }
    }
    if seeds.is_empty() {
        for node in nodes.iter().take(8) {
            seeds.insert(node.id.clone(), 1.0);
        }
    }
    add_project_seed(&mut seeds, id_set, input);
    seeds
}

fn recall_ppr(
    adjacency: &HashMap<String, Vec<(String, f64)>>,
    seeds: HashMap<String, f64>,
    project_anchor: Option<&str>,
    tenant_id: &str,
    graph_version: u64,
) -> HashMap<String, f64> {
    if seeds.is_empty() {
        return HashMap::new();
    }
    let mut live_seeds = seeds;
    let mut ppr = HashMap::new();
    if let Some(anchor) = project_anchor {
        if let Some(weight) = live_seeds.remove(anchor) {
            let scope = format!("memory-project-anchor:{tenant_id}");
            ppr = cached_single_seed_personalized_pagerank(
                &scope,
                graph_version,
                adjacency,
                anchor,
                weight,
                0.15,
                1e-5,
                20_000,
            );
        }
    }
    if !live_seeds.is_empty() {
        merge_ppr_scores(
            &mut ppr,
            personalized_pagerank(adjacency, &live_seeds, 0.15, 1e-5, 20_000),
        );
    }
    ppr
}

fn ranked_memory(
    node: &NodeRecord,
    query: &str,
    ppr_score: f64,
    project_bonus: f64,
) -> RankedMemory {
    let activation = prop_f64(&node.properties, "activation").unwrap_or(0.0);
    let fitness = fitness_score(&node.properties);
    let recency = prop_i64(&node.properties, "updated_at_ms")
        .or_else(|| prop_i64(&node.properties, "created_at_ms"))
        .map(|value| (value.max(0) as f64 + 1.0).log10() / 16.0)
        .unwrap_or(0.0);
    let lexical = lexical_score(query, node);
    let score =
        ppr_score * 2.0 + lexical + activation * 0.05 + fitness * 0.2 + recency + project_bonus;
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
    if let Some(project_slug) =
        prop_str(&node.properties, "project_slug").filter(|value| !value.trim().is_empty())
    {
        provenance.insert("project_slug".to_string(), Value::String(project_slug));
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
    edge_time_interval(edge)
        .map(|interval| interval.contains_ms(as_of_ms))
        .unwrap_or_else(|| node_valid_at(&edge.properties, as_of_ms))
}

fn edge_interval(edge: &EdgeRecord) -> TimeInterval {
    edge_time_interval(edge).unwrap_or(TimeInterval {
        start_ms: prop_i64(&edge.properties, "valid_at_ms"),
        end_ms: prop_i64(&edge.properties, "invalid_at_ms"),
    })
}

fn edges_contradict(
    existing: &EdgeRecord,
    incoming: &EdgeRecord,
    policy: &ContradictionPolicy,
) -> bool {
    if existing.from_id != incoming.from_id {
        return false;
    }
    if existing.edge_type == incoming.edge_type {
        if policy.is_functional(&incoming.edge_type) && existing.to_id != incoming.to_id {
            return intervals_overlap(existing, incoming);
        }
        return false;
    }
    if policy.is_mutually_exclusive(&existing.edge_type, &incoming.edge_type) {
        return intervals_overlap(existing, incoming);
    }
    false
}

fn intervals_overlap(left: &EdgeRecord, right: &EdgeRecord) -> bool {
    edge_interval(left).overlaps(edge_interval(right))
}

fn consolidation_key(node: &NodeRecord) -> String {
    let project = prop_str(&node.properties, "project_slug")
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "tenant".to_string());
    prop_str(&node.properties, "source_ref")
        .filter(|value| !value.trim().is_empty())
        .map(|value| format!("project:{project}:source:{value}"))
        .unwrap_or_else(|| {
            let content = prop_str(&node.properties, "content")
                .or_else(|| prop_str(&node.properties, "summary"))
                .unwrap_or_else(|| node.id.clone());
            format!("project:{project}:content:{}", stable_hash(content))
        })
}

/// Stable id for a project's anchor hub node.
///
/// This MUST resolve to the same id the write path (`theorem-harness-runtime::memory`)
/// produces, because the membership edge `member --MEMORY_IN_PROJECT--> anchor` is
/// written there and read here: recall only sees the edge when this id matches the
/// edge's `to_id`. The write path preserves tenant casing because the deployed graph
/// partitions are case-sensitive, while slugifying the project segment. We therefore
/// mirror the write path's normalization exactly. A cross-crate parity test
/// (`theorem-harness-runtime/tests/project_anchor_parity.rs`) fails if the two ever drift.
pub fn project_anchor_node_id(tenant_id: &str, project_slug: &str) -> String {
    let tenant = {
        let normalized = tenant_id.trim();
        if normalized.is_empty() {
            "default".to_string()
        } else {
            normalized.to_string()
        }
    };
    let slug = {
        let slugged = project_anchor_slugify(project_slug);
        if slugged.is_empty() {
            "unknown".to_string()
        } else {
            slugged
        }
    };
    format!("mem:project:{tenant}:{slug}")
}

/// Mirror of `theorem-harness-runtime::memory::slugify`, kept byte-identical so the
/// recall-side anchor id matches the write-side anchor id. Guarded by the cross-crate
/// parity test.
fn project_anchor_slugify(value: &str) -> String {
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

pub fn project_membership_edge_id(tenant_id: &str, memory_id: &str, project_slug: &str) -> String {
    memory_edge_id(
        tenant_id,
        MEMORY_IN_PROJECT,
        memory_id,
        &project_anchor_node_id(tenant_id, project_slug),
    )
}

fn project_seed_node(tenant_id: &str, input: &MemoryRecallInput) -> Option<String> {
    let project_slug = input.project_slug.trim();
    if project_slug.is_empty() || project_permeability(input) <= 0.0 {
        None
    } else {
        Some(project_anchor_node_id(tenant_id, project_slug))
    }
}

fn add_project_seed(
    seeds: &mut HashMap<String, f64>,
    id_set: &BTreeSet<String>,
    input: &MemoryRecallInput,
) {
    let Some(anchor) = project_seed_node(&normalized_tenant(input), input) else {
        return;
    };
    if !id_set.contains(&anchor) {
        return;
    }
    let weight = project_permeability(input);
    seeds
        .entry(anchor)
        .and_modify(|existing| *existing = existing.max(weight))
        .or_insert(weight);
}

fn project_permeability(input: &MemoryRecallInput) -> f64 {
    input.project_permeability.clamp(0.0, 1.0) * 4.0
}

fn project_rank_bonus(node: &NodeRecord, input: &MemoryRecallInput) -> f64 {
    let project_slug = input.project_slug.trim();
    if project_slug.is_empty() {
        return 0.0;
    }
    if prop_str(&node.properties, "project_slug").as_deref() == Some(project_slug) {
        project_permeability(input) * 10.0
    } else {
        0.0
    }
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
        .map(|value| {
            tenant_aliases(tenant_id)
                .iter()
                .any(|alias| alias == &value)
        })
        .unwrap_or(false)
}

fn tenant_aliases(tenant_id: &str) -> Vec<String> {
    let canonical = if tenant_id.trim().is_empty() {
        "default".to_string()
    } else {
        tenant_id.trim().to_string()
    };
    let legacy_lowercase = canonical.to_lowercase();
    if legacy_lowercase == canonical {
        vec![canonical]
    } else {
        vec![canonical, legacy_lowercase]
    }
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

fn default_ppr_alpha() -> f64 {
    0.15
}

fn default_budget_tokens() -> i64 {
    2_000
}

fn default_project_permeability() -> f64 {
    0.75
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
    use rustyred_thg_core::{
        structural_epistemic_pass, InMemoryGraphStore, StructuralEpistemicInput,
    };

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

    fn memory_doc_in_project(id: &str, title: &str, content: &str, project: &str) -> NodeRecord {
        let mut node = memory_doc(id, title, content);
        set_prop_str(&mut node.properties, "project_slug", project);
        node
    }

    fn project_anchor(tenant: &str, project: &str) -> NodeRecord {
        NodeRecord::new(
            project_anchor_node_id(tenant, project),
            [HARNESS_MEMORY_LABEL, MEMORY_PROJECT_LABEL],
            json!({
                "tenant_id": tenant,
                "tenant_slug": tenant,
                "project_slug": project,
                "source": MEMORY_PLUGIN_SOURCE,
            }),
        )
    }

    #[test]
    fn recall_include_epistemic_attaches_shadow_without_changing_default() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(memory_doc("m1", "Cache claim", "cache is enabled"))
            .unwrap();
        structural_epistemic_pass(
            &mut store,
            StructuralEpistemicInput {
                batch_node_ids: vec!["m1".to_string()],
                ..StructuralEpistemicInput::default()
            },
        )
        .unwrap();

        let plain = recall(
            &mut store,
            MemoryRecallInput {
                tenant_id: "theorem".to_string(),
                query: "cache".to_string(),
                bump_activation: false,
                ..MemoryRecallInput::default()
            },
        )
        .unwrap();
        assert_eq!(plain.memories.len(), 1);
        assert!(!plain.memories[0]
            .provenance
            .contains_key("epistemic_shadow"));

        let with_epistemic = recall(
            &mut store,
            MemoryRecallInput {
                tenant_id: "theorem".to_string(),
                query: "cache".to_string(),
                include_epistemic: true,
                bump_activation: false,
                ..MemoryRecallInput::default()
            },
        )
        .unwrap();
        assert_eq!(with_epistemic.memories.len(), 1);
        assert_eq!(
            with_epistemic.memories[0].provenance["epistemic_shadow"]["source_kind"].as_str(),
            Some("structural")
        );
    }

    fn link_project(store: &mut InMemoryGraphStore, tenant: &str, memory: &str, project: &str) {
        store
            .upsert_edge(EdgeRecord::new(
                project_membership_edge_id(tenant, memory, project),
                memory,
                MEMORY_IN_PROJECT,
                project_anchor_node_id(tenant, project),
                json!({ "tenant_id": tenant, "project_slug": project }),
            ))
            .unwrap();
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
    fn project_scope_biases_recall_without_filtering_sibling_context() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(project_anchor("theorem", "alpha"))
            .unwrap();
        store
            .upsert_node(project_anchor("theorem", "beta"))
            .unwrap();
        store
            .upsert_node(memory_doc_in_project(
                "mem:alpha",
                "Shared planning",
                "shared graph recall",
                "alpha",
            ))
            .unwrap();
        store
            .upsert_node(memory_doc_in_project(
                "mem:alpha-2",
                "Shared build",
                "shared graph implementation",
                "alpha",
            ))
            .unwrap();
        store
            .upsert_node(memory_doc_in_project(
                "mem:beta",
                "Shared sibling",
                "shared graph context from another project",
                "beta",
            ))
            .unwrap();
        link_project(&mut store, "theorem", "mem:alpha", "alpha");
        link_project(&mut store, "theorem", "mem:alpha-2", "alpha");
        link_project(&mut store, "theorem", "mem:beta", "beta");
        store
            .upsert_edge(EdgeRecord::new(
                "edge:alpha-beta",
                "mem:alpha",
                SUPPORTS,
                "mem:beta",
                json!({ "tenant_id": "theorem" }),
            ))
            .unwrap();

        let result = recall(
            &mut store,
            MemoryRecallInput {
                tenant_id: "theorem".to_string(),
                query: "shared graph".to_string(),
                project_slug: "alpha".to_string(),
                project_permeability: 1.0,
                top_k: 3,
                budget_tokens: 200,
                bump_activation: false,
                ..MemoryRecallInput::default()
            },
        )
        .unwrap();

        assert_eq!(result.memories[0].provenance["project_slug"], "alpha");
        assert!(
            result
                .memories
                .iter()
                .any(|item| item.graph_id == "mem:beta"),
            "sibling project memory connected through the tenant graph still surfaces"
        );
    }

    #[test]
    fn project_scope_does_not_cross_the_tenant_wall() {
        let mut store = InMemoryGraphStore::new();
        store
            .upsert_node(project_anchor("theorem", "alpha"))
            .unwrap();
        store.upsert_node(project_anchor("other", "alpha")).unwrap();
        store
            .upsert_node(memory_doc_in_project(
                "mem:alpha",
                "Tenant memory",
                "tenant wall",
                "alpha",
            ))
            .unwrap();
        let mut other = memory_doc_in_project("mem:other", "Other tenant", "tenant wall", "alpha");
        set_prop_str(&mut other.properties, "tenant_id", "other");
        set_prop_str(&mut other.properties, "tenant_slug", "other");
        store.upsert_node(other).unwrap();
        link_project(&mut store, "theorem", "mem:alpha", "alpha");
        link_project(&mut store, "other", "mem:other", "alpha");

        let result = recall(
            &mut store,
            MemoryRecallInput {
                tenant_id: "theorem".to_string(),
                query: "tenant wall".to_string(),
                project_slug: "alpha".to_string(),
                project_permeability: 1.0,
                top_k: 10,
                budget_tokens: 200,
                bump_activation: false,
                ..MemoryRecallInput::default()
            },
        )
        .unwrap();

        assert!(result
            .memories
            .iter()
            .all(|item| item.graph_id != "mem:other"));
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

    #[test]
    fn contradiction_invalidates_old_functional_edge_without_deleting_it() {
        let mut store = InMemoryGraphStore::new();
        for id in ["person:1", "city:old", "city:new"] {
            store
                .upsert_node(NodeRecord::new(id, ["Entity"], json!({})))
                .unwrap();
        }
        store
            .upsert_edge(EdgeRecord::new(
                "edge:old",
                "person:1",
                "LIVES_IN",
                "city:old",
                json!({ "t_start_ms": 10 }),
            ))
            .unwrap();
        let new_edge = EdgeRecord::new(
            "edge:new",
            "person:1",
            "LIVES_IN",
            "city:new",
            json!({ "t_start_ms": 20, "actor": "codex" }),
        );

        let contradictions = invalidate_on_contradiction(
            &mut store,
            &new_edge,
            &ContradictionPolicy::functional(["LIVES_IN"]),
        )
        .unwrap();
        store.upsert_edge(new_edge).unwrap();

        assert_eq!(contradictions.len(), 1);
        assert_eq!(contradictions[0].existing_edge_id, "edge:old");
        assert!(store.get_edge("edge:old").is_some());
        assert!(store.get_edge("edge:new").is_some());
        assert_eq!(
            store.get_edge("edge:old").unwrap().properties["t_end_ms"].as_i64(),
            Some(20)
        );
    }

    #[test]
    fn recall_valid_time_excludes_edges_after_invalidity() {
        let mut store = InMemoryGraphStore::new();
        for id in ["mem:a", "mem:b", "mem:c"] {
            store.upsert_node(memory_doc(id, id, "fixture")).unwrap();
        }
        store
            .upsert_edge(EdgeRecord::new(
                "edge:ab",
                "mem:a",
                SUPPORTS,
                "mem:b",
                json!({ "t_start_ms": 10, "t_end_ms": 20 }),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "edge:ac",
                "mem:a",
                SUPPORTS,
                "mem:c",
                json!({ "t_start_ms": 10 }),
            ))
            .unwrap();

        let before = recall_valid_time(
            &store,
            RecallQuery {
                seeds: vec!["mem:a".to_string()],
                at_ms: Some(19),
                alpha: 0.15,
                top_k: 3,
            },
        );
        let after = recall_valid_time(
            &store,
            RecallQuery {
                seeds: vec!["mem:a".to_string()],
                at_ms: Some(21),
                alpha: 0.15,
                top_k: 3,
            },
        );

        assert!(before.iter().any(|(id, _)| id == "mem:b"));
        assert!(!after.iter().any(|(id, _)| id == "mem:b"));
        assert!(store.get_edge("edge:ab").is_some());
    }

    #[test]
    fn recall_valid_time_concentrates_activation_near_seed() {
        let mut store = InMemoryGraphStore::new();
        for id in ["mem:a", "mem:b", "mem:c", "mem:d"] {
            store.upsert_node(memory_doc(id, id, "fixture")).unwrap();
        }
        for (edge_id, from, to) in [
            ("edge:ab", "mem:a", "mem:b"),
            ("edge:bc", "mem:b", "mem:c"),
            ("edge:cd", "mem:c", "mem:d"),
        ] {
            store
                .upsert_edge(EdgeRecord::new(
                    edge_id,
                    from,
                    SUPPORTS,
                    to,
                    json!({ "t_start_ms": 1 }),
                ))
                .unwrap();
        }

        let ranked = recall_valid_time(
            &store,
            RecallQuery {
                seeds: vec!["mem:a".to_string()],
                at_ms: Some(2),
                alpha: 0.15,
                top_k: 4,
            },
        );
        let pos_b = ranked.iter().position(|(id, _)| id == "mem:b").unwrap();
        let pos_d = ranked.iter().position(|(id, _)| id == "mem:d").unwrap();

        assert!(pos_b < pos_d);
    }

    #[test]
    fn concurrent_contradiction_converges_by_hlc() {
        // SPEC Part 4 A4.4: two replicas concurrently invalidate the same
        // functional edge. Shipped as Hlc-stamped facts through the CRDT join,
        // both replicas converge to one deterministic validity (the Hlc-max
        // invalidation), and the edge stays present - never deleted.
        //
        // FINDING (punch-list): production invalidate_on_contradiction writes
        // invalid_at via plain upsert_edge WITHOUT an _crdt_hlc stamp, so
        // diff_since skips it (merge.rs only ships records with record_max_hlc)
        // and it will not propagate over the sync transport as-is. This test
        // ships the invalidations as the Hlc-stamped facts the spec requires
        // (Part 4 #2: "stamped with Hlc"); invalidate_on_contradiction should
        // stamp likewise so the property holds end-to-end.
        use rustyred_thg_core::{join_delta, GraphMutation, Hlc, StampedBatch, StampedMutation};

        let invalidation = |end_ms: i64| {
            EdgeRecord::new(
                "e:1",
                "person:1",
                "LIVES_IN",
                "city:a",
                json!({ "t_start_ms": 10, "t_end_ms": end_ms, "invalid_at_ms": end_ms }),
            )
        };
        let edge_delta = |edge: EdgeRecord, hlc: Hlc| {
            StampedBatch::new([StampedMutation::new(GraphMutation::EdgeUpsert(edge), hlc)])
        };
        let base = EdgeRecord::new(
            "e:1",
            "person:1",
            "LIVES_IN",
            "city:a",
            json!({ "t_start_ms": 10 }),
        );
        let base_hlc = Hlc::new(10, 0, ActorId::from_label("codex"));
        let hlc_l = Hlc::new(50, 0, ActorId::from_label("codex"));
        let hlc_r = Hlc::new(60, 0, ActorId::from_label("claude"));

        let seed = || {
            let mut store = InMemoryGraphStore::new();
            for id in ["person:1", "city:a"] {
                store
                    .upsert_node(NodeRecord::new(id, ["Entity"], json!({})))
                    .unwrap();
            }
            join_delta(&mut store, edge_delta(base.clone(), base_hlc));
            store
        };

        // Replica L sees its own invalidation, then R's; replica R the reverse.
        let mut left = seed();
        join_delta(&mut left, edge_delta(invalidation(50), hlc_l));
        join_delta(&mut left, edge_delta(invalidation(60), hlc_r));

        let mut right = seed();
        join_delta(&mut right, edge_delta(invalidation(60), hlc_r));
        join_delta(&mut right, edge_delta(invalidation(50), hlc_l));

        let edge_l = left.get_edge("e:1").expect("edge present on L");
        let edge_r = right.get_edge("e:1").expect("edge present on R");

        // Deterministic convergence regardless of receive order: the Hlc-max
        // invalidation (60 @ claude) wins the validity LWW on both replicas.
        assert_eq!(
            edge_l.properties["invalid_at_ms"],
            edge_r.properties["invalid_at_ms"]
        );
        assert_eq!(edge_l.properties["t_end_ms"], edge_r.properties["t_end_ms"]);
        assert_eq!(edge_l.properties["invalid_at_ms"].as_i64(), Some(60));
        // Contradiction invalidates, never deletes.
        assert!(!edge_l.tombstone);
        assert!(!edge_r.tombstone);
    }
}
