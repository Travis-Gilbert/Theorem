use rustyred_thg_core::{
    stable_hash, GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord,
    RedCoreGraphStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::datawave_projection::{
    datawave_fact_summaries_for_repo, project_code_to_datawave, CodeToDatawaveProjectionInput,
    FieldFactSummary, ProjectionReceipt,
};
use crate::ensure::repo_id_from_url;
use crate::{
    compile_code_implementation_obligations_in_store, compile_code_spec_in_store,
    detect_code_spec_drift_in_store, ensure_repo_kg_in_store, extract_code_features_in_store,
    normalize_tenant, relevant_code_patterns, CodeFeatureExtractInput, CodeFeatureRecord,
    CodeImplementationObligation, CodeImplementationObligationInput, CodePatternMemoryRecord,
    CodeSpecCompileInput, CodeSpecCompileOutput, CodeSpecDriftFinding, CodeSpecDriftInput,
    CodeSpecificationSummary, RepoFetchCaps, RepoKgStatus, BINARY_ARTIFACT_LABEL,
    CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION, CODE_REPO_LABEL, HEAD_SHA_PROPERTY,
    SOURCE,
};

pub const REVERSE_ENGINEER_COMPOSE_RECEIPT_LABEL: &str = "ReverseEngineerComposeReceipt";

const DEFAULT_PATTERN_LIMIT: usize = 16;
const DEFAULT_DATAWAVE_FACT_LIMIT: usize = 500_000;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct SourceRef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
}

impl SourceRef {
    pub fn repo_url(&self) -> Option<&str> {
        self.github_url
            .as_deref()
            .or(self.repo_url.as_deref())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    }

    pub fn resolved_repo_id(&self) -> Option<String> {
        self.repo_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .or_else(|| self.repo_url().map(repo_id_from_url))
            .or_else(|| {
                self.local_path
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .and_then(|path| path.rsplit('/').find(|part| !part.is_empty()))
                    .map(|slug| format!("repo:{slug}"))
            })
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ComposeInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    pub source: SourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_symbols: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_features: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub datawave_fact_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_label: Option<String>,
}

impl ComposeInput {
    pub fn for_repo(tenant_id: impl Into<String>, repo_id: impl Into<String>) -> Self {
        Self {
            tenant_id: Some(tenant_id.into()),
            source: SourceRef {
                repo_id: Some(repo_id.into()),
                ..SourceRef::default()
            },
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryReconstructionSummary {
    pub artifact_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ComposeProvenance {
    pub ingest_path: String,
    pub repo_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha: Option<String>,
    pub compiler_version: String,
    pub feature_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_to_datawave: Option<ProjectionReceipt>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ReconstructionSpec {
    pub source_ref: SourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_spec: Option<CodeSpecCompileOutput>,
    pub features: Vec<CodeFeatureRecord>,
    pub obligations: Vec<CodeImplementationObligation>,
    pub patterns: Vec<CodePatternMemoryRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<BinaryReconstructionSummary>,
    pub datawave_facts: Vec<FieldFactSummary>,
    pub drift: Vec<CodeSpecDriftFinding>,
    pub provenance: ComposeProvenance,
    pub code_files_count: usize,
    pub code_symbols_count: usize,
}

pub fn compose_reconstruction_spec_in_store<S: GraphStore>(
    store: &mut S,
    input: &ComposeInput,
) -> GraphStoreResult<ReconstructionSpec> {
    compose_reconstruction_spec_with_provenance(store, input, None)
}

pub fn compose_reconstruction_spec_with_ensure_in_store(
    store: &mut RedCoreGraphStore,
    mut input: ComposeInput,
    caps: &RepoFetchCaps,
) -> GraphStoreResult<ReconstructionSpec> {
    let tenant_id = input_tenant(&input);
    let repo_url = input.source.repo_url().map(str::to_string);
    let repo_id = input
        .source
        .resolved_repo_id()
        .ok_or_else(|| invalid_compose_input("source.repo_id or source.github_url is required"))?;
    let status = if let Some(url) = repo_url.as_deref() {
        ensure_repo_kg_in_store(
            store,
            &tenant_id,
            url,
            input.source.sha.as_deref(),
            Some(&repo_id),
            caps,
        )
        .map_err(code_index_error)?
    } else {
        RepoKgStatus::LoadedFromSnapshot {
            sha: current_repo_sha(store, &tenant_id, &repo_id).unwrap_or_default(),
        }
    };
    input.tenant_id = Some(tenant_id);
    input.source.repo_id = Some(repo_id);
    if input.source.sha.is_none() && !status.sha().trim().is_empty() {
        input.source.sha = Some(status.sha().to_string());
    }
    compose_reconstruction_spec_with_provenance(store, &input, Some(&status))
}

fn compose_reconstruction_spec_with_provenance<S: GraphStore>(
    store: &mut S,
    input: &ComposeInput,
    ingest_status: Option<&RepoKgStatus>,
) -> GraphStoreResult<ReconstructionSpec> {
    let tenant_id = input_tenant(input);
    let repo_id = input
        .source
        .resolved_repo_id()
        .ok_or_else(|| invalid_compose_input("source.repo_id or source.github_url is required"))?;

    let mut spec_input = CodeSpecCompileInput::new(&tenant_id, &repo_id);
    spec_input.repo_label = input.repo_label.clone();
    if let Some(max_symbols) = input.max_symbols {
        spec_input.max_symbols = max_symbols;
    }
    let code_spec = compile_code_spec_in_store(store, spec_input)?;

    let mut feature_input = CodeFeatureExtractInput::new(&tenant_id, &repo_id);
    if let Some(max_features) = input.max_features {
        feature_input.max_pairs = max_features;
    }
    let features = extract_code_features_in_store(store, feature_input)?.records;

    let patterns = relevant_code_patterns(
        store,
        &tenant_id,
        &repo_id,
        pattern_query(input),
        input.path_prefix.as_deref().unwrap_or_default(),
        input.pattern_limit.unwrap_or(DEFAULT_PATTERN_LIMIT),
    );

    let mut obligation_input = CodeImplementationObligationInput::new(&tenant_id, &repo_id);
    obligation_input.code_spec_summary = Some(CodeSpecificationSummary {
        spec_node_id: Some(code_spec.spec_node.id.clone()),
        artifact_hash: Some(code_spec.artifact_hash.clone()),
        tenant_id: tenant_id.clone(),
        repo_id: repo_id.clone(),
        file_count: code_spec.file_count,
        symbol_count: code_spec.symbol_count,
    });
    obligation_input.feature_records = features.clone();
    obligation_input.pattern_memories = patterns.clone();
    let obligations =
        compile_code_implementation_obligations_in_store(store, obligation_input)?.obligations;

    let drift = detect_code_spec_drift_in_store(
        store,
        CodeSpecDriftInput::new(&tenant_id, &repo_id, &code_spec.spec_node.id),
    )?
    .findings;

    let projection = project_code_to_datawave(
        store,
        &CodeToDatawaveProjectionInput::new(&tenant_id, &repo_id),
    )?;
    let datawave_facts = datawave_fact_summaries_for_repo(
        store,
        &tenant_id,
        &repo_id,
        input
            .datawave_fact_limit
            .unwrap_or(DEFAULT_DATAWAVE_FACT_LIMIT),
    );
    let binary = binary_summary(store, &tenant_id, &repo_id);
    let provenance = ComposeProvenance {
        ingest_path: ingest_status
            .map(repo_status_name)
            .unwrap_or("AlreadyInStore")
            .to_string(),
        repo_id: repo_id.clone(),
        sha: input
            .source
            .sha
            .clone()
            .or_else(|| current_repo_sha(store, &tenant_id, &repo_id)),
        compiler_version: CODE_COMPILER_VERSION.to_string(),
        feature_version: CODE_COMPILER_FEATURE_VERSION.to_string(),
        code_to_datawave: Some(projection),
    };
    let spec = ReconstructionSpec {
        source_ref: input.source.clone(),
        code_files_count: code_spec.file_count,
        code_symbols_count: code_spec.symbol_count,
        code_spec: Some(code_spec),
        features,
        obligations,
        patterns,
        binary,
        datawave_facts,
        drift,
        provenance,
    };
    write_compose_receipt(store, &tenant_id, &repo_id, &spec)?;
    Ok(spec)
}

fn write_compose_receipt<S: GraphStore>(
    store: &mut S,
    tenant_id: &str,
    repo_id: &str,
    spec: &ReconstructionSpec,
) -> GraphStoreResult<()> {
    let receipt_id = format!(
        "reverse-engineer:compose:{}",
        stable_hash(json!([
            tenant_id,
            repo_id,
            spec.provenance.sha,
            spec.code_files_count,
            spec.code_symbols_count
        ]))
    );
    store.upsert_node(NodeRecord::new(
        receipt_id,
        [REVERSE_ENGINEER_COMPOSE_RECEIPT_LABEL],
        json!({
            "tenant_id": tenant_id,
            "repo_id": repo_id,
            "sha": spec.provenance.sha,
            "code_files_count": spec.code_files_count,
            "code_symbols_count": spec.code_symbols_count,
            "features_count": spec.features.len(),
            "obligations_count": spec.obligations.len(),
            "patterns_count": spec.patterns.len(),
            "datawave_facts_count": spec.datawave_facts.len(),
            "drift_count": spec.drift.len(),
            "ingest_path": spec.provenance.ingest_path,
            "compiler_version": CODE_COMPILER_VERSION,
            "feature_version": CODE_COMPILER_FEATURE_VERSION,
            "source": SOURCE,
        }),
    ))?;
    Ok(())
}

fn input_tenant(input: &ComposeInput) -> String {
    normalize_tenant(input.tenant_id.as_deref().unwrap_or("Travis-Gilbert"))
}

fn pattern_query(input: &ComposeInput) -> &str {
    input
        .repo_label
        .as_deref()
        .or(input.source.github_url.as_deref())
        .or(input.source.repo_url.as_deref())
        .or(input.source.repo_id.as_deref())
        .unwrap_or_default()
}

fn repo_status_name(status: &RepoKgStatus) -> &'static str {
    match status {
        RepoKgStatus::LoadedFromSnapshot { .. } => "LoadedFromSnapshot",
        RepoKgStatus::IncrementallyIngested { .. } => "IncrementallyIngested",
        RepoKgStatus::FullyIngested { .. } => "FullyIngested",
    }
}

fn current_repo_sha<S: GraphStore>(store: &S, tenant_id: &str, repo_id: &str) -> Option<String> {
    store
        .query_nodes(
            NodeQuery::label(CODE_REPO_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(10),
        )
        .into_iter()
        .find_map(|node| {
            node.properties
                .get(HEAD_SHA_PROPERTY)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
}

fn binary_summary<S: GraphStore>(
    store: &S,
    tenant_id: &str,
    repo_id: &str,
) -> Option<BinaryReconstructionSummary> {
    let artifact_count = store
        .query_nodes(
            NodeQuery::label(BINARY_ARTIFACT_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_limit(10_000),
        )
        .into_iter()
        .filter(|node| {
            node.properties
                .get("repo_id")
                .and_then(Value::as_str)
                .is_none_or(|value| value == repo_id)
        })
        .filter(|node| !node.tombstone)
        .count();
    (artifact_count > 0).then_some(BinaryReconstructionSummary { artifact_count })
}

fn invalid_compose_input(message: impl Into<String>) -> GraphStoreError {
    GraphStoreError::new("invalid_reverse_engineer_compose_input", message.into())
}

fn code_index_error(error: crate::CodeIndexError) -> GraphStoreError {
    GraphStoreError::new(error.code, error.message)
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{
        EdgeRecord, InMemoryGraphStore, NodeQuery, NodeRecord, RedCoreGraphStore, RedCoreOptions,
    };
    use serde_json::json;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{
        record_code_pattern_memory_in_store, CodePatternMemoryInput, CALLS_SYMBOL, CODE_FILE_LABEL,
        CODE_REPO_LABEL, CODE_SYMBOL_LABEL, CONTAINS_FILE, DECLARES_SYMBOL,
    };

    #[test]
    fn compose_reconstruction_spec_projects_code_into_datawave_facts() {
        let mut store = InMemoryGraphStore::new();
        seed_code_graph(&mut store);

        let input = ComposeInput::for_repo("Travis-Gilbert", "repo:compose");
        let spec = compose_reconstruction_spec_in_store(&mut store, &input).unwrap();

        assert_eq!(spec.code_files_count, 2);
        assert_eq!(spec.code_symbols_count, 3);
        assert!(spec.code_spec.is_some());
        assert!(!spec.obligations.is_empty());
        assert_eq!(spec.binary, None);
        assert!(spec.drift.is_empty());
        assert_eq!(spec.provenance.ingest_path, "AlreadyInStore");
        assert!(spec.datawave_facts.len() >= spec.code_files_count);
        assert!(spec
            .datawave_facts
            .iter()
            .any(|fact| fact.field == "file_path" && fact.value == "src/lib.rs"));
        assert!(store
            .query_nodes(NodeQuery::label(REVERSE_ENGINEER_COMPOSE_RECEIPT_LABEL))
            .iter()
            .any(|node| node.properties["repo_id"] == json!("repo:compose")));
    }

    #[test]
    #[ignore = "live GitHub acceptance test for docs/plans/reverse-engineer-compose/SPEC.md"]
    fn live_lightwood_cold_path_compose_returns_populated_spec() {
        let root = unique_temp_dir("thg-lightwood-compose");
        let mut store = RedCoreGraphStore::open(&root, RedCoreOptions::default()).unwrap();
        let spec = compose_reconstruction_spec_with_ensure_in_store(
            &mut store,
            ComposeInput {
                tenant_id: Some("Travis-Gilbert".to_string()),
                source: SourceRef {
                    github_url: Some("https://github.com/mindsdb/lightwood.git".to_string()),
                    ..SourceRef::default()
                },
                datawave_fact_limit: Some(1_000_000),
                ..ComposeInput::default()
            },
            &RepoFetchCaps::from_requested(128 * 1024 * 1024),
        )
        .unwrap();

        assert_eq!(spec.provenance.ingest_path, "FullyIngested");
        assert!(spec.code_files_count >= 200, "{:?}", spec.provenance);
        assert!(spec.code_symbols_count >= 500, "{:?}", spec.provenance);
        assert!(spec.code_spec.is_some());
        assert!(!spec.features.is_empty());
        assert!(!spec.obligations.is_empty());
        assert!(spec.binary.is_none());
        assert!(spec.drift.is_empty());
        assert!(spec.datawave_facts.len() >= spec.code_files_count);
    }

    fn unique_temp_dir(prefix: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    fn seed_code_graph(store: &mut InMemoryGraphStore) {
        store
            .upsert_node(NodeRecord::new(
                "repo:compose",
                [CODE_REPO_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "repo_id": "repo:compose",
                    "head_sha": "abc123",
                    "source": SOURCE,
                }),
            ))
            .unwrap();
        for (file_id, path, hash) in [
            ("file:lib", "src/lib.rs", "hash:lib"),
            ("file:model", "src/model.rs", "hash:model"),
        ] {
            store
                .upsert_node(NodeRecord::new(
                    file_id,
                    [CODE_FILE_LABEL],
                    json!({
                        "tenant_id": "Travis-Gilbert",
                        "repo_id": "repo:compose",
                        "file_id": file_id,
                        "path": path,
                        "language": "rust",
                        "content_hash": hash,
                        "source": SOURCE,
                    }),
                ))
                .unwrap();
            store
                .upsert_edge(EdgeRecord::new(
                    format!("edge:repo-file:{file_id}"),
                    "repo:compose",
                    CONTAINS_FILE,
                    file_id,
                    json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compose", "source": SOURCE}),
                ))
                .unwrap();
        }
        for (symbol_id, file_id, path, kind, name, line) in [
            (
                "sym:engine",
                "file:lib",
                "src/lib.rs",
                "struct",
                "Engine",
                3u64,
            ),
            (
                "sym:compose",
                "file:lib",
                "src/lib.rs",
                "function",
                "compose",
                8u64,
            ),
            (
                "sym:model",
                "file:model",
                "src/model.rs",
                "struct",
                "Model",
                2u64,
            ),
        ] {
            store
                .upsert_node(NodeRecord::new(
                    symbol_id,
                    [CODE_SYMBOL_LABEL],
                    json!({
                        "tenant_id": "Travis-Gilbert",
                        "repo_id": "repo:compose",
                        "symbol_id": symbol_id,
                        "file_id": file_id,
                        "file_path": path,
                        "kind": kind,
                        "name": name,
                        "language": "rust",
                        "line": line,
                        "signature": format!("pub {kind} {name}"),
                        "call_names": [],
                        "dependency_names": [],
                        "parser_backed": true,
                        "source": SOURCE,
                    }),
                ))
                .unwrap();
            store
                .upsert_edge(EdgeRecord::new(
                    format!("edge:file-symbol:{symbol_id}"),
                    file_id,
                    DECLARES_SYMBOL,
                    symbol_id,
                    json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compose", "source": SOURCE}),
                ))
                .unwrap();
        }
        store
            .upsert_edge(EdgeRecord::new(
                "edge:compose-engine",
                "sym:compose",
                CALLS_SYMBOL,
                "sym:engine",
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compose", "source": SOURCE}),
            ))
            .unwrap();
        let mut pattern = CodePatternMemoryInput::new(
            "Travis-Gilbert",
            "repo:compose",
            "Compose feature requires parser-backed validation",
            "Validate generated behavior IR against the compose source path before target emission.",
        );
        pattern.root_cause = "compose feature bridge".to_string();
        pattern.feedback = "Preserve the compiler-backed source evidence.".to_string();
        pattern.symbol_ids = vec!["sym:compose".to_string()];
        pattern.file_paths = vec!["src/lib.rs".to_string()];
        pattern.confidence = 0.95;
        record_code_pattern_memory_in_store(store, pattern).unwrap();
    }
}
