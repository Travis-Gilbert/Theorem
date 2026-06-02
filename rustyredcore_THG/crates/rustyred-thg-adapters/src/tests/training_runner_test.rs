use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{NeighborQuery, RedCoreGraphStore};

use crate::{
    run_local_training_smoke, runpod_input_for_manifest, TrainingSnapshotBundle, EVALUATED_BY,
    GRAPH_SNAPSHOT_FILE, MANIFEST_FILE, RUNPOD_INPUT_FILE,
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

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{unique}"))
}
