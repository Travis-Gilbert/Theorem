use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use rustyred_thg_core::{NeighborQuery, RedCoreDurability, RedCoreGraphStore, RedCoreOptions};

use crate::{
    export_training_snapshot, register_model_artifact, register_training_fixture,
    ModelArtifactInput, EVALUATED_BY, TRAINED_ON,
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

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{unique}"))
}
