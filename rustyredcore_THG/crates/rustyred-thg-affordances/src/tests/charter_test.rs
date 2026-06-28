use rustyred_thg_core::InMemoryGraphStore;
use serde_json::json;
use theorem_harness_core::{
    AgentBinding, AgentHead, BindingBudgetScope, BindingComposition, BindingIdentity,
    HeadCostProfile, HeadKind, HeadReliabilityProfile, HeadTransport, TraceTier,
};

use crate::types::CapabilityScope;
use crate::{
    compile_binding_charter_from_store, register_builtin_affordances,
    register_theseus_app_affordances, BindingCharterRequest,
};

fn fixture_binding() -> AgentBinding {
    AgentBinding::new(
        BindingIdentity {
            agent_id: "theorem".to_string(),
            owner_id: "travis".to_string(),
            agent_name: "Theorem".to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set: vec!["claude".to_string(), "deepseek".to_string()],
            agent_constitution: None,
        },
        BindingComposition {
            heads: vec![
                head("claude", HeadKind::ReasoningCore),
                head("deepseek", HeadKind::ReasoningCore),
            ],
        },
        BindingBudgetScope::new("theorem", 100.0, 2),
    )
    .unwrap()
}

fn head(head_id: &str, kind: HeadKind) -> AgentHead {
    AgentHead {
        head_id: head_id.to_string(),
        display_name: head_id.to_string(),
        provider: "test".to_string(),
        model: head_id.to_string(),
        credential_ref: format!("credential:{head_id}"),
        transport: HeadTransport::Api,
        kind,
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}

#[test]
fn charter_enumerates_all_builtin_reasoning_affordances() {
    let mut store = InMemoryGraphStore::new();
    register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    let binding = fixture_binding();

    let charter = compile_binding_charter_from_store(
        &store,
        &binding,
        BindingCharterRequest {
            tenant_id: "theorem".to_string(),
            stance: "grounded composed agent".to_string(),
            task_type: "reasoning".to_string(),
            scope: CapabilityScope::unrestricted("theorem"),
            max_visible_tools: 64,
            min_fitness: None,
        },
    )
    .unwrap();

    assert_eq!(charter.visible_tools.len(), 12);
    assert_eq!(charter.callable_tools.len(), 12);
    assert!(charter
        .visible_tool_ids()
        .contains(&"datalog.derive".to_string()));
    assert!(charter
        .visible_tool_ids()
        .contains(&"solver.check".to_string()));
    assert!(charter
        .visible_tool_ids()
        .contains(&"compute_offload.route_operation".to_string()));
    assert!(charter
        .confirmation_gated_tools
        .contains(&"solver.check".to_string()));
    assert!(!charter.charter_hash.is_empty());
    assert!(!charter.capability_scope_hash.is_empty());

    let charter_payload = charter.charter_compiled_payload();
    assert_eq!(charter_payload["charter_hash"], charter.charter_hash);
    assert_eq!(charter_payload["visible_tool_count"], 12);

    let capability_payload = charter.capabilities_selected_payload();
    assert_eq!(
        capability_payload["capability_scope_hash"],
        charter.capability_scope_hash
    );
    assert_eq!(
        capability_payload["visible_tools"]
            .as_array()
            .unwrap()
            .len(),
        12
    );
}

#[test]
fn capability_scope_limits_the_charter_surface() {
    let mut store = InMemoryGraphStore::new();
    register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    let binding = fixture_binding();

    let charter = compile_binding_charter_from_store(
        &store,
        &binding,
        BindingCharterRequest {
            tenant_id: "theorem".to_string(),
            stance: "proof focused".to_string(),
            task_type: "proof".to_string(),
            scope: CapabilityScope {
                agent_id: "theorem".to_string(),
                allow_families: vec!["proof".to_string(), "solver".to_string()],
                ..Default::default()
            },
            max_visible_tools: 64,
            min_fitness: None,
        },
    )
    .unwrap();

    assert_eq!(
        charter.visible_tool_ids(),
        vec!["proof.create_obligation", "solver.check"]
    );
    assert_eq!(charter.callable_tools, charter.visible_tool_ids());
    assert_eq!(
        charter.confirmation_gated_tools,
        vec!["proof.create_obligation", "solver.check"]
    );
}

#[test]
fn charter_hash_is_stable_and_excludes_credentials() {
    let mut store = InMemoryGraphStore::new();
    register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    let binding = fixture_binding();
    let request = BindingCharterRequest {
        tenant_id: "theorem".to_string(),
        stance: "grounded composed agent".to_string(),
        task_type: "reasoning".to_string(),
        scope: CapabilityScope::unrestricted("theorem"),
        max_visible_tools: 64,
        min_fitness: None,
    };

    let first = compile_binding_charter_from_store(&store, &binding, request.clone()).unwrap();
    let second = compile_binding_charter_from_store(&store, &binding, request).unwrap();

    assert_eq!(first.charter_hash, second.charter_hash);
    let encoded = serde_json::to_string(&first).unwrap();
    assert!(!encoded.contains("credential:"));
}

#[test]
fn store_backed_charter_filters_by_decayed_effective_fitness() {
    let mut store = InMemoryGraphStore::new();
    register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    let node_id = crate::types::affordance_node_id("theorem", "solver.check");
    let mut node = store.get_node(&node_id).unwrap().clone();
    node.properties["fitness"] = json!(1.0);
    node.properties["fitness_updated_at_ms"] = json!(0);
    node.properties["fitness_half_life_days"] = json!(1.0);
    store.upsert_node(node).unwrap();

    let binding = fixture_binding();
    let charter = compile_binding_charter_from_store(
        &store,
        &binding,
        BindingCharterRequest {
            tenant_id: "theorem".to_string(),
            stance: "proof focused".to_string(),
            task_type: "proof".to_string(),
            scope: CapabilityScope {
                agent_id: "theorem".to_string(),
                allow_affordance_ids: vec!["solver.check".to_string()],
                ..Default::default()
            },
            max_visible_tools: 64,
            min_fitness: Some(0.3),
        },
    )
    .unwrap();

    assert!(
        charter.visible_tools.is_empty(),
        "stale tools whose effective fitness decayed below the threshold stay out of the charter"
    );
}

#[test]
fn charter_can_scope_to_theseus_app_affordance_families() {
    let mut store = InMemoryGraphStore::new();
    register_builtin_affordances(&mut store, "theorem", Some("test")).unwrap();
    register_theseus_app_affordances(&mut store, "theorem", Some("test")).unwrap();
    let binding = fixture_binding();

    let charter = compile_binding_charter_from_store(
        &store,
        &binding,
        BindingCharterRequest {
            tenant_id: "theorem".to_string(),
            stance: "research with publication boundary".to_string(),
            task_type: "research".to_string(),
            scope: CapabilityScope {
                agent_id: "theorem".to_string(),
                allow_families: vec!["research".to_string(), "publisher".to_string()],
                ..Default::default()
            },
            max_visible_tools: 64,
            min_fitness: None,
        },
    )
    .unwrap();

    assert_eq!(
        charter.visible_tool_ids(),
        vec![
            "theorem_grpc.publisher.publish",
            "theorem_grpc.research.expand"
        ]
    );
    assert_eq!(charter.callable_tools, charter.visible_tool_ids());
    assert_eq!(
        charter.confirmation_gated_tools,
        vec![
            "theorem_grpc.publisher.publish",
            "theorem_grpc.research.expand"
        ]
    );
    let publisher = charter
        .visible_tools
        .iter()
        .find(|tool| tool.affordance_id == "theorem_grpc.publisher.publish")
        .unwrap();
    assert_eq!(publisher.execution_surface, "theorem_grpc");
    assert_eq!(publisher.source_module, "theseus_apps");
}
