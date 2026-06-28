use std::cell::RefCell;

use serde_json::json;
use theorem_harness_core::{
    default_authority_order, run_fake_intra_agent_loop, user_model_hash, AgentBinding, AgentHead,
    BindingBudgetScope, BindingComposition, BindingError, BindingIdentity,
    BindingVerificationOutcome, ContextMembranePrime, FakeIntraAgentLoopInput, GroundedClaim,
    HeadCapabilityReliability, HeadCostProfile, HeadInvocationError, HeadInvocationKind,
    HeadInvocationReceipt, HeadInvocationRequest, HeadInvoker, HeadKind, HeadReliabilityProfile,
    HeadTransport, IntraAgentLoopError, ScratchpadRelationKind, TraceTier, UserModel,
    UserModelNote, UserModelProjectRef, UserModelReference,
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

#[test]
fn mounted_with_user_model_writes_context_entry_before_heads_contribute() {
    let mut model = UserModel::default();
    model
        .preferences
        .insert("voice".to_string(), "spare".to_string());
    model
        .preferences
        .insert("emojis".to_string(), "never".to_string());
    model
        .style_notes
        .push(UserModelNote::new("no em-dashes", "2026-06-28T00:00:00Z"));
    model.recent_focus.push(UserModelReference::new(
        "node:theorem.harness.core",
        "harness kernel",
    ));
    model.open_frustrations.push(UserModelNote::new(
        "clippy 1.95 noisy",
        "2026-06-28T00:00:00Z",
    ));
    model.working_on.push(UserModelProjectRef::new(
        "project:agent-theorem",
        "Agent Theorem",
        "in_progress",
    ));
    let expected_hash = user_model_hash(&model);

    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    input.user_model = Some(model.clone());

    let result = run_fake_intra_agent_loop(fixture_binding(), input).unwrap();

    // 1) The MOUNTED transition receipt records the user_model hash.
    let mount_event = result
        .events
        .iter()
        .find(|event| event.event_type == "MEMORY_SCOPE.MOUNTED")
        .expect("MEMORY_SCOPE.MOUNTED transition fires");
    let stamped_hash = mount_event
        .payload
        .get("user_model_hash")
        .and_then(serde_json::Value::as_str)
        .expect("user_model_hash is stamped on the receipt");
    assert_eq!(stamped_hash, expected_hash);

    // 2) The scratchpad gained a Context revision authored by "binding:mount"
    //    whose payload deserializes back to the original UserModel.
    let scratchpad = &result.binding.working_memory_scope.scratchpad;
    let context_index = scratchpad
        .revisions
        .iter()
        .position(|revision| {
            revision.actor_head_id == "binding:mount"
                && revision
                    .payload
                    .get("kind")
                    .and_then(serde_json::Value::as_str)
                    == Some("context")
        })
        .expect("a binding:mount Context revision lands on the scratchpad");
    let mounted_revision = &scratchpad.revisions[context_index];
    assert_eq!(mounted_revision.summary, "user model mounted");
    assert_eq!(mounted_revision.content_hash, expected_hash);
    let payload_model: UserModel = serde_json::from_value(
        mounted_revision
            .payload
            .get("user_model")
            .cloned()
            .expect("revision payload carries the user_model"),
    )
    .expect("user_model deserializes back");
    assert_eq!(payload_model, model);

    // 3) The Context revision is BEFORE any HEADS.CONTRIBUTE revision.
    //    HEADS.CONTRIBUTE rides on proposal / critique / synthesis kinds in
    //    the scratchpad; assert at least one such revision exists and is
    //    written after the mount.
    let first_contribute_index = scratchpad
        .revisions
        .iter()
        .position(|revision| {
            matches!(
                revision
                    .payload
                    .get("kind")
                    .and_then(serde_json::Value::as_str),
                Some("proposal") | Some("critique") | Some("synthesis")
            )
        })
        .expect("the loop produces at least one contribution revision");
    assert!(
        context_index < first_contribute_index,
        "user_model context entry (idx {context_index}) must precede the first contribution revision (idx {first_contribute_index})"
    );
}

#[test]
fn mounted_without_user_model_omits_context_entry() {
    // Baseline: no user_model => no binding:mount revision, no user_model_hash.
    let result = run_fake_intra_agent_loop(
        fixture_binding(),
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]),
    )
    .unwrap();

    let mount_event = result
        .events
        .iter()
        .find(|event| event.event_type == "MEMORY_SCOPE.MOUNTED")
        .expect("MEMORY_SCOPE.MOUNTED transition fires");
    assert!(
        mount_event.payload.get("user_model_hash").is_none(),
        "no user_model_hash should be stamped when no user_model was supplied"
    );
    assert!(
        mount_event.payload.get("user_model").is_none(),
        "no user_model field should leak into the receipt when none was supplied"
    );

    let scratchpad = &result.binding.working_memory_scope.scratchpad;
    assert!(
        !scratchpad
            .revisions
            .iter()
            .any(|revision| revision.actor_head_id == "binding:mount"),
        "no binding:mount Context revision should be appended"
    );
    let context_kinds = scratchpad
        .revisions
        .iter()
        .filter(|revision| {
            revision
                .payload
                .get("kind")
                .and_then(serde_json::Value::as_str)
                == Some("context")
        })
        .count();
    assert_eq!(
        context_kinds, 0,
        "no kind=context revisions should appear without a user_model"
    );
}

#[test]
fn user_model_serde_defaults_and_roundtrip() {
    // 1) Deserializing an object missing every field yields the empty model.
    let parsed: UserModel = serde_json::from_value(json!({})).unwrap();
    assert_eq!(parsed, UserModel::default());
    assert!(parsed.is_empty());

    // 2) Serializing-then-deserializing a populated UserModel round-trips byte-identical.
    let mut model = UserModel::default();
    model
        .preferences
        .insert("voice".to_string(), "spare".to_string());
    model
        .preferences
        .insert("emojis".to_string(), "never".to_string());
    model
        .style_notes
        .push(UserModelNote::new("no em-dashes", "2026-06-28T00:00:00Z"));
    model.recent_focus.push(UserModelReference::new(
        "node:theorem.harness.core",
        "harness kernel",
    ));
    model.open_frustrations.push(UserModelNote::new(
        "clippy 1.95 noisy",
        "2026-06-28T00:00:00Z",
    ));
    model.working_on.push(UserModelProjectRef::new(
        "project:agent-theorem",
        "Agent Theorem",
        "in_progress",
    ));

    let json_form = serde_json::to_value(&model).unwrap();
    let reparsed: UserModel = serde_json::from_value(json_form.clone()).unwrap();
    assert_eq!(reparsed, model);
    let reserialized = serde_json::to_value(&reparsed).unwrap();
    assert_eq!(reserialized, json_form);
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
