//! Compute-offload planner for Theorem.
//!
//! The core contract is small: decompose work into operations, consider cache,
//! CPU affordance, cheap model, and expensive model candidates, then choose the
//! cheapest executor that satisfies the declared quality floor. The trace is
//! replayable and carries the six axes from the reconciled plan: symbolic
//! offload, predicate pushdown, operator fusion, calibrated model cascade,
//! computation reuse, and verification offload.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use rustyred_thg_core::{
    label_propagation_communities, pagerank, paths_shortest_weighted, stable_hash, GraphSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const COMPUTE_OFFLOAD_FAMILY: &str = "compute_offload";
pub const COMPUTE_OFFLOAD_ROUTE_AFFORDANCE_ID: &str = "compute_offload.route_operation";
pub const COMPUTE_OFFLOAD_ENGINE_ID: &str = "rustyred-thg-offload";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OffloadAxis {
    SymbolicOffload,
    PredicatePushdown,
    OperatorFusion,
    ModelCascade,
    ComputationReuse,
    VerificationOffload,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    PredicateFilter,
    DatalogDerivation,
    ProbabilisticReliability,
    ConstraintCheck,
    GraphPageRank,
    GraphCommunity,
    GraphShortestPath,
    NeuralSynthesis,
    VerificationCheck,
    CodeSynthesisSurrogate,
}

impl OperationKind {
    pub fn primary_axis(&self) -> OffloadAxis {
        match self {
            Self::PredicateFilter => OffloadAxis::PredicatePushdown,
            Self::NeuralSynthesis => OffloadAxis::ModelCascade,
            Self::VerificationCheck => OffloadAxis::VerificationOffload,
            Self::CodeSynthesisSurrogate => OffloadAxis::ComputationReuse,
            Self::DatalogDerivation
            | Self::ProbabilisticReliability
            | Self::ConstraintCheck
            | Self::GraphPageRank
            | Self::GraphCommunity
            | Self::GraphShortestPath => OffloadAxis::SymbolicOffload,
        }
    }

    fn is_filter(&self) -> bool {
        matches!(self, Self::PredicateFilter)
    }

    fn is_model_heavy(&self) -> bool {
        matches!(self, Self::NeuralSynthesis)
    }

    fn is_cpu_symbolic(&self) -> bool {
        matches!(
            self,
            Self::PredicateFilter
                | Self::DatalogDerivation
                | Self::ProbabilisticReliability
                | Self::ConstraintCheck
                | Self::GraphPageRank
                | Self::GraphCommunity
                | Self::GraphShortestPath
                | Self::VerificationCheck
                | Self::CodeSynthesisSurrogate
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorKind {
    Cache,
    CpuAffordance,
    CheapModel,
    ExpensiveModel,
    VerificationAffordance,
}

impl ExecutorKind {
    fn preference_rank(&self) -> u8 {
        match self {
            Self::Cache => 0,
            Self::CpuAffordance => 1,
            Self::VerificationAffordance => 2,
            Self::CheapModel => 3,
            Self::ExpensiveModel => 4,
        }
    }

    fn is_model(&self) -> bool {
        matches!(self, Self::CheapModel | Self::ExpensiveModel)
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CostEstimate {
    #[serde(default)]
    pub prompt_tokens: f64,
    #[serde(default)]
    pub completion_tokens: f64,
    #[serde(default)]
    pub gpu_seconds: f64,
    #[serde(default)]
    pub cpu_ms: f64,
    #[serde(default)]
    pub latency_ms: f64,
    #[serde(default)]
    pub quality: f64,
}

impl CostEstimate {
    pub fn tokens(&self) -> f64 {
        self.prompt_tokens + self.completion_tokens
    }

    pub fn scaled_model_cost(mut self, factor: f64) -> Self {
        let factor = factor.clamp(0.0, 1.0);
        self.prompt_tokens *= factor;
        self.completion_tokens *= factor;
        self.gpu_seconds *= factor;
        self
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct CostWeights {
    pub prompt_token: f64,
    pub completion_token: f64,
    pub gpu_second: f64,
    pub cpu_ms: f64,
    pub latency_ms: f64,
}

impl Default for CostWeights {
    fn default() -> Self {
        Self {
            prompt_token: 1.0,
            completion_token: 1.5,
            gpu_second: 50_000.0,
            cpu_ms: 0.01,
            latency_ms: 0.02,
        }
    }
}

impl CostWeights {
    pub fn weighted_cost(&self, cost: CostEstimate) -> f64 {
        cost.prompt_tokens * self.prompt_token
            + cost.completion_tokens * self.completion_token
            + cost.gpu_seconds * self.gpu_second
            + cost.cpu_ms * self.cpu_ms
            + cost.latency_ms * self.latency_ms
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PhysicalCandidate {
    pub executor: ExecutorKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub affordance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    pub cost: CostEstimate,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl PhysicalCandidate {
    pub fn cpu(affordance_id: impl Into<String>, cost: CostEstimate) -> Self {
        Self {
            executor: ExecutorKind::CpuAffordance,
            affordance_id: Some(affordance_id.into()),
            model_id: None,
            cost,
            notes: Vec::new(),
        }
    }

    pub fn model(executor: ExecutorKind, model_id: impl Into<String>, cost: CostEstimate) -> Self {
        Self {
            executor,
            affordance_id: None,
            model_id: Some(model_id.into()),
            cost,
            notes: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct Operation {
    pub operation_id: String,
    pub kind: OperationKind,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub input_key: Value,
    #[serde(default)]
    pub estimated_rows: u64,
    #[serde(default = "default_selectivity")]
    pub selectivity: f64,
    #[serde(default)]
    pub quality_floor: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fusion_key: Option<String>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub candidates: Vec<PhysicalCandidate>,
}

impl Operation {
    pub fn new(operation_id: impl Into<String>, kind: OperationKind) -> Self {
        Self {
            operation_id: operation_id.into(),
            kind,
            description: String::new(),
            input_key: Value::Null,
            estimated_rows: 1,
            selectivity: 1.0,
            quality_floor: 0.0,
            fusion_key: None,
            dependencies: Vec::new(),
            candidates: Vec::new(),
        }
    }

    pub fn with_candidate(mut self, candidate: PhysicalCandidate) -> Self {
        self.candidates.push(candidate);
        self
    }

    fn normalized(mut self) -> Self {
        if self.operation_id.trim().is_empty() {
            self.operation_id = stable_hash(json!({
                "kind": self.kind,
                "input_key": self.input_key
            }));
        }
        if self.estimated_rows == 0 {
            self.estimated_rows = 1;
        }
        self.selectivity = self.selectivity.clamp(0.0, 1.0);
        self.quality_floor = self.quality_floor.clamp(0.0, 1.0);
        self.dependencies = clean_strings(self.dependencies);
        if self.candidates.is_empty() {
            self.candidates = default_candidates(&self);
        }
        self
    }
}

fn default_selectivity() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannerConfig {
    #[serde(default)]
    pub graph_version: u64,
    #[serde(default)]
    pub cost_weights: CostWeights,
    #[serde(default = "default_fusion_follow_on_cost_ratio")]
    pub fusion_follow_on_cost_ratio: f64,
}

impl Default for PlannerConfig {
    fn default() -> Self {
        Self {
            graph_version: 0,
            cost_weights: CostWeights::default(),
            fusion_follow_on_cost_ratio: default_fusion_follow_on_cost_ratio(),
        }
    }
}

fn default_fusion_follow_on_cost_ratio() -> f64 {
    0.35
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct InMemoryComputationCache {
    pub entries: BTreeMap<String, CachedComputation>,
}

impl InMemoryComputationCache {
    pub fn put(
        &mut self,
        operation: &Operation,
        graph_version: u64,
        value: Value,
        provenance: Value,
    ) -> CachedComputation {
        let operation = operation.clone().normalized();
        let entry = CachedComputation {
            cache_key: cache_key_for_operation(&operation),
            operation_id: operation.operation_id,
            operation_kind: operation.kind,
            input_hash: stable_hash(operation.input_key),
            graph_version,
            value,
            provenance,
        };
        self.entries.insert(entry.cache_key.clone(), entry.clone());
        entry
    }

    pub fn insert_entry(&mut self, entry: CachedComputation) {
        self.entries.insert(entry.cache_key.clone(), entry);
    }

    pub fn lookup(&self, operation: &Operation, graph_version: u64) -> CacheLookup {
        let key = cache_key_for_operation(operation);
        match self.entries.get(&key).cloned() {
            Some(entry) if entry.graph_version == graph_version => CacheLookup {
                status: CacheLookupStatus::Fresh,
                entry: Some(entry),
            },
            Some(entry) => CacheLookup {
                status: CacheLookupStatus::Stale,
                entry: Some(entry),
            },
            None => CacheLookup {
                status: CacheLookupStatus::Miss,
                entry: None,
            },
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CachedComputation {
    pub cache_key: String,
    pub operation_id: String,
    pub operation_kind: OperationKind,
    pub input_hash: String,
    pub graph_version: u64,
    pub value: Value,
    #[serde(default)]
    pub provenance: Value,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheLookupStatus {
    Fresh,
    Stale,
    Miss,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CacheLookup {
    pub status: CacheLookupStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<CachedComputation>,
}

pub fn cache_key_for_operation(operation: &Operation) -> String {
    stable_hash(json!({
        "kind": operation.kind,
        "input": operation.input_key
    }))
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OperationPlanner {
    pub config: PlannerConfig,
    #[serde(default)]
    pub cache: InMemoryComputationCache,
}

impl OperationPlanner {
    pub fn new(config: PlannerConfig) -> Self {
        Self {
            config,
            cache: InMemoryComputationCache::default(),
        }
    }

    pub fn with_cache(mut self, cache: InMemoryComputationCache) -> Self {
        self.cache = cache;
        self
    }

    pub fn plan(&self, operations: Vec<Operation>) -> OffloadPlan {
        let original = operations
            .into_iter()
            .map(Operation::normalized)
            .collect::<Vec<_>>();
        let ordered = reorder_for_pushdown(original.clone());
        let mut rewrites = Vec::new();
        if ordered_ids(&ordered) != ordered_ids(&original) {
            rewrites.push(PlannerRewrite {
                axis: OffloadAxis::PredicatePushdown,
                description:
                    "cheap predicate filters moved before model-heavy operations where dependencies allow"
                        .to_string(),
                operation_ids: ordered_ids(&ordered),
            });
        }

        let mut current_rows = ordered
            .iter()
            .find(|op| op.estimated_rows > 0)
            .map(|op| op.estimated_rows as f64)
            .unwrap_or(1.0);
        let mut previous_fusion_key: Option<String> = None;
        let mut steps = Vec::with_capacity(ordered.len());
        let mut totals = CostSummary::default();

        for operation in ordered {
            let cache_lookup = self.cache.lookup(&operation, self.config.graph_version);
            let baseline = baseline_candidate(&operation);
            let row_factor = row_factor_for_operation(&operation, current_rows);
            let mut selected = self.choose_candidate(&operation, &cache_lookup, row_factor);
            let mut fused = false;
            if selected.executor.is_model() {
                if let Some(key) = operation.fusion_key.as_ref() {
                    if previous_fusion_key.as_deref() == Some(key.as_str()) {
                        selected.cost.prompt_tokens *= self.config.fusion_follow_on_cost_ratio;
                        selected.cost.completion_tokens *= self.config.fusion_follow_on_cost_ratio;
                        selected.cost.gpu_seconds *= self.config.fusion_follow_on_cost_ratio;
                        fused = true;
                    }
                    previous_fusion_key = Some(key.clone());
                } else {
                    previous_fusion_key = None;
                }
            } else {
                previous_fusion_key = None;
            }
            if fused {
                rewrites.push(PlannerRewrite {
                    axis: OffloadAxis::OperatorFusion,
                    description: "adjacent model operations with the same fusion key were batched"
                        .to_string(),
                    operation_ids: vec![operation.operation_id.clone()],
                });
            }

            let quality_floor_met = selected.cost.quality + f64::EPSILON >= operation.quality_floor;
            let output_rows = if operation.kind.is_filter() {
                (current_rows * operation.selectivity).ceil().max(1.0)
            } else {
                current_rows
            };
            current_rows = output_rows;
            totals.add(baseline.cost, selected.cost);
            let ledger_entry = LedgerEntry::from_decision(
                &operation,
                &baseline,
                &selected,
                self.config.graph_version,
            );
            steps.push(PlannedOperation {
                operation,
                baseline,
                selected,
                cache_status: cache_lookup.status,
                output_rows_estimate: output_rows as u64,
                quality_floor_met,
                ledger_entry,
            });
        }

        OffloadPlan {
            graph_version: self.config.graph_version,
            steps,
            rewrites,
            totals,
        }
    }

    fn choose_candidate(
        &self,
        operation: &Operation,
        cache_lookup: &CacheLookup,
        row_factor: f64,
    ) -> PhysicalCandidate {
        let mut candidates = operation.candidates.clone();
        if cache_lookup.status == CacheLookupStatus::Fresh {
            candidates.push(PhysicalCandidate {
                executor: ExecutorKind::Cache,
                affordance_id: Some("computation_cache".to_string()),
                model_id: None,
                cost: CostEstimate {
                    quality: 1.0,
                    ..CostEstimate::default()
                },
                notes: vec!["cache-as-router-arm".to_string()],
            });
        }
        candidates
            .into_iter()
            .map(|mut candidate| {
                if candidate.executor.is_model() {
                    candidate.cost = candidate.cost.scaled_model_cost(row_factor);
                }
                candidate
            })
            .min_by(|left, right| self.compare_candidates(operation, left, right))
            .unwrap_or_else(|| baseline_candidate(operation))
    }

    fn compare_candidates(
        &self,
        operation: &Operation,
        left: &PhysicalCandidate,
        right: &PhysicalCandidate,
    ) -> Ordering {
        let left_meets = left.cost.quality + f64::EPSILON >= operation.quality_floor;
        let right_meets = right.cost.quality + f64::EPSILON >= operation.quality_floor;
        match (left_meets, right_meets) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => {
                let left_cost = self.config.cost_weights.weighted_cost(left.cost);
                let right_cost = self.config.cost_weights.weighted_cost(right.cost);
                left_cost
                    .partial_cmp(&right_cost)
                    .unwrap_or(Ordering::Equal)
                    .then_with(|| {
                        left.executor
                            .preference_rank()
                            .cmp(&right.executor.preference_rank())
                    })
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OffloadPlan {
    pub graph_version: u64,
    pub steps: Vec<PlannedOperation>,
    pub rewrites: Vec<PlannerRewrite>,
    pub totals: CostSummary,
}

impl OffloadPlan {
    pub fn ledger_entries(&self) -> Vec<LedgerEntry> {
        self.steps
            .iter()
            .map(|step| step.ledger_entry.clone())
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannedOperation {
    pub operation: Operation,
    pub baseline: PhysicalCandidate,
    pub selected: PhysicalCandidate,
    pub cache_status: CacheLookupStatus,
    pub output_rows_estimate: u64,
    pub quality_floor_met: bool,
    pub ledger_entry: LedgerEntry,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct PlannerRewrite {
    pub axis: OffloadAxis,
    pub description: String,
    pub operation_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CostSummary {
    pub baseline_prompt_tokens: f64,
    pub baseline_completion_tokens: f64,
    pub baseline_gpu_seconds: f64,
    pub selected_prompt_tokens: f64,
    pub selected_completion_tokens: f64,
    pub selected_gpu_seconds: f64,
    pub selected_cpu_ms: f64,
    pub tokens_saved: f64,
    pub gpu_seconds_saved: f64,
}

impl CostSummary {
    fn add(&mut self, baseline: CostEstimate, selected: CostEstimate) {
        self.baseline_prompt_tokens += baseline.prompt_tokens;
        self.baseline_completion_tokens += baseline.completion_tokens;
        self.baseline_gpu_seconds += baseline.gpu_seconds;
        self.selected_prompt_tokens += selected.prompt_tokens;
        self.selected_completion_tokens += selected.completion_tokens;
        self.selected_gpu_seconds += selected.gpu_seconds;
        self.selected_cpu_ms += selected.cpu_ms;
        self.tokens_saved = (self.baseline_prompt_tokens + self.baseline_completion_tokens
            - self.selected_prompt_tokens
            - self.selected_completion_tokens)
            .max(0.0);
        self.gpu_seconds_saved = (self.baseline_gpu_seconds - self.selected_gpu_seconds).max(0.0);
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OffloadLedger {
    pub entries: Vec<LedgerEntry>,
}

impl OffloadLedger {
    pub fn record_plan(&mut self, plan: &OffloadPlan) {
        self.entries.extend(plan.ledger_entries());
    }

    pub fn totals(&self) -> CostSummary {
        let mut totals = CostSummary::default();
        for entry in &self.entries {
            totals.baseline_prompt_tokens += entry.baseline.prompt_tokens;
            totals.baseline_completion_tokens += entry.baseline.completion_tokens;
            totals.baseline_gpu_seconds += entry.baseline.gpu_seconds;
            totals.selected_prompt_tokens += entry.selected.prompt_tokens;
            totals.selected_completion_tokens += entry.selected.completion_tokens;
            totals.selected_gpu_seconds += entry.selected.gpu_seconds;
            totals.selected_cpu_ms += entry.selected.cpu_ms;
        }
        totals.tokens_saved = (totals.baseline_prompt_tokens + totals.baseline_completion_tokens
            - totals.selected_prompt_tokens
            - totals.selected_completion_tokens)
            .max(0.0);
        totals.gpu_seconds_saved =
            (totals.baseline_gpu_seconds - totals.selected_gpu_seconds).max(0.0);
        totals
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct LedgerEntry {
    pub ledger_id: String,
    pub graph_version: u64,
    pub operation_id: String,
    pub axis: OffloadAxis,
    pub selected_executor: ExecutorKind,
    pub baseline: CostEstimate,
    pub selected: CostEstimate,
    pub tokens_saved: f64,
    pub gpu_seconds_saved: f64,
}

impl LedgerEntry {
    fn from_decision(
        operation: &Operation,
        baseline: &PhysicalCandidate,
        selected: &PhysicalCandidate,
        graph_version: u64,
    ) -> Self {
        let tokens_saved = (baseline.cost.tokens() - selected.cost.tokens()).max(0.0);
        let gpu_seconds_saved = (baseline.cost.gpu_seconds - selected.cost.gpu_seconds).max(0.0);
        let payload = json!({
            "graph_version": graph_version,
            "operation_id": operation.operation_id,
            "selected": selected.executor,
            "tokens_saved": tokens_saved,
            "gpu_seconds_saved": gpu_seconds_saved
        });
        Self {
            ledger_id: stable_hash(payload),
            graph_version,
            operation_id: operation.operation_id.clone(),
            axis: operation.kind.primary_axis(),
            selected_executor: selected.executor.clone(),
            baseline: baseline.cost,
            selected: selected.cost,
            tokens_saved,
            gpu_seconds_saved,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CalibrationSample {
    pub raw_score: f64,
    pub observed_success: bool,
    #[serde(default = "default_weight")]
    pub weight: f64,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub struct CalibrationPoint {
    pub min_score: f64,
    pub max_score: f64,
    pub calibrated_probability: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct IsotonicCalibrator {
    pub points: Vec<CalibrationPoint>,
}

impl IsotonicCalibrator {
    pub fn fit(samples: &[CalibrationSample]) -> Self {
        let mut sorted = samples
            .iter()
            .filter(|sample| sample.weight > 0.0)
            .map(|sample| {
                let y = if sample.observed_success { 1.0 } else { 0.0 };
                IsotonicBlock {
                    min_score: sample.raw_score,
                    max_score: sample.raw_score,
                    weight: sample.weight,
                    weighted_success: sample.weight * y,
                }
            })
            .collect::<Vec<_>>();
        sorted.sort_by(|a, b| {
            a.min_score
                .partial_cmp(&b.min_score)
                .unwrap_or(Ordering::Equal)
        });

        let mut blocks: Vec<IsotonicBlock> = Vec::new();
        for block in sorted {
            blocks.push(block);
            while blocks.len() >= 2 {
                let last = blocks.len() - 1;
                if blocks[last - 1].mean() <= blocks[last].mean() + f64::EPSILON {
                    break;
                }
                let right = blocks.pop().expect("right block");
                let left = blocks.pop().expect("left block");
                blocks.push(left.merge(right));
            }
        }

        Self {
            points: blocks
                .into_iter()
                .map(|block| CalibrationPoint {
                    min_score: block.min_score,
                    max_score: block.max_score,
                    calibrated_probability: block.mean().clamp(0.0, 1.0),
                })
                .collect(),
        }
    }

    pub fn predict(&self, raw_score: f64) -> f64 {
        if self.points.is_empty() {
            return raw_score.clamp(0.0, 1.0);
        }
        for point in &self.points {
            if raw_score <= point.max_score {
                return point.calibrated_probability;
            }
        }
        self.points
            .last()
            .map(|point| point.calibrated_probability)
            .unwrap_or_else(|| raw_score.clamp(0.0, 1.0))
    }
}

#[derive(Clone, Copy, Debug)]
struct IsotonicBlock {
    min_score: f64,
    max_score: f64,
    weight: f64,
    weighted_success: f64,
}

impl IsotonicBlock {
    fn mean(&self) -> f64 {
        if self.weight <= 0.0 {
            0.0
        } else {
            self.weighted_success / self.weight
        }
    }

    fn merge(self, other: Self) -> Self {
        Self {
            min_score: self.min_score.min(other.min_score),
            max_score: self.max_score.max(other.max_score),
            weight: self.weight + other.weight,
            weighted_success: self.weighted_success + other.weighted_success,
        }
    }
}

fn default_weight() -> f64 {
    1.0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ModelCandidate {
    pub model_id: String,
    pub raw_confidence: f64,
    pub cost: CostEstimate,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CascadeDecision {
    pub selected_model_id: String,
    pub selected_cost: CostEstimate,
    pub calibrated_confidence: f64,
    pub escalated_models: Vec<String>,
    pub quality_floor_met: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct CascadeRouter {
    pub calibrator: IsotonicCalibrator,
    #[serde(default)]
    pub weights: CostWeights,
}

impl CascadeRouter {
    pub fn route(
        &self,
        candidates: &[ModelCandidate],
        quality_floor: f64,
    ) -> Option<CascadeDecision> {
        let mut ordered = candidates.to_vec();
        ordered.sort_by(|left, right| {
            self.weights
                .weighted_cost(left.cost)
                .partial_cmp(&self.weights.weighted_cost(right.cost))
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.model_id.cmp(&right.model_id))
        });
        let mut escalated = Vec::new();
        let mut fallback: Option<(ModelCandidate, f64)> = None;
        for candidate in ordered {
            let calibrated = self.calibrator.predict(candidate.raw_confidence);
            let quality_floor_met = calibrated + f64::EPSILON >= quality_floor;
            if quality_floor_met {
                return Some(CascadeDecision {
                    selected_model_id: candidate.model_id,
                    selected_cost: candidate.cost,
                    calibrated_confidence: calibrated,
                    escalated_models: escalated,
                    quality_floor_met: true,
                });
            }
            escalated.push(candidate.model_id.clone());
            if fallback
                .as_ref()
                .map(|(_, current)| calibrated > *current)
                .unwrap_or(true)
            {
                fallback = Some((candidate, calibrated));
            }
        }
        fallback.map(|(candidate, calibrated)| CascadeDecision {
            selected_model_id: candidate.model_id,
            selected_cost: candidate.cost,
            calibrated_confidence: calibrated,
            escalated_models: escalated,
            quality_floor_met: false,
        })
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RequiredEdge {
    pub from_id: String,
    pub edge_type: String,
    pub to_id: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaimCheck {
    pub claim_id: String,
    #[serde(default)]
    pub required_node_ids: Vec<String>,
    #[serde(default)]
    pub required_edges: Vec<RequiredEdge>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationIssue {
    pub claim_id: String,
    pub reason: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct VerificationReceipt {
    pub receipt_hash: String,
    pub graph_version: u64,
    pub passed: bool,
    pub checked_claims: usize,
    pub issues: Vec<VerificationIssue>,
}

pub fn verify_claims_against_graph(
    snapshot: &GraphSnapshot,
    checks: &[ClaimCheck],
) -> VerificationReceipt {
    let node_ids = snapshot
        .nodes
        .iter()
        .filter(|node| !node.tombstone)
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();
    let edge_keys = snapshot
        .edges
        .iter()
        .filter(|edge| !edge.tombstone)
        .map(|edge| {
            (
                edge.from_id.clone(),
                edge.edge_type.clone(),
                edge.to_id.clone(),
            )
        })
        .collect::<BTreeSet<_>>();

    let mut issues = Vec::new();
    for check in checks {
        for required in &check.required_node_ids {
            if !node_ids.contains(required) {
                issues.push(VerificationIssue {
                    claim_id: check.claim_id.clone(),
                    reason: format!("missing required node {required}"),
                });
            }
        }
        for required in &check.required_edges {
            let key = (
                required.from_id.clone(),
                required.edge_type.clone(),
                required.to_id.clone(),
            );
            if !edge_keys.contains(&key) {
                issues.push(VerificationIssue {
                    claim_id: check.claim_id.clone(),
                    reason: format!(
                        "missing required edge {} -{}-> {}",
                        required.from_id, required.edge_type, required.to_id
                    ),
                });
            }
        }
    }

    let passed = issues.is_empty();
    let mut receipt = VerificationReceipt {
        receipt_hash: String::new(),
        graph_version: snapshot.version,
        passed,
        checked_claims: checks.len(),
        issues,
    };
    receipt.receipt_hash = stable_hash(json!({
        "graph_version": receipt.graph_version,
        "passed": receipt.passed,
        "checked_claims": receipt.checked_claims,
        "issues": receipt.issues
    }));
    receipt
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum GraphAffordanceRequest {
    PageRank {
        #[serde(default = "default_pagerank_damping")]
        damping: f64,
        #[serde(default = "default_max_iter")]
        max_iter: usize,
        #[serde(default = "default_tolerance")]
        tolerance: f64,
    },
    Communities,
    ShortestPath {
        source: String,
        target: String,
        #[serde(default = "default_max_depth")]
        max_depth: usize,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct GraphAffordanceReceipt {
    pub receipt_hash: String,
    pub affordance_id: String,
    pub graph_version: u64,
    pub input_hash: String,
    pub payload: Value,
    pub cost: CostEstimate,
}

pub struct GraphAffordanceEngine;

impl GraphAffordanceEngine {
    pub fn run(
        snapshot: &GraphSnapshot,
        request: GraphAffordanceRequest,
    ) -> GraphAffordanceReceipt {
        let input_hash = stable_hash(json!({
            "graph_version": snapshot.version,
            "request": request
        }));
        let (affordance_id, payload) = match request {
            GraphAffordanceRequest::PageRank {
                damping,
                max_iter,
                tolerance,
            } => (
                "graph.pagerank".to_string(),
                json!({
                    "scores": pagerank(&snapshot.edges, damping, max_iter, tolerance),
                }),
            ),
            GraphAffordanceRequest::Communities => {
                let (communities, modularity) = label_propagation_communities(&snapshot.edges);
                (
                    "graph.communities".to_string(),
                    json!({
                        "communities": communities,
                        "modularity": modularity,
                    }),
                )
            }
            GraphAffordanceRequest::ShortestPath {
                source,
                target,
                max_depth,
            } => {
                let found = paths_shortest_weighted(&snapshot.edges, &source, &target, max_depth);
                (
                    "graph.shortest_path".to_string(),
                    json!({
                        "source": source,
                        "target": target,
                        "path": found.as_ref().map(|(path, _)| path.clone()).unwrap_or_default(),
                        "cost": found.map(|(_, cost)| cost),
                    }),
                )
            }
        };
        let mut receipt = GraphAffordanceReceipt {
            receipt_hash: String::new(),
            affordance_id,
            graph_version: snapshot.version,
            input_hash,
            payload,
            cost: CostEstimate {
                cpu_ms: snapshot.edges.len() as f64,
                quality: 1.0,
                ..CostEstimate::default()
            },
        };
        receipt.receipt_hash = stable_hash(json!({
            "affordance_id": receipt.affordance_id,
            "graph_version": receipt.graph_version,
            "input_hash": receipt.input_hash,
            "payload": receipt.payload,
        }));
        receipt
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct PlannerInvocation {
    #[serde(default)]
    pub operations: Vec<Operation>,
    #[serde(default)]
    pub cache_entries: Vec<CachedComputation>,
    #[serde(default)]
    pub graph_version: Option<u64>,
    #[serde(default)]
    pub config: PlannerConfig,
}

pub fn plan_from_json(
    arguments: &Value,
    fallback_graph_version: u64,
) -> Result<OffloadPlan, String> {
    let mut invocation: PlannerInvocation =
        serde_json::from_value(arguments.clone()).map_err(|e| e.to_string())?;
    if invocation.config.graph_version == 0 {
        invocation.config.graph_version =
            invocation.graph_version.unwrap_or(fallback_graph_version);
    }
    if invocation.operations.is_empty() {
        return Err("compute-offload planning requires at least one operation".to_string());
    }
    let mut cache = InMemoryComputationCache::default();
    for entry in invocation.cache_entries {
        cache.insert_entry(entry);
    }
    Ok(OperationPlanner::new(invocation.config)
        .with_cache(cache)
        .plan(invocation.operations))
}

fn default_pagerank_damping() -> f64 {
    0.85
}

fn default_max_iter() -> usize {
    100
}

fn default_tolerance() -> f64 {
    1e-6
}

fn default_max_depth() -> usize {
    8
}

fn baseline_candidate(operation: &Operation) -> PhysicalCandidate {
    operation
        .candidates
        .iter()
        .find(|candidate| candidate.executor == ExecutorKind::ExpensiveModel)
        .cloned()
        .unwrap_or_else(|| {
            let rows = operation.estimated_rows.max(1) as f64;
            PhysicalCandidate::model(
                ExecutorKind::ExpensiveModel,
                "frontier-model",
                CostEstimate {
                    prompt_tokens: 600.0 + rows * 12.0,
                    completion_tokens: 250.0,
                    gpu_seconds: 0.08 + rows * 0.002,
                    latency_ms: 1_200.0,
                    quality: 0.92,
                    ..CostEstimate::default()
                },
            )
        })
}

fn default_candidates(operation: &Operation) -> Vec<PhysicalCandidate> {
    let rows = operation.estimated_rows.max(1) as f64;
    if operation.kind.is_cpu_symbolic() {
        let executor = if operation.kind == OperationKind::VerificationCheck {
            ExecutorKind::VerificationAffordance
        } else {
            ExecutorKind::CpuAffordance
        };
        return vec![
            PhysicalCandidate {
                executor,
                affordance_id: Some(format!("{COMPUTE_OFFLOAD_ENGINE_ID}.{:?}", operation.kind)),
                model_id: None,
                cost: CostEstimate {
                    cpu_ms: 5.0 + rows,
                    latency_ms: 20.0 + rows * 0.2,
                    quality: 1.0,
                    ..CostEstimate::default()
                },
                notes: vec!["cpu-exact".to_string()],
            },
            baseline_candidate(operation),
        ];
    }
    vec![
        PhysicalCandidate::model(
            ExecutorKind::CheapModel,
            "cheap-model",
            CostEstimate {
                prompt_tokens: 200.0 + rows * 4.0,
                completion_tokens: 80.0,
                gpu_seconds: 0.02 + rows * 0.0005,
                latency_ms: 500.0,
                quality: 0.78,
                ..CostEstimate::default()
            },
        ),
        baseline_candidate(operation),
    ]
}

fn reorder_for_pushdown(operations: Vec<Operation>) -> Vec<Operation> {
    let mut pending = operations;
    let mut complete = BTreeSet::new();
    let mut ordered = Vec::with_capacity(pending.len());
    while !pending.is_empty() {
        let mut ready = pending
            .iter()
            .enumerate()
            .filter(|(_, op)| op.dependencies.iter().all(|dep| complete.contains(dep)))
            .map(|(index, op)| (index, operation_order_key(op)))
            .collect::<Vec<_>>();
        if ready.is_empty() {
            ordered.extend(pending.into_iter());
            break;
        }
        ready.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let index = ready[0].0;
        let op = pending.remove(index);
        complete.insert(op.operation_id.clone());
        ordered.push(op);
    }
    ordered
}

fn operation_order_key(operation: &Operation) -> (u8, u64) {
    let class = if operation.kind.is_filter() {
        0
    } else if operation.kind.is_cpu_symbolic() {
        1
    } else if operation.kind.is_model_heavy() {
        3
    } else {
        2
    };
    let selectivity_rank = (operation.selectivity * 1_000_000.0) as u64;
    (class, selectivity_rank)
}

fn ordered_ids(operations: &[Operation]) -> Vec<String> {
    operations
        .iter()
        .map(|operation| operation.operation_id.clone())
        .collect()
}

fn row_factor_for_operation(operation: &Operation, current_rows: f64) -> f64 {
    if operation.kind.is_model_heavy() {
        (current_rows / operation.estimated_rows.max(1) as f64).clamp(0.0, 1.0)
    } else {
        1.0
    }
}

fn clean_strings(values: Vec<String>) -> Vec<String> {
    let mut out = values
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::{EdgeRecord, NodeRecord};

    fn expensive_model() -> PhysicalCandidate {
        PhysicalCandidate::model(
            ExecutorKind::ExpensiveModel,
            "frontier",
            CostEstimate {
                prompt_tokens: 1_000.0,
                completion_tokens: 400.0,
                gpu_seconds: 2.0,
                quality: 0.94,
                ..CostEstimate::default()
            },
        )
    }

    fn cheap_model(quality: f64) -> PhysicalCandidate {
        PhysicalCandidate::model(
            ExecutorKind::CheapModel,
            "small",
            CostEstimate {
                prompt_tokens: 100.0,
                completion_tokens: 40.0,
                gpu_seconds: 0.1,
                quality,
                ..CostEstimate::default()
            },
        )
    }

    #[test]
    fn cache_is_router_arm_and_stale_cache_is_rejected() {
        let operation = Operation::new("summarize", OperationKind::NeuralSynthesis)
            .with_candidate(cheap_model(0.8))
            .with_candidate(expensive_model());
        let mut cache = InMemoryComputationCache::default();
        cache.put(
            &operation,
            7,
            json!({"answer": "cached"}),
            json!({"source": "test"}),
        );

        let fresh = OperationPlanner::new(PlannerConfig {
            graph_version: 7,
            ..Default::default()
        })
        .with_cache(cache.clone())
        .plan(vec![operation.clone()]);
        assert_eq!(fresh.steps[0].cache_status, CacheLookupStatus::Fresh);
        assert_eq!(fresh.steps[0].selected.executor, ExecutorKind::Cache);

        let stale = OperationPlanner::new(PlannerConfig {
            graph_version: 8,
            ..Default::default()
        })
        .with_cache(cache)
        .plan(vec![operation]);
        assert_eq!(stale.steps[0].cache_status, CacheLookupStatus::Stale);
        assert_ne!(stale.steps[0].selected.executor, ExecutorKind::Cache);
    }

    #[test]
    fn predicate_pushdown_prunes_model_cost() {
        let synth = Operation {
            operation_id: "synth".to_string(),
            kind: OperationKind::NeuralSynthesis,
            estimated_rows: 100,
            quality_floor: 0.7,
            candidates: vec![cheap_model(0.8), expensive_model()],
            ..Operation::new("", OperationKind::NeuralSynthesis)
        };
        let filter = Operation {
            operation_id: "filter".to_string(),
            kind: OperationKind::PredicateFilter,
            estimated_rows: 100,
            selectivity: 0.1,
            ..Operation::new("", OperationKind::PredicateFilter)
        };
        let plan = OperationPlanner::new(PlannerConfig::default()).plan(vec![synth, filter]);
        assert_eq!(plan.steps[0].operation.operation_id, "filter");
        assert!(plan
            .rewrites
            .iter()
            .any(|rewrite| rewrite.axis == OffloadAxis::PredicatePushdown));
        let synth_step = plan
            .steps
            .iter()
            .find(|step| step.operation.operation_id == "synth")
            .unwrap();
        assert!(
            synth_step.selected.cost.prompt_tokens < 100.0,
            "model prompt tokens should scale down after pushdown"
        );
    }

    #[test]
    fn model_fusion_discounts_follow_on_operation() {
        let first = Operation {
            operation_id: "classify-a".to_string(),
            kind: OperationKind::NeuralSynthesis,
            fusion_key: Some("classify".to_string()),
            candidates: vec![cheap_model(0.8), expensive_model()],
            ..Operation::new("", OperationKind::NeuralSynthesis)
        };
        let second = Operation {
            operation_id: "classify-b".to_string(),
            kind: OperationKind::NeuralSynthesis,
            fusion_key: Some("classify".to_string()),
            candidates: vec![cheap_model(0.8), expensive_model()],
            ..Operation::new("", OperationKind::NeuralSynthesis)
        };
        let plan = OperationPlanner::new(PlannerConfig::default()).plan(vec![first, second]);
        assert!(plan
            .rewrites
            .iter()
            .any(|rewrite| rewrite.axis == OffloadAxis::OperatorFusion));
        assert!(plan.steps[1].selected.cost.gpu_seconds < plan.steps[0].selected.cost.gpu_seconds);
    }

    #[test]
    fn isotonic_cascade_uses_calibrated_confidence() {
        let calibrator = IsotonicCalibrator::fit(&[
            CalibrationSample {
                raw_score: 0.2,
                observed_success: true,
                weight: 1.0,
            },
            CalibrationSample {
                raw_score: 0.8,
                observed_success: false,
                weight: 1.0,
            },
            CalibrationSample {
                raw_score: 0.9,
                observed_success: true,
                weight: 1.0,
            },
        ]);
        let router = CascadeRouter {
            calibrator,
            weights: CostWeights::default(),
        };
        let decision = router
            .route(
                &[
                    ModelCandidate {
                        model_id: "cheap-uncalibrated".to_string(),
                        raw_confidence: 0.8,
                        cost: CostEstimate {
                            prompt_tokens: 10.0,
                            quality: 0.8,
                            ..CostEstimate::default()
                        },
                    },
                    ModelCandidate {
                        model_id: "frontier".to_string(),
                        raw_confidence: 0.95,
                        cost: CostEstimate {
                            prompt_tokens: 100.0,
                            quality: 0.95,
                            ..CostEstimate::default()
                        },
                    },
                ],
                0.6,
            )
            .unwrap();
        assert_eq!(decision.selected_model_id, "frontier");
        assert!(decision
            .escalated_models
            .contains(&"cheap-uncalibrated".to_string()));
    }

    #[test]
    fn verification_checks_claims_against_graph() {
        let snapshot = GraphSnapshot {
            version: 3,
            nodes: vec![
                NodeRecord::new("n:a", ["Thing"], json!({})),
                NodeRecord::new("n:b", ["Thing"], json!({})),
            ],
            edges: vec![EdgeRecord::new("e:ab", "n:a", "LINKS", "n:b", json!({}))],
        };
        let ok = verify_claims_against_graph(
            &snapshot,
            &[ClaimCheck {
                claim_id: "claim:1".to_string(),
                required_node_ids: vec!["n:a".to_string()],
                required_edges: vec![RequiredEdge {
                    from_id: "n:a".to_string(),
                    edge_type: "LINKS".to_string(),
                    to_id: "n:b".to_string(),
                }],
            }],
        );
        assert!(ok.passed);
        let bad = verify_claims_against_graph(
            &snapshot,
            &[ClaimCheck {
                claim_id: "claim:2".to_string(),
                required_node_ids: vec!["n:missing".to_string()],
                required_edges: Vec::new(),
            }],
        );
        assert!(!bad.passed);
        assert_eq!(bad.issues.len(), 1);
    }

    #[test]
    fn graph_affordances_run_cpu_exact_receipts() {
        let snapshot = GraphSnapshot {
            version: 11,
            nodes: vec![
                NodeRecord::new("a", ["N"], json!({})),
                NodeRecord::new("b", ["N"], json!({})),
                NodeRecord::new("c", ["N"], json!({})),
            ],
            edges: vec![
                EdgeRecord::new("ab", "a", "LINKS", "b", json!({"confidence": 1.0})),
                EdgeRecord::new("bc", "b", "LINKS", "c", json!({"confidence": 1.0})),
            ],
        };
        let pagerank = GraphAffordanceEngine::run(
            &snapshot,
            GraphAffordanceRequest::PageRank {
                damping: 0.85,
                max_iter: 100,
                tolerance: 1e-9,
            },
        );
        assert_eq!(pagerank.affordance_id, "graph.pagerank");
        assert!(pagerank.payload["scores"]
            .as_object()
            .unwrap()
            .contains_key("a"));

        let path = GraphAffordanceEngine::run(
            &snapshot,
            GraphAffordanceRequest::ShortestPath {
                source: "a".to_string(),
                target: "c".to_string(),
                max_depth: 4,
            },
        );
        assert_eq!(path.payload["path"], json!(["a", "b", "c"]));
    }
}
