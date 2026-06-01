use serde_json::{json, Map, Value};
use theorem_harness_core::{
    affordance_by_id, affordance_ids, default_affordance_registry, validate_affordance_registry,
    AffordanceReceipt,
};

#[test]
fn affordance_registry_covers_python_projection_surface() {
    let ids = affordance_ids();
    assert_eq!(
        ids,
        vec![
            "datalog.derive",
            "probabilistic.source_reliability",
            "probabilistic.expected_value_of_information",
            "causal.intervention_effect",
            "evolution.archive",
            "proof.create_obligation",
            "optimizer.optimize",
            "expression.render",
            "egraph.extract",
            "simulation.dry_run",
            "solver.check",
        ]
    );
    assert_eq!(default_affordance_registry().len(), 11);
    validate_affordance_registry().expect("registry should be internally valid");
}

#[test]
fn affordance_lookup_preserves_execution_boundary_metadata() {
    let datalog = affordance_by_id("datalog.derive").expect("datalog should be registered");
    assert_eq!(datalog.execution_surface, "rustyred-thg-core");
    assert_eq!(datalog.parity_status, "native-parity");

    let solver = affordance_by_id("solver.check").expect("solver should be registered");
    assert_eq!(solver.writeback_policy, "proposal-only");
    assert_eq!(solver.execution_surface, "runtime-adapter");
}

#[test]
fn affordance_receipt_hash_is_content_addressed() {
    let mut payload = Map::new();
    payload.insert("status".to_string(), json!("passed"));
    payload.insert("count".to_string(), json!(3));

    let receipt = AffordanceReceipt::new(
        "simulation-receipt-fallback",
        "simulation.dry_run",
        "input-hash-1",
        payload.clone(),
    )
    .with_input_node_refs(vec!["validator:schema".to_string()])
    .with_writeback_policy("read-only");

    let same = AffordanceReceipt::new(
        "simulation-receipt-fallback",
        "simulation.dry_run",
        "input-hash-1",
        payload,
    )
    .with_input_node_refs(vec!["validator:schema".to_string()])
    .with_writeback_policy("read-only");

    assert_eq!(receipt.receipt_hash, same.receipt_hash);
    assert_eq!(receipt.receipt_hash, receipt.computed_receipt_hash());

    let mut changed_payload = Map::new();
    changed_payload.insert("status".to_string(), Value::String("failed".to_string()));
    let changed = AffordanceReceipt::new(
        "simulation-receipt-fallback",
        "simulation.dry_run",
        "input-hash-1",
        changed_payload,
    )
    .with_input_node_refs(vec!["validator:schema".to_string()]);
    assert_ne!(receipt.receipt_hash, changed.receipt_hash);
}
