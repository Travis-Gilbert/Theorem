use serde_json::{json, Map};

use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery};
use theorem_harness_core::AffordanceReceipt;

use crate::types::affordance_node_id;
use crate::{
    record_invocation, register_connector, ConnectorManifest, InvocationRecordRequest,
    ToolManifest, CONNECTOR_FAMILY, DEFAULT_BASE_FITNESS, PRODUCED_OUTCOME, SEQUENCED_WITH,
    SERVED_TASK,
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
                family: CONNECTOR_FAMILY.to_string(),
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
                family: CONNECTOR_FAMILY.to_string(),
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

fn invocation(selected: &str, task: &str, previous: Option<&str>) -> InvocationRecordRequest {
    InvocationRecordRequest {
        tenant_id: "theorem".to_string(),
        task_type: task.to_string(),
        candidate_affordance_ids: vec![
            "github.create_issue".to_string(),
            "github.search_code".to_string(),
        ],
        selected_affordance_id: selected.to_string(),
        outcome_value: 1.0,
        outcome_weight: 8.0,
        outcome_label: "success".to_string(),
        previous_affordance_id: previous.map(str::to_string),
        query_text: "do the thing".to_string(),
        recorded_at_ms: None,
    }
}

#[test]
fn record_invocation_writes_receipt_edges_and_updates_fitness() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();

    let result = record_invocation(
        &mut store,
        invocation("github.create_issue", "triage_issue", None),
        Some("test"),
    )
    .unwrap();

    // Receipt node exists and the decision's graph version was recorded.
    assert!(store.get_node(&result.receipt_node_id).is_some());
    assert!(!result.receipt_hash.is_empty());
    // graph_version is the pre-write version the selection was made against.
    // (InMemory starts at version 0 and advances per commit, so registration
    // has already moved it past 0.)
    assert!(result.graph_version >= 1);

    let selected_node = affordance_node_id("theorem", "github.create_issue");
    let served = store.neighbors(NeighborQuery::out(&selected_node).with_edge_type(SERVED_TASK));
    assert_eq!(served.len(), 1, "one SERVED_TASK edge to the task type");
    let produced =
        store.neighbors(NeighborQuery::out(&selected_node).with_edge_type(PRODUCED_OUTCOME));
    assert_eq!(
        produced.len(),
        1,
        "one PRODUCED_OUTCOME edge to the receipt"
    );

    assert!(
        result.effective_fitness > DEFAULT_BASE_FITNESS,
        "a positive outcome raises fitness above the base"
    );
}

#[test]
fn confirmation_required_records_receipt_without_changing_fitness() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();

    let selected_node_id = affordance_node_id("theorem", "github.create_issue");
    let before = store.get_node(&selected_node_id).cloned().unwrap();
    let before_fitness = before.properties["fitness"].as_f64().unwrap();
    assert!(before.properties.get("fitness_updated_at_ms").is_none());

    let mut denied = invocation("github.create_issue", "triage_issue", None);
    denied.outcome_value = 0.0;
    denied.outcome_weight = 1.0;
    denied.outcome_label = "confirmation_required".to_string();

    let result = record_invocation(&mut store, denied, Some("test")).unwrap();
    let after = store.get_node(&selected_node_id).cloned().unwrap();
    let after_fitness = after.properties["fitness"].as_f64().unwrap();
    assert_eq!(after_fitness, before_fitness);
    assert!(after.properties.get("fitness_updated_at_ms").is_none());
    assert_eq!(result.effective_fitness, DEFAULT_BASE_FITNESS);

    let receipt = store.get_node(&result.receipt_node_id).cloned().unwrap();
    assert_eq!(receipt.properties["outcome_label"], "confirmation_required");
    assert_eq!(receipt.properties["fitness_observed"], json!(false));
}

#[test]
fn non_policy_failure_still_lowers_fitness() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();

    let mut failed = invocation("github.create_issue", "triage_issue", None);
    failed.outcome_value = 0.0;
    failed.outcome_weight = 1.0;
    failed.outcome_label = "handler_failed".to_string();

    let result = record_invocation(&mut store, failed, Some("test")).unwrap();
    assert!(
        result.effective_fitness < DEFAULT_BASE_FITNESS,
        "ordinary failed execution should still lower fitness"
    );
}

#[test]
fn sequenced_with_links_consecutive_selections() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();

    record_invocation(
        &mut store,
        invocation("github.create_issue", "triage_issue", None),
        Some("test"),
    )
    .unwrap();
    let second = record_invocation(
        &mut store,
        invocation(
            "github.search_code",
            "triage_issue",
            Some("github.create_issue"),
        ),
        Some("test"),
    )
    .unwrap();

    assert!(second.sequenced_with_edge_id.is_some());
    let from = affordance_node_id("theorem", "github.create_issue");
    let seq = store.neighbors(NeighborQuery::out(&from).with_edge_type(SEQUENCED_WITH));
    assert_eq!(seq.len(), 1, "create_issue is sequenced with search_code");
}

#[test]
fn record_invocation_rejects_unregistered_affordance() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, manifest(), Some("test")).unwrap();
    let err = record_invocation(
        &mut store,
        invocation("github.delete_repo", "triage_issue", None),
        Some("test"),
    );
    assert!(
        err.is_err(),
        "cannot record an outcome for an unregistered affordance"
    );
}

#[test]
fn receipt_hashing_is_content_addressed() {
    // The invocation receipt reuses the harness-core content-addressed receipt:
    // identical inputs hash identically, any change changes the hash.
    let mut payload = Map::new();
    payload.insert("task_type".to_string(), json!("triage_issue"));
    payload.insert("outcome_value".to_string(), json!(1.0));
    let a = AffordanceReceipt::new(
        "github",
        "github.create_issue",
        "input-hash",
        payload.clone(),
    );
    let b = AffordanceReceipt::new(
        "github",
        "github.create_issue",
        "input-hash",
        payload.clone(),
    );
    assert_eq!(a.receipt_hash, b.receipt_hash);

    let mut changed = payload.clone();
    changed.insert("outcome_value".to_string(), json!(0.0));
    let c = AffordanceReceipt::new("github", "github.create_issue", "input-hash", changed);
    assert_ne!(a.receipt_hash, c.receipt_hash);
}
