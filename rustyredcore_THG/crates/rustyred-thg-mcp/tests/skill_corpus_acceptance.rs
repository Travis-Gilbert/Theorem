//! T7 acceptance: skill-corpus bring-up, end to end.
//!
//! Proves the addendum's skill-corpus acceptance on one store:
//! - publishing the three engineering packs makes `skill_list` return them (not zero);
//! - applying a pack records a use receipt;
//! - a repeatedly-useful pack ranks higher in a later `ensemble_select`, because `skill_apply`
//!   receipts compound through the run-close hook (the Compound spine) and the ensemble selector
//!   reads that compound standing as live pack fitness.
//!
//! The same dual-labeled `["CapabilityPack", "SkillPack"]` node serves both the skill-pack loop
//! and the ensemble registry: "the skill corpus is the same machine as the tool corpus."

use ensemble::selector::{select_from_store, EnsembleSelectRequest};
use rustyred_thg_core::InMemoryGraphStore;
use serde_json::{json, Value};
use theorem_harness_core::TransitionInput;
use theorem_harness_runtime::engineering_packs::{
    publish_engineering_packs, RUST_ENGINEERING_PACK_CONTENT_HASH,
};
use theorem_harness_runtime::event_log::append_transition_from_store;
use theorem_harness_runtime::skill_pack::{
    apply_skill_pack, list_skill_packs, SkillPackApplyInput, SkillPackApplyReceipt,
    SkillPackListInput,
};

const TENANT: &str = "default";
const TS: &str = "2026-06-20T00:00:00Z";

fn transition(run_id: &str, event_type: &str, payload: Value) -> TransitionInput {
    TransitionInput {
        run_id: run_id.to_string(),
        event_type: event_type.to_string(),
        payload: payload.as_object().cloned().unwrap_or_default(),
        actor: "claude-code".to_string(),
        idempotency_key: format!("{run_id}:{event_type}"),
        created_at: TS.to_string(),
    }
}

fn append(store: &mut InMemoryGraphStore, run_id: &str, event_type: &str, payload: Value) {
    append_transition_from_store(store, transition(run_id, event_type, payload)).unwrap();
}

/// Apply `pack_hash` inside a run and close that run with a positive outcome, mirroring the
/// harness run lifecycle so the Compound spine records a use receipt and compounds the pack's
/// standing by one positive run. Returns the apply receipt.
fn apply_and_close_positive_run(
    store: &mut InMemoryGraphStore,
    run_id: &str,
    task: &str,
    pack_hash: &str,
) -> SkillPackApplyReceipt {
    append(
        store,
        run_id,
        "RUN.CREATED",
        json!({
            "task": task,
            "actor": "claude-code",
            "scope": {"tenant_slug": TENANT, "repo": "Theorem", "agent_host": "claude-code"}
        }),
    );
    // Record the skill application: writes a SkillPackUseReceipt keyed by run_id, which the
    // run-close hook reads to compound this pack's standing.
    let receipt = apply_skill_pack(
        store,
        SkillPackApplyInput {
            tenant_slug: TENANT.to_string(),
            pack_content_hash: pack_hash.to_string(),
            actor_id: "claude-code".to_string(),
            run_id: run_id.to_string(),
            task: task.to_string(),
            ..SkillPackApplyInput::default()
        },
    )
    .unwrap();
    for (event_type, payload) in [
        (
            "HOST.OBSERVED",
            json!({"repo":"Theorem","branch":"main","commit_sha":"abc123","cwd":"/repo/Theorem"}),
        ),
        ("TASK.RESOLVED", json!({"task_signature": task})),
        (
            "PROFILE.SELECTED",
            json!({"profile_id":"claude-code","profile_version":"1","policy_hash":"policy:1"}),
        ),
        (
            "TOOLKIT.COMPILED",
            json!({"selected_tools":["apply_patch","cargo test"],"selected_plugins":[],"excluded_tools":[],"permission_reasons":{},"tool_permission_requirements":{},"policy_receipts":[]}),
        ),
        (
            "CONTEXT.PLANNED",
            json!({"budget_tokens":4000,"plan_hash":"plan:1","candidate_token_count":1200}),
        ),
        (
            "CONTEXT.PACKED",
            json!({"artifact_id":"ctx:1","capsule_tokens":1000,"budget_tokens":4000,"included_atom_count":2,"excluded_atom_count":0,"token_ledger":{},"memory_doc_ids":[]}),
        ),
        (
            "CONTEXT.INJECTED",
            json!({"artifact_id":"ctx:1","adapter":"claude-code","target":"active_context"}),
        ),
        ("AGENT.ACTING", json!({"adapter":"claude-code","started_at": TS})),
        (
            "SESSION.EVENT_RECORDED",
            json!({"event_subtype":"tool_invocation","tool_id":"apply_patch"}),
        ),
        (
            "OUTCOME.RECORDED",
            json!({"accepted":true,"tests_passed":true,"manual_override":true,"validator_results":[],"files_changed":[],"summary":"accepted"}),
        ),
        (
            "RUN.CLOSED",
            json!({"summary":"done","closed_by":"claude-code","source_identifiers":[]}),
        ),
    ] {
        append(store, run_id, event_type, payload);
    }
    receipt
}

fn select(store: &InMemoryGraphStore, task: &str) -> ensemble::decision::EnsembleDecision {
    select_from_store(
        store,
        TENANT,
        None,
        EnsembleSelectRequest {
            task: task.to_string(),
            budget_units: Some(100_000),
            max_selected: Some(3),
            candidates: Vec::new(),
            priors: json!({}),
        },
    )
    .unwrap()
}

#[test]
fn skill_corpus_publish_apply_and_rank_end_to_end() {
    let mut store = InMemoryGraphStore::new();

    // Acceptance 1: publishing the three engineering packs makes skill_list return them.
    let receipts = publish_engineering_packs(&mut store, TENANT, "claude-code").unwrap();
    assert_eq!(receipts.len(), 3);
    let listed = list_skill_packs(
        &store,
        SkillPackListInput {
            tenant_slug: TENANT.to_string(),
            limit: 50,
            ..SkillPackListInput::default()
        },
    )
    .unwrap();
    assert!(
        listed.len() >= 3,
        "skill_list must return the published packs, got {}",
        listed.len()
    );

    // A neutral task so all three "*-engineering" packs share equal lexical overlap and the
    // compound fitness is the differentiator that moves the ranking.
    let task = "engineering pack work";
    let before = select(&store, task);
    let rust_before = before
        .selected
        .iter()
        .find(|pack| pack.pack_content_hash == RUST_ENGINEERING_PACK_CONTENT_HASH)
        .map(|pack| pack.score)
        .expect("rust-engineering is a selectable candidate before any use");

    // Acceptance 2 + 3: apply rust-engineering across three positive closed runs (repeatedly
    // useful). Each apply records a use receipt.
    for index in 0..3 {
        let receipt =
            apply_and_close_positive_run(&mut store, &format!("run-rust-{index}"), task, RUST_ENGINEERING_PACK_CONTENT_HASH);
        assert_eq!(receipt.status, "applied", "apply records a use receipt");
        assert_eq!(receipt.run_id, format!("run-rust-{index}"));
    }

    let after = select(&store, task);

    // Acceptance 3: the repeatedly-useful pack ranks higher -- rust-engineering is now first.
    let top = after.selected.first().expect("at least one selected pack");
    assert_eq!(
        top.pack_content_hash, RUST_ENGINEERING_PACK_CONTENT_HASH,
        "rust-engineering should rank first after repeated useful applies; selected={:?}",
        after.selected
    );
    let rust_after = after
        .selected
        .iter()
        .find(|pack| pack.pack_content_hash == RUST_ENGINEERING_PACK_CONTENT_HASH)
        .map(|pack| pack.score)
        .expect("rust-engineering is still selectable after use");
    assert!(
        rust_after > rust_before,
        "compounding should raise rust-engineering's score: before={rust_before} after={rust_after}"
    );

    // The other two packs were never used, so they must not have overtaken rust.
    assert!(
        after
            .selected
            .iter()
            .filter(|pack| pack.pack_content_hash != RUST_ENGINEERING_PACK_CONTENT_HASH)
            .all(|pack| pack.score <= rust_after),
        "an unused pack outranked the repeatedly-useful one: {:?}",
        after.selected
    );
}
