//! Proactive, scoped, learned affordance selection.
//!
//! This is the cold-start Pairformer: given a task and a capability scope, it
//! returns a ranked set of affordances the agent is primed to use. The learned
//! prior is structural (Personalized PageRank seeded at the task-type node,
//! flowing back along `SERVED_TASK` / `PRODUCED_OUTCOME` / `SEQUENCED_WITH`)
//! multiplied by each affordance's time-decayed fitness. It works with zero
//! training: a positive recorded outcome raises both the structural link and
//! the fitness, so selection compounds with use. A trained Pairformer head
//! (an external job consuming the training export) is the later enrichment;
//! this PPR+fitness selector is the warm start.
//!
//! Selection is proactive, not a passthrough: the charter scopes the candidate
//! set and the learned prior ranks within it, so the model sees a curated,
//! ranked set rather than a thousand tools to triage.

use std::cmp::Ordering;
use std::collections::HashMap;

use rustyred_thg_core::{personalized_pagerank, EdgeRecord, ThgResult};

use crate::outcomes::{affordance_nodes, effective_affordance_fitness_from_node};
use crate::types::{
    embedding_from_properties, task_type_node_id, thg_error_from_store, Affordance,
    AffordanceGraphStore, AffordanceRef, SelectionRequest, DEFAULT_COLD_START_SCORE,
    DEFAULT_MIN_FITNESS, PRODUCED_OUTCOME, SEQUENCED_WITH, SERVED_TASK,
};

/// Select the affordances an agent should reach for, ranked by the learned
/// structural prior times fitness, within the capability scope.
pub fn select_affordances<S: AffordanceGraphStore>(
    store: &S,
    req: &SelectionRequest,
) -> ThgResult<Vec<AffordanceRef>> {
    let req = req.clone().normalized();
    if req.task_type.is_empty() {
        return Ok(Vec::new());
    }
    let snapshot = store.snapshot().map_err(thg_error_from_store)?;
    let adjacency = affordance_adjacency(&snapshot.edges);

    let seed_node = task_type_node_id(&req.tenant_id, &req.task_type);
    let mut seeds = HashMap::new();
    seeds.insert(seed_node, 1.0_f64);
    let alpha = (1.0 - req.ppr_damping as f64).clamp(0.01, 0.99);
    let ppr = personalized_pagerank(&adjacency, &seeds, alpha, 1e-4, req.ppr_max_iter as usize);

    let min_fitness = req.min_fitness.unwrap_or(DEFAULT_MIN_FITNESS);
    let mut refs = Vec::new();
    for node in affordance_nodes(store)? {
        let affordance = match Affordance::from_node_record(&node) {
            Ok(affordance) => affordance,
            Err(_) => continue,
        };
        if affordance.tenant_id != req.tenant_id {
            continue;
        }
        if !req.scope.admits(&affordance) {
            continue;
        }
        let fitness = effective_affordance_fitness_from_node(&node);
        if fitness < min_fitness {
            continue;
        }
        // Cold-start floor keeps an unprimed affordance reachable (the
        // forwarding fallback); the structural prior lifts the ones that have
        // served this task shape before.
        let structural = ppr.get(&node.id).copied().unwrap_or(0.0) + DEFAULT_COLD_START_SCORE;
        let mut scored = affordance;
        scored.fitness = fitness;
        refs.push(AffordanceRef {
            affordance: scored,
            score: (structural * fitness as f64) as f32,
        });
    }

    sort_and_truncate(&mut refs, req.k as usize);
    Ok(refs)
}

/// Select affordances by semantic similarity of a query embedding to the
/// affordances' description embeddings, within scope. Falls back to an empty
/// result when no embeddings are present (RustyRed has no text embedder; the
/// caller supplies vectors).
pub fn select_affordances_by_embedding<S: AffordanceGraphStore>(
    store: &S,
    req: &SelectionRequest,
    query_embedding: &[f32],
) -> ThgResult<Vec<AffordanceRef>> {
    let req = req.clone().normalized();
    if query_embedding.is_empty() {
        return Ok(Vec::new());
    }
    let min_fitness = req.min_fitness.unwrap_or(DEFAULT_MIN_FITNESS);
    let mut refs = Vec::new();
    for node in affordance_nodes(store)? {
        let affordance = match Affordance::from_node_record(&node) {
            Ok(affordance) => affordance,
            Err(_) => continue,
        };
        if affordance.tenant_id != req.tenant_id {
            continue;
        }
        if !req.scope.admits(&affordance) {
            continue;
        }
        let Some(embedding) = embedding_from_properties(&node.properties) else {
            continue;
        };
        if embedding.len() != query_embedding.len() {
            continue;
        }
        let similarity = cosine_similarity(query_embedding, &embedding).max(0.0) as f64;
        if similarity <= 0.0 {
            continue;
        }
        let fitness = effective_affordance_fitness_from_node(&node);
        if fitness < min_fitness {
            continue;
        }
        let mut scored = affordance;
        scored.fitness = fitness;
        refs.push(AffordanceRef {
            affordance: scored,
            score: (similarity * fitness as f64) as f32,
        });
    }
    sort_and_truncate(&mut refs, req.k as usize);
    Ok(refs)
}

fn sort_and_truncate(refs: &mut Vec<AffordanceRef>, k: usize) {
    refs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.affordance.affordance_id.cmp(&b.affordance.affordance_id))
    });
    refs.truncate(k);
}

/// Build the selection adjacency. PPR is seeded at the task-type node, so the
/// structural edges are reversed where needed to flow back to affordances:
/// - `SERVED_TASK` (affordance -> task_type) reversed so the seed reaches the
///   affordances that served it.
/// - `PRODUCED_OUTCOME` (affordance -> receipt) both directions, weighted by
///   the recorded outcome value, so well-evidenced affordances get a lift.
/// - `SEQUENCED_WITH` (affordance -> affordance) both directions, so tools
///   commonly used together share signal.
fn affordance_adjacency(edges: &[EdgeRecord]) -> HashMap<String, Vec<(String, f64)>> {
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for edge in edges {
        if edge.tombstone {
            continue;
        }
        match edge.edge_type.as_str() {
            SERVED_TASK => {
                // Clamp confidence into [0,1]. effective_confidence() defaults
                // to 1.0 when unset, so unconfidenced SERVED_TASK edges still
                // propagate at full weight; an explicit confidence is now
                // honored. (Was .max(0.0).max(1.0), which pinned every edge to
                // a constant 1.0 and ignored the confidence entirely.)
                let weight = edge.effective_confidence().clamp(0.0, 1.0);
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), weight));
            }
            PRODUCED_OUTCOME => {
                let weight = edge_outcome_weight(edge);
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), weight));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), weight));
            }
            SEQUENCED_WITH => {
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), 0.5));
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), 0.5));
            }
            _ => {}
        }
    }
    adjacency
}

fn edge_outcome_weight(edge: &EdgeRecord) -> f64 {
    edge.properties
        .get("outcome_value")
        .and_then(|value| value.as_f64())
        .unwrap_or(0.5)
        .max(0.05)
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for (left, right) in a.iter().zip(b) {
        dot += left * right;
        norm_a += left * left;
        norm_b += right * right;
    }
    if norm_a <= 1e-12 || norm_b <= 1e-12 {
        0.0
    } else {
        dot / (norm_a.sqrt() * norm_b.sqrt())
    }
}
