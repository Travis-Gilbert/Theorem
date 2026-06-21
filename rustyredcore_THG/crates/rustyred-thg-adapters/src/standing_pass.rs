//! Standing-pass organizer engine.
//!
//! The engine generalizes the existing reflexive candidate generators behind
//! one hook-facing contract. Generators propose advisory candidates over a
//! bounded region; admission is the only place that writes graph structure.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, HookContext,
    HookError, HookHandler, HookOutcome, HookRegistration, MutationEvent, MutationKind,
    MutationMatcher, NodeRecord, RedCoreGraphStore, ThgError, ThgResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::pairformer::{PairformerConfig, PairformerSupportPath};
use crate::reflexive::{
    quarantine_densification_candidates, rank_pairformer_densification_candidates,
    rank_spatial_candidates, rank_temporal_candidates, DensificationRequest, DensificationResult,
    InferredEdgeCandidate, REFLEXIVE_CANDIDATE_OF, REFLEXIVE_CANDIDATE_SOURCE,
    REFLEXIVE_CANDIDATE_TARGET, REFLEXIVE_DENSIFICATION_RUN_LABEL, REFLEXIVE_EDGE_CANDIDATE_LABEL,
    REFLEXIVE_PROPERTY_CANDIDATE_LABEL,
};
use crate::types::{thg_error_from_store, THG_ADAPTER_SOURCE};

pub const STANDING_PASS_ADMITTED_BY: &str = "standing_pass";
pub const DEFAULT_STANDING_PASS_MAX_NODES: usize = 128;
pub const DEFAULT_STANDING_PASS_MAX_DEPTH: usize = 2;
pub const DEFAULT_STANDING_PASS_MAX_CANDIDATES: usize = 64;
pub const DEFAULT_STANDING_PASS_CONFIDENCE_CEILING: f32 = 0.74;
pub const DEFAULT_STANDING_PASS_CONFIDENCE_THRESHOLD: f32 = 0.30;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GeneratorInput {
    pub nodes: Vec<NodeRecord>,
    pub edges: Vec<EdgeRecord>,
    pub query: GeneratorQuery,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GeneratorQuery {
    pub kind: GeneratorQueryKind,
    pub tenant_id: String,
    pub seed_node_ids: Vec<String>,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub min_path_confidence: f32,
    pub confidence_threshold: f32,
    pub confidence_ceiling: f32,
    pub max_candidates: usize,
    pub admission_tier: String,
    pub allowed_edge_types: Vec<String>,
}

impl GeneratorQuery {
    fn densification_request(&self, model_id: impl Into<String>) -> DensificationRequest {
        DensificationRequest {
            tenant_id: self.tenant_id.clone(),
            seed_node_ids: self.seed_node_ids.clone(),
            max_nodes: self.max_nodes,
            max_depth: self.max_depth,
            min_path_confidence: self.min_path_confidence,
            confidence_threshold: self.confidence_threshold,
            confidence_ceiling: self.confidence_ceiling,
            max_candidates: self.max_candidates,
            admission_tier: self.admission_tier.clone(),
            model_id: model_id.into(),
            allowed_edge_types: self.allowed_edge_types.clone(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GeneratorQueryKind {
    SeedNodes,
    CandidatePairs(Vec<CandidatePair>),
    Region(Vec<String>),
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
pub struct CandidatePair {
    pub source_id: String,
    pub target_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    ProposedEdge,
    EquivalenceMerge,
    DerivedFact,
    ReliabilityAnnotation,
    SalienceUpdate,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CandidateRef {
    EdgeProposal {
        source_id: String,
        target_id: String,
        edge_type: String,
    },
    Node {
        node_id: String,
    },
    Edge {
        edge_id: String,
    },
    Fact {
        fact_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "value")]
pub enum AdvisoryPayload {
    Edge(InferredEdgeCandidate),
    Json(Value),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AdvisoryCandidate {
    pub kind: CandidateKind,
    pub subject: CandidateRef,
    pub score: f32,
    pub support: Option<PairformerSupportPath>,
    pub generator_id: String,
    pub payload: AdvisoryPayload,
}

impl AdvisoryCandidate {
    pub fn from_edge(generator_id: impl Into<String>, candidate: InferredEdgeCandidate) -> Self {
        let generator_id = generator_id.into();
        let support = Some(PairformerSupportPath {
            edge_ids: candidate.support_path_edge_ids.clone(),
            node_ids: candidate.support_path_node_ids.clone(),
            relation_hint: candidate.proposed_edge_type.clone(),
            confidence: candidate.confidence,
        });
        Self {
            kind: CandidateKind::ProposedEdge,
            subject: CandidateRef::EdgeProposal {
                source_id: candidate.source_id.clone(),
                target_id: candidate.target_id.clone(),
                edge_type: candidate.proposed_edge_type.clone(),
            },
            score: candidate.confidence,
            support,
            generator_id,
            payload: AdvisoryPayload::Edge(candidate),
        }
    }

    pub fn edge_candidate(&self) -> Option<&InferredEdgeCandidate> {
        match &self.payload {
            AdvisoryPayload::Edge(candidate) => Some(candidate),
            AdvisoryPayload::Json(_) => None,
        }
    }

    fn dedupe_key(&self) -> (CandidateKind, CandidateRef) {
        (self.kind.clone(), self.subject.clone())
    }
}

pub trait StandingGenerator: Send + Sync + std::fmt::Debug {
    fn id(&self) -> &str;
    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>>;
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StandingPassConfig {
    pub tenant_id: String,
    pub max_nodes: usize,
    pub max_depth: usize,
    pub min_path_confidence: f32,
    pub confidence_threshold: f32,
    pub confidence_ceiling: f32,
    pub max_candidates: usize,
    pub admission_tier: String,
    pub allowed_edge_types: Vec<String>,
    pub pairformer_config: PairformerConfig,
    pub auto_apply_at_confidence_ceiling: bool,
}

impl Default for StandingPassConfig {
    fn default() -> Self {
        Self {
            tenant_id: "default".to_string(),
            max_nodes: DEFAULT_STANDING_PASS_MAX_NODES,
            max_depth: DEFAULT_STANDING_PASS_MAX_DEPTH,
            min_path_confidence: 0.0,
            confidence_threshold: DEFAULT_STANDING_PASS_CONFIDENCE_THRESHOLD,
            confidence_ceiling: DEFAULT_STANDING_PASS_CONFIDENCE_CEILING,
            max_candidates: DEFAULT_STANDING_PASS_MAX_CANDIDATES,
            admission_tier: "advisory_inferred".to_string(),
            allowed_edge_types: Vec::new(),
            pairformer_config: PairformerConfig::default(),
            auto_apply_at_confidence_ceiling: true,
        }
    }
}

impl StandingPassConfig {
    pub fn normalized(mut self) -> Self {
        self.tenant_id = self.tenant_id.trim().to_string();
        if self.tenant_id.is_empty() {
            self.tenant_id = "default".to_string();
        }
        if self.max_nodes == 0 {
            self.max_nodes = DEFAULT_STANDING_PASS_MAX_NODES;
        }
        if self.max_depth == 0 {
            self.max_depth = DEFAULT_STANDING_PASS_MAX_DEPTH;
        }
        if self.max_candidates == 0 {
            self.max_candidates = DEFAULT_STANDING_PASS_MAX_CANDIDATES;
        }
        self.min_path_confidence = self.min_path_confidence.clamp(0.0, 1.0);
        self.confidence_threshold = self.confidence_threshold.clamp(0.0, 1.0);
        if self.confidence_ceiling <= 0.0 || !self.confidence_ceiling.is_finite() {
            self.confidence_ceiling = DEFAULT_STANDING_PASS_CONFIDENCE_CEILING;
        }
        self.confidence_ceiling = self.confidence_ceiling.clamp(0.0, 1.0);
        self.admission_tier = self.admission_tier.trim().to_string();
        if self.admission_tier.is_empty() {
            self.admission_tier = "advisory_inferred".to_string();
        }
        self.allowed_edge_types = self
            .allowed_edge_types
            .into_iter()
            .map(|edge_type| edge_type.trim().to_string())
            .filter(|edge_type| !edge_type.is_empty())
            .collect();
        self.pairformer_config = self.pairformer_config.normalized();
        self
    }

    fn generator_query(&self, seed_node_ids: Vec<String>) -> GeneratorQuery {
        GeneratorQuery {
            kind: GeneratorQueryKind::SeedNodes,
            tenant_id: self.tenant_id.clone(),
            seed_node_ids,
            max_nodes: self.max_nodes,
            max_depth: self.max_depth,
            min_path_confidence: self.min_path_confidence,
            confidence_threshold: self.confidence_threshold,
            confidence_ceiling: self.confidence_ceiling,
            max_candidates: self.max_candidates,
            admission_tier: self.admission_tier.clone(),
            allowed_edge_types: self.allowed_edge_types.clone(),
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct StandingPassResult {
    pub tenant_id: String,
    pub run_id: String,
    pub seed_node_ids: Vec<String>,
    pub considered_node_ids: Vec<String>,
    pub generator_ids: Vec<String>,
    pub candidates: Vec<AdvisoryCandidate>,
    pub candidate_node_ids: Vec<String>,
    pub applied_edge_ids: Vec<String>,
    pub writes: usize,
}

#[derive(Debug)]
pub struct StandingPassEngine {
    config: StandingPassConfig,
    generators: Vec<Arc<dyn StandingGenerator>>,
}

impl StandingPassEngine {
    pub fn new(
        config: StandingPassConfig,
        generators: Vec<Arc<dyn StandingGenerator>>,
    ) -> ThgResult<Self> {
        let config = config.normalized();
        if generators.is_empty() {
            return Err(ThgError::new(
                "standing_pass_requires_generator",
                "at least one standing generator is required",
            ));
        }
        Ok(Self { config, generators })
    }

    pub fn with_default_generators(config: StandingPassConfig) -> ThgResult<Self> {
        let config = config.normalized();
        Self::new(config.clone(), default_standing_generators(&config))
    }

    pub fn run<S: crate::types::AdapterGraphStore>(
        &self,
        store: &mut S,
        seed_node_ids: Vec<String>,
    ) -> ThgResult<StandingPassResult> {
        let seed_node_ids = normalize_seed_node_ids(seed_node_ids);
        if seed_node_ids.is_empty() {
            return Ok(StandingPassResult {
                tenant_id: self.config.tenant_id.clone(),
                ..StandingPassResult::default()
            });
        }

        let snapshot = store.snapshot().map_err(thg_error_from_store)?;
        let input = select_generator_input(&snapshot, &self.config, seed_node_ids.clone());
        let mut by_key: BTreeMap<(CandidateKind, CandidateRef), AdvisoryCandidate> =
            BTreeMap::new();
        let mut generator_ids = Vec::with_capacity(self.generators.len());
        for generator in &self.generators {
            generator_ids.push(generator.id().to_string());
            for candidate in generator.generate(&input)? {
                if candidate.score < self.config.confidence_threshold {
                    continue;
                }
                let key = candidate.dedupe_key();
                match by_key.get(&key) {
                    Some(prior) if prior.score >= candidate.score => {}
                    _ => {
                        by_key.insert(key, candidate);
                    }
                }
            }
        }

        let mut candidates = by_key.into_values().collect::<Vec<_>>();
        candidates.sort_by(|left, right| {
            right
                .score
                .partial_cmp(&left.score)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.generator_id.cmp(&right.generator_id))
                .then_with(|| left.subject.cmp(&right.subject))
        });
        candidates.truncate(self.config.max_candidates);

        let run_id = standing_pass_run_id(&self.config.tenant_id, &seed_node_ids, &candidates);
        let edge_candidates = candidates
            .iter()
            .filter_map(|candidate| candidate.edge_candidate().cloned())
            .collect::<Vec<_>>();
        let quarantine = if edge_candidates.is_empty() {
            None
        } else {
            Some(quarantine_densification_candidates(
                store,
                &self.config.tenant_id,
                &run_id,
                &edge_candidates,
                Some(STANDING_PASS_ADMITTED_BY),
            )?)
        };

        let applied_edge_ids = admit_edge_candidates(store, &self.config, &candidates)?;
        let candidate_node_ids = quarantine
            .as_ref()
            .map(|result| result.candidate_node_ids.clone())
            .unwrap_or_default();
        let writes = candidate_node_ids.len() + applied_edge_ids.len();

        Ok(StandingPassResult {
            tenant_id: self.config.tenant_id.clone(),
            run_id,
            seed_node_ids,
            considered_node_ids: input.nodes.iter().map(|node| node.id.clone()).collect(),
            generator_ids,
            candidates,
            candidate_node_ids,
            applied_edge_ids,
            writes,
        })
    }

    fn run_hook(
        &self,
        store: &mut RedCoreGraphStore,
        events: &[MutationEvent],
    ) -> Result<HookOutcome, HookError> {
        let seeds = seed_node_ids_from_events(store, events)?;
        if seeds.is_empty() {
            return Ok(HookOutcome::Done);
        }
        let result = self
            .run(store, seeds)
            .map_err(|error| HookError::new(format!("{}: {}", error.code, error.message)))?;
        if result.writes == 0 {
            Ok(HookOutcome::Done)
        } else {
            Ok(HookOutcome::Wrote {
                mutations: result.writes,
            })
        }
    }
}

#[derive(Clone, Debug)]
pub struct PairformerStandingGenerator {
    id: String,
    config: PairformerConfig,
}

impl PairformerStandingGenerator {
    pub fn new(config: PairformerConfig) -> Self {
        Self {
            id: "pairformer-structural/default".to_string(),
            config: config.normalized(),
        }
    }
}

impl StandingGenerator for PairformerStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let result = rank_pairformer_densification_candidates(
            &snapshot_from_input(input),
            input.query.densification_request(self.id()),
            self.config.clone(),
        )?;
        Ok(edge_result_to_advisories(self.id(), result))
    }
}

#[derive(Clone, Debug)]
pub struct SpatialStandingGenerator {
    id: String,
}

impl Default for SpatialStandingGenerator {
    fn default() -> Self {
        Self {
            id: "rule-spatial/proximity-v0".to_string(),
        }
    }
}

impl StandingGenerator for SpatialStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let result = rank_spatial_candidates(
            &snapshot_from_input(input),
            input.query.densification_request(self.id()),
        )?;
        Ok(edge_result_to_advisories(self.id(), result))
    }
}

#[derive(Clone, Debug)]
pub struct HotTemporalStandingGenerator {
    id: String,
}

impl Default for HotTemporalStandingGenerator {
    fn default() -> Self {
        Self {
            // SPEC-8's trained HOT scorer is not in this repo yet. This ID keeps
            // the temporal slot explicit while using the existing temporal
            // relation scorer behind the same standing-generator contract.
            id: "hot-temporal/heuristic-v0".to_string(),
        }
    }
}

impl StandingGenerator for HotTemporalStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let result = rank_temporal_candidates(
            &snapshot_from_input(input),
            input.query.densification_request(self.id()),
        )?;
        Ok(edge_result_to_advisories(self.id(), result))
    }
}

pub fn default_standing_generators(config: &StandingPassConfig) -> Vec<Arc<dyn StandingGenerator>> {
    vec![
        Arc::new(PairformerStandingGenerator::new(
            config.pairformer_config.clone(),
        )),
        Arc::new(SpatialStandingGenerator::default()),
        Arc::new(HotTemporalStandingGenerator::default()),
    ]
}

pub fn standing_pass_hook(config: StandingPassConfig) -> ThgResult<HookRegistration> {
    let engine = Arc::new(StandingPassEngine::with_default_generators(config)?);
    Ok(standing_pass_hook_with_engine(engine))
}

pub fn standing_pass_hook_with_engine(engine: Arc<StandingPassEngine>) -> HookRegistration {
    let handler: HookHandler = Arc::new(move |ctx: &mut HookContext, events: &[MutationEvent]| {
        engine.run_hook(ctx.store, events)
    });
    HookRegistration::new(
        "reflexive.standing_pass",
        MutationMatcher::any().with_kinds([MutationKind::NodeUpserted, MutationKind::EdgeUpserted]),
        coalesce_standing_pass,
        handler,
    )
}

fn coalesce_standing_pass(event: &MutationEvent) -> Option<String> {
    Some(format!("standing-pass:{}", event.tenant))
}

fn edge_result_to_advisories(
    generator_id: &str,
    result: DensificationResult,
) -> Vec<AdvisoryCandidate> {
    result
        .candidates
        .into_iter()
        .map(|candidate| AdvisoryCandidate::from_edge(generator_id, candidate))
        .collect()
}

fn snapshot_from_input(input: &GeneratorInput) -> GraphSnapshot {
    GraphSnapshot {
        version: 0,
        nodes: input.nodes.clone(),
        edges: input.edges.clone(),
    }
}

fn select_generator_input(
    snapshot: &GraphSnapshot,
    config: &StandingPassConfig,
    seed_node_ids: Vec<String>,
) -> GeneratorInput {
    let live_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| (node.id.clone(), node.clone()))
        .collect::<BTreeMap<_, _>>();
    let live_edges = snapshot
        .edges
        .iter()
        .filter(|edge| !edge.tombstone)
        .cloned()
        .collect::<Vec<_>>();
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    for edge in &live_edges {
        if !live_nodes.contains_key(&edge.from_id) || !live_nodes.contains_key(&edge.to_id) {
            continue;
        }
        adjacency
            .entry(edge.from_id.clone())
            .or_default()
            .insert(edge.to_id.clone());
        adjacency
            .entry(edge.to_id.clone())
            .or_default()
            .insert(edge.from_id.clone());
    }

    let mut visited = BTreeSet::new();
    let mut queue = VecDeque::new();
    for seed in &seed_node_ids {
        if live_nodes.contains_key(seed) && visited.insert(seed.clone()) {
            queue.push_back((seed.clone(), 0usize));
        }
    }
    while let Some((node_id, depth)) = queue.pop_front() {
        if depth >= config.max_depth {
            continue;
        }
        let Some(neighbors) = adjacency.get(&node_id) else {
            continue;
        };
        for neighbor in neighbors {
            if visited.len() >= config.max_nodes {
                break;
            }
            if visited.insert(neighbor.clone()) {
                queue.push_back((neighbor.clone(), depth + 1));
            }
        }
    }

    let nodes = visited
        .iter()
        .filter_map(|node_id| live_nodes.get(node_id).cloned())
        .collect::<Vec<_>>();
    let node_ids = nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let edges = live_edges
        .into_iter()
        .filter(|edge| node_ids.contains(&edge.from_id) && node_ids.contains(&edge.to_id))
        .collect::<Vec<_>>();

    GeneratorInput {
        nodes,
        edges,
        query: config.generator_query(seed_node_ids),
    }
}

fn normalize_seed_node_ids(seed_node_ids: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    seed_node_ids
        .into_iter()
        .map(|seed| seed.trim().to_string())
        .filter(|seed| !seed.is_empty())
        .filter(|seed| seen.insert(seed.clone()))
        .collect()
}

fn seed_node_ids_from_events(
    store: &RedCoreGraphStore,
    events: &[MutationEvent],
) -> Result<Vec<String>, HookError> {
    let mut seeds = BTreeSet::new();
    for event in events {
        if internal_event(event) {
            continue;
        }
        match event.kind {
            MutationKind::NodeUpserted => {
                seeds.insert(event.id.clone());
            }
            MutationKind::EdgeUpserted => {
                let Some(edge) = store.get_edge(&event.id).map_err(HookError::from)? else {
                    continue;
                };
                if internal_edge(&edge) {
                    continue;
                }
                seeds.insert(edge.from_id);
                seeds.insert(edge.to_id);
            }
            MutationKind::NodeDeleted | MutationKind::EdgeDeleted => {}
        }
    }
    Ok(seeds.into_iter().collect())
}

fn internal_event(event: &MutationEvent) -> bool {
    event.labels.iter().any(|label| {
        matches!(
            label.as_str(),
            REFLEXIVE_DENSIFICATION_RUN_LABEL
                | REFLEXIVE_EDGE_CANDIDATE_LABEL
                | REFLEXIVE_PROPERTY_CANDIDATE_LABEL
                | REFLEXIVE_CANDIDATE_OF
                | REFLEXIVE_CANDIDATE_SOURCE
                | REFLEXIVE_CANDIDATE_TARGET
        )
    })
}

fn internal_edge(edge: &EdgeRecord) -> bool {
    if edge.properties.get("admitted_by").and_then(Value::as_str) == Some(STANDING_PASS_ADMITTED_BY)
    {
        return true;
    }
    matches!(
        edge.edge_type.as_str(),
        REFLEXIVE_CANDIDATE_OF | REFLEXIVE_CANDIDATE_SOURCE | REFLEXIVE_CANDIDATE_TARGET
    )
}

fn admit_edge_candidates<S: crate::types::AdapterGraphStore>(
    store: &mut S,
    config: &StandingPassConfig,
    candidates: &[AdvisoryCandidate],
) -> ThgResult<Vec<String>> {
    if !config.auto_apply_at_confidence_ceiling {
        return Ok(Vec::new());
    }
    let mut mutations = Vec::new();
    let mut applied = Vec::new();
    for candidate in candidates {
        let Some(edge_candidate) = candidate.edge_candidate() else {
            continue;
        };
        if edge_candidate.confidence < edge_candidate.confidence_ceiling {
            continue;
        }
        let edge_id = admitted_edge_id(edge_candidate);
        if store
            .get_edge(&edge_id)
            .map_err(thg_error_from_store)?
            .is_some()
        {
            continue;
        }
        mutations.push(GraphMutation::EdgeUpsert(
            EdgeRecord::new(
                &edge_id,
                &edge_candidate.source_id,
                &edge_candidate.proposed_edge_type,
                &edge_candidate.target_id,
                json!({
                    "tenant_id": edge_candidate.tenant_id,
                    "candidate_id": edge_candidate.candidate_id,
                    "generator_id": candidate.generator_id,
                    "model_id": edge_candidate.model_id,
                    "confidence": edge_candidate.confidence,
                    "confidence_ceiling": edge_candidate.confidence_ceiling,
                    "admission_tier": edge_candidate.admission_tier,
                    "support_path_edge_ids": edge_candidate.support_path_edge_ids,
                    "support_path_node_ids": edge_candidate.support_path_node_ids,
                    "inferred": true,
                    "admitted_by": STANDING_PASS_ADMITTED_BY,
                    "source": THG_ADAPTER_SOURCE,
                }),
            )
            .with_confidence(edge_candidate.confidence as f64),
        ));
        applied.push(edge_id);
    }
    if !mutations.is_empty() {
        store
            .commit_batch(GraphMutationBatch::new(mutations))
            .map_err(thg_error_from_store)?;
    }
    Ok(applied)
}

pub fn admitted_edge_id(candidate: &InferredEdgeCandidate) -> String {
    format!(
        "edge:{}:{}:{}",
        candidate.source_id, candidate.proposed_edge_type, candidate.target_id
    )
}

pub fn standing_pass_run_id(
    tenant_id: &str,
    seed_node_ids: &[String],
    candidates: &[AdvisoryCandidate],
) -> String {
    stable_hash(json!({
        "tenant_id": tenant_id,
        "seed_node_ids": seed_node_ids,
        "candidate_subjects": candidates
            .iter()
            .map(|candidate| json!({
                "kind": &candidate.kind,
                "subject": &candidate.subject,
                "score": candidate.score,
                "generator_id": candidate.generator_id,
            }))
            .collect::<Vec<_>>(),
    }))
}
