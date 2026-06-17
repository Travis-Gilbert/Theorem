//! Independent acceptance suite for Handoff 4 (Unify the Learned Tool Graph).
//!
//! Written by the verifier head (claude-code) against the PUBLIC api of `ensemble`,
//! with zero edits to the crate source. It confirms the load-bearing cut-4 criteria
//! from the outside, end-to-end, and is deliberately complementary to codex's
//! in-crate unit tests:
//!   - Criterion 1 (pack outcome lifts later selection via the live graph, empty
//!     offline priors) and Criterion 3 (pure `select` stays replayable) and
//!     Criterion 5 (trust/budget gates still reject) are proven here independently.
//!   - Criterion 2 (pack fitness decays like affordance fitness) is covered by
//!     `outcomes::tests` (`effective_pack_fitness_from_node` stale < fresh).
//!   - Criterion 4 (one ordering across packs AND affordances, domain reorders) is
//!     covered by `selector::tests::unified_selection_returns_packs_and_affordances_with_domain_bias`.
//! Both of those ran green alongside this suite.

use ensemble::{
    record_pack_invocation, register_pack, select, select_from_store, CapabilityPack,
    EnsembleSelectRequest, PackExposure, PackInvocationRecordRequest, TrustTier,
};
use rustyred_thg_core::InMemoryGraphStore;
use serde_json::{json, Value};

const TENANT: &str = "default";

fn pack(title: &str, description: &str, caps: &[&str], trust: TrustTier) -> CapabilityPack {
    CapabilityPack {
        tenant_slug: TENANT.to_string(),
        origin_tenant_slug: String::new(),
        pack_content_hash: String::new(),
        kind: "skill".to_string(),
        title: title.to_string(),
        description: description.to_string(),
        spec: json!({ "kind": "skill", "capabilities": caps }),
        trust,
        exposure: PackExposure::default(),
        source_content_hash: String::new(),
        artifact_hashes: vec![],
    }
}

fn score_of(decision: &ensemble::EnsembleDecision, hash: &str) -> Option<f64> {
    decision
        .selected
        .iter()
        .find(|s| s.pack_content_hash == hash)
        .map(|s| s.score)
}

/// Criterion 1: "After a recorded pack outcome and with empty offline priors,
/// selecting for the same task shape lifts that pack through the live graph,
/// demonstrating the structural signal no longer depends on an offline workbench
/// rerun."
///
/// The task vocabulary deliberately does NOT overlap either pack's text, so lexical
/// overlap is zero for both and the ranking is driven purely by the live structural
/// signal. Pack A receives a recorded outcome (which writes PACK_SERVED_TASK to the
/// task-type node and bumps fitness); pack B does not. With empty offline priors,
/// `select_from_store` must rank A above B purely from the live graph.
#[test]
fn c1_recorded_pack_outcome_lifts_selection_via_live_graph() {
    let mut store = InMemoryGraphStore::new();
    let task = "alpha bravo charlie";
    let a = register_pack(
        &mut store,
        pack(
            "Toolkit One",
            "delta echo foxtrot",
            &["delta"],
            TrustTier::Unverified,
        ),
    )
    .unwrap();
    let b = register_pack(
        &mut store,
        pack(
            "Toolkit Two",
            "golf hotel india",
            &["golf"],
            TrustTier::Unverified,
        ),
    )
    .unwrap();

    // Build a real decision selecting A (only candidate), then record a strong
    // positive outcome for it on this task shape.
    let decision_a = select(&EnsembleSelectRequest {
        task: task.to_string(),
        budget_units: None,
        max_selected: Some(1),
        candidates: vec![a.clone()],
        priors: Value::Null,
    });
    record_pack_invocation(
        &mut store,
        PackInvocationRecordRequest {
            tenant_slug: TENANT.to_string(),
            decision: decision_a,
            outcome_value: 1.0,
            outcome_weight: 8.0,
            outcome_label: String::new(),
            previous_pack_content_hash: None,
            domain_refs: vec![],
            recorded_at_ms: None,
        },
        Some("verifier"),
    )
    .unwrap();

    // Empty offline priors: the only differentiator is the live graph.
    let decision = select_from_store(
        &store,
        TENANT,
        None,
        EnsembleSelectRequest {
            task: task.to_string(),
            budget_units: None,
            max_selected: Some(2),
            candidates: vec![],
            priors: Value::Null,
        },
    )
    .unwrap();

    let pos_a = decision
        .selected
        .iter()
        .position(|s| s.pack_content_hash == a.pack_content_hash);
    let pos_b = decision
        .selected
        .iter()
        .position(|s| s.pack_content_hash == b.pack_content_hash);
    assert_eq!(
        pos_a,
        Some(0),
        "the pack with a recorded outcome must rank first via the live graph; selected: {:?}",
        decision
            .selected
            .iter()
            .map(|s| (s.pack_content_hash.as_str(), s.score))
            .collect::<Vec<_>>()
    );
    let score_a = score_of(&decision, &a.pack_content_hash).unwrap_or(0.0);
    let score_b = pos_b
        .and_then(|_| score_of(&decision, &b.pack_content_hash))
        .unwrap_or(0.0);
    assert!(
        score_a > score_b,
        "live structural lift must give the outcome pack a strictly higher score ({score_a} > {score_b})"
    );
}

fn pack_with_hash(
    hash: &str,
    title: &str,
    description: &str,
    caps: &[&str],
    trust: TrustTier,
) -> CapabilityPack {
    let mut p = pack(title, description, caps, trust);
    p.pack_content_hash = hash.to_string();
    p
}

/// Criterion 3: the pure `select` stays pure and replayable — identical inputs,
/// including a live-computed `pack_scores` prior, yield an identical decision
/// content address. (The PPR runs in the store-backed wrapper, never inside `select`.)
#[test]
fn c3_pure_select_is_replayable_with_live_prior() {
    let mk = || EnsembleSelectRequest {
        task: "graph traversal work".to_string(),
        budget_units: Some(5),
        max_selected: Some(2),
        candidates: vec![
            pack_with_hash(
                "h_alpha",
                "Alpha",
                "graph traversal",
                &["graph"],
                TrustTier::Unverified,
            ),
            pack_with_hash(
                "h_beta",
                "Beta",
                "graph build",
                &["graph"],
                TrustTier::Unverified,
            ),
        ],
        // A live-computed prior, injected as the store-backed wrapper would — `select`
        // treats it as opaque data, so replay must be byte-identical.
        priors: json!({ "pack_scores": { "h_alpha": 0.42 }, "prior_weight": 0.7 }),
    };
    let d1 = select(&mk());
    let d2 = select(&mk());
    assert_eq!(
        d1.content_address(),
        d2.content_address(),
        "identical inputs (incl. live-computed prior) must yield an identical content address"
    );
}

/// Criterion 5: the trust floor and budget gates still apply to the result. An
/// unverified pack under a first_party floor and an over-budget pack are rejected
/// with the existing reasons.
#[test]
fn c5_trust_floor_and_budget_gates_still_reject() {
    // Trust floor: an unverified pack is rejected under a first_party floor.
    let trust_decision = select(&EnsembleSelectRequest {
        task: "graph work".to_string(),
        budget_units: None,
        max_selected: None,
        candidates: vec![
            pack_with_hash(
                "unv",
                "Unverified",
                "graph",
                &["graph"],
                TrustTier::Unverified,
            ),
            pack_with_hash(
                "fp",
                "Trusted",
                "graph",
                &["graph"],
                TrustTier::FirstParty {
                    passport_id: "fp-1".to_string(),
                },
            ),
        ],
        priors: json!({ "min_trust": "first_party" }),
    });
    assert!(
        trust_decision
            .selected
            .iter()
            .all(|s| s.pack_content_hash == "fp"),
        "only the first_party pack may be selected under a first_party floor"
    );
    assert!(
        trust_decision
            .rejected
            .iter()
            .any(|r| r.pack_content_hash == "unv" && r.reason.contains("trust floor")),
        "the unverified pack must be rejected for the trust floor"
    );

    // Budget gate: two default-cost (1 unit) packs against a budget of 1 -> exactly
    // one fits, the other is rejected as over budget.
    let budget_decision = select(&EnsembleSelectRequest {
        task: "graph".to_string(),
        budget_units: Some(1),
        max_selected: None,
        candidates: vec![
            pack_with_hash("p1", "P1", "graph", &["graph"], TrustTier::Unverified),
            pack_with_hash("p2", "P2", "graph", &["graph"], TrustTier::Unverified),
        ],
        priors: Value::Null,
    });
    assert_eq!(
        budget_decision.selected.len(),
        1,
        "a budget of 1 admits exactly one default-cost pack"
    );
    assert!(
        budget_decision
            .rejected
            .iter()
            .any(|r| r.reason.contains("over budget")),
        "the second pack must be rejected as over budget"
    );
}
