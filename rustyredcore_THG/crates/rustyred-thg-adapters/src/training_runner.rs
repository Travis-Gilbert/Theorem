//! Operator-facing training snapshot and writeback helpers.
//!
//! These helpers keep provider orchestration outside the graph engine while
//! giving Theorem a concrete RedCore -> trainer -> RedCore seam.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphMutationBatch, GraphSnapshot, NodeRecord, RedCoreDurability,
    RedCoreGraphStore, RedCoreOptions, ThgError, ThgResult,
};

use crate::hot::{
    evaluate_hot_predictions, hot_temporal_edge_timestamp_for_record,
    hot_temporal_link_dataset_from_snapshot, score_hot_timed_labels, train_hot_link_model,
    HotTemporalLinkDataset, HotTemporalSplitConfig, HotTimedPairLabel, HotTrainingConfig,
    HotTrainingReport,
};
use crate::reflexive::{
    rank_densification_candidates, rank_pairformer_densification_candidates,
    rank_reflexive_organizing_candidates, DensificationRequest, DensificationResult,
};
use crate::training_substrate::{
    export_training_snapshot, register_gnn_export_dir, register_model_artifact,
    register_training_fixture, GnnExportImportOptions, GnnExportImportResult, ModelArtifactInput,
    ModelWritebackResult, TrainingExportManifest, TrainingFixtureResult,
};
use crate::types::thg_error_from_store;
use crate::{paraphrase_pair_node_id, training_pack_node_id, PairformerConfig};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const GRAPH_SNAPSHOT_FILE: &str = "graph_snapshot.json";
pub const RUNPOD_INPUT_FILE: &str = "runpod_input.json";
pub const HOT_MODEL_FILE: &str = "hot_model.json";
pub const HOT_MODEL_ARTIFACT_FILE: &str = "hot_model_artifact.json";

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingSnapshotBundle {
    pub manifest: TrainingExportManifest,
    pub snapshot: GraphSnapshot,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RunPodTrainingInput {
    pub kind: String,
    pub tenant_id: String,
    pub export_id: String,
    pub source_graph_version: u64,
    pub snapshot_hash: String,
    pub manifest: TrainingExportManifest,
    pub local_files: TrainingSnapshotLocalFiles,
    pub trainer_contract: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TrainingSnapshotLocalFiles {
    pub manifest_json: PathBuf,
    pub graph_snapshot_json: PathBuf,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingExportFiles {
    pub manifest_path: PathBuf,
    pub graph_snapshot_path: PathBuf,
    pub runpod_input_path: PathBuf,
    pub manifest: TrainingExportManifest,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TrainingSmokeResult {
    pub fixture: TrainingFixtureResult,
    pub export: TrainingExportFiles,
    pub writeback: ModelWritebackResult,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTemporalFixtureResult {
    pub tenant_id: String,
    pub node_ids: Vec<String>,
    pub edge_ids: Vec<String>,
    pub graph_version: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTrainingRunOptions {
    pub model_id: String,
    pub promotion_decision: String,
    pub s3_uri: Option<String>,
    pub training: HotTrainingConfig,
    pub split: HotTemporalSplitConfig,
    pub max_baseline_candidates: usize,
}

impl Default for HotTrainingRunOptions {
    fn default() -> Self {
        Self {
            model_id: "hot-temporal-link-native-v1".to_string(),
            promotion_decision: "shadow".to_string(),
            s3_uri: None,
            training: HotTrainingConfig::default(),
            split: HotTemporalSplitConfig::default(),
            max_baseline_candidates: 64,
        }
    }
}

impl HotTrainingRunOptions {
    pub fn normalized(mut self) -> Self {
        if self.model_id.trim().is_empty() {
            self.model_id = "hot-temporal-link-native-v1".to_string();
        } else {
            self.model_id = self.model_id.trim().to_string();
        }
        if self.promotion_decision.trim().is_empty() {
            self.promotion_decision = "shadow".to_string();
        } else {
            self.promotion_decision = self.promotion_decision.trim().to_string();
        }
        self.s3_uri = self
            .s3_uri
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        self.training.model = self.training.model.clone().normalized();
        if self.training.epochs == 0 {
            self.training.epochs = 50;
        }
        if self.training.batch_size == 0 {
            self.training.batch_size = 100;
        }
        if self.training.learning_rate <= 0.0 || !self.training.learning_rate.is_finite() {
            self.training.learning_rate = 1e-4;
        }
        self.split = self.split.normalized();
        if self.max_baseline_candidates == 0 {
            self.max_baseline_candidates = 64;
        }
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotBaselineReport {
    pub scorer: String,
    pub report: HotTrainingReport,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTrainingRunMetrics {
    pub run_kind: String,
    pub training_path: String,
    pub burn_path_policy: String,
    pub cubecl_path_policy: String,
    pub train_report: HotTrainingReport,
    pub test_report: HotTrainingReport,
    pub deterministic_report: HotTrainingReport,
    pub baseline_reports: Vec<HotBaselineReport>,
    pub train_examples: usize,
    pub test_examples: usize,
    pub positive_edges: usize,
    pub train_positive_edges: usize,
    pub test_positive_edges: usize,
    pub max_timestamp: i64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct HotTrainingRunResult {
    pub export: TrainingExportFiles,
    pub model_path: PathBuf,
    pub model_artifact_input_path: PathBuf,
    pub writeback: ModelWritebackResult,
    pub metrics: HotTrainingRunMetrics,
}

pub fn redcore_training_options() -> RedCoreOptions {
    RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        snapshot_interval_writes: 100,
        strict_acid: true,
    }
}

pub fn open_training_store(data_dir: impl AsRef<Path>) -> ThgResult<RedCoreGraphStore> {
    RedCoreGraphStore::open(data_dir.as_ref(), redcore_training_options()).map_err(|err| {
        ThgError::new(
            "redcore_training_store_open_failed",
            format!("failed to open RedCore training store: {err:?}"),
        )
    })
}

pub fn seed_training_fixture(
    data_dir: impl AsRef<Path>,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<TrainingFixtureResult> {
    let mut store = open_training_store(data_dir)?;
    let fixture = register_training_fixture(&mut store, tenant_id, actor)?;
    snapshot_store(&mut store)?;
    Ok(fixture)
}

pub fn seed_hot_temporal_fixture(
    data_dir: impl AsRef<Path>,
    tenant_id: &str,
    actor: Option<&str>,
) -> ThgResult<HotTemporalFixtureResult> {
    let mut store = open_training_store(data_dir)?;
    let tenant_id = tenant_id.trim().to_string();
    let node_ids = ["hot:a", "hot:b", "hot:c", "hot:d", "hot:e", "hot:f", "hot:g"]
        .into_iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut mutations = Vec::new();
    for (idx, node_id) in node_ids.iter().enumerate() {
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            node_id,
            ["Object", "HotTemporalFixture"],
            json!({
                "tenant_id": tenant_id,
                "title": format!("HOT temporal fixture node {}", idx + 1),
                "embedding": [
                    idx as f32 / 10.0,
                    ((idx + 1) % 3) as f32,
                    ((idx + 2) % 5) as f32 / 5.0
                ],
                "features": [
                    (idx % 2) as f32,
                    (idx % 3) as f32 / 2.0
                ],
                "source": "hot_temporal_fixture",
            }),
        )));
    }

    let edge_specs = [
        ("hot:a", "hot:b", 1_000_i64),
        ("hot:b", "hot:c", 2_000),
        ("hot:a", "hot:c", 3_000),
        ("hot:c", "hot:d", 4_000),
        ("hot:d", "hot:e", 5_000),
        ("hot:c", "hot:e", 6_000),
        ("hot:e", "hot:f", 7_000),
        ("hot:f", "hot:g", 8_000),
        ("hot:e", "hot:g", 9_000),
    ];
    let mut edge_ids = Vec::with_capacity(edge_specs.len());
    for (idx, (source_id, target_id, timestamp_ms)) in edge_specs.iter().enumerate() {
        let edge_id = format!("hot-fixture-edge-{}", idx + 1);
        edge_ids.push(edge_id.clone());
        mutations.push(GraphMutation::EdgeUpsert(
            EdgeRecord::new(
                &edge_id,
                *source_id,
                "RELATES_TO",
                *target_id,
                json!({
                    "tenant_id": tenant_id,
                    "timestamp_ms": timestamp_ms,
                    "features": [0.9_f32 - idx as f32 * 0.03, idx as f32 / 10.0],
                    "source": "hot_temporal_fixture",
                    "actor": actor,
                }),
            )
            .with_confidence(0.9),
        ));
    }

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(thg_error_from_store)?;
    snapshot_store(&mut store)?;
    Ok(HotTemporalFixtureResult {
        tenant_id,
        node_ids,
        edge_ids,
        graph_version: transaction.graph_version,
    })
}

pub fn import_gnn_export_dir(
    data_dir: impl AsRef<Path>,
    export_dir: impl AsRef<Path>,
    tenant_id: &str,
    export_id: &str,
    options: GnnExportImportOptions,
    actor: Option<&str>,
) -> ThgResult<GnnExportImportResult> {
    let mut store = open_training_store(data_dir)?;
    let result =
        register_gnn_export_dir(&mut store, export_dir, tenant_id, export_id, options, actor)?;
    snapshot_store(&mut store)?;
    Ok(result)
}

pub fn export_training_snapshot_files(
    data_dir: impl AsRef<Path>,
    tenant_id: &str,
    export_id: &str,
    output_dir: impl AsRef<Path>,
) -> ThgResult<TrainingExportFiles> {
    let mut store = open_training_store(data_dir)?;
    snapshot_store(&mut store)?;
    let snapshot = store.graph_snapshot();
    let manifest = export_training_snapshot(&snapshot, tenant_id, export_id)?;

    let output_dir = output_dir.as_ref();
    fs::create_dir_all(output_dir).map_err(io_error("training_export_dir_create_failed"))?;

    let manifest_path = output_dir.join(MANIFEST_FILE);
    let graph_snapshot_path = output_dir.join(GRAPH_SNAPSHOT_FILE);
    let runpod_input_path = output_dir.join(RUNPOD_INPUT_FILE);

    write_json(&manifest_path, &manifest, "training_manifest_write_failed")?;
    write_json(
        &graph_snapshot_path,
        &TrainingSnapshotBundle {
            manifest: manifest.clone(),
            snapshot,
        },
        "training_snapshot_write_failed",
    )?;
    write_json(
        &runpod_input_path,
        &runpod_input_for_manifest(&manifest, &manifest_path, &graph_snapshot_path),
        "runpod_input_write_failed",
    )?;

    Ok(TrainingExportFiles {
        manifest_path,
        graph_snapshot_path,
        runpod_input_path,
        manifest,
    })
}

pub fn writeback_model_artifact_file(
    data_dir: impl AsRef<Path>,
    input_path: impl AsRef<Path>,
    actor: Option<&str>,
) -> ThgResult<ModelWritebackResult> {
    let input = read_model_artifact_input(input_path)?;
    let mut store = open_training_store(data_dir)?;
    let writeback = register_model_artifact(&mut store, input, actor)?;
    snapshot_store(&mut store)?;
    Ok(writeback)
}

#[allow(clippy::too_many_arguments)]
pub fn run_local_training_smoke(
    data_dir: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    tenant_id: &str,
    export_id: &str,
    model_id: &str,
    model_type: &str,
    s3_uri: Option<&str>,
    promotion_decision: &str,
    actor: Option<&str>,
) -> ThgResult<TrainingSmokeResult> {
    let fixture = seed_training_fixture(data_dir.as_ref(), tenant_id, actor)?;
    let export =
        export_training_snapshot_files(data_dir.as_ref(), tenant_id, export_id, output_dir)?;
    let input = ModelArtifactInput {
        model_id: model_id.trim().to_string(),
        tenant_id: tenant_id.trim().to_string(),
        model_type: model_type.trim().to_string(),
        s3_uri: s3_uri
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                format!(
                    "s3://theseus-training/models/{}/model.safetensors",
                    model_id.trim()
                )
            }),
        dataset_hash: export.manifest.snapshot_hash.clone(),
        source_graph_version: export.manifest.graph_version,
        trained_on_node_ids: vec![
            training_pack_node_id(tenant_id, "pack-fixture-v1"),
            paraphrase_pair_node_id(tenant_id, "pair-fixture-v1"),
        ],
        metrics: json!({
            "run_kind": "local_redcore_training_smoke",
            "dataset_hash": export.manifest.snapshot_hash.clone(),
            "nodes_total": export.manifest.counts.nodes_total,
            "edges_total": export.manifest.counts.edges_total,
            "semantic_preservation": 1.0,
            "citation_preservation": 1.0,
            "hallucination_rate": 0.0
        }),
        promotion_decision: promotion_decision.trim().to_string(),
        manifest_version: 1,
    };
    let mut store = open_training_store(data_dir)?;
    let writeback = register_model_artifact(&mut store, input, actor)?;
    snapshot_store(&mut store)?;

    Ok(TrainingSmokeResult {
        fixture,
        export,
        writeback,
    })
}

pub fn run_hot_temporal_training(
    data_dir: impl AsRef<Path>,
    output_dir: impl AsRef<Path>,
    tenant_id: &str,
    export_id: &str,
    options: HotTrainingRunOptions,
    actor: Option<&str>,
) -> ThgResult<HotTrainingRunResult> {
    let options = options.normalized();
    let output_dir = output_dir.as_ref();
    let export =
        export_training_snapshot_files(data_dir.as_ref(), tenant_id, export_id, output_dir)?;
    let bundle = read_training_snapshot_bundle(&export.graph_snapshot_path)?;
    let dataset = hot_temporal_link_dataset_from_snapshot(
        &bundle.snapshot,
        options.training.model.clone(),
        options.split.clone(),
    )?;
    let (model, train_report) =
        train_hot_link_model(&dataset.train_examples, options.training.clone())?;
    let test_report = evaluate_hot_predictions(
        &dataset
            .test_examples
            .iter()
            .map(|example| (model.score_features(&example.features), example.label))
            .collect::<Vec<_>>(),
        options.training.clone(),
    );
    let deterministic_report = evaluate_hot_predictions(
        &score_hot_timed_labels(
            &bundle.snapshot,
            &dataset.test_labels,
            options.training.model.clone(),
        )?
        .into_iter()
        .map(|prediction| (prediction.score, prediction.label))
        .collect::<Vec<_>>(),
        options.training.clone(),
    );
    let baseline_reports = hot_baseline_reports(
        &bundle.snapshot,
        &dataset,
        tenant_id,
        &options.training,
        options.max_baseline_candidates,
    )?;

    let metrics = HotTrainingRunMetrics {
        run_kind: "hot_temporal_link_training".to_string(),
        training_path: "native_mlp_over_hot_pair_representations".to_string(),
        burn_path_policy:
            "feature_gated_scaffold; promote only after held-out AP/AUC plus latency/memory beats native_mlp"
                .to_string(),
        cubecl_path_policy:
            "patch_sum_kernel_scaffold; wire into default HOT path only after profiling shows the patch stage dominates"
                .to_string(),
        train_report,
        test_report,
        deterministic_report,
        baseline_reports,
        train_examples: dataset.train_examples.len(),
        test_examples: dataset.test_examples.len(),
        positive_edges: dataset.positive_edges,
        train_positive_edges: dataset.train_positive_edges,
        test_positive_edges: dataset.test_positive_edges,
        max_timestamp: dataset.max_timestamp,
    };

    fs::create_dir_all(output_dir).map_err(io_error("hot_training_output_dir_create_failed"))?;
    let model_path = output_dir.join(HOT_MODEL_FILE);
    write_json(
        &model_path,
        &json!({
            "kind": "hot_learned_model",
            "model": model,
            "metrics": &metrics,
        }),
        "hot_model_write_failed",
    )?;

    let artifact_input = ModelArtifactInput {
        model_id: options.model_id.clone(),
        tenant_id: tenant_id.trim().to_string(),
        model_type: "hot-temporal-link/native-mlp".to_string(),
        s3_uri: options.s3_uri.unwrap_or_else(|| {
            format!(
                "s3://theseus-training/models/{}/hot_model.json",
                options.model_id
            )
        }),
        dataset_hash: export.manifest.snapshot_hash.clone(),
        source_graph_version: export.manifest.graph_version,
        trained_on_node_ids: trained_on_node_ids(&dataset),
        metrics: serde_json::to_value(&metrics).map_err(|err| {
            ThgError::new(
                "hot_metrics_serialize_failed",
                format!("failed to serialize HOT metrics: {err}"),
            )
        })?,
        promotion_decision: options.promotion_decision,
        manifest_version: 1,
    };
    let model_artifact_input_path = output_dir.join(HOT_MODEL_ARTIFACT_FILE);
    write_json(
        &model_artifact_input_path,
        &artifact_input,
        "hot_model_artifact_write_failed",
    )?;
    let writeback = writeback_model_artifact_file(data_dir, &model_artifact_input_path, actor)?;

    Ok(HotTrainingRunResult {
        export,
        model_path,
        model_artifact_input_path,
        writeback,
        metrics,
    })
}

pub fn runpod_input_for_manifest(
    manifest: &TrainingExportManifest,
    manifest_path: &Path,
    graph_snapshot_path: &Path,
) -> RunPodTrainingInput {
    RunPodTrainingInput {
        kind: "theorem_rustyred_training_snapshot".to_string(),
        tenant_id: manifest.tenant_id.clone(),
        export_id: manifest.export_id.clone(),
        source_graph_version: manifest.graph_version,
        snapshot_hash: manifest.snapshot_hash.clone(),
        manifest: manifest.clone(),
        local_files: TrainingSnapshotLocalFiles {
            manifest_json: manifest_path.to_path_buf(),
            graph_snapshot_json: graph_snapshot_path.to_path_buf(),
        },
        trainer_contract: json!({
            "input_format": "graph_snapshot_json_v1",
            "writeback_format": "ModelArtifactInput",
            "required_output_fields": [
                "model_id",
                "tenant_id",
                "model_type",
                "s3_uri",
                "dataset_hash",
                "source_graph_version",
                "trained_on_node_ids",
                "metrics",
                "promotion_decision",
                "manifest_version"
            ],
            "notes": [
                "Upload manifest_json and graph_snapshot_json to object storage before a remote RunPod worker consumes them.",
                "The RunPod worker must write back a ModelArtifactInput JSON, then theorem_training_run writeback records it in RedCore."
            ]
        }),
    }
}

fn read_training_snapshot_bundle(path: &Path) -> ThgResult<TrainingSnapshotBundle> {
    let raw = fs::read_to_string(path).map_err(io_error("training_snapshot_read_failed"))?;
    serde_json::from_str(&raw).map_err(|err| {
        ThgError::new(
            "training_snapshot_parse_failed",
            format!("failed to parse {}: {err}", path.display()),
        )
    })
}

fn hot_baseline_reports(
    snapshot: &GraphSnapshot,
    dataset: &HotTemporalLinkDataset,
    tenant_id: &str,
    training: &HotTrainingConfig,
    max_candidates: usize,
) -> ThgResult<Vec<HotBaselineReport>> {
    Ok(vec![
        HotBaselineReport {
            scorer: "graph_densification".to_string(),
            report: evaluate_hot_predictions(
                &score_candidate_baseline(
                    snapshot,
                    &dataset.test_labels,
                    tenant_id,
                    training,
                    max_candidates,
                    |context, request| rank_densification_candidates(context, request),
                )?,
                training.clone(),
            ),
        },
        HotBaselineReport {
            scorer: "pairformer_densification".to_string(),
            report: evaluate_hot_predictions(
                &score_candidate_baseline(
                    snapshot,
                    &dataset.test_labels,
                    tenant_id,
                    training,
                    max_candidates,
                    |context, request| {
                        rank_pairformer_densification_candidates(
                            context,
                            request,
                            PairformerConfig {
                                max_nodes: training.model.max_nodes,
                                ..PairformerConfig::default()
                            },
                        )
                    },
                )?,
                training.clone(),
            ),
        },
        HotBaselineReport {
            scorer: "reflexive_merged".to_string(),
            report: evaluate_hot_predictions(
                &score_candidate_baseline(
                    snapshot,
                    &dataset.test_labels,
                    tenant_id,
                    training,
                    max_candidates,
                    |context, request| {
                        rank_reflexive_organizing_candidates(
                            context,
                            request,
                            PairformerConfig {
                                max_nodes: training.model.max_nodes,
                                ..PairformerConfig::default()
                            },
                        )
                    },
                )?,
                training.clone(),
            ),
        },
    ])
}

fn score_candidate_baseline<F>(
    snapshot: &GraphSnapshot,
    labels: &[HotTimedPairLabel],
    tenant_id: &str,
    training: &HotTrainingConfig,
    max_candidates: usize,
    scorer: F,
) -> ThgResult<Vec<(f32, bool)>>
where
    F: Fn(&GraphSnapshot, DensificationRequest) -> ThgResult<DensificationResult>,
{
    let mut scored = Vec::with_capacity(labels.len());
    for label in labels {
        let context = context_snapshot_before(snapshot, label.as_of);
        let result = scorer(
            &context,
            DensificationRequest {
                tenant_id: tenant_id.trim().to_string(),
                seed_node_ids: vec![label.source_id.clone()],
                max_nodes: training.model.max_nodes,
                max_depth: 2,
                min_path_confidence: 0.0,
                confidence_threshold: 0.0,
                confidence_ceiling: 1.0,
                max_candidates: max_candidates.max(1),
                admission_tier: "evaluation_only".to_string(),
                model_id: "hot-baseline-eval".to_string(),
                allowed_edge_types: Vec::new(),
            },
        )?;
        scored.push((candidate_score(&result, label), label.label));
    }
    Ok(scored)
}

fn context_snapshot_before(snapshot: &GraphSnapshot, as_of: i64) -> GraphSnapshot {
    GraphSnapshot {
        version: snapshot.version,
        nodes: snapshot.nodes.clone(),
        edges: snapshot
            .edges
            .iter()
            .filter(|edge| {
                hot_temporal_edge_timestamp_for_record(edge)
                    .map(|timestamp| timestamp < as_of)
                    .unwrap_or(true)
            })
            .cloned()
            .collect(),
    }
}

fn candidate_score(result: &DensificationResult, label: &HotTimedPairLabel) -> f32 {
    result
        .candidates
        .iter()
        .find(|candidate| {
            candidate.source_id == label.source_id && candidate.target_id == label.target_id
        })
        .map(|candidate| candidate.confidence)
        .unwrap_or(0.0)
}

fn trained_on_node_ids(dataset: &HotTemporalLinkDataset) -> Vec<String> {
    let mut ids = BTreeSet::new();
    for label in &dataset.train_labels {
        ids.insert(label.source_id.clone());
        ids.insert(label.target_id.clone());
    }
    ids.into_iter().collect()
}

fn read_model_artifact_input(input_path: impl AsRef<Path>) -> ThgResult<ModelArtifactInput> {
    let input_path = input_path.as_ref();
    let raw = fs::read_to_string(input_path).map_err(io_error("model_artifact_read_failed"))?;
    serde_json::from_str(&raw).map_err(|err| {
        ThgError::new(
            "model_artifact_parse_failed",
            format!("failed to parse {}: {err}", input_path.display()),
        )
    })
}

fn snapshot_store(store: &mut RedCoreGraphStore) -> ThgResult<()> {
    store.snapshot_now().map_err(|err| {
        ThgError::new(
            "redcore_training_snapshot_failed",
            format!("failed to force RedCore snapshot: {err:?}"),
        )
    })
}

fn write_json<T: Serialize>(path: &Path, value: &T, code: &'static str) -> ThgResult<()> {
    let raw = serde_json::to_vec_pretty(value).map_err(|err| {
        ThgError::new(
            code,
            format!("failed to serialize {}: {err}", path.display()),
        )
    })?;
    fs::write(path, raw).map_err(io_error(code))
}

fn io_error(code: &'static str) -> impl FnOnce(std::io::Error) -> ThgError {
    move |err| ThgError::new(code, err.to_string())
}
