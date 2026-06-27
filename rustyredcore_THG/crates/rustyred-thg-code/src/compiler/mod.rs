mod ambient;
mod annotation;
mod code_to_spec;
mod drift;
mod features;
mod hooks;
mod ir;
mod obligations;
mod pattern;
mod process;
mod runpod;
mod trace_contract;

pub use ambient::{
    compiler_ambient_readout_in_store, refresh_code_compiler_artifacts_for_repo,
    AmbientCompilerReadout, DEFAULT_AMBIENT_COMPILER_FINDING_LIMIT,
};
pub use annotation::{
    annotate_code_features_in_store, CodeAnnotationInput, CodeAnnotationOutput,
    CodeCompilerAnnotationRecord, CodeEblFeatureContribution, ANNOTATES_CODE_FEATURE,
    CODE_ANNOTATION_LABEL,
};
pub use code_to_spec::{compile_code_spec_in_store, compile_code_spec_snapshot};
pub use drift::{detect_code_spec_drift, detect_code_spec_drift_in_store};
pub use features::{
    extract_code_features_in_store, CodeConnectionFeatureVector, CodeFeatureExtractInput,
    CodeFeatureExtractOutput, CodeFeatureRecord, CODE_FEATURE_LABEL, FEATURE_SOURCE_CODE,
    FEATURE_TARGET_CODE,
};
pub use hooks::incremental_code_compiler_hook;
pub use ir::{
    CodeDependencySnapshot, CodeFileSnapshot, CodeSpecCompileInput, CodeSpecCompileOutput,
    CodeSpecDriftFinding, CodeSpecDriftInput, CodeSpecDriftKind, CodeSpecDriftReport,
    CodeSymbolSnapshot, CODE_COMPILER_DRIFT_LABEL, CODE_COMPILER_FEATURE_VERSION,
    CODE_COMPILER_VERSION, CODE_SPEC_LABEL, DEFAULT_CODE_COMPILER_SYMBOL_LIMIT, DRIFT_FOR_CODE,
    DRIFT_FOR_SPEC, SPECIFIES_CODE,
};
pub use obligations::{
    compile_code_implementation_obligations, compile_code_implementation_obligations_in_store,
    CodeImplementationObligation, CodeImplementationObligationInput,
    CodeImplementationObligationOutput, CodeSpecificationSummary,
    CODE_IMPLEMENTATION_OBLIGATION_LABEL, OBLIGATES_CODE_SYMBOL, OBLIGATION_DERIVES_FROM,
};
pub use pattern::{
    record_code_pattern_memory_in_store, relevant_code_patterns, CodePatternMemoryInput,
    CodePatternMemoryRecord, CODE_PATTERN_LABEL, PATTERN_APPLIES_TO_CODE,
};
pub use process::{
    detect_code_processes_in_store, CodeProcessDetectInput, CodeProcessDetectOutput,
    CodeProcessFlow, CodeProcessStep, CODE_PROCESS_LABEL, PROCESS_ENTRYPOINT, PROCESS_TOUCHES_CODE,
};
pub use runpod::{
    build_runpod_burst_request, build_runpod_burst_request_from_store,
    import_runpod_burst_response_in_store, CodeRunPodArtifact, CodeRunPodBurstRequest,
    CodeRunPodBurstResponse, CodeRunPodImportReport, BURST_PRODUCED_ARTIFACT,
    CODE_BURST_ARTIFACT_LABEL, CODE_BURST_JOB_LABEL,
};
pub use trace_contract::{
    compile_trace_contract, ApiContractObservation, BodyShapeHint, EndpointContract,
    HttpExchangeTrace, ObservedStateTransition, RuntimeTraceEvent, RuntimeTraceEventKind,
    TimingRange, TraceContractReport, TraceErrorObservation, TraceValidatorSpec,
};

#[cfg(test)]
mod tests {
    use rustyred_thg_core::{
        EdgeRecord, GraphMutation, GraphMutationBatch, HookDispatcher, HookDispatcherConfig,
        InMemoryGraphStore, NeighborQuery, NodeQuery, NodeRecord, RedCoreGraphStore,
    };
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;
    use crate::{
        CALLS_SYMBOL, CODE_FILE_LABEL, CODE_REPO_LABEL, CODE_SYMBOL_LABEL, CONTAINS_FILE,
        DECLARES_SYMBOL, SOURCE,
    };

    #[test]
    fn code_spec_compiler_writes_spec_node_and_coverage_edges() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:engine",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "struct",
                name: "Engine",
                line: 3,
                signature: "pub struct Engine",
            },
        );
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:compile",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "compile_code",
                line: 9,
                signature: "pub fn compile_code()",
            },
        );
        store
            .upsert_edge(EdgeRecord::new(
                "edge:compile_engine",
                "sym:compile",
                CALLS_SYMBOL,
                "sym:engine",
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();

        let mut input = CodeSpecCompileInput::new("Travis-Gilbert", "repo:compiler");
        input.repo_label = Some("compiler-fixture".to_string());
        let output = compile_code_spec_in_store(&mut store, input).unwrap();

        assert_eq!(output.file_count, 1);
        assert_eq!(output.symbol_count, 2);
        assert_eq!(output.structure_count, 1);
        assert_eq!(output.member_count, 1);
        assert_eq!(output.dependency_edge_count, 1);
        assert!(output.spec_body.contains("## Module Inventory"));
        assert!(output.spec_body.contains("## Structure Catalog"));
        assert!(output.spec_body.contains("## Symbol Dependency Graph"));
        assert!(store.get_node(&output.spec_node.id).is_some());

        let spec_nodes = store.query_nodes(NodeQuery::label(CODE_SPEC_LABEL));
        assert_eq!(spec_nodes.len(), 1);
        let coverage_edges = store
            .neighbors(NeighborQuery::out(&output.spec_node.id).with_edge_type(SPECIFIES_CODE));
        assert_eq!(coverage_edges.len(), 2);
    }

    #[test]
    fn code_spec_drift_reports_missing_undocumented_and_signature_change() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");

        let expected_symbols = vec![
            symbol_snapshot("sym:missing", "old_fn", "function", "pub fn old_fn()"),
            symbol_snapshot(
                "sym:compile",
                "compile_code",
                "function",
                "pub fn compile_code()",
            ),
        ];
        store
            .upsert_node(NodeRecord::new(
                "spec:baseline",
                [CODE_SPEC_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "repo_id": "repo:compiler",
                    "compiled_symbols": &expected_symbols,
                    "source": SOURCE,
                }),
            ))
            .unwrap();
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:compile",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "compile_code",
                line: 9,
                signature: "pub fn compile_code(input: &str)",
            },
        );
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:new",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "new_fn",
                line: 20,
                signature: "pub fn new_fn()",
            },
        );

        let input = CodeSpecDriftInput::new("Travis-Gilbert", "repo:compiler", "spec:baseline");
        let report = detect_code_spec_drift(&store, &input).unwrap();
        let kinds = report
            .findings
            .iter()
            .map(|finding| finding.drift_kind)
            .collect::<Vec<_>>();
        assert!(kinds.contains(&CodeSpecDriftKind::MissingSymbol));
        assert!(kinds.contains(&CodeSpecDriftKind::UndocumentedSymbol));
        assert!(kinds.contains(&CodeSpecDriftKind::SignatureChanged));

        let written = detect_code_spec_drift_in_store(&mut store, input).unwrap();
        assert_eq!(written.findings.len(), 3);
        let finding_nodes = store.query_nodes(NodeQuery::label(CODE_COMPILER_DRIFT_LABEL));
        assert_eq!(finding_nodes.len(), 3);
        let spec_edges =
            store.neighbors(NeighborQuery::in_("spec:baseline").with_edge_type(DRIFT_FOR_SPEC));
        assert_eq!(spec_edges.len(), 3);
    }

    #[test]
    fn compiler_hook_bootstraps_spec_then_surfaces_drift() {
        let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
        let dispatcher = HookDispatcher::start(
            Arc::clone(&store),
            vec![incremental_code_compiler_hook()],
            HookDispatcherConfig::default(),
        );
        store
            .lock()
            .unwrap()
            .attach_hook_emitter(dispatcher.emitter());

        {
            let mut guard = store.lock().unwrap();
            guard
                .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                    hook_symbol("pub fn compiler_entry()"),
                )]))
                .unwrap();
        }
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "compiler hook drained after initial symbol write"
        );
        {
            let guard = store.lock().unwrap();
            let specs = guard
                .query_nodes(NodeQuery::label(CODE_SPEC_LABEL))
                .unwrap();
            assert_eq!(specs.len(), 1);
        }

        {
            let mut guard = store.lock().unwrap();
            guard
                .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(
                    hook_symbol("pub fn compiler_entry(input: &str)"),
                )]))
                .unwrap();
        }
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "compiler hook drained after signature change"
        );
        let guard = store.lock().unwrap();
        let drift_nodes = guard
            .query_nodes(NodeQuery::label(CODE_COMPILER_DRIFT_LABEL))
            .unwrap();
        assert_eq!(drift_nodes.len(), 1);
    }

    #[test]
    fn process_detector_lowers_entrypoint_flow() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:main",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "main",
                line: 1,
                signature: "fn main()",
            },
        );
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:helper",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "helper",
                line: 5,
                signature: "fn helper()",
            },
        );
        store
            .upsert_edge(EdgeRecord::new(
                "edge:main-helper",
                "sym:main",
                CALLS_SYMBOL,
                "sym:helper",
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();

        let output = detect_code_processes_in_store(
            &mut store,
            CodeProcessDetectInput::new("Travis-Gilbert", "repo:compiler"),
        )
        .unwrap();

        assert_eq!(output.processes.len(), 1);
        assert_eq!(output.processes[0].entry_symbol_id, "sym:main");
        assert_eq!(output.processes[0].steps.len(), 2);
        let process_nodes = store.query_nodes(NodeQuery::label(CODE_PROCESS_LABEL));
        assert_eq!(process_nodes.len(), 1);
        let process_edges = store.neighbors(
            NeighborQuery::out(&output.processes[0].process_id)
                .with_edge_type(PROCESS_TOUCHES_CODE),
        );
        assert_eq!(process_edges.len(), 2);
    }

    #[test]
    fn pattern_memory_records_and_retrieves_relevant_fix() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:compile",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "compile_code",
                line: 9,
                signature: "pub fn compile_code()",
            },
        );
        let mut input = CodePatternMemoryInput::new(
            "Travis-Gilbert",
            "repo:compiler",
            "Prefer parser-backed edits",
            "Use parser-backed symbol extraction before touching edge inference.",
        );
        input.feedback = "Fix worked after avoiding lexical-only symbol matching.".to_string();
        input.root_cause = "Lexical-only edit missed moved symbol.".to_string();
        input.symbol_ids = vec!["sym:compile".to_string()];
        input.file_paths = vec!["src/lib.rs".to_string()];
        let record = record_code_pattern_memory_in_store(&mut store, input).unwrap();

        let patterns = relevant_code_patterns(
            &store,
            "Travis-Gilbert",
            "repo:compiler",
            "parser symbol",
            "src/",
            5,
        );
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].pattern_id, record.pattern_id);
        let edges = store.neighbors(
            NeighborQuery::out(&record.pattern_id).with_edge_type(PATTERN_APPLIES_TO_CODE),
        );
        assert_eq!(edges.len(), 1);
    }

    #[test]
    fn feature_contract_and_edl_ebl_annotations_write_through() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:parse",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "parse_request",
                line: 1,
                signature: "fn parse_request(json: &str)",
            },
        );
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:decode",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "decode_json",
                line: 6,
                signature: "fn decode_json(json: &str)",
            },
        );
        store
            .upsert_edge(EdgeRecord::new(
                "edge:parse-decode",
                "sym:parse",
                CALLS_SYMBOL,
                "sym:decode",
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();

        let features = extract_code_features_in_store(
            &mut store,
            CodeFeatureExtractInput::new("Travis-Gilbert", "repo:compiler"),
        )
        .unwrap();
        assert!(!features.records.is_empty());
        assert_eq!(features.records[0].features.values().len(), 21);
        assert!(features.records[0].features.jaccard_coefficient > 0.0);

        let annotations = annotate_code_features_in_store(
            &mut store,
            CodeAnnotationInput::new("Travis-Gilbert", "repo:compiler"),
        )
        .unwrap();
        assert_eq!(annotations.annotations.len(), features.records.len());
        assert!(!annotations.annotations[0].top_features.is_empty());
        assert!(annotations.annotations[0].epistemic_uncertainty < 1.0);
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(CODE_FEATURE_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(CODE_ANNOTATION_LABEL))
                .len(),
            1
        );
    }

    #[test]
    fn runpod_burst_import_writes_provenance_artifacts() {
        let mut store = InMemoryGraphStore::new();
        insert_repo(&mut store);
        insert_file(&mut store, "file:lib", "src/lib.rs");
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:a",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "alpha",
                line: 10,
                signature: "pub fn alpha()",
            },
        );
        insert_symbol(
            &mut store,
            SymbolFixture {
                id: "sym:b",
                file_id: "file:lib",
                file_path: "src/lib.rs",
                kind: "function",
                name: "beta",
                line: 20,
                signature: "pub fn beta()",
            },
        );
        store
            .upsert_edge(EdgeRecord::new(
                "edge:beta-alpha",
                "sym:b",
                CALLS_SYMBOL,
                "sym:a",
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();
        let request = build_runpod_burst_request(
            "Travis-Gilbert",
            "repo:compiler",
            Some("refs/heads/main".to_string()),
        );
        let self_contained_request = build_runpod_burst_request_from_store(
            &store,
            "Travis-Gilbert",
            "repo:compiler",
            Some("refs/heads/main".to_string()),
        );
        assert_eq!(self_contained_request.symbols.len(), 2);
        assert_eq!(self_contained_request.dependency_edges.len(), 1);
        assert_eq!(
            self_contained_request.dependency_edges[0].edge_type,
            CALLS_SYMBOL
        );
        let feature = CodeFeatureRecord {
            feature_id: "feature:runpod".to_string(),
            source_symbol_id: "sym:a".to_string(),
            target_symbol_id: "sym:b".to_string(),
            feature_version: CODE_COMPILER_FEATURE_VERSION.to_string(),
            model_id: Some("runpod-scorer-v1".to_string()),
            features: CodeConnectionFeatureVector {
                nli_entailment_score: 0.7,
                sbert_cosine: 0.8,
                ..CodeConnectionFeatureVector::default()
            },
            provenance: json!({"kind": "runpod"}),
        };
        let annotation = CodeCompilerAnnotationRecord {
            annotation_id: "annotation:runpod".to_string(),
            feature_id: feature.feature_id.clone(),
            epistemic_uncertainty: 0.2,
            aleatoric_uncertainty: 0.1,
            evidence_count: 5.0,
            active_feature_count: 2,
            explanation: "RunPod model found entailment and semantic similarity.".to_string(),
            top_features: vec![CodeEblFeatureContribution {
                feature: "nli_entailment_score".to_string(),
                value: 0.7,
                importance: 0.7,
            }],
            calibration_version: "runpod-cal-v1".to_string(),
        };
        let response = CodeRunPodBurstResponse {
            tenant_id: request.tenant_id,
            repo_id: request.repo_id,
            job_id: request.job_id.clone(),
            worker_id: Some("worker-a".to_string()),
            model_id: Some("runpod-scorer-v1".to_string()),
            processes: Vec::new(),
            patterns: Vec::new(),
            features: vec![feature],
            annotations: vec![annotation],
            artifacts: vec![CodeRunPodArtifact {
                artifact_id: "artifact:embedding-batch".to_string(),
                artifact_kind: "embedding_batch".to_string(),
                payload: json!({"rows": 2}),
                provenance: json!({"object_uri": "s3://bucket/features.parquet"}),
            }],
            completed_at_ms: 42,
        };

        let report = import_runpod_burst_response_in_store(&mut store, response).unwrap();
        assert_eq!(report.job_id, request.job_id);
        assert_eq!(report.feature_count, 1);
        assert_eq!(report.annotation_count, 1);
        assert_eq!(report.artifact_count, 1);
        assert_eq!(
            store
                .query_nodes(NodeQuery::label(CODE_BURST_JOB_LABEL))
                .len(),
            1
        );
        assert_eq!(
            store
                .neighbors(
                    NeighborQuery::out(&request.job_id).with_edge_type(BURST_PRODUCED_ARTIFACT)
                )
                .len(),
            3
        );
    }

    fn insert_repo(store: &mut InMemoryGraphStore) {
        store
            .upsert_node(NodeRecord::new(
                "repo:compiler",
                [CODE_REPO_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "repo_id": "repo:compiler",
                    "repo_root": "compiler-fixture",
                    "source": SOURCE,
                }),
            ))
            .unwrap();
    }

    fn insert_file(store: &mut InMemoryGraphStore, file_id: &str, path: &str) {
        store
            .upsert_node(NodeRecord::new(
                file_id,
                [CODE_FILE_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "repo_id": "repo:compiler",
                    "file_id": file_id,
                    "path": path,
                    "language": "rust",
                    "content_hash": "hash:lib",
                    "source": SOURCE,
                }),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                format!("edge:repo_file:{file_id}"),
                "repo:compiler",
                CONTAINS_FILE,
                file_id,
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();
    }

    fn insert_symbol(store: &mut InMemoryGraphStore, fixture: SymbolFixture<'_>) {
        store
            .upsert_node(NodeRecord::new(
                fixture.id,
                [CODE_SYMBOL_LABEL],
                json!({
                    "tenant_id": "Travis-Gilbert",
                    "repo_id": "repo:compiler",
                    "file_id": fixture.file_id,
                    "file_path": fixture.file_path,
                    "symbol_id": fixture.id,
                    "kind": fixture.kind,
                    "name": fixture.name,
                    "language": "rust",
                    "line": fixture.line,
                    "signature": fixture.signature,
                    "call_names": [],
                    "dependency_names": [],
                    "parser_backed": true,
                    "source": SOURCE,
                }),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                format!("edge:file_symbol:{}", fixture.id),
                fixture.file_id,
                DECLARES_SYMBOL,
                fixture.id,
                json!({"tenant_id": "Travis-Gilbert", "repo_id": "repo:compiler", "source": SOURCE}),
            ))
            .unwrap();
    }

    fn symbol_snapshot(
        symbol_id: &str,
        name: &str,
        kind: &str,
        signature: &str,
    ) -> CodeSymbolSnapshot {
        CodeSymbolSnapshot {
            symbol_id: symbol_id.to_string(),
            file_id: Some("file:lib".to_string()),
            file_path: "src/lib.rs".to_string(),
            kind: kind.to_string(),
            name: name.to_string(),
            language: "rust".to_string(),
            line: Some(9),
            signature: Some(signature.to_string()),
            call_names: Vec::new(),
            dependency_names: Vec::new(),
            parser_backed: true,
        }
    }

    fn hook_symbol(signature: &str) -> NodeRecord {
        NodeRecord::new(
            "sym:compiler_entry",
            [CODE_SYMBOL_LABEL],
            json!({
                "tenant_id": "Travis-Gilbert",
                "repo_id": "repo:compiler-hook",
                "file_id": "file:lib",
                "file_path": "src/lib.rs",
                "symbol_id": "sym:compiler_entry",
                "kind": "function",
                "name": "compiler_entry",
                "language": "rust",
                "line": 1,
                "signature": signature,
                "call_names": [],
                "dependency_names": [],
                "parser_backed": true,
                "source": SOURCE,
            }),
        )
    }

    struct SymbolFixture<'a> {
        id: &'a str,
        file_id: &'a str,
        file_path: &'a str,
        kind: &'a str,
        name: &'a str,
        line: u64,
        signature: &'a str,
    }
}
