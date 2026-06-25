use rustyred_thg_core::{GraphStore, GraphStoreResult, NodeQuery, NodeRecord};
use serde_json::{json, Value};

use super::annotation::count_annotations;
use super::code_to_spec::compile_code_spec_in_store;
use super::drift::detect_code_spec_drift_in_store;
use super::features::count_features;
use super::ir::{
    CodeSpecCompileInput, CodeSpecDriftFinding, CodeSpecDriftInput, CodeSpecDriftReport,
    CODE_COMPILER_VERSION, CODE_SPEC_LABEL,
};
use super::pattern::{count_patterns, relevant_code_patterns, CodePatternMemoryRecord};
use super::process::{count_processes, detect_code_processes_in_store, CodeProcessDetectInput};
use crate::{property_string, property_u64, query_terms};

pub const DEFAULT_AMBIENT_COMPILER_FINDING_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub struct AmbientCompilerReadout {
    pub spec_node_id: String,
    pub artifact_hash: Option<String>,
    pub compiler_version: String,
    pub file_count: u64,
    pub symbol_count: u64,
    pub structure_count: u64,
    pub process_count: usize,
    pub pattern_count: usize,
    pub feature_count: usize,
    pub annotation_count: usize,
    pub drift_findings: Vec<CodeSpecDriftFinding>,
    pub pattern_memories: Vec<CodePatternMemoryRecord>,
    pub bootstrapped_spec: bool,
}

impl AmbientCompilerReadout {
    pub fn drift_count(&self) -> usize {
        self.drift_findings.len()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "spec_node_id": self.spec_node_id,
            "artifact_hash": self.artifact_hash,
            "compiler_version": self.compiler_version,
            "file_count": self.file_count,
            "symbol_count": self.symbol_count,
            "structure_count": self.structure_count,
            "process_count": self.process_count,
            "pattern_count": self.pattern_count,
            "feature_count": self.feature_count,
            "annotation_count": self.annotation_count,
            "drift_count": self.drift_findings.len(),
            "drift_findings": self.drift_findings,
            "pattern_memories": self.pattern_memories,
            "bootstrapped_spec": self.bootstrapped_spec,
        })
    }
}

pub fn compiler_ambient_readout_in_store<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
    query: &str,
    path_prefix: &str,
    max_findings: usize,
) -> GraphStoreResult<AmbientCompilerReadout> {
    let Some(spec_node) = latest_code_spec_node(store, tenant_id, repo_id) else {
        let output =
            compile_code_spec_in_store(store, CodeSpecCompileInput::new(tenant_id, repo_id))?;
        let supplements = ambient_supplements(store, tenant_id, repo_id, query, path_prefix)?;
        return Ok(readout_from_spec(
            output.spec_node,
            Vec::new(),
            supplements,
            true,
        ));
    };

    let report = detect_code_spec_drift_in_store(
        store,
        CodeSpecDriftInput::new(tenant_id, repo_id, spec_node.id.clone()),
    )?;
    let supplements = ambient_supplements(store, tenant_id, repo_id, query, path_prefix)?;
    Ok(readout_from_spec(
        spec_node,
        relevant_findings(report, query, path_prefix, max_findings),
        supplements,
        false,
    ))
}

pub fn refresh_code_compiler_artifacts_for_repo<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
) -> GraphStoreResult<AmbientCompilerReadout> {
    compiler_ambient_readout_in_store(
        store,
        tenant_id,
        repo_id,
        "",
        "",
        DEFAULT_AMBIENT_COMPILER_FINDING_LIMIT,
    )
}

pub(super) fn latest_code_spec_node<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    repo_id: &str,
) -> Option<NodeRecord> {
    let mut specs = store
        .query_nodes(
            NodeQuery::label(CODE_SPEC_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(100_000),
        )
        .into_iter()
        .filter(|node| !node.tombstone)
        .collect::<Vec<_>>();
    specs.sort_by(|left, right| {
        property_u64(&right.properties, "compiled_at_ms")
            .cmp(&property_u64(&left.properties, "compiled_at_ms"))
            .then_with(|| right.id.cmp(&left.id))
    });
    specs.into_iter().next()
}

fn readout_from_spec(
    spec_node: NodeRecord,
    drift_findings: Vec<CodeSpecDriftFinding>,
    supplements: AmbientSupplements,
    bootstrapped_spec: bool,
) -> AmbientCompilerReadout {
    AmbientCompilerReadout {
        spec_node_id: spec_node.id,
        artifact_hash: property_string(&spec_node.properties, "artifact_hash"),
        compiler_version: property_string(&spec_node.properties, "compiler_version")
            .unwrap_or_else(|| CODE_COMPILER_VERSION.to_string()),
        file_count: property_u64(&spec_node.properties, "file_count").unwrap_or(0),
        symbol_count: property_u64(&spec_node.properties, "symbol_count").unwrap_or(0),
        structure_count: property_u64(&spec_node.properties, "structure_count").unwrap_or(0),
        process_count: supplements.process_count,
        pattern_count: supplements.pattern_count,
        feature_count: supplements.feature_count,
        annotation_count: supplements.annotation_count,
        drift_findings,
        pattern_memories: supplements.pattern_memories,
        bootstrapped_spec,
    }
}

struct AmbientSupplements {
    process_count: usize,
    pattern_count: usize,
    feature_count: usize,
    annotation_count: usize,
    pattern_memories: Vec<CodePatternMemoryRecord>,
}

fn ambient_supplements<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
    query: &str,
    path_prefix: &str,
) -> GraphStoreResult<AmbientSupplements> {
    let detected = detect_code_processes_in_store(
        store,
        CodeProcessDetectInput::new(tenant_id.to_string(), repo_id.to_string()),
    )?;
    let process_count = detected
        .processes
        .len()
        .max(count_processes(store, tenant_id, repo_id));
    let pattern_memories = relevant_code_patterns(store, tenant_id, repo_id, query, path_prefix, 5);
    Ok(AmbientSupplements {
        process_count,
        pattern_count: count_patterns(store, tenant_id, repo_id),
        feature_count: count_features(store, tenant_id, repo_id),
        annotation_count: count_annotations(store, tenant_id, repo_id),
        pattern_memories,
    })
}

fn relevant_findings(
    report: CodeSpecDriftReport,
    query: &str,
    path_prefix: &str,
    max_findings: usize,
) -> Vec<CodeSpecDriftFinding> {
    let limit = if max_findings == 0 {
        DEFAULT_AMBIENT_COMPILER_FINDING_LIMIT
    } else {
        max_findings
    };
    let terms = query_terms(query);
    let prefix = path_prefix.trim();
    let mut relevant = report
        .findings
        .iter()
        .filter(|finding| prefix.is_empty() || finding.file_path.starts_with(prefix))
        .filter(|finding| terms.is_empty() || finding_matches_terms(finding, &terms))
        .cloned()
        .collect::<Vec<_>>();

    if relevant.is_empty() && (!terms.is_empty() || !prefix.is_empty()) {
        relevant = report.findings;
    }

    relevant.truncate(limit);
    relevant
}

fn finding_matches_terms(finding: &CodeSpecDriftFinding, terms: &[String]) -> bool {
    let haystack = format!(
        "{} {} {} {} {}",
        finding.name,
        finding.kind,
        finding.file_path,
        finding.drift_kind.as_str(),
        finding.suggested_next_step
    )
    .to_ascii_lowercase();
    terms.iter().any(|term| haystack.contains(term))
}
