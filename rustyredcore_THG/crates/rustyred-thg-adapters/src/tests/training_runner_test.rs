use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{NeighborQuery, RedCoreGraphStore};

use crate::{
    run_hot_temporal_training, run_local_training_smoke, runpod_input_for_manifest,
    seed_hot_temporal_fixture, HotTemporalSplitConfig, HotTrainingConfig, HotTrainingRunOptions,
    TrainingSnapshotBundle, EVALUATED_BY, GRAPH_SNAPSHOT_FILE, HOT_MODEL_ARTIFACT_FILE,
    HOT_MODEL_FILE, MANIFEST_FILE, RUNPOD_INPUT_FILE,
};

#[test]
fn local_training_smoke_writes_snapshot_files_and_model_receipt() {
    let data_dir = unique_temp_dir("rustyred-training-runner-store");
    let output_dir = unique_temp_dir("rustyred-training-runner-export");

    let result = run_local_training_smoke(
        &data_dir,
        &output_dir,
        "theorem",
        "runner-export-1",
        "runner-paraphramer-v1",
        "paraphramer",
        Some("s3://theseus-training/models/runner-paraphramer-v1/model.safetensors"),
        "active",
        Some("test"),
    )
    .unwrap();

    assert_eq!(result.export.manifest_path, output_dir.join(MANIFEST_FILE));
    assert_eq!(
        result.export.graph_snapshot_path,
        output_dir.join(GRAPH_SNAPSHOT_FILE)
    );
    assert_eq!(
        result.export.runpod_input_path,
        output_dir.join(RUNPOD_INPUT_FILE)
    );

    let manifest_raw = std::fs::read_to_string(&result.export.manifest_path).unwrap();
    let manifest: crate::TrainingExportManifest = serde_json::from_str(&manifest_raw).unwrap();
    assert_eq!(manifest.counts.reasoning_traces, 2);
    assert_eq!(manifest.counts.paraphrase_pairs, 1);

    let snapshot_raw = std::fs::read_to_string(&result.export.graph_snapshot_path).unwrap();
    let bundle: TrainingSnapshotBundle = serde_json::from_str(&snapshot_raw).unwrap();
    assert_eq!(bundle.manifest.snapshot_hash, manifest.snapshot_hash);
    assert!(bundle.snapshot.nodes.len() >= manifest.counts.nodes_total);

    let runpod_raw = std::fs::read_to_string(&result.export.runpod_input_path).unwrap();
    let runpod_input: crate::RunPodTrainingInput = serde_json::from_str(&runpod_raw).unwrap();
    assert_eq!(runpod_input.kind, "theorem_rustyred_training_snapshot");
    assert_eq!(runpod_input.snapshot_hash, manifest.snapshot_hash);

    let store = RedCoreGraphStore::open(&data_dir, crate::redcore_training_options()).unwrap();
    assert!(store
        .get_node(&result.writeback.model_node_id)
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .neighbors(
                NeighborQuery::out(&result.writeback.model_node_id).with_edge_type(EVALUATED_BY)
            )
            .unwrap()
            .len(),
        1
    );

    std::fs::remove_dir_all(data_dir).ok();
    std::fs::remove_dir_all(output_dir).ok();
}

#[test]
fn runpod_input_names_expected_writeback_schema() {
    let data_dir = unique_temp_dir("rustyred-training-runpod-contract-store");
    let output_dir = unique_temp_dir("rustyred-training-runpod-contract-export");
    let export = run_local_training_smoke(
        &data_dir,
        &output_dir,
        "theorem",
        "runner-export-contract",
        "runner-contract-v1",
        "paraphramer",
        None,
        "shadow",
        Some("test"),
    )
    .unwrap()
    .export;

    let input = runpod_input_for_manifest(
        &export.manifest,
        &export.manifest_path,
        &export.graph_snapshot_path,
    );
    let required = input.trainer_contract["required_output_fields"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert!(required.contains(&"dataset_hash"));
    assert!(required.contains(&"trained_on_node_ids"));
    assert!(required.contains(&"metrics"));

    std::fs::remove_dir_all(data_dir).ok();
    std::fs::remove_dir_all(output_dir).ok();
}

#[test]
fn hot_temporal_training_smoke_writes_model_metrics_and_receipt() {
    let data_dir = unique_temp_dir("rustyred-hot-training-runner-store");
    let output_dir = unique_temp_dir("rustyred-hot-training-runner-export");

    let fixture = seed_hot_temporal_fixture(&data_dir, "theorem", Some("test")).unwrap();
    assert_eq!(fixture.edge_ids.len(), 9);

    let result = run_hot_temporal_training(
        &data_dir,
        &output_dir,
        "theorem",
        "hot-runner-export-1",
        HotTrainingRunOptions {
            model_id: "runner-hot-v1".to_string(),
            promotion_decision: "shadow".to_string(),
            training: HotTrainingConfig {
                epochs: 80,
                learning_rate: 0.03,
                batch_size: 4,
                ..HotTrainingConfig::default()
            },
            split: HotTemporalSplitConfig {
                holdout_fraction: 0.25,
                negatives_per_positive: 1,
                max_positive_edges: 32,
            },
            max_baseline_candidates: 16,
            ..HotTrainingRunOptions::default()
        },
        Some("test"),
    )
    .unwrap();

    assert_eq!(result.model_path, output_dir.join(HOT_MODEL_FILE));
    assert_eq!(
        result.model_artifact_input_path,
        output_dir.join(HOT_MODEL_ARTIFACT_FILE)
    );
    assert!(result.model_path.exists());
    assert!(result.model_artifact_input_path.exists());
    assert_eq!(result.metrics.run_kind, "hot_temporal_link_training");
    assert_eq!(result.metrics.baseline_reports.len(), 3);
    assert!(result
        .metrics
        .baseline_reports
        .iter()
        .any(|report| report.scorer == "pairformer_densification"));
    assert!(result.metrics.train_examples > 0);
    assert!(result.metrics.test_examples > 0);
    assert_eq!(
        result.metrics.deterministic_report.examples,
        result.metrics.test_examples
    );
    assert!(result.metrics.burn_path_policy.contains("promote only after"));

    let artifact_raw = std::fs::read_to_string(&result.model_artifact_input_path).unwrap();
    let artifact: crate::ModelArtifactInput = serde_json::from_str(&artifact_raw).unwrap();
    assert_eq!(artifact.model_type, "hot-temporal-link/native-mlp");
    assert_eq!(artifact.model_id, "runner-hot-v1");
    assert_eq!(artifact.metrics["run_kind"], "hot_temporal_link_training");

    let store = RedCoreGraphStore::open(&data_dir, crate::redcore_training_options()).unwrap();
    assert!(store
        .get_node(&result.writeback.model_node_id)
        .unwrap()
        .is_some());
    assert_eq!(
        store
            .neighbors(
                NeighborQuery::out(&result.writeback.model_node_id).with_edge_type(EVALUATED_BY)
            )
            .unwrap()
            .len(),
        1
    );

    std::fs::remove_dir_all(data_dir).ok();
    std::fs::remove_dir_all(output_dir).ok();
}

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{unique}"))
}
