use std::collections::BTreeMap;

use rustyred_thg_core::{
    stable_hash, EdgeRecord, GraphStore, GraphStoreError, GraphStoreResult, NodeRecord,
};
use serde_json::{json, Value};

use super::code_to_spec::collect_code_symbols;
use super::ir::{
    CodeSpecDriftFinding, CodeSpecDriftInput, CodeSpecDriftKind, CodeSpecDriftReport,
    CodeSymbolSnapshot, CODE_COMPILER_DRIFT_LABEL, CODE_COMPILER_FEATURE_VERSION,
    CODE_COMPILER_VERSION, DRIFT_FOR_CODE, DRIFT_FOR_SPEC,
};
use crate::SOURCE;

pub fn detect_code_spec_drift<S: GraphStore>(
    store: &S,
    input: &CodeSpecDriftInput,
) -> GraphStoreResult<CodeSpecDriftReport> {
    let spec_node = store
        .get_node(&input.spec_node_id)
        .ok_or_else(|| {
            GraphStoreError::new(
                "code_spec_missing",
                format!("code spec node '{}' was not found", input.spec_node_id),
            )
        })?
        .clone();
    let expected_symbols = compiled_symbols_from_spec(&spec_node)?;
    let current_symbols = collect_code_symbols(
        store,
        &input.tenant_id,
        &input.repo_id,
        input.symbol_limit(),
    );
    let expected_by_key = symbols_by_key(&expected_symbols);
    let current_by_key = symbols_by_key(&current_symbols);
    let mut findings = Vec::new();

    for (key, expected) in &expected_by_key {
        match current_by_key.get(key) {
            Some(actual) => {
                if expected.signature != actual.signature {
                    findings.push(signature_changed(input, expected, actual));
                }
            }
            None => findings.push(missing_symbol(input, expected)),
        }
    }

    for (key, actual) in &current_by_key {
        if !expected_by_key.contains_key(key) {
            findings.push(undocumented_symbol(input, actual));
        }
    }

    findings.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.kind.cmp(&right.kind))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.drift_kind.as_str().cmp(right.drift_kind.as_str()))
    });

    Ok(CodeSpecDriftReport {
        tenant_id: input.tenant_id.clone(),
        repo_id: input.repo_id.clone(),
        spec_node_id: input.spec_node_id.clone(),
        findings,
    })
}

pub fn detect_code_spec_drift_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeSpecDriftInput,
) -> GraphStoreResult<CodeSpecDriftReport> {
    let report = detect_code_spec_drift(store, &input)?;
    for finding in &report.findings {
        store.upsert_node(drift_finding_node(&report, finding))?;
        store.upsert_edge(EdgeRecord::new(
            drift_edge_id(&finding.finding_id, DRIFT_FOR_SPEC, &report.spec_node_id),
            &finding.finding_id,
            DRIFT_FOR_SPEC,
            &report.spec_node_id,
            json!({
                "tenant_id": &report.tenant_id,
                "repo_id": &report.repo_id,
                "compiler_version": CODE_COMPILER_VERSION,
                "feature_version": CODE_COMPILER_FEATURE_VERSION,
                "source": SOURCE,
            }),
        ))?;
        if let Some(symbol_id) = &finding.symbol_id {
            store.upsert_edge(EdgeRecord::new(
                drift_edge_id(&finding.finding_id, DRIFT_FOR_CODE, symbol_id),
                &finding.finding_id,
                DRIFT_FOR_CODE,
                symbol_id,
                json!({
                    "tenant_id": &report.tenant_id,
                    "repo_id": &report.repo_id,
                    "compiler_version": CODE_COMPILER_VERSION,
                    "feature_version": CODE_COMPILER_FEATURE_VERSION,
                    "source": SOURCE,
                }),
            ))?;
        }
    }
    Ok(report)
}

fn compiled_symbols_from_spec(spec_node: &NodeRecord) -> GraphStoreResult<Vec<CodeSymbolSnapshot>> {
    let value = spec_node
        .properties
        .get("compiled_symbols")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()));
    serde_json::from_value(value).map_err(|error| {
        GraphStoreError::new(
            "invalid_compiled_symbols",
            format!(
                "code spec node '{}' has invalid compiled_symbols: {error}",
                spec_node.id
            ),
        )
    })
}

fn symbols_by_key(symbols: &[CodeSymbolSnapshot]) -> BTreeMap<String, CodeSymbolSnapshot> {
    symbols
        .iter()
        .cloned()
        .map(|symbol| (symbol.symbol_key(), symbol))
        .collect()
}

fn missing_symbol(
    input: &CodeSpecDriftInput,
    expected: &CodeSymbolSnapshot,
) -> CodeSpecDriftFinding {
    drift_finding(
        input,
        CodeSpecDriftKind::MissingSymbol,
        "high",
        expected,
        None,
        json!(expected),
        Value::Null,
        "Restore the symbol or recompile the spec after an intentional removal.",
    )
}

fn undocumented_symbol(
    input: &CodeSpecDriftInput,
    actual: &CodeSymbolSnapshot,
) -> CodeSpecDriftFinding {
    drift_finding(
        input,
        CodeSpecDriftKind::UndocumentedSymbol,
        "medium",
        actual,
        Some(actual),
        Value::Null,
        json!(actual),
        "Update the compiled spec to cover the new symbol, or remove the stray code.",
    )
}

fn signature_changed(
    input: &CodeSpecDriftInput,
    expected: &CodeSymbolSnapshot,
    actual: &CodeSymbolSnapshot,
) -> CodeSpecDriftFinding {
    drift_finding(
        input,
        CodeSpecDriftKind::SignatureChanged,
        "medium",
        expected,
        Some(actual),
        json!({ "signature": expected.signature }),
        json!({ "signature": actual.signature }),
        "Review the API change and update the compiled spec if the change is intentional.",
    )
}

fn drift_finding(
    input: &CodeSpecDriftInput,
    drift_kind: CodeSpecDriftKind,
    severity: &str,
    anchor: &CodeSymbolSnapshot,
    actual: Option<&CodeSymbolSnapshot>,
    expected: Value,
    actual_value: Value,
    suggested_next_step: &str,
) -> CodeSpecDriftFinding {
    let symbol_id = actual.map(|symbol| symbol.symbol_id.clone());
    let symbol_key = anchor.symbol_key();
    let finding_id = format!(
        "code:drift:{}",
        stable_hash(json!([
            &input.spec_node_id,
            drift_kind.as_str(),
            &symbol_key,
            &expected,
            &actual_value
        ]))
    );
    CodeSpecDriftFinding {
        finding_id,
        drift_kind,
        severity: severity.to_string(),
        symbol_key,
        symbol_id,
        name: anchor.name.clone(),
        kind: anchor.kind.clone(),
        file_path: anchor.file_path.clone(),
        expected,
        actual: actual_value,
        suggested_next_step: suggested_next_step.to_string(),
    }
}

fn drift_finding_node(report: &CodeSpecDriftReport, finding: &CodeSpecDriftFinding) -> NodeRecord {
    NodeRecord::new(
        &finding.finding_id,
        [CODE_COMPILER_DRIFT_LABEL],
        json!({
            "tenant_id": &report.tenant_id,
            "repo_id": &report.repo_id,
            "spec_node_id": &report.spec_node_id,
            "drift_kind": finding.drift_kind.as_str(),
            "severity": &finding.severity,
            "symbol_key": &finding.symbol_key,
            "symbol_id": &finding.symbol_id,
            "name": &finding.name,
            "kind": &finding.kind,
            "file_path": &finding.file_path,
            "expected": &finding.expected,
            "actual": &finding.actual,
            "suggested_next_step": &finding.suggested_next_step,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn drift_edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!(
        "code:edge:drift:{}",
        stable_hash(json!([from, edge_type, to]))
    )
}
