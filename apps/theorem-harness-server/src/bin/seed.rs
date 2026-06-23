//! Demo seed: write one complete harness run through the real runtime into a
//! durable RedCore store, so `theorem-harness-server` can serve it and the iOS
//! client can render it live. This is a dev tool (like the iOS smoke binary),
//! not product code: the plumbing is 100% real (runtime append -> RedCore ->
//! HTTP -> client); only the run content is a known fixture lifecycle.
//!
//! Usage: THEOREM_HARNESS_DATA_DIR=/tmp/harness-demo cargo run --bin seed

use rustyred_thg_core::{RedCoreGraphStore, RedCoreOptions};
use serde_json::{json, Map, Value};
use theorem_harness_core::TransitionInput;
use theorem_harness_runtime::append_transition_from_store;

fn payload(pairs: &[(&str, Value)]) -> Map<String, Value> {
    let mut map = Map::new();
    for (key, value) in pairs {
        map.insert((*key).to_string(), value.clone());
    }
    map
}

fn main() {
    let data_dir =
        std::env::var("THEOREM_HARNESS_DATA_DIR").unwrap_or_else(|_| "harness-data".to_string());
    let mut store =
        RedCoreGraphStore::open(&data_dir, RedCoreOptions::default()).expect("open RedCore store");

    let run_id = "run-demo-0001";
    let steps: Vec<(&str, Map<String, Value>)> = vec![
        (
            "RUN.CREATED",
            payload(&[
                ("task", json!("port harness to rust")),
                ("actor", json!("claude-code")),
            ]),
        ),
        (
            "HOST.OBSERVED",
            payload(&[
                ("repo", json!("Theorem")),
                ("branch", json!("main")),
                ("commit_sha", json!("deadbeef")),
                ("cwd", json!("/repo/Theorem")),
            ]),
        ),
        (
            "TASK.RESOLVED",
            payload(&[("task_signature", json!("sig-port-harness"))]),
        ),
        (
            "PROFILE.SELECTED",
            payload(&[
                ("profile_id", json!("rust-port")),
                ("profile_version", json!("1")),
                ("policy_hash", json!("policy-abc")),
            ]),
        ),
        (
            "TOOLKIT.COMPILED",
            payload(&[
                ("selected_tools", json!(["read", "edit"])),
                ("selected_plugins", json!([])),
                ("excluded_tools", json!(["network"])),
                ("permission_reasons", json!({"network": "policy:no-egress"})),
            ]),
        ),
        (
            "MAPS.LOADED",
            payload(&[("maps", json!([{"id": "codebase", "version": "1"}]))]),
        ),
        (
            "CONTEXT.PLANNED",
            payload(&[
                ("budget_tokens", json!(1000)),
                ("plan_hash", json!("plan-1")),
                ("candidate_token_count", json!(500)),
            ]),
        ),
        (
            "CONTEXT.PACKED",
            payload(&[
                ("artifact_id", json!("art-1")),
                ("capsule_tokens", json!(200)),
                ("budget_tokens", json!(1000)),
                ("included_atom_count", json!(5)),
                ("excluded_atom_count", json!(2)),
                ("token_ledger", json!({"saved": 300})),
            ]),
        ),
        (
            "CONTEXT.INJECTED",
            payload(&[
                ("artifact_id", json!("art-1")),
                ("adapter", json!("mcp")),
                ("target", json!("claude")),
            ]),
        ),
        (
            "AGENT.ACTING",
            payload(&[
                ("adapter", json!("mcp")),
                ("started_at", json!("2026-06-01T00:00:00Z")),
            ]),
        ),
        (
            "OUTCOME.RECORDED",
            payload(&[
                ("accepted", json!(true)),
                ("tests_passed", json!(true)),
                (
                    "validator_results",
                    json!([{"id": "v1", "status": "passed"}]),
                ),
                ("files_changed", json!(["state_machine.rs"])),
                ("summary", json!("ported")),
            ]),
        ),
        (
            "LEARNING.PROPOSED",
            payload(&[
                ("patch_type", json!("memory")),
                ("confidence", json!(0.8)),
                ("review_required", json!(true)),
                ("payload_hash", json!("patch-1")),
            ]),
        ),
        (
            "REVIEW.QUEUED",
            payload(&[
                ("review_type", json!("memory")),
                ("review_target_id", json!("patch-1")),
            ]),
        ),
        (
            "FEDERATION.SIGNAL_PREPARED",
            payload(&[
                ("plugin_id", json!("core")),
                ("profile_id", json!("rust-port")),
                ("task_type", json!("port")),
                ("task_signature_hash", json!("tsh-1")),
                ("context_shape_hash", json!("csh-1")),
                ("outcome_bucket", json!("accepted")),
                ("token_bucket", json!("small")),
                ("raw_content_included", json!(false)),
                ("consent", json!(true)),
            ]),
        ),
        (
            "RUN.CLOSED",
            payload(&[
                ("summary", json!("harness kernel ported")),
                ("closed_by", json!("claude-code")),
            ]),
        ),
    ];

    for (event_type, body) in steps {
        let input = TransitionInput::new(event_type, body).with_run_id(run_id);
        let result = append_transition_from_store(&mut store, input)
            .unwrap_or_else(|error| panic!("{event_type}: {error}"));
        let hash = &result.state_hash_after[..12.min(result.state_hash_after.len())];
        println!("{event_type:<28} -> {:<22} {hash}", result.run.status);
    }
    println!("seeded run {run_id} into {data_dir}");
}
