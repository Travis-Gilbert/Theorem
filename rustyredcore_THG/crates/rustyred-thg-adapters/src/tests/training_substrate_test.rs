use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use rustyred_thg_core::{NeighborQuery, RedCoreDurability, RedCoreGraphStore, RedCoreOptions};

use crate::{
    export_training_snapshot, register_gnn_export_dir, register_model_artifact,
    register_training_fixture, GnnExportImportOptions, ModelArtifactInput, EVALUATED_BY,
    GNN_ENTITY_LABEL, TRAINED_ON,
};

#[test]
fn durable_training_fixture_exports_and_writeback_survive_redcore_reopen() {
    let data_dir = unique_temp_dir("rustyred-training-substrate");
    let options = RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        snapshot_interval_writes: 100,
        strict_acid: true,
    };

    let fixture = {
        let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
        let fixture = register_training_fixture(&mut store, "theorem", Some("test")).unwrap();
        store.snapshot_now().unwrap();
        assert_eq!(store.status().durability, "aof_always");
        fixture
    };

    {
        let mut store = RedCoreGraphStore::open(&data_dir, options).unwrap();
        let snapshot = store.graph_snapshot();
        let manifest =
            export_training_snapshot(&snapshot, "theorem", "training-export-fixture").unwrap();

        assert_eq!(manifest.counts.objects, 3);
        assert_eq!(manifest.counts.reasoning_traces, 2);
        assert_eq!(manifest.counts.trace_steps, 4);
        assert_eq!(manifest.counts.postmortems, 1);
        assert_eq!(manifest.counts.artifacts, 1);
        assert_eq!(manifest.counts.training_packs, 1);
        assert_eq!(manifest.counts.paraphrase_pairs, 1);
        assert_eq!(manifest.counts.gnn_exports, 1);
        assert_eq!(manifest.counts.lora_adapters, 1);
        assert!(manifest.graph_version > 0);
        assert!(!manifest.snapshot_hash.is_empty());
        assert!(manifest
            .reasoning_trace_ids
            .contains(&fixture.reasoning_trace_node_ids[0]));
        assert!(manifest.artifact_ids.contains(&fixture.artifact_node_id));

        let writeback = register_model_artifact(
            &mut store,
            ModelArtifactInput {
                model_id: "paraphramer-fixture-v1".to_string(),
                tenant_id: "theorem".to_string(),
                model_type: "paraphramer".to_string(),
                s3_uri: "s3://theseus-training/models/paraphramer-fixture-v1/model.safetensors"
                    .to_string(),
                dataset_hash: manifest.snapshot_hash.clone(),
                source_graph_version: manifest.graph_version,
                trained_on_node_ids: vec![
                    fixture.training_pack_node_id.clone(),
                    fixture.paraphrase_pair_node_id.clone(),
                ],
                metrics: json!({
                    "semantic_preservation": 0.98,
                    "citation_preservation": 1.0,
                    "hallucination_rate": 0.0
                }),
                promotion_decision: "active".to_string(),
                manifest_version: 1,
            },
            Some("test"),
        )
        .unwrap();

        assert!(store.get_node(&writeback.model_node_id).unwrap().is_some());
        assert!(store
            .get_node(&writeback.evaluation_node_id)
            .unwrap()
            .is_some());
        assert_eq!(
            store
                .neighbors(NeighborQuery::out(&writeback.model_node_id).with_edge_type(TRAINED_ON))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store
                .neighbors(
                    NeighborQuery::out(&writeback.model_node_id).with_edge_type(EVALUATED_BY)
                )
                .unwrap()
                .len(),
            1
        );
    }

    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn gnn_export_import_materializes_entities_triples_and_artifacts() {
    let data_dir = unique_temp_dir("rustyred-training-gnn-import-store");
    let export_dir = unique_temp_dir("rustyred-training-gnn-import-export");
    std::fs::create_dir_all(&export_dir).unwrap();
    std::fs::write(
        export_dir.join("manifest.json"),
        r#"{
  "schema_version": "v1",
  "exported_at": "2026-04-25T03:02:24Z",
  "generator": "export_gnn_data --include-features --upload",
  "graph_snapshot": {"object_count": 3, "edge_count": 3, "hash": "fixture"},
  "files": {
    "entity_map.tsv": {"rows": 3},
    "triples.tsv": {"rows": 3},
    "temporal_triples.tsv": {"rows": 2},
    "sha_to_object_id.json": {"rows": 4},
    "node_features.npy": {"shape": [3, 384], "dtype": "float32"}
  }
}"#,
    )
    .unwrap();
    std::fs::write(
        export_dir.join("training_metadata.json"),
        r#"{"model":"geomoe-rich-v1","embedding_dim":128}"#,
    )
    .unwrap();
    std::fs::write(
        export_dir.join("export_metadata.json"),
        r#"{"entity_count":3,"triple_count":3,"relation_count":2}"#,
    )
    .unwrap();
    std::fs::write(
        export_dir.join("entity_map.tsv"),
        "sha_hash\ttitle\tobject_type\nsha-a\tAlpha\tnote\nsha-b\tBeta\tsource\nsha-c\tGamma\tnote\n",
    )
    .unwrap();
    std::fs::write(
        export_dir.join("sha_to_object_id.json"),
        r#"{"sha-a":1,"sha-b":2,"sha-c":3,"sha-d":4}"#,
    )
    .unwrap();
    std::fs::write(
        export_dir.join("triples.tsv"),
        "head\trelation\ttail\nsha-a\tstructural\tsha-b\nsha-b\tspatial:adjacent\tsha-c\nsha-missing\tstructural\tsha-c\n",
    )
    .unwrap();
    std::fs::write(
        export_dir.join("temporal_triples.tsv"),
        "head\trelation\ttail\ttime_bucket\tweight\nsha-c\tcausal\tsha-d\t2026-W14\t0.8\nsha-missing\tcausal\tsha-a\t2026-W14\t0.5\n",
    )
    .unwrap();

    let options = RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        snapshot_interval_writes: 100,
        strict_acid: true,
    };
    let mut store = RedCoreGraphStore::open(&data_dir, options).unwrap();
    let result = register_gnn_export_dir(
        &mut store,
        &export_dir,
        "theorem",
        "fixture-gnn-export",
        GnnExportImportOptions {
            batch_size: 2,
            max_entities: None,
            max_triples: None,
            max_temporal_triples: None,
        },
        Some("test"),
    )
    .unwrap();

    assert_eq!(result.imported_entity_nodes, 3);
    assert_eq!(result.imported_sha_map_nodes, 1);
    assert_eq!(result.imported_triple_edges, 2);
    assert_eq!(result.imported_temporal_edges, 1);
    assert_eq!(result.skipped_triples, 1);
    assert_eq!(result.skipped_temporal_triples, 1);
    assert_eq!(result.artifact_nodes, 5);
    assert!(result.transaction_count > 1);

    store.snapshot_now().unwrap();
    let snapshot = store.graph_snapshot();
    let manifest = export_training_snapshot(&snapshot, "theorem", "fixture-export").unwrap();
    assert_eq!(manifest.counts.objects, 4);
    assert_eq!(manifest.counts.gnn_exports, 1);
    assert_eq!(manifest.counts.training_packs, 1);
    assert_eq!(manifest.counts.artifacts, 5);
    assert!(manifest
        .selected_labels
        .iter()
        .any(|label| label == GNN_ENTITY_LABEL));
    assert!(manifest
        .selected_edge_types
        .iter()
        .any(|edge_type| edge_type == "GNN_STRUCTURAL"));
    assert!(manifest
        .selected_edge_types
        .iter()
        .any(|edge_type| edge_type == "GNN_SPATIAL_ADJACENT"));
    assert!(manifest
        .selected_edge_types
        .iter()
        .any(|edge_type| edge_type == "GNN_TEMPORAL_CAUSAL"));

    std::fs::remove_dir_all(data_dir).ok();
    std::fs::remove_dir_all(export_dir).ok();
}

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{unique}"))
}
