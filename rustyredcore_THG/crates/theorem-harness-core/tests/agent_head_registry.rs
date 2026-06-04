use serde_json::Value;
use theorem_harness_core::{
    AgentBinding, AgentHead, AgentHeadRegistry, AgentHeadRegistryError, BindingBudgetScope,
    BindingComposition, BindingIdentity, HeadCostProfile, HeadKind, HeadReliabilityProfile,
    HeadTransport, TraceTier,
};

#[test]
fn resolves_each_transport_to_fake_endpoint_without_network() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();

    let api = registry.resolve("claude", HeadTransport::Api).unwrap();
    let mcp = registry.resolve("mistral_ocr", HeadTransport::Mcp).unwrap();
    let local = registry
        .resolve("gemma_local", HeadTransport::Local)
        .unwrap();
    let hosted = registry
        .resolve("codex_hosted", HeadTransport::Hosted)
        .unwrap();

    assert_eq!(api.endpoint.target, "fake://api/anthropic/claude/claude");
    assert_eq!(
        mcp.endpoint.target,
        "fake://mcp/mistral/voxtral/mistral_ocr"
    );
    assert_eq!(
        local.endpoint.target,
        "fake://local/google/gemma_26b/gemma_local"
    );
    assert_eq!(
        hosted.endpoint.target,
        "fake://hosted/openai/codex/codex_hosted"
    );
    assert!(api.endpoint.fake);
    assert!(mcp.endpoint.fake);
    assert!(local.endpoint.fake);
    assert!(hosted.endpoint.fake);
}

#[test]
fn resolved_view_preserves_head_kind_boundaries() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();

    let summary = registry.kind_summary();

    assert_eq!(summary.reasoning_cores, vec!["claude", "gemma_local"]);
    assert_eq!(summary.skill_plugins, vec!["mistral_ocr"]);
    assert_eq!(summary.specialized_coders, vec!["codex_hosted"]);
    assert_eq!(
        registry
            .resolve("mistral_ocr", HeadTransport::Mcp)
            .unwrap()
            .kind,
        HeadKind::SkillPlugin
    );
}

#[test]
fn inactive_and_unknown_heads_are_rejected_before_invocation() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();

    let inactive = registry
        .resolve("deepseek_inactive", HeadTransport::Api)
        .unwrap_err();
    let unknown = registry.resolve("missing", HeadTransport::Api).unwrap_err();

    assert!(matches!(
        inactive,
        AgentHeadRegistryError::InactiveHead { .. }
    ));
    assert!(matches!(
        unknown,
        AgentHeadRegistryError::UnknownHead { .. }
    ));
}

#[test]
fn transport_mismatch_is_rejected() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();

    let error = registry.resolve("claude", HeadTransport::Mcp).unwrap_err();

    assert!(matches!(
        error,
        AgentHeadRegistryError::TransportMismatch { .. }
    ));
}

#[test]
fn heads_probed_transition_carries_secret_free_resolved_manifest() {
    let registry = AgentHeadRegistry::from_binding(&fixture_binding()).unwrap();
    let transition = registry.heads_probed_transition();
    let encoded = serde_json::to_string(&transition.payload).unwrap();

    assert_eq!(transition.event_type, "HEADS.PROBED");
    assert!(encoded.contains("credential:claude"));
    assert!(!encoded.contains("credential_value"));
    assert!(!encoded.contains("api_key"));
    assert!(!encoded.contains("secret_material"));

    let resolved_heads = transition
        .payload
        .get("resolved_heads")
        .and_then(Value::as_array)
        .unwrap();
    assert_eq!(resolved_heads.len(), 4);
}

#[test]
fn raw_credential_material_is_rejected_by_registry() {
    let mut binding = fixture_binding();
    binding.composition.heads[0].credential_ref = "sk-test-should-not-enter-graph".to_string();

    let error = AgentHeadRegistry::from_binding(&binding).unwrap_err();

    assert!(matches!(
        error,
        AgentHeadRegistryError::CredentialMaterialRejected { .. }
    ));
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
            active_head_set: vec![
                "claude".to_string(),
                "mistral_ocr".to_string(),
                "gemma_local".to_string(),
                "codex_hosted".to_string(),
            ],
        },
        BindingComposition {
            heads: vec![
                head(
                    "claude",
                    "anthropic",
                    "claude",
                    "credential:claude",
                    HeadTransport::Api,
                    HeadKind::ReasoningCore,
                ),
                head(
                    "mistral_ocr",
                    "mistral",
                    "voxtral",
                    "credential:mistral-ocr",
                    HeadTransport::Mcp,
                    HeadKind::SkillPlugin,
                ),
                head(
                    "gemma_local",
                    "google",
                    "gemma 26b",
                    "credential:gemma-local",
                    HeadTransport::Local,
                    HeadKind::ReasoningCore,
                ),
                head(
                    "codex_hosted",
                    "openai",
                    "codex",
                    "credential:codex-hosted",
                    HeadTransport::Hosted,
                    HeadKind::SpecializedCoder,
                ),
                head(
                    "deepseek_inactive",
                    "deepseek",
                    "v4",
                    "credential:deepseek",
                    HeadTransport::Api,
                    HeadKind::ReasoningCore,
                ),
            ],
        },
        BindingBudgetScope::new("theorem", 100.0, 4),
    )
    .unwrap()
}

fn head(
    head_id: &str,
    provider: &str,
    model: &str,
    credential_ref: &str,
    transport: HeadTransport,
    kind: HeadKind,
) -> AgentHead {
    AgentHead {
        head_id: head_id.to_string(),
        display_name: head_id.to_string(),
        provider: provider.to_string(),
        model: model.to_string(),
        credential_ref: credential_ref.to_string(),
        transport,
        kind,
        capabilities: Vec::new(),
        cost_profile: HeadCostProfile::default(),
        reliability_profile: HeadReliabilityProfile::default(),
        allowed_tools: Vec::new(),
        trace_tier: TraceTier::Receipt,
    }
}
