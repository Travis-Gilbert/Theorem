use std::cell::RefCell;

use serde_json::json;
use theorem_harness_core::{
    default_authority_order, run_fake_intra_agent_loop, user_model_hash, AgentBinding, AgentHead,
    BindingBudgetScope, BindingComposition, BindingError, BindingIdentity,
    BindingLineageMemoryEntry, BindingVerificationOutcome, ContextMembranePrime,
    FakeIntraAgentLoopInput, GroundedClaim, HeadCapabilityReliability, HeadCostProfile,
    HeadInvocationError, HeadInvocationKind, HeadInvocationReceipt, HeadInvocationRequest,
    HeadInvoker, HeadKind, HeadReliabilityProfile, HeadTransport, IntraAgentLoopError,
    ScratchpadRelationKind, TraceTier, UserModel, UserModelNote, UserModelProjectRef,
    UserModelReference,
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
    assert_eq!(synthesis.prior_context[0].kind, "proposal");
    assert_eq!(synthesis.prior_context[1].kind, "critique");
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

    let invoker = RecordingInvoker::default();
    let result =
        theorem_harness_core::run_intra_agent_loop_with_invoker(fixture_binding(), input, &invoker)
            .unwrap();

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

    // 4) PR #72 P2: every subsequent head invocation (proposal, critique,
    //    synthesis, verification) must see the mount revision in
    //    `prior_revision_ids` AND in `prior_context` -- so heads can act on
    //    the user_model the binding mount staged for them.
    let mount_revision_id = mounted_revision.revision_id.clone();
    let requests = invoker.requests.into_inner();
    assert!(
        !requests.is_empty(),
        "the loop produced at least one head invocation"
    );
    for request in &requests {
        assert!(
            request.prior_revision_ids.contains(&mount_revision_id),
            "{:?} request prior_revision_ids should include the mount revision id; got {:?}",
            request.kind,
            request.prior_revision_ids
        );
        let mount_ctx = request
            .prior_context
            .iter()
            .find(|context| context.revision_id == mount_revision_id)
            .unwrap_or_else(|| {
                panic!(
                    "{:?} request prior_context should include the mount revision",
                    request.kind
                )
            });
        assert_eq!(mount_ctx.kind, "context");
        let head_view_user_model: UserModel = serde_json::from_value(
            mount_ctx
                .payload
                .get("user_model")
                .cloned()
                .expect("mount prior_context entry carries the user_model"),
        )
        .expect("user_model on prior_context deserializes back");
        assert_eq!(head_view_user_model, model);
    }
}

#[test]
fn mounted_with_user_model_is_replayable_with_stable_state_hash() {
    // PR #72 P1: two independent applies of the same MEMORY_SCOPE.MOUNTED
    // transition (same agent_id + composition_hash + created_at + user_model
    // payload) MUST produce identical binding state hashes. Before the fix
    // the mount path generated UUID-based scratchpad revision and op ids on
    // each apply, so `hash_agent_binding` returned a different value every
    // time and binding receipts were not replayable for mounted user-model
    // runs.
    use theorem_harness_core::{
        apply_binding_transition, hash_agent_binding, AgentHeadRegistry, BindingTransitionInput,
    };

    let mut model = UserModel::default();
    model
        .preferences
        .insert("voice".to_string(), "spare".to_string());
    model.style_notes.push(UserModelNote::new(
        "no em-dashes",
        "2026-06-28T00:00:00Z",
    ));
    let user_model_value = serde_json::to_value(&model).unwrap();
    let created_at = "2026-06-28T00:00:00Z".to_string();

    fn apply_transition(
        binding: AgentBinding,
        event_type: &str,
        payload: serde_json::Map<String, serde_json::Value>,
        created_at: &str,
    ) -> AgentBinding {
        apply_binding_transition(
            binding,
            BindingTransitionInput {
                event_type: event_type.to_string(),
                payload,
                run_id: String::new(),
                actor: String::new(),
                created_at: created_at.to_string(),
            },
        )
        .unwrap()
        .binding
    }

    let user_model_value_for_a = user_model_value.clone();
    let created_at_for_a = created_at.clone();
    let user_model_value_for_b = user_model_value.clone();
    let created_at_for_b = created_at.clone();

    let build_through_mount = move |user_model_value: serde_json::Value,
                                    created_at: String|
          -> AgentBinding {
        let mut binding = fixture_binding();
        // Pin the per-process nondeterminism in `BindingLifecycleState::new()`
        // (UUID run_id + wall-clock created_at/updated_at) to fixed values so
        // the state-hash comparison isolates the mount-path determinism we
        // care about.
        binding.lifecycle.run_id = "bindingrun:replay-test".to_string();
        binding.lifecycle.created_at = created_at.clone();
        binding.lifecycle.updated_at = created_at.clone();
        let registry = AgentHeadRegistry::from_binding(&binding).unwrap();

        let mut resolved_payload = serde_json::Map::new();
        resolved_payload.insert("binding_id".to_string(), json!("agent:theorem"));
        resolved_payload.insert(
            "composition_hash".to_string(),
            json!("computed-by-kernel"),
        );
        let binding = apply_transition(
            binding,
            "BINDING.RESOLVED",
            resolved_payload,
            &created_at,
        );

        let binding = apply_transition(
            binding,
            "HEADS.PROBED",
            registry.heads_probed_payload(),
            &created_at,
        );

        let mut mount_payload = serde_json::Map::new();
        mount_payload.insert("scope_id".to_string(), json!("scope:test"));
        mount_payload.insert("scratchpad_id".to_string(), json!("scratchpad:test"));
        mount_payload.insert("user_model".to_string(), user_model_value);
        apply_transition(binding, "MEMORY_SCOPE.MOUNTED", mount_payload, &created_at)
    };

    let binding_a = build_through_mount(user_model_value_for_a, created_at_for_a);
    let binding_b = build_through_mount(user_model_value_for_b, created_at_for_b);

    assert_eq!(
        hash_agent_binding(&binding_a),
        hash_agent_binding(&binding_b),
        "two independent mount applies with the same input must produce the same binding state hash"
    );

    // And the scratchpad's mount-revision id is the deterministic seed,
    // not a UUID-prefixed one (any other ScratchpadDocument field drift
    // would still trip the state-hash equality above).
    let mount_rev = binding_a
        .working_memory_scope
        .scratchpad
        .revisions
        .last()
        .expect("scratchpad gained the mount revision");
    assert!(
        mount_rev.revision_id.starts_with("scratchrev:mount:"),
        "mount revision should carry the deterministic seed prefix, got {}",
        mount_rev.revision_id
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

// --- Agent Theorem S3 #73 P1: lineage_memory threaded into head requests ----
//
// When `MEMORY_SCOPE.MOUNTED` projects a lineage memory entry as a
// `lineage:agent_published` scratchpad revision, the intra-agent loop must
// add it to its local `revisions` vec so the first proposal/critique/synthesis
// `invoke_head` call carries the lineage revision_id in
// `prior_revision_ids`. Without P1 the kernel mounts the memory into the
// binding but the heads never receive it.
#[test]
fn fake_loop_threads_lineage_memory_into_head_requests() {
    let invoker = RecordingInvoker::default();
    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    let lineage_entry = BindingLineageMemoryEntry {
        source_binding_id: "harness:binding:agent:theorem:v1".to_string(),
        source_composition_hash: "comp:v1".to_string(),
        source_version: 1,
        summary: "prior binding agent:theorem:v1 (v1) published memory".to_string(),
        patch_ids: vec!["patch:1".to_string()],
        substrate_receipt_id: "substrate:1".to_string(),
        published_at: "1970-01-01T00:00:01.000Z".to_string(),
    };
    input.lineage_memory = vec![lineage_entry.clone()];

    let result = theorem_harness_core::run_intra_agent_loop_with_invoker(
        fixture_binding(),
        input,
        &invoker,
    )
    .unwrap();

    // The kernel must have appended one synthetic lineage:agent_published
    // revision into the binding scratchpad (the existing P1 invariant on the
    // MEMORY_SCOPE.MOUNTED arm). Capture its id so we can assert the heads
    // saw the same id in their prior_revision_ids.
    let lineage_revisions: Vec<_> = result
        .binding
        .working_memory_scope
        .scratchpad
        .revisions
        .iter()
        .filter(|revision| revision.actor_head_id == "lineage:agent_published")
        .collect();
    assert_eq!(
        lineage_revisions.len(),
        1,
        "MEMORY_SCOPE.MOUNTED must project the lineage entry as one scratchpad revision"
    );
    let lineage_revision_id = lineage_revisions[0].revision_id.clone();

    // Each invoked head must carry that lineage revision_id in
    // prior_revision_ids. Proposal sees only it (the first revision in the
    // loop); critique/synthesis/verification see it plus the contributions
    // appended along the way.
    let requests = invoker.requests.into_inner();
    assert!(
        !requests.is_empty(),
        "loop must invoke at least one head; got 0 requests"
    );

    let proposal = requests
        .iter()
        .find(|request| request.kind == HeadInvocationKind::Proposal)
        .expect("loop must produce a proposal request");
    assert!(
        proposal.prior_revision_ids.contains(&lineage_revision_id),
        "P1: proposal must see the lineage revision_id in prior_revision_ids; got {:?}",
        proposal.prior_revision_ids
    );

    for kind in [
        HeadInvocationKind::Critique,
        HeadInvocationKind::Synthesis,
        HeadInvocationKind::Verification,
    ] {
        let request = requests
            .iter()
            .find(|request| request.kind == kind)
            .unwrap_or_else(|| panic!("loop must produce a {kind:?} request"));
        assert!(
            request.prior_revision_ids.contains(&lineage_revision_id),
            "P1: {kind:?} must see the lineage revision_id; got {:?}",
            request.prior_revision_ids
        );
    }

    // The lineage revision payload (kind = "lineage_memory") is intentionally
    // outside the four typed HeadInvocationKind variants, so it does NOT
    // appear in prior_context (which is typed by HeadInvocationKind). The
    // contract is: heads see the lineage by revision_id reference and can
    // resolve the payload through the shared scratchpad CRDT. Pin that
    // boundary so future refactors do not silently widen the typed slot.
    assert!(
        proposal
            .prior_context
            .iter()
            .all(|ctx| ctx.revision_id != lineage_revision_id),
        "lineage revisions must not collapse into the typed prior_context slot"
    );
}

// --- F2 (PR #73 CodeRabbit): post-MOUNTED capture is scoped to the new slice -
//
// If a binding loaded into the loop already carries a stale
// `lineage:agent_published` revision on its scratchpad (e.g. from a previous
// run that persisted state), the post-MOUNTED capture must NOT re-add it to
// the loop's local `revisions` vec. With the F2 fix the scan iterates only
// the slice appended in this MOUNTED call.
#[test]
fn fake_loop_post_mounted_capture_ignores_pre_existing_lineage_revisions() {
    let invoker = RecordingInvoker::default();
    let mut binding = fixture_binding();
    // Stale prior-run revision sitting on the binding's scratchpad BEFORE the
    // loop runs. The actor id matches the synthetic lineage actor so it would
    // be matched by the post-MOUNTED scan if the scan saw it.
    let stale = binding.working_memory_scope.scratchpad.append(
        "lineage:agent_published",
        "stale prior-run lineage memory (must not be re-captured)",
        "hash:stale",
        serde_json::Map::new(),
        "1970-01-01T00:00:00.000Z",
    );
    let stale_revision_id = stale.revision_id.clone();

    let lineage_entry = BindingLineageMemoryEntry {
        source_binding_id: "harness:binding:agent:theorem:v2".to_string(),
        source_composition_hash: "comp:v2".to_string(),
        source_version: 2,
        summary: "prior binding agent:theorem:v2 (v2) published memory".to_string(),
        patch_ids: vec!["patch:fresh".to_string()],
        substrate_receipt_id: "substrate:fresh".to_string(),
        published_at: "1970-01-01T00:00:02.000Z".to_string(),
    };
    let mut input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    input.lineage_memory = vec![lineage_entry];

    let result =
        theorem_harness_core::run_intra_agent_loop_with_invoker(binding, input, &invoker).unwrap();

    // The MOUNTED arm appends exactly one fresh lineage revision; the stale
    // one is still on the scratchpad (we never delete revisions), but the
    // post-MOUNTED capture must NOT have surfaced it to the heads.
    let lineage_count = result
        .binding
        .working_memory_scope
        .scratchpad
        .revisions
        .iter()
        .filter(|revision| revision.actor_head_id == "lineage:agent_published")
        .count();
    assert_eq!(lineage_count, 2, "stale + one freshly appended");

    let requests = invoker.requests.into_inner();
    let proposal = requests
        .iter()
        .find(|request| request.kind == HeadInvocationKind::Proposal)
        .expect("loop must produce a proposal request");
    assert!(
        !proposal.prior_revision_ids.contains(&stale_revision_id),
        "F2: stale pre-existing lineage revision must NOT enter prior_revision_ids; got {:?}",
        proposal.prior_revision_ids
    );
}

#[test]
fn fake_loop_omits_lineage_memory_revisions_when_input_has_none() {
    // Back-compat: when input.lineage_memory is empty, no
    // lineage:agent_published revisions are appended and head requests look
    // exactly like the pre-S3 contract -- prior_revision_ids on proposal is
    // just the PRIVATE_WORK.OPENED revision id.
    let invoker = RecordingInvoker::default();
    let input =
        FakeIntraAgentLoopInput::new("publish", vec![GroundedClaim::new("grounded", "source:1")]);
    assert!(input.lineage_memory.is_empty());

    let result = theorem_harness_core::run_intra_agent_loop_with_invoker(
        fixture_binding(),
        input,
        &invoker,
    )
    .unwrap();

    let lineage_count = result
        .binding
        .working_memory_scope
        .scratchpad
        .revisions
        .iter()
        .filter(|revision| revision.actor_head_id == "lineage:agent_published")
        .count();
    assert_eq!(lineage_count, 0);

    let requests = invoker.requests.into_inner();
    let proposal = requests
        .iter()
        .find(|request| request.kind == HeadInvocationKind::Proposal)
        .expect("loop must produce a proposal request");
    assert_eq!(
        proposal.prior_revision_ids.len(),
        1,
        "back-compat: proposal sees only the PRIVATE_WORK.OPENED revision when no lineage memory is threaded"
    );
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
