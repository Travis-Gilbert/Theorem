use serde_json::json;

use rustyred_thg_core::{EdgeRecord, InMemoryGraphStore, NodeRecord};

use crate::{
    find_adapters_for, supersede_adapter, upsert_adapter, AdapterFindRequest, LoraAdapter,
    SHARED_WITH,
};

#[test]
fn routing_uses_ppr_then_fitness_for_ordering_and_supersession() {
    let mut store = seeded_store();
    upsert_adapter(
        &mut store,
        fixture_adapter("adapter-low", "tenant-a", 1, 0.55),
        None,
        None,
    )
    .unwrap();
    upsert_adapter(
        &mut store,
        fixture_adapter("adapter-high", "tenant-a", 2, 0.9),
        None,
        None,
    )
    .unwrap();
    supersede_adapter(&mut store, "adapter-low", "adapter-high", false, None).unwrap();

    let refs = find_adapters_for(&store, &find_request(false)).unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(refs[0].adapter.adapter_id, "adapter-high");

    let refs = find_adapters_for(&store, &find_request(true)).unwrap();
    assert_eq!(refs[0].adapter.adapter_id, "adapter-high");
    assert!(refs
        .iter()
        .any(|item| item.adapter.adapter_id == "adapter-low"));
}

#[test]
fn routing_excludes_cross_tenant_adapters_until_shared() {
    let mut store = seeded_store();
    upsert_adapter(
        &mut store,
        fixture_adapter("tenant-local", "tenant-a", 1, 0.6),
        None,
        None,
    )
    .unwrap();
    let shared = upsert_adapter(
        &mut store,
        fixture_adapter("tenant-b-shared", "tenant-b", 1, 0.99),
        None,
        None,
    )
    .unwrap();

    let refs = find_adapters_for(&store, &find_request(false)).unwrap();
    assert!(refs
        .iter()
        .all(|item| item.adapter.adapter_id != "tenant-b-shared"));

    store
        .upsert_edge(EdgeRecord::new(
            "edge:share-b-to-a",
            shared.node_id,
            SHARED_WITH,
            "tenant:tenant-a",
            json!({ "weight": 0.5 }),
        ))
        .unwrap();
    let refs = find_adapters_for(&store, &find_request(false)).unwrap();
    assert!(refs
        .iter()
        .any(|item| item.adapter.adapter_id == "tenant-b-shared"));
}

fn seeded_store() -> InMemoryGraphStore {
    let mut store = InMemoryGraphStore::new();
    for node_id in ["object:1", "tenant:tenant-a"] {
        store
            .upsert_node(NodeRecord::new(node_id, ["Object"], json!({})))
            .unwrap();
    }
    store
}

fn fixture_adapter(adapter_id: &str, tenant_id: &str, version: u32, fitness: f32) -> LoraAdapter {
    LoraAdapter {
        adapter_id: adapter_id.to_string(),
        tenant_id: tenant_id.to_string(),
        base_model_sha: "sha-base".to_string(),
        rank: 16,
        target_modules: vec!["q_proj".to_string(), "v_proj".to_string()],
        s3_uri: format!("s3://bucket/{adapter_id}/adapter_model.safetensors"),
        training_object_ids: vec![1],
        version,
        fitness,
        created_at_ms: version as i64,
        manifest_version: 1,
    }
}

fn find_request(include_superseded: bool) -> AdapterFindRequest {
    AdapterFindRequest {
        tenant_id: "tenant-a".to_string(),
        seed_node_ids: vec!["object:1".to_string()],
        k: 5,
        base_model_sha: Some("sha-base".to_string()),
        include_superseded,
        min_fitness: Some(0.0),
        ppr_damping: 0.85,
        ppr_max_iter: 30,
        shared_weight: Some(0.5),
    }
}
