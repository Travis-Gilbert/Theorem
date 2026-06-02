//! Operator-facing training snapshot and writeback helpers.
//!
//! These helpers keep provider orchestration outside the graph engine while
//! giving Theorem a concrete RedCore -> trainer -> RedCore seam.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use rustyred_thg_core::{
    GraphSnapshot, RedCoreDurability, RedCoreGraphStore, RedCoreOptions, ThgError, ThgResult,
};

use crate::training_substrate::{
    export_training_snapshot, register_model_artifact, register_training_fixture,
    ModelArtifactInput, ModelWritebackResult, TrainingExportManifest, TrainingFixtureResult,
};
use crate::{paraphrase_pair_node_id, training_pack_node_id};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const GRAPH_SNAPSHOT_FILE: &str = "graph_snapshot.json";
pub const RUNPOD_INPUT_FILE: &str = "runpod_input.json";

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
