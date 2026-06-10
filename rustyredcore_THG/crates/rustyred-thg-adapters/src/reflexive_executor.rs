//! Executor-side reflexive inference: join plain topology with the
//! representation sidecar, apply GraphLoRA-style low-rank adapter deltas, and
//! run the bounded Pairformer during a MATCH walk.
//!
//! The split of responsibilities mirrors the plan: the graph stays plain
//! topology; learned representations live in the sidecar; adapter weights
//! live in their own factor sidecar (never in the node struct); and the
//! executor is the place where topology and sidecar meet at query time.
//! Output is always advisory candidates, never materialized edges.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    Direction, EdgeRecord, GraphMutation, GraphMutationBatch, GraphStoreResult, NeighborHit,
    NeighborQuery, NodeRecord, ThgError, ThgResult,
};

use crate::pairformer::{run_pairformer, PairformerConfig, PairformerEdgeInput, PairformerInput};
use crate::reflexive::{
    DensificationRequest, InferredEdgeCandidate, REPRESENTATION_SIDECAR_LABEL, REPRESENTS_NODE,
};
use crate::types::{
    adapter_node_id, edge_with_adapter_provenance, normalize_tenant_id, thg_error_from_store,
    AdapterGraphStore, THG_ADAPTER_SOURCE,
};

pub const ADAPTER_FACTORS_LABEL: &str = "AdapterFactorSidecar";
pub const FACTORS_FOR_ADAPTER: &str = "FACTORS_FOR_ADAPTER";

/// Read-only store surface the executor join needs. Implemented for the
/// adapter store types here, and directly by server-side store wrappers
/// (the Cypher query surface's tenant store) so the same join code serves
/// both. A blanket impl over [`AdapterGraphStore`] would block downstream
/// crates from implementing this trait for their own local store wrappers,
/// so the impls stay explicit.
pub trait ReflexiveReadStore {
    fn read_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>>;
    fn read_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>>;
    fn read_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>>;
}

impl ReflexiveReadStore for rustyred_thg_core::InMemoryGraphStore {
    fn read_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        AdapterGraphStore::get_node(self, id)
    }

    fn read_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        AdapterGraphStore::get_edge(self, id)
    }

    fn read_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        AdapterGraphStore::neighbors(self, query)
    }
}

impl ReflexiveReadStore for rustyred_thg_core::RedCoreGraphStore {
    fn read_node(&self, id: &str) -> GraphStoreResult<Option<NodeRecord>> {
        AdapterGraphStore::get_node(self, id)
    }

    fn read_edge(&self, id: &str) -> GraphStoreResult<Option<EdgeRecord>> {
        AdapterGraphStore::get_edge(self, id)
    }

    fn read_neighbors(&self, query: NeighborQuery) -> GraphStoreResult<Vec<NeighborHit>> {
        AdapterGraphStore::neighbors(self, query)
    }
}

/// A representation joined from the sidecar for one topology node.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct NodeRepresentation {
    pub node_id: String,
    pub representation_node_id: String,
    pub model_id: String,
    pub embedding: Vec<f32>,
    pub adapter_ids: Vec<String>,
    pub graph_version: u64,
}

/// Low-rank adapter factors in the GraphLoRA pattern: a frozen base
/// representation plus a small trainable delta `up @ (down @ x) * alpha/rank`.
/// Factors live in their own sidecar node keyed by adapter id.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LowRankAdapterFactors {
    pub adapter_id: String,
    pub rank: usize,
    pub input_dim: usize,
    pub alpha: f32,
    /// Row-major `[rank, input_dim]` down-projection (A).
    pub down: Vec<f32>,
    /// Row-major `[input_dim, rank]` up-projection (B).
    pub up: Vec<f32>,
}

impl LowRankAdapterFactors {
    pub fn validated(self) -> ThgResult<Self> {
        if self.adapter_id.trim().is_empty() {
            return Err(ThgError::new(
                "invalid_adapter_factors",
                "adapter_id is required",
            ));
        }
        if self.rank == 0 || self.input_dim == 0 {
            return Err(ThgError::new(
                "invalid_adapter_factors",
                "rank and input_dim must be non-zero",
            ));
        }
        if self.down.len() != self.rank * self.input_dim {
            return Err(ThgError::new(
                "invalid_adapter_factors",
                format!(
                    "down factor has {} values, expected rank*input_dim = {}",
                    self.down.len(),
                    self.rank * self.input_dim
                ),
            ));
        }
        if self.up.len() != self.input_dim * self.rank {
            return Err(ThgError::new(
                "invalid_adapter_factors",
                format!(
                    "up factor has {} values, expected input_dim*rank = {}",
                    self.up.len(),
                    self.input_dim * self.rank
                ),
            ));
        }
        if !self.alpha.is_finite()
            || self.down.iter().any(|value| !value.is_finite())
            || self.up.iter().any(|value| !value.is_finite())
        {
            return Err(ThgError::new(
                "invalid_adapter_factors",
                "alpha and factors must be finite",
            ));
        }
        Ok(self)
    }
}

pub fn adapter_factors_node_id(tenant_id: &str, adapter_id: &str) -> String {
    format!(
        "adapter_factor_sidecar:{}:{}",
        normalize_tenant_id(tenant_id),
        adapter_id.trim()
    )
}

/// Write adapter factors into the factor sidecar. When the adapter catalog
/// node exists, the sidecar links to it for provenance.
pub fn upsert_adapter_factors_sidecar<S: AdapterGraphStore>(
    store: &mut S,
    tenant_id: &str,
    factors: LowRankAdapterFactors,
    actor: Option<&str>,
) -> ThgResult<String> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let factors = factors.validated()?;
    let node_id = adapter_factors_node_id(&tenant_id, &factors.adapter_id);
    let mut mutations = vec![GraphMutation::NodeUpsert(NodeRecord::new(
        &node_id,
        [ADAPTER_FACTORS_LABEL],
        json!({
            "tenant_id": tenant_id,
            "adapter_id": factors.adapter_id,
            "rank": factors.rank,
            "input_dim": factors.input_dim,
            "alpha": factors.alpha,
            "down": factors.down,
            "up": factors.up,
            "source": THG_ADAPTER_SOURCE,
        }),
    ))];

    let catalog_node_id = adapter_node_id(&tenant_id, &factors.adapter_id);
    if store
        .get_node(&catalog_node_id)
        .map_err(thg_error_from_store)?
        .is_some()
    {
        mutations.push(GraphMutation::EdgeUpsert(edge_with_adapter_provenance(
            format!("edge:{node_id}:{FACTORS_FOR_ADAPTER}:{catalog_node_id}"),
            &node_id,
            FACTORS_FOR_ADAPTER,
            &catalog_node_id,
            json!({ "tenant_id": tenant_id }),
            actor,
        )));
    }

    store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    Ok(node_id)
}

pub fn load_adapter_factors<S: ReflexiveReadStore>(
    store: &S,
    tenant_id: &str,
    adapter_id: &str,
) -> ThgResult<Option<LowRankAdapterFactors>> {
    let node_id = adapter_factors_node_id(tenant_id, adapter_id);
    let Some(node) = store.read_node(&node_id).map_err(thg_error_from_store)? else {
        return Ok(None);
    };
    let properties = &node.properties;
    let factors = LowRankAdapterFactors {
        adapter_id: property_string(properties, "adapter_id")
            .unwrap_or_else(|| adapter_id.trim().to_string()),
        rank: property_usize(properties, "rank"),
        input_dim: property_usize(properties, "input_dim"),
        alpha: property_f32(properties, "alpha").unwrap_or(1.0),
        down: numeric_array(properties, "down"),
        up: numeric_array(properties, "up"),
    };
    Ok(Some(factors.validated()?))
}

/// Apply the low-rank delta: `x + up @ (down @ x) * alpha / rank`. The base
/// representation stays frozen; the adapter contributes only the delta.
pub fn apply_low_rank_adapter(
    embedding: &[f32],
    factors: &LowRankAdapterFactors,
) -> ThgResult<Vec<f32>> {
    if embedding.len() != factors.input_dim {
        return Err(ThgError::new(
            "adapter_dimension_mismatch",
            format!(
                "embedding has {} dims, adapter {} expects {}",
                embedding.len(),
                factors.adapter_id,
                factors.input_dim
            ),
        ));
    }
    let mut bottleneck = vec![0.0_f32; factors.rank];
    for (row, slot) in bottleneck.iter_mut().enumerate() {
        let offset = row * factors.input_dim;
        *slot = factors.down[offset..offset + factors.input_dim]
            .iter()
            .zip(embedding)
            .map(|(weight, value)| weight * value)
            .sum();
    }
    let scale = factors.alpha / factors.rank as f32;
    let mut adapted = embedding.to_vec();
    for (dim_idx, slot) in adapted.iter_mut().enumerate() {
        let offset = dim_idx * factors.rank;
        let delta: f32 = factors.up[offset..offset + factors.rank]
            .iter()
            .zip(&bottleneck)
            .map(|(weight, value)| weight * value)
            .sum();
        *slot += delta * scale;
    }
    Ok(adapted)
}

/// Join one topology node with its sidecar representation: incoming
/// `REPRESENTS_NODE` edges point from sidecar nodes at the topology node.
/// When several representations exist, the highest `graph_version` wins,
/// with the node id as a deterministic tie-break.
pub fn load_node_representation<S: ReflexiveReadStore>(
    store: &S,
    tenant_id: &str,
    node_id: &str,
) -> ThgResult<Option<NodeRepresentation>> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let hits = store
        .read_neighbors(NeighborQuery {
            node_id: node_id.to_string(),
            direction: Direction::In,
            edge_type: Some(REPRESENTS_NODE.to_string()),
            include_expired: false,
        })
        .map_err(thg_error_from_store)?;

    let mut best: Option<NodeRepresentation> = None;
    for hit in hits {
        let Some(sidecar) = store
            .read_node(&hit.node_id)
            .map_err(thg_error_from_store)?
        else {
            continue;
        };
        if sidecar.tombstone
            || !sidecar
                .labels
                .iter()
                .any(|label| label == REPRESENTATION_SIDECAR_LABEL)
        {
            continue;
        }
        let properties = &sidecar.properties;
        if property_string(properties, "tenant_id").as_deref() != Some(tenant_id.as_str()) {
            continue;
        }
        let embedding = numeric_array(properties, "embedding");
        if embedding.is_empty() {
            continue;
        }
        let representation = NodeRepresentation {
            node_id: node_id.to_string(),
            representation_node_id: sidecar.id.clone(),
            model_id: property_string(properties, "model_id").unwrap_or_default(),
            embedding,
            adapter_ids: string_array(properties, "adapter_ids"),
            graph_version: properties
                .get("graph_version")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        };
        best = match best {
            Some(prior)
                if (prior.graph_version, &prior.representation_node_id)
                    >= (
                        representation.graph_version,
                        &representation.representation_node_id,
                    ) =>
            {
                Some(prior)
            }
            _ => Some(representation),
        };
    }
    Ok(best)
}

/// Everything the pure scorer needs, gathered by whichever executor owns the
/// store access (Cypher query surface, MCP tool, or test).
#[derive(Clone, Debug, Default)]
pub struct MatchNeighborhoodInput {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    pub representations: Vec<NodeRepresentation>,
    pub adapter_factors: Vec<LowRankAdapterFactors>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchInferenceResult {
    pub tenant_id: String,
    pub considered_node_ids: Vec<String>,
    pub representations_joined: usize,
    pub adapters_applied: usize,
    pub adapter_skips: Vec<String>,
    pub bounded: bool,
    pub candidates: Vec<InferredEdgeCandidate>,
}

/// Run the Pairformer over a bounded MATCH neighborhood with sidecar-fed,
/// adapter-adjusted representations. Pure function: no store access, so the
/// query surface can call it with its own reads.
pub fn score_match_neighborhood(
    input: &MatchNeighborhoodInput,
    request: DensificationRequest,
    config: PairformerConfig,
) -> ThgResult<MatchInferenceResult> {
    let request = request.normalized();
    let config = config.normalized();

    let mut nodes = input
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .cloned()
        .collect::<Vec<_>>();
    nodes.sort_by(|left, right| left.id.cmp(&right.id));
    nodes.dedup_by(|left, right| left.id == right.id);
    let mut bounded = false;
    if nodes.len() > config.max_nodes {
        bounded = true;
        nodes.truncate(config.max_nodes);
    }
    if nodes.is_empty() {
        return Ok(MatchInferenceResult {
            tenant_id: request.tenant_id,
            considered_node_ids: Vec::new(),
            representations_joined: 0,
            adapters_applied: 0,
            adapter_skips: Vec::new(),
            bounded,
            candidates: Vec::new(),
        });
    }
    let node_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();

    let representations_by_node = input
        .representations
        .iter()
        .map(|representation| (representation.node_id.clone(), representation))
        .collect::<BTreeMap<_, _>>();
    let factors_by_adapter = input
        .adapter_factors
        .iter()
        .map(|factors| (factors.adapter_id.clone(), factors))
        .collect::<BTreeMap<_, _>>();

    let mut representations_joined = 0usize;
    let mut adapters_applied = 0usize;
    let mut adapter_skips = Vec::new();

    let pairformer_nodes = nodes
        .iter()
        .map(|node| {
            let features = match representations_by_node.get(&node.id) {
                Some(representation) => {
                    representations_joined += 1;
                    let mut embedding = representation.embedding.clone();
                    for adapter_id in &representation.adapter_ids {
                        match factors_by_adapter.get(adapter_id) {
                            Some(factors) => match apply_low_rank_adapter(&embedding, factors) {
                                Ok(adapted) => {
                                    embedding = adapted;
                                    adapters_applied += 1;
                                }
                                Err(error) => {
                                    adapter_skips
                                        .push(format!("{}: {}", adapter_id, error.message));
                                }
                            },
                            None => {
                                adapter_skips.push(format!("{adapter_id}: factors_not_loaded"));
                            }
                        }
                    }
                    embedding
                }
                None => fallback_node_features(node),
            };
            crate::pairformer::PairformerNodeInput {
                node_id: node.id.clone(),
                features,
            }
        })
        .collect::<Vec<_>>();

    let edges = input
        .edges
        .iter()
        .filter(|edge| {
            !edge.tombstone
                && node_ids.contains(&edge.from_id)
                && node_ids.contains(&edge.to_id)
                && edge.effective_confidence() as f32 >= request.min_path_confidence
        })
        .collect::<Vec<_>>();
    let existing_direct_pairs = edges
        .iter()
        .map(|edge| (edge.from_id.clone(), edge.to_id.clone()))
        .collect::<BTreeSet<_>>();
    let pairformer_edges = edges
        .iter()
        .map(|edge| PairformerEdgeInput {
            edge_id: edge.id.clone(),
            source_id: edge.from_id.clone(),
            target_id: edge.to_id.clone(),
            edge_type: edge.edge_type.clone(),
            features: fallback_edge_features(edge),
            confidence: edge.effective_confidence() as f32,
        })
        .collect::<Vec<_>>();

    let output = run_pairformer(
        &PairformerInput {
            nodes: pairformer_nodes,
            edges: pairformer_edges,
        },
        config,
    )?;

    let mut candidates = output
        .link_scores
        .iter()
        .filter_map(|score| {
            crate::reflexive::pairformer_score_to_candidate(&request, score, &existing_direct_pairs)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .confidence
            .partial_cmp(&left.confidence)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.candidate_id.cmp(&right.candidate_id))
    });
    candidates.truncate(request.max_candidates);

    Ok(MatchInferenceResult {
        tenant_id: request.tenant_id,
        considered_node_ids: nodes.into_iter().map(|node| node.id).collect(),
        representations_joined,
        adapters_applied,
        adapter_skips,
        bounded,
        candidates,
    })
}

/// Store-generic convenience wrapper: gather the bounded neighborhood's
/// nodes, interior edges, sidecar representations, and adapter factors, then
/// run the pure scorer. Callers with their own store access (the server query
/// surface) do the same join inline and call [`score_match_neighborhood`].
pub fn reflexive_match_inference<S: ReflexiveReadStore>(
    store: &S,
    node_ids: &[String],
    request: DensificationRequest,
    config: PairformerConfig,
) -> ThgResult<MatchInferenceResult> {
    let tenant_id = normalize_tenant_id(&request.tenant_id);
    let mut nodes = Vec::new();
    let mut edges: BTreeMap<String, EdgeRecord> = BTreeMap::new();
    let id_set = node_ids.iter().cloned().collect::<BTreeSet<_>>();

    for node_id in &id_set {
        let Some(node) = store.read_node(node_id).map_err(thg_error_from_store)? else {
            continue;
        };
        if node.tombstone {
            continue;
        }
        let hits = store
            .read_neighbors(NeighborQuery {
                node_id: node_id.clone(),
                direction: Direction::Out,
                edge_type: None,
                include_expired: false,
            })
            .map_err(thg_error_from_store)?;
        for hit in hits {
            if !id_set.contains(&hit.node_id) {
                continue;
            }
            if let Some(edge) = store
                .read_edge(&hit.edge_id)
                .map_err(thg_error_from_store)?
            {
                edges.insert(edge.id.clone(), edge);
            }
        }
        nodes.push(node);
    }

    let mut representations = Vec::new();
    let mut adapter_ids = BTreeSet::new();
    for node in &nodes {
        if let Some(representation) = load_node_representation(store, &tenant_id, &node.id)? {
            adapter_ids.extend(representation.adapter_ids.iter().cloned());
            representations.push(representation);
        }
    }
    let mut adapter_factors = Vec::new();
    for adapter_id in adapter_ids {
        if let Some(factors) = load_adapter_factors(store, &tenant_id, &adapter_id)? {
            adapter_factors.push(factors);
        }
    }

    score_match_neighborhood(
        &MatchNeighborhoodInput {
            nodes,
            edges: edges.into_values().collect(),
            representations,
            adapter_factors,
        },
        request,
        config,
    )
}

fn fallback_node_features(node: &NodeRecord) -> Vec<f32> {
    let mut features = numeric_array(&node.properties, "embedding");
    features.extend(numeric_array(&node.properties, "features"));
    features.push(node.labels.len() as f32);
    features.push(node.version as f32 / 1024.0);
    features
}

fn fallback_edge_features(edge: &EdgeRecord) -> Vec<f32> {
    let mut features = numeric_array(&edge.properties, "embedding");
    features.extend(numeric_array(&edge.properties, "features"));
    features.push(edge.effective_confidence() as f32);
    features.push((edge.edge_type.len() as f32).ln_1p());
    features
}

fn property_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn property_usize(properties: &Value, key: &str) -> usize {
    properties.get(key).and_then(Value::as_u64).unwrap_or(0) as usize
}

fn property_f32(properties: &Value, key: &str) -> Option<f32> {
    properties
        .get(key)
        .and_then(Value::as_f64)
        .map(|value| value as f32)
}

fn numeric_array(properties: &Value, key: &str) -> Vec<f32> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_f64().map(|value| value as f32))
                .filter(|value| value.is_finite())
                .collect()
        })
        .unwrap_or_default()
}

fn string_array(properties: &Value, key: &str) -> Vec<String> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}
