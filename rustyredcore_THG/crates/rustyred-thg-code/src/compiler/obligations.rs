use std::collections::BTreeMap;

use rustyred_thg_core::{stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::{
    annotation::CodeCompilerAnnotationRecord,
    features::CodeFeatureRecord,
    ir::{
        CodeSpecDriftFinding, CodeSpecDriftKind, CODE_COMPILER_FEATURE_VERSION,
        CODE_COMPILER_VERSION,
    },
    pattern::CodePatternMemoryRecord,
    process::CodeProcessFlow,
};
use crate::SOURCE;

pub const CODE_IMPLEMENTATION_OBLIGATION_LABEL: &str = "CodeImplementationObligation";
pub const OBLIGATES_CODE_SYMBOL: &str = "OBLIGATES_CODE_SYMBOL";
pub const OBLIGATION_DERIVES_FROM: &str = "OBLIGATION_DERIVES_FROM";

const VALIDATOR_MISSING_SYMBOL: &str = "validator:code:drift-missing-symbol";
const VALIDATOR_UNDOCUMENTED_SYMBOL: &str = "validator:code:drift-undocumented-symbol";
const VALIDATOR_SIGNATURE_CHANGED: &str = "validator:code:drift-signature-changed";
const VALIDATOR_PROCESS_ENTRYPOINT: &str = "validator:code:entrypoint-process";
const VALIDATOR_PROCESS_STEP_COVERAGE: &str = "validator:code:process-step-coverage";
const VALIDATOR_PATTERN_REUSE: &str = "validator:code:pattern-reuse-review";
const VALIDATOR_FEATURE_ANCHOR: &str = "validator:code:feature-evidence";

const UNKNOWN_DERIVED_FROM_FEATURE_GAP: &str = "feature-symbol-mapping-missing";
const UNKNOWN_NO_PATTERN_FILES: &str = "pattern-file-path-missing";
const UNKNOWN_NO_SHARED_SYMBOLS: &str = "symbol-not-found-for-obligation";
const UNKNOWN_DRAINAGE_PENDING: &str = "obligation-target-resolution-incomplete";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeSpecificationSummary {
    pub spec_node_id: Option<String>,
    pub artifact_hash: Option<String>,
    pub tenant_id: String,
    pub repo_id: String,
    pub file_count: usize,
    pub symbol_count: usize,
}

#[derive(Clone, Debug)]
pub struct CodeImplementationObligationInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub code_spec_summary: Option<CodeSpecificationSummary>,
    pub drift_findings: Vec<CodeSpecDriftFinding>,
    pub process_flows: Vec<CodeProcessFlow>,
    pub pattern_memories: Vec<CodePatternMemoryRecord>,
    pub feature_annotations: Vec<CodeCompilerAnnotationRecord>,
    pub feature_records: Vec<CodeFeatureRecord>,
}

impl CodeImplementationObligationInput {
    pub fn new(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            repo_id: repo_id.into(),
            code_spec_summary: None,
            drift_findings: Vec::new(),
            process_flows: Vec::new(),
            pattern_memories: Vec::new(),
            feature_annotations: Vec::new(),
            feature_records: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeImplementationObligation {
    pub tenant_id: String,
    pub repo_id: String,
    pub obligation_id: String,
    pub target_file: Option<String>,
    pub target_symbol_id: Option<String>,
    pub obligation: String,
    pub rationale: String,
    pub evidence_ids: Vec<String>,
    pub suggested_validators: Vec<String>,
    pub risks: Vec<String>,
    pub unknowns: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CodeImplementationObligationOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub obligations: Vec<CodeImplementationObligation>,
    pub artifact_hash: String,
}

pub fn compile_code_implementation_obligations(
    input: CodeImplementationObligationInput,
) -> CodeImplementationObligationOutput {
    let mut feature_records_by_id = BTreeMap::new();
    for feature in &input.feature_records {
        feature_records_by_id.insert(feature.feature_id.clone(), feature.clone());
    }
    let mut obligations = Vec::new();
    let shared_evidence = shared_evidence_ids(&input.code_spec_summary);

    obligations.extend(derive_drift_obligations(
        &input.tenant_id,
        &input.repo_id,
        &input.drift_findings,
        &shared_evidence,
    ));
    obligations.extend(derive_process_obligations(
        &input.tenant_id,
        &input.repo_id,
        &input.process_flows,
        &shared_evidence,
    ));
    obligations.extend(derive_pattern_obligations(
        &input.tenant_id,
        &input.repo_id,
        &input.pattern_memories,
        &shared_evidence,
    ));
    obligations.extend(derive_feature_obligations(
        &input.tenant_id,
        &input.repo_id,
        &input.feature_records,
        &shared_evidence,
    ));
    obligations.extend(derive_annotation_obligations(
        &input.tenant_id,
        &input.repo_id,
        &input.feature_annotations,
        &feature_records_by_id,
        &shared_evidence,
    ));

    normalize_and_finalize(input.tenant_id.clone(), input.repo_id.clone(), obligations)
}

pub fn compile_code_implementation_obligations_in_store<S: GraphStore>(
    store: &mut S,
    input: CodeImplementationObligationInput,
) -> GraphStoreResult<CodeImplementationObligationOutput> {
    let output = compile_code_implementation_obligations(input);
    for obligation in &output.obligations {
        store.upsert_node(obligation_node(&output, obligation))?;
        if let Some(symbol_id) = &obligation.target_symbol_id {
            if store.get_node(symbol_id).is_some() {
                store.upsert_edge(EdgeRecord::new(
                    obligation_edge_id(&obligation.obligation_id, OBLIGATES_CODE_SYMBOL, symbol_id),
                    &obligation.obligation_id,
                    OBLIGATES_CODE_SYMBOL,
                    symbol_id,
                    json!({
                        "tenant_id": &obligation.tenant_id,
                        "repo_id": &obligation.repo_id,
                        "compiler_version": CODE_COMPILER_VERSION,
                        "feature_version": CODE_COMPILER_FEATURE_VERSION,
                        "source": SOURCE,
                    }),
                ))?;
            }
        }
        for evidence_id in &obligation.evidence_ids {
            if store.get_node(evidence_id).is_some() {
                store.upsert_edge(EdgeRecord::new(
                    obligation_evidence_edge_id(&obligation.obligation_id, evidence_id),
                    &obligation.obligation_id,
                    OBLIGATION_DERIVES_FROM,
                    evidence_id,
                    json!({
                        "tenant_id": &obligation.tenant_id,
                        "repo_id": &obligation.repo_id,
                        "compiler_version": CODE_COMPILER_VERSION,
                        "feature_version": CODE_COMPILER_FEATURE_VERSION,
                        "source": SOURCE,
                    }),
                ))?;
            }
        }
    }
    Ok(output)
}

fn normalize_and_finalize(
    tenant_id: String,
    repo_id: String,
    mut obligations: Vec<CodeImplementationObligation>,
) -> CodeImplementationObligationOutput {
    obligations.sort_by(|left, right| left.obligation_id.cmp(&right.obligation_id));
    obligations.dedup_by(|left, right| left.obligation_id == right.obligation_id);

    let artifact_hash = stable_hash(json!({
        "tenant_id": &tenant_id,
        "repo_id": &repo_id,
        "obligations": &obligations,
    }));

    CodeImplementationObligationOutput {
        tenant_id,
        repo_id,
        obligations,
        artifact_hash,
    }
}

fn shared_evidence_ids(summary: &Option<CodeSpecificationSummary>) -> Vec<String> {
    summary
        .as_ref()
        .and_then(|item| item.spec_node_id.clone())
        .into_iter()
        .collect()
}

fn derive_drift_obligations(
    tenant_id: &str,
    repo_id: &str,
    findings: &[CodeSpecDriftFinding],
    shared_evidence: &[String],
) -> Vec<CodeImplementationObligation> {
    findings
        .iter()
        .map(|finding| {
            let mut evidence_ids = vec![finding.finding_id.clone()];
            evidence_ids.extend_from_slice(shared_evidence);
            let (obligation, rationale, suggested_validators, risks, unknowns) =
                match finding.drift_kind {
                    CodeSpecDriftKind::MissingSymbol => (
                        "Reconcile missing symbol from spec".to_string(),
                        format!(
                            "Spec-anchored drift indicates symbol '{}' is missing from code.",
                            finding.symbol_key
                        ),
                        vec![VALIDATOR_MISSING_SYMBOL.to_string()],
                        vec!["high-risk:runtime-api-removal".to_string()],
                        Vec::new(),
                    ),
                    CodeSpecDriftKind::UndocumentedSymbol => (
                        "Address undocumented symbol in active repository code".to_string(),
                        format!(
                            "Active symbol '{}' was not found in spec baseline {}.",
                            finding.symbol_key, finding.file_path
                        ),
                        vec![VALIDATOR_UNDOCUMENTED_SYMBOL.to_string()],
                        vec!["medium-risk:unexpected-public-surface".to_string()],
                        Vec::new(),
                    ),
                    CodeSpecDriftKind::SignatureChanged => (
                        "Resolve symbol signature drift".to_string(),
                        format!(
                            "Signature drift detected for symbol '{}' on {}.",
                            finding.symbol_key, finding.name
                        ),
                        vec![VALIDATOR_SIGNATURE_CHANGED.to_string()],
                        vec!["high-risk:contract-compatibility".to_string()],
                        vec!["signature_verification_required".to_string()],
                    ),
                };

            CodeImplementationObligation {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                obligation_id: obligation_id(
                    tenant_id,
                    repo_id,
                    &evidence_ids,
                    &obligation,
                    &finding.symbol_key,
                ),
                target_file: Some(finding.file_path.clone()),
                target_symbol_id: finding.symbol_id.clone(),
                obligation,
                rationale,
                evidence_ids: normalize_strings(evidence_ids),
                suggested_validators: normalize_strings(suggested_validators),
                risks: normalize_strings(risks),
                unknowns: normalize_strings(unknowns),
            }
        })
        .collect()
}

fn derive_process_obligations(
    tenant_id: &str,
    repo_id: &str,
    flows: &[CodeProcessFlow],
    shared_evidence: &[String],
) -> Vec<CodeImplementationObligation> {
    flows
        .iter()
        .map(|flow| {
            let mut evidence_ids = Vec::from([flow.process_id.clone()]);
            evidence_ids.extend_from_slice(shared_evidence);
            evidence_ids.extend(flow.steps.iter().map(|step| step.symbol_id.clone()));
            let (obligation, rationale, validators, risks) = if flow.confidence >= 0.9 {
                (
                    "Validate validated entrypoint contract path".to_string(),
                    format!(
                        "Entrypoint flow '{}' should be revalidated end-to-end.",
                        flow.entry_name
                    ),
                    vec![
                        VALIDATOR_PROCESS_ENTRYPOINT.to_string(),
                        VALIDATOR_PROCESS_STEP_COVERAGE.to_string(),
                    ],
                    vec!["medium-risk:runtime-path-assumptions".to_string()],
                )
            } else {
                (
                    "Validate process flow coverage".to_string(),
                    format!(
                        "Entrypoint '{}' includes {} steps and should be revalidated.",
                        flow.entry_name,
                        flow.steps.len()
                    ),
                    vec![VALIDATOR_PROCESS_STEP_COVERAGE.to_string()],
                    vec!["medium-risk:trigger-path-uncertainty".to_string()],
                )
            };
            CodeImplementationObligation {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                obligation_id: obligation_id(
                    tenant_id,
                    repo_id,
                    &evidence_ids,
                    &obligation,
                    &flow.entry_symbol_id,
                ),
                target_file: Some(flow.entry_file_path.clone()),
                target_symbol_id: Some(flow.entry_symbol_id.clone()),
                obligation,
                rationale,
                evidence_ids: normalize_strings(evidence_ids),
                suggested_validators: normalize_strings(validators),
                risks: normalize_strings(risks),
                unknowns: normalize_strings(vec![UNKNOWN_DRAINAGE_PENDING.to_string()]),
            }
        })
        .collect()
}

fn derive_pattern_obligations(
    tenant_id: &str,
    repo_id: &str,
    patterns: &[CodePatternMemoryRecord],
    shared_evidence: &[String],
) -> Vec<CodeImplementationObligation> {
    patterns
        .iter()
        .map(|pattern| {
            let mut evidence_ids = vec![pattern.pattern_id.clone()];
            evidence_ids.extend_from_slice(shared_evidence);
            let target_symbol_id = pattern.symbol_ids.first().cloned();
            let (target_file, unknowns) = match pattern.file_paths.first() {
                Some(path) => (Some(path.clone()), Vec::new()),
                None => (None, vec![UNKNOWN_NO_PATTERN_FILES.to_string()]),
            };

            let unknowns = if target_symbol_id.is_none() {
                append_unknowns(unknowns, UNKNOWN_NO_SHARED_SYMBOLS.to_string())
            } else {
                unknowns
            };

            CodeImplementationObligation {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                obligation_id: obligation_id(
                    tenant_id,
                    repo_id,
                    &evidence_ids,
                    &pattern.title,
                    target_symbol_id.as_deref().unwrap_or("pattern"),
                ),
                target_file,
                target_symbol_id,
                obligation: pattern.title.clone(),
                rationale: format!(
                    "Pattern memory '{}' should be checked before related refactors.",
                    pattern.root_cause
                ),
                evidence_ids: normalize_strings(evidence_ids),
                suggested_validators: normalize_strings(vec![VALIDATOR_PATTERN_REUSE.to_string()]),
                risks: normalize_strings(vec!["medium-risk:historical-fix-dependency".to_string()]),
                unknowns: normalize_strings(unknowns),
            }
        })
        .collect()
}

fn derive_feature_obligations(
    tenant_id: &str,
    repo_id: &str,
    features: &[CodeFeatureRecord],
    shared_evidence: &[String],
) -> Vec<CodeImplementationObligation> {
    let Some(first) = features.first() else {
        return Vec::new();
    };
    let mut evidence_ids = features
        .iter()
        .take(32)
        .map(|feature| feature.feature_id.clone())
        .collect::<Vec<_>>();
    evidence_ids.extend_from_slice(shared_evidence);
    vec![CodeImplementationObligation {
        tenant_id: tenant_id.to_string(),
        repo_id: repo_id.to_string(),
        obligation_id: obligation_id(
            tenant_id,
            repo_id,
            &evidence_ids,
            "Validate compiler feature-evidence anchors",
            &first.source_symbol_id,
        ),
        target_file: None,
        target_symbol_id: Some(first.source_symbol_id.clone()),
        obligation: "Validate compiler feature-evidence anchors".to_string(),
        rationale: format!(
            "{} compiler feature records connect source and target code symbols; validate the selected feature anchors before treating them as portable behavior.",
            features.len()
        ),
        evidence_ids: normalize_strings(evidence_ids),
        suggested_validators: normalize_strings(vec![VALIDATOR_FEATURE_ANCHOR.to_string()]),
        risks: normalize_strings(vec!["medium-risk:compiler-feature-evidence".to_string()]),
        unknowns: Vec::new(),
    }]
}

fn derive_annotation_obligations(
    tenant_id: &str,
    repo_id: &str,
    annotations: &[CodeCompilerAnnotationRecord],
    feature_records_by_id: &BTreeMap<String, CodeFeatureRecord>,
    shared_evidence: &[String],
) -> Vec<CodeImplementationObligation> {
    annotations
        .iter()
        .map(|annotation| {
            let mut evidence_ids = vec![annotation.annotation_id.clone()];
            if let Some(feature) = feature_records_by_id.get(&annotation.feature_id) {
                evidence_ids.push(feature.feature_id.clone());
            }
            evidence_ids.extend_from_slice(shared_evidence);
            let (risk, validator, unknowns) = if annotation.evidence_count < 3.0 {
                (
                    vec!["low-confidence:annotation-evidence".to_string()],
                    vec![VALIDATOR_FEATURE_ANCHOR.to_string()],
                    vec![UNKNOWN_DERIVED_FROM_FEATURE_GAP.to_string()],
                )
            } else {
                (
                    vec!["medium-confidence:annotation-evidence".to_string()],
                    vec![VALIDATOR_FEATURE_ANCHOR.to_string()],
                    Vec::new(),
                )
            };

            let target_symbol_id =
                feature_records_by_id
                    .get(&annotation.feature_id)
                    .and_then(|feature| {
                        if feature.source_symbol_id.is_empty() {
                            None
                        } else {
                            Some(feature.source_symbol_id.clone())
                        }
                    });

            CodeImplementationObligation {
                tenant_id: tenant_id.to_string(),
                repo_id: repo_id.to_string(),
                obligation_id: obligation_id(
                    tenant_id,
                    repo_id,
                    &evidence_ids,
                    "Validate annotation-backed change risk",
                    annotation.feature_id.as_str(),
                ),
                target_file: None,
                target_symbol_id,
                obligation: "Validate annotation-backed change risk".to_string(),
                rationale: annotation.explanation.clone(),
                evidence_ids: normalize_strings(evidence_ids),
                suggested_validators: normalize_strings(validator),
                risks: normalize_strings(risk),
                unknowns: normalize_strings(unknowns),
            }
        })
        .collect()
}

fn normalize_strings(items: Vec<String>) -> Vec<String> {
    let mut cleaned = items
        .into_iter()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    cleaned.sort();
    cleaned.dedup();
    cleaned
}

fn append_unknowns(mut left: Vec<String>, item: String) -> Vec<String> {
    left.push(item);
    normalize_strings(left)
}

fn obligation_id(
    tenant_id: &str,
    repo_id: &str,
    evidence_ids: &[String],
    obligation: &str,
    anchor: &str,
) -> String {
    format!(
        "code:implementation-obligation:{}",
        stable_hash(json!([
            tenant_id,
            repo_id,
            obligation,
            anchor,
            evidence_ids
        ]))
    )
}

fn obligation_node(
    output: &CodeImplementationObligationOutput,
    record: &CodeImplementationObligation,
) -> NodeRecord {
    NodeRecord::new(
        &record.obligation_id,
        [CODE_IMPLEMENTATION_OBLIGATION_LABEL],
        json!({
            "tenant_id": &record.tenant_id,
            "repo_id": &record.repo_id,
            "obligation": &record.obligation,
            "rationale": &record.rationale,
            "target_file": &record.target_file,
            "target_symbol_id": &record.target_symbol_id,
            "evidence_ids": &record.evidence_ids,
            "suggested_validators": &record.suggested_validators,
            "risks": &record.risks,
            "unknowns": &record.unknowns,
            "artifact_hash": &output.artifact_hash,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn obligation_edge_id(from: &str, edge_type: &str, to: &str) -> String {
    format!(
        "code:edge:obligation:{}",
        stable_hash(json!([from, edge_type, to]))
    )
}

fn obligation_evidence_edge_id(from: &str, evidence_id: &str) -> String {
    obligation_edge_id(from, OBLIGATION_DERIVES_FROM, evidence_id)
}

#[cfg(test)]
mod tests {
    use super::super::ir::CodeSpecDriftKind;
    use super::*;
    use rustyred_thg_core::{InMemoryGraphStore, NodeQuery};
    use serde_json::json;

    #[test]
    fn code_obligations_emit_for_missing_and_signature_drift() {
        let findings = vec![
            make_finding(
                "sym:find_missing",
                CodeSpecDriftKind::MissingSymbol,
                "src/lib.rs",
                "old_fn",
                "function",
                "pub fn old_fn()",
                None,
            ),
            make_finding(
                "sym:find_signature",
                CodeSpecDriftKind::SignatureChanged,
                "src/lib.rs",
                "compile",
                "function",
                "pub fn compile(req: &Request)",
                Some("sig:compile:new".to_string()),
            ),
        ];
        let mut input = CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler");
        input.drift_findings = findings.clone();
        input.code_spec_summary = Some(CodeSpecificationSummary {
            spec_node_id: Some("spec:compiled".to_string()),
            artifact_hash: Some("abc".to_string()),
            tenant_id: "Travis-Gilbert".to_string(),
            repo_id: "repo:compiler".to_string(),
            file_count: 1,
            symbol_count: 2,
        });
        let output = compile_code_implementation_obligations(input);

        let missing = output
            .obligations
            .iter()
            .find(|o| o.obligation == "Reconcile missing symbol from spec")
            .expect("missing drift obligation present");
        assert!(!missing.evidence_ids.is_empty());
        assert!(missing.evidence_ids.contains(&findings[0].finding_id));
        assert!(missing
            .suggested_validators
            .contains(&VALIDATOR_MISSING_SYMBOL.to_string()));

        let signature = output
            .obligations
            .iter()
            .find(|o| o.obligation == "Resolve symbol signature drift")
            .expect("signature drift obligation present");
        assert!(!signature.evidence_ids.is_empty());
        assert!(signature.evidence_ids.contains(&findings[1].finding_id));
        assert!(signature
            .suggested_validators
            .contains(&VALIDATOR_SIGNATURE_CHANGED.to_string()));
    }

    #[test]
    fn code_obligation_includes_pattern_evidence() {
        let pattern = super::super::pattern::CodePatternMemoryRecord {
            pattern_id: "pattern:reuse".to_string(),
            tenant_id: "Travis-Gilbert".to_string(),
            repo_id: "repo:compiler".to_string(),
            title: "Prefer parser-backed edit".to_string(),
            feedback: String::new(),
            root_cause: "Symbol move drift".to_string(),
            fix_summary: "Use parser-backed extraction".to_string(),
            symbol_ids: vec!["sym:compile".to_string()],
            file_paths: vec!["src/lib.rs".to_string()],
            confidence: 0.9,
            created_at_ms: 1_700_000_000_000,
            source_event_id: None,
        };
        let mut input = CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler");
        input.pattern_memories.push(pattern.clone());

        let output = compile_code_implementation_obligations(input);
        assert_eq!(output.obligations.len(), 1);
        let obligation = &output.obligations[0];
        assert!(obligation.evidence_ids.contains(&pattern.pattern_id));
        assert_eq!(obligation.target_symbol_id.as_deref(), Some("sym:compile"));
        assert_eq!(obligation.target_file.as_deref(), Some("src/lib.rs"));
    }

    #[test]
    fn code_obligation_includes_feature_evidence_anchor() {
        let feature = CodeFeatureRecord {
            feature_id: "feature:compile-model".to_string(),
            source_symbol_id: "sym:compile".to_string(),
            target_symbol_id: "sym:model".to_string(),
            feature_version: CODE_COMPILER_FEATURE_VERSION.to_string(),
            model_id: None,
            features: super::super::features::CodeConnectionFeatureVector::default(),
            provenance: json!({"kind": "test"}),
        };
        let mut input = CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler");
        input.feature_records.push(feature.clone());

        let output = compile_code_implementation_obligations(input);
        assert_eq!(output.obligations.len(), 1);
        let obligation = &output.obligations[0];
        assert_eq!(
            obligation.obligation,
            "Validate compiler feature-evidence anchors"
        );
        assert_eq!(obligation.target_symbol_id.as_deref(), Some("sym:compile"));
        assert!(obligation.evidence_ids.contains(&feature.feature_id));
        assert!(obligation
            .suggested_validators
            .contains(&VALIDATOR_FEATURE_ANCHOR.to_string()));
    }

    #[test]
    fn code_obligation_preserves_tenant_repo_metadata() {
        let mut store = InMemoryGraphStore::new();
        let mut input = CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler");
        input.drift_findings.push(make_finding(
            "sym:drift",
            CodeSpecDriftKind::UndocumentedSymbol,
            "src/lib.rs",
            "compile",
            "function",
            "pub fn compile()",
            Some("sym:compile".to_string()),
        ));
        let output = compile_code_implementation_obligations_in_store(&mut store, input)
            .expect("obligations should write to store");

        assert_eq!(output.tenant_id, "Travis-Gilbert");
        assert_eq!(output.repo_id, "repo:compiler");

        let obligations = store.query_nodes(NodeQuery::label(CODE_IMPLEMENTATION_OBLIGATION_LABEL));
        assert_eq!(obligations.len(), 1);
        let node = &obligations[0];
        assert!(node
            .labels
            .contains(&CODE_IMPLEMENTATION_OBLIGATION_LABEL.to_string()));
        assert_eq!(
            node.properties
                .get("tenant_id")
                .and_then(|value| value.as_str()),
            Some("Travis-Gilbert")
        );
        assert_eq!(
            node.properties
                .get("repo_id")
                .and_then(|value| value.as_str()),
            Some("repo:compiler")
        );
    }

    #[test]
    fn code_obligation_no_input_artifacts_emit_none() {
        let mut store = InMemoryGraphStore::new();
        let input = CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler");
        let output = compile_code_implementation_obligations(
            CodeImplementationObligationInput::new("Travis-Gilbert", "repo:compiler"),
        );
        assert_eq!(output.obligations.len(), 0);
        assert_eq!(output.tenant_id, input.tenant_id);
        assert_eq!(output.repo_id, input.repo_id);

        let store_output =
            compile_code_implementation_obligations_in_store(&mut store, input).unwrap();
        assert_eq!(store_output.obligations.len(), 0);
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(CODE_IMPLEMENTATION_OBLIGATION_LABEL))
                .len(),
            0
        );
    }

    fn make_finding(
        finding_id: &str,
        drift_kind: CodeSpecDriftKind,
        file_path: &str,
        name: &str,
        kind: &str,
        expected: &str,
        symbol_id: Option<String>,
    ) -> CodeSpecDriftFinding {
        CodeSpecDriftFinding {
            finding_id: finding_id.to_string(),
            drift_kind,
            severity: "medium".to_string(),
            symbol_key: format!("{}\u{0}{}\u{0}{}", file_path, kind, name),
            symbol_id,
            name: name.to_string(),
            kind: kind.to_string(),
            file_path: file_path.to_string(),
            expected: json!({"signature": expected}),
            actual: json!({}),
            suggested_next_step: "Re-run compiler".to_string(),
        }
    }
}
