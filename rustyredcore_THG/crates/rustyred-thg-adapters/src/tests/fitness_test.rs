use serde_json::json;

use rustyred_thg_core::{now_ms, InMemoryGraphStore, NodeRecord};

use crate::{
    effective_fitness, record_fitness, upsert_adapter, AdapterFitnessRecordRequest, LoraAdapter,
};

#[test]
fn fitness_signal_updates_adapter_ema_and_records_edge() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new("run_step:1", ["RunStep"], json!({})))
        .unwrap();
    upsert_adapter(&mut store, fixture_adapter("adapter-a", 0.5), None, None).unwrap();

    let result = record_fitness(
        &mut store,
        AdapterFitnessRecordRequest {
            adapter_id: "adapter-a".to_string(),
            source_node_id: "run_step:1".to_string(),
            value: 1.0,
            weight: 1.0,
            kind: "scorer_agreement".to_string(),
            recorded_at_ms: Some(now_ms()),
        },
        Some("test"),
    )
    .unwrap();

    assert!(result.effective_fitness > 0.5);
    assert!(store.get_edge(&result.edge_id).is_some());
}

#[test]
fn fitness_decays_at_query_time_without_rewriting_history() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new("run_step:1", ["RunStep"], json!({})))
        .unwrap();
    upsert_adapter(&mut store, fixture_adapter("adapter-a", 0.9), None, None).unwrap();
    record_fitness(
        &mut store,
        AdapterFitnessRecordRequest {
            adapter_id: "adapter-a".to_string(),
            source_node_id: "run_step:1".to_string(),
            value: 1.0,
            weight: 1.0,
            kind: "manual".to_string(),
            recorded_at_ms: Some(now_ms() - 28 * 86_400_000),
        },
        Some("test"),
    )
    .unwrap();

    let adapter = crate::find_adapter_by_id(&store, "adapter-a")
        .unwrap()
        .unwrap();
    let decayed = effective_fitness(&store, &adapter).unwrap();
    assert!(decayed >= crate::DEFAULT_FITNESS_EPSILON);
    assert!(decayed < adapter.fitness);
}

fn fixture_adapter(adapter_id: &str, fitness: f32) -> LoraAdapter {
    LoraAdapter {
        adapter_id: adapter_id.to_string(),
        tenant_id: "tenant-a".to_string(),
        base_model_sha: "sha-base".to_string(),
        rank: 16,
        target_modules: vec!["q_proj".to_string(), "v_proj".to_string()],
        s3_uri: format!("s3://bucket/{adapter_id}/adapter_model.safetensors"),
        training_object_ids: vec![],
        version: 1,
        fitness,
        created_at_ms: 1,
        manifest_version: 1,
    }
}
