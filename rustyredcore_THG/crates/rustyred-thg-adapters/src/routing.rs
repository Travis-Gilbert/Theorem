use std::cmp::Ordering;
use std::collections::HashMap;

use serde_json::json;

use rustyred_thg_core::{
    personalized_pagerank, GraphMutation, GraphMutationBatch, ThgError, ThgResult,
};

use crate::fitness::{
    adapter_nodes, effective_fitness_from_node, find_adapter_node_by_id, is_shared_with_tenant,
    is_superseded,
};
use crate::types::{
    thg_error_from_store, AdapterFindRequest, AdapterGraphStore, AdapterRef, LoraAdapter,
    SHARED_WITH, TRAINED_ON,
};

pub fn find_adapters_for<S: AdapterGraphStore>(
    store: &S,
    req: &AdapterFindRequest,
) -> ThgResult<Vec<AdapterRef>> {
    let req = req.clone().normalized();
    if req.seed_node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let snapshot = store.snapshot().map_err(thg_error_from_store)?;
    let adjacency = adapter_adjacency(&snapshot.edges);
    let seed_weight = 1.0 / req.seed_node_ids.len() as f64;
    let seeds = req
        .seed_node_ids
        .iter()
        .map(|seed| (seed.clone(), seed_weight))
        .collect::<HashMap<_, _>>();
    let alpha = (1.0 - req.ppr_damping as f64).clamp(0.01, 0.99);
    let ppr = personalized_pagerank(&adjacency, &seeds, alpha, 1e-4, req.ppr_max_iter as usize);
    rank_candidate_scores(store, &req, ppr)
}

pub fn find_adapters_by_query_embedding<S: AdapterGraphStore>(
    store: &S,
    req: &AdapterFindRequest,
    query_embedding: &[f32],
) -> ThgResult<Vec<AdapterRef>> {
    let req = req.clone().normalized();
    if query_embedding.is_empty() {
        return Ok(Vec::new());
    }
    let mut scores = HashMap::new();
    for node in adapter_nodes(store)? {
        let Some(embedding) = embedding_from_properties(&node.properties) else {
            continue;
        };
        if embedding.len() != query_embedding.len() {
            continue;
        }
        let score = cosine_similarity(query_embedding, &embedding).max(0.0) as f64;
        if score > 0.0 {
            scores.insert(node.id.clone(), score);
        }
    }
    rank_candidate_scores(store, &req, scores)
}

pub fn recompute_embedding<S: AdapterGraphStore>(
    store: &mut S,
    adapter_id: &str,
) -> ThgResult<Option<Vec<f32>>> {
    let node = find_adapter_node_by_id(store, adapter_id)?
        .ok_or_else(|| ThgError::new("adapter_not_found", "adapter_id not found"))?;
    let adapter = LoraAdapter::from_node_record(&node)?;
    let Some(centroid) = adapter_training_centroid(store, &adapter.training_object_ids)? else {
        return Ok(None);
    };
    let mut updated = node;
    updated.properties["embedding"] = json!(centroid);
    store
        .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
            updated,
        )]))
        .map_err(thg_error_from_store)?;
    Ok(Some(centroid))
}

pub fn adapter_training_centroid<S: AdapterGraphStore>(
    store: &S,
    training_object_ids: &[i64],
) -> ThgResult<Option<Vec<f32>>> {
    let mut centroid: Vec<f32> = Vec::new();
    let mut count = 0usize;
    for object_pk in training_object_ids {
        let object_id = crate::types::object_node_id(*object_pk);
        let Some(node) = store.get_node(&object_id).map_err(thg_error_from_store)? else {
            continue;
        };
        let Some(embedding) = embedding_from_properties(&node.properties) else {
            continue;
        };
        if centroid.is_empty() {
            centroid = vec![0.0; embedding.len()];
        }
        if centroid.len() != embedding.len() {
            return Err(ThgError::new(
                "dimension_mismatch",
                "training Object embeddings have inconsistent dimensions",
            ));
        }
        for (slot, value) in centroid.iter_mut().zip(embedding) {
            *slot += value;
        }
        count += 1;
    }
    if count == 0 {
        return Ok(None);
    }
    for value in &mut centroid {
        *value /= count as f32;
    }
    Ok(Some(centroid))
}

fn rank_candidate_scores<S: AdapterGraphStore>(
    store: &S,
    req: &AdapterFindRequest,
    scores: HashMap<String, f64>,
) -> ThgResult<Vec<AdapterRef>> {
    let min_fitness = req.min_fitness.unwrap_or(crate::types::DEFAULT_MIN_FITNESS);
    let shared_weight = req
        .shared_weight
        .unwrap_or(crate::types::DEFAULT_SHARED_WEIGHT) as f64;
    let mut refs = Vec::new();
    for (node_id, score) in scores {
        let Some(node) = store.get_node(&node_id).map_err(thg_error_from_store)? else {
            continue;
        };
        let mut adapter = match LoraAdapter::from_node_record(&node) {
            Ok(adapter) => adapter,
            Err(_) => continue,
        };
        if let Some(base_model_sha) = req.base_model_sha.as_deref() {
            if adapter.base_model_sha != base_model_sha {
                continue;
            }
        }
        if !req.include_superseded && is_superseded(store, &adapter)? {
            continue;
        }
        let tenant_match = adapter.tenant_id == req.tenant_id;
        let shared = if tenant_match {
            false
        } else {
            is_shared_with_tenant(store, &adapter, &req.tenant_id)?
        };
        if !tenant_match && !shared {
            continue;
        }
        let fitness = effective_fitness_from_node(&node, &adapter);
        if fitness < min_fitness {
            continue;
        }
        adapter.fitness = fitness;
        let shared_multiplier = if shared { shared_weight } else { 1.0 };
        refs.push(AdapterRef {
            adapter,
            score: (score * fitness as f64 * shared_multiplier) as f32,
        });
    }
    refs.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.adapter.adapter_id.cmp(&b.adapter.adapter_id))
    });
    refs.truncate(req.k as usize);
    Ok(refs)
}

fn adapter_adjacency(
    edges: &[rustyred_thg_core::EdgeRecord],
) -> HashMap<String, Vec<(String, f64)>> {
    let mut adjacency: HashMap<String, Vec<(String, f64)>> = HashMap::new();
    for edge in edges {
        if edge.tombstone {
            continue;
        }
        match edge.edge_type.as_str() {
            TRAINED_ON => {
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), edge.effective_confidence().max(0.0)));
            }
            SHARED_WITH => {
                adjacency
                    .entry(edge.to_id.clone())
                    .or_default()
                    .push((edge.from_id.clone(), edge.effective_confidence().max(0.0)));
                adjacency
                    .entry(edge.from_id.clone())
                    .or_default()
                    .push((edge.to_id.clone(), edge.effective_confidence().max(0.0)));
            }
            _ => {}
        }
    }
    adjacency
}

fn embedding_from_properties(properties: &serde_json::Value) -> Option<Vec<f32>> {
    properties.get("embedding")?.as_array().map(|items| {
        items
            .iter()
            .filter_map(|item| {
                item.as_f64()
                    .map(|value| value as f32)
                    .or_else(|| item.as_str().and_then(|raw| raw.parse::<f32>().ok()))
            })
            .collect::<Vec<_>>()
    })
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
