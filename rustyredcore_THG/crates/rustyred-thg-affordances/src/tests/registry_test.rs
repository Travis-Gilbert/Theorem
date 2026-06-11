use serde_json::json;

use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeQuery};

use crate::types::{affordance_node_id, Affordance};
use crate::{
    record_invocation, register_builtin_affordances, register_connector,
    register_theseus_app_affordances, ConnectorManifest, InvocationRecordRequest, ToolManifest,
    AFFORDANCE_LABEL, DEFAULT_BASE_FITNESS, OFFERS, THEOREM_GRPC_CODE_INGEST_TIMEOUT_MS,
    THEOREM_GRPC_MAX_TIMEOUT_MS, THEOREM_GRPC_SERVER_ID, THEOREM_GRPC_TIMEOUT_MS,
};

fn tool(name: &str, policy: &str, tags: &[&str]) -> ToolManifest {
    ToolManifest {
        name: name.to_string(),
        label: String::new(),
        description: format!("the {name} tool"),
        input_schema: json!({ "type": "object" }),
        permissions: vec!["repo_read".to_string()],
        cost: json!({ "latency_class": "fast" }),
        writeback_policy: policy.to_string(),
        tags: tags.iter().map(|tag| tag.to_string()).collect(),
        description_embedding: None,
    }
}

fn github_manifest() -> ConnectorManifest {
    ConnectorManifest {
        tenant_id: "theorem".to_string(),
        server_id: "github".to_string(),
        label: "GitHub".to_string(),
        tools: vec![
            tool("create_issue", "write", &["write", "issues"]),
            tool("search_code", "read-only", &["read"]),
            tool("get_file", "read-only", &["read"]),
        ],
    }
}

#[test]
fn register_connector_creates_one_node_per_tool() {
    let mut store = InMemoryGraphStore::new();
    let result = register_connector(&mut store, github_manifest(), Some("test")).unwrap();

    assert_eq!(result.affordance_node_ids.len(), 3);
    assert!(store.get_node(&result.connector_node_id).is_some());

    let affordances = store.query_nodes(NodeQuery::label(AFFORDANCE_LABEL).with_limit(100));
    assert_eq!(affordances.len(), 3);

    let offers =
        store.neighbors(NeighborQuery::out(&result.connector_node_id).with_edge_type(OFFERS));
    assert_eq!(offers.len(), 3);

    // Tenant-scoped, deterministic node ids.
    assert!(result
        .affordance_node_ids
        .contains(&affordance_node_id("theorem", "github.create_issue")));
}

#[test]
fn re_registration_is_idempotent_and_preserves_learned_fitness() {
    let mut store = InMemoryGraphStore::new();
    register_connector(&mut store, github_manifest(), Some("test")).unwrap();

    // Record a strong positive outcome so create_issue's fitness rises.
    record_invocation(
        &mut store,
        InvocationRecordRequest {
            tenant_id: "theorem".to_string(),
            task_type: "triage_issue".to_string(),
            candidate_affordance_ids: vec!["github.create_issue".to_string()],
            selected_affordance_id: "github.create_issue".to_string(),
            outcome_value: 1.0,
            outcome_weight: 12.0,
            outcome_label: "success".to_string(),
            previous_affordance_id: None,
            query_text: "open a bug".to_string(),
            recorded_at_ms: None,
        },
        Some("test"),
    )
    .unwrap();

    let raised = Affordance::from_node_record(
        store
            .get_node(&affordance_node_id("theorem", "github.create_issue"))
            .unwrap(),
    )
    .unwrap()
    .fitness;
    assert!(
        raised > DEFAULT_BASE_FITNESS,
        "fitness should rise after a positive outcome"
    );

    // Re-register the same connector (idempotent): node count unchanged and the
    // learned fitness is preserved, not reset to the base.
    register_connector(&mut store, github_manifest(), Some("test")).unwrap();
    let affordances = store.query_nodes(NodeQuery::label(AFFORDANCE_LABEL).with_limit(100));
    assert_eq!(
        affordances.len(),
        3,
        "re-registration must not duplicate nodes"
    );

    let after = Affordance::from_node_record(
        store
            .get_node(&affordance_node_id("theorem", "github.create_issue"))
            .unwrap(),
    )
    .unwrap()
    .fitness;
    assert!(
        (after - raised).abs() < 1e-6,
        "re-registration must preserve learned fitness"
    );
}

#[test]
fn builtin_affordances_become_graph_nodes() {
    let mut store = InMemoryGraphStore::new();
    let result = register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    // The harness-core registry has 11 symbolic-engine affordances.
    assert_eq!(result.affordance_node_ids.len(), 11);
    let affordances = store.query_nodes(NodeQuery::label(AFFORDANCE_LABEL).with_limit(100));
    assert_eq!(affordances.len(), 11);
    assert!(result
        .affordance_node_ids
        .contains(&affordance_node_id("theorem", "datalog.derive")));
}

#[test]
fn theseus_app_affordances_become_theorem_grpc_nodes() {
    let mut store = InMemoryGraphStore::new();
    let result = register_theseus_app_affordances(&mut store, "theorem", Some("test")).unwrap();

    assert_eq!(result.server_id, THEOREM_GRPC_SERVER_ID);
    assert_eq!(result.affordance_node_ids.len(), 20);
    assert!(store.get_node(&result.connector_node_id).is_some());

    let publisher_id = affordance_node_id("theorem", "theorem_grpc.publisher.publish");
    let code_ingest_id = affordance_node_id("theorem", "theorem_grpc.code_search.ingest");
    let code_ingest_status_id =
        affordance_node_id("theorem", "theorem_grpc.code_search.ingest_status");
    let code_search_id = affordance_node_id("theorem", "theorem_grpc.code_search.search");
    let code_explore_id = affordance_node_id("theorem", "theorem_grpc.code_search.explore");
    let code_explain_id = affordance_node_id("theorem", "theorem_grpc.code_search.explain");
    let code_use_id = affordance_node_id("theorem", "theorem_grpc.code_search.record_use_receipt");
    assert!(result.affordance_node_ids.contains(&publisher_id));
    assert!(result.affordance_node_ids.contains(&code_ingest_id));
    assert!(result.affordance_node_ids.contains(&code_ingest_status_id));
    assert!(result.affordance_node_ids.contains(&code_search_id));
    assert!(result.affordance_node_ids.contains(&code_explore_id));
    assert!(result.affordance_node_ids.contains(&code_explain_id));
    assert!(result.affordance_node_ids.contains(&code_use_id));
    let publisher = Affordance::from_node_record(store.get_node(&publisher_id).unwrap()).unwrap();
    let code_ingest =
        Affordance::from_node_record(store.get_node(&code_ingest_id).unwrap()).unwrap();
    let code_search =
        Affordance::from_node_record(store.get_node(&code_search_id).unwrap()).unwrap();

    assert_eq!(publisher.server_id, THEOREM_GRPC_SERVER_ID);
    assert_eq!(publisher.family, "publisher");
    assert_eq!(publisher.writeback_policy, "confirm-before-external");
    assert!(publisher
        .permissions
        .contains(&"external_action".to_string()));
    assert_eq!(publisher.cost["transport"], "theorem_grpc");
    assert_eq!(publisher.cost["timeout_ms"], THEOREM_GRPC_TIMEOUT_MS);
    assert_eq!(
        publisher.cost["failure_receipt"]["receipt_type"],
        "THEOREM_GRPC.AFFORDANCE_FAILED"
    );
    assert_eq!(code_search.family, "code_search");
    assert_eq!(code_search.writeback_policy, "receipt-only");
    assert!(code_search.permissions.contains(&"code_read".to_string()));
    assert_eq!(
        code_ingest.input_schema["timeout_ms"],
        THEOREM_GRPC_CODE_INGEST_TIMEOUT_MS
    );
    assert_eq!(
        code_ingest.cost["timeout_ms"],
        THEOREM_GRPC_CODE_INGEST_TIMEOUT_MS
    );
    assert_eq!(code_search.cost["timeout_ms"], THEOREM_GRPC_TIMEOUT_MS);

    let offers =
        store.neighbors(NeighborQuery::out(&result.connector_node_id).with_edge_type(OFFERS));
    assert_eq!(offers.len(), 20);
}

#[test]
fn theorem_grpc_timeout_budget_extends_only_code_ingest_writes() {
    assert_eq!(
        crate::theorem_grpc_timeout_ms("theorem_grpc.code_search.ingest", 0),
        THEOREM_GRPC_CODE_INGEST_TIMEOUT_MS
    );
    assert_eq!(
        crate::theorem_grpc_timeout_ms("code_search.reindex", 240_000),
        240_000
    );
    assert_eq!(
        crate::theorem_grpc_timeout_ms("theorem_grpc.code_search.ingest", 999_000),
        THEOREM_GRPC_MAX_TIMEOUT_MS
    );
    assert_eq!(
        crate::theorem_grpc_timeout_ms("theorem_grpc.code_search.search", 90_000),
        THEOREM_GRPC_TIMEOUT_MS
    );
}

#[test]
fn theseus_app_affordance_invocations_record_receipts() {
    let mut store = InMemoryGraphStore::new();
    register_theseus_app_affordances(&mut store, "theorem", Some("test")).unwrap();

    let receipt = record_invocation(
        &mut store,
        InvocationRecordRequest {
            tenant_id: "theorem".to_string(),
            task_type: "research".to_string(),
            candidate_affordance_ids: vec![
                "theorem_grpc.research.expand".to_string(),
                "theorem_grpc.observability.read_trace".to_string(),
            ],
            selected_affordance_id: "theorem_grpc.research.expand".to_string(),
            outcome_value: 1.0,
            outcome_weight: 3.0,
            outcome_label: "evidence_found".to_string(),
            previous_affordance_id: None,
            query_text: "expand the research frontier".to_string(),
            recorded_at_ms: Some(1_000),
        },
        Some("test"),
    )
    .unwrap();

    assert!(!receipt.receipt_node_id.is_empty());
    assert!(store.get_node(&receipt.receipt_node_id).is_some());
    assert!(store.get_edge(&receipt.produced_outcome_edge_id).is_some());
    assert!(receipt.effective_fitness > DEFAULT_BASE_FITNESS);
}
