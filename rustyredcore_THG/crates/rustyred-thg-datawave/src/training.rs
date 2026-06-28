//! Next Build Cut item 4: feed parity receipts into the same learned-scorer
//! training stream the reconstruction lane feeds.
//!
//! A `ParityReceipt` records one (fixture, field, expected, actual, passed)
//! comparison. It persists as a `DatawaveParityReceipt` graph node (mirroring the
//! reconstruction lane's `ValidationReceipt` node pattern) AND converts into the
//! shared `LabeledTrainingRun` -> `TrainingExportRecord` JSONL stream
//! (`rustyred_thg_core::{labeled_training_run, training_export}`), so successful
//! and failed parity checks become labeled training signals next to the
//! reconstruction lane's own labeled runs.

use rustyred_thg_core::labeled_training_run::{
    LabeledTrainingRun, TrainingLabel, TrainingLabelFamily, TrainingOutcome, TrainingTaskType,
    ValidatorResult,
};
use rustyred_thg_core::training_export::{
    export_records_jsonl, RedactionStatus, TrainingExportKind, TrainingExportRecord,
};
use rustyred_thg_core::{stable_hash, GraphStore, GraphStoreResult, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::BTreeMap;

use crate::materialize::{SOURCE, VERSION};

pub const PARITY_RECEIPT_LABEL: &str = "DatawaveParityReceipt";

/// One parity comparison against a reference fixture's expected output.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParityReceipt {
    pub receipt_id: String,
    /// The reference fixture this checks, e.g. "datawave/my-nci.csv".
    pub fixture: String,
    pub data_type: String,
    pub field: String,
    pub expected: String,
    pub actual: String,
    pub passed: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

impl ParityReceipt {
    pub fn new(
        fixture: impl Into<String>,
        data_type: impl Into<String>,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        let (fixture, data_type, field, expected, actual) =
            (fixture.into(), data_type.into(), field.into(), expected.into(), actual.into());
        let passed = expected == actual;
        let receipt_id = format!(
            "dw:parity:{}",
            stable_hash((
                fixture.as_str(),
                data_type.as_str(),
                field.as_str(),
                expected.as_str(),
                actual.as_str(),
            ))
        );
        Self { receipt_id, fixture, data_type, field, expected, actual, passed, notes: Vec::new() }
    }
}

/// Persist a parity receipt as a `DatawaveParityReceipt` node.
pub fn write_parity_receipt<S: GraphStore>(
    store: &mut S,
    receipt: &ParityReceipt,
    tenant_id: Option<&str>,
) -> GraphStoreResult<()> {
    let id = match tenant_id {
        Some(t) => format!("{}:{t}", receipt.receipt_id),
        None => receipt.receipt_id.clone(),
    };
    store.upsert_node(NodeRecord::new(
        id,
        [PARITY_RECEIPT_LABEL],
        json!({
            "fixture": receipt.fixture,
            "data_type": receipt.data_type,
            "field": receipt.field,
            "expected": receipt.expected,
            "actual": receipt.actual,
            "passed": receipt.passed,
            "notes": receipt.notes,
            "authority": if receipt.passed { "validated_parity" } else { "parity" },
            "source": SOURCE,
            "version": VERSION,
            "tenant_id": tenant_id,
        }),
    ))?;
    Ok(())
}

/// Convert a parity receipt into the shared labeled-training-run shape, with a
/// ValidatorPolicy label the learned scorer consumes.
pub fn parity_receipt_to_labeled_run(
    receipt: &ParityReceipt,
    run_id: impl Into<String>,
    actor: impl Into<String>,
    graph_version: u64,
) -> LabeledTrainingRun {
    let mut scope: BTreeMap<String, String> = BTreeMap::new();
    scope.insert("fixture".to_string(), receipt.fixture.clone());
    scope.insert("data_type".to_string(), receipt.data_type.clone());

    let mut run = LabeledTrainingRun::new(
        receipt.receipt_id.clone(),
        run_id,
        receipt.fixture.clone(),
        TrainingTaskType::Review,
        actor,
        scope,
        graph_version,
    );
    run.outcome = if receipt.passed { TrainingOutcome::Success } else { TrainingOutcome::Failure };
    run.accepted = receipt.passed;
    run.validator_results = vec![ValidatorResult::new(
        "datawave_parity",
        if receipt.passed { "pass" } else { "fail" },
    )];
    run.add_label(TrainingLabel::new(
        TrainingLabelFamily::ValidatorPolicy,
        if receipt.passed { "parity_pass" } else { "parity_fail" },
        receipt.field.clone(),
    ));
    run
}

/// Export a batch of parity receipts as JSONL into the shared training-export
/// stream (one `TrainingExportRecord` of kind `ValidatorPolicy` per receipt).
pub fn export_parity_receipts(
    receipts: &[ParityReceipt],
    run_id: &str,
    actor: &str,
    graph_version: u64,
) -> GraphStoreResult<String> {
    let records: GraphStoreResult<Vec<TrainingExportRecord>> = receipts
        .iter()
        .map(|receipt| {
            let run = parity_receipt_to_labeled_run(receipt, run_id, actor, graph_version);
            TrainingExportRecord::from_labeled_run(
                TrainingExportKind::ValidatorPolicy,
                &run,
                RedactionStatus::Safe,
            )
        })
        .collect();
    export_records_jsonl(&records?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::field::FieldType;
    use rustyred_thg_core::InMemoryGraphStore;

    #[test]
    fn receipt_records_pass_or_fail_from_real_normalization() {
        let actual = FieldType::Number.normalize("111").unwrap();
        let pass = ParityReceipt::new("datawave/my-nci.csv", "mycsv", "HEADER_NUMBER", "+cE1.11", actual);
        assert!(pass.passed);

        let fail = ParityReceipt::new("datawave/my-nci.csv", "mycsv", "HEADER_NUMBER", "+cE1.11", "WRONG");
        assert!(!fail.passed);
    }

    #[test]
    fn receipt_persists_as_graph_node() {
        let mut store = InMemoryGraphStore::default();
        let receipt = ParityReceipt::new("datawave/my-nci.csv", "mycsv", "HEADER_ID", "header_one", "header_one");
        write_parity_receipt(&mut store, &receipt, Some("acme")).unwrap();
        let node = store.get_node(&format!("{}:acme", receipt.receipt_id)).unwrap();
        assert!(node.labels.contains(&PARITY_RECEIPT_LABEL.to_string()));
        assert_eq!(node.properties["passed"], serde_json::json!(true));
    }

    #[test]
    fn receipts_export_into_the_training_stream() {
        let receipts = vec![
            ParityReceipt::new("datawave/my-nci.csv", "mycsv", "HEADER_NUMBER", "+cE1.11", FieldType::Number.normalize("111").unwrap()),
            ParityReceipt::new("datawave/my-nci.csv", "mycsv", "HEADER_DATE", "2024-02-29T12:01:47.000Z", FieldType::Date.normalize("2024-02-29 12:01:47").unwrap()),
        ];
        let jsonl = export_parity_receipts(&receipts, "parity-run-1", "datawave-parity", 0).unwrap();
        let lines: Vec<&str> = jsonl.lines().collect();
        assert_eq!(lines.len(), 2);
        // Each line is a TrainingExportRecord carrying the labeled run payload.
        assert!(jsonl.contains("parity_pass"));
        assert!(jsonl.contains("validator_policy"));
        assert!(jsonl.contains("HEADER_NUMBER"));
        // Both checks passed against the asserted DATAWAVE outputs.
        for line in lines {
            let record: serde_json::Value = serde_json::from_str(line).unwrap();
            assert_eq!(record["payload"]["outcome"], serde_json::json!("success"));
        }
    }
}
