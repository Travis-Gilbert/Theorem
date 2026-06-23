use serde_json::{json, to_value};
use theorem_harness_core::{
    PrepareMemoryBank, PrepareMemoryContract, PrepareMemoryHydrationHandle,
    PrepareMemoryRecallPolicy, PrepareMemoryRecallPreview,
};

#[test]
fn hydration_handles_and_banks_normalize_strings() {
    let handle = PrepareMemoryHydrationHandle::from_value(&json!({
        "handle_id": "h-1",
        "handle_type": "record",
        "source": "AGENTS.md",
        "reason": "read first",
        "scope": "repo"
    }));
    assert_eq!(handle.handle_id, "h-1");
    assert_eq!(handle.status, "");

    let bank = PrepareMemoryBank::from_value(&json!({
        "bank_id": "project",
        "kind": "memory",
        "scope": "Theorem",
        "selector": "active",
        "rationale": "repo context"
    }));
    assert_eq!(bank.bank_id, "project");
    assert_eq!(bank.rationale, "repo context");
}

#[test]
fn recall_policy_normalizes_lists_weights_and_status() {
    let policy = PrepareMemoryRecallPolicy::from_value(&json!({
        "policy_id": "recall-1",
        "kind": "task",
        "scope_filters": ["repo", "harness"],
        "selected_banks": ["project", "user"],
        "bank_weights": {
            " Project ": "1.5",
            "negative": -1,
            "invalid": "nope"
        },
        "status": ""
    }));

    assert_eq!(policy.scope_filters, vec!["repo", "harness"]);
    assert_eq!(policy.selected_banks, vec!["project", "user"]);
    assert_eq!(policy.bank_weights.len(), 1);
    assert_eq!(policy.bank_weights["project"], 1.5);
    assert_eq!(policy.status, "active");
}

#[test]
fn recall_preview_parses_nested_hydration_handles() {
    let preview = PrepareMemoryRecallPreview::from_value(&json!({
        "read_first": ["AGENTS.md"],
        "risks": ["stale mirror"],
        "do_not": ["include secrets"],
        "next_actions": ["run cargo test"],
        "hydration_handles": [
            {
                "handle_id": "h-1",
                "handle_type": "map",
                "source": "CodebaseMap",
                "reason": "orientation",
                "status": "ready"
            },
            "ignore me"
        ],
        "recalled_evidence": ["ev-1"],
        "selected_banks": ["project"],
        "recall_policy": ["policy-1"],
        "active_policy": ["active"],
        "proposed_policy": ["proposed"]
    }));

    assert_eq!(preview.read_first, vec!["AGENTS.md"]);
    assert_eq!(preview.hydration_handles.len(), 1);
    assert_eq!(preview.hydration_handles[0].handle_id, "h-1");
    assert_eq!(preview.selected_banks, vec!["project"]);
}

#[test]
fn contract_from_value_parses_nested_sections_and_defaults() {
    let contract = PrepareMemoryContract::from_value(&json!({
        "evidence": [
            {
                "evidence_id": "ev-1",
                "kind": "record",
                "source": "CLAIMS.md",
                "immutable": 0,
                "payload": {"path": "docs/plans/harness-rust-port/CLAIMS.md"}
            },
            ["ignore", "non-object"]
        ],
        "operational_policy": [
            {
                "policy_id": "pol-1",
                "kind": "coordination",
                "scope": "repo",
                "editable": "",
                "status": "",
                "payload": {"mode": "git"}
            }
        ],
        "memory_banks": [
            {
                "bank_id": "bank-1",
                "kind": "project",
                "scope": "Theorem",
                "selector": "active"
            }
        ],
        "evidence_hash": "sha256:evidence",
        "policy_hash": "sha256:policy",
        "recall_policy": {
            "policy_id": "recall-1",
            "kind": "task",
            "selected_banks": ["bank-1"]
        },
        "recall_preview": {
            "read_first": ["AGENTS.md"],
            "selected_banks": ["bank-1"]
        }
    }));

    assert_eq!(contract.evidence.len(), 1);
    assert!(!contract.evidence[0].immutable);
    assert_eq!(contract.operational_policy.len(), 1);
    assert!(!contract.operational_policy[0].editable);
    assert_eq!(contract.operational_policy[0].status, "active");
    assert_eq!(contract.memory_banks.len(), 1);
    assert_eq!(contract.evidence_hash, "sha256:evidence");
    assert_eq!(
        contract.recall_policy.as_ref().unwrap().selected_banks,
        vec!["bank-1"]
    );
    assert_eq!(
        contract.recall_preview.as_ref().unwrap().read_first,
        vec!["AGENTS.md"]
    );

    let serialized = to_value(&contract).expect("contract serializes");
    assert_eq!(serialized["evidence"][0]["immutable"], false);
    assert_eq!(serialized["operational_policy"][0]["editable"], false);
    assert_eq!(serialized["recall_policy"]["status"], "active");
}
