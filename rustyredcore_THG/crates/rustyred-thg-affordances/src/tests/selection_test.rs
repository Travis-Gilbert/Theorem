use serde_json::json;

use rustyred_thg_core::InMemoryGraphStore;

use crate::types::CapabilityScope;
use crate::{
    record_invocation, register_connector, register_theseus_app_affordances, select_affordances,
    select_affordances_by_embedding, ConnectorManifest, InvocationRecordRequest, SelectionRequest,
    ToolManifest,
};

fn tool(name: &str, embedding: Option<Vec<f32>>) -> ToolManifest {
    ToolManifest {
        name: name.to_string(),
        label: String::new(),
        description: format!("the {name} tool"),
        input_schema: json!({}),
        permissions: vec![],
        cost: json!({}),
        writeback_policy: "read-only".to_string(),
        tags: vec![],
        description_embedding: embedding,
    }
}

fn connector(server: &str, tools: Vec<ToolManifest>) -> ConnectorManifest {
    ConnectorManifest {
        tenant_id: "theorem".to_string(),
        server_id: server.to_string(),
        label: server.to_string(),
        tools,
    }
}

fn select(
    store: &InMemoryGraphStore,
    task: &str,
    scope: CapabilityScope,
) -> Vec<crate::AffordanceRef> {
    select_affordances(
        store,
        &SelectionRequest {
            tenant_id: "theorem".to_string(),
            task_type: task.to_string(),
            k: 10,
            scope,
            ..Default::default()
        },
    )
    .unwrap()
}

#[test]
fn unprimed_affordances_are_reachable_forwarding_fallback() {
    let mut store = InMemoryGraphStore::new();
    register_connector(
        &mut store,
        connector(
            "github",
            vec![
                tool("create_issue", None),
                tool("search_code", None),
                tool("get_file", None),
            ],
        ),
        Some("test"),
    )
    .unwrap();

    // With no recorded outcomes, every scoped affordance is still reachable.
    let refs = select(&store, "anything", CapabilityScope::unrestricted("agent"));
    assert_eq!(
        refs.len(),
        3,
        "freshly connected tools are reachable (forwarding fallback)"
    );
}

#[test]
fn positive_outcome_changes_rank_order_compounding() {
    let mut store = InMemoryGraphStore::new();
    register_connector(
        &mut store,
        connector(
            "github",
            vec![tool("create_issue", None), tool("search_code", None)],
        ),
        Some("test"),
    )
    .unwrap();

    // Record a success for search_code on the "search" task.
    record_invocation(
        &mut store,
        InvocationRecordRequest {
            tenant_id: "theorem".to_string(),
            task_type: "search".to_string(),
            candidate_affordance_ids: vec![
                "github.create_issue".to_string(),
                "github.search_code".to_string(),
            ],
            selected_affordance_id: "github.search_code".to_string(),
            outcome_value: 1.0,
            outcome_weight: 10.0,
            outcome_label: "success".to_string(),
            previous_affordance_id: None,
            query_text: "find the function".to_string(),
            recorded_at_ms: None,
        },
        Some("test"),
    )
    .unwrap();

    let refs = select(&store, "search", CapabilityScope::unrestricted("agent"));
    assert!(refs.len() >= 2);
    assert_eq!(
        refs[0].affordance.affordance_id, "github.search_code",
        "the affordance that worked for this task now ranks first"
    );
    assert!(
        refs[0].score > refs[1].score,
        "selection compounds: the proven affordance outscores the unprimed one"
    );
}

#[test]
fn capability_scope_excludes_out_of_scope_affordances() {
    let mut store = InMemoryGraphStore::new();
    register_connector(
        &mut store,
        connector(
            "github",
            vec![tool("create_issue", None), tool("search_code", None)],
        ),
        Some("test"),
    )
    .unwrap();
    register_connector(
        &mut store,
        connector("filesystem", vec![tool("read", None), tool("write", None)]),
        Some("test"),
    )
    .unwrap();

    let scope = CapabilityScope {
        agent_id: "agent".to_string(),
        allow_servers: vec!["github".to_string()],
        ..Default::default()
    };
    let refs = select(&store, "anything", scope);
    assert_eq!(
        refs.len(),
        2,
        "only the two github affordances are in scope"
    );
    assert!(refs.iter().all(|r| r.affordance.server_id == "github"));
}

#[test]
fn capability_scope_can_select_theseus_app_families() {
    let mut store = InMemoryGraphStore::new();
    register_theseus_app_affordances(&mut store, "theorem", Some("test")).unwrap();

    let refs = select(
        &store,
        "publish",
        CapabilityScope {
            agent_id: "agent".to_string(),
            allow_families: vec!["publisher".to_string()],
            ..Default::default()
        },
    );

    assert_eq!(refs.len(), 1);
    assert_eq!(
        refs[0].affordance.affordance_id,
        "theorem_grpc.publisher.publish"
    );
    assert_eq!(refs[0].affordance.server_id, "theorem_grpc");
}

#[test]
fn embedding_selection_ranks_by_cosine() {
    let mut store = InMemoryGraphStore::new();
    register_connector(
        &mut store,
        connector(
            "vec",
            vec![
                tool("east", Some(vec![1.0, 0.0])),
                tool("north", Some(vec![0.0, 1.0])),
            ],
        ),
        Some("test"),
    )
    .unwrap();

    let refs = select_affordances_by_embedding(
        &store,
        &SelectionRequest {
            tenant_id: "theorem".to_string(),
            task_type: "navigate".to_string(),
            k: 10,
            scope: CapabilityScope::unrestricted("agent"),
            ..Default::default()
        },
        &[1.0, 0.0],
    )
    .unwrap();

    assert_eq!(refs[0].affordance.affordance_id, "vec.east");
}
