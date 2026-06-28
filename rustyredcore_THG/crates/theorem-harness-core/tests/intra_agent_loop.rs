use std::cell::RefCell;

use serde_json::json;
use theorem_harness_core::{
    default_authority_order, run_fake_intra_agent_loop, AgentBinding, AgentHead,
    BindingBudgetScope, BindingComposition, BindingError, BindingIdentity,
    BindingVerificationOutcome, ContextMembranePrime, FakeIntraAgentLoopInput, GroundedClaim,
    HeadCapabilityReliability, HeadCostProfile, HeadInvocationError, HeadInvocationKind,
    HeadInvocationReceipt, HeadInvocationRequest, HeadInvoker, HeadKind, HeadReliabilityProfile,
    HeadTransport, IntraAgentLoopError, ScratchpadRelationKind, TraceTier,
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
    assert_eq!(result.events.len(), 18);
    assert_eq!(result.invocation_receipts.len(), 4);
    assert_eq!(
        result
            .invocation_receipts
            .iter()
            .map(|receipt| receipt.kind)
            .collect::<Vec<_>>(),
        vec![
            HeadInvocationKind::Proposal,
            HeadInvocationKind::Critique,
            HeadInvocationKind::Synthesis,
            HeadInvocationKind::Verification
        ]
    );
    assert_eq!(result.events[0].event_type, "BINDING.RESOLVED");
    assert_eq!(result.events[1].event_type, "HEADS.PROBED");
    assert_eq!(result.events[8].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[9].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[10].event_type, "HEADS.CONTRIBUTE");
    assert_eq!(result.events[11].event_type, "DRAFTS.SYNTHESIZED");
    assert_eq!(result.events[12].event_type, "SYNTHESIS.VERIFIED");
    assert_eq!(result.events[14].event_type, "POLICY.CHECKED");
    assert_eq!(result.events[17].event_type, "RUN.CLOSED");
    let policy_decision = result.events[14]
        .payload
        .get("policy_decision")
        .and_then(serde_json::Value::as_object)
        .expect("POLICY.CHECKED carries a policy decision");
    assert_eq!(
        policy_decision.get("authority_order").unwrap(),
        &json!(default_authority_order())
    );
    assert_eq!(result.scratchpad_revisions.len(), 5);
    assert_eq!(result.rounds[0].escalation_signal, "verified_converged");
    assert_eq!(result.rounds[0].disagreement_count, 0);
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
            "fake synthesis",
            "fake verification"
        ]
    );
    assert_eq!(
        result.scratchpad_revisions[1].content_hash,
        result.invocation_receipts[0].content_hash
    );
    assert!(result
        .binding
        .working_memory_scope
        .scratchpad
        .relations
        .iter()
        .any(|relation| relation.relation_kind == ScratchpadRelationKind::Supports));
}

#[test]
fn fake_loop_charges_head_contributions_through_budget_guard() {
    let result = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap();

    assert_eq!(result.binding.trace_scope.contributions.len(), 3);
    assert_eq!(result.binding.budget_state.spent_total, 4.0);
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
    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    input.context_membrane = vec![ContextMembranePrime::new(
        "context:repo",
        "repo state",
        "ambient intelligence loaded at run start",
        "harness:recall",
        0.9,
    )];

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
        request.scratchpad_crdt.graph_root_id == "crdtgraph:scratchpad_theorem"
            && request.scratchpad_crdt.stream_topic == "scratchpad.crdt.scratchpad_theorem"
    }));
    assert!(requests.iter().all(|request| {
        request.context_membrane.len() == 1
            && request.context_membrane[0].artifact_id == "context:repo"
    }));
    assert!(requests
        .iter()
        .all(|request| request.head_system_prompt.contains("one mind of Theorem")));
    assert!(requests.iter().all(|request| {
        request
            .policy_decision
            .as_ref()
            .map(|decision| decision.authority_order == default_authority_order())
            .unwrap_or(false)
    }));
}

#[test]
fn fake_loop_routes_roles_by_learned_capability_reliability() {
    let mut binding = fixture_binding();
    binding
        .composition
        .heads
        .iter_mut()
        .find(|head| head.head_id == "deepseek")
        .unwrap()
        .reliability_profile
        .capability_scores
        .push(HeadCapabilityReliability::new("proposal", "rust", 8, 1));
    binding
        .composition
        .heads
        .iter_mut()
        .find(|head| head.head_id == "claude")
        .unwrap()
        .reliability_profile
        .capability_scores
        .push(HeadCapabilityReliability::new("critique", "rust", 8, 1));
    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    input.domain = "rust".to_string();

    let result = run_fake_intra_agent_loop(binding, input).unwrap();

    assert_eq!(result.primary_head_id, "deepseek");
    assert_eq!(result.critic_head_id, "claude");
    assert_eq!(result.routing_decisions.len(), 4);
    assert!(result
        .routing_decisions
        .iter()
        .any(|decision| decision.capability == "proposal" && decision.head_id == "deepseek"));
}

#[test]
fn fake_loop_iterates_after_defective_verification_until_accepted() {
    let invoker = DefectThenAcceptedVerifier::default();
    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    input.max_rounds = 2;
    input.budget_units = 20.0;

    let result =
        theorem_harness_core::run_intra_agent_loop_with_invoker(fixture_binding(), input, &invoker)
            .unwrap();

    assert_eq!(result.rounds.len(), 2);
    assert_eq!(
        result.rounds[0].verification_outcome,
        BindingVerificationOutcome::DefectFound
    );
    assert_eq!(result.rounds[0].escalation_signal, "verification_defect");
    assert_eq!(result.rounds[0].disagreement_count, 1);
    assert_eq!(result.rounds[0].stop_reason, "continue");
    assert_eq!(
        result.rounds[1].verification_outcome,
        BindingVerificationOutcome::Accepted
    );
    assert_eq!(result.rounds[1].escalation_signal, "verified_converged");
    assert_eq!(result.rounds[1].stop_reason, "verified_converged");
    assert_eq!(result.invocation_receipts.len(), 8);
    assert_eq!(result.events.len(), 22);
    assert!(result
        .binding
        .working_memory_scope
        .scratchpad
        .relations
        .iter()
        .any(|relation| relation.relation_kind == ScratchpadRelationKind::Undercuts));
    assert!(result
        .binding
        .working_memory_scope
        .scratchpad
        .relations
        .iter()
        .any(|relation| relation.relation_kind == ScratchpadRelationKind::Supports));
}

#[test]
fn fake_loop_compounds_outcome_reliability_for_later_routing() {
    let result = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap();

    let primary = result.binding.head(&result.primary_head_id).unwrap();
    assert!(
        primary
            .reliability_profile
            .reliability_for("proposal", "general")
            > 0.5
    );
    assert!(!primary.reliability_profile.last_outcome_hash.is_empty());
    let routed_head_id = result
        .binding
        .route_subtask(
            &theorem_harness_core::BindingSubtask::new("next", "proposal", "general"),
            999,
        )
        .unwrap()
        .head_id;
    assert!(
        result
            .binding
            .head(&routed_head_id)
            .unwrap()
            .reliability_profile
            .reliability_for("proposal", "general")
            > 0.5
    );
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
            agent_constitution: None,
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
            HeadInvocationKind::Verification => "recorded verification",
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

#[derive(Default)]
struct DefectThenAcceptedVerifier {
    verification_calls: RefCell<u32>,
}

impl HeadInvoker for DefectThenAcceptedVerifier {
    fn invoke(
        &self,
        request: HeadInvocationRequest,
    ) -> Result<HeadInvocationReceipt, HeadInvocationError> {
        let output_summary = match request.kind {
            HeadInvocationKind::Proposal => "iterated proposal",
            HeadInvocationKind::Critique => "iterated critique",
            HeadInvocationKind::Synthesis => "iterated synthesis",
            HeadInvocationKind::Verification => "iterated verification",
        };
        let mut payload = serde_json::Map::new();
        payload.insert("kind".to_string(), json!(request.kind.as_str()));
        payload.insert("task".to_string(), json!(request.task));
        payload.insert("prior_context".to_string(), json!(request.prior_context));
        if request.kind == HeadInvocationKind::Verification {
            let mut calls = self.verification_calls.borrow_mut();
            *calls += 1;
            payload.insert(
                "attempted_failure_modes".to_string(),
                json!(["grounding gap", "counterexample search"]),
            );
            payload.insert(
                "commands_run".to_string(),
                json!(["binding synthesis verification"]),
            );
            payload.insert(
                "outcome".to_string(),
                json!(if *calls == 1 {
                    "defect_found"
                } else {
                    "accepted"
                }),
            );
        }
        Ok(HeadInvocationReceipt::from_request(
            &request,
            output_summary,
            payload,
            1.0,
        ))
    }
}

// --- Agent Theorem S1: persona/voice in AgentBinding -----------------------
//
// The three tests below prove the wiring added in S1: when a binding carries
// an `agent_constitution`, the intra-agent loop threads it into every head
// invocation request (proposal, critique, synthesis, verification), and the
// receipt surfaces the same text so the synthesis step (DRAFTS.SYNTHESIZED) is
// guaranteed to have seen Theorem's voice. The third test pins the
// back-compat parity: a binding serialized without the new field still
// deserializes (`agent_constitution: None`) and produces invocation requests
// without a constitution field.

const FAKE_CONSTITUTION_TEXT: &str =
    "Theorem speaks one voice across many heads: grounded, concise, no slop.";

#[test]
fn fake_loop_threads_agent_constitution_through_every_head_invocation() {
    let mut binding = fixture_binding();
    binding.identity.agent_constitution = Some(FAKE_CONSTITUTION_TEXT.to_string());
    let input = FakeIntraAgentLoopInput::new(
        "publish a grounded theorem answer",
        vec![GroundedClaim::new(
            "Theorem can publish grounded composed-agent output",
            "source:binding-test",
        )],
    );

    let result = run_fake_intra_agent_loop(binding, input).unwrap();

    // All four invocations (proposal, critique, synthesis, verification) must
    // carry the constitution. Voice consistency across steps is the whole
    // point of S1.
    assert_eq!(result.invocation_receipts.len(), 4);
    for receipt in &result.invocation_receipts {
        let constitution = receipt
            .payload
            .get("constitution")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "expected receipt for kind {:?} to carry constitution",
                    receipt.kind
                )
            });
        assert_eq!(constitution, FAKE_CONSTITUTION_TEXT);
    }

    // Synthesis is the load-bearing surface: assert explicitly that the
    // synthesis receipt (the one that DRAFTS.SYNTHESIZED later commits) saw
    // the constitution.
    let synthesis_receipt = result
        .invocation_receipts
        .iter()
        .find(|receipt| receipt.kind == HeadInvocationKind::Synthesis)
        .expect("loop must produce a synthesis receipt");
    assert_eq!(
        synthesis_receipt
            .payload
            .get("constitution")
            .and_then(|value| value.as_str()),
        Some(FAKE_CONSTITUTION_TEXT)
    );
}

#[test]
fn fake_loop_omits_constitution_when_binding_has_none() {
    // Back-compat: a binding without a constitution should produce invocation
    // receipts that do NOT carry a constitution field, so existing
    // replay/parity hashes continue to match.
    let binding = fixture_binding();
    assert!(binding.identity.agent_constitution.is_none());

    let input = FakeIntraAgentLoopInput::new(
        "publish a grounded theorem answer",
        vec![GroundedClaim::new(
            "Theorem can publish grounded composed-agent output",
            "source:binding-test",
        )],
    );

    let result = run_fake_intra_agent_loop(binding, input).unwrap();

    assert_eq!(result.invocation_receipts.len(), 4);
    for receipt in &result.invocation_receipts {
        assert!(
            receipt.payload.get("constitution").is_none(),
            "receipt for kind {:?} should not carry constitution when binding has none",
            receipt.kind
        );
    }
}

#[test]
fn binding_identity_deserializes_without_agent_constitution_field_for_back_compat() {
    // Older serialized BindingIdentity blobs (written before S1) did not
    // include `agent_constitution`. They must still deserialize, with the new
    // field defaulted to `None`.
    let legacy_json = serde_json::json!({
        "agent_id": "theorem",
        "owner_id": "travis",
        "agent_name": "Theorem",
        "composition_hash": "",
        "version": 1,
        "trust_tier": "first_party",
        "active_head_set": ["claude", "deepseek"],
    });
    let identity: BindingIdentity =
        serde_json::from_value(legacy_json).expect("legacy BindingIdentity must deserialize");
    assert_eq!(identity.agent_constitution, None);

    // And a fresh identity round-trips through JSON with `None` preserved.
    let round_trip: BindingIdentity =
        serde_json::from_value(serde_json::to_value(&identity).unwrap()).unwrap();
    assert_eq!(round_trip.agent_constitution, None);
}
