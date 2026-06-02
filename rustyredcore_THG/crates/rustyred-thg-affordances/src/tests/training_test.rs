use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::json;

use rustyred_thg_core::{
    InMemoryGraphStore, NeighborQuery, RedCoreDurability, RedCoreGraphStore, RedCoreOptions,
};

use crate::types::affordance_node_id;
use crate::{
    export_affordance_training_view, pairformer_validation_gate, record_invocation,
    register_connector, register_pairformer_artifact, ConnectorManifest, InvocationRecordRequest,
    PairformerArtifactInput, ToolManifest, EVALUATED_BY, TRAINED_ON,
};

fn manifest() -> ConnectorManifest {
    ConnectorManifest {
        tenant_id: "theorem".to_string(),
        server_id: "github".to_string(),
        label: "GitHub".to_string(),
        tools: vec![
            ToolManifest {
                name: "create_issue".to_string(),
                label: String::new(),
                description: "open an issue".to_string(),
                input_schema: json!({}),
                permissions: vec![],
                cost: json!({}),
                writeback_policy: "write".to_string(),
                tags: vec![],
                description_embedding: None,
            },
            ToolManifest {
                name: "search_code".to_string(),
                label: String::new(),
                description: "search code".to_string(),
                input_schema: json!({}),
                permissions: vec![],
                cost: json!({}),
                writeback_policy: "read-only".to_string(),
                tags: vec![],
                description_embedding: None,
            },
        ],
    }
}

fn invocation(selected: &str, task: &str, value: f32) -> InvocationRecordRequest {
    InvocationRecordRequest {
        tenant_id: "theorem".to_string(),
        task_type: task.to_string(),
        candidate_affordance_ids: vec![
            "github.create_issue".to_string(),
            "github.search_code".to_string(),
        ],
        selected_affordance_id: selected.to_string(),
        outcome_value: value,
        outcome_weight: 6.0,
        outcome_label: "recorded".to_string(),
        previous_affordance_id: None,
        query_text: "do the thing".to_string(),
        recorded_at_ms: Some(1_000),
    }
}

#[test]
fn affordance_substrate_survives_redcore_reopen_and_exports() {
    let data_dir = unique_temp_dir("rustyred-affordance-substrate");
    let options = RedCoreOptions {
        durability: RedCoreDurability::AofAlways,
        snapshot_interval_writes: 100,
        strict_acid: true,
    };

    {
        let mut store = RedCoreGraphStore::open(&data_dir, options.clone()).unwrap();
        register_connector(&mut store, manifest(), Some("test")).unwrap();
        record_invocation(&mut store, invocation("github.create_issue", "triage", 1.0), Some("test"))
            .unwrap();
        record_invocation(&mut store, invocation("github.search_code", "search", 0.8), Some("test"))
            .unwrap();
        store.snapshot_now().unwrap();
        assert_eq!(store.status().durability, "aof_always");
    }

    {
        let mut store = RedCoreGraphStore::open(&data_dir, options).unwrap();

        // Affordances and their outcome edges survive a durable reopen.
        let create_node = affordance_node_id("theorem", "github.create_issue");
        assert!(store.get_node(&create_node).unwrap().is_some());
        let produced = store
            .neighbors(NeighborQuery::out(&create_node).with_edge_type(crate::PRODUCED_OUTCOME))
            .unwrap();
        assert_eq!(produced.len(), 1, "PRODUCED_OUTCOME edge survived reopen");

        // Export the affordance-outcome ranking pairs from the frozen snapshot.
        let snapshot = store.graph_snapshot();
        let export = export_affordance_training_view(&snapshot, "theorem", "aff-export-1").unwrap();
        assert_eq!(export.ranking_pair_count, 2);
        assert_eq!(export.distinct_task_types, 2);
        assert!(export.graph_version > 0);
        assert!(!export.snapshot_hash.is_empty());

        // Export is deterministic for the same snapshot.
        let export_again =
            export_affordance_training_view(&snapshot, "theorem", "aff-export-1").unwrap();
        assert_eq!(export, export_again);

        // Write a trained pairformer artifact back into the graph.
        let writeback = register_pairformer_artifact(
            &mut store,
            PairformerArtifactInput {
                model_id: "pairformer-v1".to_string(),
                tenant_id: "theorem".to_string(),
                model_type: "pairformer".to_string(),
                s3_uri: "s3://theseus-training/models/pairformer-v1/model.safetensors".to_string(),
                dataset_hash: export.snapshot_hash.clone(),
                source_graph_version: export.graph_version,
                trained_on_node_ids: vec![
                    affordance_node_id("theorem", "github.create_issue"),
                    affordance_node_id("theorem", "github.search_code"),
                ],
                metrics: json!({ "held_out_token_reduction": 0.18, "z_score": 1.9 }),
                promotion_decision: "active".to_string(),
                manifest_version: 1,
            },
            Some("test"),
        )
        .unwrap();

        assert!(store.get_node(&writeback.model_node_id).unwrap().is_some());
        assert!(store.get_node(&writeback.evaluation_node_id).unwrap().is_some());
        assert_eq!(
            store
                .neighbors(NeighborQuery::out(&writeback.model_node_id).with_edge_type(TRAINED_ON))
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            store
                .neighbors(NeighborQuery::out(&writeback.model_node_id).with_edge_type(EVALUATED_BY))
                .unwrap()
                .len(),
            1
        );
    }

    std::fs::remove_dir_all(data_dir).ok();
}

#[test]
fn empty_training_view_is_rejected() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();
    // No invocations recorded -> no ranking pairs -> export refuses.
    let snapshot = store.snapshot();
    let err = export_affordance_training_view(&snapshot, "theorem", "empty");
    assert!(err.is_err());
}

#[test]
fn pairformer_refuses_active_without_evaluation() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();
    let err = register_pairformer_artifact(
        &mut store,
        PairformerArtifactInput {
            model_id: "pairformer-x".to_string(),
            tenant_id: "theorem".to_string(),
            model_type: "pairformer".to_string(),
            s3_uri: "s3://models/pairformer-x/model.bin".to_string(),
            dataset_hash: "hash".to_string(),
            source_graph_version: 1,
            trained_on_node_ids: vec![],
            metrics: json!({}),
            promotion_decision: "active".to_string(),
            manifest_version: 1,
        },
        Some("test"),
    );
    assert!(err.is_err(), "promotion to active requires evaluation metrics");
}

#[test]
fn validation_gate_wraps_compare_modes() {
    let lines = vec![
        r#"{"task_completion": true, "pairformer_mode": "off", "task_category": "code", "total_input_tokens": 1000, "total_output_tokens": 200, "total_tool_calls": 8}"#,
        r#"{"task_completion": true, "pairformer_mode": "full", "task_category": "code", "total_input_tokens": 600, "total_output_tokens": 150, "total_tool_calls": 5}"#,
    ];
    let verdict = pairformer_validation_gate(lines, Some("off"), Some("full")).unwrap();
    assert!(verdict.get("status").is_some());
    // With only one completed session per arm, the 90%-confidence bar is not met.
    assert_eq!(verdict.get("passed").and_then(|value| value.as_bool()), Some(false));
}

fn unique_temp_dir(label: &str) -> std::path::PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("{label}-{unique}"))
}
