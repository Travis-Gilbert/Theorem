use serde_json::{json, Map};
use theorem_harness_core::{
    AgentBinding, AgentHead, AgentHeadRegistry, BindingBudgetScope, BindingComposition,
    BindingIdentity, FakeHeadInvoker, GroundedClaim, HeadCostProfile, HeadInvocationError,
    HeadInvocationKind, HeadInvocationRequest, HeadInvoker, HeadKind, HeadReliabilityProfile,
    HeadTransport, RevisionContext, TraceTier,
};

#[test]
fn fake_invoker_produces_content_addressed_receipt() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();
    let head = registry.resolve("claude", HeadTransport::Api).unwrap();
    let request = HeadInvocationRequest::new(
        head,
        HeadInvocationKind::Proposal,
        "publish grounded result",
        1,
        vec!["scratchrev:1".to_string()],
        vec![GroundedClaim::new("grounded claim", "source:1")],
        "2026-06-03T00:00:00Z",
    );

    let receipt = FakeHeadInvoker::default().invoke(request).unwrap();

    assert_eq!(receipt.kind, HeadInvocationKind::Proposal);
    assert_eq!(receipt.head_id, "claude");
    assert_eq!(receipt.output_summary, "fake primary proposal");
    assert_eq!(receipt.cost_units, 1.0);
    assert_eq!(receipt.receipt_hash, receipt.computed_receipt_hash());
    assert_eq!(receipt.contribution_kind(), "proposal");
    assert_eq!(receipt.content_hash.len(), 64);
    assert!(receipt
        .content_hash
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    assert_eq!(receipt.payload.get("fake").unwrap(), true);
}

#[test]
fn invocation_request_carries_prior_revision_context() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();
    let head = registry.resolve("claude", HeadTransport::Api).unwrap();
    let request = HeadInvocationRequest::new_with_context(
        head,
        HeadInvocationKind::Synthesis,
        "synthesize prior work",
        3,
        vec![
            "scratchrev:proposal".to_string(),
            "scratchrev:critique".to_string(),
        ],
        vec![
            RevisionContext {
                revision_id: "scratchrev:proposal".to_string(),
                kind: HeadInvocationKind::Proposal,
                output_summary: "proposal summary".to_string(),
                payload: object_payload(json!({ "text": "proposal body" })),
            },
            RevisionContext {
                revision_id: "scratchrev:critique".to_string(),
                kind: HeadInvocationKind::Critique,
                output_summary: "critique summary".to_string(),
                payload: object_payload(json!({ "text": "critique body" })),
            },
        ],
        vec![GroundedClaim::new("grounded claim", "source:1")],
        "2026-06-03T00:00:00Z",
    );

    let receipt = FakeHeadInvoker::default().invoke(request).unwrap();

    let prior_context = receipt.payload["prior_context"].as_array().unwrap();
    assert_eq!(prior_context.len(), 2);
    assert_eq!(prior_context[0]["output_summary"], "proposal summary");
    assert_eq!(prior_context[1]["kind"], "critique");
}

#[test]
fn fake_invoker_rejects_skill_plugin_heads() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();
    let head = registry.resolve("mistral_ocr", HeadTransport::Mcp).unwrap();
    let request = HeadInvocationRequest::new(
        head,
        HeadInvocationKind::Proposal,
        "ocr should be a tool call",
        1,
        Vec::new(),
        vec![GroundedClaim::new("grounded claim", "source:1")],
        "2026-06-03T00:00:00Z",
    );

    let error = FakeHeadInvoker::default().invoke(request).unwrap_err();

    assert!(matches!(
        error,
        HeadInvocationError::SkillPluginDenied { .. }
    ));
}

#[test]
fn invocation_request_id_is_deterministic() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();
    let head = registry.resolve("claude", HeadTransport::Api).unwrap();

    let first = HeadInvocationRequest::new(
        head.clone(),
        HeadInvocationKind::Critique,
        "review",
        2,
        vec!["scratchrev:1".to_string()],
        vec![GroundedClaim::new("grounded claim", "source:1")],
        "2026-06-03T00:00:00Z",
    );
    let second = HeadInvocationRequest::new(
        head,
        HeadInvocationKind::Critique,
        "review",
        2,
        vec!["scratchrev:1".to_string()],
        vec![GroundedClaim::new("grounded claim", "source:1")],
        "2026-06-03T00:00:00Z",
    );

    assert_eq!(first.invocation_id, second.invocation_id);
    assert_eq!(first.invocation_id, first.computed_invocation_id());
}

fn fixture_binding() -> AgentBinding {
    AgentBinding::new(
        BindingIdentity {
            agent_id: "theorem".to_string(),
            owner_id: "travis".to_string(),
            agent_name: "Theorem".to_string(),
            composition_hash: String::new(),
            version: 1,
            trust_tier: "first_party".to_string(),
            active_head_set: vec!["claude".to_string(), "mistral_ocr".to_string()],
        },
        BindingComposition {
            heads: vec![
                head(
                    "claude",
                    "anthropic",
                    "claude",
                    HeadTransport::Api,
                    HeadKind::ReasoningCore,
                ),
                head(
                    "mistral_ocr",
                    "mistral",
                    "voxtral",
                    HeadTransport::Mcp,
                    HeadKind::SkillPlugin,
                ),
            ],
        },
        BindingBudgetScope::new("theorem", 100.0, 2),
    )
    .unwrap()
}

fn head(
    head_id: &str,
    provider: &str,
    model: &str,
    transport: HeadTransport,
    kind: HeadKind,
) -> AgentHead {
    AgentHead {
        head_id: head_id.to_string(),
        display_name: head_id.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        credential_ref: format!("credential:{head_id}"),
        transport,
        kind,
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}

fn object_payload(value: serde_json::Value) -> Map<String, serde_json::Value> {
    match value {
        serde_json::Value::Object(map) => map,
        _ => Map::new(),
    }
}
