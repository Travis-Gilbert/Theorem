use std::cell::RefCell;

use serde_json::json;
use theorem_harness_core::{
    default_authority_order, run_fake_intra_agent_loop, AgentBinding, AgentHead,
    BindingBudgetScope, BindingComposition, BindingError, BindingIdentity, FakeIntraAgentLoopInput,
    GroundedClaim, HeadCostProfile, HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt,
    HeadInvocationRequest, HeadInvoker, HeadKind, HeadReliabilityProfile, HeadTransport,
    IntraAgentLoopError, TraceTier,
};

#[test]
fn fake_loop_runs_full_lifecycle_with_scratchpad_revisions() {
    let input = FakeIntraAgentLoopInput::new(
        "publish a grounded theorem answer",
        vec![GroundedClaim::new(
            "Theorem can publish grounded composed-agent output",
            "source:binding-test",
        )],
    );

    let result = run_fake_intra_agent_loop(fixture_binding(), input).unwrap();

    assert_eq!(result.binding.lifecycle.status, "closed");
    assert_eq!(result.events.len(), 17);
    assert_eq!(result.invocation_receipts.len(), 3);
    assert_eq!(
        result
            .invocation_receipts
            .iter()
            .map(|receipt| receipt.kind)
            .collect::<Vec<_>>(),
        vec![
            HeadInvocationKind::Proposal,
            HeadInvocationKind::Critique,
            HeadInvocationKind::Synthesis
        ]
    );
    assert_eq!(result.events[0].event_type, "BINDING.RESOLVED");
    assert_eq!(result.events[1].event_type, "HEADS.PROBED");
    assert_eq!(result.events[8].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[9].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[10].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[11].event_type, "DRAFTS.SYNTHESIZED");
    assert_eq!(result.events[13].event_type, "POLICY.CHECKED");
    assert_eq!(result.events[16].event_type, "RUN.CLOSED");
    let policy_decision = result.events[13]
        .payload
        .get("policy_decision")
        .and_then(serde_json::Value::as_object)
        .expect("POLICY.CHECKED carries a policy decision");
    assert_eq!(
        policy_decision.get("authority_order").unwrap(),
        &json!(default_authority_order())
    );
    assert_eq!(result.scratchpad_revisions.len(), 4);
    assert_eq!(
        result
            .scratchpad_revisions
            .iter()
            .map(|revision| revision.summary.as_str())
            .collect::<Vec<_>>(),
        vec![
            "private work opened",
            "fake primary proposal",
            "fake critic review",
            "fake synthesis"
        ]
    );
    assert_eq!(
        result.scratchpad_revisions[1].content_hash,
        result.invocation_receipts[0].content_hash
    );
}

#[test]
fn fake_loop_charges_head_contributions_through_budget_guard() {
    let result = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap();

    assert_eq!(result.binding.trace_scope.contributions.len(), 3);
    assert_eq!(result.binding.budget_state.spent_total, 3.0);
}

#[test]
fn fake_loop_records_two_distinct_synthesis_heads() {
    let result = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap();

    assert_eq!(
        result.binding.trace_scope.synthesis_heads,
        vec!["claude", "deepseek"]
    );
}

#[test]
fn synthesis_receives_proposal_and_critique_context() {
    let invoker = RecordingInvoker::default();
    let input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);

    theorem_harness_core::run_intra_agent_loop_with_invoker(fixture_binding(), input, &invoker)
        .unwrap();

    let requests = invoker.requests.into_inner();
    let synthesis = requests
        .iter()
        .find(|request| request.kind == HeadInvocationKind::Synthesis)
        .unwrap();
    assert_eq!(synthesis.prior_context.len(), 2);
    assert_eq!(
        synthesis.prior_context[0].kind,
        HeadInvocationKind::Proposal
    );
    assert_eq!(
        synthesis.prior_context[1].kind,
        HeadInvocationKind::Critique
    );
    assert!(synthesis.prior_context[0]
        .payload
        .get("task")
        .and_then(serde_json::Value::as_str)
        .is_some());
    assert!(requests
        .iter()
        .all(|request| request.policy_decision.is_some()));
    assert!(requests.iter().all(|request| {
        request
            .policy_decision
            .as_ref()
            .map(|decision| decision.authority_order == default_authority_order())
            .unwrap_or(false)
    }));
}

#[test]
fn fake_loop_claimless_publication_fails_strict_grounding() {
    let error = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish without claims", Vec::new()),
    )
    .unwrap_err();

    assert_guard(error, "grounding_missing");
}

#[test]
fn fake_loop_ungrounded_publication_fails_strict_grounding() {
    let error = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new(
            "publish ungrounded claim",
            vec![GroundedClaim::new("ungrounded", "")],
        ),
    )
    .unwrap_err();

    assert_guard(error, "grounding_missing");
}

#[test]
fn fake_loop_requires_two_active_non_plugin_heads() {
    let mut binding = fixture_binding();
    binding.identity.active_head_set = vec!["claude".to_string(), "mistral_ocr".to_string()];

    let error = run_fake_intra_agent_loop(
        binding,
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        IntraAgentLoopError::NotEnoughReasoningHeads { available: 1 }
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
                "deepseek".to_string(),
                "mistral_ocr".to_string(),
            ],
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
                    "deepseek",
                    "deepseek",
                    "v4",
                    HeadTransport::Hosted,
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
        BindingBudgetScope::new("theorem", 100.0, 3),
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

fn assert_guard(error: IntraAgentLoopError, expected_code: &str) {
    match error {
        IntraAgentLoopError::Binding(BindingError::Guard(violation)) => {
            assert_eq!(violation.code, expected_code);
        }
        other => panic!("expected binding guard {expected_code}, got {other:?}"),
    }
}

#[derive(Default)]
struct RecordingInvoker {
    requests: RefCell<Vec<HeadInvocationRequest>>,
}

impl HeadInvoker for RecordingInvoker {
    fn invoke(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        self.requests.borrow_mut().push(request.clone());
        let output_summary = match request.kind {
            HeadInvocationKind::Proposal => "recorded proposal",
            HeadInvocationKind::Critique => "recorded critique",
            HeadInvocationKind::Synthesis => "recorded synthesis",
        };
        let mut payload = serde_json::Map::new();
        payload.insert("kind".to_string(), json!(request.kind.as_str()));
        payload.insert("task".to_string(), json!(request.task));
        payload.insert("prior_context".to_string(), json!(request.prior_context));
        Ok(HeadInvocationReceipt::from_request(
            &request,
            output_summary,
            payload,
            1.0,
        ))
    }
}
