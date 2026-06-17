//! Graph-level hooks for the code knowledge graph.
//!
//! These turn the previously-inert `Hook` capability the code plugin already
//! advertised (`code.graph.tenant_store_commit`) into real reactive compute:
//! when the code graph mutates, derived structure is warmed *inside* the store,
//! off the writer's path, so prompt-time reads find it ready.
//!
//! - [`IncrementalCentralityHook`]: warm PPR centrality on the changed
//!   neighborhood, so `context_pack` reads a pre-warmed `centrality` prior
//!   instead of running cold global PPR under the prompt budget.
//! - [`IncrementalEmbedHook`]: keep symbol embeddings fresh as signatures/docs
//!   change, with no batch backfill job. (see [`crate::code_embed_hook`]).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

use rustyred_thg_core::{
    personalized_pagerank, Direction, HookContext, HookError, HookHandler, HookOutcome,
    HookRegistration, MutationEvent, MutationKind, MutationMatcher, NeighborQuery,
    RedCoreGraphStore,
};
use serde_json::{json, Value};

use crate::{property_string, CALLS_SYMBOL, CODE_SYMBOL_LABEL, DECLARES_SYMBOL, DEPENDS_ON_SYMBOL};

/// Derived centrality property warmed onto `CodeSymbol` nodes.
pub const CENTRALITY_PROPERTY: &str = "centrality";

/// Localization bounds: a recompute touches at most this many symbols, reached
/// by a bounded BFS this deep from the changed seeds. A full-graph PPR on every
/// batch would reintroduce the cost the hook exists to remove.
const MAX_NEIGHBORHOOD: usize = 5_000;
const EXPANSION_DEPTH: usize = 2;
const PPR_ALPHA: f64 = 0.15;
const PPR_EPSILON: f64 = 1e-4;
const PPR_MAX_PUSHES: usize = 100_000;
/// Idempotency threshold: skip the write when centrality moved less than this,
/// so a self-induced re-trigger does no work and the hook chain terminates.
const CENTRALITY_EPSILON: f64 = 1e-6;

/// All code-KG hook registrations, collected by `CodeParsingPlugin::hooks()`.
pub fn code_kg_hooks() -> Vec<HookRegistration> {
    let mut hooks = vec![incremental_centrality_hook()];
    hooks.push(crate::code_embed_hook::incremental_embed_hook());
    hooks
}

/// Hook 1 from the spec: warm PPR/centrality on the changed neighborhood.
///
/// Fires on a `CodeSymbol` upsert or a `CALLS_SYMBOL`/`DEPENDS_ON_SYMBOL`/
/// `DECLARES_SYMBOL` edge change. Both shapes are admitted by one matcher
/// (`labels` matches a node label *or* an edge type), so a mixed ingest batch
/// coalesces into a single handler call.
pub fn incremental_centrality_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(centrality_handler);
    HookRegistration::new(
        "code.incremental_centrality",
        MutationMatcher::any()
            .with_kinds([
                MutationKind::NodeUpserted,
                MutationKind::EdgeUpserted,
                MutationKind::EdgeDeleted,
            ])
            .with_labels([
                CODE_SYMBOL_LABEL,
                CALLS_SYMBOL,
                DEPENDS_ON_SYMBOL,
                DECLARES_SYMBOL,
            ]),
        // Funnel every matched event into one group; the handler re-groups by
        // the authoritative `repo_id` node property. (Node ids are content
        // hashes, so repo_id is not recoverable from the event alone.)
        coalesce_code_kg,
        handler,
    )
}

fn coalesce_code_kg(_event: &MutationEvent) -> Option<String> {
    Some("code-kg-centrality".to_string())
}

/// True when an event's only changed property is the derived centrality value,
/// i.e. it is a self-induced re-trigger we must not treat as fresh structure.
fn is_self_induced(changed_props: &[String]) -> bool {
    !changed_props.is_empty() && changed_props.iter().all(|key| key == CENTRALITY_PROPERTY)
}

fn centrality_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    // 1. Resolve the changed symbol ids. Edge events carry the edge id; resolve
    //    its endpoints. Self-induced centrality-only events are skipped so the
    //    hook chain terminates after one productive generation.
    let mut affected: BTreeSet<String> = BTreeSet::new();
    for event in events {
        match event.kind {
            MutationKind::NodeUpserted => {
                if is_self_induced(&event.changed_props) {
                    continue;
                }
                affected.insert(event.id.clone());
            }
            MutationKind::EdgeUpserted | MutationKind::EdgeDeleted => {
                if let Ok(Some(edge)) = ctx.store.get_edge(&event.id) {
                    affected.insert(edge.from_id);
                    affected.insert(edge.to_id);
                }
            }
            MutationKind::NodeDeleted => {}
        }
    }
    if affected.is_empty() {
        return Ok(HookOutcome::Done);
    }

    // 2. Group affected symbols by their repo so each repo recomputes once.
    let mut by_repo: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for id in &affected {
        let Some(node) = ctx.store.get_node(id).map_err(HookError::from)? else {
            continue;
        };
        if !node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
            continue;
        }
        if let Some(repo) = property_string(&node.properties, "repo_id") {
            by_repo.entry(repo).or_default().push(id.clone());
        }
    }

    // 3. Localized PPR per repo; warm centrality back onto the neighborhood.
    let mut writes = 0usize;
    for (_repo, seeds) in by_repo {
        writes += recompute_localized_centrality(ctx.store, &seeds)?;
    }
    Ok(HookOutcome::Wrote { mutations: writes })
}

/// Bounded BFS from `seeds` over call/depend edges to gather the changed
/// neighborhood, run forward-push PPR seeded there, and write the resulting
/// `centrality` onto each touched `CodeSymbol`. Idempotent: unchanged values are
/// not re-written, so a self-triggered second pass does no work.
fn recompute_localized_centrality(
    store: &mut RedCoreGraphStore,
    seeds: &[String],
) -> Result<usize, HookError> {
    let edge_types = [CALLS_SYMBOL, DEPENDS_ON_SYMBOL];

    // Gather the neighborhood (both directions, to capture the component).
    let mut neighborhood: BTreeSet<String> = seeds.iter().cloned().collect();
    let mut frontier: Vec<String> = seeds.to_vec();
    'expand: for _ in 0..EXPANSION_DEPTH {
        let mut next = Vec::new();
        for node_id in &frontier {
            for direction in [Direction::Out, Direction::In] {
                for edge_type in edge_types {
                    let hits = store
                        .neighbors(NeighborQuery {
                            node_id: node_id.clone(),
                            direction: direction.clone(),
                            edge_type: Some(edge_type.to_string()),
                            include_expired: false,
                        })
                        .map_err(HookError::from)?;
                    for hit in hits {
                        if neighborhood.insert(hit.node_id.clone()) {
                            if neighborhood.len() >= MAX_NEIGHBORHOOD {
                                break 'expand; // localized cap reached
                            }
                            next.push(hit.node_id);
                        }
                    }
                }
            }
        }
        if next.is_empty() {
            break;
        }
        frontier = next;
    }

    // Directed adjacency (caller -> callee) restricted to the neighborhood.
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for node_id in &neighborhood {
        let mut outs = Vec::new();
        for edge_type in edge_types {
            let hits = store
                .neighbors(NeighborQuery {
                    node_id: node_id.clone(),
                    direction: Direction::Out,
                    edge_type: Some(edge_type.to_string()),
                    include_expired: false,
                })
                .map_err(HookError::from)?;
            for hit in hits {
                if neighborhood.contains(&hit.node_id) {
                    let weight = hit.confidence.unwrap_or(1.0).max(0.0);
                    outs.push((hit.node_id, weight));
                }
            }
        }
        adjacency.insert(node_id.clone(), outs);
    }

    let seed_map: HashMap<String, f64> = seeds
        .iter()
        .filter(|seed| neighborhood.contains(*seed))
        .map(|seed| (seed.clone(), 1.0))
        .collect();
    if seed_map.is_empty() {
        return Ok(0);
    }

    let scores = personalized_pagerank(
        &adjacency,
        &seed_map,
        PPR_ALPHA,
        PPR_EPSILON,
        PPR_MAX_PUSHES,
    );

    // Warm centrality back, idempotently.
    let mut writes = 0usize;
    for (node_id, score) in &scores {
        if *score <= 0.0 {
            continue;
        }
        let Some(mut node) = store.get_node(node_id).map_err(HookError::from)? else {
            continue;
        };
        if !node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
            continue;
        }
        let rounded = round6(*score);
        let prior = node
            .properties
            .get(CENTRALITY_PROPERTY)
            .and_then(Value::as_f64);
        if let Some(prior) = prior {
            if (prior - rounded).abs() <= CENTRALITY_EPSILON {
                continue; // unchanged: skip the write so the chain terminates
            }
        }
        match node.properties.as_object_mut() {
            Some(map) => {
                map.insert(CENTRALITY_PROPERTY.to_string(), json!(rounded));
            }
            None => {
                node.properties = json!({ CENTRALITY_PROPERTY: rounded });
            }
        }
        store.upsert_node(node).map_err(HookError::from)?;
        writes += 1;
    }
    Ok(writes)
}

fn round6(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}
