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
#[cfg(feature = "pairformer-burn-cubecl")]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(feature = "pairformer-burn-cubecl")]
use rustyred_thg_core::stable_hash;
use rustyred_thg_core::{
    Direction, EdgeRecord, GraphMutation, GraphMutationBatch, GraphStoreResult, NeighborHit,
    NeighborQuery, NodeRecord, ThgError, ThgResult,
};

use crate::pairformer::{run_pairformer, PairformerConfig, PairformerEdgeInput, PairformerInput};
use crate::reflexive::{
    DensificationRequest, InferredEdgeCandidate, REPRESENTATION_SIDECAR_LABEL, REPRESENTS_NODE,
};
#[cfg(feature = "pairformer-burn-cubecl")]
use crate::training_substrate::{EVALUATED_BY, MODEL_ARTIFACT_LABEL, PROMOTED_TO_ACTIVE};
#[cfg(feature = "pairformer-burn-cubecl")]
use crate::types::tenant_node_id;
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
#[serde(rename_all = "snake_case")]
pub enum MatchInferenceScorer {
    LearnedBurnPairformer,
    DeterministicPairformer,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MatchInferenceResult {
    pub tenant_id: String,
    pub considered_node_ids: Vec<String>,
    pub representations_joined: usize,
    pub adapters_applied: usize,
    pub adapter_skips: Vec<String>,
    pub scorer: MatchInferenceScorer,
    pub scorer_model_id: String,
    pub scorer_notes: Vec<String>,
    pub bounded: bool,
    pub candidates: Vec<InferredEdgeCandidate>,
}

struct PreparedMatchNeighborhood {
    considered_node_ids: Vec<String>,
    representations_joined: usize,
    adapters_applied: usize,
    adapter_skips: Vec<String>,
    bounded: bool,
    pairformer_input: PairformerInput,
    existing_direct_pairs: BTreeSet<(String, String)>,
}

/// Run the Pairformer over a bounded MATCH neighborhood with sidecar-fed,
/// adapter-adjusted representations. Pure function: no store access, so the
/// query surface can call it with its own reads.
pub fn score_match_neighborhood(
    input: &MatchNeighborhoodInput,
    request: DensificationRequest,
    config: PairformerConfig,
) -> ThgResult<MatchInferenceResult> {
    let (request, config, prepared) = prepare_match_neighborhood(input, request, config)?;
    score_prepared_match_with_deterministic(
        &request,
        config,
        prepared,
        Vec::new(),
        request.model_id.clone(),
    )
}

fn prepare_match_neighborhood(
    input: &MatchNeighborhoodInput,
    request: DensificationRequest,
    config: PairformerConfig,
) -> ThgResult<(
    DensificationRequest,
    PairformerConfig,
    PreparedMatchNeighborhood,
)> {
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
        return Ok((
            request,
            config,
            PreparedMatchNeighborhood {
                considered_node_ids: Vec::new(),
                representations_joined: 0,
                adapters_applied: 0,
                adapter_skips: Vec::new(),
                bounded,
                pairformer_input: PairformerInput {
                    nodes: Vec::new(),
                    edges: Vec::new(),
                },
                existing_direct_pairs: BTreeSet::new(),
            },
        ));
    }
    let considered_node_ids = nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>();
    let node_ids = considered_node_ids.iter().cloned().collect::<BTreeSet<_>>();

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

    Ok((
        request,
        config,
        PreparedMatchNeighborhood {
            considered_node_ids,
            representations_joined,
            adapters_applied,
            adapter_skips,
            bounded,
            pairformer_input: PairformerInput {
                nodes: pairformer_nodes,
                edges: pairformer_edges,
            },
            existing_direct_pairs,
        },
    ))
}

fn score_prepared_match_with_deterministic(
    request: &DensificationRequest,
    config: PairformerConfig,
    prepared: PreparedMatchNeighborhood,
    scorer_notes: Vec<String>,
    scorer_model_id: String,
) -> ThgResult<MatchInferenceResult> {
    if prepared.pairformer_input.nodes.is_empty() {
        return Ok(MatchInferenceResult {
            tenant_id: request.tenant_id.clone(),
            considered_node_ids: prepared.considered_node_ids,
            representations_joined: prepared.representations_joined,
            adapters_applied: prepared.adapters_applied,
            adapter_skips: prepared.adapter_skips,
            scorer: MatchInferenceScorer::DeterministicPairformer,
            scorer_model_id,
            scorer_notes,
            bounded: prepared.bounded,
            candidates: Vec::new(),
        });
    }

    let output = run_pairformer(&prepared.pairformer_input, config)?;
    let candidates = candidates_from_link_scores(
        request,
        &prepared.existing_direct_pairs,
        &output.link_scores,
    );

    Ok(MatchInferenceResult {
        tenant_id: request.tenant_id.clone(),
        considered_node_ids: prepared.considered_node_ids,
        representations_joined: prepared.representations_joined,
        adapters_applied: prepared.adapters_applied,
        adapter_skips: prepared.adapter_skips,
        scorer: MatchInferenceScorer::DeterministicPairformer,
        scorer_model_id,
        scorer_notes,
        bounded: prepared.bounded,
        candidates,
    })
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn score_prepared_match_with_link_scores(
    request: &DensificationRequest,
    prepared: PreparedMatchNeighborhood,
    link_scores: &[crate::pairformer::PairformerLinkScore],
    scorer: MatchInferenceScorer,
    scorer_model_id: String,
    scorer_notes: Vec<String>,
) -> MatchInferenceResult {
    let candidates =
        candidates_from_link_scores(request, &prepared.existing_direct_pairs, link_scores);

    MatchInferenceResult {
        tenant_id: request.tenant_id.clone(),
        considered_node_ids: prepared.considered_node_ids,
        representations_joined: prepared.representations_joined,
        adapters_applied: prepared.adapters_applied,
        adapter_skips: prepared.adapter_skips,
        scorer,
        scorer_model_id,
        scorer_notes,
        bounded: prepared.bounded,
        candidates,
    }
}

fn candidates_from_link_scores(
    request: &DensificationRequest,
    existing_direct_pairs: &BTreeSet<(String, String)>,
    link_scores: &[crate::pairformer::PairformerLinkScore],
) -> Vec<InferredEdgeCandidate> {
    let mut candidates = link_scores
        .iter()
        .filter_map(|score| {
            crate::reflexive::pairformer_score_to_candidate(request, score, existing_direct_pairs)
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
    candidates
}

#[cfg(feature = "pairformer-burn-cubecl")]
struct PromotedPairformerArtifact {
    model_id: String,
    config: crate::burn_pairformer::BurnPairformerConfig,
    local_path: Option<PathBuf>,
    source_graph_version: u64,
    ranking_accuracy: Option<f32>,
}

#[cfg(feature = "pairformer-burn-cubecl")]
const ONLINE_PAIRFORMER_MODEL_ID: &str = "pairformer-burn/online-bootstrap";

#[cfg(feature = "pairformer-burn-cubecl")]
const ONLINE_PAIRFORMER_EPOCHS: usize = 8;

#[cfg(feature = "pairformer-burn-cubecl")]
static ONLINE_PAIRFORMER_SEED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(feature = "pairformer-burn-cubecl")]
fn score_prepared_match_with_default_learning_model<S: ReflexiveReadStore>(
    store: &S,
    request: &DensificationRequest,
    config: PairformerConfig,
    prepared: PreparedMatchNeighborhood,
) -> ThgResult<MatchInferenceResult> {
    if prepared.pairformer_input.nodes.is_empty() {
        return Ok(score_prepared_match_with_link_scores(
            request,
            prepared,
            &[],
            MatchInferenceScorer::LearnedBurnPairformer,
            ONLINE_PAIRFORMER_MODEL_ID.to_string(),
            vec!["empty_pairformer_neighborhood".to_string()],
        ));
    }

    use burn::backend::ndarray::NdArrayDevice;
    type InferenceBackend = burn::backend::NdArray<f32>;

    let artifact = load_promoted_pairformer_artifact(store, &request.tenant_id)?;
    let burn_config = artifact
        .as_ref()
        .map(|artifact| artifact.config)
        .unwrap_or_else(|| burn_config_from_pairformer_config(&config, &prepared.pairformer_input));
    let mut scorer_notes = Vec::new();
    let device = NdArrayDevice::default();

    if let Some(artifact) = artifact {
        if let Some(local_path) = artifact.local_path.clone() {
            match crate::burn_pairformer::load_pairformer_file::<InferenceBackend>(
                &artifact.config,
                &local_path,
                &device,
            ) {
                Ok(model) => match crate::burn_pairformer::score_links_with_trained(
                    &model,
                    &device,
                    &prepared.pairformer_input,
                    &artifact.config,
                ) {
                    Ok(link_scores) => {
                        return Ok(score_prepared_match_with_link_scores(
                            request,
                            prepared,
                            &link_scores,
                            MatchInferenceScorer::LearnedBurnPairformer,
                            artifact.model_id,
                            vec!["promoted_pairformer_artifact_loaded".to_string()],
                        ));
                    }
                    Err(error) => scorer_notes.push(format!(
                        "promoted_pairformer_score_failed:{}:{}",
                        artifact.model_id, error.message
                    )),
                },
                Err(error) => scorer_notes.push(format!(
                    "promoted_pairformer_load_failed:{}:{}",
                    artifact.model_id, error.message
                )),
            }
        } else {
            scorer_notes.push(format!(
                "promoted_pairformer_artifact_remote_only:{}",
                artifact.model_id
            ));
        }
    } else {
        scorer_notes.push("no_promoted_pairformer_artifact_using_online_training".to_string());
    }

    score_prepared_match_with_online_pairformer(
        request,
        config,
        prepared,
        burn_config,
        scorer_notes,
    )
}

#[cfg(not(feature = "pairformer-burn-cubecl"))]
fn score_prepared_match_with_default_learning_model<S: ReflexiveReadStore>(
    _store: &S,
    request: &DensificationRequest,
    config: PairformerConfig,
    prepared: PreparedMatchNeighborhood,
) -> ThgResult<MatchInferenceResult> {
    score_prepared_match_with_deterministic(
        request,
        config,
        prepared,
        vec!["pairformer_burn_cubecl_feature_disabled".to_string()],
        request.model_id.clone(),
    )
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn score_prepared_match_with_online_pairformer(
    request: &DensificationRequest,
    deterministic_config: PairformerConfig,
    prepared: PreparedMatchNeighborhood,
    burn_config: crate::burn_pairformer::BurnPairformerConfig,
    mut scorer_notes: Vec<String>,
) -> ThgResult<MatchInferenceResult> {
    use burn::backend::ndarray::NdArrayDevice;
    use burn::module::AutodiffModule;
    type InferenceBackend = burn::backend::NdArray<f32>;
    type TrainingBackend = burn::backend::Autodiff<InferenceBackend>;

    let burn_config = burn_config.normalized();
    let device = NdArrayDevice::default();
    let seed = online_pairformer_seed(request, &prepared.pairformer_input);
    let training = crate::burn_pairformer::PairformerTrainingConfig {
        epochs: ONLINE_PAIRFORMER_EPOCHS,
        learning_rate: 4e-3,
        mask_fraction: 0.3,
        negatives_per_positive: 2,
        seed,
    };

    let trained = {
        let _seed_guard = ONLINE_PAIRFORMER_SEED_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        crate::burn_pairformer::train_pairformer::<TrainingBackend>(
            &device,
            &prepared.pairformer_input,
            burn_config,
            training,
        )
    };

    match trained {
        Ok((model, report)) => {
            let model = model.valid();
            match crate::burn_pairformer::score_links_with_trained(
                &model,
                &device,
                &prepared.pairformer_input,
                &burn_config,
            ) {
                Ok(link_scores) => {
                    let link_scores = blend_online_scores_with_structural_prior(
                        &prepared.pairformer_input,
                        &link_scores,
                        &deterministic_config,
                    )?;
                    scorer_notes.push(format!(
                        "online_pairformer_trained:epochs={}:ranking_accuracy={:.3}",
                        report.epochs, report.final_ranking_accuracy
                    ));
                    scorer_notes.push("online_pairformer_structural_prior_blended".to_string());
                    Ok(score_prepared_match_with_link_scores(
                        request,
                        prepared,
                        &link_scores,
                        MatchInferenceScorer::LearnedBurnPairformer,
                        ONLINE_PAIRFORMER_MODEL_ID.to_string(),
                        scorer_notes,
                    ))
                }
                Err(error) => score_prepared_match_with_deterministic(
                    request,
                    deterministic_config,
                    prepared,
                    with_note(
                        scorer_notes,
                        format!("online_pairformer_score_failed:{}", error.message),
                    ),
                    request.model_id.clone(),
                ),
            }
        }
        Err(error) if error.code == "invalid_pairformer_training" => {
            score_prepared_match_with_seeded_burn_model(
                request,
                deterministic_config,
                prepared,
                burn_config,
                with_note(
                    scorer_notes,
                    format!("online_pairformer_training_unavailable:{}", error.message),
                ),
                seed,
            )
        }
        Err(error) => score_prepared_match_with_seeded_burn_model(
            request,
            deterministic_config,
            prepared,
            burn_config,
            with_note(
                scorer_notes,
                format!("online_pairformer_training_failed:{}", error.message),
            ),
            seed,
        ),
    }
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn score_prepared_match_with_seeded_burn_model(
    request: &DensificationRequest,
    deterministic_config: PairformerConfig,
    prepared: PreparedMatchNeighborhood,
    burn_config: crate::burn_pairformer::BurnPairformerConfig,
    mut scorer_notes: Vec<String>,
    seed: u64,
) -> ThgResult<MatchInferenceResult> {
    use burn::backend::ndarray::NdArrayDevice;
    type InferenceBackend = burn::backend::NdArray<f32>;

    let device = NdArrayDevice::default();
    let model = {
        let _seed_guard = ONLINE_PAIRFORMER_SEED_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        <InferenceBackend as burn::tensor::backend::Backend>::seed(&device, seed);
        burn_config.init::<InferenceBackend>(&device)
    };
    match crate::burn_pairformer::score_links_with_trained(
        &model,
        &device,
        &prepared.pairformer_input,
        &burn_config,
    ) {
        Ok(link_scores) => {
            let link_scores = blend_online_scores_with_structural_prior(
                &prepared.pairformer_input,
                &link_scores,
                &deterministic_config,
            )?;
            scorer_notes.push("online_pairformer_initialized".to_string());
            scorer_notes.push("online_pairformer_structural_prior_blended".to_string());
            Ok(score_prepared_match_with_link_scores(
                request,
                prepared,
                &link_scores,
                MatchInferenceScorer::LearnedBurnPairformer,
                ONLINE_PAIRFORMER_MODEL_ID.to_string(),
                scorer_notes,
            ))
        }
        Err(error) => score_prepared_match_with_deterministic(
            request,
            deterministic_config,
            prepared,
            with_note(
                scorer_notes,
                format!("online_pairformer_init_score_failed:{}", error.message),
            ),
            request.model_id.clone(),
        ),
    }
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn blend_online_scores_with_structural_prior(
    input: &PairformerInput,
    online_scores: &[crate::pairformer::PairformerLinkScore],
    config: &PairformerConfig,
) -> ThgResult<Vec<crate::pairformer::PairformerLinkScore>> {
    let mut blended = run_pairformer(input, config.clone())?
        .link_scores
        .into_iter()
        .map(|score| ((score.source_id.clone(), score.target_id.clone()), score))
        .collect::<BTreeMap<_, _>>();

    for score in online_scores {
        blended
            .entry((score.source_id.clone(), score.target_id.clone()))
            .and_modify(|prior| {
                prior.score = prior.score.max(score.score);
                if prior.support_path.is_none() {
                    prior.support_path = score.support_path.clone();
                }
            })
            .or_insert_with(|| score.clone());
    }

    Ok(blended.into_values().collect())
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn burn_config_from_pairformer_config(
    config: &PairformerConfig,
    input: &PairformerInput,
) -> crate::burn_pairformer::BurnPairformerConfig {
    let config = config.clone().normalized();
    let node_in_dim = input
        .nodes
        .iter()
        .map(|node| node.features.len())
        .max()
        .unwrap_or(16)
        .max(1);
    let edge_in_dim = input
        .edges
        .iter()
        .map(|edge| edge.features.len())
        .max()
        .unwrap_or(8)
        .max(1);
    let pair_dim = config.pair_dim.max(1);
    let single_dim = config.single_dim.max(1);
    let mut heads = pair_dim.min(single_dim).min(4).max(1);
    while heads > 1 && (pair_dim % heads != 0 || single_dim % heads != 0) {
        heads -= 1;
    }
    let transition_base = pair_dim.max(single_dim).max(1);
    let transition_mult =
        (config.transition_hidden_dim.max(transition_base) + transition_base - 1) / transition_base;

    crate::burn_pairformer::BurnPairformerConfig {
        node_in_dim,
        edge_in_dim,
        pair_dim,
        single_dim,
        heads,
        blocks: config.blocks.max(1),
        transition_mult: transition_mult.max(1),
        max_nodes: config.max_nodes.max(1),
    }
    .normalized()
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn online_pairformer_seed(request: &DensificationRequest, input: &PairformerInput) -> u64 {
    let digest = stable_hash(json!({
        "tenant_id": request.tenant_id,
        "seed_node_ids": request.seed_node_ids,
        "nodes": input.nodes.iter().map(|node| &node.node_id).collect::<Vec<_>>(),
        "edges": input.edges.iter().map(|edge| &edge.edge_id).collect::<Vec<_>>(),
    }));
    digest
        .strip_prefix("sha256:")
        .and_then(|hex| hex.get(..16))
        .and_then(|hex| u64::from_str_radix(hex, 16).ok())
        .unwrap_or(17)
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn with_note(mut notes: Vec<String>, note: impl Into<String>) -> Vec<String> {
    notes.push(note.into());
    notes
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn load_promoted_pairformer_artifact<S: ReflexiveReadStore>(
    store: &S,
    tenant_id: &str,
) -> ThgResult<Option<PromotedPairformerArtifact>> {
    let tenant_id = normalize_tenant_id(tenant_id);
    let hits = store
        .read_neighbors(NeighborQuery {
            node_id: tenant_node_id(&tenant_id),
            direction: Direction::Out,
            edge_type: Some(PROMOTED_TO_ACTIVE.to_string()),
            include_expired: false,
        })
        .map_err(thg_error_from_store)?;

    let mut artifacts = Vec::new();
    for hit in hits {
        let Some(node) = store
            .read_node(&hit.node_id)
            .map_err(thg_error_from_store)?
        else {
            continue;
        };
        if node.tombstone
            || !node
                .labels
                .iter()
                .any(|label| label == MODEL_ARTIFACT_LABEL)
        {
            continue;
        }
        let properties = &node.properties;
        if property_string(properties, "tenant_id").as_deref() != Some(tenant_id.as_str())
            || property_string(properties, "model_type").as_deref() != Some("pairformer-burn")
            || property_string(properties, "promotion_decision").as_deref() != Some("active")
        {
            continue;
        }

        let (config, ranking_accuracy) =
            promoted_pairformer_metrics(store, &node.id)?.unwrap_or_default();
        artifacts.push(PromotedPairformerArtifact {
            model_id: property_string(properties, "model_id").unwrap_or_else(|| node.id.clone()),
            config,
            local_path: promoted_pairformer_local_path(properties),
            source_graph_version: properties
                .get("source_graph_version")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            ranking_accuracy,
        });
    }

    artifacts.sort_by(|left, right| {
        right
            .source_graph_version
            .cmp(&left.source_graph_version)
            .then_with(|| {
                right
                    .ranking_accuracy
                    .partial_cmp(&left.ranking_accuracy)
                    .unwrap_or(Ordering::Equal)
            })
            .then_with(|| left.model_id.cmp(&right.model_id))
    });
    Ok(artifacts.into_iter().next())
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn promoted_pairformer_metrics<S: ReflexiveReadStore>(
    store: &S,
    model_node_id: &str,
) -> ThgResult<Option<(crate::burn_pairformer::BurnPairformerConfig, Option<f32>)>> {
    let hits = store
        .read_neighbors(NeighborQuery {
            node_id: model_node_id.to_string(),
            direction: Direction::Out,
            edge_type: Some(EVALUATED_BY.to_string()),
            include_expired: false,
        })
        .map_err(thg_error_from_store)?;

    for hit in hits {
        let Some(node) = store
            .read_node(&hit.node_id)
            .map_err(thg_error_from_store)?
        else {
            continue;
        };
        let Some(metrics) = node.properties.get("metrics") else {
            continue;
        };
        let config = metrics
            .get("config")
            .cloned()
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default();
        let ranking_accuracy = metrics
            .get("final_ranking_accuracy")
            .and_then(Value::as_f64)
            .map(|value| value as f32);
        return Ok(Some((config, ranking_accuracy)));
    }
    Ok(None)
}

#[cfg(feature = "pairformer-burn-cubecl")]
fn promoted_pairformer_local_path(properties: &Value) -> Option<PathBuf> {
    for key in ["local_path", "artifact_path", "weights_path", "cache_path"] {
        if let Some(path) = property_string(properties, key).filter(|path| !path.is_empty()) {
            return Some(PathBuf::from(path));
        }
    }
    if let Some(file_uri) = property_string(properties, "s3_uri")
        .filter(|uri| uri.starts_with("file://"))
        .and_then(|uri| uri.strip_prefix("file://").map(str::to_string))
    {
        return Some(PathBuf::from(file_uri));
    }
    let s3_uri = property_string(properties, "s3_uri")?;
    let s3_key = s3_uri.strip_prefix("s3://")?;
    for env_key in ["THEOREM_MODEL_CACHE_DIR", "RUSTYRED_MODEL_CACHE_DIR"] {
        if let Ok(cache_root) = std::env::var(env_key) {
            let candidate = PathBuf::from(cache_root).join(s3_key);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
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

    let input = MatchNeighborhoodInput {
        nodes,
        edges: edges.into_values().collect(),
        representations,
        adapter_factors,
    };
    let (request, config, prepared) = prepare_match_neighborhood(&input, request, config)?;
    score_prepared_match_with_default_learning_model(store, &request, config, prepared)
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
