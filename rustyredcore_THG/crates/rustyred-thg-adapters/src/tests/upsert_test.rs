use serde_json::json;

use rustyred_thg_core::{Direction, InMemoryGraphStore, NeighborQuery, NodeRecord};

use crate::{upsert_adapter, LoraAdapter, DERIVED_FROM, TRAINED_ON};

#[test]
fn upsert_registers_adapter_and_training_edges_idempotently() {
    let mut store = InMemoryGraphStore::new();
    store
        .upsert_node(NodeRecord::new("object:1", ["Object"], json!({})))
        .unwrap();
    store
        .upsert_node(NodeRecord::new("object:2", ["Object"], json!({})))
        .unwrap();

    let adapter = fixture_adapter("adapter-a", 1, vec![1, 2], 0.5);
    let first = upsert_adapter(&mut store, adapter.clone(), None, Some("test")).unwrap();
    assert_eq!(first.edge_count, 2);
    assert!(store.get_node(&first.node_id).is_some());
    assert_eq!(
        store
            .neighbors(NeighborQuery {
                node_id: first.node_id.clone(),
                direction: Direction::Out,
                edge_type: Some(TRAINED_ON.to_string()),
                include_expired: false,
            })
            .len(),
        2
    );

    upsert_adapter(&mut store, adapter, None, Some("test")).unwrap();
    assert_eq!(store.stats().edges_total, 2);
}

#[test]
fn upsert_records_derived_from_chain() {
    let mut store = InMemoryGraphStore::new();
    let base = fixture_adapter("adapter-v1", 1, vec![], 0.5);
    let next = fixture_adapter("adapter-v2", 2, vec![], 0.5);
    upsert_adapter(&mut store, base, None, Some("test")).unwrap();
    let result = upsert_adapter(&mut store, next, Some("adapter-v1"), Some("test")).unwrap();

    let edges = store.neighbors(NeighborQuery::out(result.node_id).with_edge_type(DERIVED_FROM));
    assert_eq!(edges.len(), 1);
    assert!(edges[0].node_id.ends_with(":adapter-v1"));
}

fn fixture_adapter(
    adapter_id: &str,
    version: u32,
    training_object_ids: Vec<i64>,
    fitness: f32,
) -> LoraAdapter {
    LoraAdapter {
        adapter_id: adapter_id.to_string(),
        tenant_id: "tenant-a".to_string(),
        base_model_sha: "sha-base".to_string(),
        rank: 16,
        target_modules: vec!["q_proj".to_string(), "v_proj".to_string()],
        s3_uri: format!("s3://bucket/{adapter_id}/adapter_model.safetensors"),
        training_object_ids,
        version,
        fitness,
        created_at_ms: 1,
        manifest_version: 1,
    }
}
