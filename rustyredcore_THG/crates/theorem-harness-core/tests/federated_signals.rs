use serde_json::{json, Map};
use theorem_harness_core::{
    assert_no_raw_content, extract_structural_signal, observed_count_bucket,
    receive_federated_signal, success_rate_bucket, StructuralSignalInput,
};

#[test]
fn privacy_filter_rejects_forbidden_keys_anywhere() {
    let err = assert_no_raw_content(&json!({
        "kind": "agent",
        "nested": [
            {"edge_types": ["skill"]},
            {"tenant_id": "t-1"}
        ]
    }))
    .expect_err("tenant identifiers must be rejected");

    assert!(err.message.contains("Forbidden key"));
    assert!(err.message.contains("tenant_id"));
}

#[test]
fn receive_federated_signal_normalizes_safe_shape() {
    let signal = receive_federated_signal(&json!({
        "kind": "skill",
        "edge_types": ["uses", "validates"],
        "success_rate_bucket": "medium",
        "observed_count": 20,
        "visual_class": "tool",
        "suggestion_type": "route"
    }))
    .expect("safe structural signal should pass");

    assert_eq!(signal.kind, "skill");
    assert_eq!(signal.edge_types, vec!["uses", "validates"]);
    assert_eq!(signal.success_rate_bucket, "medium");
    assert_eq!(signal.observed_count, 20);
    assert_eq!(signal.visual_class, "tool");
    assert_eq!(signal.suggestion_type, "route");
}

#[test]
fn receive_federated_signal_rejects_non_objects() {
    let err = receive_federated_signal(&json!(["not", "a", "dict"]))
        .expect_err("federated signal must be an object");
    assert!(err.message.contains("must be a dict"));
}

#[test]
fn bucket_helpers_match_python_privacy_contract() {
    assert_eq!(success_rate_bucket(0.66), "high");
    assert_eq!(success_rate_bucket(0.33), "medium");
    assert_eq!(success_rate_bucket(0.32), "low");

    assert_eq!(observed_count_bucket(0), 0);
    assert_eq!(observed_count_bucket(5), 10);
    assert_eq!(observed_count_bucket(25), 20);
    assert_eq!(observed_count_bucket(27), 30);
    assert_eq!(observed_count_bucket(35), 40);
}

#[test]
fn extractor_projects_structural_patch_input_without_raw_content() {
    let mut structural_signal = Map::new();
    structural_signal.insert("skill_ids".to_string(), json!(["a", "b"]));
    structural_signal.insert("tool_ids".to_string(), json!(["t"]));
    structural_signal.insert("empty_ids".to_string(), json!([]));

    let signal = extract_structural_signal(StructuralSignalInput {
        kind: "rule".to_string(),
        structural_signal,
        evidence_count: 27,
        confidence: 0.67,
        visual_class: "class-a".to_string(),
        suggestion_type: "promote".to_string(),
    })
    .expect("structural projection should pass");

    assert_eq!(signal.kind, "rule");
    assert_eq!(signal.edge_types, vec!["skill", "tool"]);
    assert_eq!(signal.success_rate_bucket, "high");
    assert_eq!(signal.observed_count, 30);
    assert_eq!(signal.visual_class, "class-a");
    assert_eq!(signal.suggestion_type, "promote");
}
