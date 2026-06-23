use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use rustyred_thg_core::{
    label_propagation_communities, EdgeRecord, GraphStore, HookContext, HookError, HookHandler,
    HookOutcome, HookRegistration, MutationEvent, MutationKind, MutationMatcher, NeighborQuery,
    NodeQuery, NodeRecord, PluginCapability, PluginCapabilityKind, RustyRedPlugin,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::embedding::{write_vector_payload, HippoTextEmbedder, VectorPayload};
use crate::indexing::edge_id;
use crate::schema::{
    hash_vector, stable_digest, HippoEdge, HippoResult, CENTRALITY_PROPERTY, HUB_SCORE_PROPERTY,
    LABEL_HUB, LABEL_PAGE, LABEL_PHRASE, SEMANTIC_VECTOR_PROPERTY,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RaptorPolicy {
    pub region_node_threshold: usize,
    pub min_members: usize,
    pub max_level: u32,
}

impl Default for RaptorPolicy {
    fn default() -> Self {
        Self {
            region_node_threshold: 200,
            min_members: 4,
            max_level: 3,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct HubBuildStats {
    pub region_nodes_seen: usize,
    pub community_calls: usize,
    pub hubs_upserted: usize,
    pub summarize_edges: usize,
    pub hub_parent_edges: usize,
    pub hook_counter: u64,
}

pub fn summary_tree_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(summary_tree_handler);
    HookRegistration::new(
        "hipporag.summary_tree",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted, MutationKind::EdgeUpserted])
            .with_labels([LABEL_PAGE, LABEL_PHRASE, HippoEdge::Relates.as_str()]),
        coalesce_region,
        handler,
    )
}

#[derive(Clone, Debug, Default)]
pub struct SummaryTreeHooksPlugin;

impl RustyRedPlugin for SummaryTreeHooksPlugin {
    fn name(&self) -> &'static str {
        "rustyred.hipporag.hooks"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![PluginCapability {
            kind: PluginCapabilityKind::Hook,
            name: "hipporag.summary_tree".to_string(),
        }]
    }

    fn hooks(&self) -> Vec<HookRegistration> {
        vec![summary_tree_hook()]
    }
}

fn coalesce_region(event: &MutationEvent) -> Option<String> {
    Some(format!("hipporag-region:{}", event.tenant))
}

fn summary_tree_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    let seeds = events
        .iter()
        .map(|event| event.id.clone())
        .collect::<Vec<_>>();
    let stats = build_summary_tree_for_region(ctx.store, RaptorPolicy::default(), &seeds)
        .map_err(|error| HookError::new(error.to_string()))?;
    if stats.hubs_upserted == 0 {
        Ok(HookOutcome::Done)
    } else {
        Ok(HookOutcome::Wrote {
            mutations: stats.hubs_upserted + stats.summarize_edges + stats.hub_parent_edges,
        })
    }
}

pub fn build_summary_tree_for_region<S: GraphStore>(
    store: &mut S,
    policy: RaptorPolicy,
    dirty_seed_ids: &[String],
) -> HippoResult<HubBuildStats> {
    let plan = plan_summary_tree(store, policy, dirty_seed_ids);
    let vectors = plan
        .hubs
        .iter()
        .map(|hub| VectorPayload::hash(hash_vector(&hub.summary, 2560)))
        .collect::<Vec<_>>();
    write_summary_tree_plan(store, plan, vectors, SEMANTIC_VECTOR_PROPERTY)
}

pub async fn build_summary_tree_with_embedder<S: GraphStore, E: HippoTextEmbedder>(
    store: &mut S,
    policy: RaptorPolicy,
    dirty_seed_ids: &[String],
    embedder: &E,
) -> HippoResult<HubBuildStats> {
    let plan = plan_summary_tree(store, policy, dirty_seed_ids);
    if plan.hubs.is_empty() {
        return write_summary_tree_plan(store, plan, Vec::new(), embedder.property());
    }

    let inputs = plan
        .hubs
        .iter()
        .map(|hub| hub.summary.clone())
        .collect::<Vec<_>>();
    let vectors = embedder.embed(&inputs).await?;
    if vectors.len() != inputs.len() {
        return Err(crate::schema::HippoError::new(
            "embedding_response",
            format!(
                "embedder {} returned {} vectors for {} HippoRAG hub summaries",
                embedder.model_id(),
                vectors.len(),
                inputs.len()
            ),
        ));
    }
    let vectors = vectors
        .into_iter()
        .map(|vector| VectorPayload::embedded(embedder, vector))
        .collect::<HippoResult<Vec<_>>>()?;
    write_summary_tree_plan(store, plan, vectors, embedder.property())
}

#[derive(Clone, Debug)]
struct SummaryTreePlan {
    stats: HubBuildStats,
    hubs: Vec<HubPlan>,
    below_threshold: bool,
}

#[derive(Clone, Debug)]
struct HubPlan {
    hub_id: String,
    summary: String,
    level: u32,
    centrality: f32,
    members: Vec<String>,
    parent_hubs: Vec<String>,
}

fn plan_summary_tree<S: GraphStore>(
    store: &S,
    policy: RaptorPolicy,
    dirty_seed_ids: &[String],
) -> SummaryTreePlan {
    let region = dirty_region(store, dirty_seed_ids);
    let mut stats = HubBuildStats {
        region_nodes_seen: region.len(),
        ..HubBuildStats::default()
    };
    if region.len() < policy.region_node_threshold.max(1) {
        return SummaryTreePlan {
            stats,
            hubs: Vec::new(),
            below_threshold: true,
        };
    }

    let mut current_members = region.into_iter().collect::<Vec<_>>();
    current_members.sort();
    let mut child_hubs = Vec::new();
    let mut hubs = Vec::new();
    for level in 0..=policy.max_level {
        let mut communities = detect_communities(store, &current_members);
        if !communities
            .iter()
            .any(|members| members.len() >= policy.min_members.max(1))
        {
            communities = vec![current_members.clone()];
        }
        stats.community_calls += 1;
        let mut next_hubs = Vec::new();
        for members in communities {
            if members.len() < policy.min_members.max(1) {
                continue;
            }
            let hub_id = hub_id(level, &members);
            let summary = cheap_summary(store, &members);
            let centrality = members.len() as f32 / stats.region_nodes_seen.max(1) as f32;
            stats.hubs_upserted += 1;
            next_hubs.push(hub_id.clone());
            let parent_hubs = child_hubs
                .iter()
                .filter(|parent_candidate| members.contains(*parent_candidate))
                .cloned()
                .collect::<Vec<_>>();
            stats.summarize_edges += members.len();
            stats.hub_parent_edges += parent_hubs.len();
            hubs.push(HubPlan {
                hub_id,
                summary,
                level,
                centrality,
                members,
                parent_hubs,
            });
        }
        if next_hubs.len() < policy.min_members.max(2) || level == policy.max_level {
            break;
        }
        current_members = next_hubs.clone();
        child_hubs = next_hubs;
    }

    SummaryTreePlan {
        stats,
        hubs,
        below_threshold: false,
    }
}

fn write_summary_tree_plan<S: GraphStore>(
    store: &mut S,
    mut plan: SummaryTreePlan,
    vectors: Vec<VectorPayload>,
    vector_property: &str,
) -> HippoResult<HubBuildStats> {
    if plan.below_threshold {
        return Ok(plan.stats);
    }
    if vectors.len() != plan.hubs.len() {
        return Err(crate::schema::HippoError::new(
            "embedding_response",
            format!(
                "received {} hub vectors for {} planned hubs",
                vectors.len(),
                plan.hubs.len()
            ),
        ));
    }
    for (hub, vector) in plan.hubs.iter().zip(vectors) {
        let mut node = NodeRecord::new(
            &hub.hub_id,
            [LABEL_HUB],
            json!({
                "summary": hub.summary.clone(),
                "level": hub.level,
                CENTRALITY_PROPERTY: hub.centrality,
                "member_count": hub.members.len(),
                HUB_SCORE_PROPERTY: hub.centrality,
            }),
        );
        write_vector_payload(&mut node.properties, vector_property, vector);
        store.upsert_node(node)?;
        for member in &hub.members {
            store.upsert_edge(EdgeRecord::new(
                edge_id(HippoEdge::Summarizes, &hub.hub_id, member),
                &hub.hub_id,
                HippoEdge::Summarizes.as_str(),
                member,
                json!({ "level": hub.level }),
            ))?;
        }
        for parent_hub in &hub.parent_hubs {
            store.upsert_edge(EdgeRecord::new(
                edge_id(HippoEdge::HubParent, &hub.hub_id, parent_hub),
                &hub.hub_id,
                HippoEdge::HubParent.as_str(),
                parent_hub,
                json!({ "level": hub.level }),
            ))?;
        }
    }
    plan.stats.hook_counter = increment_hook_counter(store)?;
    Ok(plan.stats)
}

fn dirty_region<S: GraphStore>(store: &S, dirty_seed_ids: &[String]) -> BTreeSet<String> {
    let seeds = dirty_seed_ids
        .iter()
        .filter(|id| store.get_node(id).is_some())
        .cloned()
        .collect::<Vec<_>>();
    if seeds.is_empty() {
        return all_hippo_nodes(store);
    }

    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in seeds {
        if seen.insert(seed.clone()) {
            queue.push_back(seed);
        }
    }
    while let Some(node_id) = queue.pop_front() {
        for neighbor in hippo_neighbors(store, &node_id) {
            if seen.insert(neighbor.clone()) {
                queue.push_back(neighbor);
            }
        }
    }
    seen
}

fn all_hippo_nodes<S: GraphStore>(store: &S) -> BTreeSet<String> {
    [LABEL_PAGE, LABEL_PHRASE, LABEL_HUB]
        .into_iter()
        .flat_map(|label| store.query_nodes(NodeQuery::label(label).with_limit(100_000)))
        .map(|node| node.id)
        .collect()
}

fn hippo_neighbors<S: GraphStore>(store: &S, node_id: &str) -> Vec<String> {
    let mut out = BTreeSet::new();
    for edge_type in [
        HippoEdge::Contains,
        HippoEdge::Relates,
        HippoEdge::Synonym,
        HippoEdge::Summarizes,
        HippoEdge::HubParent,
    ] {
        for hit in store.neighbors(NeighborQuery::out(node_id).with_edge_type(edge_type.as_str())) {
            out.insert(hit.node_id);
        }
        for hit in store.neighbors(NeighborQuery::in_(node_id).with_edge_type(edge_type.as_str())) {
            out.insert(hit.node_id);
        }
    }
    out.into_iter().collect()
}

fn detect_communities<S: GraphStore>(store: &S, members: &[String]) -> Vec<Vec<String>> {
    let member_set = members.iter().cloned().collect::<BTreeSet<_>>();
    let mut edges = Vec::new();
    for member in members {
        for edge_type in [
            HippoEdge::Contains,
            HippoEdge::Relates,
            HippoEdge::Synonym,
            HippoEdge::Summarizes,
            HippoEdge::HubParent,
        ] {
            for hit in
                store.neighbors(NeighborQuery::out(member).with_edge_type(edge_type.as_str()))
            {
                if member_set.contains(&hit.node_id) {
                    if let Some(edge) = store.get_edge(&hit.edge_id) {
                        edges.push(edge.clone());
                    }
                }
            }
        }
    }
    if edges.is_empty() {
        return vec![members.to_vec()];
    }
    let (labels, _) = label_propagation_communities(&edges);
    let mut by_label: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for member in members {
        by_label
            .entry(labels.get(member).copied().unwrap_or(u64::MAX))
            .or_default()
            .push(member.clone());
    }
    by_label.into_values().collect()
}

fn cheap_summary<S: GraphStore>(store: &S, members: &[String]) -> String {
    let mut snippets = members
        .iter()
        .filter_map(|id| store.get_node(id))
        .filter_map(node_text)
        .collect::<Vec<_>>();
    snippets.sort();
    snippets.truncate(6);
    format!(
        "Summary of {} members: {}",
        members.len(),
        snippets.join("; ")
    )
}

fn node_text(node: &NodeRecord) -> Option<String> {
    for key in ["text", "summary", "title", "url"] {
        if let Some(value) = node.properties.get(key).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

fn hub_id(level: u32, members: &[String]) -> String {
    let mut sorted = members.to_vec();
    sorted.sort();
    format!("hippo:hub:l{level}:{}", stable_digest(&sorted.join("\n")))
}

fn increment_hook_counter<S: GraphStore>(store: &mut S) -> HippoResult<u64> {
    let existing = store
        .get_node("hippo:summary_tree_hook_counter")
        .and_then(|node| node.properties.get("fires").and_then(Value::as_u64))
        .unwrap_or(0);
    let next = existing + 1;
    store.upsert_node(NodeRecord::new(
        "hippo:summary_tree_hook_counter",
        ["HippoHookCounter"],
        json!({ "name": "SummaryTreeHook", "fires": next }),
    ))?;
    Ok(next)
}
