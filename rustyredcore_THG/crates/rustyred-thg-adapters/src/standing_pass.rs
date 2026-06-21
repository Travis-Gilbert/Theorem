//! Standing-pass organizer engine.
//!
//! The engine generalizes the existing reflexive candidate generators behind
//! one hook-facing contract. Generators propose advisory candidates over a
//! bounded region; admission is the only place that writes graph structure.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

use rustyred_thg_core::{
    probabilistic_source_reliability, stable_hash, EdgeRecord, GraphMutation, GraphMutationBatch,
    GraphSnapshot, HookContext, HookError, HookHandler, HookOutcome, HookRegistration,
    MutationEvent, MutationKind, MutationMatcher, NodeRecord, RedCoreGraphStore, ThgError,
    ThgResult,
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
pub const DATALOG_STANDING_GENERATOR_ID: &str = "symbolic-datalog/core-rules-v0";
pub const EGGLOG_EQUIVALENCE_STANDING_GENERATOR_ID: &str = "symbolic-egglog/equivalence-v0";
pub const SOURCE_RELIABILITY_STANDING_GENERATOR_ID: &str =
    "symbolic-beta-bernoulli/source-reliability-v0";

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

#[derive(Clone, Debug)]
pub struct DatalogStandingGenerator {
    id: String,
    rule_ids: Vec<String>,
}

impl Default for DatalogStandingGenerator {
    fn default() -> Self {
        Self {
            id: DATALOG_STANDING_GENERATOR_ID.to_string(),
            rule_ids: rustyred_thg_core::symbolic::DATALOG_RULE_IDS
                .iter()
                .map(|rule_id| (*rule_id).to_string())
                .collect(),
        }
    }
}

impl DatalogStandingGenerator {
    pub fn with_rule_ids(rule_ids: impl IntoIterator<Item = impl Into<String>>) -> Self {
        let rule_ids = rule_ids
            .into_iter()
            .map(Into::into)
            .filter(|rule_id: &String| !rule_id.trim().is_empty())
            .collect();
        Self {
            rule_ids,
            ..Self::default()
        }
    }
}

impl StandingGenerator for DatalogStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let (facts, fact_sources) = symbolic_facts_from_input(input);
        if facts.is_empty() {
            return Ok(Vec::new());
        }
        let receipt = rustyred_thg_core::derive_datalog_receipt(&json!({
            "facts": facts,
            "rule_ids": self.rule_ids,
        }))
        .map_err(symbolic_generator_error)?;
        let fact_pack_hash = receipt
            .get("fact_pack_hash")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let derived_facts = receipt
            .get("derived_facts")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        Ok(derived_facts
            .iter()
            .filter_map(|fact| {
                derived_fact_to_advisory(self.id(), fact, &fact_pack_hash, &fact_sources, input)
            })
            .collect())
    }
}

#[derive(Clone, Debug)]
pub struct EgglogEquivalenceStandingGenerator {
    id: String,
}

impl Default for EgglogEquivalenceStandingGenerator {
    fn default() -> Self {
        Self {
            id: EGGLOG_EQUIVALENCE_STANDING_GENERATOR_ID.to_string(),
        }
    }
}

impl StandingGenerator for EgglogEquivalenceStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let mut by_title: BTreeMap<String, Vec<&NodeRecord>> = BTreeMap::new();
        for node in &input.nodes {
            let title = symbolic_title(node);
            let normalized = normalize_symbolic_title(&title);
            if normalized.is_empty() {
                continue;
            }
            by_title.entry(normalized).or_default().push(node);
        }

        let mut candidates = Vec::new();
        for (canonical_title, mut nodes) in by_title {
            if nodes.len() < 2 {
                continue;
            }
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
            let member_node_ids = nodes.iter().map(|node| node.id.clone()).collect::<Vec<_>>();
            let representative_node_id = member_node_ids[0].clone();
            let class_id = format!(
                "eclass:{}",
                stable_hash(json!({
                    "generator_id": self.id(),
                    "canonical_title": canonical_title,
                    "member_node_ids": member_node_ids,
                }))
            );
            let score = 0.70_f32;
            candidates.push(AdvisoryCandidate {
                kind: CandidateKind::EquivalenceMerge,
                subject: CandidateRef::Fact {
                    fact_id: class_id.clone(),
                },
                score,
                support: Some(PairformerSupportPath {
                    edge_ids: Vec::new(),
                    node_ids: member_node_ids.clone(),
                    relation_hint: "same_normalized_title".to_string(),
                    confidence: score,
                }),
                generator_id: self.id().to_string(),
                payload: AdvisoryPayload::Json(json!({
                    "candidate_id": class_id,
                    "relation": "equivalence_merge",
                    "canonical_title": canonical_title,
                    "representative_node_id": representative_node_id,
                    "member_node_ids": member_node_ids,
                    "engine": self.id(),
                    "backend": "egglog-compatible-normalized-title",
                    "upstream_ref": "egraphs-good/egglog@5294cdc",
                    "upstream_api": "EGraph::parse_and_run_program",
                    "rewrite_trace": ["normalize_title", "union_same_title"],
                    "writeback_policy": "proposal-only",
                })),
            });
        }
        Ok(candidates)
    }
}

#[derive(Clone, Debug)]
pub struct SourceReliabilityStandingGenerator {
    id: String,
    prior_alpha: f64,
    prior_beta: f64,
}

impl Default for SourceReliabilityStandingGenerator {
    fn default() -> Self {
        Self {
            id: SOURCE_RELIABILITY_STANDING_GENERATOR_ID.to_string(),
            prior_alpha: 1.0,
            prior_beta: 1.0,
        }
    }
}

impl StandingGenerator for SourceReliabilityStandingGenerator {
    fn id(&self) -> &str {
        &self.id
    }

    fn generate(&self, input: &GeneratorInput) -> ThgResult<Vec<AdvisoryCandidate>> {
        let observations = source_observations_from_input(input);
        let mut candidates = Vec::new();
        for (source_id, observations) in observations {
            let corroborated = observations
                .iter()
                .filter(|observation| observation.corroborated)
                .count();
            let contradicted = observations.len().saturating_sub(corroborated);
            let receipt = probabilistic_source_reliability(&json!({
                "source_id": source_id,
                "prior_alpha": self.prior_alpha,
                "prior_beta": self.prior_beta,
                "corroborated": corroborated,
                "contradicted": contradicted,
            }))
            .map_err(symbolic_generator_error)?;
            let score = receipt
                .get("posterior")
                .and_then(|posterior| posterior.get("mean"))
                .and_then(Value::as_f64)
                .unwrap_or(0.5) as f32;
            let receipt_hash = receipt
                .get("receipt_hash")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let support_edge_ids = observations
                .iter()
                .filter_map(|observation| observation.edge_id.clone())
                .collect::<Vec<_>>();
            let support_node_ids = observations
                .iter()
                .filter_map(|observation| observation.node_id.clone())
                .collect::<Vec<_>>();
            candidates.push(AdvisoryCandidate {
                kind: CandidateKind::ReliabilityAnnotation,
                subject: CandidateRef::Fact {
                    fact_id: format!("source-reliability:{source_id}"),
                },
                score,
                support: Some(PairformerSupportPath {
                    edge_ids: support_edge_ids,
                    node_ids: support_node_ids,
                    relation_hint: "source_reliability".to_string(),
                    confidence: score,
                }),
                generator_id: self.id().to_string(),
                payload: AdvisoryPayload::Json(json!({
                    "candidate_id": receipt_hash,
                    "relation": "source_reliability",
                    "source_id": source_id,
                    "receipt": receipt,
                    "corroborated": corroborated,
                    "contradicted": contradicted,
                    "writeback_policy": "read-only",
                })),
            });
        }
        Ok(candidates)
    }
}

pub fn default_standing_generators(config: &StandingPassConfig) -> Vec<Arc<dyn StandingGenerator>> {
    vec![
        Arc::new(PairformerStandingGenerator::new(
            config.pairformer_config.clone(),
        )),
        Arc::new(SpatialStandingGenerator::default()),
        Arc::new(HotTemporalStandingGenerator::default()),
        Arc::new(DatalogStandingGenerator::default()),
        Arc::new(EgglogEquivalenceStandingGenerator::default()),
        Arc::new(SourceReliabilityStandingGenerator::default()),
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

fn symbolic_generator_error(message: String) -> ThgError {
    ThgError::new("standing_pass_symbolic_generator_error", message)
}

fn symbolic_facts_from_input(input: &GeneratorInput) -> (Vec<Value>, BTreeMap<String, String>) {
    let claim_ids = input
        .nodes
        .iter()
        .filter(|node| symbolic_is_claim(node))
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut facts = Vec::new();
    let mut fact_sources = BTreeMap::new();

    for node in &input.nodes {
        let object_fact_id = format!("fact:node:{}:object", node.id);
        facts.push(json!({
            "fact_id": object_fact_id,
            "relation": "object",
            "entity_id": node.id,
            "attributes": symbolic_node_attributes(node),
            "source_ref": node.id,
        }));
        fact_sources.insert(format!("fact:node:{}:object", node.id), node.id.clone());

        if symbolic_is_claim(node) {
            let claim_fact_id = format!("fact:node:{}:claim", node.id);
            facts.push(json!({
                "fact_id": claim_fact_id,
                "relation": "claim",
                "entity_id": node.id,
                "attributes": symbolic_claim_attributes(node),
                "source_ref": node.id,
            }));
            fact_sources.insert(format!("fact:node:{}:claim", node.id), node.id.clone());
        }
    }

    for edge in &input.edges {
        let relation_type = symbolic_relation_type(edge);
        let claim_id = if claim_ids.contains(&edge.to_id) {
            Some(edge.to_id.clone())
        } else if claim_ids.contains(&edge.from_id) {
            Some(edge.from_id.clone())
        } else {
            None
        };
        let Some(claim_id) = claim_id else {
            continue;
        };
        let other_id = if claim_id == edge.from_id {
            edge.to_id.clone()
        } else {
            edge.from_id.clone()
        };
        if symbolic_support_relation(&relation_type) {
            let fact_id = format!("fact:edge:{}:evidence_link", edge.id);
            facts.push(json!({
                "fact_id": fact_id,
                "relation": "evidence_link",
                "entity_id": edge.id,
                "attributes": {
                    "claim_id": claim_id,
                    "artifact_id": other_id,
                    "relation_type": relation_type,
                    "strength": edge.confidence.unwrap_or(1.0),
                    "source_id": source_id_from_value(&edge.properties)
                        .or_else(|| edge.provenance.as_ref().and_then(|p| p.source_id.clone()))
                        .unwrap_or_default(),
                },
                "source_ref": edge.id,
            }));
            fact_sources.insert(
                format!("fact:edge:{}:evidence_link", edge.id),
                edge.id.clone(),
            );
        }
        let fact_id = format!("fact:edge:{}:claim_dependency", edge.id);
        facts.push(json!({
            "fact_id": fact_id,
            "relation": "claim_dependency",
            "entity_id": edge.id,
            "attributes": {
                "claim_id": claim_id,
                "depends_on_object_id": other_id,
                "justification_type": relation_type,
                "strength": edge.confidence.unwrap_or(1.0),
            },
            "source_ref": edge.id,
        }));
        fact_sources.insert(
            format!("fact:edge:{}:claim_dependency", edge.id),
            edge.id.clone(),
        );
    }

    (facts, fact_sources)
}

fn derived_fact_to_advisory(
    generator_id: &str,
    fact: &Value,
    fact_pack_hash: &str,
    fact_sources: &BTreeMap<String, String>,
    input: &GeneratorInput,
) -> Option<AdvisoryCandidate> {
    let fact_id = value_string(fact.get("fact_id"))?;
    let rule_id = value_string(fact.get("rule_id")).unwrap_or_default();
    let score = fact
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(1.0)
        .clamp(0.0, 1.0) as f32;
    let dependency_fact_ids = fact
        .get("dependency_fact_ids")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let edge_ids = input
        .edges
        .iter()
        .map(|edge| edge.id.clone())
        .collect::<BTreeSet<_>>();
    let node_ids = input
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let mut support_edge_ids = BTreeSet::new();
    let mut support_node_ids = BTreeSet::new();
    for dep in &dependency_fact_ids {
        let Some(dep_id) = dep.as_str() else {
            continue;
        };
        let Some(source_ref) = fact_sources.get(dep_id) else {
            continue;
        };
        if edge_ids.contains(source_ref) {
            support_edge_ids.insert(source_ref.clone());
        }
        if node_ids.contains(source_ref) {
            support_node_ids.insert(source_ref.clone());
        }
    }
    Some(AdvisoryCandidate {
        kind: CandidateKind::DerivedFact,
        subject: CandidateRef::Fact {
            fact_id: fact_id.clone(),
        },
        score,
        support: Some(PairformerSupportPath {
            edge_ids: support_edge_ids.into_iter().collect(),
            node_ids: support_node_ids.into_iter().collect(),
            relation_hint: rule_id,
            confidence: score,
        }),
        generator_id: generator_id.to_string(),
        payload: AdvisoryPayload::Json(json!({
            "candidate_id": fact_id,
            "relation": "derived_fact",
            "fact_pack_hash": fact_pack_hash,
            "derived_fact": fact,
        })),
    })
}

#[derive(Clone, Debug)]
struct SourceObservation {
    node_id: Option<String>,
    edge_id: Option<String>,
    corroborated: bool,
}

fn source_observations_from_input(
    input: &GeneratorInput,
) -> BTreeMap<String, Vec<SourceObservation>> {
    let mut observations: BTreeMap<String, Vec<SourceObservation>> = BTreeMap::new();
    for node in &input.nodes {
        let Some(source_id) = source_id_from_value(&node.properties) else {
            continue;
        };
        observations
            .entry(source_id)
            .or_default()
            .push(SourceObservation {
                node_id: Some(node.id.clone()),
                edge_id: None,
                corroborated: !symbolic_value_is_contradictory(&node.properties),
            });
    }
    for edge in &input.edges {
        let Some(source_id) = source_id_from_value(&edge.properties).or_else(|| {
            edge.provenance
                .as_ref()
                .and_then(|provenance| provenance.source_id.clone())
        }) else {
            continue;
        };
        observations
            .entry(source_id)
            .or_default()
            .push(SourceObservation {
                node_id: None,
                edge_id: Some(edge.id.clone()),
                corroborated: !symbolic_edge_is_contradictory(edge),
            });
    }
    observations
}

fn symbolic_node_attributes(node: &NodeRecord) -> Value {
    let mut attributes = node.properties.as_object().cloned().unwrap_or_default();
    attributes.insert("labels".to_string(), json!(node.labels));
    let title = symbolic_title(node);
    if !title.is_empty() {
        attributes.insert("title".to_string(), json!(title));
    }
    if let Some(source_id) = source_id_from_value(&Value::Object(attributes.clone())) {
        attributes.insert("source_id".to_string(), json!(source_id));
    }
    Value::Object(attributes)
}

fn symbolic_claim_attributes(node: &NodeRecord) -> Value {
    let mut attributes = symbolic_node_attributes(node)
        .as_object()
        .cloned()
        .unwrap_or_default();
    if let Some(claim_text) = first_string_property(
        &node.properties,
        &["claim_text", "claim", "text", "title", "name", "label"],
    ) {
        attributes.insert("claim_text".to_string(), json!(claim_text));
    }
    Value::Object(attributes)
}

fn symbolic_is_claim(node: &NodeRecord) -> bool {
    node.labels
        .iter()
        .any(|label| label.eq_ignore_ascii_case("claim"))
        || first_string_property(&node.properties, &["claim_text", "claim"]).is_some()
}

fn symbolic_title(node: &NodeRecord) -> String {
    first_string_property(
        &node.properties,
        &["title", "name", "label", "claim_text", "text"],
    )
    .unwrap_or_else(|| node.id.clone())
}

fn first_string_property(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| value.get(*key).and_then(value_as_nonempty_string))
        .next()
}

fn source_id_from_value(value: &Value) -> Option<String> {
    first_string_property(value, &["source_id", "source_ref", "source"])
}

fn value_as_nonempty_string(value: &Value) -> Option<String> {
    let string = match value {
        Value::String(value) => value.trim().to_string(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Null | Value::Array(_) | Value::Object(_) => return None,
    };
    (!string.is_empty()).then_some(string)
}

fn value_string(value: Option<&Value>) -> Option<String> {
    value.and_then(value_as_nonempty_string)
}

fn symbolic_relation_type(edge: &EdgeRecord) -> String {
    edge.properties
        .get("relation_type")
        .and_then(value_as_nonempty_string)
        .unwrap_or_else(|| edge.edge_type.to_lowercase())
}

fn symbolic_support_relation(relation_type: &str) -> bool {
    matches!(
        relation_type.to_ascii_lowercase().as_str(),
        "supports" | "derived_from" | "derives" | "cites" | "references" | "evidence"
    )
}

fn symbolic_value_is_contradictory(value: &Value) -> bool {
    ["status", "verdict", "relation_type", "epistemic_type"]
        .iter()
        .filter_map(|key| value.get(*key).and_then(value_as_nonempty_string))
        .any(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "contradicted" | "contradicts" | "refuted" | "false" | "tension"
            )
        })
}

fn symbolic_edge_is_contradictory(edge: &EdgeRecord) -> bool {
    if edge.epistemic_type.as_ref().is_some_and(|kind| {
        matches!(
            kind,
            rustyred_thg_core::EpistemicType::Contradicts
                | rustyred_thg_core::EpistemicType::Tension
        )
    }) {
        return true;
    }
    symbolic_value_is_contradictory(&edge.properties)
        || matches!(
            edge.edge_type.to_ascii_lowercase().as_str(),
            "contradicts" | "refutes" | "tension"
        )
}

fn normalize_symbolic_title(value: &str) -> String {
    let mut cleaned = String::with_capacity(value.len());
    for ch in value.to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            cleaned.push(ch);
        } else {
            cleaned.push(' ');
        }
    }
    cleaned.split_whitespace().collect::<Vec<_>>().join(" ")
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
