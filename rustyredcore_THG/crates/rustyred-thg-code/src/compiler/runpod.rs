use rustyred_thg_core::{
    now_ms, stable_hash, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::annotation::{write_code_annotation_record, CodeCompilerAnnotationRecord};
use super::code_to_spec::collect_code_symbols;
use super::features::{write_code_feature_record, CodeFeatureRecord};
use super::ir::{
    CodeDependencySnapshot, CodeSymbolSnapshot, CODE_COMPILER_FEATURE_VERSION,
    CODE_COMPILER_VERSION,
};
use super::pattern::{write_code_pattern_record, CodePatternMemoryRecord};
use super::process::CodeProcessFlow;
use crate::{CALLS_SYMBOL, DEPENDS_ON_SYMBOL, SOURCE};

pub const CODE_BURST_JOB_LABEL: &str = "CodeCompilerBurstJob";
pub const CODE_BURST_ARTIFACT_LABEL: &str = "CodeCompilerBurstArtifact";
pub const BURST_PRODUCED_ARTIFACT: &str = "BURST_PRODUCED_ARTIFACT";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeRunPodBurstRequest {
    pub tenant_id: String,
    pub repo_id: String,
    pub job_id: String,
    pub repo_ref: Option<String>,
    pub feature_version: String,
    pub requested_artifacts: Vec<String>,
    #[serde(default)]
    pub symbols: Vec<CodeSymbolSnapshot>,
    #[serde(default)]
    pub dependency_edges: Vec<CodeDependencySnapshot>,
    pub max_pairs: usize,
    pub model_id: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeRunPodBurstResponse {
    pub tenant_id: String,
    pub repo_id: String,
    pub job_id: String,
    pub worker_id: Option<String>,
    pub model_id: Option<String>,
    pub processes: Vec<CodeProcessFlow>,
    pub patterns: Vec<CodePatternMemoryRecord>,
    pub features: Vec<CodeFeatureRecord>,
    pub annotations: Vec<CodeCompilerAnnotationRecord>,
    pub artifacts: Vec<CodeRunPodArtifact>,
    pub completed_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CodeRunPodArtifact {
    pub artifact_id: String,
    pub artifact_kind: String,
    pub payload: Value,
    pub provenance: Value,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodeRunPodImportReport {
    pub job_id: String,
    pub process_count: usize,
    pub pattern_count: usize,
    pub feature_count: usize,
    pub annotation_count: usize,
    pub artifact_count: usize,
}

pub fn build_runpod_burst_request(
    tenant_id: impl Into<String>,
    repo_id: impl Into<String>,
    repo_ref: Option<String>,
) -> CodeRunPodBurstRequest {
    let tenant_id = tenant_id.into();
    let repo_id = repo_id.into();
    let created_at_ms = now_ms() as u64;
    let job_id = format!(
        "code:burst:{}",
        stable_hash(json!([&tenant_id, &repo_id, &repo_ref, created_at_ms]))
    );
    CodeRunPodBurstRequest {
        tenant_id,
        repo_id,
        job_id,
        repo_ref,
        feature_version: CODE_COMPILER_FEATURE_VERSION.to_string(),
        requested_artifacts: vec![
            "processes".to_string(),
            "patterns".to_string(),
            "features".to_string(),
            "annotations".to_string(),
        ],
        symbols: Vec::new(),
        dependency_edges: Vec::new(),
        max_pairs: 512,
        model_id: None,
        created_at_ms,
    }
}

pub fn build_runpod_burst_request_from_store<S: GraphStore>(
    store: &S,
    tenant_id: impl Into<String>,
    repo_id: impl Into<String>,
    repo_ref: Option<String>,
) -> CodeRunPodBurstRequest {
    let tenant_id = tenant_id.into();
    let repo_id = repo_id.into();
    let symbols = collect_code_symbols(store, &tenant_id, &repo_id, 100_000);
    let dependency_edges = collect_runpod_dependencies(store, &symbols);
    let mut request = build_runpod_burst_request(tenant_id, repo_id, repo_ref);
    request.symbols = symbols;
    request.dependency_edges = dependency_edges;
    request
}

pub fn import_runpod_burst_response_in_store<S: GraphStore>(
    store: &mut S,
    response: CodeRunPodBurstResponse,
) -> GraphStoreResult<CodeRunPodImportReport> {
    store.upsert_node(job_node(&response))?;

    for process in &response.processes {
        store.upsert_node(process_node(&response, process))?;
        link_job_artifact(store, &response, &process.process_id, "process")?;
    }
    for pattern in &response.patterns {
        write_code_pattern_record(store, pattern)?;
        link_job_artifact(store, &response, &pattern.pattern_id, "pattern")?;
    }
    for feature in &response.features {
        write_code_feature_record(store, &response.tenant_id, &response.repo_id, feature)?;
        link_job_artifact(store, &response, &feature.feature_id, "feature")?;
    }
    for annotation in &response.annotations {
        write_code_annotation_record(store, &response.tenant_id, &response.repo_id, annotation)?;
        link_job_artifact(store, &response, &annotation.annotation_id, "annotation")?;
    }
    for artifact in &response.artifacts {
        store.upsert_node(generic_artifact_node(&response, artifact))?;
        link_job_artifact(
            store,
            &response,
            &artifact.artifact_id,
            &artifact.artifact_kind,
        )?;
    }

    Ok(CodeRunPodImportReport {
        job_id: response.job_id,
        process_count: response.processes.len(),
        pattern_count: response.patterns.len(),
        feature_count: response.features.len(),
        annotation_count: response.annotations.len(),
        artifact_count: response.artifacts.len(),
    })
}

fn job_node(response: &CodeRunPodBurstResponse) -> NodeRecord {
    NodeRecord::new(
        &response.job_id,
        [CODE_BURST_JOB_LABEL],
        json!({
            "tenant_id": &response.tenant_id,
            "repo_id": &response.repo_id,
            "job_id": &response.job_id,
            "worker_id": &response.worker_id,
            "model_id": &response.model_id,
            "process_count": response.processes.len(),
            "pattern_count": response.patterns.len(),
            "feature_count": response.features.len(),
            "annotation_count": response.annotations.len(),
            "artifact_count": response.artifacts.len(),
            "completed_at_ms": response.completed_at_ms,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn process_node(response: &CodeRunPodBurstResponse, process: &CodeProcessFlow) -> NodeRecord {
    NodeRecord::new(
        &process.process_id,
        [super::process::CODE_PROCESS_LABEL],
        json!({
            "tenant_id": &response.tenant_id,
            "repo_id": &response.repo_id,
            "entry_symbol_id": &process.entry_symbol_id,
            "entry_name": &process.entry_name,
            "entry_file_path": &process.entry_file_path,
            "entry_line": process.entry_line,
            "trigger": &process.trigger,
            "confidence": process.confidence,
            "steps": &process.steps,
            "step_count": process.steps.len(),
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
            "provenance": {
                "kind": "runpod_burst_import",
                "job_id": &response.job_id,
                "worker_id": &response.worker_id,
                "model_id": &response.model_id,
            },
        }),
    )
}

fn generic_artifact_node(
    response: &CodeRunPodBurstResponse,
    artifact: &CodeRunPodArtifact,
) -> NodeRecord {
    NodeRecord::new(
        &artifact.artifact_id,
        [CODE_BURST_ARTIFACT_LABEL],
        json!({
            "tenant_id": &response.tenant_id,
            "repo_id": &response.repo_id,
            "job_id": &response.job_id,
            "artifact_kind": &artifact.artifact_kind,
            "payload": &artifact.payload,
            "provenance": &artifact.provenance,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    )
}

fn link_job_artifact<S: GraphStore>(
    store: &mut S,
    response: &CodeRunPodBurstResponse,
    artifact_id: &str,
    artifact_kind: &str,
) -> GraphStoreResult<()> {
    store.upsert_edge(EdgeRecord::new(
        burst_edge_id(&response.job_id, artifact_id),
        &response.job_id,
        BURST_PRODUCED_ARTIFACT,
        artifact_id,
        json!({
            "tenant_id": &response.tenant_id,
            "repo_id": &response.repo_id,
            "artifact_kind": artifact_kind,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    ))?;
    Ok(())
}

fn burst_edge_id(job_id: &str, artifact_id: &str) -> String {
    format!(
        "code:edge:burst:{}",
        stable_hash(json!([job_id, artifact_id]))
    )
}

fn collect_runpod_dependencies<S: GraphStore>(
    store: &S,
    symbols: &[CodeSymbolSnapshot],
) -> Vec<CodeDependencySnapshot> {
    let symbol_ids = symbols
        .iter()
        .map(|symbol| symbol.symbol_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut seen = std::collections::BTreeSet::new();
    let mut dependencies = Vec::new();
    for symbol in symbols {
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            for hit in store
                .neighbors(
                    rustyred_thg_core::NeighborQuery::out(&symbol.symbol_id)
                        .with_edge_type(edge_type),
                )
                .into_iter()
                .filter(|hit| symbol_ids.contains(hit.node_id.as_str()))
            {
                let key = (
                    symbol.symbol_id.clone(),
                    hit.node_id.clone(),
                    hit.edge_type.clone(),
                );
                if !seen.insert(key.clone()) {
                    continue;
                }
                dependencies.push(CodeDependencySnapshot {
                    from_symbol_id: key.0,
                    to_symbol_id: key.1,
                    edge_type: key.2,
                });
            }
        }
    }
    dependencies.sort_by(|left, right| {
        left.from_symbol_id
            .cmp(&right.from_symbol_id)
            .then_with(|| left.edge_type.cmp(&right.edge_type))
            .then_with(|| left.to_symbol_id.cmp(&right.to_symbol_id))
    });
    dependencies
}
