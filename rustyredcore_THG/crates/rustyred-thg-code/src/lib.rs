//! Code parsing plugin/runtime over RedCore.
//!
//! This crate owns the graph shape for code search. It is intentionally
//! independent of tonic/MCP/HTTP so every transport can call the same parser
//! and write into the caller's `RedCoreGraphStore`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rayon::prelude::*;
use rustyred_thg_core::{
    now_ms, stable_hash, CodeKgManifest, Direction, EdgeRecord, GraphMutation, GraphMutationBatch,
    GraphSnapshot, GraphStore, GraphStoreError, GraphStoreResult, HookDispatcher,
    HookDispatcherConfig, HookRegistration, NeighborQuery, NodeQuery, NodeRecord, PluginCapability,
    PluginCapabilityKind, PluginExecutionOutput, PluginOperationContext,
    PluginOperationRegistration, PluginRegistry, RedCoreDurability, RedCoreGraphStore,
    RedCoreOptions, RustyRedPlugin, EPISTEMIC_SHADOW_LABEL, EPISTEMIC_SUPPORTS,
    HAS_EPISTEMIC_SHADOW, SAME_ECLASS, UNDERCUTS,
};
use serde_json::{json, Value};
use syn::visit::Visit;

mod code_embed_hook;
mod code_epistemic_hook;
mod code_hooks;
mod compiler;
mod context_pack;
pub mod engineering;
mod ensure;
mod ingest_jobs;
mod map_projection;
mod repo_fetch;
mod tree_sitter_extract;

pub use code_embed_hook::{
    incremental_embed_hook, incremental_embed_hook_with_embedder, EMBEDDING_DIM, EMBEDDING_PROPERTY,
};
pub use code_epistemic_hook::{
    code_epistemic_hook, run_code_epistemic_pass_for_repo, CodeDriftFinding, CodeEpistemicReadout,
    CODE_EPISTEMIC_ENGINE, DEFAULT_CODE_EPISTEMIC_TOP_K,
};
pub use code_hooks::{code_kg_hooks, incremental_centrality_hook, CENTRALITY_PROPERTY};
pub use compiler::{
    annotate_code_features_in_store, build_runpod_burst_request,
    build_runpod_burst_request_from_store, compile_code_implementation_obligations,
    compile_code_implementation_obligations_in_store, compile_code_spec_in_store,
    compile_code_spec_snapshot, compile_trace_contract, compiler_ambient_readout_in_store,
    detect_code_processes_in_store, detect_code_spec_drift, detect_code_spec_drift_in_store,
    extract_code_features_in_store, import_runpod_burst_response_in_store,
    incremental_code_compiler_hook, record_code_pattern_memory_in_store,
    refresh_code_compiler_artifacts_for_repo, relevant_code_patterns, AmbientCompilerReadout,
    ApiContractObservation, BodyShapeHint, CodeAnnotationInput, CodeAnnotationOutput,
    CodeCompilerAnnotationRecord, CodeConnectionFeatureVector, CodeDependencySnapshot,
    CodeEblFeatureContribution, CodeFeatureExtractInput, CodeFeatureExtractOutput,
    CodeFeatureRecord, CodeFileSnapshot, CodeImplementationObligation,
    CodeImplementationObligationInput, CodeImplementationObligationOutput, CodePatternMemoryInput,
    CodePatternMemoryRecord, CodeProcessDetectInput, CodeProcessDetectOutput, CodeProcessFlow,
    CodeProcessStep, CodeRunPodArtifact, CodeRunPodBurstRequest, CodeRunPodBurstResponse,
    CodeRunPodImportReport, CodeSpecCompileInput, CodeSpecCompileOutput, CodeSpecDriftFinding,
    CodeSpecDriftInput, CodeSpecDriftKind, CodeSpecDriftReport, CodeSpecificationSummary,
    CodeSymbolSnapshot, EndpointContract, HttpExchangeTrace, ObservedStateTransition,
    RuntimeTraceEvent, RuntimeTraceEventKind, TimingRange, TraceContractReport,
    TraceErrorObservation, TraceValidatorSpec, ANNOTATES_CODE_FEATURE, BURST_PRODUCED_ARTIFACT,
    CODE_ANNOTATION_LABEL, CODE_BURST_ARTIFACT_LABEL, CODE_BURST_JOB_LABEL,
    CODE_COMPILER_DRIFT_LABEL, CODE_COMPILER_FEATURE_VERSION, CODE_COMPILER_VERSION,
    CODE_FEATURE_LABEL, CODE_IMPLEMENTATION_OBLIGATION_LABEL, CODE_PATTERN_LABEL,
    CODE_PROCESS_LABEL, CODE_SPEC_LABEL, DEFAULT_AMBIENT_COMPILER_FINDING_LIMIT,
    DEFAULT_CODE_COMPILER_SYMBOL_LIMIT, DRIFT_FOR_CODE, DRIFT_FOR_SPEC, FEATURE_SOURCE_CODE,
    FEATURE_TARGET_CODE, OBLIGATES_CODE_SYMBOL, OBLIGATION_DERIVES_FROM, PATTERN_APPLIES_TO_CODE,
    PROCESS_ENTRYPOINT, PROCESS_TOUCHES_CODE, SPECIFIES_CODE,
};
pub use context_pack::{
    code_context_pack_in_store, context_pack, context_pack_fetch, AdmittedSymbol,
    CodeContextPackInput, CodeContextPackOutput, ContextPackOutput,
    DEFAULT_CONTEXT_PACK_BUDGET_TOKENS,
};
pub use engineering::{
    compile_engineering_in_memory, compile_engineering_in_store,
    compile_program_analysis_run_in_memory, compile_program_analysis_run_in_store,
    ghidra_oracle_fixture_to_program_analysis_input, AnalyzerPassReceipt, ApiContract,
    ArchitectureComponent, ArchitectureMap, BehaviorSpec, BinaryArtifact, BinaryImport,
    BinaryRelocation, BinarySection, BinaryString, BinarySymbol, EngineeringCompileInput,
    EngineeringCompileOutput, EvidenceAuthority, EvidenceMap, EvidenceSource, GhidraOracleFixture,
    GhidraOracleProgramSummary, ImplementationObligation, InstructionFact, LoaderFact,
    ObservedApiContractInput, ObservedArchitectureInput, ObservedBehaviorInput,
    ObservedImplementationObligationInput, ObservedValidatorSpecInput, ProgramAnalysisInput,
    ProgramAnalysisOutput, ProgramAnalysisRun, ProgramAnalysisStatus, ProgramAnalysisTargetKind,
    ProgramDataFlowFact, ProgramSemanticHypothesis, TheoremIrFunction, UnknownsLedger,
    ValidatorSpec, ANALYZER_PASS_RECEIPT_LABEL, ANALYZES_ARTIFACT, BINARY_ARTIFACT_LABEL,
    DERIVED_FROM_ORACLE, ENGINEERING_API_LABEL, ENGINEERING_ARCHITECTURE_LABEL,
    ENGINEERING_BEHAVIOR_LABEL, ENGINEERING_COMPILER_VERSION, ENGINEERING_COMPILE_LABEL,
    ENGINEERING_EVIDENCE_LABEL, ENGINEERING_OBLIGATION_LABEL, ENGINEERING_VALIDATOR_LABEL,
    GHIDRA_ORACLE_FIXTURE_LABEL, HAS_ANALYZER_RECEIPT, HAS_API_CONTRACT, HAS_ARCHITECTURE_MAP,
    HAS_BEHAVIOR, HAS_DATA_FLOW_FACT, HAS_EVIDENCE_SOURCE, HAS_IMPLEMENTATION_OBLIGATION,
    HAS_INSTRUCTION_FACT, HAS_LOADER_FACT, HAS_SEMANTIC_HYPOTHESIS, HAS_THIR_FUNCTION,
    HAS_VALIDATOR, INSTRUCTION_FACT_LABEL, LOADER_FACT_LABEL, PROGRAM_ANALYSIS_RUN_LABEL,
    PROGRAM_DATA_FLOW_FACT_LABEL, PROGRAM_SEMANTIC_HYPOTHESIS_LABEL, THEOREM_IR_FUNCTION_LABEL,
};
pub use ensure::{
    code_ingest_ensure, ensure_repo_kg, ensure_repo_kg_in_store, RepoKgStatus, HEAD_SHA_PROPERTY,
    REPO_URL_PROPERTY,
};
pub use ingest_jobs::{
    IngestJobEvent, IngestJobEventKind, IngestJobRegistry, IngestJobRequest, IngestJobState,
    IngestJobStatus, CODE_INGEST_JOB_LABEL,
};
pub use map_projection::{
    project_codebase_map_entries, project_codebase_map_in_store, render_codebase_map_markdown,
    CodebaseMapEntry, CodebaseMapProjection, CodebaseMapProjectionEvent, CodebaseMapProjectionSink,
    CodebaseMarkdownFileSink, CODEBASE_MAP_DEFAULT_TOP_K,
};
pub use repo_fetch::{
    fetch_repo, fetch_repo_with_credential, is_fetchable_repo_url, FetchedRepo, GitCredential,
    GitCredentialResolver, RepoFetchCaps, RepoFetchError,
};

pub const CODE_REPO_LABEL: &str = "CodeRepository";
pub const CODE_FILE_LABEL: &str = "CodeFile";
pub const CODE_SYMBOL_LABEL: &str = "CodeSymbol";
pub const CODE_SYMBOL_NAME_LABEL: &str = "CodeSymbolName";
pub const CODE_RECEIPT_LABEL: &str = "CodeInvocationReceipt";
/// Content-addressed side record holding a file's full text, keyed by the
/// file's `content_hash`. Kept OFF the `CodeFile` node so symbol/file node
/// loads (search, explore) never deserialize file contents; only
/// `code_context` reads it back.
pub const CODE_FILE_TEXT_LABEL: &str = "CodeFileText";
pub const SOURCE: &str = "rustyred_thg_code";
pub const CONTAINS_FILE: &str = "CONTAINS_FILE";
pub const DECLARES_SYMBOL: &str = "DECLARES_SYMBOL";
pub const SYMBOL_NAME_TARGET: &str = "SYMBOL_NAME_TARGET";
pub const CALLS_SYMBOL: &str = "CALLS_SYMBOL";
pub const DEPENDS_ON_SYMBOL: &str = "DEPENDS_ON_SYMBOL";
const DEFAULT_TRUST_TIER: &str = "advisory";
const DEFAULT_MAX_FILES: usize = 25_000;
const ABSOLUTE_MAX_FILES: u64 = 100_000;
const DEFAULT_MAX_FILE_BYTES: u64 = 5_000_000;
const ABSOLUTE_MAX_FILE_BYTES: u64 = 25_000_000;
const DEFAULT_LIMIT: usize = 20;
const DEFAULT_CONTEXT_LINES: u64 = 20;
const DEFAULT_MAX_CONTEXT_CHARS: usize = 20_000;
const BINARY_SNIFF_BYTES: usize = 8 * 1024;
/// Fan-out control for inferred symbol edges: a name whose `symbols_by_name`
/// bucket exceeds this is too common (`new`, `get`, `handle`) to carry signal,
/// so no edges are emitted for it at all.
const EDGE_NAME_BUCKET_CAP: usize = 24;
/// Within an admitted name, at most this many targets receive an edge from one
/// source symbol. Buckets are sorted by (file_path, line, symbol_id) so the cap
/// cuts deterministically.
const EDGE_TARGETS_PER_NAME_CAP: usize = 8;
/// Files parsed per progress/budget checkpoint during ingest.
const PARSE_CHUNK_FILES: usize = 200;
/// Dense/generated files can contain hundreds of shallow declarations where
/// the line extractor is both cheaper and semantically equivalent for graph
/// ranking. Tree-sitter stays on for normal source files and falls back here
/// to protect large-repo ingest latency.
const TREE_SITTER_SYMBOL_LINE_CAP: usize = 80;

#[derive(Clone)]
pub struct CodeIndexRuntime {
    store: Arc<Mutex<RedCoreGraphStore>>,
    jobs: Arc<IngestJobRegistry>,
    worker_started: Arc<std::sync::Once>,
    credential_resolver: Option<Arc<dyn GitCredentialResolver>>,
    map_projection_sink: Option<Arc<dyn CodebaseMapProjectionSink>>,
    // Kept alive for the runtime's lifetime so the hook worker keeps draining;
    // `None` unless graph-level code-KG hooks are enabled (see THEOREM_CODE_HOOKS).
    #[allow(dead_code)]
    hook_dispatcher: Option<Arc<HookDispatcher>>,
}

/// Build and start a graph-level hook dispatcher for the code KG over `store`,
/// attaching the emitter so every commit warms centrality + embeddings off the
/// writer's path. The returned dispatcher must outlive the store (its worker
/// stops on drop). This is the one-call embedder wiring an owner of an
/// `Arc<Mutex<RedCoreGraphStore>>` uses to register the code plugin's hooks.
pub fn start_code_kg_dispatcher(store: Arc<Mutex<RedCoreGraphStore>>) -> HookDispatcher {
    let dispatcher = HookDispatcher::start(
        Arc::clone(&store),
        code_kg_hooks(),
        HookDispatcherConfig::default(),
    );
    if let Ok(mut guard) = store.lock() {
        guard.attach_hook_emitter(dispatcher.emitter());
    }
    dispatcher
}

/// Whether `CodeIndexRuntime` auto-starts the code-KG hooks. Default off so a
/// deploy is a deliberate flag flip, not a silent behavior change. Truthy:
/// `1`/`true`/`on`/`yes`.
fn code_hooks_enabled() -> bool {
    std::env::var("THEOREM_CODE_HOOKS")
        .map(|raw| {
            matches!(
                raw.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "on" | "yes"
            )
        })
        .unwrap_or(false)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodePluginOperation {
    pub operation: &'static str,
    pub command: &'static str,
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub writes_graph: bool,
}

pub fn builtin_code_plugin_registry() -> PluginRegistry {
    let mut registry = PluginRegistry::new();
    registry.register(CodeParsingPlugin);
    registry
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodePluginManifest {
    pub name: &'static str,
    pub source: &'static str,
    pub labels: &'static [&'static str],
    pub edge_types: &'static [&'static str],
    pub capabilities: Vec<PluginCapability>,
    pub operations: Vec<CodePluginOperation>,
}

#[derive(Clone, Debug, Default)]
pub struct CodeParsingPlugin;

impl CodeParsingPlugin {
    pub fn manifest(&self) -> CodePluginManifest {
        let operations = self
            .operations()
            .into_iter()
            .map(|registration| CodePluginOperation {
                operation: registration.operation,
                command: registration.command,
                aliases: registration.aliases,
                summary: registration.summary,
                writes_graph: registration.writes_graph,
            })
            .collect();
        CodePluginManifest {
            name: self.name(),
            source: SOURCE,
            labels: &[
                CODE_REPO_LABEL,
                CODE_FILE_LABEL,
                CODE_SYMBOL_LABEL,
                CODE_SYMBOL_NAME_LABEL,
                CODE_RECEIPT_LABEL,
                CODE_SPEC_LABEL,
                CODE_COMPILER_DRIFT_LABEL,
                CODE_PROCESS_LABEL,
                CODE_PATTERN_LABEL,
                CODE_FEATURE_LABEL,
                CODE_ANNOTATION_LABEL,
                CODE_BURST_JOB_LABEL,
                CODE_BURST_ARTIFACT_LABEL,
                EPISTEMIC_SHADOW_LABEL,
            ],
            edge_types: &[
                CONTAINS_FILE,
                DECLARES_SYMBOL,
                CALLS_SYMBOL,
                DEPENDS_ON_SYMBOL,
                SPECIFIES_CODE,
                DRIFT_FOR_SPEC,
                DRIFT_FOR_CODE,
                PROCESS_ENTRYPOINT,
                PROCESS_TOUCHES_CODE,
                PATTERN_APPLIES_TO_CODE,
                FEATURE_SOURCE_CODE,
                FEATURE_TARGET_CODE,
                ANNOTATES_CODE_FEATURE,
                BURST_PRODUCED_ARTIFACT,
                HAS_EPISTEMIC_SHADOW,
                UNDERCUTS,
                EPISTEMIC_SUPPORTS,
                SAME_ECLASS,
            ],
            capabilities: self.capabilities(),
            operations,
        }
    }

    pub fn manifest_json(&self) -> Value {
        let manifest = self.manifest();
        json!({
            "name": manifest.name,
            "source": manifest.source,
            "labels": manifest.labels,
            "edge_types": manifest.edge_types,
            "capabilities": manifest.capabilities.iter().map(|capability| {
                json!({
                    "kind": match capability.kind {
                        PluginCapabilityKind::Designation => "designation",
                        PluginCapabilityKind::Encoder => "encoder",
                        PluginCapabilityKind::Index => "index",
                        PluginCapabilityKind::Operation => "operation",
                        PluginCapabilityKind::Hook => "hook",
                    },
                    "name": capability.name,
                })
            }).collect::<Vec<_>>(),
            "operations": manifest.operations.iter().map(|operation| {
                json!({
                    "operation": operation.operation,
                    "command": operation.command,
                    "aliases": operation.aliases,
                    "summary": operation.summary,
                    "writes_graph": operation.writes_graph,
                })
            }).collect::<Vec<_>>(),
        })
    }
}

impl RustyRedPlugin for CodeParsingPlugin {
    fn name(&self) -> &'static str {
        "rustyred.thg.code"
    }

    fn capabilities(&self) -> Vec<PluginCapability> {
        vec![
            PluginCapability {
                kind: PluginCapabilityKind::Encoder,
                name: "code.encoder.tree_sitter_tags".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Encoder,
                name: "code.encoder.rust_syn".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Encoder,
                name: "code.encoder.line_symbol".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Index,
                name: "code.graph.symbol_index".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Operation,
                name: "code.graph.operations".to_string(),
            },
            PluginCapability {
                kind: PluginCapabilityKind::Hook,
                name: "code.graph.tenant_store_commit".to_string(),
            },
        ]
    }

    /// Graph-level hooks: warm centrality + fresh embeddings on code-graph
    /// mutations. Realizes the previously-inert `code.graph.tenant_store_commit`
    /// Hook capability.
    fn hooks(&self) -> Vec<HookRegistration> {
        code_kg_hooks()
    }

    fn operations(&self) -> Vec<PluginOperationRegistration> {
        vec![
            PluginOperationRegistration {
                operation: "ingest",
                command: "RUSTYRED_THG.CODE.INGEST",
                aliases: &["code.ingest", "rustyred.thg.code.ingest", "RUSTYRED.CODE.INGEST"],
                summary: "Parse a repository and commit CodeRepository, CodeFile, and CodeSymbol records into a tenant RedCore graph.",
                writes_graph: true,
                handler: handle_ingest_code_operation,
            },
            PluginOperationRegistration {
                operation: "reindex",
                command: "RUSTYRED_THG.CODE.REINDEX",
                aliases: &[
                    "code.reindex",
                    "rustyred.thg.code.reindex",
                    "RUSTYRED.CODE.REINDEX",
                ],
                summary: "Refresh a repository code graph generation in a tenant RedCore graph.",
                writes_graph: true,
                handler: handle_ingest_code_operation,
            },
            PluginOperationRegistration {
                operation: "search",
                command: "RUSTYRED_THG.CODE.SEARCH",
                aliases: &["code.search", "rustyred.thg.code.search", "RUSTYRED.CODE.SEARCH"],
                summary: "Search latest-generation CodeSymbol records in the tenant graph.",
                writes_graph: false,
                handler: handle_search_code_operation,
            },
            PluginOperationRegistration {
                operation: "context_pack",
                command: "RUSTYRED_THG.CODE.CONTEXT_PACK",
                aliases: &[
                    "code.context_pack",
                    "rustyred.thg.code.context_pack",
                    "RUSTYRED.CODE.CONTEXT_PACK",
                ],
                summary: "Generate CodeSymbol candidates, rerank them with the membrane scorer, and return a budgeted Admission.",
                writes_graph: true,
                handler: handle_context_pack_code_operation,
            },
            PluginOperationRegistration {
                operation: "code_ingest_ensure",
                command: "RUSTYRED_THG.CODE.ENSURE",
                aliases: &[
                    "code.ensure",
                    "code_ingest_ensure",
                    "ensure_repo_kg",
                    "rustyred.thg.code.ensure",
                    "RUSTYRED.CODE.ENSURE",
                ],
                summary: "SHA-keyed idempotent entry into a repo code graph: load the snapshot at the current sha, incrementally reindex a changed sha, or full-ingest an unknown repo.",
                writes_graph: true,
                handler: handle_ensure_code_operation,
            },
            PluginOperationRegistration {
                operation: "context",
                command: "RUSTYRED_THG.CODE.CONTEXT",
                aliases: &[
                    "code.context",
                    "rustyred.thg.code.context",
                    "RUSTYRED.CODE.CONTEXT",
                ],
                summary: "Return file context and sibling symbols for a CodeFile or CodeSymbol.",
                writes_graph: false,
                handler: handle_context_code_operation,
            },
            PluginOperationRegistration {
                operation: "recognize",
                command: "RUSTYRED_THG.CODE.RECOGNIZE",
                aliases: &[
                    "code.recognize",
                    "rustyred.thg.code.recognize",
                    "RUSTYRED.CODE.RECOGNIZE",
                ],
                summary: "Recognize indexed or inline code symbols.",
                writes_graph: false,
                handler: handle_recognize_code_operation,
            },
            PluginOperationRegistration {
                operation: "explore",
                command: "RUSTYRED_THG.CODE.EXPLORE",
                aliases: &[
                    "code.explore",
                    "rustyred.thg.code.explore",
                    "RUSTYRED.CODE.EXPLORE",
                ],
                summary: "Traverse calls and dependencies from a focused CodeSymbol.",
                writes_graph: false,
                handler: handle_explore_code_operation,
            },
            PluginOperationRegistration {
                operation: "explain",
                command: "RUSTYRED_THG.CODE.EXPLAIN",
                aliases: &[
                    "code.explain",
                    "rustyred.thg.code.explain",
                    "RUSTYRED.CODE.EXPLAIN",
                ],
                summary: "Explain a focused CodeSymbol using context and graph edges.",
                writes_graph: false,
                handler: handle_explain_code_operation,
            },
            PluginOperationRegistration {
                operation: "record_use_receipt",
                command: "RUSTYRED_THG.CODE.RECORD_USE_RECEIPT",
                aliases: &[
                    "code.record_use_receipt",
                    "rustyred.thg.code.record_use_receipt",
                    "RUSTYRED.CODE.RECORD_USE_RECEIPT",
                ],
                summary: "Record an agent use receipt for a code graph node.",
                writes_graph: true,
                handler: handle_record_use_receipt_code_operation,
            },
            PluginOperationRegistration {
                operation: "list_repos",
                command: "RUSTYRED_THG.CODE.LIST_REPOS",
                aliases: &[
                    "code.list_repos",
                    "rustyred.thg.code.list_repos",
                    "RUSTYRED.CODE.LIST_REPOS",
                ],
                summary: "List the code repositories indexed in the tenant with per-repo file and symbol counts.",
                writes_graph: false,
                handler: handle_list_repos_code_operation,
            },
            PluginOperationRegistration {
                operation: "kg_status",
                command: "RUSTYRED_THG.CODE.KG_STATUS",
                aliases: &[
                    "code.kg_status",
                    "rustyred.thg.code.kg_status",
                    "RUSTYRED.CODE.KG_STATUS",
                ],
                summary: "Report the code-graph-backed Instant KG base and manifest for a repo.",
                writes_graph: false,
                handler: handle_kg_status_code_operation,
            },
        ]
    }
}

impl CodeIndexRuntime {
    pub fn try_new() -> Result<Self, CodeIndexError> {
        Self::try_new_at(code_index_data_dir(), code_index_options())
    }

    pub fn try_new_with_credential_resolver(
        credential_resolver: Arc<dyn GitCredentialResolver>,
    ) -> Result<Self, CodeIndexError> {
        Self::try_new_at_with_credential_resolver(
            code_index_data_dir(),
            code_index_options(),
            credential_resolver,
        )
    }

    pub fn try_new_at(
        data_dir: impl AsRef<Path>,
        options: RedCoreOptions,
    ) -> Result<Self, CodeIndexError> {
        let store = RedCoreGraphStore::open(data_dir.as_ref(), options)
            .map_err(CodeIndexError::from_store)?;
        Self::try_new_with_store(store)
    }

    pub fn try_new_at_with_credential_resolver(
        data_dir: impl AsRef<Path>,
        options: RedCoreOptions,
        credential_resolver: Arc<dyn GitCredentialResolver>,
    ) -> Result<Self, CodeIndexError> {
        let store = RedCoreGraphStore::open(data_dir.as_ref(), options)
            .map_err(CodeIndexError::from_store)?;
        Self::try_new_with_store_and_credential_resolver(store, credential_resolver)
    }

    pub fn try_new_with_integrations(
        credential_resolver: Option<Arc<dyn GitCredentialResolver>>,
        map_projection_sink: Option<Arc<dyn CodebaseMapProjectionSink>>,
    ) -> Result<Self, CodeIndexError> {
        let store = RedCoreGraphStore::open(code_index_data_dir(), code_index_options())
            .map_err(CodeIndexError::from_store)?;
        Self::try_new_with_store_integrations(store, credential_resolver, map_projection_sink)
    }

    pub fn try_new_with_store(store: RedCoreGraphStore) -> Result<Self, CodeIndexError> {
        Self::build(store, code_hooks_enabled(), None, None)
    }

    pub fn try_new_with_store_and_credential_resolver(
        store: RedCoreGraphStore,
        credential_resolver: Arc<dyn GitCredentialResolver>,
    ) -> Result<Self, CodeIndexError> {
        Self::build(store, code_hooks_enabled(), Some(credential_resolver), None)
    }

    pub fn try_new_with_store_and_map_projection_sink(
        store: RedCoreGraphStore,
        map_projection_sink: Arc<dyn CodebaseMapProjectionSink>,
    ) -> Result<Self, CodeIndexError> {
        Self::build(store, code_hooks_enabled(), None, Some(map_projection_sink))
    }

    pub fn try_new_with_store_integrations(
        store: RedCoreGraphStore,
        credential_resolver: Option<Arc<dyn GitCredentialResolver>>,
        map_projection_sink: Option<Arc<dyn CodebaseMapProjectionSink>>,
    ) -> Result<Self, CodeIndexError> {
        Self::build(
            store,
            code_hooks_enabled(),
            credential_resolver,
            map_projection_sink,
        )
    }

    /// Like [`Self::try_new_with_store`], but always starts the graph-level
    /// code-KG hooks regardless of the `THEOREM_CODE_HOOKS` env flag. For
    /// embedders/tests that opt in explicitly.
    pub fn try_new_with_store_hooked(store: RedCoreGraphStore) -> Result<Self, CodeIndexError> {
        Self::build(store, true, None, None)
    }

    fn build(
        store: RedCoreGraphStore,
        with_hooks: bool,
        credential_resolver: Option<Arc<dyn GitCredentialResolver>>,
        map_projection_sink: Option<Arc<dyn CodebaseMapProjectionSink>>,
    ) -> Result<Self, CodeIndexError> {
        let store = Arc::new(Mutex::new(store));
        let jobs = Arc::new(IngestJobRegistry::with_persistence(Arc::downgrade(&store)));
        // Start the hook dispatcher before recovery so any re-enqueued ingest's
        // commits are observed. No-op for stores without hooks enabled.
        let hook_dispatcher = if with_hooks {
            Some(Arc::new(start_code_kg_dispatcher(Arc::clone(&store))))
        } else {
            None
        };
        let runtime = Self {
            store,
            jobs,
            worker_started: Arc::new(std::sync::Once::new()),
            credential_resolver,
            map_projection_sink,
            hook_dispatcher,
        };
        // D-jobs: recover durably-mirrored ingest jobs. Terminal jobs become
        // queryable again; jobs interrupted mid-flight (queued/running at the
        // crash) are re-enqueued and the worker is started to run them.
        let runnable = {
            let store = runtime.lock_store()?;
            runtime.jobs.recover_from_store(&store)
        };
        if runnable > 0 {
            runtime.ensure_ingest_worker();
        }
        Ok(runtime)
    }

    pub fn ingest_codebase(
        &self,
        input: IngestCodebaseInput,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        self.run_codebase_ingest(input, "ingest", None)
    }

    pub fn ingest_codebase_from_url(
        &self,
        url: &str,
        input: IngestCodebaseInput,
        caps: &RepoFetchCaps,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        self.run_codebase_ingest(input, "ingest", Some((url, caps)))
    }

    pub fn reindex_codebase(
        &self,
        input: IngestCodebaseInput,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        self.run_codebase_ingest(input, "reindex", None)
    }

    pub fn reindex_codebase_from_url(
        &self,
        url: &str,
        input: IngestCodebaseInput,
        caps: &RepoFetchCaps,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        self.run_codebase_ingest(input, "reindex", Some((url, caps)))
    }

    /// Synchronous ingest/reindex. The clone, walk, and parse run with NO
    /// store lock; the lock is taken twice, briefly: once to snapshot the
    /// prior generation (reindex only) and once for the final commit. The
    /// async submit path (`submit_ingest_job`) follows the same shape on the
    /// worker thread.
    fn run_codebase_ingest(
        &self,
        input: IngestCodebaseInput,
        operation: &str,
        url: Option<(&str, &RepoFetchCaps)>,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let started = Instant::now();
        let is_local_worktree = url.is_none();
        let (input, clone_ms, _fetched) = stage_repo_for_ingest(input, url)?;
        let resolve_started = Instant::now();
        let config = resolve_ingest_config(input)?;
        let resolve_ms = elapsed_ms(resolve_started);
        let projection_tenant_id = config.tenant_id.clone();
        let projection_repo_id = config.repo_id.clone();
        let projection_repo_path = is_local_worktree.then(|| config.repo_root.clone());
        let prior = {
            let store = self.lock_store()?;
            snapshot_for_operation(&store, operation, &config)?
        };
        let tenant_id = config.tenant_id.clone();
        let store_handle = Arc::clone(&self.store);
        let loader = move |hashes: &[String]| match store_handle.lock() {
            Ok(store) => load_file_texts(&store, &tenant_id, hashes),
            Err(_) => HashMap::new(),
        };
        let prepared = prepare_codebase_ingest_resolved(
            config,
            clone_ms,
            resolve_ms,
            started,
            IngestPipelineOptions {
                carried_text_loader: Some(&loader),
                ..IngestPipelineOptions::sync_default(prior)
            },
        )?;
        let mut store = self.lock_store()?;
        let output = commit_prepared_ingest(&mut store, prepared, operation)?;
        map_projection::publish_codebase_map_projection(
            &store,
            self.map_projection_sink.as_deref(),
            &projection_tenant_id,
            &projection_repo_id,
            operation,
            projection_repo_path.as_deref(),
        );
        Ok(output)
    }

    /// D1: submit an ingest/reindex as a background job and return its status
    /// (with `job_id`) immediately. The heavy path runs on a dedicated worker
    /// thread; watch it with `wait_ingest_job_events` (streaming) or poll
    /// `ingest_job_status`.
    pub fn submit_ingest_job(&self, request: IngestJobRequest) -> IngestJobStatus {
        self.ensure_ingest_worker();
        self.jobs.submit(request)
    }

    pub fn ingest_job_status(&self, job_id: &str) -> Option<IngestJobStatus> {
        self.jobs.status(job_id)
    }

    /// Events with `sequence > after_sequence`, blocking up to `timeout` for
    /// new ones. Returns `None` for an unknown job; the bool is true once the
    /// job reached a terminal state (no further events will arrive).
    pub fn wait_ingest_job_events(
        &self,
        job_id: &str,
        after_sequence: u64,
        timeout: std::time::Duration,
    ) -> Option<(Vec<IngestJobEvent>, bool)> {
        self.jobs.wait_events(job_id, after_sequence, timeout)
    }

    pub fn ingest_jobs(&self) -> Arc<IngestJobRegistry> {
        Arc::clone(&self.jobs)
    }

    fn ensure_ingest_worker(&self) {
        let store = Arc::downgrade(&self.store);
        let registry = Arc::clone(&self.jobs);
        let credential_resolver = self.credential_resolver.clone();
        let map_projection_sink = self.map_projection_sink.clone();
        self.worker_started.call_once(move || {
            if let Err(error) = std::thread::Builder::new()
                .name("code-ingest-worker".to_string())
                .spawn(move || {
                    ingest_jobs::ingest_worker_loop(
                        store,
                        registry,
                        credential_resolver,
                        map_projection_sink,
                    )
                })
            {
                // The registry stays usable; submitted jobs will sit queued.
                // A second runtime clone cannot retry (Once), so surface it.
                eprintln!("code-ingest-worker spawn failed: {error}");
            }
        });
    }

    pub fn search_code(&self, input: SearchCodeInput) -> Result<SearchCodeOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        search_code_with_store(&mut store, input)
    }

    pub fn context_pack(
        &self,
        input: CodeContextPackInput,
    ) -> Result<CodeContextPackOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        code_context_pack_in_store(&mut store, input)
    }

    pub fn list_repos(&self, input: ListReposInput) -> Result<ListReposOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        list_repos_in_store(&mut store, input)
    }

    pub fn code_kg_status(
        &self,
        input: CodeKgStatusInput,
    ) -> Result<CodeKgStatusOutput, CodeIndexError> {
        let store = self.lock_store()?;
        Ok(code_kg_status_in_store(&store, input))
    }

    /// AM2 bridge accessor: the code-graph-backed base snapshot for a repo, for
    /// callers (theorem-grpc session_reingest / context_pack) that overlay a
    /// `SessionDelta` on it via `HarnessInstantKg`.
    pub fn code_graph_snapshot(
        &self,
        tenant_id: &str,
        repo_id: &str,
    ) -> Result<GraphSnapshot, CodeIndexError> {
        let store = self.lock_store()?;
        Ok(code_graph_snapshot_in_store(&store, tenant_id, repo_id))
    }

    /// Commit graph enrichments that are anchored to the code index, such as
    /// GitHub collaboration nodes that link to `CodeFile` and `CodeSymbol`.
    pub fn commit_graph_mutations(
        &self,
        mutations: Vec<GraphMutation>,
    ) -> Result<u64, CodeIndexError> {
        if mutations.is_empty() {
            let store = self.lock_store()?;
            return Ok(GraphStore::stats(&*store).version);
        }
        let mut store = self.lock_store()?;
        let transaction = store
            .commit_batch(GraphMutationBatch::new(mutations))
            .map_err(CodeIndexError::from_store)?;
        Ok(transaction.graph_version)
    }

    pub fn query_graph_nodes(&self, query: NodeQuery) -> Result<Vec<NodeRecord>, CodeIndexError> {
        let store = self.lock_store()?;
        Ok(GraphStore::query_nodes(&*store, query))
    }

    pub fn graph_node_exists(&self, node_id: &str) -> Result<bool, CodeIndexError> {
        let store = self.lock_store()?;
        Ok(GraphStore::get_node(&*store, node_id).is_some())
    }

    pub fn graph_snapshot(&self) -> Result<GraphSnapshot, CodeIndexError> {
        let store = self.lock_store()?;
        GraphStore::graph_snapshot(&*store).map_err(CodeIndexError::from_store)
    }

    pub fn code_context(
        &self,
        input: CodeContextInput,
    ) -> Result<CodeContextOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        code_context_with_store(&mut store, input)
    }

    pub fn recognize_code(
        &self,
        input: RecognizeCodeInput,
    ) -> Result<RecognizeCodeOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        recognize_code_with_store(&mut store, input)
    }

    pub fn explore_code(
        &self,
        input: ExploreCodeInput,
    ) -> Result<ExploreCodeOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        explore_code_with_store(&mut store, input)
    }

    pub fn explain_code(
        &self,
        input: ExplainCodeInput,
    ) -> Result<ExplainCodeOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        explain_code_with_store(&mut store, input)
    }

    pub fn record_use_receipt(
        &self,
        input: RecordUseReceiptInput,
    ) -> Result<RecordUseReceiptOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        record_use_receipt_with_store(&mut store, input)
    }

    fn lock_store(&self) -> Result<std::sync::MutexGuard<'_, RedCoreGraphStore>, CodeIndexError> {
        self.store.lock().map_err(|_| CodeIndexError {
            code: "code_index_lock_poisoned".to_string(),
            message: "code index RedCore store lock poisoned".to_string(),
        })
    }
}

pub fn ingest_codebase_in_store(
    store: &mut RedCoreGraphStore,
    input: IngestCodebaseInput,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    run_codebase_ingest_in_store(store, input, "ingest", None)
}

pub fn reindex_codebase_in_store(
    store: &mut RedCoreGraphStore,
    input: IngestCodebaseInput,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    run_codebase_ingest_in_store(store, input, "reindex", None)
}

/// CA-1: ingest a repository given a remote URL. Shallow-clones it into a
/// quarantined tempdir (removed after ingest), then runs the normal in-store
/// ingest over the local checkout. The clone's `.git` dir is always excluded,
/// and `repo_id` defaults to a slug derived from the URL when not supplied.
pub fn ingest_codebase_from_url_in_store(
    store: &mut RedCoreGraphStore,
    url: &str,
    input: IngestCodebaseInput,
    caps: &RepoFetchCaps,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    run_codebase_ingest_in_store(store, input, "ingest", Some((url, caps)))
}

pub fn reindex_codebase_from_url_in_store(
    store: &mut RedCoreGraphStore,
    url: &str,
    input: IngestCodebaseInput,
    caps: &RepoFetchCaps,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    run_codebase_ingest_in_store(store, input, "reindex", Some((url, caps)))
}

fn run_codebase_ingest_in_store(
    store: &mut RedCoreGraphStore,
    input: IngestCodebaseInput,
    operation: &str,
    url: Option<(&str, &RepoFetchCaps)>,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    let started = Instant::now();
    let (input, clone_ms, _fetched) = stage_repo_for_ingest(input, url)?;
    let resolve_started = Instant::now();
    let config = resolve_ingest_config(input)?;
    let resolve_ms = elapsed_ms(resolve_started);
    let prior = snapshot_for_operation(store, operation, &config)?;
    // Carried-text loads read the same `&store` immutably; the borrow ends when
    // prepare returns, before the `&mut store` commit below.
    let prepared = {
        let tenant_id = config.tenant_id.clone();
        let store_ref: &RedCoreGraphStore = store;
        let loader = |hashes: &[String]| load_file_texts(store_ref, &tenant_id, hashes);
        prepare_codebase_ingest_resolved(
            config,
            clone_ms,
            resolve_ms,
            started,
            IngestPipelineOptions {
                carried_text_loader: Some(&loader),
                ..IngestPipelineOptions::sync_default(prior)
            },
        )?
    };
    commit_prepared_ingest(store, prepared, operation)
}

/// Stage the repository for ingest. With a URL, shallow-clone it (CA-1) and
/// point the input at the quarantined checkout; the returned `FetchedRepo`
/// handle must stay alive until parsing has read all file text, after which
/// dropping it removes the clone.
pub fn stage_repo_for_ingest(
    input: IngestCodebaseInput,
    url: Option<(&str, &RepoFetchCaps)>,
) -> Result<(IngestCodebaseInput, u64, Option<FetchedRepo>), CodeIndexError> {
    stage_repo_for_ingest_with_credential(input, url, None)
}

/// Stage a repository for ingest or workspace import. With a URL, this
/// shallow-clones through the existing credential-aware fetch path and rewrites
/// `input.repo_path` to the temporary checkout; with `None`, it returns the
/// local input unchanged.
pub fn stage_repo_for_ingest_with_credential(
    mut input: IngestCodebaseInput,
    url: Option<(&str, &RepoFetchCaps)>,
    credential: Option<&GitCredential>,
) -> Result<(IngestCodebaseInput, u64, Option<FetchedRepo>), CodeIndexError> {
    let Some((url, caps)) = url else {
        return Ok((input, 0, None));
    };
    let clone_started = Instant::now();
    let fetched = fetch_repo_with_credential(url, caps, credential)
        .map_err(|err| CodeIndexError::invalid(err.to_string()))?;
    let clone_ms = elapsed_ms(clone_started);
    input.repo_path = fetched.path().display().to_string();
    if !input.exclude_dirs.iter().any(|dir| dir == ".git") {
        input.exclude_dirs.push(".git".to_string());
    }
    if input.repo_id.trim().is_empty() {
        input.repo_id = repo_id_from_url(url);
    }
    Ok((input, clone_ms, Some(fetched)))
}

/// Best-effort stable repo id from a clone URL: `repo:<last-path-segment>`.
fn repo_id_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let slug = trimmed.rsplit(['/', ':']).next().filter(|s| !s.is_empty());
    format!("repo:{}", slug.unwrap_or("repo"))
}

pub fn search_code_in_store(
    store: &mut RedCoreGraphStore,
    input: SearchCodeInput,
) -> Result<SearchCodeOutput, CodeIndexError> {
    search_code_with_store(store, input)
}

/// Fix-handoff F8: a read-only inventory of the code repos indexed in a tenant.
/// Answers "what repos are indexed here" without a `job_id`. Per repo it reports
/// the latest generation, the file/symbol counts at that generation, and the
/// last `indexed_at_ms`. `repo_url`/`head_sha` come from the `CodeRepository`
/// node when the AM6 manifest registry recorded them.
pub fn list_repos_in_store(
    store: &mut RedCoreGraphStore,
    input: ListReposInput,
) -> Result<ListReposOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);

    let repo_nodes = store
        .query_nodes(NodeQuery::label(CODE_REPO_LABEL).with_limit(100_000))
        .map_err(CodeIndexError::from_store)?;
    let mut summaries: HashMap<String, RepoSummary> = HashMap::new();
    for node in repo_nodes {
        if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_id.as_str()) {
            continue;
        }
        let Some(repo_id) = property_string(&node.properties, "repo_id") else {
            continue;
        };
        let latest_generation = property_u64(&node.properties, "latest_generation").unwrap_or(0);
        summaries.insert(
            repo_id.clone(),
            RepoSummary {
                repo_id,
                repo_url: property_string(&node.properties, "repo_url").unwrap_or_default(),
                head_sha: property_string(&node.properties, "head_sha").unwrap_or_default(),
                latest_generation,
                indexed_at_ms: property_u64(&node.properties, "indexed_at_ms")
                    .unwrap_or(latest_generation),
                repo_root: property_string(&node.properties, "repo_root").unwrap_or_default(),
                files_indexed: 0,
                symbols_indexed: 0,
            },
        );
    }

    let latest: HashMap<String, u64> = summaries
        .iter()
        .map(|(id, summary)| (id.clone(), summary.latest_generation))
        .collect();
    let file_counts = count_code_nodes_by_repo(store, &tenant_id, CODE_FILE_LABEL, &latest)?;
    let symbol_counts = count_code_nodes_by_repo(store, &tenant_id, CODE_SYMBOL_LABEL, &latest)?;
    for (repo_id, summary) in summaries.iter_mut() {
        summary.files_indexed = file_counts.get(repo_id).copied().unwrap_or(0);
        summary.symbols_indexed = symbol_counts.get(repo_id).copied().unwrap_or(0);
    }

    let mut repos: Vec<RepoSummary> = summaries.into_values().collect();
    repos.sort_by(|a, b| a.repo_id.cmp(&b.repo_id));
    let total_repos = repos.len() as u64;

    let receipt_payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_list_repos",
        "total_repos": total_repos,
        "repo_ids": repos.iter().map(|r| r.repo_id.clone()).collect::<Vec<_>>(),
    });
    let receipt = record_receipt(store, &tenant_id, "code_list_repos", &receipt_payload)?;

    Ok(ListReposOutput {
        tenant_id,
        repos,
        total_repos,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

/// Count code nodes of `label` per repo, tenant-scoped and limited to each
/// repo's current generation (so stale generations do not inflate the count,
/// matching `search_code_with_store`'s generation gate).
fn count_code_nodes_by_repo(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    label: &str,
    latest: &HashMap<String, u64>,
) -> Result<HashMap<String, u64>, CodeIndexError> {
    let nodes = store
        .query_nodes(NodeQuery::label(label).with_limit(100_000))
        .map_err(CodeIndexError::from_store)?;
    let mut counts: HashMap<String, u64> = HashMap::new();
    for node in nodes {
        if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_id) {
            continue;
        }
        let Some(repo_id) = property_string(&node.properties, "repo_id") else {
            continue;
        };
        let Some(&generation) = latest.get(&repo_id) else {
            continue;
        };
        if property_u64(&node.properties, "generation") != Some(generation) {
            continue;
        }
        *counts.entry(repo_id).or_insert(0) += 1;
    }
    Ok(counts)
}

/// AM2 bridge: build the base `GraphSnapshot` for `HarnessInstantKg` from the
/// tenant code graph filtered to `repo_id` at its latest generation. This is the
/// seam that connects the CodeCrawler code graph to the Instant KG surface, so a
/// session delta overlays the real indexed base rather than an empty graph. Edges
/// are kept only when both endpoints survive the node filter.
pub fn code_graph_snapshot_in_store(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
) -> GraphSnapshot {
    let tenant = normalize_tenant(tenant_id);
    let repo_id = repo_id.trim();
    let snapshot = store.graph_snapshot();

    let latest_generation = snapshot
        .nodes
        .iter()
        .filter(|node| node.labels.iter().any(|label| label == CODE_REPO_LABEL))
        .filter(|node| {
            node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant.as_str())
                && node.properties.get("repo_id").and_then(Value::as_str) == Some(repo_id)
        })
        .filter_map(|node| property_u64(&node.properties, "latest_generation"))
        .max();

    let mut nodes: Vec<NodeRecord> = Vec::new();
    for node in &snapshot.nodes {
        if node.tombstone {
            continue;
        }
        let is_code = node.labels.iter().any(|label| {
            label == CODE_REPO_LABEL || label == CODE_FILE_LABEL || label == CODE_SYMBOL_LABEL
        });
        if !is_code {
            continue;
        }
        if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant.as_str()) {
            continue;
        }
        if node.properties.get("repo_id").and_then(Value::as_str) != Some(repo_id) {
            continue;
        }
        let is_repo = node.labels.iter().any(|label| label == CODE_REPO_LABEL);
        if !is_repo {
            // File/symbol nodes are gated to the repo's current generation.
            if let Some(latest) = latest_generation {
                if property_u64(&node.properties, "generation") != Some(latest) {
                    continue;
                }
            }
        }
        nodes.push(node.clone());
    }

    let node_ids: HashSet<&str> = nodes.iter().map(|node| node.id.as_str()).collect();
    let edges: Vec<EdgeRecord> = snapshot
        .edges
        .iter()
        .filter(|edge| !edge.tombstone)
        .filter(|edge| {
            node_ids.contains(edge.from_id.as_str()) && node_ids.contains(edge.to_id.as_str())
        })
        .cloned()
        .collect();

    GraphSnapshot {
        version: snapshot.version,
        nodes,
        edges,
    }
}

/// AM2/AM3: report the code-graph-backed Instant KG base for a repo plus a
/// `CodeKgManifest` provenance record. Read-only (no receipt write).
pub fn code_kg_status_in_store(
    store: &RedCoreGraphStore,
    input: CodeKgStatusInput,
) -> CodeKgStatusOutput {
    let tenant = normalize_tenant(&input.tenant_id);
    let repo_id = input.repo_id.trim().to_string();
    let base = code_graph_snapshot_in_store(store, &tenant, &repo_id);

    let repo_node = base
        .nodes
        .iter()
        .find(|node| node.labels.iter().any(|label| label == CODE_REPO_LABEL));
    let repo_url = repo_node
        .and_then(|node| property_string(&node.properties, "repo_url"))
        .unwrap_or_default();
    let head_sha = repo_node
        .and_then(|node| property_string(&node.properties, "head_sha"))
        .unwrap_or_default();
    let latest_generation = repo_node
        .and_then(|node| property_u64(&node.properties, "latest_generation"))
        .unwrap_or(0);
    let indexed_at_ms = repo_node
        .and_then(|node| property_u64(&node.properties, "indexed_at_ms"))
        .unwrap_or(latest_generation);

    let manifest = CodeKgManifest::from_base_snapshot(&repo_id, head_sha.clone(), &base);

    CodeKgStatusOutput {
        tenant_id: tenant,
        repo_id,
        repo_url,
        head_sha,
        base_objects: manifest.objects_total as u64,
        base_edges: manifest.edges_total as u64,
        base_graph_hash: manifest.base_graph_hash,
        encoder_version: manifest.encoder_version,
        ingest_version: manifest.ingest_version,
        latest_generation,
        indexed_at_ms,
        indexed: repo_node.is_some(),
    }
}

pub fn code_context_in_store(
    store: &mut RedCoreGraphStore,
    input: CodeContextInput,
) -> Result<CodeContextOutput, CodeIndexError> {
    code_context_with_store(store, input)
}

pub fn recognize_code_in_store(
    store: &mut RedCoreGraphStore,
    input: RecognizeCodeInput,
) -> Result<RecognizeCodeOutput, CodeIndexError> {
    recognize_code_with_store(store, input)
}

pub fn explore_code_in_store(
    store: &mut RedCoreGraphStore,
    input: ExploreCodeInput,
) -> Result<ExploreCodeOutput, CodeIndexError> {
    explore_code_with_store(store, input)
}

pub fn explain_code_in_store(
    store: &mut RedCoreGraphStore,
    input: ExplainCodeInput,
) -> Result<ExplainCodeOutput, CodeIndexError> {
    explain_code_with_store(store, input)
}

pub fn record_code_use_receipt_in_store(
    store: &mut RedCoreGraphStore,
    input: RecordUseReceiptInput,
) -> Result<RecordUseReceiptOutput, CodeIndexError> {
    record_use_receipt_with_store(store, input)
}

pub type CodePluginExecutionOutput = PluginExecutionOutput;

pub fn execute_code_plugin_operation(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    operation: &str,
    arguments: Value,
) -> Result<CodePluginExecutionOutput, CodeIndexError> {
    let registry = builtin_code_plugin_registry();
    registry
        .execute(store, tenant_id, operation, arguments)
        .map_err(CodeIndexError::from_store)
}

fn handle_ingest_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let repo_url = code_plugin_arg_string(&arguments, &["repo_url", "repoUrl", "url"]);
    let input = IngestCodebaseInput {
        tenant_id: context.tenant_id.to_string(),
        repo_path: code_plugin_arg_string(&arguments, &["repo_path", "repoPath"]),
        repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
        include_extensions: code_plugin_arg_string_vec(
            &arguments,
            &["include_extensions", "includeExtensions"],
        ),
        exclude_dirs: code_plugin_arg_string_vec(&arguments, &["exclude_dirs", "excludeDirs"]),
        max_files: code_plugin_arg_u64(&arguments, &["max_files", "maxFiles"]),
        max_file_bytes: code_plugin_arg_u64(&arguments, &["max_file_bytes", "maxFileBytes"]),
        max_total_bytes: code_plugin_arg_u64(
            &arguments,
            &[
                "max_total_bytes",
                "maxTotalBytes",
                "max_clone_bytes",
                "maxCloneBytes",
                "max_repo_bytes",
                "maxRepoBytes",
            ],
        ),
        materialize_symbol_name_index: arguments
            .get("materialize_symbol_name_index")
            .or_else(|| arguments.get("materializeSymbolNameIndex"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        actor: code_plugin_arg_string(&arguments, &["actor", "actor_id", "actorId"]),
    };
    let output = if !repo_url.trim().is_empty() {
        // CA-1: ingest by URL -> shallow clone into a quarantined tempdir ->
        // ingest the local checkout (the clone is removed afterward).
        let fetch_caps = RepoFetchCaps::from_requested(input.max_total_bytes);
        if context.operation == "reindex" {
            reindex_codebase_from_url_in_store(context.store, &repo_url, input, &fetch_caps)?
        } else {
            ingest_codebase_from_url_in_store(context.store, &repo_url, input, &fetch_caps)?
        }
    } else if context.operation == "reindex" {
        reindex_codebase_in_store(context.store, input)?
    } else {
        ingest_codebase_in_store(context.store, input)?
    };
    Ok(output.to_json())
}

fn handle_list_repos_code_operation(
    context: PluginOperationContext<'_>,
    _arguments: Value,
) -> GraphStoreResult<Value> {
    let output = list_repos_in_store(
        context.store,
        ListReposInput {
            tenant_id: context.tenant_id.to_string(),
        },
    )?;
    Ok(output.to_json())
}

fn handle_kg_status_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = code_kg_status_in_store(
        context.store,
        CodeKgStatusInput {
            tenant_id: context.tenant_id.to_string(),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
        },
    );
    Ok(output.to_json())
}

fn handle_search_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = search_code_in_store(
        context.store,
        SearchCodeInput {
            tenant_id: context.tenant_id.to_string(),
            query: code_plugin_arg_string(&arguments, &["query"]),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            path_prefix: code_plugin_arg_string(&arguments, &["path_prefix", "pathPrefix"]),
            kinds: code_plugin_arg_string_vec(&arguments, &["kinds"]),
            limit: code_plugin_arg_u64(&arguments, &["limit"]),
        },
    )?;
    Ok(output.to_json())
}

fn handle_context_pack_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    // Spec composition (Code arm + the /compute_code reflex): a context_pack that
    // carries a fetchable repo_url ensures the code graph is resident at `sha`
    // FIRST (SHA-keyed: snapshot load / incremental reindex / full ingest), then
    // gates. Without this, a one-call pack on a not-yet-ingested repo would
    // return an empty pack instead of ingesting it.
    let repo_url = code_plugin_arg_string(&arguments, &["repo_url", "repoUrl", "url"]);
    let mut repo_id = code_plugin_arg_string(&arguments, &["repo_id", "repoId"]);
    let mut ingest_status: Option<Value> = None;
    if is_fetchable_repo_url(repo_url.trim()) {
        let sha = code_plugin_arg_string(&arguments, &["sha", "head_sha", "headSha", "ref"]);
        let sha = if sha.trim().is_empty() {
            None
        } else {
            Some(sha.trim())
        };
        let supplied_repo_id = if repo_id.trim().is_empty() {
            None
        } else {
            Some(repo_id.trim())
        };
        let status = ensure_repo_kg_in_store(
            context.store,
            context.tenant_id,
            repo_url.trim(),
            sha,
            supplied_repo_id,
            &RepoFetchCaps::default(),
        )?;
        if repo_id.trim().is_empty() {
            repo_id = crate::ensure::repo_id_from_url(repo_url.trim());
        }
        ingest_status = Some(status.to_json());
    }
    let output = code_context_pack_in_store(
        context.store,
        CodeContextPackInput {
            tenant_id: context.tenant_id.to_string(),
            query: code_plugin_arg_string(&arguments, &["query", "task"]),
            repo_id,
            path_prefix: code_plugin_arg_string(&arguments, &["path_prefix", "pathPrefix"]),
            kinds: code_plugin_arg_string_vec(&arguments, &["kinds"]),
            limit: code_plugin_arg_u64(&arguments, &["limit"]),
            budget_tokens: code_plugin_arg_u64(&arguments, &["budget_tokens", "budgetTokens"]),
        },
    )?;
    let mut json = output.to_json();
    if let (Some(status), Some(object)) = (ingest_status, json.as_object_mut()) {
        object.insert("ingest_status".to_string(), status);
    }
    Ok(json)
}

fn handle_ensure_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let repo_url = code_plugin_arg_string(&arguments, &["repo_url", "repoUrl", "url"]);
    let sha = code_plugin_arg_string(&arguments, &["sha", "head_sha", "headSha", "ref"]);
    let sha = if sha.trim().is_empty() {
        None
    } else {
        Some(sha.trim())
    };
    let repo_id = code_plugin_arg_string(&arguments, &["repo_id", "repoId"]);
    let repo_id = if repo_id.trim().is_empty() {
        None
    } else {
        Some(repo_id.trim())
    };
    let status = ensure_repo_kg_in_store(
        context.store,
        context.tenant_id,
        &repo_url,
        sha,
        repo_id,
        &RepoFetchCaps::default(),
    )?;
    Ok(status.to_json())
}

fn handle_context_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = code_context_in_store(
        context.store,
        CodeContextInput {
            tenant_id: context.tenant_id.to_string(),
            node_id: code_plugin_arg_string(&arguments, &["node_id", "nodeId"]),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            file_path: code_plugin_arg_string(&arguments, &["file_path", "filePath"]),
            before_lines: code_plugin_arg_u64(&arguments, &["before_lines", "beforeLines"]),
            after_lines: code_plugin_arg_u64(&arguments, &["after_lines", "afterLines"]),
            max_chars: code_plugin_arg_u64(&arguments, &["max_chars", "maxChars"]),
        },
    )?;
    Ok(output.to_json())
}

fn handle_recognize_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = recognize_code_in_store(
        context.store,
        RecognizeCodeInput {
            tenant_id: context.tenant_id.to_string(),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            file_path: code_plugin_arg_string(&arguments, &["file_path", "filePath"]),
            text: code_plugin_arg_string(&arguments, &["text"]),
            limit: code_plugin_arg_u64(&arguments, &["limit"]),
        },
    )?;
    Ok(output.to_json())
}

fn handle_explore_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = explore_code_in_store(
        context.store,
        ExploreCodeInput {
            tenant_id: context.tenant_id.to_string(),
            node_id: code_plugin_arg_string(&arguments, &["node_id", "nodeId"]),
            query: code_plugin_arg_string(&arguments, &["query"]),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            max_depth: code_plugin_arg_u64(&arguments, &["max_depth", "maxDepth"]),
            limit: code_plugin_arg_u64(&arguments, &["limit"]),
        },
    )?;
    Ok(output.to_json())
}

fn handle_explain_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = explain_code_in_store(
        context.store,
        ExplainCodeInput {
            tenant_id: context.tenant_id.to_string(),
            node_id: code_plugin_arg_string(&arguments, &["node_id", "nodeId"]),
            query: code_plugin_arg_string(&arguments, &["query"]),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            max_chars: code_plugin_arg_u64(&arguments, &["max_chars", "maxChars"]),
        },
    )?;
    Ok(output.to_json())
}

fn handle_record_use_receipt_code_operation(
    context: PluginOperationContext<'_>,
    arguments: Value,
) -> GraphStoreResult<Value> {
    let output = record_code_use_receipt_in_store(
        context.store,
        RecordUseReceiptInput {
            tenant_id: context.tenant_id.to_string(),
            node_id: code_plugin_arg_string(&arguments, &["node_id", "nodeId"]),
            repo_id: code_plugin_arg_string(&arguments, &["repo_id", "repoId"]),
            query: code_plugin_arg_string(&arguments, &["query"]),
            action: code_plugin_arg_string(&arguments, &["action"]),
            outcome: code_plugin_arg_string(&arguments, &["outcome"]),
            actor: code_plugin_arg_string(&arguments, &["actor", "actor_id", "actorId"]),
            use_json: code_plugin_use_json(&arguments),
        },
    )?;
    Ok(output.to_json())
}

fn code_plugin_arg_string(arguments: &Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_str))
        .unwrap_or_default()
        .trim()
        .to_string()
}

fn code_plugin_arg_u64(arguments: &Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| arguments.get(*key).and_then(Value::as_u64))
        .unwrap_or(0)
}

fn code_plugin_arg_string_vec(arguments: &Value, keys: &[&str]) -> Vec<String> {
    let Some(value) = keys.iter().find_map(|key| arguments.get(*key)) else {
        return Vec::new();
    };
    if let Some(raw) = value.as_str() {
        return raw
            .split(',')
            .map(str::trim)
            .filter(|item| !item.is_empty())
            .map(ToString::to_string)
            .collect();
    }
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn code_plugin_use_json(arguments: &Value) -> String {
    arguments
        .get("use_json")
        .or_else(|| arguments.get("useJson"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            arguments
                .get("use")
                .map(|value| serde_json::to_string(value).unwrap_or_else(|_| "{}".to_string()))
        })
        .unwrap_or_default()
}

#[derive(Clone, Debug, Default)]
pub struct IngestCodebaseInput {
    pub tenant_id: String,
    pub repo_path: String,
    pub repo_id: String,
    pub include_extensions: Vec<String>,
    pub exclude_dirs: Vec<String>,
    pub max_files: u64,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
    /// W1 opt-in: materialize `CodeSymbolName` inverse-index bucket nodes for
    /// touched-name lookup. Kept off by default so existing full-ingest paths do
    /// not pay a per-unique-name write cost until the on-write lane enables it.
    pub materialize_symbol_name_index: bool,
    pub actor: String,
}

#[derive(Clone, Debug)]
pub struct IngestCodebaseOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub repo_root: String,
    pub generation: u64,
    pub files_indexed: u64,
    pub symbols_indexed: u64,
    pub files_skipped: u64,
    /// Files whose symbols were extracted in this pass.
    pub files_parsed: u64,
    /// D4: unchanged files whose prior-generation nodes were carried forward
    /// (restamped) without re-parsing. `files_indexed = parsed + carried`.
    pub files_carried: u64,
    pub graph_version: u64,
    pub receipt_hash: String,
    pub receipt_json: String,
    pub status: String,
    pub message: String,
    pub epistemic_readout: Value,
    pub stage_timings: IngestStageTimings,
    pub language_stats: BTreeMap<String, LanguageIngestStats>,
    pub skip_stats: IngestSkipStats,
}

#[derive(Clone, Debug)]
pub struct SourceFileWriteIndexInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub repo_root_display: String,
    pub file_path: String,
    pub content: Vec<u8>,
    pub actor: String,
    /// Optional explicit generation. `0` means "use now".
    pub generation: u64,
    pub materialize_symbol_name_index: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceFileWriteIndexOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub file_path: String,
    pub file_id: String,
    pub content_hash: String,
    pub generation: u64,
    pub graph_version: u64,
    pub symbols_indexed: u64,
    pub files_carried: u64,
    pub edges_indexed: u64,
    pub edges_retired: u64,
    pub bucket_lookups: u64,
    pub symbol_names: Vec<String>,
    pub bucket_names: Vec<String>,
}

impl IngestCodebaseOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "repo_root": self.repo_root,
            "generation": self.generation,
            "files_indexed": self.files_indexed,
            "symbols_indexed": self.symbols_indexed,
            "files_skipped": self.files_skipped,
            "files_parsed": self.files_parsed,
            "files_carried": self.files_carried,
            "graph_version": self.graph_version,
            "receipt_hash": self.receipt_hash,
            "status": self.status,
            "message": self.message,
            "epistemic_readout": self.epistemic_readout,
            "stage_timings": self.stage_timings.to_json(),
            "language_stats": language_stats_json(&self.language_stats),
            "skip_stats": self.skip_stats.to_json(),
        })
    }
}

/// Rebuild an `IngestCodebaseOutput` from the JSON shape `to_json` emits, used
/// to recover a durably-persisted finished job (D-jobs). `receipt_json` is the
/// one field `to_json` omits; a recovered output leaves it empty (the receipt
/// node itself is durable in the graph).
pub(crate) fn ingest_output_from_json(value: &Value) -> Option<IngestCodebaseOutput> {
    let u64_at = |key: &str| value.get(key).and_then(Value::as_u64).unwrap_or(0);
    let str_at = |key: &str| {
        value
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let timings = value.get("stage_timings").cloned().unwrap_or(json!({}));
    let timing = |key: &str| timings.get(key).and_then(Value::as_u64).unwrap_or(0);
    let skips = value.get("skip_stats").cloned().unwrap_or(json!({}));
    let skip = |key: &str| skips.get(key).and_then(Value::as_u64).unwrap_or(0);
    let mut language_stats = BTreeMap::new();
    if let Some(map) = value.get("language_stats").and_then(Value::as_object) {
        for (language, stats) in map {
            language_stats.insert(
                language.clone(),
                LanguageIngestStats {
                    files_indexed: stats
                        .get("files_indexed")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                    symbols_indexed: stats
                        .get("symbols_indexed")
                        .and_then(Value::as_u64)
                        .unwrap_or(0),
                },
            );
        }
    }
    Some(IngestCodebaseOutput {
        tenant_id: str_at("tenant_id"),
        repo_id: str_at("repo_id"),
        repo_root: str_at("repo_root"),
        generation: u64_at("generation"),
        files_indexed: u64_at("files_indexed"),
        symbols_indexed: u64_at("symbols_indexed"),
        files_skipped: u64_at("files_skipped"),
        files_parsed: u64_at("files_parsed"),
        files_carried: u64_at("files_carried"),
        graph_version: u64_at("graph_version"),
        receipt_hash: str_at("receipt_hash"),
        receipt_json: String::new(),
        status: str_at("status"),
        message: str_at("message"),
        epistemic_readout: value
            .get("epistemic_readout")
            .cloned()
            .unwrap_or_else(|| json!({})),
        stage_timings: IngestStageTimings {
            clone_ms: timing("clone_ms"),
            resolve_ms: timing("resolve_ms"),
            walk_ms: timing("walk_ms"),
            parse_ms: timing("parse_ms"),
            mutation_ms: timing("mutation_ms"),
            write_ms: timing("write_ms"),
            total_ms: timing("total_ms"),
        },
        language_stats,
        skip_stats: IngestSkipStats {
            unsupported_extension: skip("unsupported_extension"),
            filename: skip("filename"),
            too_large: skip("too_large"),
            binary: skip("binary"),
            read_error: skip("read_error"),
        },
    })
}

#[derive(Clone, Debug, Default)]
pub struct IngestStageTimings {
    pub clone_ms: u64,
    pub resolve_ms: u64,
    pub walk_ms: u64,
    pub parse_ms: u64,
    pub mutation_ms: u64,
    pub write_ms: u64,
    pub total_ms: u64,
}

impl IngestStageTimings {
    pub fn to_json(&self) -> Value {
        json!({
            "clone_ms": self.clone_ms,
            "resolve_ms": self.resolve_ms,
            "walk_ms": self.walk_ms,
            "parse_ms": self.parse_ms,
            "mutation_ms": self.mutation_ms,
            "write_ms": self.write_ms,
            "total_ms": self.total_ms,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct LanguageIngestStats {
    pub files_indexed: u64,
    pub symbols_indexed: u64,
}

#[derive(Clone, Debug, Default)]
pub struct IngestSkipStats {
    pub unsupported_extension: u64,
    pub filename: u64,
    pub too_large: u64,
    pub binary: u64,
    pub read_error: u64,
}

impl IngestSkipStats {
    pub fn total(&self) -> u64 {
        self.unsupported_extension + self.filename + self.too_large + self.binary + self.read_error
    }

    fn merge_from(&mut self, other: &IngestSkipStats) {
        self.unsupported_extension += other.unsupported_extension;
        self.filename += other.filename;
        self.too_large += other.too_large;
        self.binary += other.binary;
        self.read_error += other.read_error;
    }

    pub fn to_json(&self) -> Value {
        json!({
            "unsupported_extension": self.unsupported_extension,
            "filename": self.filename,
            "too_large": self.too_large,
            "binary": self.binary,
            "read_error": self.read_error,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SearchCodeInput {
    pub tenant_id: String,
    pub query: String,
    pub repo_id: String,
    pub path_prefix: String,
    pub kinds: Vec<String>,
    pub limit: u64,
}

#[derive(Clone, Debug)]
pub struct SearchCodeOutput {
    pub tenant_id: String,
    pub query: String,
    pub hits: Vec<CodeHitRecord>,
    pub total_admitted: u64,
    pub total_returned: u64,
    pub latency_ms: u64,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl SearchCodeOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "query": self.query,
            "hits": self.hits.iter().map(CodeHitRecord::to_json).collect::<Vec<_>>(),
            "total_admitted": self.total_admitted,
            "total_returned": self.total_returned,
            "latency_ms": self.latency_ms,
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ListReposInput {
    pub tenant_id: String,
}

/// One row of the tenant code-repo inventory (F8 `list_repos`). `repo_url` and
/// `head_sha` are populated by the AM6 manifest registry; empty when an ingest
/// predates it.
#[derive(Clone, Debug, Default)]
pub struct RepoSummary {
    pub repo_id: String,
    pub repo_url: String,
    pub head_sha: String,
    pub latest_generation: u64,
    pub files_indexed: u64,
    pub symbols_indexed: u64,
    pub indexed_at_ms: u64,
    pub repo_root: String,
}

impl RepoSummary {
    pub fn to_json(&self) -> Value {
        json!({
            "repo_id": self.repo_id,
            "repo_url": self.repo_url,
            "head_sha": self.head_sha,
            "latest_generation": self.latest_generation,
            "files_indexed": self.files_indexed,
            "symbols_indexed": self.symbols_indexed,
            "indexed_at_ms": self.indexed_at_ms,
            "repo_root": self.repo_root,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ListReposOutput {
    pub tenant_id: String,
    pub repos: Vec<RepoSummary>,
    pub total_repos: u64,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl ListReposOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repos": self.repos.iter().map(RepoSummary::to_json).collect::<Vec<_>>(),
            "total_repos": self.total_repos,
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct CodeKgStatusInput {
    pub tenant_id: String,
    pub repo_id: String,
}

/// AM2/AM3: the code-graph-backed Instant KG base report plus a `CodeKgManifest`
/// provenance record for a repo. `to_json` emits a `manifest` block the
/// SessionStart hook mirrors to `.harness/code-kg-manifest.json`.
#[derive(Clone, Debug, Default)]
pub struct CodeKgStatusOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub repo_url: String,
    pub head_sha: String,
    pub base_objects: u64,
    pub base_edges: u64,
    pub base_graph_hash: String,
    pub encoder_version: String,
    pub ingest_version: String,
    pub latest_generation: u64,
    pub indexed_at_ms: u64,
    pub indexed: bool,
}

impl CodeKgStatusOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "indexed": self.indexed,
            "base_objects": self.base_objects,
            "base_edges": self.base_edges,
            "base_graph_hash": self.base_graph_hash,
            "latest_generation": self.latest_generation,
            "indexed_at_ms": self.indexed_at_ms,
            "manifest": {
                "tenant_id": self.tenant_id,
                "repo_id": self.repo_id,
                "repo_url": self.repo_url,
                "head_sha": self.head_sha,
                "generation": self.latest_generation,
                "base_graph_hash": self.base_graph_hash,
                "encoder_version": self.encoder_version,
                "ingest_version": self.ingest_version,
                "objects_total": self.base_objects,
                "edges_total": self.base_edges,
            },
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct CodeContextInput {
    pub tenant_id: String,
    pub node_id: String,
    pub repo_id: String,
    pub file_path: String,
    pub before_lines: u64,
    pub after_lines: u64,
    pub max_chars: u64,
}

#[derive(Clone, Debug)]
pub struct CodeContextOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub file_id: String,
    pub symbol_id: String,
    pub file_path: String,
    pub start_line: u64,
    pub end_line: u64,
    pub context: String,
    pub symbols: Vec<CodeSymbolRecord>,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl CodeContextOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "file_id": self.file_id,
            "symbol_id": self.symbol_id,
            "file_path": self.file_path,
            "start_line": self.start_line,
            "end_line": self.end_line,
            "context": self.context,
            "symbols": self.symbols.iter().map(CodeSymbolRecord::to_json).collect::<Vec<_>>(),
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecognizeCodeInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub file_path: String,
    pub text: String,
    pub limit: u64,
}

#[derive(Clone, Debug)]
pub struct RecognizeCodeOutput {
    pub tenant_id: String,
    pub repo_id: String,
    pub file_path: String,
    pub symbols: Vec<CodeSymbolRecord>,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl RecognizeCodeOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "repo_id": self.repo_id,
            "file_path": self.file_path,
            "symbols": self.symbols.iter().map(CodeSymbolRecord::to_json).collect::<Vec<_>>(),
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExploreCodeInput {
    pub tenant_id: String,
    pub node_id: String,
    pub query: String,
    pub repo_id: String,
    pub max_depth: u64,
    pub limit: u64,
}

#[derive(Clone, Debug)]
pub struct ExploreCodeOutput {
    pub tenant_id: String,
    pub focus: Option<CodeSymbolRecord>,
    pub related_symbols: Vec<CodeSymbolRecord>,
    pub edges: Vec<CodeGraphEdgeRecord>,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl ExploreCodeOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "focus": self.focus.as_ref().map(CodeSymbolRecord::to_json),
            "related_symbols": self.related_symbols.iter().map(CodeSymbolRecord::to_json).collect::<Vec<_>>(),
            "edges": self.edges.iter().map(CodeGraphEdgeRecord::to_json).collect::<Vec<_>>(),
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExplainCodeInput {
    pub tenant_id: String,
    pub node_id: String,
    pub query: String,
    pub repo_id: String,
    pub max_chars: u64,
}

#[derive(Clone, Debug)]
pub struct ExplainCodeOutput {
    pub tenant_id: String,
    pub symbol: Option<CodeSymbolRecord>,
    pub summary: String,
    pub context: String,
    pub edges: Vec<CodeGraphEdgeRecord>,
    pub receipt_hash: String,
    pub receipt_json: String,
}

impl ExplainCodeOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "symbol": self.symbol.as_ref().map(CodeSymbolRecord::to_json),
            "summary": self.summary,
            "context": self.context,
            "edges": self.edges.iter().map(CodeGraphEdgeRecord::to_json).collect::<Vec<_>>(),
            "receipt_hash": self.receipt_hash,
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct RecordUseReceiptInput {
    pub tenant_id: String,
    pub node_id: String,
    pub repo_id: String,
    pub query: String,
    pub action: String,
    pub outcome: String,
    pub actor: String,
    pub use_json: String,
}

#[derive(Clone, Debug)]
pub struct RecordUseReceiptOutput {
    pub tenant_id: String,
    pub node_id: String,
    pub repo_id: String,
    pub receipt_hash: String,
    pub receipt_json: String,
    pub status: String,
    pub message: String,
}

impl RecordUseReceiptOutput {
    pub fn to_json(&self) -> Value {
        json!({
            "tenant_id": self.tenant_id,
            "node_id": self.node_id,
            "repo_id": self.repo_id,
            "receipt_hash": self.receipt_hash,
            "status": self.status,
            "message": self.message,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CodeHitRecord {
    pub node_id: String,
    pub repo_id: String,
    pub file_id: String,
    pub file_path: String,
    pub kind: String,
    pub name: String,
    pub language: String,
    pub line: u64,
    pub snippet: String,
    pub score: f64,
    pub trust_tier: String,
    pub community_id: String,
}

impl CodeHitRecord {
    fn to_json(&self) -> Value {
        json!({
            "node_id": self.node_id,
            "repo_id": self.repo_id,
            "file_id": self.file_id,
            "file_path": self.file_path,
            "kind": self.kind,
            "name": self.name,
            "language": self.language,
            "line": self.line,
            "snippet": self.snippet,
            "score": self.score,
            "trust_tier": self.trust_tier,
            "community_id": self.community_id,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CodeSymbolRecord {
    pub node_id: String,
    pub repo_id: String,
    pub file_id: String,
    pub file_path: String,
    pub kind: String,
    pub name: String,
    pub language: String,
    pub line: u64,
    pub signature: String,
    pub snippet: String,
    pub trust_tier: String,
    pub community_id: String,
    pub callers: Vec<String>,
    pub callees: Vec<String>,
    pub dependencies: Vec<String>,
    pub dependents: Vec<String>,
}

impl CodeSymbolRecord {
    fn from_indexed(symbol: IndexedSymbol, repo_id: &str) -> Self {
        Self {
            node_id: symbol.symbol_id,
            repo_id: repo_id.to_string(),
            file_id: symbol.file_id,
            file_path: symbol.file_path,
            kind: symbol.kind,
            name: symbol.name,
            language: symbol.language,
            line: symbol.line,
            signature: symbol.signature,
            snippet: symbol.snippet,
            trust_tier: symbol.trust_tier,
            community_id: symbol.community_id,
            callers: Vec::new(),
            callees: Vec::new(),
            dependencies: Vec::new(),
            dependents: Vec::new(),
        }
    }

    fn to_json(&self) -> Value {
        json!({
            "node_id": self.node_id,
            "repo_id": self.repo_id,
            "file_id": self.file_id,
            "file_path": self.file_path,
            "kind": self.kind,
            "name": self.name,
            "language": self.language,
            "line": self.line,
            "signature": self.signature,
            "snippet": self.snippet,
            "trust_tier": self.trust_tier,
            "community_id": self.community_id,
            "callers": self.callers,
            "callees": self.callees,
            "dependencies": self.dependencies,
            "dependents": self.dependents,
        })
    }
}

#[derive(Clone, Debug)]
pub struct CodeGraphEdgeRecord {
    pub from_node_id: String,
    pub to_node_id: String,
    pub edge_type: String,
    pub from_name: String,
    pub to_name: String,
    pub evidence: String,
}

impl CodeGraphEdgeRecord {
    fn to_json(&self) -> Value {
        json!({
            "from_node_id": self.from_node_id,
            "to_node_id": self.to_node_id,
            "edge_type": self.edge_type,
            "from_name": self.from_name,
            "to_name": self.to_name,
            "evidence": self.evidence,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeIndexError {
    pub code: String,
    pub message: String,
}

impl CodeIndexError {
    fn invalid(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_code_index_request".to_string(),
            message: message.into(),
        }
    }

    fn from_store(error: GraphStoreError) -> Self {
        Self {
            code: error.code,
            message: error.message,
        }
    }

    fn io(action: &str, path: &Path, error: impl std::fmt::Display) -> Self {
        Self {
            code: "code_index_io_error".to_string(),
            message: format!("{action} {}: {error}", path.display()),
        }
    }
}

impl From<CodeIndexError> for GraphStoreError {
    fn from(error: CodeIndexError) -> Self {
        GraphStoreError::new(error.code, error.message)
    }
}

impl std::fmt::Display for CodeIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CodeIndexError {}

pub struct IngestConfig {
    pub tenant_id: String,
    pub repo_root: PathBuf,
    pub repo_root_display: String,
    pub repo_id: String,
    pub include_extensions: BTreeSet<String>,
    pub exclude_dirs: BTreeSet<String>,
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub materialize_symbol_name_index: bool,
    pub actor: String,
    pub generation: u64,
    /// AM6 provenance: the URL this repo was ingested from (empty for a purely
    /// local-binding ingest) and the resolved HEAD commit sha. Stamped on the
    /// `CodeRepository` node so `list_repos` / the KG manifest can answer
    /// "is the base stale" without re-reading the working tree.
    pub repo_url: String,
    pub head_sha: String,
}

#[derive(Clone)]
pub struct IndexedFile {
    file_id: String,
    pub rel_path: String,
    language: String,
    extension: String,
    content_hash: String,
    text: String,
    symbols: Vec<IndexedSymbol>,
}

#[derive(Clone)]
pub(crate) struct IndexedSymbol {
    symbol_id: String,
    file_id: String,
    file_path: String,
    kind: String,
    name: String,
    language: String,
    line: u64,
    signature: String,
    snippet: String,
    body: String,
    trust_tier: String,
    community_id: String,
    call_names: BTreeSet<String>,
    dependency_names: BTreeSet<String>,
    parser_backed: bool,
}

/// Build the code-graph mutations (repo/file/symbol node upserts + contains/
/// declares/call/dependency edge upserts) for a set of indexed files WITHOUT
/// writing them to a store. This is the reusable extraction seam:
/// `ingest_codebase_with_store` wraps the result in a `GraphMutationBatch` and
/// commits it, while the instant-KG session-delta converter (`session_delta.rs`)
/// partitions the same mutations into a `SessionDelta`'s objects + edges.
/// Separating record-building from the store write is what lets a code edit
/// produce an overlay delta instead of an unconditional commit. The mutation
/// sequence is identical to the original inline build, so the committed graph is
/// byte-for-byte unchanged.
pub fn build_code_mutations(config: &IngestConfig, files: &[IndexedFile]) -> Vec<GraphMutation> {
    let mut mutations = build_node_mutations(config, files);
    for edge in infer_symbol_call_edges(files, config) {
        mutations.push(GraphMutation::EdgeUpsert(edge));
    }
    mutations
}

/// Build only the node-and-structural mutations (repo node, per-file CodeFile +
/// CodeFileText + CONTAINS_FILE, per-symbol CodeSymbol + DECLARES_SYMBOL) for a
/// set of indexed files, WITHOUT the inferred CALLS/DEPENDS edges. The
/// incremental-reindex path (D4) emits nodes for freshly parsed files only,
/// then infers edges over the FULL symbol set (fresh plus carried) so a
/// reindex produces the same edge graph as a full ingest. `build_code_mutations`
/// is this plus edge inference over the same files; the instant-KG session
/// delta (`session_delta.rs`) uses `build_code_mutations` unchanged.
pub(crate) fn build_node_mutations(
    config: &IngestConfig,
    files: &[IndexedFile],
) -> Vec<GraphMutation> {
    let repo_node = NodeRecord::new(
        &config.repo_id,
        [CODE_REPO_LABEL],
        json!({
            "tenant_id": config.tenant_id,
            "repo_id": config.repo_id,
            "repo_root": config.repo_root_display,
            "latest_generation": config.generation,
            "indexed_at_ms": config.generation,
            "actor": config.actor.clone(),
            "repo_url": config.repo_url,
            "head_sha": config.head_sha,
            "source": SOURCE,
        }),
    );

    let mut mutations = vec![GraphMutation::NodeUpsert(repo_node)];
    let symbol_name_targets = if config.materialize_symbol_name_index {
        let mut targets_by_name: BTreeMap<String, Vec<SymbolNameTarget>> = BTreeMap::new();
        for symbol in files.iter().flat_map(|file| file.symbols.iter()) {
            targets_by_name
                .entry(symbol.name.clone())
                .or_default()
                .push(SymbolNameTarget {
                    symbol_id: symbol.symbol_id.clone(),
                    file_path: symbol.file_path.clone(),
                    line: symbol.line,
                });
        }
        for targets in targets_by_name.values_mut() {
            targets.sort_by(|a, b| {
                a.file_path
                    .cmp(&b.file_path)
                    .then_with(|| a.line.cmp(&b.line))
                    .then_with(|| a.symbol_id.cmp(&b.symbol_id))
            });
            targets.dedup_by(|a, b| a.symbol_id == b.symbol_id);
        }
        targets_by_name
    } else {
        BTreeMap::new()
    };
    let mut emitted_symbol_names = BTreeSet::new();
    for file in files {
        // D3: the CodeFile node carries metadata only; the full text lives in
        // a content-addressed CodeFileText side record so search/explore node
        // loads never deserialize file contents.
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            &file.file_id,
            [CODE_FILE_LABEL],
            json!({
                "tenant_id": config.tenant_id,
                "repo_id": config.repo_id,
                "repo_root": config.repo_root_display,
                "file_id": file.file_id,
                "path": file.rel_path,
                "extension": file.extension,
                "language": file.language,
                "content_hash": file.content_hash,
                "generation": config.generation,
                "indexed_at_ms": config.generation,
                "source": SOURCE,
            }),
        )));
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            file_text_node_id(&config.tenant_id, &file.content_hash),
            [CODE_FILE_TEXT_LABEL],
            json!({
                "tenant_id": config.tenant_id,
                "repo_id": config.repo_id,
                "content_hash": file.content_hash,
                "path": file.rel_path,
                "text": file.text,
                "source": SOURCE,
            }),
        )));
        mutations.push(GraphMutation::EdgeUpsert(EdgeRecord::new(
            edge_id("code:edge:repo_file", &config.repo_id, &file.file_id),
            &config.repo_id,
            CONTAINS_FILE,
            &file.file_id,
            json!({
                "tenant_id": config.tenant_id,
                "repo_id": config.repo_id,
                "generation": config.generation,
                "source": SOURCE,
            }),
        )));
        for symbol in &file.symbols {
            if config.materialize_symbol_name_index
                && emitted_symbol_names.insert(symbol.name.clone())
            {
                let name_id = symbol_name_node_id(&config.repo_id, config.generation, &symbol.name);
                let targets = symbol_name_targets
                    .get(&symbol.name)
                    .map(|targets| {
                        targets
                            .iter()
                            .map(|target| {
                                json!({
                                    "symbol_id": target.symbol_id,
                                    "file_path": target.file_path,
                                    "line": target.line,
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                    &name_id,
                    [CODE_SYMBOL_NAME_LABEL],
                    json!({
                        "tenant_id": config.tenant_id,
                        "repo_id": config.repo_id,
                        "name": symbol.name,
                        "generation": config.generation,
                        "target_count": targets.len(),
                        "targets": targets,
                        "source": SOURCE,
                    }),
                )));
            }
            mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
                &symbol.symbol_id,
                [CODE_SYMBOL_LABEL],
                json!({
                    "tenant_id": config.tenant_id,
                    "repo_id": config.repo_id,
                    "repo_root": config.repo_root_display,
                    "file_id": symbol.file_id,
                    "file_path": symbol.file_path,
                    "symbol_id": symbol.symbol_id,
                    "kind": symbol.kind,
                    "name": symbol.name,
                    "language": symbol.language,
                    "line": symbol.line,
                    "signature": symbol.signature,
                    "snippet": symbol.snippet,
                    "trust_tier": symbol.trust_tier,
                    "community_id": symbol.community_id,
                    "call_names": symbol.call_names.iter().cloned().collect::<Vec<_>>(),
                    "dependency_names": symbol.dependency_names.iter().cloned().collect::<Vec<_>>(),
                    "parser_backed": symbol.parser_backed,
                    "search_text": format!("{} {} {} {}", symbol.name, symbol.kind, symbol.signature, symbol.file_path),
                    "generation": config.generation,
                    "indexed_at_ms": config.generation,
                    "source": SOURCE,
                }),
            )));
            mutations.push(GraphMutation::EdgeUpsert(EdgeRecord::new(
                edge_id("code:edge:file_symbol", &file.file_id, &symbol.symbol_id),
                &file.file_id,
                DECLARES_SYMBOL,
                &symbol.symbol_id,
                json!({
                    "tenant_id": config.tenant_id,
                    "repo_id": config.repo_id,
                    "generation": config.generation,
                    "source": SOURCE,
                }),
            )));
        }
    }
    mutations
}

pub(crate) struct PreparedCodebaseIngest {
    started: Instant,
    config: IngestConfig,
    mutations: Vec<GraphMutation>,
    symbols_indexed: u64,
    files_skipped: u64,
    files_parsed: u64,
    files_carried: u64,
    candidates_total: u64,
    budget_exceeded: bool,
    stage_timings: IngestStageTimings,
    language_stats: BTreeMap<String, LanguageIngestStats>,
    skip_stats: IngestSkipStats,
}

/// D4: everything a reindex needs to know about the prior generation, loaded
/// in one brief store pass so the heavy walk/parse work runs without the
/// store lock. Carried-forward files reuse their prior file/symbol nodes
/// (restamped to the new generation) instead of being re-parsed.
pub(crate) struct PriorGenerationSnapshot {
    pub(crate) generation: u64,
    pub(crate) files: HashMap<String, PriorFileEntry>,
}

pub(crate) struct PriorFileEntry {
    pub(crate) content_hash: String,
    pub(crate) file_node: NodeRecord,
    pub(crate) symbol_nodes: Vec<NodeRecord>,
}

pub(crate) fn load_prior_generation_snapshot(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
) -> Result<Option<PriorGenerationSnapshot>, CodeIndexError> {
    let repo_id = repo_id.trim();
    if repo_id.is_empty() {
        return Ok(None);
    }
    let Some(repo_node) = store
        .get_node(repo_id)
        .map_err(CodeIndexError::from_store)?
    else {
        return Ok(None);
    };
    if repo_node
        .properties
        .get("tenant_id")
        .and_then(Value::as_str)
        != Some(tenant_id)
    {
        return Ok(None);
    }
    let Some(generation) = property_u64(&repo_node.properties, "latest_generation") else {
        return Ok(None);
    };

    let file_nodes = store
        .query_nodes(
            NodeQuery::label(CODE_FILE_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(200_000),
        )
        .map_err(CodeIndexError::from_store)?;
    let mut files: HashMap<String, PriorFileEntry> = HashMap::new();
    let mut paths_by_file_id: HashMap<String, String> = HashMap::new();
    for node in file_nodes {
        if property_u64(&node.properties, "generation") != Some(generation) {
            continue;
        }
        let Some(path) = property_string(&node.properties, "path") else {
            continue;
        };
        let Some(content_hash) = property_string(&node.properties, "content_hash") else {
            continue;
        };
        paths_by_file_id.insert(node.id.clone(), path.clone());
        files.insert(
            path,
            PriorFileEntry {
                content_hash,
                file_node: node,
                symbol_nodes: Vec::new(),
            },
        );
    }

    let symbol_nodes = store
        .query_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("tenant_id", json!(tenant_id))
                .with_property("repo_id", json!(repo_id))
                .with_limit(1_000_000),
        )
        .map_err(CodeIndexError::from_store)?;
    for node in symbol_nodes {
        if property_u64(&node.properties, "generation") != Some(generation) {
            continue;
        }
        let Some(file_id) = property_string(&node.properties, "file_id") else {
            continue;
        };
        let Some(path) = paths_by_file_id.get(&file_id) else {
            continue;
        };
        if let Some(entry) = files.get_mut(path) {
            entry.symbol_nodes.push(node);
        }
    }
    for entry in files.values_mut() {
        entry.symbol_nodes.sort_by(|a, b| {
            property_u64(&a.properties, "line")
                .unwrap_or(0)
                .cmp(&property_u64(&b.properties, "line").unwrap_or(0))
                .then_with(|| a.id.cmp(&b.id))
        });
    }
    Ok(Some(PriorGenerationSnapshot { generation, files }))
}

fn snapshot_for_operation(
    store: &RedCoreGraphStore,
    operation: &str,
    config: &IngestConfig,
) -> Result<Option<PriorGenerationSnapshot>, CodeIndexError> {
    if operation == "reindex" {
        load_prior_generation_snapshot(store, &config.tenant_id, &config.repo_id)
    } else {
        Ok(None)
    }
}

/// Loads carried files' text by `content_hash` from the CodeFileText side
/// records (D3) so the incremental reindex can reconstruct carried symbols'
/// bodies as edge sources without re-reading the working tree. Returns a map
/// `content_hash -> text`; missing hashes are simply absent. Called once,
/// sequentially, after the change split, so the runtime path can take the
/// store lock for one brief batch read off the heavy parse path.
pub(crate) type CarriedTextLoader<'a> = &'a dyn Fn(&[String]) -> HashMap<String, String>;

/// Controls threaded through the prepare pipeline: the prior-generation
/// snapshot (D4 incremental reindex), an optional progress sink (D1 streaming
/// jobs), the carried-text loader (incremental edge reinference), and the parse
/// budget (D7: commit partial progress instead of dying on a transport
/// deadline). `parse_budget_ms == 0` means unlimited.
pub(crate) struct IngestPipelineOptions<'a> {
    pub(crate) prior: Option<PriorGenerationSnapshot>,
    pub(crate) sink: Option<&'a (dyn Fn(IngestJobEventKind) + Sync)>,
    pub(crate) carried_text_loader: Option<CarriedTextLoader<'a>>,
    pub(crate) parse_budget_ms: u64,
}

impl IngestPipelineOptions<'static> {
    pub(crate) fn sync_default(prior: Option<PriorGenerationSnapshot>) -> Self {
        Self {
            prior,
            sink: None,
            carried_text_loader: None,
            parse_budget_ms: default_parse_budget_ms(),
        }
    }
}

pub(crate) fn default_parse_budget_ms() -> u64 {
    std::env::var("THEOREM_CODE_INGEST_PARSE_BUDGET_MS")
        .ok()
        .and_then(|raw| raw.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_PARSE_BUDGET_MS)
}

/// Default server-side parse budget. Matches the harness MCP 120s ceiling so
/// even the synchronous in-store path finishes (with partial progress and a
/// `budget_exceeded` status) before any documented client deadline fires.
/// With submit-plus-stream (D1) the heavy path has no client deadline at all;
/// the budget protects the server from unbounded parses. Override with
/// `THEOREM_CODE_INGEST_PARSE_BUDGET_MS`; `0` disables the budget.
const DEFAULT_PARSE_BUDGET_MS: u64 = 120_000;

fn emit_ingest_event(
    sink: Option<&(dyn Fn(IngestJobEventKind) + Sync)>,
    event: impl FnOnce() -> IngestJobEventKind,
) {
    if let Some(sink) = sink {
        sink(event());
    }
}

pub(crate) fn prepare_codebase_ingest_resolved(
    mut config: IngestConfig,
    clone_ms: u64,
    resolve_ms: u64,
    started: Instant,
    options: IngestPipelineOptions<'_>,
) -> Result<PreparedCodebaseIngest, CodeIndexError> {
    let IngestPipelineOptions {
        prior,
        sink,
        carried_text_loader,
        parse_budget_ms,
    } = options;
    // A reindex in the same millisecond as the prior generation would collide
    // with it (generation is a timestamp); bump past the prior so the new
    // generation always supersedes it.
    if let Some(prior) = &prior {
        if config.generation <= prior.generation {
            config.generation = prior.generation + 1;
        }
    }

    let walk_started = Instant::now();
    let mut candidates = Vec::new();
    let mut skip_stats = IngestSkipStats::default();
    collect_code_file_candidates(&config.repo_root, &config, &mut candidates, &mut skip_stats)?;
    let walk_ms = elapsed_ms(walk_started);
    emit_ingest_event(sink, || IngestJobEventKind::WalkDone {
        files_found: candidates.len() as u64,
    });

    let parse_started = Instant::now();
    // LOAD: read and hash every candidate in parallel. Reading is cheap and
    // the hash is what lets a reindex skip the expensive symbol extraction.
    let loaded = load_code_file_candidates(&config, &candidates);
    let mut work = Vec::with_capacity(loaded.len());
    for item in loaded {
        match item {
            Some(loaded) => work.push(loaded),
            None => skip_stats.read_error += 1,
        }
    }
    let candidates_total = work.len() as u64;

    // SPLIT (D4): unchanged files carry their prior nodes forward un-parsed.
    let mut carried_entries: Vec<&PriorFileEntry> = Vec::new();
    let mut to_extract: Vec<LoadedCandidate> = Vec::new();
    if let Some(prior) = &prior {
        for loaded in work {
            match prior.files.get(&loaded.candidate.rel_path) {
                Some(entry) if entry.content_hash == loaded.content_hash => {
                    carried_entries.push(entry);
                }
                _ => to_extract.push(loaded),
            }
        }
    } else {
        to_extract = work;
    }

    // EXTRACT: symbol extraction in chunks, with progress events and the
    // parse budget checked at every chunk boundary. A budget stop keeps the
    // files parsed so far; the commit then reports `budget_exceeded` with
    // honest partial counts instead of surfacing a transport timeout.
    let extract_total = to_extract.len() as u64;
    let mut files: Vec<IndexedFile> = Vec::with_capacity(to_extract.len());
    let mut budget_exceeded = false;
    let chunk_count = to_extract.len().div_ceil(PARSE_CHUNK_FILES);
    for chunk_index in 0..chunk_count {
        if parse_budget_ms > 0 && chunk_index > 0 && elapsed_ms(parse_started) > parse_budget_ms {
            budget_exceeded = true;
            break;
        }
        let start = chunk_index * PARSE_CHUNK_FILES;
        let end = (start + PARSE_CHUNK_FILES).min(to_extract.len());
        let mut parsed: Vec<IndexedFile> = to_extract[start..end]
            .par_iter_mut()
            .map(|loaded| indexed_file_from_loaded(&config, loaded))
            .collect();
        files.append(&mut parsed);
        emit_ingest_event(sink, || IngestJobEventKind::ParseProgress {
            done: end as u64,
            total: extract_total,
        });
    }
    let parse_ms = elapsed_ms(parse_started);

    // RECONSTRUCT (D4 incremental edges): rebuild carried files as full edge
    // SOURCES from their persisted nodes plus, for non-parser-backed (body
    // tokenized) symbols, their text from the D3 side records. Parser-backed
    // (Rust) carried symbols carry their call/dependency names on the node, so
    // they need no text. Running edge inference over fresh PLUS carried makes a
    // reindex's edge graph identical to a full ingest of the same tree, which
    // closes the carried-symbol-to-changed-target and carried-symbol-to-newly-
    // defined-name seams.
    let carried_text = load_carried_text(&carried_entries, &config, carried_text_loader);
    let carried_files: Vec<IndexedFile> = carried_entries
        .iter()
        .map(|entry| reconstruct_carried_file(entry, &carried_text))
        .collect();
    let carried_symbol_count = carried_files
        .iter()
        .map(|file| file.symbols.len() as u64)
        .sum::<u64>();

    let mutation_started = Instant::now();
    let mut all_files: Vec<IndexedFile> = Vec::with_capacity(files.len() + carried_files.len());
    all_files.extend(files.iter().cloned());
    all_files.extend(carried_files);

    // Node mutations: fresh files emit their nodes + text records; carried
    // files restamp their existing nodes onto the new generation. Edges are
    // inferred over the FULL symbol set so every current edge is re-emitted at
    // the new generation (moved-target stale edges are left at the old
    // generation and retired by the latest-generation traversal filter).
    let mut mutations = build_node_mutations(&config, &files);
    mutations.extend(carried_forward_mutations(
        &carried_entries,
        config.generation,
    ));
    for edge in infer_symbol_call_edges(&all_files, &config) {
        mutations.push(GraphMutation::EdgeUpsert(edge));
    }
    let mutation_ms = elapsed_ms(mutation_started);

    let files_parsed = files.len() as u64;
    let files_carried = carried_entries.len() as u64;
    let symbols_indexed = files
        .iter()
        .map(|file| file.symbols.len() as u64)
        .sum::<u64>()
        + carried_symbol_count;
    let language_stats = language_stats_for(&files);

    Ok(PreparedCodebaseIngest {
        started,
        config,
        mutations,
        symbols_indexed,
        files_skipped: skip_stats.total(),
        files_parsed,
        files_carried,
        candidates_total,
        budget_exceeded,
        stage_timings: IngestStageTimings {
            clone_ms,
            resolve_ms,
            walk_ms,
            parse_ms,
            mutation_ms,
            ..IngestStageTimings::default()
        },
        language_stats,
        skip_stats,
    })
}

/// Restamp a prior generation's file and symbol nodes onto the new generation
/// without re-parsing. Node ids are generation-free (`stable_hash(repo_id,
/// path, kind, name, line)`), so the carried nodes keep their identities and
/// every existing edge stays attached. Removed files are simply not carried:
/// their nodes stay at the old generation, which the latest-generation filter
/// on search/context retires (the tombstone semantics for this store).
fn carried_forward_mutations(entries: &[&PriorFileEntry], generation: u64) -> Vec<GraphMutation> {
    let mut mutations = Vec::new();
    for entry in entries {
        mutations.push(GraphMutation::NodeUpsert(restamped_node(
            &entry.file_node,
            generation,
        )));
        for symbol in &entry.symbol_nodes {
            mutations.push(GraphMutation::NodeUpsert(restamped_node(
                symbol, generation,
            )));
        }
    }
    mutations
}

fn restamped_node(node: &NodeRecord, generation: u64) -> NodeRecord {
    let mut node = node.clone();
    if let Some(properties) = node.properties.as_object_mut() {
        properties.insert("generation".to_string(), json!(generation));
        properties.insert("indexed_at_ms".to_string(), json!(generation));
    }
    node
}

/// True when a carried file has any non-parser-backed symbol: those infer
/// edges by tokenizing their body, so the file's text must be reconstructed.
/// A purely parser-backed (Rust) carried file needs no text.
fn carried_file_needs_text(entry: &PriorFileEntry) -> bool {
    entry
        .symbol_nodes
        .iter()
        .any(|node| !property_bool(&node.properties, "parser_backed"))
}

/// Batch-load text for carried files that need it (one call into the injected
/// loader, so the runtime path takes the store lock once). Keyed by content
/// hash, which a carried file shares with its existing CodeFileText record.
fn load_carried_text(
    carried_entries: &[&PriorFileEntry],
    config: &IngestConfig,
    loader: Option<CarriedTextLoader<'_>>,
) -> HashMap<String, String> {
    let Some(loader) = loader else {
        return HashMap::new();
    };
    let mut hashes: Vec<String> = carried_entries
        .iter()
        .filter(|entry| carried_file_needs_text(entry))
        .map(|entry| entry.content_hash.clone())
        .collect();
    hashes.sort();
    hashes.dedup();
    if hashes.is_empty() {
        return HashMap::new();
    }
    let _ = config;
    loader(&hashes)
}

/// Rebuild a carried file as a full `IndexedFile` (an edge SOURCE) from its
/// persisted file/symbol nodes plus, when present, its reconstructed text.
/// Symbol identity, name, kind, line, and the parser-backed call/dependency
/// names all come straight off the persisted symbol nodes; the body is sliced
/// from the text for the inverted-index path. This reproduces exactly what a
/// fresh parse of the same (unchanged) file would yield for edge inference.
fn reconstruct_carried_file(
    entry: &PriorFileEntry,
    carried_text: &HashMap<String, String>,
) -> IndexedFile {
    let file_props = &entry.file_node.properties;
    let rel_path = property_string(file_props, "path").unwrap_or_default();
    let language = property_string(file_props, "language").unwrap_or_default();
    let extension = property_string(file_props, "extension").unwrap_or_default();
    let file_id =
        property_string(file_props, "file_id").unwrap_or_else(|| entry.file_node.id.clone());

    let text = carried_text.get(&entry.content_hash).cloned();
    let lines: Vec<&str> = text
        .as_deref()
        .map(|t| t.lines().collect())
        .unwrap_or_default();

    let mut symbols = Vec::with_capacity(entry.symbol_nodes.len());
    for (index, node) in entry.symbol_nodes.iter().enumerate() {
        let props = &node.properties;
        let line = property_u64(props, "line").unwrap_or(0);
        // Bodies run from this symbol's line to just before the next symbol's,
        // matching `extract_line_symbols`. Only needed for the tokenized path.
        let next_line = entry
            .symbol_nodes
            .get(index + 1)
            .map(|next| {
                property_u64(&next.properties, "line")
                    .unwrap_or(0)
                    .saturating_sub(1)
            })
            .unwrap_or(lines.len() as u64);
        let body = if lines.is_empty() {
            String::new()
        } else {
            body_between_lines(&lines, line, next_line)
        };
        symbols.push(IndexedSymbol {
            symbol_id: property_string(props, "symbol_id").unwrap_or_else(|| node.id.clone()),
            file_id: property_string(props, "file_id").unwrap_or_else(|| file_id.clone()),
            file_path: property_string(props, "file_path").unwrap_or_else(|| rel_path.clone()),
            kind: property_string(props, "kind").unwrap_or_default(),
            name: property_string(props, "name").unwrap_or_default(),
            language: property_string(props, "language").unwrap_or_else(|| language.clone()),
            line,
            signature: property_string(props, "signature").unwrap_or_default(),
            snippet: property_string(props, "snippet").unwrap_or_default(),
            body,
            trust_tier: property_string(props, "trust_tier")
                .unwrap_or_else(|| DEFAULT_TRUST_TIER.to_string()),
            community_id: property_string(props, "community_id").unwrap_or_default(),
            call_names: property_string_set(props, "call_names"),
            dependency_names: property_string_set(props, "dependency_names"),
            parser_backed: property_bool(props, "parser_backed"),
        });
    }

    IndexedFile {
        file_id,
        rel_path,
        language,
        extension,
        content_hash: entry.content_hash.clone(),
        // Carried files do not re-emit their text record (it already exists at
        // the same content hash); this field is unused for carried files.
        text: String::new(),
        symbols,
    }
}

pub(crate) fn commit_prepared_ingest(
    store: &mut RedCoreGraphStore,
    mut prepared: PreparedCodebaseIngest,
    operation: &str,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    let write_started = Instant::now();

    let transaction = store
        .commit_batch(GraphMutationBatch::new(prepared.mutations))
        .map_err(CodeIndexError::from_store)?;

    let epistemic = code_epistemic_hook::run_code_epistemic_pass_for_repo(
        store,
        &prepared.config.tenant_id,
        &prepared.config.repo_id,
        Some(prepared.config.generation),
    )?;
    let epistemic_readout = epistemic.to_json();

    prepared.stage_timings.write_ms = elapsed_ms(write_started);
    prepared.stage_timings.total_ms = elapsed_ms(prepared.started);

    let files_indexed = prepared.files_parsed + prepared.files_carried;
    let (status, message) = if prepared.budget_exceeded {
        (
            "budget_exceeded".to_string(),
            format!(
                "parse budget exhausted: parsed {} of {} candidate files (carried {} unchanged); committed partial progress",
                prepared.files_parsed, prepared.candidates_total, prepared.files_carried
            ),
        )
    } else {
        (
            "ok".to_string(),
            "codebase indexed into RedCore".to_string(),
        )
    };

    let summary = json!({
        "tenant_id": prepared.config.tenant_id,
        "repo_id": prepared.config.repo_id,
        "repo_root": prepared.config.repo_root_display,
        "generation": prepared.config.generation,
        "operation": operation,
        "status": status,
        "files_indexed": files_indexed,
        "files_parsed": prepared.files_parsed,
        "files_carried": prepared.files_carried,
        "candidates_total": prepared.candidates_total,
        "symbols_indexed": prepared.symbols_indexed,
        "files_skipped": prepared.files_skipped,
        "graph_version": transaction.graph_version,
        "actor": prepared.config.actor.clone(),
        "epistemic_readout": epistemic_readout,
        "stage_timings": prepared.stage_timings.to_json(),
        "language_stats": language_stats_json(&prepared.language_stats),
        "skip_stats": prepared.skip_stats.to_json(),
    });
    let receipt = record_receipt(
        store,
        &prepared.config.tenant_id,
        &format!("code_{operation}"),
        &summary,
    )?;
    prepared.stage_timings.write_ms = elapsed_ms(write_started);
    prepared.stage_timings.total_ms = elapsed_ms(prepared.started);

    Ok(IngestCodebaseOutput {
        tenant_id: prepared.config.tenant_id,
        repo_id: prepared.config.repo_id,
        repo_root: prepared.config.repo_root_display,
        generation: prepared.config.generation,
        files_indexed,
        symbols_indexed: prepared.symbols_indexed,
        files_skipped: prepared.files_skipped,
        files_parsed: prepared.files_parsed,
        files_carried: prepared.files_carried,
        graph_version: receipt.graph_version,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
        status,
        message,
        epistemic_readout,
        stage_timings: prepared.stage_timings,
        language_stats: prepared.language_stats,
        skip_stats: prepared.skip_stats,
    })
}

fn search_code_with_store(
    store: &mut RedCoreGraphStore,
    input: SearchCodeInput,
) -> Result<SearchCodeOutput, CodeIndexError> {
    let started = std::time::Instant::now();
    let tenant_id = normalize_tenant(&input.tenant_id);
    let query = input.query.trim().to_string();
    let limit = bounded_limit(input.limit);
    let kinds = normalize_set(input.kinds);
    let latest = latest_repo_generations(store)?;

    let mut node_query = NodeQuery::label(CODE_SYMBOL_LABEL).with_limit(100_000);
    if !input.repo_id.trim().is_empty() {
        node_query = node_query.with_property("repo_id", json!(input.repo_id.trim()));
    }
    let nodes = store
        .query_nodes(node_query)
        .map_err(CodeIndexError::from_store)?;
    let mut scored = Vec::new();
    let query_terms = query_terms(&query);
    let path_prefix = input.path_prefix.trim();

    for node in nodes {
        let Some(hit) = hit_from_node(&node) else {
            continue;
        };
        if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_id.as_str()) {
            continue;
        }
        if let Some(generation) = latest.get(&hit.repo_id) {
            if property_u64(&node.properties, "generation") != Some(*generation) {
                continue;
            }
        }
        if !path_prefix.is_empty() && !hit.file_path.starts_with(path_prefix) {
            continue;
        }
        if !kinds.is_empty() && !kinds.contains(&hit.kind.to_ascii_lowercase()) {
            continue;
        }
        let score = score_hit(&hit, &query, &query_terms);
        if score <= 0.0 && !query.is_empty() {
            continue;
        }
        scored.push(CodeHitRecord { score, ..hit });
    }

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.file_path.cmp(&b.file_path))
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    let total_admitted = scored.len() as u64;
    let hits = scored.into_iter().take(limit).collect::<Vec<_>>();
    let latency_ms = started.elapsed().as_millis() as u64;

    let receipt_payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_search",
        "query": query,
        "repo_id": input.repo_id,
        "path_prefix": input.path_prefix,
        "kinds": kinds,
        "total_admitted": total_admitted,
        "total_returned": hits.len(),
        "latency_ms": latency_ms,
    });
    let receipt = record_receipt(store, &tenant_id, "code_search", &receipt_payload)?;

    Ok(SearchCodeOutput {
        tenant_id,
        query,
        total_admitted,
        total_returned: hits.len() as u64,
        hits,
        latency_ms,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

fn code_context_with_store(
    store: &mut RedCoreGraphStore,
    input: CodeContextInput,
) -> Result<CodeContextOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let latest = latest_repo_generations(store)?;
    let node_id = input.node_id.trim();

    let (symbol_id, file_id, target_line) = if node_id.is_empty() {
        (
            "".to_string(),
            resolve_file_by_path(store, &tenant_id, &input)?,
            1u64,
        )
    } else {
        let node = store
            .get_node(node_id)
            .map_err(CodeIndexError::from_store)?
            .ok_or_else(|| CodeIndexError::invalid(format!("code node {node_id} was not found")))?;
        if node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
            (
                node.id.clone(),
                property_string(&node.properties, "file_id").ok_or_else(|| {
                    CodeIndexError::invalid(format!("symbol node {node_id} is missing file_id"))
                })?,
                property_u64(&node.properties, "line").unwrap_or(1),
            )
        } else if node.labels.iter().any(|label| label == CODE_FILE_LABEL) {
            ("".to_string(), node.id.clone(), 1u64)
        } else {
            return Err(CodeIndexError::invalid(format!(
                "node {node_id} is not a code file or symbol"
            )));
        }
    };

    let file = store
        .get_node(&file_id)
        .map_err(CodeIndexError::from_store)?
        .ok_or_else(|| CodeIndexError::invalid(format!("code file {file_id} was not found")))?;
    let repo_id = property_string(&file.properties, "repo_id").unwrap_or_default();
    if file.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_id.as_str()) {
        return Err(CodeIndexError::invalid(
            "code file belongs to a different tenant",
        ));
    }
    if let Some(generation) = latest.get(&repo_id) {
        if property_u64(&file.properties, "generation") != Some(*generation) {
            return Err(CodeIndexError::invalid(
                "code file is not part of the latest repo generation",
            ));
        }
    }

    let text = file_text_for(store, &tenant_id, &file)?;
    let file_path = property_string(&file.properties, "path").unwrap_or_default();
    let before = nonzero_or(input.before_lines, DEFAULT_CONTEXT_LINES);
    let after = nonzero_or(input.after_lines, DEFAULT_CONTEXT_LINES);
    let max_chars = nonzero_or(input.max_chars, DEFAULT_MAX_CONTEXT_CHARS as u64) as usize;
    let (start_line, end_line, context) =
        line_context(&text, target_line, before, after, max_chars);
    let symbols = symbols_for_file(store, &tenant_id, &file_id, &repo_id, &latest)?;

    let receipt_payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_context",
        "repo_id": repo_id,
        "file_id": file_id,
        "symbol_id": symbol_id,
        "file_path": file_path,
        "start_line": start_line,
        "end_line": end_line,
    });
    let receipt = record_receipt(store, &tenant_id, "code_context", &receipt_payload)?;

    Ok(CodeContextOutput {
        tenant_id,
        repo_id,
        file_id,
        symbol_id,
        file_path,
        start_line,
        end_line,
        context,
        symbols,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

fn recognize_code_with_store(
    store: &mut RedCoreGraphStore,
    input: RecognizeCodeInput,
) -> Result<RecognizeCodeOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let limit = bounded_limit(input.limit);
    let mut symbols = if input.text.trim().is_empty() {
        recognize_indexed_symbols(store, &tenant_id, &input)?
    } else {
        extract_symbols(
            input.repo_id.trim(),
            "",
            input.file_path.trim(),
            language_for_extension(&extension_for(Path::new(input.file_path.trim()))),
            &input.text,
        )
        .into_iter()
        .map(|symbol| CodeSymbolRecord::from_indexed(symbol, input.repo_id.trim()))
        .collect::<Vec<_>>()
    };
    symbols.truncate(limit);
    let payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_recognize",
        "repo_id": input.repo_id,
        "file_path": input.file_path,
        "symbols_returned": symbols.len(),
    });
    let receipt = record_receipt(store, &tenant_id, "code_recognize", &payload)?;
    Ok(RecognizeCodeOutput {
        tenant_id,
        repo_id: input.repo_id,
        file_path: input.file_path,
        symbols,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

fn explore_code_with_store(
    store: &mut RedCoreGraphStore,
    input: ExploreCodeInput,
) -> Result<ExploreCodeOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let focus_node = resolve_symbol_node(
        store,
        &tenant_id,
        &input.node_id,
        &input.query,
        &input.repo_id,
    )?;
    let Some(focus_node) = focus_node else {
        let payload = json!({
            "tenant_id": tenant_id,
            "operation": "code_explore",
            "query": input.query,
            "repo_id": input.repo_id,
            "resolved": false,
        });
        let receipt = record_receipt(store, &tenant_id, "code_explore", &payload)?;
        return Ok(ExploreCodeOutput {
            tenant_id,
            focus: None,
            related_symbols: Vec::new(),
            edges: Vec::new(),
            receipt_hash: receipt.receipt_hash,
            receipt_json: receipt.receipt_json,
        });
    };
    let limit = bounded_limit(input.limit);
    let max_depth = if input.max_depth == 0 {
        1
    } else {
        input.max_depth.min(4) as usize
    };
    let latest = latest_repo_generations(store)?;
    let mut edges = Vec::new();
    let mut related_ids = BTreeSet::new();
    expand_symbol_edges(
        store,
        &focus_node.id,
        max_depth,
        limit,
        &latest,
        &mut related_ids,
        &mut edges,
    )?;
    related_ids.remove(&focus_node.id);
    let mut related_symbols = Vec::new();
    for node_id in related_ids.into_iter().take(limit) {
        if let Some(node) = store
            .get_node(&node_id)
            .map_err(CodeIndexError::from_store)?
        {
            if let Some(mut symbol) = symbol_record_from_node(&node) {
                enrich_symbol_graph(store, &mut symbol, &latest)?;
                related_symbols.push(symbol);
            }
        }
    }
    let mut focus = symbol_record_from_node(&focus_node);
    if let Some(symbol) = focus.as_mut() {
        enrich_symbol_graph(store, symbol, &latest)?;
    }
    let payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_explore",
        "focus_node_id": focus_node.id,
        "related_count": related_symbols.len(),
        "edge_count": edges.len(),
    });
    let receipt = record_receipt(store, &tenant_id, "code_explore", &payload)?;
    Ok(ExploreCodeOutput {
        tenant_id,
        focus,
        related_symbols,
        edges,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

fn explain_code_with_store(
    store: &mut RedCoreGraphStore,
    input: ExplainCodeInput,
) -> Result<ExplainCodeOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let focus_node = resolve_symbol_node(
        store,
        &tenant_id,
        &input.node_id,
        &input.query,
        &input.repo_id,
    )?;
    let Some(focus_node) = focus_node else {
        let payload = json!({
            "tenant_id": tenant_id,
            "operation": "code_explain",
            "query": input.query,
            "resolved": false,
        });
        let receipt = record_receipt(store, &tenant_id, "code_explain", &payload)?;
        return Ok(ExplainCodeOutput {
            tenant_id,
            symbol: None,
            summary: "No indexed code symbol matched the explanation request.".to_string(),
            context: String::new(),
            edges: Vec::new(),
            receipt_hash: receipt.receipt_hash,
            receipt_json: receipt.receipt_json,
        });
    };
    let mut symbol = symbol_record_from_node(&focus_node)
        .ok_or_else(|| CodeIndexError::invalid("resolved code node is missing symbol metadata"))?;
    let latest = latest_repo_generations(store)?;
    enrich_symbol_graph(store, &mut symbol, &latest)?;
    let context = code_context_with_store(
        store,
        CodeContextInput {
            tenant_id: tenant_id.clone(),
            node_id: focus_node.id.clone(),
            max_chars: input.max_chars,
            ..Default::default()
        },
    )?;
    let mut edges = Vec::new();
    let mut related_ids = BTreeSet::new();
    expand_symbol_edges(
        store,
        &focus_node.id,
        1,
        DEFAULT_LIMIT,
        &latest,
        &mut related_ids,
        &mut edges,
    )?;
    let summary = format!(
        "{} `{}` is a {} in `{}`. Trust tier: {}. It calls {} symbol(s), is called by {} symbol(s), and depends on {} symbol(s).",
        symbol.language,
        symbol.name,
        symbol.kind,
        symbol.file_path,
        symbol.trust_tier,
        symbol.callees.len(),
        symbol.callers.len(),
        symbol.dependencies.len()
    );
    let payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_explain",
        "symbol_id": symbol.node_id,
        "edge_count": edges.len(),
    });
    let receipt = record_receipt(store, &tenant_id, "code_explain", &payload)?;
    Ok(ExplainCodeOutput {
        tenant_id,
        symbol: Some(symbol),
        summary,
        context: context.context,
        edges,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
    })
}

fn record_use_receipt_with_store(
    store: &mut RedCoreGraphStore,
    input: RecordUseReceiptInput,
) -> Result<RecordUseReceiptOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let node_id = input.node_id.trim().to_string();
    if node_id.is_empty() {
        return Err(CodeIndexError::invalid(
            "node_id is required to record code use",
        ));
    }
    let node = store
        .get_node(&node_id)
        .map_err(CodeIndexError::from_store)?
        .ok_or_else(|| CodeIndexError::invalid(format!("code node {node_id} was not found")))?;
    if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant_id.as_str()) {
        return Err(CodeIndexError::invalid(
            "code node belongs to a different tenant",
        ));
    }
    let repo_id =
        property_string(&node.properties, "repo_id").unwrap_or_else(|| input.repo_id.clone());
    if !input.repo_id.trim().is_empty() && input.repo_id.trim() != repo_id {
        return Err(CodeIndexError::invalid(
            "code node belongs to a different repository",
        ));
    }
    let use_payload = parse_optional_json(&input.use_json)?;
    let payload = json!({
        "tenant_id": tenant_id,
        "operation": "code_use",
        "node_id": node_id,
        "repo_id": repo_id,
        "file_path": property_string(&node.properties, "file_path")
            .or_else(|| property_string(&node.properties, "path"))
            .unwrap_or_default(),
        "symbol_name": property_string(&node.properties, "name").unwrap_or_default(),
        "query": input.query,
        "action": input.action,
        "outcome": input.outcome,
        "actor": input.actor,
        "use": use_payload,
    });
    let receipt = record_receipt(store, &tenant_id, "code_use", &payload)?;
    Ok(RecordUseReceiptOutput {
        tenant_id,
        node_id,
        repo_id,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
        status: "ok".to_string(),
        message: "code use receipt recorded".to_string(),
    })
}

pub fn collect_code_files(
    dir: &Path,
    config: &IngestConfig,
    out: &mut Vec<IndexedFile>,
    skipped: &mut u64,
) -> Result<(), CodeIndexError> {
    if out.len() >= config.max_files {
        return Ok(());
    }
    let mut candidates = Vec::new();
    let mut skip_stats = IngestSkipStats::default();
    collect_code_file_candidates(dir, config, &mut candidates, &mut skip_stats)?;
    let (mut files, read_skips) = parse_code_file_candidates(config, &candidates);
    skip_stats.read_error += read_skips;
    *skipped += skip_stats.total();
    out.append(&mut files);
    if out.len() > config.max_files {
        out.truncate(config.max_files);
    }
    Ok(())
}

/// W1.1/W1.3 source-file write seam: parse and commit exactly one source file
/// into the code graph without walking or reindexing the repository.
///
/// The caller supplies the file bytes that were just written into the workspace
/// store. This reuses the same per-file encoder and node mutation builder as
/// full ingest, so CodeFile/CodeSymbol graph shape stays identical. When the
/// persistent name-bucket index is enabled, it also rewrites outgoing
/// CALLS/DEPENDS edges for the changed file's symbols by looking up only the
/// names observed in the changed file.
pub fn index_source_file_write_in_store(
    store: &mut RedCoreGraphStore,
    input: SourceFileWriteIndexInput,
) -> Result<SourceFileWriteIndexOutput, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let repo_id = input.repo_id.trim().to_string();
    if repo_id.is_empty() {
        return Err(CodeIndexError::invalid(
            "repo_id is required for source-file write indexing",
        ));
    }
    let rel_path = safe_source_file_path(&input.file_path)?;
    let extension = extension_for(Path::new(&rel_path));
    if !default_extensions().contains(&extension) {
        return Err(CodeIndexError::invalid(format!(
            "unsupported source extension for {rel_path:?}"
        )));
    }
    if input.content.len() as u64 > ABSOLUTE_MAX_FILE_BYTES {
        return Err(CodeIndexError::invalid(format!(
            "source file {rel_path:?} exceeds max file bytes"
        )));
    }
    if input.content.contains(&0) {
        return Err(CodeIndexError::invalid(format!(
            "source file {rel_path:?} appears to be binary"
        )));
    }
    let text = String::from_utf8(input.content).map_err(|error| {
        CodeIndexError::invalid(format!("source file {rel_path:?} is not utf-8: {error}"))
    })?;
    let prior = load_prior_generation_snapshot(store, &tenant_id, &repo_id)?;
    let mut generation = if input.generation == 0 {
        now_ms().max(1) as u64
    } else {
        input.generation
    };
    if let Some(prior) = &prior {
        if generation <= prior.generation {
            generation = prior.generation + 1;
        }
    }
    let repo_root_display = if input.repo_root_display.trim().is_empty() {
        repo_id.clone()
    } else {
        input.repo_root_display.trim().to_string()
    };
    let config = IngestConfig {
        tenant_id: tenant_id.clone(),
        repo_root: PathBuf::from(&repo_root_display),
        repo_root_display,
        repo_id: repo_id.clone(),
        include_extensions: default_extensions(),
        exclude_dirs: default_exclude_dirs(),
        max_files: 1,
        max_file_bytes: ABSOLUTE_MAX_FILE_BYTES,
        // Source-file writes materialize touched buckets below. Keeping the
        // generic node builder off avoids emitting an incomplete per-file
        // bucket when another current file defines the same symbol name.
        materialize_symbol_name_index: false,
        actor: input.actor.trim().to_string(),
        generation,
        repo_url: String::new(),
        head_sha: String::new(),
    };
    let content_hash = stable_hash(json!({
        "repo_id": repo_id,
        "path": rel_path,
        "content": text,
    }));
    let language = language_for_extension(&extension).to_string();
    let mut loaded = LoadedCandidate {
        candidate: CodeFileCandidate {
            path: PathBuf::from(&rel_path),
            rel_path: rel_path.clone(),
            extension,
            language,
        },
        text,
        content_hash: content_hash.clone(),
    };
    let indexed = indexed_file_from_loaded(&config, &mut loaded);
    let symbol_names = indexed
        .symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect::<Vec<_>>();
    let file_id = indexed.file_id.clone();
    let symbols_indexed = indexed.symbols.len() as u64;
    let carried_entries = prior
        .as_ref()
        .map(|prior| {
            prior
                .files
                .iter()
                .filter_map(|(path, entry)| (path != &rel_path).then_some(entry))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let touched_names = touched_names_for_file(&indexed);
    let changed_definition_names = indexed
        .symbols
        .iter()
        .map(|symbol| symbol.name.clone())
        .collect::<BTreeSet<_>>();
    let bucket_names = if input.materialize_symbol_name_index {
        touched_names.iter().cloned().collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let targets_by_name = if input.materialize_symbol_name_index {
        collect_touched_symbol_name_targets(
            store,
            &config,
            prior.as_ref(),
            &indexed,
            &touched_names,
        )?
    } else {
        BTreeMap::new()
    };
    let mut mutations = build_node_mutations(&config, std::slice::from_ref(&indexed));
    mutations.extend(carried_forward_mutations(
        &carried_entries,
        config.generation,
    ));
    if input.materialize_symbol_name_index {
        mutations.extend(symbol_name_bucket_mutations(&config, &targets_by_name));
    }
    let retired_edges = tombstone_existing_outgoing_symbol_edges(store, &indexed.symbols)?;
    let edges_retired = retired_edges.len() as u64;
    mutations.extend(retired_edges);
    let carried_sources_for_incoming = if input.materialize_symbol_name_index
        && !changed_definition_names.is_empty()
        && !carried_entries.is_empty()
    {
        let carried_text = load_carried_text(
            &carried_entries,
            &config,
            Some(&|hashes| load_file_texts(store, &config.tenant_id, hashes)),
        );
        carried_entries
            .iter()
            .map(|entry| reconstruct_carried_file(entry, &carried_text))
            .flat_map(|file| {
                file.symbols
                    .into_iter()
                    .filter(|symbol| symbol_references_any(symbol, &changed_definition_names))
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>()
    } else {
        Vec::new()
    };
    let incoming_retired_edges = tombstone_existing_outgoing_symbol_edges_to_names(
        store,
        &carried_sources_for_incoming,
        &changed_definition_names,
    )?;
    let edges_retired = edges_retired.saturating_add(incoming_retired_edges.len() as u64);
    mutations.extend(incoming_retired_edges);
    let outgoing_edges = if input.materialize_symbol_name_index {
        infer_touched_outgoing_edges(&indexed.symbols, &targets_by_name, &config)
    } else {
        Vec::new()
    };
    let incoming_edges = if input.materialize_symbol_name_index {
        infer_incoming_edges_for_names(
            &carried_sources_for_incoming,
            &targets_by_name,
            &changed_definition_names,
            &config,
        )
    } else {
        Vec::new()
    };
    let edges_indexed = (outgoing_edges.len() + incoming_edges.len()) as u64;
    mutations.extend(outgoing_edges.into_iter().map(GraphMutation::EdgeUpsert));
    mutations.extend(incoming_edges.into_iter().map(GraphMutation::EdgeUpsert));
    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(CodeIndexError::from_store)?;
    Ok(SourceFileWriteIndexOutput {
        tenant_id,
        repo_id,
        file_path: rel_path,
        file_id,
        content_hash,
        generation,
        graph_version: transaction.graph_version,
        symbols_indexed,
        files_carried: carried_entries.len() as u64,
        edges_indexed,
        edges_retired,
        bucket_lookups: touched_names.len() as u64,
        symbol_names,
        bucket_names,
    })
}

#[derive(Clone, Debug)]
struct CodeFileCandidate {
    path: PathBuf,
    rel_path: String,
    extension: String,
    language: String,
}

/// D5: parallel, gitignore-aware walk via the `ignore` crate. The repo's
/// `.gitignore` and `.ignore` files prune `node_modules`/`target`/`dist`
/// without a hardcoded list (`require_git(false)` so plain directories honor
/// them too), and the walk fans out across threads. The extension allowlist,
/// the file-size cap, the binary sniff, the lockfile/minified-name skip, and
/// the dot-dir plus `exclude_dirs` pruning from the old manual walk all stay.
/// Dot FILES stay eligible (matching the old walk, which only skipped dot
/// directories). Per-entry IO errors count as `read_error` skips instead of
/// failing the whole ingest; the repo root itself is validated upstream by
/// `resolve_ingest_config`. When the tree holds more eligible files than
/// `max_files`, the walk quits early, so WHICH files fill the cap follows
/// discovery order; the surviving set is then sorted by relative path so the
/// output stays deterministic for a given discovered set.
fn collect_code_file_candidates(
    dir: &Path,
    config: &IngestConfig,
    out: &mut Vec<CodeFileCandidate>,
    skip_stats: &mut IngestSkipStats,
) -> Result<(), CodeIndexError> {
    #[derive(Default)]
    struct WalkAccumulator {
        candidates: Vec<CodeFileCandidate>,
        skips: IngestSkipStats,
    }

    let capacity = config.max_files.saturating_sub(out.len());
    if capacity == 0 {
        return Ok(());
    }

    let accumulator = Mutex::new(WalkAccumulator::default());
    let exclude_dirs = config.exclude_dirs.clone();
    let mut builder = ignore::WalkBuilder::new(dir);
    builder
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .parents(false)
        .require_git(false)
        .follow_links(false)
        .threads(walk_threads());
    builder.filter_entry(move |entry| {
        if entry.depth() == 0 {
            return true;
        }
        match entry.file_type() {
            Some(file_type) if file_type.is_dir() => {
                !should_skip_dir(&entry.file_name().to_string_lossy(), &exclude_dirs)
            }
            _ => true,
        }
    });

    builder.build_parallel().run(|| {
        let accumulator = &accumulator;
        Box::new(move |entry| {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => {
                    accumulator
                        .lock()
                        .expect("walk accumulator")
                        .skips
                        .read_error += 1;
                    return ignore::WalkState::Continue;
                }
            };
            if entry.depth() == 0 {
                return ignore::WalkState::Continue;
            }
            let Some(file_type) = entry.file_type() else {
                return ignore::WalkState::Continue;
            };
            if !file_type.is_file() {
                return ignore::WalkState::Continue;
            }
            let file_name = entry.file_name().to_string_lossy().to_string();
            if should_skip_file_name(&file_name) {
                accumulator.lock().expect("walk accumulator").skips.filename += 1;
                return ignore::WalkState::Continue;
            }
            let path = entry.path();
            let extension = extension_for(path);
            if !config.include_extensions.contains(&extension) {
                accumulator
                    .lock()
                    .expect("walk accumulator")
                    .skips
                    .unsupported_extension += 1;
                return ignore::WalkState::Continue;
            }
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => {
                    accumulator
                        .lock()
                        .expect("walk accumulator")
                        .skips
                        .read_error += 1;
                    return ignore::WalkState::Continue;
                }
            };
            if metadata.len() > config.max_file_bytes {
                accumulator
                    .lock()
                    .expect("walk accumulator")
                    .skips
                    .too_large += 1;
                return ignore::WalkState::Continue;
            }
            match has_null_byte_prefix(path) {
                Ok(true) => {
                    accumulator.lock().expect("walk accumulator").skips.binary += 1;
                    return ignore::WalkState::Continue;
                }
                Ok(false) => {}
                Err(_) => {
                    accumulator
                        .lock()
                        .expect("walk accumulator")
                        .skips
                        .read_error += 1;
                    return ignore::WalkState::Continue;
                }
            }
            let Ok(rel_path) = relative_path(&config.repo_root, path) else {
                accumulator
                    .lock()
                    .expect("walk accumulator")
                    .skips
                    .read_error += 1;
                return ignore::WalkState::Continue;
            };
            let mut state = accumulator.lock().expect("walk accumulator");
            state.candidates.push(CodeFileCandidate {
                path: path.to_path_buf(),
                rel_path,
                extension: extension.clone(),
                language: language_for_extension(&extension).to_string(),
            });
            if state.candidates.len() >= capacity {
                return ignore::WalkState::Quit;
            }
            ignore::WalkState::Continue
        })
    });

    let WalkAccumulator {
        mut candidates,
        skips,
    } = accumulator.into_inner().expect("walk accumulator");
    candidates.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    candidates.truncate(capacity);
    skip_stats.merge_from(&skips);
    out.extend(candidates);
    Ok(())
}

fn walk_threads() -> usize {
    std::thread::available_parallelism()
        .map(|threads| threads.get())
        .unwrap_or(4)
        .min(16)
}

struct LoadedCandidate {
    candidate: CodeFileCandidate,
    text: String,
    content_hash: String,
}

/// LOAD stage: read and content-hash every candidate in parallel. Split from
/// symbol extraction so the reindex path can compare hashes against the prior
/// generation and skip extraction for unchanged files (D4).
fn load_code_file_candidates(
    config: &IngestConfig,
    candidates: &[CodeFileCandidate],
) -> Vec<Option<LoadedCandidate>> {
    candidates
        .par_iter()
        .map(|candidate| {
            let text = fs::read_to_string(&candidate.path).ok()?;
            let content_hash = stable_hash(json!({
                "repo_id": config.repo_id,
                "path": candidate.rel_path,
                "content": text,
            }));
            Some(LoadedCandidate {
                candidate: candidate.clone(),
                text,
                content_hash,
            })
        })
        .collect()
}

fn parse_code_file_candidates(
    config: &IngestConfig,
    candidates: &[CodeFileCandidate],
) -> (Vec<IndexedFile>, u64) {
    let loaded = load_code_file_candidates(config, candidates);
    let mut read_skips = 0u64;
    let mut work = Vec::with_capacity(loaded.len());
    for item in loaded {
        match item {
            Some(loaded) => work.push(loaded),
            None => read_skips += 1,
        }
    }
    let files = work
        .par_iter_mut()
        .map(|loaded| indexed_file_from_loaded(config, loaded))
        .collect();
    (files, read_skips)
}

/// EXTRACT stage: symbol extraction over already-loaded text. Takes the text
/// out of the loaded candidate rather than cloning it.
fn indexed_file_from_loaded(config: &IngestConfig, loaded: &mut LoadedCandidate) -> IndexedFile {
    let text = std::mem::take(&mut loaded.text);
    let file_id = file_node_id(&config.repo_id, &loaded.candidate.rel_path);
    let symbols = extract_symbols(
        &config.repo_id,
        &file_id,
        &loaded.candidate.rel_path,
        &loaded.candidate.language,
        &text,
    );
    IndexedFile {
        file_id,
        rel_path: loaded.candidate.rel_path.clone(),
        language: loaded.candidate.language.clone(),
        extension: loaded.candidate.extension.clone(),
        content_hash: loaded.content_hash.clone(),
        text,
        symbols,
    }
}

fn has_null_byte_prefix(path: &Path) -> std::io::Result<bool> {
    let mut file = fs::File::open(path)?;
    let mut buffer = [0u8; BINARY_SNIFF_BYTES];
    let read = file.read(&mut buffer)?;
    Ok(buffer[..read].contains(&0))
}

fn language_stats_for(files: &[IndexedFile]) -> BTreeMap<String, LanguageIngestStats> {
    let mut stats: BTreeMap<String, LanguageIngestStats> = BTreeMap::new();
    for file in files {
        let entry = stats.entry(file.language.clone()).or_default();
        entry.files_indexed += 1;
        entry.symbols_indexed += file.symbols.len() as u64;
    }
    stats
}

fn language_stats_json(stats: &BTreeMap<String, LanguageIngestStats>) -> Value {
    Value::Object(
        stats
            .iter()
            .map(|(language, stats)| {
                (
                    language.clone(),
                    json!({
                        "files_indexed": stats.files_indexed,
                        "symbols_indexed": stats.symbols_indexed,
                    }),
                )
            })
            .collect(),
    )
}

pub(crate) fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u64::MAX as u128) as u64
}

pub fn resolve_ingest_config(input: IngestCodebaseInput) -> Result<IngestConfig, CodeIndexError> {
    let tenant_id = normalize_tenant(&input.tenant_id);
    let repo_root = if input.repo_path.trim().is_empty() {
        std::env::current_dir().map_err(|err| CodeIndexError {
            code: "code_index_cwd_error".to_string(),
            message: format!("resolve current directory: {err}"),
        })?
    } else {
        PathBuf::from(input.repo_path.trim())
    };
    let repo_root = repo_root
        .canonicalize()
        .map_err(|err| CodeIndexError::io("canonicalize repo path", &repo_root, err))?;
    if !repo_root.is_dir() {
        return Err(CodeIndexError::invalid(format!(
            "repo_path {} is not a directory",
            repo_root.display()
        )));
    }
    let repo_root_display = repo_root.display().to_string();
    let repo_id = input
        .repo_id
        .trim()
        .to_string()
        .if_empty_then(|| repo_node_id(&repo_root_display));
    let include_extensions = if input.include_extensions.is_empty() {
        default_extensions()
    } else {
        normalize_set(input.include_extensions)
    };
    let mut exclude_dirs = default_exclude_dirs();
    exclude_dirs.extend(normalize_set(input.exclude_dirs));
    let max_files = if input.max_files == 0 {
        DEFAULT_MAX_FILES
    } else {
        input.max_files.min(ABSOLUTE_MAX_FILES) as usize
    };
    let max_file_bytes = if input.max_file_bytes == 0 {
        DEFAULT_MAX_FILE_BYTES
    } else {
        input.max_file_bytes.min(ABSOLUTE_MAX_FILE_BYTES)
    };

    Ok(IngestConfig {
        tenant_id,
        repo_root,
        repo_root_display,
        repo_id,
        include_extensions,
        exclude_dirs,
        max_files,
        max_file_bytes,
        materialize_symbol_name_index: input.materialize_symbol_name_index,
        actor: input.actor.trim().to_string(),
        generation: now_ms().max(1) as u64,
        repo_url: String::new(),
        head_sha: String::new(),
    })
}

fn extract_symbols(
    repo_id: &str,
    file_id: &str,
    file_path: &str,
    language: &str,
    text: &str,
) -> Vec<IndexedSymbol> {
    let mut symbols = if should_try_tree_sitter(language, text) {
        tree_sitter_extract::extract_symbols(repo_id, file_id, file_path, language, text)
    } else {
        None
    }
    .unwrap_or_else(|| extract_line_symbols(repo_id, file_id, file_path, language, text));
    if language == "rust" {
        let references = rust_reference_index(text);
        for symbol in &mut symbols {
            if let Some(reference) = references.get(&symbol.name) {
                symbol.call_names = reference.call_names.clone();
                symbol.dependency_names = reference.dependency_names.clone();
                symbol.parser_backed = true;
            }
        }
    }
    symbols
}

fn should_try_tree_sitter(language: &str, text: &str) -> bool {
    if !matches!(
        language,
        "c" | "cpp" | "java" | "javascript" | "python" | "ruby" | "rust" | "typescript"
    ) {
        return false;
    }
    let mut symbol_lines = 0usize;
    for line in text.lines() {
        if symbol_from_line(line.trim(), language).is_some() {
            symbol_lines += 1;
            if symbol_lines > TREE_SITTER_SYMBOL_LINE_CAP {
                return false;
            }
        }
    }
    true
}

fn extract_line_symbols(
    repo_id: &str,
    file_id: &str,
    file_path: &str,
    language: &str,
    text: &str,
) -> Vec<IndexedSymbol> {
    let lines = text.lines().collect::<Vec<_>>();
    let mut raw_symbols = Vec::new();
    for (idx, raw_line) in text.lines().enumerate() {
        let line_number = idx as u64 + 1;
        let trimmed = raw_line.trim();
        let Some((kind, name)) = symbol_from_line(trimmed, language) else {
            continue;
        };
        let signature = trimmed.to_string();
        let symbol_id = symbol_node_id(repo_id, file_path, &kind, &name, line_number);
        raw_symbols.push((symbol_id, kind, name, line_number, signature));
    }

    let mut symbols = Vec::new();
    for (idx, (symbol_id, kind, name, line, signature)) in raw_symbols.iter().enumerate() {
        let next_line = raw_symbols
            .get(idx + 1)
            .map(|(_, _, _, next_line, _)| next_line.saturating_sub(1))
            .unwrap_or(lines.len() as u64);
        let body = body_between_lines(&lines, *line, next_line);
        symbols.push(IndexedSymbol {
            symbol_id: symbol_id.clone(),
            file_id: file_id.to_string(),
            file_path: file_path.to_string(),
            kind: kind.clone(),
            name: name.clone(),
            language: language.to_string(),
            line: *line,
            signature: signature.clone(),
            snippet: signature.clone(),
            body,
            trust_tier: DEFAULT_TRUST_TIER.to_string(),
            community_id: community_id(repo_id, language, kind),
            call_names: BTreeSet::new(),
            dependency_names: BTreeSet::new(),
            parser_backed: false,
        });
    }
    symbols
}

#[derive(Clone, Copy)]
struct EdgeTargetRef<'a> {
    symbol_id: &'a str,
    name: &'a str,
    file_path: &'a str,
    line: u64,
}

#[derive(Clone)]
struct SymbolNameTarget {
    symbol_id: String,
    file_path: String,
    line: u64,
}

/// D2: inverted-index edge inference. Each non-parser-backed symbol body is
/// tokenized ONCE into identifier tokens, and each token is looked up in the
/// name index, instead of scanning every distinct symbol name in the repo
/// against the body. Cost drops from `symbols x distinct_names x body_length`
/// to `symbols x body_tokens`. The Rust `syn` extraction path is unchanged;
/// the fan-out caps live in `push_symbol_edges` and apply to both branches.
///
/// `files` is the FULL current symbol set for the generation: on a full ingest
/// it is every parsed file; on an incremental reindex (D4) it is the freshly
/// parsed files plus the reconstructed carried files, so a reindex produces the
/// same edge graph as a full ingest of the same tree (carried symbols are edge
/// sources, not just targets, which is what closes the carried-to-changed and
/// carried-to-new-name seams).
fn infer_symbol_call_edges(files: &[IndexedFile], config: &IngestConfig) -> Vec<EdgeRecord> {
    let mut symbols_by_name: HashMap<&str, Vec<EdgeTargetRef<'_>>> = HashMap::new();
    for symbol in files.iter().flat_map(|file| file.symbols.iter()) {
        symbols_by_name
            .entry(symbol.name.as_str())
            .or_default()
            .push(EdgeTargetRef {
                symbol_id: &symbol.symbol_id,
                name: &symbol.name,
                file_path: &symbol.file_path,
                line: symbol.line,
            });
    }
    for targets in symbols_by_name.values_mut() {
        targets.sort_by(|a, b| {
            a.file_path
                .cmp(b.file_path)
                .then_with(|| a.line.cmp(&b.line))
                .then_with(|| a.symbol_id.cmp(b.symbol_id))
        });
        targets.dedup_by(|a, b| a.symbol_id == b.symbol_id);
    }

    let mut edges = Vec::new();
    let mut seen = BTreeSet::new();
    for symbol in files.iter().flat_map(|file| file.symbols.iter()) {
        if symbol.parser_backed {
            for name in &symbol.call_names {
                push_symbol_edges(
                    &mut edges,
                    &mut seen,
                    &symbols_by_name,
                    symbol,
                    name,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    parser_call_evidence_kind(symbol),
                    config,
                );
            }
            for name in &symbol.dependency_names {
                push_symbol_edges(
                    &mut edges,
                    &mut seen,
                    &symbols_by_name,
                    symbol,
                    name,
                    DEPENDS_ON_SYMBOL,
                    "code:edge:symbol_dependency",
                    parser_dependency_evidence_kind(symbol),
                    config,
                );
            }
        } else {
            for token in identifier_tokens(&symbol.body) {
                if token == symbol.name {
                    continue;
                }
                if !symbols_by_name.contains_key(token) {
                    continue;
                }
                push_symbol_edges(
                    &mut edges,
                    &mut seen,
                    &symbols_by_name,
                    symbol,
                    token,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    "text_body_reference",
                    config,
                );
            }
        }
    }
    edges
}

fn parser_call_evidence_kind(symbol: &IndexedSymbol) -> &'static str {
    if symbol.language == "rust" {
        "rust_ast_call"
    } else {
        "tree_sitter_call"
    }
}

fn parser_dependency_evidence_kind(symbol: &IndexedSymbol) -> &'static str {
    if symbol.language == "rust" {
        "rust_ast_dependency"
    } else {
        "tree_sitter_dependency"
    }
}

/// Split a symbol body into its distinct identifier tokens. Same character
/// class the old `body_references_name` token match used, so token-level edge
/// matches are unchanged; only the lookup direction flipped.
fn identifier_tokens(body: &str) -> BTreeSet<&str> {
    body.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '!')
        .filter(|token| !token.is_empty())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn push_symbol_edges(
    edges: &mut Vec<EdgeRecord>,
    seen: &mut BTreeSet<String>,
    symbols_by_name: &HashMap<&str, Vec<EdgeTargetRef<'_>>>,
    symbol: &IndexedSymbol,
    name: &str,
    edge_type: &str,
    edge_prefix: &str,
    evidence_kind: &str,
    config: &IngestConfig,
) {
    if name == symbol.name {
        return;
    }
    let Some(targets) = symbols_by_name.get(name) else {
        return;
    };
    if targets.len() > EDGE_NAME_BUCKET_CAP {
        return;
    }
    let mut emitted = 0usize;
    for target in targets {
        if emitted >= EDGE_TARGETS_PER_NAME_CAP {
            break;
        }
        if target.symbol_id == symbol.symbol_id {
            continue;
        }
        let symbol_edge_id = edge_id(edge_prefix, &symbol.symbol_id, target.symbol_id);
        if !seen.insert(symbol_edge_id.clone()) {
            continue;
        }
        edges.push(EdgeRecord::new(
            symbol_edge_id,
            &symbol.symbol_id,
            edge_type,
            target.symbol_id,
            json!({
                "tenant_id": config.tenant_id,
                "repo_id": config.repo_id,
                "generation": config.generation,
                "evidence": format!("{evidence_kind}: {} references {}", symbol.name, target.name),
                "source": SOURCE,
            }),
        ));
        emitted += 1;
    }
}

fn touched_names_for_file(file: &IndexedFile) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for symbol in &file.symbols {
        names.insert(symbol.name.clone());
        names.extend(observed_names_for_symbol(symbol));
    }
    names
}

fn observed_names_for_symbol(symbol: &IndexedSymbol) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    if symbol.parser_backed {
        names.extend(symbol.call_names.iter().cloned());
        names.extend(symbol.dependency_names.iter().cloned());
    } else {
        names.extend(
            identifier_tokens(&symbol.body)
                .into_iter()
                .filter(|token| *token != symbol.name)
                .map(str::to_string),
        );
    }
    names
}

fn symbol_references_any(symbol: &IndexedSymbol, names: &BTreeSet<String>) -> bool {
    if names.is_empty() {
        return false;
    }
    observed_names_for_symbol(symbol)
        .into_iter()
        .any(|name| names.contains(&name))
}

fn collect_touched_symbol_name_targets(
    store: &RedCoreGraphStore,
    config: &IngestConfig,
    prior: Option<&PriorGenerationSnapshot>,
    changed_file: &IndexedFile,
    touched_names: &BTreeSet<String>,
) -> Result<BTreeMap<String, Vec<SymbolNameTarget>>, CodeIndexError> {
    let mut targets_by_name = BTreeMap::new();
    let prior_generation = prior.map(|prior| prior.generation);
    for name in touched_names {
        let mut targets = changed_file
            .symbols
            .iter()
            .filter(|symbol| symbol.name == *name)
            .map(target_from_indexed_symbol)
            .collect::<Vec<_>>();
        if let Some(generation) = prior_generation {
            let nodes = store
                .query_nodes(
                    NodeQuery::label(CODE_SYMBOL_LABEL)
                        .with_property("tenant_id", json!(config.tenant_id))
                        .with_property("repo_id", json!(config.repo_id))
                        .with_property("name", json!(name))
                        .with_limit(100_000),
                )
                .map_err(CodeIndexError::from_store)?;
            for node in nodes {
                if property_u64(&node.properties, "generation") != Some(generation) {
                    continue;
                }
                if property_string(&node.properties, "file_path").as_deref()
                    == Some(changed_file.rel_path.as_str())
                {
                    continue;
                }
                if let Some(target) = target_from_symbol_node(&node) {
                    targets.push(target);
                }
            }
        }
        sort_symbol_name_targets(&mut targets);
        targets_by_name.insert(name.clone(), targets);
    }
    Ok(targets_by_name)
}

fn target_from_indexed_symbol(symbol: &IndexedSymbol) -> SymbolNameTarget {
    SymbolNameTarget {
        symbol_id: symbol.symbol_id.clone(),
        file_path: symbol.file_path.clone(),
        line: symbol.line,
    }
}

fn target_from_symbol_node(node: &NodeRecord) -> Option<SymbolNameTarget> {
    Some(SymbolNameTarget {
        symbol_id: property_string(&node.properties, "symbol_id")
            .unwrap_or_else(|| node.id.clone()),
        file_path: property_string(&node.properties, "file_path")?,
        line: property_u64(&node.properties, "line").unwrap_or(0),
    })
}

fn sort_symbol_name_targets(targets: &mut Vec<SymbolNameTarget>) {
    targets.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.symbol_id.cmp(&b.symbol_id))
    });
    targets.dedup_by(|a, b| a.symbol_id == b.symbol_id);
}

fn symbol_name_bucket_mutations(
    config: &IngestConfig,
    targets_by_name: &BTreeMap<String, Vec<SymbolNameTarget>>,
) -> Vec<GraphMutation> {
    targets_by_name
        .iter()
        .map(|(name, targets)| {
            let target_values = targets
                .iter()
                .map(|target| {
                    json!({
                        "symbol_id": target.symbol_id,
                        "file_path": target.file_path,
                        "line": target.line,
                    })
                })
                .collect::<Vec<_>>();
            GraphMutation::NodeUpsert(NodeRecord::new(
                symbol_name_node_id(&config.repo_id, config.generation, name),
                [CODE_SYMBOL_NAME_LABEL],
                json!({
                    "tenant_id": config.tenant_id,
                    "repo_id": config.repo_id,
                    "name": name,
                    "generation": config.generation,
                    "target_count": target_values.len(),
                    "targets": target_values,
                    "source": SOURCE,
                }),
            ))
        })
        .collect()
}

fn tombstone_existing_outgoing_symbol_edges(
    store: &RedCoreGraphStore,
    symbols: &[IndexedSymbol],
) -> Result<Vec<GraphMutation>, CodeIndexError> {
    let mut mutations = Vec::new();
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            let neighbors = store
                .neighbors(NeighborQuery {
                    node_id: symbol.symbol_id.clone(),
                    direction: Direction::Out,
                    edge_type: Some(edge_type.to_string()),
                    include_expired: false,
                })
                .map_err(CodeIndexError::from_store)?;
            for neighbor in neighbors {
                if !seen.insert(neighbor.edge_id.clone()) {
                    continue;
                }
                let Some(mut edge) = store
                    .get_edge(&neighbor.edge_id)
                    .map_err(CodeIndexError::from_store)?
                else {
                    continue;
                };
                if edge.tombstone {
                    continue;
                }
                edge.tombstone = true;
                mutations.push(GraphMutation::EdgeUpsert(edge));
            }
        }
    }
    Ok(mutations)
}

fn tombstone_existing_outgoing_symbol_edges_to_names(
    store: &RedCoreGraphStore,
    symbols: &[IndexedSymbol],
    target_names: &BTreeSet<String>,
) -> Result<Vec<GraphMutation>, CodeIndexError> {
    if target_names.is_empty() {
        return Ok(Vec::new());
    }
    let mut mutations = Vec::new();
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            let neighbors = store
                .neighbors(NeighborQuery {
                    node_id: symbol.symbol_id.clone(),
                    direction: Direction::Out,
                    edge_type: Some(edge_type.to_string()),
                    include_expired: false,
                })
                .map_err(CodeIndexError::from_store)?;
            for neighbor in neighbors {
                if !seen.insert(neighbor.edge_id.clone()) {
                    continue;
                }
                let Some(mut edge) = store
                    .get_edge(&neighbor.edge_id)
                    .map_err(CodeIndexError::from_store)?
                else {
                    continue;
                };
                if edge.tombstone {
                    continue;
                }
                let Some(target) = store
                    .get_node(&edge.to_id)
                    .map_err(CodeIndexError::from_store)?
                else {
                    continue;
                };
                let Some(target_name) = property_string(&target.properties, "name") else {
                    continue;
                };
                if !target_names.contains(&target_name) {
                    continue;
                }
                edge.tombstone = true;
                mutations.push(GraphMutation::EdgeUpsert(edge));
            }
        }
    }
    Ok(mutations)
}

fn infer_touched_outgoing_edges(
    symbols: &[IndexedSymbol],
    targets_by_name: &BTreeMap<String, Vec<SymbolNameTarget>>,
    config: &IngestConfig,
) -> Vec<EdgeRecord> {
    let mut edges = Vec::new();
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        if symbol.parser_backed {
            for name in &symbol.call_names {
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    name,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    "rust_ast_call",
                    config,
                );
            }
            for name in &symbol.dependency_names {
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    name,
                    DEPENDS_ON_SYMBOL,
                    "code:edge:symbol_dependency",
                    "rust_ast_dependency",
                    config,
                );
            }
        } else {
            for token in identifier_tokens(&symbol.body) {
                if token == symbol.name || !targets_by_name.contains_key(token) {
                    continue;
                }
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    token,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    "text_body_reference",
                    config,
                );
            }
        }
    }
    edges
}

fn infer_incoming_edges_for_names(
    symbols: &[IndexedSymbol],
    targets_by_name: &BTreeMap<String, Vec<SymbolNameTarget>>,
    target_names: &BTreeSet<String>,
    config: &IngestConfig,
) -> Vec<EdgeRecord> {
    if target_names.is_empty() {
        return Vec::new();
    }
    let mut edges = Vec::new();
    let mut seen = BTreeSet::new();
    for symbol in symbols {
        if symbol.parser_backed {
            for name in symbol
                .call_names
                .iter()
                .filter(|name| target_names.contains(*name))
            {
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    name,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    "rust_ast_call",
                    config,
                );
            }
            for name in symbol
                .dependency_names
                .iter()
                .filter(|name| target_names.contains(*name))
            {
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    name,
                    DEPENDS_ON_SYMBOL,
                    "code:edge:symbol_dependency",
                    "rust_ast_dependency",
                    config,
                );
            }
        } else {
            for token in identifier_tokens(&symbol.body) {
                if !target_names.contains(token) {
                    continue;
                }
                push_touched_symbol_edges(
                    &mut edges,
                    &mut seen,
                    targets_by_name,
                    symbol,
                    token,
                    CALLS_SYMBOL,
                    "code:edge:symbol_call",
                    "text_body_reference",
                    config,
                );
            }
        }
    }
    edges
}

#[allow(clippy::too_many_arguments)]
fn push_touched_symbol_edges(
    edges: &mut Vec<EdgeRecord>,
    seen: &mut BTreeSet<String>,
    targets_by_name: &BTreeMap<String, Vec<SymbolNameTarget>>,
    symbol: &IndexedSymbol,
    name: &str,
    edge_type: &str,
    edge_prefix: &str,
    evidence_kind: &str,
    config: &IngestConfig,
) {
    if name == symbol.name {
        return;
    }
    let Some(targets) = targets_by_name.get(name) else {
        return;
    };
    if targets.len() > EDGE_NAME_BUCKET_CAP {
        return;
    }
    let mut emitted = 0usize;
    for target in targets {
        if emitted >= EDGE_TARGETS_PER_NAME_CAP {
            break;
        }
        if target.symbol_id == symbol.symbol_id {
            continue;
        }
        let symbol_edge_id = edge_id(edge_prefix, &symbol.symbol_id, &target.symbol_id);
        if !seen.insert(symbol_edge_id.clone()) {
            continue;
        }
        edges.push(EdgeRecord::new(
            symbol_edge_id,
            &symbol.symbol_id,
            edge_type,
            &target.symbol_id,
            json!({
                "tenant_id": config.tenant_id,
                "repo_id": config.repo_id,
                "generation": config.generation,
                "evidence": format!("{evidence_kind}: {} references {name}", symbol.name),
                "source": SOURCE,
            }),
        ));
        emitted += 1;
    }
}

fn symbol_from_line(line: &str, language: &str) -> Option<(String, String)> {
    if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
        if language == "markdown" {
            return markdown_claim_from_line(line);
        }
        return None;
    }
    let normalized = strip_leading_modifiers(line);
    if language == "markdown" {
        return markdown_claim_from_line(normalized);
    }
    if language == "go" {
        return go_symbol_from_line(normalized);
    }
    let patterns: &[(&str, &str)] = match language {
        "c" | "cpp" => &[
            ("struct ", "struct"),
            ("union ", "union"),
            ("enum ", "enum"),
            ("typedef ", "type"),
        ],
        "java" => &[
            ("class ", "class"),
            ("interface ", "interface"),
            ("enum ", "enum"),
            ("record ", "record"),
        ],
        "python" => &[("def ", "function"), ("class ", "class")],
        "ruby" => &[
            ("def ", "method"),
            ("class ", "class"),
            ("module ", "module"),
        ],
        "swift" => &[
            ("func ", "function"),
            ("class ", "class"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("protocol ", "protocol"),
        ],
        "protobuf" => &[
            ("service ", "service"),
            ("message ", "message"),
            ("rpc ", "rpc"),
            ("enum ", "enum"),
        ],
        "javascript" | "typescript" => &[
            ("function ", "function"),
            ("class ", "class"),
            ("interface ", "interface"),
            ("type ", "type"),
            ("const ", "constant"),
            ("let ", "binding"),
            ("var ", "binding"),
        ],
        _ => &[
            ("fn ", "function"),
            ("struct ", "struct"),
            ("enum ", "enum"),
            ("trait ", "trait"),
            ("impl ", "impl"),
            ("mod ", "module"),
            ("macro_rules! ", "macro"),
        ],
    };

    patterns.iter().find_map(|(prefix, kind)| {
        normalized
            .strip_prefix(prefix)
            .and_then(symbol_name)
            .map(|name| ((*kind).to_string(), name))
    })
}

fn markdown_claim_from_line(line: &str) -> Option<(String, String)> {
    let cleaned = line
        .trim()
        .trim_start_matches('#')
        .trim_start_matches('-')
        .trim_start_matches('*')
        .trim();
    if cleaned.len() < 8 {
        return None;
    }
    let lower = cleaned.to_ascii_lowercase();
    let claim_like = lower.starts_with("claim:")
        || lower.contains(" references ")
        || lower.contains(" reference ")
        || lower.contains(" calls ")
        || lower.contains(" uses ")
        || lower.contains(" depends on ")
        || lower.contains(" requires ")
        || lower.contains(" is not ");
    if !claim_like {
        return None;
    }
    let hash = stable_hash(cleaned);
    Some(("claim".to_string(), format!("claim_{}", &hash[..12])))
}

fn go_symbol_from_line(line: &str) -> Option<(String, String)> {
    let line = line.trim_start();
    if let Some(rest) = line.strip_prefix("func ") {
        return go_function_symbol(rest);
    }
    if let Some(rest) = line.strip_prefix("type ") {
        return go_type_symbol(rest);
    }
    if let Some(rest) = line.strip_prefix("const ") {
        return symbol_name(rest).map(|name| ("constant".to_string(), name));
    }
    if let Some(rest) = line.strip_prefix("var ") {
        return symbol_name(rest).map(|name| ("binding".to_string(), name));
    }
    None
}

fn go_function_symbol(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim_start();
    let (kind, name_start) = if rest.starts_with('(') {
        let receiver_end = rest.find(')')?;
        ("method", rest.get(receiver_end + 1..)?.trim_start())
    } else {
        ("function", rest)
    };
    symbol_name(name_start).map(|name| (kind.to_string(), name))
}

fn go_type_symbol(rest: &str) -> Option<(String, String)> {
    let rest = rest.trim_start();
    let name = symbol_name(rest)?;
    let after_name = rest.get(name.len()..).unwrap_or_default().trim_start();
    let kind = match after_name.split_whitespace().next() {
        Some("struct") => "struct",
        Some("interface") => "interface",
        _ => "type",
    };
    Some((kind.to_string(), name))
}

fn strip_leading_modifiers(line: &str) -> &str {
    let mut rest = line.trim_start();
    loop {
        let next = rest
            .strip_prefix("pub ")
            .or_else(|| rest.strip_prefix("pub(crate) "))
            .or_else(|| rest.strip_prefix("pub(super) "))
            .or_else(|| rest.strip_prefix("export "))
            .or_else(|| rest.strip_prefix("default "))
            .or_else(|| rest.strip_prefix("async "))
            .or_else(|| rest.strip_prefix("unsafe "));
        match next {
            Some(value) => rest = value.trim_start(),
            None => return rest,
        }
    }
}

fn symbol_name(rest: &str) -> Option<String> {
    let mut name = String::new();
    for ch in rest.trim_start().chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '!' {
            name.push(ch);
        } else {
            break;
        }
    }
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[derive(Clone, Debug, Default)]
struct RustSymbolReferences {
    call_names: BTreeSet<String>,
    dependency_names: BTreeSet<String>,
}

fn rust_reference_index(text: &str) -> HashMap<String, RustSymbolReferences> {
    let Ok(file) = syn::parse_file(text) else {
        return HashMap::new();
    };
    let mut index = HashMap::new();
    for item in &file.items {
        match item {
            syn::Item::Fn(item_fn) => {
                index.insert(
                    item_fn.sig.ident.to_string(),
                    rust_references_for_signature(&item_fn.sig, Some(&item_fn.block)),
                );
            }
            syn::Item::Struct(item_struct) => {
                let mut refs = RustSymbolReferences::default();
                collect_field_dependencies(&item_struct.fields, &mut refs.dependency_names);
                index.insert(item_struct.ident.to_string(), refs);
            }
            syn::Item::Enum(item_enum) => {
                let mut refs = RustSymbolReferences::default();
                for variant in &item_enum.variants {
                    collect_field_dependencies(&variant.fields, &mut refs.dependency_names);
                }
                index.insert(item_enum.ident.to_string(), refs);
            }
            syn::Item::Trait(item_trait) => {
                let mut refs = RustSymbolReferences::default();
                for item in &item_trait.items {
                    if let syn::TraitItem::Fn(function) = item {
                        refs.dependency_names.extend(
                            rust_references_for_signature(&function.sig, None).dependency_names,
                        );
                    }
                }
                index.insert(item_trait.ident.to_string(), refs);
            }
            syn::Item::Impl(item_impl) => {
                let impl_name = item_impl
                    .trait_
                    .as_ref()
                    .and_then(|(_, path, _)| path.segments.last())
                    .map(|segment| segment.ident.to_string())
                    .or_else(|| type_tail_name(&item_impl.self_ty));
                if let Some(name) = impl_name {
                    let mut refs = RustSymbolReferences::default();
                    collect_type_names(&item_impl.self_ty, &mut refs.dependency_names);
                    for item in &item_impl.items {
                        if let syn::ImplItem::Fn(function) = item {
                            let method_refs =
                                rust_references_for_signature(&function.sig, Some(&function.block));
                            refs.call_names.extend(method_refs.call_names.clone());
                            refs.dependency_names
                                .extend(method_refs.dependency_names.clone());
                            index.insert(function.sig.ident.to_string(), method_refs);
                        }
                    }
                    index.entry(name).or_insert(refs);
                }
            }
            _ => {}
        }
    }
    index
}

fn rust_references_for_signature(
    signature: &syn::Signature,
    block: Option<&syn::Block>,
) -> RustSymbolReferences {
    let mut refs = RustSymbolReferences::default();
    for input in &signature.inputs {
        if let syn::FnArg::Typed(arg) = input {
            collect_type_names(&arg.ty, &mut refs.dependency_names);
        }
    }
    if let syn::ReturnType::Type(_, ty) = &signature.output {
        collect_type_names(ty, &mut refs.dependency_names);
    }
    if let Some(block) = block {
        let mut collector = RustCallCollector::default();
        collector.visit_block(block);
        refs.call_names = collector.call_names;
    }
    refs.dependency_names.remove(&signature.ident.to_string());
    refs
}

fn collect_field_dependencies(fields: &syn::Fields, out: &mut BTreeSet<String>) {
    for field in fields {
        collect_type_names(&field.ty, out);
    }
}

fn collect_type_names(ty: &syn::Type, out: &mut BTreeSet<String>) {
    let mut collector = RustTypeCollector { names: out };
    collector.visit_type(ty);
}

fn type_tail_name(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path
            .path
            .segments
            .last()
            .map(|segment| segment.ident.to_string()),
        _ => None,
    }
}

#[derive(Default)]
struct RustCallCollector {
    call_names: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for RustCallCollector {
    fn visit_expr_call(&mut self, node: &'ast syn::ExprCall) {
        if let syn::Expr::Path(path) = node.func.as_ref() {
            insert_path_tail_name(&path.path, &mut self.call_names);
        }
        syn::visit::visit_expr_call(self, node);
    }

    fn visit_expr_method_call(&mut self, node: &'ast syn::ExprMethodCall) {
        self.call_names.insert(node.method.to_string());
        syn::visit::visit_expr_method_call(self, node);
    }
}

struct RustTypeCollector<'a> {
    names: &'a mut BTreeSet<String>,
}

impl<'ast> Visit<'ast> for RustTypeCollector<'_> {
    fn visit_type_path(&mut self, node: &'ast syn::TypePath) {
        insert_path_tail_name(&node.path, self.names);
        syn::visit::visit_type_path(self, node);
    }
}

fn insert_path_tail_name(path: &syn::Path, out: &mut BTreeSet<String>) {
    if let Some(segment) = path.segments.last() {
        let name = segment.ident.to_string();
        if !matches!(
            name.as_str(),
            "Self" | "str" | "usize" | "u64" | "u32" | "i64" | "i32" | "bool"
        ) {
            out.insert(name);
        }
    }
}

fn latest_repo_generations(
    store: &RedCoreGraphStore,
) -> Result<HashMap<String, u64>, CodeIndexError> {
    let repos = store
        .query_nodes(NodeQuery::label(CODE_REPO_LABEL).with_limit(100_000))
        .map_err(CodeIndexError::from_store)?;
    Ok(repos
        .into_iter()
        .filter_map(|node| {
            Some((
                property_string(&node.properties, "repo_id")?,
                property_u64(&node.properties, "latest_generation")?,
            ))
        })
        .collect())
}

fn resolve_file_by_path(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    input: &CodeContextInput,
) -> Result<String, CodeIndexError> {
    if input.file_path.trim().is_empty() {
        return Err(CodeIndexError::invalid(
            "node_id or file_path is required for code context",
        ));
    }
    let mut query = NodeQuery::label(CODE_FILE_LABEL)
        .with_property("tenant_id", json!(tenant_id))
        .with_property("path", json!(input.file_path.trim()))
        .with_limit(100);
    if !input.repo_id.trim().is_empty() {
        query = query.with_property("repo_id", json!(input.repo_id.trim()));
    }
    let files = store
        .query_nodes(query)
        .map_err(CodeIndexError::from_store)?;
    files
        .into_iter()
        .next()
        .map(|node| node.id)
        .ok_or_else(|| CodeIndexError::invalid("code file path was not found in the index"))
}

fn symbols_for_file(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    file_id: &str,
    repo_id: &str,
    latest: &HashMap<String, u64>,
) -> Result<Vec<CodeSymbolRecord>, CodeIndexError> {
    let latest_generation = latest.get(repo_id);
    let symbols = store
        .query_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("file_id", json!(file_id))
                .with_limit(1_000),
        )
        .map_err(CodeIndexError::from_store)?;
    let mut out = symbols
        .into_iter()
        .filter(|node| node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id))
        .filter(|node| node.properties.get("repo_id").and_then(Value::as_str) == Some(repo_id))
        .filter(|node| match latest_generation {
            Some(generation) => property_u64(&node.properties, "generation") == Some(*generation),
            None => true,
        })
        .filter_map(|node| symbol_record_from_node(&node))
        .collect::<Vec<_>>();
    for symbol in &mut out {
        enrich_symbol_graph(store, symbol, latest)?;
    }
    out.sort_by(|a, b| a.line.cmp(&b.line).then_with(|| a.name.cmp(&b.name)));
    Ok(out)
}

fn recognize_indexed_symbols(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    input: &RecognizeCodeInput,
) -> Result<Vec<CodeSymbolRecord>, CodeIndexError> {
    let mut query = NodeQuery::label(CODE_SYMBOL_LABEL)
        .with_property("tenant_id", json!(tenant_id))
        .with_limit(100_000);
    if !input.repo_id.trim().is_empty() {
        query = query.with_property("repo_id", json!(input.repo_id.trim()));
    }
    if !input.file_path.trim().is_empty() {
        query = query.with_property("file_path", json!(input.file_path.trim()));
    }
    let latest = latest_repo_generations(store)?;
    let mut out = store
        .query_nodes(query)
        .map_err(CodeIndexError::from_store)?
        .into_iter()
        .filter(|node| match property_string(&node.properties, "repo_id") {
            Some(repo_id) => latest
                .get(&repo_id)
                .map(|generation| property_u64(&node.properties, "generation") == Some(*generation))
                .unwrap_or(true),
            None => false,
        })
        .filter_map(|node| symbol_record_from_node(&node))
        .collect::<Vec<_>>();
    for symbol in &mut out {
        enrich_symbol_graph(store, symbol, &latest)?;
    }
    out.sort_by(|a, b| {
        a.file_path
            .cmp(&b.file_path)
            .then_with(|| a.line.cmp(&b.line))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(out)
}

fn resolve_symbol_node(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    node_id: &str,
    query: &str,
    repo_id: &str,
) -> Result<Option<NodeRecord>, CodeIndexError> {
    if !node_id.trim().is_empty() {
        let node = store
            .get_node(node_id.trim())
            .map_err(CodeIndexError::from_store)?;
        return Ok(node.filter(|node| {
            node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL)
                && node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id)
        }));
    }
    if query.trim().is_empty() {
        return Ok(None);
    }
    let mut node_query = NodeQuery::label(CODE_SYMBOL_LABEL).with_limit(100_000);
    if !repo_id.trim().is_empty() {
        node_query = node_query.with_property("repo_id", json!(repo_id.trim()));
    }
    let latest = latest_repo_generations(store)?;
    let query_terms = query_terms(query);
    let mut scored = store
        .query_nodes(node_query)
        .map_err(CodeIndexError::from_store)?
        .into_iter()
        .filter(|node| node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant_id))
        .filter(|node| match property_string(&node.properties, "repo_id") {
            Some(repo_id) => latest
                .get(&repo_id)
                .map(|generation| property_u64(&node.properties, "generation") == Some(*generation))
                .unwrap_or(true),
            None => false,
        })
        .filter_map(|node| {
            hit_from_node(&node).map(|hit| (node, score_hit(&hit, query, &query_terms)))
        })
        .filter(|(_, score)| *score > 0.0)
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    Ok(scored.into_iter().next().map(|(node, _)| node))
}

/// A code node is "current" when its generation matches its repo's latest
/// generation. Incremental reindex re-emits every live edge at the new
/// generation but leaves moved-target stale edges pointing at superseded
/// (old-generation) symbol nodes; filtering traversal neighbors by this
/// retires those stale edges without an edge-delete mutation (the store has
/// none) and without rewriting the read path's generation model.
fn node_is_current_generation(node: &NodeRecord, latest: &HashMap<String, u64>) -> bool {
    match property_string(&node.properties, "repo_id") {
        Some(repo_id) => match latest.get(&repo_id) {
            Some(generation) => property_u64(&node.properties, "generation") == Some(*generation),
            None => true,
        },
        None => true,
    }
}

fn expand_symbol_edges(
    store: &RedCoreGraphStore,
    focus_id: &str,
    max_depth: usize,
    limit: usize,
    latest: &HashMap<String, u64>,
    related_ids: &mut BTreeSet<String>,
    edges: &mut Vec<CodeGraphEdgeRecord>,
) -> Result<(), CodeIndexError> {
    let mut frontier = vec![(focus_id.to_string(), 0usize)];
    related_ids.insert(focus_id.to_string());
    let mut seen_edges = BTreeSet::new();
    while let Some((node_id, depth)) = frontier.pop() {
        if depth >= max_depth || related_ids.len() >= limit.saturating_add(1) {
            continue;
        }
        for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
            for direction in [Direction::Out, Direction::In] {
                let neighbors = store
                    .neighbors(NeighborQuery {
                        node_id: node_id.clone(),
                        direction,
                        edge_type: Some(edge_type.to_string()),
                        include_expired: false,
                    })
                    .map_err(CodeIndexError::from_store)?;
                for neighbor in neighbors {
                    // Skip stale-generation neighbors (moved/superseded targets).
                    let Some(neighbor_node) = store
                        .get_node(&neighbor.node_id)
                        .map_err(CodeIndexError::from_store)?
                    else {
                        continue;
                    };
                    if !node_is_current_generation(&neighbor_node, latest) {
                        continue;
                    }
                    if !seen_edges.insert(neighbor.edge_id.clone()) {
                        continue;
                    }
                    if let Some(edge) = store
                        .get_edge(&neighbor.edge_id)
                        .map_err(CodeIndexError::from_store)?
                    {
                        if let Some(record) = graph_edge_record(store, &edge, latest)? {
                            edges.push(record);
                        }
                    }
                    if related_ids.insert(neighbor.node_id.clone()) && related_ids.len() <= limit {
                        frontier.push((neighbor.node_id, depth + 1));
                    }
                }
            }
        }
    }
    edges.sort_by(|a, b| {
        a.from_name
            .cmp(&b.from_name)
            .then_with(|| a.to_name.cmp(&b.to_name))
    });
    Ok(())
}

fn enrich_symbol_graph(
    store: &RedCoreGraphStore,
    symbol: &mut CodeSymbolRecord,
    latest: &HashMap<String, u64>,
) -> Result<(), CodeIndexError> {
    symbol.callees =
        neighbor_symbol_names(store, &symbol.node_id, Direction::Out, CALLS_SYMBOL, latest)?;
    symbol.callers =
        neighbor_symbol_names(store, &symbol.node_id, Direction::In, CALLS_SYMBOL, latest)?;
    symbol.dependencies = neighbor_symbol_names(
        store,
        &symbol.node_id,
        Direction::Out,
        DEPENDS_ON_SYMBOL,
        latest,
    )?;
    symbol.dependents = neighbor_symbol_names(
        store,
        &symbol.node_id,
        Direction::In,
        DEPENDS_ON_SYMBOL,
        latest,
    )?;
    Ok(())
}

fn neighbor_symbol_names(
    store: &RedCoreGraphStore,
    node_id: &str,
    direction: Direction,
    edge_type: &str,
    latest: &HashMap<String, u64>,
) -> Result<Vec<String>, CodeIndexError> {
    let mut names = store
        .neighbors(NeighborQuery {
            node_id: node_id.to_string(),
            direction,
            edge_type: Some(edge_type.to_string()),
            include_expired: false,
        })
        .map_err(CodeIndexError::from_store)?
        .into_iter()
        .filter_map(|hit| store.get_node(&hit.node_id).ok().flatten())
        .filter(|node| node_is_current_generation(node, latest))
        .filter_map(|node| property_string(&node.properties, "name"))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

fn graph_edge_record(
    store: &RedCoreGraphStore,
    edge: &EdgeRecord,
    latest: &HashMap<String, u64>,
) -> Result<Option<CodeGraphEdgeRecord>, CodeIndexError> {
    let Some(from) = store
        .get_node(&edge.from_id)
        .map_err(CodeIndexError::from_store)?
    else {
        return Ok(None);
    };
    let Some(to) = store
        .get_node(&edge.to_id)
        .map_err(CodeIndexError::from_store)?
    else {
        return Ok(None);
    };
    // Both endpoints must be current: a stale (moved) target retires the edge.
    if !node_is_current_generation(&from, latest) || !node_is_current_generation(&to, latest) {
        return Ok(None);
    }
    Ok(Some(CodeGraphEdgeRecord {
        from_node_id: edge.from_id.clone(),
        to_node_id: edge.to_id.clone(),
        edge_type: edge.edge_type.clone(),
        from_name: property_string(&from.properties, "name")
            .unwrap_or_else(|| edge.from_id.clone()),
        to_name: property_string(&to.properties, "name").unwrap_or_else(|| edge.to_id.clone()),
        evidence: property_string(&edge.properties, "evidence").unwrap_or_default(),
    }))
}

fn hit_from_node(node: &NodeRecord) -> Option<CodeHitRecord> {
    Some(CodeHitRecord {
        node_id: node.id.clone(),
        repo_id: property_string(&node.properties, "repo_id")?,
        file_id: property_string(&node.properties, "file_id")?,
        file_path: property_string(&node.properties, "file_path")?,
        kind: property_string(&node.properties, "kind")?,
        name: property_string(&node.properties, "name")?,
        language: property_string(&node.properties, "language").unwrap_or_default(),
        line: property_u64(&node.properties, "line").unwrap_or(0),
        snippet: property_string(&node.properties, "snippet").unwrap_or_default(),
        score: 0.0,
        trust_tier: property_string(&node.properties, "trust_tier")
            .unwrap_or_else(|| DEFAULT_TRUST_TIER.to_string()),
        community_id: property_string(&node.properties, "community_id").unwrap_or_default(),
    })
}

fn symbol_record_from_node(node: &NodeRecord) -> Option<CodeSymbolRecord> {
    Some(CodeSymbolRecord {
        node_id: node.id.clone(),
        repo_id: property_string(&node.properties, "repo_id")?,
        file_id: property_string(&node.properties, "file_id")?,
        file_path: property_string(&node.properties, "file_path")?,
        kind: property_string(&node.properties, "kind")?,
        name: property_string(&node.properties, "name")?,
        language: property_string(&node.properties, "language").unwrap_or_default(),
        line: property_u64(&node.properties, "line").unwrap_or(0),
        signature: property_string(&node.properties, "signature").unwrap_or_default(),
        snippet: property_string(&node.properties, "snippet").unwrap_or_default(),
        trust_tier: property_string(&node.properties, "trust_tier")
            .unwrap_or_else(|| DEFAULT_TRUST_TIER.to_string()),
        community_id: property_string(&node.properties, "community_id").unwrap_or_default(),
        callers: Vec::new(),
        callees: Vec::new(),
        dependencies: Vec::new(),
        dependents: Vec::new(),
    })
}

fn score_hit(hit: &CodeHitRecord, query: &str, terms: &[String]) -> f64 {
    if query.is_empty() {
        return 0.1;
    }
    let query = query.to_ascii_lowercase();
    let name = hit.name.to_ascii_lowercase();
    let kind = hit.kind.to_ascii_lowercase();
    let path = hit.file_path.to_ascii_lowercase();
    let snippet = hit.snippet.to_ascii_lowercase();
    let mut score = 0.0;
    if name == query {
        score += 8.0;
    }
    if name.contains(&query) {
        score += 5.0;
    }
    if path.contains(&query) {
        score += 2.0;
    }
    if kind.contains(&query) {
        score += 1.0;
    }
    if snippet.contains(&query) {
        score += 1.0;
    }
    for term in terms {
        if name.contains(term) {
            score += 2.0;
        }
        if path.contains(term) {
            score += 0.75;
        }
        if snippet.contains(term) {
            score += 0.5;
        }
    }
    score
}

fn line_context(
    text: &str,
    target_line: u64,
    before: u64,
    after: u64,
    max_chars: usize,
) -> (u64, u64, String) {
    let lines = text.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return (0, 0, String::new());
    }
    let target = target_line.max(1).min(lines.len() as u64);
    let start = target.saturating_sub(before).max(1);
    let end = (target + after).min(lines.len() as u64);
    let mut context = String::new();
    for line_no in start..=end {
        if let Some(line) = lines.get(line_no as usize - 1) {
            let next = format!("{line_no}: {line}\n");
            if context.len() + next.len() > max_chars {
                break;
            }
            context.push_str(&next);
        }
    }
    (start, end, context)
}

fn body_between_lines(lines: &[&str], start_line: u64, end_line: u64) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let start = start_line.max(1) as usize - 1;
    let end = end_line.min(lines.len() as u64) as usize;
    lines.get(start..end).unwrap_or(&[]).join("\n")
}

fn community_id(repo_id: &str, language: &str, kind: &str) -> String {
    format!(
        "code:community:{}",
        stable_hash(json!({
            "repo_id": repo_id,
            "language": language,
            "kind": kind,
        }))
    )
}

struct Receipt {
    receipt_hash: String,
    receipt_json: String,
    graph_version: u64,
}

fn record_receipt(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    operation: &str,
    payload: &Value,
) -> Result<Receipt, CodeIndexError> {
    let receipt_hash = stable_hash(json!({
        "tenant_id": tenant_id,
        "operation": operation,
        "payload": payload,
        "recorded_at_ms": now_ms(),
    }));
    let receipt_json = serde_json::to_string(&json!({
        "receipt_hash": receipt_hash,
        "tenant_id": tenant_id,
        "operation": operation,
        "payload": payload,
        "source": SOURCE,
    }))
    .unwrap_or_else(|_| "{}".to_string());
    let node = NodeRecord::new(
        format!("code:receipt:{receipt_hash}"),
        [CODE_RECEIPT_LABEL],
        json!({
            "tenant_id": tenant_id,
            "operation": operation,
            "receipt_hash": receipt_hash,
            "receipt_json": receipt_json,
            "recorded_at_ms": now_ms(),
            "source": SOURCE,
        }),
    );
    let transaction = store
        .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(node)]))
        .map_err(CodeIndexError::from_store)?;
    Ok(Receipt {
        receipt_hash,
        receipt_json,
        graph_version: transaction.graph_version,
    })
}

fn code_index_data_dir() -> PathBuf {
    if let Ok(raw) = std::env::var("THEOREM_CODE_INDEX_DIR") {
        return PathBuf::from(raw);
    }
    if let Ok(raw) = std::env::var("THEOREM_GRPC_CODE_INDEX_DIR") {
        return PathBuf::from(raw);
    }
    if let Ok(raw) = std::env::var("THEOREM_GRPC_DATA_DIR") {
        return PathBuf::from(raw).join("code-index");
    }
    PathBuf::from("data/theorem-grpc/code-index")
}

fn code_index_options() -> RedCoreOptions {
    let mut options = RedCoreOptions::default();
    if let Ok(raw) = std::env::var("THEOREM_CODE_INDEX_DURABILITY")
        .or_else(|_| std::env::var("THEOREM_GRPC_REDCORE_DURABILITY"))
    {
        options.durability = RedCoreDurability::parse(&raw);
    }
    if let Ok(raw) = std::env::var("THEOREM_CODE_INDEX_SNAPSHOT_INTERVAL") {
        if let Ok(value) = raw.parse::<u64>() {
            options.snapshot_interval_writes = value;
        }
    }
    options
}

pub(crate) fn normalize_tenant(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "theorem".to_string()
    } else {
        trimmed.to_string()
    }
}

fn normalize_set(values: Vec<String>) -> BTreeSet<String> {
    values
        .into_iter()
        .map(|value| value.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|value| !value.is_empty())
        .collect()
}

fn default_extensions() -> BTreeSet<String> {
    [
        "c", "cc", "cpp", "cxx", "h", "hh", "hpp", "hxx", "java", "rb", "rs", "go", "swift", "py",
        "ts", "tsx", "js", "jsx", "mjs", "cjs", "proto", "toml", "md", "json",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn default_exclude_dirs() -> BTreeSet<String> {
    [
        ".git",
        ".idea",
        ".next",
        ".venv",
        "__pycache__",
        "build",
        "coverage",
        "dist",
        "node_modules",
        "out",
        "target",
        "vendor",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn should_skip_dir(name: &str, excluded: &BTreeSet<String>) -> bool {
    let lower = name.to_ascii_lowercase();
    excluded.contains(&lower) || lower.starts_with('.')
}

fn should_skip_file_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "bun.lock"
            | "bun.lockb"
            | "cargo.lock"
            | "composer.lock"
            | "deno.lock"
            | "package-lock.json"
            | "pdm.lock"
            | "pnpm-lock.yaml"
            | "poetry.lock"
            | "uv.lock"
            | "yarn.lock"
    ) || lower.ends_with(".min.css")
        || lower.ends_with(".min.js")
        || lower.ends_with(".map")
}

fn extension_for(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn language_for_extension(extension: &str) -> &str {
    match extension {
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hh" | "hpp" | "hxx" => "cpp",
        "go" => "go",
        "java" => "java",
        "py" => "python",
        "rb" => "ruby",
        "swift" => "swift",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "proto" => "protobuf",
        "rs" => "rust",
        "toml" => "toml",
        "md" => "markdown",
        "json" => "json",
        _ => "unknown",
    }
}

fn relative_path(root: &Path, path: &Path) -> Result<String, CodeIndexError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|err| CodeIndexError::io("resolve relative path", path, err))?;
    Ok(relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

fn safe_source_file_path(path: &str) -> Result<String, CodeIndexError> {
    let raw = Path::new(path.trim());
    if raw.as_os_str().is_empty() {
        return Err(CodeIndexError::invalid("source file path is required"));
    }
    let mut parts = Vec::new();
    for component in raw.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    CodeIndexError::invalid(format!("non-utf8 source path {path:?}"))
                })?;
                parts.push(value.to_string());
            }
            Component::CurDir => {}
            _ => {
                return Err(CodeIndexError::invalid(format!(
                    "unsafe source file path {path:?}"
                )));
            }
        }
    }
    if parts.is_empty() {
        return Err(CodeIndexError::invalid("source file path is required"));
    }
    Ok(parts.join("/"))
}

fn repo_node_id(repo_root: &str) -> String {
    format!(
        "code:repo:{}",
        stable_hash(json!({ "repo_root": repo_root }))
    )
}

fn file_node_id(repo_id: &str, path: &str) -> String {
    format!(
        "code:file:{}",
        stable_hash(json!({ "repo_id": repo_id, "path": path }))
    )
}

/// D3: content-addressed id for a file's text side record. Keyed by tenant +
/// content_hash (which already folds in repo_id and path), so an unchanged
/// file shares one record across generations and a reindex never rewrites it.
fn file_text_node_id(tenant_id: &str, content_hash: &str) -> String {
    format!(
        "code:filetext:{}",
        stable_hash(json!({ "tenant_id": tenant_id, "content_hash": content_hash }))
    )
}

/// Batch-read CodeFileText side records by content hash for one tenant. The
/// incremental-reindex carried-text loader wraps this with the store handle
/// (Arc for the runtime, &store for the in-store path), so prepare stays
/// store-agnostic and the runtime takes the lock for exactly one batch read.
pub(crate) fn load_file_texts(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    content_hashes: &[String],
) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(content_hashes.len());
    for hash in content_hashes {
        if let Ok(Some(node)) = store.get_node(&file_text_node_id(tenant_id, hash)) {
            if let Some(text) = property_string(&node.properties, "text") {
                out.insert(hash.clone(), text);
            }
        }
    }
    out
}

/// Read a file's text from its CodeFileText side record. Falls back to the
/// legacy `text` property on the CodeFile node for generations ingested before
/// the side record existed.
fn file_text_for(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    file_node: &NodeRecord,
) -> Result<String, CodeIndexError> {
    if let Some(content_hash) = property_string(&file_node.properties, "content_hash") {
        if let Some(node) = store
            .get_node(&file_text_node_id(tenant_id, &content_hash))
            .map_err(CodeIndexError::from_store)?
        {
            if let Some(text) = property_string(&node.properties, "text") {
                return Ok(text);
            }
        }
    }
    Ok(property_string(&file_node.properties, "text").unwrap_or_default())
}

fn symbol_node_id(repo_id: &str, path: &str, kind: &str, name: &str, line: u64) -> String {
    format!(
        "code:symbol:{}",
        stable_hash(json!({
            "repo_id": repo_id,
            "path": path,
            "kind": kind,
            "name": name,
            "line": line,
        }))
    )
}

fn symbol_name_node_id(repo_id: &str, generation: u64, name: &str) -> String {
    format!(
        "code:symbol_name:{}",
        stable_hash(json!({
            "repo_id": repo_id,
            "generation": generation,
            "name": name,
        }))
    )
}

fn edge_id(prefix: &str, from: &str, to: &str) -> String {
    format!(
        "{prefix}:{}",
        stable_hash(json!({ "from": from, "to": to }))
    )
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

fn bounded_limit(raw: u64) -> usize {
    if raw == 0 {
        DEFAULT_LIMIT
    } else {
        raw.min(100) as usize
    }
}

fn nonzero_or(raw: u64, fallback: u64) -> u64 {
    if raw == 0 {
        fallback
    } else {
        raw
    }
}

fn property_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn property_u64(properties: &Value, key: &str) -> Option<u64> {
    properties.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_i64().and_then(|v| v.try_into().ok()))
    })
}

fn property_bool(properties: &Value, key: &str) -> bool {
    properties
        .get(key)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Read a JSON string array property back into a `BTreeSet<String>` (the shape
/// `call_names` / `dependency_names` are persisted in).
fn property_string_set(properties: &Value, key: &str) -> BTreeSet<String> {
    properties
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn parse_optional_json(raw: &str) -> Result<Value, CodeIndexError> {
    if raw.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(raw).map_err(|error| {
        CodeIndexError::invalid(format!(
            "use_json must be valid JSON when provided: {error}"
        ))
    })
}

trait IfEmptyThen {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String;
}

impl IfEmptyThen for String {
    fn if_empty_then(self, fallback: impl FnOnce() -> String) -> String {
        if self.is_empty() {
            fallback()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use rustyred_code_embedding::{cosine_similarity, CodeEmbedder, CodeEmbeddingError};

    use super::*;

    fn test_options() -> RedCoreOptions {
        RedCoreOptions {
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 100,
            strict_acid: true,
        }
    }

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "theorem-code-index-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write_fixture_repo() -> (PathBuf, PathBuf) {
        let repo_dir = unique_dir("repo");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::write(
            repo_dir.join("src/lib.rs"),
            "pub struct SearchKernel {}\n\npub struct QueryPlan {\n    kernel: SearchKernel,\n}\n\npub fn helper_len(query: &str) -> usize {\n    query.len()\n}\n\npub fn search_code(query: &str) -> usize {\n    helper_len(query)\n}\n\npub fn build_plan(kernel: SearchKernel) -> QueryPlan {\n    QueryPlan { kernel }\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/app.py"),
            "class SearchAdapter:\n    pass\n\ndef code_context():\n    return 'ok'\n",
        )
        .unwrap();
        let store_dir = unique_dir("store");
        (repo_dir, store_dir)
    }

    fn write_epistemic_fixture_repo() -> PathBuf {
        let repo_dir = unique_dir("epistemic-repo");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::write(
            repo_dir.join("README.md"),
            "Claim: cache is enabled\nClaim: cache is not enabled\nClaim: docs references MissingCacheAdapter\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/lib.rs"),
            "pub fn cache_entry() -> bool {\n    true\n}\n",
        )
        .unwrap();
        repo_dir
    }

    #[test]
    fn ingest_returns_instant_epistemic_readout_with_drift_and_bounded_pairs() {
        let repo_dir = write_epistemic_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let output = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:epistemic-fixture".to_string(),
                actor: "test".to_string(),
                ..IngestCodebaseInput::default()
            },
        )
        .unwrap();

        let readout = &output.epistemic_readout;
        assert_eq!(readout["repo_id"], "repo:epistemic-fixture");
        assert!(
            readout["readout"]["contradictions"]
                .as_array()
                .is_some_and(|items| !items.is_empty()),
            "planted README contradiction appears as an Undercuts relation: {readout}"
        );
        assert!(
            readout["drift"].as_array().is_some_and(|items| items
                .iter()
                .any(|item| { item["missing_target"].as_str() == Some("MissingCacheAdapter") })),
            "missing code entity appears as drift: {readout}"
        );
        let shadows = readout["readout"]["shadows"].as_array().unwrap();
        assert!(shadows.iter().any(|shadow| {
            shadow["source_kind"].as_str() == Some("structural")
                && shadow["support_in_degree"].as_u64().is_some()
                && shadow["attack_in_degree"].as_u64().is_some()
                && shadow["field_provenance"]["grounded_extension_status"]["source_kind"].as_str()
                    == Some("structural")
        }));
        let checked = readout["checked_pair_count"].as_u64().unwrap();
        let bound = (shadows.len() * DEFAULT_CODE_EPISTEMIC_TOP_K) as u64;
        assert!(
            checked <= bound,
            "candidate checks stay bounded by k times claim count: {checked} <= {bound}"
        );

        fs::remove_dir_all(&repo_dir).ok();
    }

    fn write_go_fixture_repo() -> (PathBuf, PathBuf) {
        let repo_dir = unique_dir("go-repo");
        fs::create_dir_all(repo_dir.join("internal")).unwrap();
        fs::write(
            repo_dir.join("go.mod"),
            "module example.com/boltbrowser\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(repo_dir.join("README.md"), "# boltbrowser fixture\n").unwrap();
        fs::write(
            repo_dir.join("main.go"),
            "package main\n\nconst AppName = \"boltbrowser\"\n\ntype Browser struct {\n    title string\n}\n\ntype Screen interface {\n    Draw() string\n}\n\nfunc main() {\n    browser := Browser{title: AppName}\n    _ = browser.Draw()\n}\n\nfunc (browser Browser) Draw() string {\n    return browser.title\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("internal/store.go"),
            "package internal\n\ntype BoltStore struct {}\n\nfunc OpenStore(path string) BoltStore {\n    return BoltStore{}\n}\n",
        )
        .unwrap();
        let store_dir = unique_dir("store");
        (repo_dir, store_dir)
    }

    /// Turn a fixture dir into a real git repo with one commit, so it can be
    /// cloned via `file://` with no network. Returns false if git is absent.
    fn init_git_fixture(dir: &Path) -> bool {
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false)
        };
        run(&["init", "--quiet"])
            && run(&["config", "user.email", "fixture@example.com"])
            && run(&["config", "user.name", "Fixture"])
            && run(&["add", "."])
            && run(&["commit", "--quiet", "-m", "fixture"])
    }

    #[test]
    fn repo_fetch_caps_cover_large_public_repos_without_unbounded_fetches() {
        const {
            assert!(repo_fetch::DEFAULT_MAX_TOTAL_BYTES >= 771_959_844);
        }

        let servo_sized = RepoFetchCaps::from_requested(771_959_844);
        assert_eq!(servo_sized.max_total_bytes, 771_959_844);
        assert_eq!(
            servo_sized.clone_timeout_ms,
            RepoFetchCaps::default().clone_timeout_ms
        );

        let capped = RepoFetchCaps::from_requested(u64::MAX);
        assert_eq!(capped.max_total_bytes, repo_fetch::ABSOLUTE_MAX_TOTAL_BYTES);
    }

    #[test]
    fn resolve_ingest_config_honors_explicit_large_budgets() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let config = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            max_files: 50_000,
            max_file_bytes: 10_000_000,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(config.max_files, 50_000);
        assert_eq!(config.max_file_bytes, 10_000_000);

        let capped = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            max_files: u64::MAX,
            max_file_bytes: u64::MAX,
            ..Default::default()
        })
        .unwrap();
        assert_eq!(capped.max_files, ABSOLUTE_MAX_FILES as usize);
        assert_eq!(capped.max_file_bytes, ABSOLUTE_MAX_FILE_BYTES);

        fs::remove_dir_all(&repo_dir).ok();
    }

    #[test]
    fn fetch_repo_rejects_unsafe_or_unsupported_urls() {
        let caps = RepoFetchCaps::default();
        assert!(fetch_repo("", &caps).is_err());
        // A leading '-' would be parsed by git as a flag, not a url.
        assert!(fetch_repo("--upload-pack=evil", &caps).is_err());
        assert!(fetch_repo("ftp://example.com/repo.git", &caps).is_err());
        assert!(fetch_repo("/etc/passwd", &caps).is_err());
    }

    #[test]
    fn ingest_from_url_clones_local_repo_and_indexes() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        if !init_git_fixture(&repo_dir) {
            // git unavailable in this environment; skip rather than fail.
            fs::remove_dir_all(&repo_dir).ok();
            return;
        }
        let url = format!("file://{}", repo_dir.display());
        let mut store = RedCoreGraphStore::memory();
        let out = ingest_codebase_from_url_in_store(
            &mut store,
            &url,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                ..Default::default()
            },
            &RepoFetchCaps::default(),
        )
        .unwrap();
        // Only the two source files are recognized code; `.git` is excluded.
        assert_eq!(out.files_indexed, 2);
        assert!(out.repo_id.starts_with("repo:"));

        let symbols = store
            .query_nodes(NodeQuery::label(CODE_SYMBOL_LABEL))
            .unwrap();
        assert!(symbols
            .iter()
            .any(|node| node.properties.get("name").and_then(Value::as_str) == Some("helper_len")));

        fs::remove_dir_all(&repo_dir).ok();
    }

    #[test]
    fn default_ingest_indexes_go_symbols_and_searches_main() {
        let (repo_dir, _store_dir) = write_go_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:boltbrowser-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();

        assert!(ingest.files_indexed >= 3, "{:?}", ingest.to_json());
        assert!(ingest.symbols_indexed >= 5, "{:?}", ingest.to_json());

        let symbols = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_LABEL)
                    .with_property("tenant_id", json!("theorem"))
                    .with_limit(100),
            )
            .unwrap();
        assert!(symbols
            .iter()
            .any(|node| node.properties.get("language").and_then(Value::as_str) == Some("go")));
        assert!(symbols
            .iter()
            .any(|node| node.properties.get("name").and_then(Value::as_str) == Some("main")));
        assert!(symbols
            .iter()
            .any(
                |node| node.properties.get("kind").and_then(Value::as_str) == Some("method")
                    && node.properties.get("name").and_then(Value::as_str) == Some("Draw")
            ));

        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "main".to_string(),
                repo_id: ingest.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(search.total_returned > 0, "{:?}", search.to_json());
        assert_eq!(search.hits[0].name, "main");
        assert_eq!(search.hits[0].language, "go");

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn tree_sitter_ingest_extracts_multilingual_symbols_and_edges() {
        let repo_dir = unique_dir("tree-sitter-repo");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::write(
            repo_dir.join("src/native.c"),
            "typedef struct SearchState {\n  int count;\n} SearchState;\n\n\
int c_helper(int value) {\n  return value + 1;\n}\n\n\
int c_entry(int value) {\n  return c_helper(value);\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/engine.cpp"),
            "class SearchEngine {\npublic:\n  int run(int value) { return cpp_helper(value); }\n};\n\n\
int cpp_helper(int value) {\n  return value + 1;\n}\n\n\
int cpp_entry(int value) {\n  return cpp_helper(value);\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/SearchService.java"),
            "interface SearchPort {}\n\n\
class SearchService implements SearchPort {\n  int helperLength(String query) { return query.length(); }\n\n\
  int runJava(String query) { return helperLength(query); }\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/app.ts"),
            "export interface SearchShape { query: string }\n\
export function helperLen(query: string): number {\n    return query.length\n}\n\n\
export const runSearch = (query: string): number => helperLen(query)\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/app.py"),
            "class PythonAdapter:\n    pass\n\n\
def normalize_query(query):\n    return query.strip()\n\n\
def build_context(query):\n    return normalize_query(query)\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/widget.js"),
            "class SearchWidget {\n  render() {\n    return makeLabel()\n  }\n}\n\n\
function makeLabel() {\n  return 'ok'\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/lib.rs"),
            "pub struct SearchKernel {}\n\npub fn rust_entry() -> usize {\n    1\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("src/search_adapter.rb"),
            "module SearchModule\n  class RubyAdapter\n    def helper_length(query)\n      query.length\n    end\n\n    def run_ruby(query)\n      helper_length(query)\n    end\n  end\nend\n",
        )
        .unwrap();

        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:tree-sitter-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ingest.files_indexed, 8);

        let symbols = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_LABEL)
                    .with_property("tenant_id", json!("theorem"))
                    .with_limit(100),
            )
            .unwrap();
        for (name, language, kind) in [
            ("SearchState", "c", "struct"),
            ("c_entry", "c", "function"),
            ("SearchEngine", "cpp", "class"),
            ("cpp_entry", "cpp", "function"),
            ("SearchPort", "java", "interface"),
            ("runJava", "java", "method"),
            ("SearchShape", "typescript", "interface"),
            ("runSearch", "typescript", "function"),
            ("PythonAdapter", "python", "class"),
            ("build_context", "python", "function"),
            ("RubyAdapter", "ruby", "class"),
            ("run_ruby", "ruby", "method"),
            ("SearchWidget", "javascript", "class"),
            ("makeLabel", "javascript", "function"),
            ("SearchKernel", "rust", "struct"),
        ] {
            assert!(
                symbols.iter().any(|node| {
                    node.properties.get("name").and_then(Value::as_str) == Some(name)
                        && node.properties.get("language").and_then(Value::as_str) == Some(language)
                        && node.properties.get("kind").and_then(Value::as_str) == Some(kind)
                        && property_bool(&node.properties, "parser_backed")
                }),
                "missing parser-backed {language} {kind} {name}; symbols={:?}",
                symbols
                    .iter()
                    .map(|node| (
                        node.properties.get("name").and_then(Value::as_str),
                        node.properties.get("language").and_then(Value::as_str),
                        node.properties.get("kind").and_then(Value::as_str),
                    ))
                    .collect::<Vec<_>>()
            );
        }

        for (source, target) in [
            ("c_entry", "c_helper"),
            ("cpp_entry", "cpp_helper"),
            ("runJava", "helperLength"),
            ("run_ruby", "helper_length"),
            ("runSearch", "helperLen"),
        ] {
            let search = search_code_in_store(
                &mut store,
                SearchCodeInput {
                    tenant_id: "theorem".to_string(),
                    query: source.to_string(),
                    repo_id: ingest.repo_id.clone(),
                    limit: 5,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(search.hits[0].name, source);
            let explored = explore_code_in_store(
                &mut store,
                ExploreCodeInput {
                    tenant_id: "theorem".to_string(),
                    node_id: search.hits[0].node_id.clone(),
                    max_depth: 1,
                    limit: 20,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                explored.edges.iter().any(|edge| {
                    edge.edge_type == CALLS_SYMBOL
                        && edge.from_name == source
                        && edge.to_name == target
                        && edge.evidence.contains("tree_sitter_call")
                }),
                "Tree-sitter call edge {source}->{target}: {:?}",
                explored
                    .edges
                    .iter()
                    .map(|edge| (&edge.from_name, &edge.to_name, &edge.evidence))
                    .collect::<Vec<_>>()
            );
        }

        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "runSearch".to_string(),
                repo_id: ingest.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(search.hits[0].name, "runSearch");
        let explored = explore_code_in_store(
            &mut store,
            ExploreCodeInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                max_depth: 1,
                limit: 20,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            explored.edges.iter().any(|edge| {
                edge.edge_type == CALLS_SYMBOL
                    && edge.from_name == "runSearch"
                    && edge.to_name == "helperLen"
                    && edge.evidence.contains("tree_sitter_call")
            }),
            "TypeScript Tree-sitter call edge: {:?}",
            explored
                .edges
                .iter()
                .map(|edge| (&edge.from_name, &edge.to_name, &edge.evidence))
                .collect::<Vec<_>>()
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn ingest_reports_stage_timings_language_stats_and_preread_skips() {
        let repo_dir = unique_dir("instrumented-repo");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::write(
            repo_dir.join("src/lib.rs"),
            "pub struct MemoryCore {}\n\npub fn remember() -> MemoryCore {\n    MemoryCore {}\n}\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("Cargo.lock"),
            "[[package]]\nname = \"skip\"\n",
        )
        .unwrap();
        fs::write(repo_dir.join("pnpm-lock.yaml"), "lockfileVersion: 9\n").unwrap();
        fs::write(
            repo_dir.join("package-lock.json"),
            "{\"lockfileVersion\":3}\n",
        )
        .unwrap();
        fs::write(repo_dir.join("app.min.js"), "function minified(){}\n").unwrap();
        fs::write(repo_dir.join("bundle.js.map"), "{}\n").unwrap();
        fs::write(repo_dir.join("binary.rs"), b"pub fn hidden() {}\0").unwrap();

        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:instrumented".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        let output = ingest.to_json();

        assert_eq!(ingest.files_indexed, 1);
        assert!(ingest.symbols_indexed >= 2, "{output}");
        assert_eq!(output["language_stats"]["rust"]["files_indexed"], json!(1));
        assert!(
            output["language_stats"]["rust"]["symbols_indexed"]
                .as_u64()
                .unwrap_or_default()
                >= 2,
            "{output}"
        );
        assert_eq!(output["skip_stats"]["filename"], json!(5));
        assert_eq!(output["skip_stats"]["binary"], json!(1));
        assert_eq!(ingest.files_skipped, 6);
        assert!(output["stage_timings"]["walk_ms"].is_u64());
        assert!(output["stage_timings"]["parse_ms"].is_u64());
        assert!(output["stage_timings"]["write_ms"].is_u64());
        assert!(output["stage_timings"]["total_ms"].is_u64());

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn reindex_unchanged_repo_carries_files_without_parsing() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let input = IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            repo_id: "repo:reindex-fixture".to_string(),
            ..Default::default()
        };
        let ingest = ingest_codebase_in_store(&mut store, input.clone()).unwrap();
        assert_eq!(ingest.files_parsed, 2);
        assert_eq!(ingest.files_carried, 0);

        let reindex = reindex_codebase_in_store(&mut store, input).unwrap();
        assert_eq!(reindex.files_parsed, 0, "{:?}", reindex.to_json());
        assert_eq!(reindex.files_carried, 2);
        assert_eq!(reindex.files_indexed, 2);
        assert!(reindex.generation > ingest.generation);
        assert!(reindex.symbols_indexed >= ingest.symbols_indexed);

        // Carried symbols are live at the NEW generation: search still hits.
        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                repo_id: reindex.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(search.hits[0].name, "helper_len");

        // Context still reads text through the content-addressed side record.
        let context = code_context_in_store(
            &mut store,
            CodeContextInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(context.context.contains("helper_len"));

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn reindex_after_single_file_edit_parses_exactly_one_file() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let input = IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            repo_id: "repo:edit-fixture".to_string(),
            ..Default::default()
        };
        ingest_codebase_in_store(&mut store, input.clone()).unwrap();

        // Edit ONE file: the python file now references the unchanged rust
        // file's `search_code` symbol by name.
        fs::write(
            repo_dir.join("src/app.py"),
            "class SearchAdapter:\n    pass\n\ndef code_helper():\n    return search_code\n",
        )
        .unwrap();

        let reindex = reindex_codebase_in_store(&mut store, input).unwrap();
        assert_eq!(reindex.files_parsed, 1, "{:?}", reindex.to_json());
        assert_eq!(reindex.files_carried, 1);

        // The fresh symbol links against a carried-forward target: the edge
        // from the edited file resolves into the unchanged file's symbol.
        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "code_helper".to_string(),
                repo_id: reindex.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(search.hits[0].name, "code_helper");
        let explored = explore_code_in_store(
            &mut store,
            ExploreCodeInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                max_depth: 1,
                limit: 20,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            explored.edges.iter().any(|edge| {
                edge.edge_type == CALLS_SYMBOL
                    && edge.from_name == "code_helper"
                    && edge.to_name == "search_code"
            }),
            "cross-file edge into a carried symbol: {:?}",
            explored
                .edges
                .iter()
                .map(|edge| (&edge.from_name, &edge.to_name))
                .collect::<Vec<_>>()
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn ingest_materializes_symbol_name_bucket_index() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let input = IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            repo_id: "repo:name-bucket-fixture".to_string(),
            materialize_symbol_name_index: true,
            ..Default::default()
        };
        let ingest = ingest_codebase_in_store(&mut store, input).unwrap();

        let buckets = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_NAME_LABEL)
                    .with_property("repo_id", json!(ingest.repo_id.clone()))
                    .with_property("name", json!("helper_len"))
                    .with_property("generation", json!(ingest.generation))
                    .with_limit(10),
            )
            .unwrap();
        assert_eq!(buckets.len(), 1, "{buckets:?}");
        let bucket = &buckets[0];
        assert_eq!(bucket.properties["target_count"], json!(1));

        let targets = bucket.properties["targets"]
            .as_array()
            .expect("bucket targets");
        assert_eq!(targets.len(), 1, "{targets:?}");
        let target_ref = &targets[0];
        let target = store
            .get_node(target_ref["symbol_id"].as_str().unwrap())
            .unwrap()
            .expect("bucket target symbol");
        assert_eq!(target_ref["file_path"], json!("src/lib.rs"));
        assert_eq!(target_ref["line"], target.properties["line"]);
        assert_eq!(target.properties["name"], json!("helper_len"));
        assert_eq!(target.properties["repo_id"], json!(ingest.repo_id));
        assert_eq!(target.properties["generation"], json!(ingest.generation));

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn source_file_write_indexes_one_file_without_repo_walk() {
        let mut store = RedCoreGraphStore::memory();
        let output = index_source_file_write_in_store(
            &mut store,
            SourceFileWriteIndexInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:on-write-fixture".to_string(),
                repo_root_display: "embedded://repo/on-write-fixture".to_string(),
                file_path: "src/lib.rs".to_string(),
                content: br#"
pub fn helper_len(value: &str) -> usize { value.len() }
pub fn caller() -> usize { helper_len("abc") }
"#
                .to_vec(),
                actor: "w1-test".to_string(),
                generation: 7,
                materialize_symbol_name_index: true,
            },
        )
        .unwrap();

        assert_eq!(output.file_path, "src/lib.rs");
        assert_eq!(output.generation, 7);
        assert_eq!(output.symbol_names, vec!["helper_len", "caller"]);
        assert_eq!(output.bucket_names, vec!["caller", "helper_len", "len"]);
        let file = store
            .get_node(&output.file_id)
            .unwrap()
            .expect("CodeFile node");
        assert_eq!(file.properties["path"], json!("src/lib.rs"));
        assert_eq!(file.properties["generation"], json!(7));

        let symbols = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_LABEL)
                    .with_property("repo_id", json!("repo:on-write-fixture"))
                    .with_property("file_path", json!("src/lib.rs"))
                    .with_property("generation", json!(7))
                    .with_limit(10),
            )
            .unwrap();
        assert_eq!(symbols.len(), 2, "{symbols:?}");
        assert!(symbols
            .iter()
            .any(|node| node.properties["name"] == json!("helper_len")));

        let helper_bucket = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_NAME_LABEL)
                    .with_property("repo_id", json!("repo:on-write-fixture"))
                    .with_property("name", json!("helper_len"))
                    .with_property("generation", json!(7))
                    .with_limit(10),
            )
            .unwrap();
        assert_eq!(helper_bucket.len(), 1, "{helper_bucket:?}");
        assert_eq!(helper_bucket[0].properties["target_count"], json!(1));
    }

    /// Collect every CALLS/DEPENDS edge currently visible for a repo as
    /// (from_name, edge_type, to_name) at the latest generation, the same way
    /// traversal sees them (stale-generation neighbors filtered out).
    fn collect_visible_edges(
        store: &RedCoreGraphStore,
        repo_id: &str,
    ) -> BTreeSet<(String, String, String)> {
        let latest = latest_repo_generations(store).unwrap();
        let symbols = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_LABEL)
                    .with_property("repo_id", json!(repo_id))
                    .with_limit(100_000),
            )
            .unwrap();
        let mut edges = BTreeSet::new();
        for node in symbols {
            if !node_is_current_generation(&node, &latest) {
                continue;
            }
            let Some(from_name) = property_string(&node.properties, "name") else {
                continue;
            };
            for edge_type in [CALLS_SYMBOL, DEPENDS_ON_SYMBOL] {
                let names =
                    neighbor_symbol_names(store, &node.id, Direction::Out, edge_type, &latest)
                        .unwrap();
                for to_name in names {
                    edges.insert((from_name.clone(), edge_type.to_string(), to_name));
                }
            }
        }
        edges
    }

    fn symbol_embedding(store: &Arc<Mutex<RedCoreGraphStore>>, symbol_id: &str) -> Vec<f32> {
        let guard = store.lock().unwrap();
        let node = guard
            .get_node(symbol_id)
            .unwrap()
            .unwrap_or_else(|| panic!("symbol {symbol_id} present"));
        let vector =
            crate::code_embed_hook::extract_float_vec(&node.properties, EMBEDDING_PROPERTY)
                .unwrap_or_else(|| panic!("embedding present on {symbol_id}"));
        vector
    }

    #[derive(Clone, Debug)]
    struct FixtureSemanticCodeEmbedder;

    impl CodeEmbedder for FixtureSemanticCodeEmbedder {
        fn embed_code(&self, text: &str) -> Result<Vec<f32>, CodeEmbeddingError> {
            let text = text.to_ascii_lowercase();
            if text.contains("parse") || text.contains("decode") || text.contains("json") {
                Ok(vec![1.0, 0.0, 0.0])
            } else if text.contains("render") || text.contains("button") || text.contains("view") {
                Ok(vec![0.0, 1.0, 0.0])
            } else {
                Ok(vec![0.0, 0.0, 1.0])
            }
        }

        fn dimension(&self) -> usize {
            3
        }

        fn name(&self) -> &str {
            "fixture-semantic-code"
        }
    }

    #[test]
    fn configured_code_embedder_controls_symbol_dimension_and_similarity() {
        let query = FixtureSemanticCodeEmbedder
            .embed_code("fn parse_request(json: &str) -> Request")
            .unwrap();
        let similar = FixtureSemanticCodeEmbedder
            .embed_code("fn decode_json(input: &str) -> Value")
            .unwrap();
        let dissimilar = FixtureSemanticCodeEmbedder
            .embed_code("fn render_button(view: &mut Ui)")
            .unwrap();
        assert!(
            cosine_similarity(&query, &similar) > cosine_similarity(&query, &dissimilar),
            "configured encoder must rank semantically similar code closer"
        );

        let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
        let dispatcher = HookDispatcher::start(
            Arc::clone(&store),
            vec![incremental_embed_hook_with_embedder(Arc::new(
                FixtureSemanticCodeEmbedder,
            ))],
            HookDispatcherConfig::default(),
        );
        store
            .lock()
            .unwrap()
            .attach_hook_emitter(dispatcher.emitter());
        let repo_id = "repo:w4-configured-embedding";
        let symbol_id = symbol_node_id(repo_id, "src/lib.rs", "function", "parse_request", 1);

        {
            let mut guard = store.lock().unwrap();
            index_source_file_write_in_store(
                &mut guard,
                SourceFileWriteIndexInput {
                    tenant_id: "theorem".to_string(),
                    repo_id: repo_id.to_string(),
                    repo_root_display: "embedded://repo/w4-configured-embedding".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    content: b"pub fn parse_request(json: &str) -> usize { json.len() }\n".to_vec(),
                    actor: "w4-test".to_string(),
                    generation: 1,
                    materialize_symbol_name_index: true,
                },
            )
            .unwrap();
        }
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "configured embedding hook drained"
        );
        let vector = symbol_embedding(&store, &symbol_id);
        assert_eq!(vector, vec![1.0, 0.0, 0.0]);
        let designations = store.lock().unwrap().vector_designations();
        assert!(
            designations.iter().any(|designation| {
                designation.label == CODE_SYMBOL_LABEL
                    && designation.property == EMBEDDING_PROPERTY
                    && designation.dimension == 3
            }),
            "CodeSymbol/embedding designation must follow configured encoder dimension: {designations:?}"
        );
    }

    #[test]
    fn source_file_write_refreshes_embedding_via_hook() {
        let store = Arc::new(Mutex::new(RedCoreGraphStore::memory()));
        let dispatcher = start_code_kg_dispatcher(Arc::clone(&store));
        let repo_id = "repo:on-write-embedding";
        let symbol_id = symbol_node_id(repo_id, "src/lib.rs", "function", "alpha", 1);

        let write_source = |content: &str, generation: u64| {
            let mut guard = store.lock().unwrap();
            index_source_file_write_in_store(
                &mut guard,
                SourceFileWriteIndexInput {
                    tenant_id: "theorem".to_string(),
                    repo_id: repo_id.to_string(),
                    repo_root_display: "embedded://repo/on-write-embedding".to_string(),
                    file_path: "src/lib.rs".to_string(),
                    content: content.as_bytes().to_vec(),
                    actor: "w1-test".to_string(),
                    generation,
                    materialize_symbol_name_index: true,
                },
            )
            .unwrap()
        };

        write_source("pub fn alpha() -> usize { 1 }\n", 11);
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "embedding hook drained after initial source write"
        );
        let first = symbol_embedding(&store, &symbol_id);
        assert_eq!(first.len(), EMBEDDING_DIM);

        write_source("pub   fn   alpha()    ->    usize   {   1   }\n", 12);
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "embedding hook drained after token-equivalent edit"
        );
        let whitespace_only = symbol_embedding(&store, &symbol_id);
        assert_eq!(
            first, whitespace_only,
            "token-equivalent whitespace edit keeps the embedding idempotent"
        );

        write_source("pub fn alpha() -> usize { 2 }\n", 13);
        assert!(
            dispatcher.quiesce(Duration::from_secs(10)),
            "embedding hook drained after token-changing edit"
        );
        let changed = symbol_embedding(&store, &symbol_id);
        assert_ne!(
            whitespace_only, changed,
            "token-changing source-file edit refreshes the embedding"
        );
    }

    #[test]
    fn source_file_write_rewrites_touched_outgoing_edges() {
        let repo_dir = unique_dir("on-write-edge-rewrite");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(repo_dir.join("helper.py"), "def helper():\n    return 1\n").unwrap();
        fs::write(repo_dir.join("caller.py"), "def caller():\n    return 1\n").unwrap();
        fs::write(
            repo_dir.join("unrelated.py"),
            "def unrelated():\n    return helper()\n",
        )
        .unwrap();
        let input = |repo: &PathBuf| IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            repo_id: "repo:on-write-edges".to_string(),
            materialize_symbol_name_index: true,
            ..Default::default()
        };

        let mut incremental = RedCoreGraphStore::memory();
        let initial = ingest_codebase_in_store(&mut incremental, input(&repo_dir)).unwrap();
        let before = collect_visible_edges(&incremental, "repo:on-write-edges");
        assert!(before.contains(&(
            "unrelated".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));
        assert!(!before.contains(&(
            "caller".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));

        let edited = "def caller():\n    return helper()\n";
        let output = index_source_file_write_in_store(
            &mut incremental,
            SourceFileWriteIndexInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:on-write-edges".to_string(),
                repo_root_display: repo_dir.display().to_string(),
                file_path: "caller.py".to_string(),
                content: edited.as_bytes().to_vec(),
                actor: "w1-test".to_string(),
                generation: initial.generation + 1,
                materialize_symbol_name_index: true,
            },
        )
        .unwrap();
        assert_eq!(output.files_carried, 2);
        assert_eq!(output.edges_indexed, 1);
        assert!(output.bucket_lookups < 10, "{output:?}");

        let incremental_edges = collect_visible_edges(&incremental, "repo:on-write-edges");
        assert!(incremental_edges.contains(&(
            "caller".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));
        assert!(incremental_edges.contains(&(
            "unrelated".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));

        fs::write(repo_dir.join("caller.py"), edited).unwrap();
        let mut full = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut full, input(&repo_dir)).unwrap();
        assert_eq!(
            incremental_edges,
            collect_visible_edges(&full, "repo:on-write-edges"),
            "single-file write edge set matches a full ingest after adding a call"
        );

        let removed = "def caller():\n    return 2\n";
        let removed_output = index_source_file_write_in_store(
            &mut incremental,
            SourceFileWriteIndexInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:on-write-edges".to_string(),
                repo_root_display: repo_dir.display().to_string(),
                file_path: "caller.py".to_string(),
                content: removed.as_bytes().to_vec(),
                actor: "w1-test".to_string(),
                generation: output.generation + 1,
                materialize_symbol_name_index: true,
            },
        )
        .unwrap();
        assert!(
            removed_output.edges_retired >= 1,
            "removed call tombstones the prior outgoing edge: {removed_output:?}"
        );
        let removed_edges = collect_visible_edges(&incremental, "repo:on-write-edges");
        assert!(!removed_edges.contains(&(
            "caller".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));
        assert!(removed_edges.contains(&(
            "unrelated".to_string(),
            CALLS_SYMBOL.to_string(),
            "helper".to_string()
        )));

        fs::write(repo_dir.join("caller.py"), removed).unwrap();
        let mut full_removed = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut full_removed, input(&repo_dir)).unwrap();
        assert_eq!(
            removed_edges,
            collect_visible_edges(&full_removed, "repo:on-write-edges"),
            "single-file write edge set matches a full ingest after removing a call"
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn source_file_write_recomputes_incoming_edges_for_new_definition() {
        let repo_dir = unique_dir("on-write-incoming-edge");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(
            repo_dir.join("caller.py"),
            "def caller():\n    return mystery()\n",
        )
        .unwrap();
        let input = |repo: &PathBuf| IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            repo_id: "repo:on-write-incoming".to_string(),
            materialize_symbol_name_index: true,
            ..Default::default()
        };

        let mut incremental = RedCoreGraphStore::memory();
        let initial = ingest_codebase_in_store(&mut incremental, input(&repo_dir)).unwrap();
        assert!(
            !collect_visible_edges(&incremental, "repo:on-write-incoming").contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "mystery".to_string()
            )),
            "no edge before mystery is defined"
        );

        let mystery = "def mystery():\n    return 7\n";
        let output = index_source_file_write_in_store(
            &mut incremental,
            SourceFileWriteIndexInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:on-write-incoming".to_string(),
                repo_root_display: repo_dir.display().to_string(),
                file_path: "mystery.py".to_string(),
                content: mystery.as_bytes().to_vec(),
                actor: "w1-test".to_string(),
                generation: initial.generation + 1,
                materialize_symbol_name_index: true,
            },
        )
        .unwrap();
        assert_eq!(output.files_carried, 1);
        assert_eq!(output.edges_indexed, 1);
        assert!(output.bucket_lookups < 10, "{output:?}");

        let incremental_edges = collect_visible_edges(&incremental, "repo:on-write-incoming");
        assert!(
            incremental_edges.contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "mystery".to_string()
            )),
            "carried caller links to newly defined mystery: {incremental_edges:?}"
        );

        fs::write(repo_dir.join("mystery.py"), mystery).unwrap();
        let mut full = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut full, input(&repo_dir)).unwrap();
        assert_eq!(
            incremental_edges,
            collect_visible_edges(&full, "repo:on-write-incoming"),
            "single-file write incoming edge set matches a full ingest"
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn source_file_write_touches_only_changed_name_buckets_in_large_repo() {
        let repo_dir = unique_dir("on-write-large-bucket-bound");
        fs::create_dir_all(&repo_dir).unwrap();
        let symbol_count = 250usize;
        for index in 0..symbol_count {
            fs::write(
                repo_dir.join(format!("helper_{index}.py")),
                format!("def helper_{index}():\n    return {index}\n"),
            )
            .unwrap();
        }
        fs::write(repo_dir.join("caller.py"), "def caller():\n    return 1\n").unwrap();
        let input = |repo: &PathBuf| IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            repo_id: "repo:on-write-large-bucket-bound".to_string(),
            materialize_symbol_name_index: true,
            max_files: (symbol_count as u64) + 10,
            ..Default::default()
        };

        let mut store = RedCoreGraphStore::memory();
        let initial = ingest_codebase_in_store(&mut store, input(&repo_dir)).unwrap();
        assert!(
            initial.symbols_indexed > 200,
            "fixture should be multi-hundred symbols: {initial:?}"
        );

        let edited = "def caller():\n    return helper_42()\n";
        let output = index_source_file_write_in_store(
            &mut store,
            SourceFileWriteIndexInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:on-write-large-bucket-bound".to_string(),
                repo_root_display: repo_dir.display().to_string(),
                file_path: "caller.py".to_string(),
                content: edited.as_bytes().to_vec(),
                actor: "w1-test".to_string(),
                generation: initial.generation + 1,
                materialize_symbol_name_index: true,
            },
        )
        .unwrap();

        assert_eq!(output.symbols_indexed, 1);
        assert!(
            output.bucket_lookups <= 4,
            "bucket work should be bounded by names in changed file, not repo size: {output:?}"
        );
        assert!(
            output.bucket_lookups < initial.symbols_indexed / 50,
            "bucket work must stay far below all repo symbols: {output:?}"
        );
        let edges = collect_visible_edges(&store, "repo:on-write-large-bucket-bound");
        assert!(
            edges.contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "helper_42".to_string()
            )),
            "changed caller links to one target in the large repo: {edges:?}"
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn reindex_edges_match_a_full_ingest_after_a_moved_target() {
        // v1: caller.py calls helper; helper.py defines helper at line 1.
        let repo_dir = unique_dir("moved-target-repo");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(
            repo_dir.join("caller.py"),
            "def caller():\n    return helper()\n",
        )
        .unwrap();
        fs::write(repo_dir.join("helper.py"), "def helper():\n    return 1\n").unwrap();

        let input = |repo: &PathBuf| IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            repo_id: "repo:moved".to_string(),
            ..Default::default()
        };

        let mut incremental = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut incremental, input(&repo_dir)).unwrap();

        // Edit ONLY helper.py: a leading comment shifts `helper` to line 2, so
        // its node id changes. caller.py is unchanged (carried).
        fs::write(
            repo_dir.join("helper.py"),
            "# shifted down\ndef helper():\n    return 1\n",
        )
        .unwrap();
        let reindexed = reindex_codebase_in_store(&mut incremental, input(&repo_dir)).unwrap();
        assert_eq!(reindexed.files_parsed, 1, "only helper.py re-extracted");
        assert_eq!(reindexed.files_carried, 1, "caller.py carried");

        // A full ingest of the SAME final tree into a clean store.
        let mut full = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut full, input(&repo_dir)).unwrap();

        // The carried caller's edge follows the moved target: caller -> helper
        // is present and points at the new (line-2) symbol, and the visible
        // edge set is identical to a full ingest (no stale caller->old_helper).
        let incremental_edges = collect_visible_edges(&incremental, "repo:moved");
        let full_edges = collect_visible_edges(&full, "repo:moved");
        assert!(
            incremental_edges.contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "helper".to_string()
            )),
            "carried caller links to the moved helper: {incremental_edges:?}"
        );
        assert_eq!(
            incremental_edges, full_edges,
            "incremental reindex edges == full ingest edges"
        );

        // Explore from caller shows exactly one helper neighbor (not a stale one).
        let caller = search_code_in_store(
            &mut incremental,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "caller".to_string(),
                repo_id: "repo:moved".to_string(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        let explored = explore_code_in_store(
            &mut incremental,
            ExploreCodeInput {
                tenant_id: "theorem".to_string(),
                node_id: caller.hits[0].node_id.clone(),
                max_depth: 1,
                limit: 20,
                ..Default::default()
            },
        )
        .unwrap();
        let helper_neighbors = explored
            .related_symbols
            .iter()
            .filter(|symbol| symbol.name == "helper")
            .count();
        assert_eq!(helper_neighbors, 1, "no stale duplicate helper neighbor");
        assert_eq!(explored.related_symbols[0].line, 2, "the line-2 helper");

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn reindex_links_carried_symbol_to_a_newly_defined_name() {
        // v1: caller.py references `mystery`, which is defined nowhere yet, so
        // no edge exists. This is the case that needs the carried body, not
        // just persisted matched names.
        let repo_dir = unique_dir("new-name-repo");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(
            repo_dir.join("caller.py"),
            "def caller():\n    return mystery()\n",
        )
        .unwrap();
        let input = |repo: &PathBuf| IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            repo_id: "repo:newname".to_string(),
            ..Default::default()
        };
        let mut store = RedCoreGraphStore::memory();
        ingest_codebase_in_store(&mut store, input(&repo_dir)).unwrap();
        assert!(
            !collect_visible_edges(&store, "repo:newname").contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "mystery".to_string()
            )),
            "no edge before mystery is defined"
        );

        // v2: add mystery.py defining `mystery`. caller.py is unchanged (carried).
        fs::write(
            repo_dir.join("mystery.py"),
            "def mystery():\n    return 7\n",
        )
        .unwrap();
        let reindexed = reindex_codebase_in_store(&mut store, input(&repo_dir)).unwrap();
        assert_eq!(reindexed.files_parsed, 1, "only mystery.py extracted");
        assert_eq!(reindexed.files_carried, 1, "caller.py carried");

        // The carried caller now links to the newly defined mystery, because
        // its body was reconstructed from the D3 text side record and retokenized.
        assert!(
            collect_visible_edges(&store, "repo:newname").contains(&(
                "caller".to_string(),
                CALLS_SYMBOL.to_string(),
                "mystery".to_string()
            )),
            "carried caller links to the newly defined mystery"
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn reindex_retires_removed_files_from_the_latest_generation() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let input = IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            repo_id: "repo:removal-fixture".to_string(),
            ..Default::default()
        };
        ingest_codebase_in_store(&mut store, input.clone()).unwrap();
        let before = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "SearchAdapter".to_string(),
                repo_id: input.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(before.total_returned > 0);

        fs::remove_file(repo_dir.join("src/app.py")).unwrap();
        let reindex = reindex_codebase_in_store(&mut store, input.clone()).unwrap();
        assert_eq!(reindex.files_parsed, 0);
        assert_eq!(reindex.files_carried, 1);

        // The removed file's symbols stay at the old generation, which the
        // latest-generation filter retires from search.
        let after = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "SearchAdapter".to_string(),
                repo_id: input.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(after.total_returned, 0, "{:?}", after.to_json());
        let survivor = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                repo_id: input.repo_id,
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(survivor.hits[0].name, "helper_len");

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn walk_respects_gitignore_without_a_git_checkout() {
        let repo_dir = unique_dir("gitignore-repo");
        fs::create_dir_all(repo_dir.join("src")).unwrap();
        fs::create_dir_all(repo_dir.join("generated")).unwrap();
        fs::write(repo_dir.join(".gitignore"), "generated/\nsecret.py\n").unwrap();
        fs::write(repo_dir.join("src/lib.rs"), "pub fn keep_me() {}\n").unwrap();
        fs::write(
            repo_dir.join("generated/output.py"),
            "def drop_me():\n    pass\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("secret.py"),
            "def also_dropped():\n    pass\n",
        )
        .unwrap();

        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:gitignore-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ingest.files_indexed, 1, "{:?}", ingest.to_json());

        let files = store
            .query_nodes(NodeQuery::label(CODE_FILE_LABEL).with_limit(100))
            .unwrap();
        assert!(files.iter().all(|node| {
            let path = node.properties["path"].as_str().unwrap_or_default();
            !path.starts_with("generated/") && path != "secret.py"
        }));

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn edge_inference_caps_name_fanout() {
        let repo_dir = unique_dir("fanout-repo");
        fs::create_dir_all(&repo_dir).unwrap();
        // 26 definitions of one name: beyond EDGE_NAME_BUCKET_CAP, the name is
        // skipped outright. 10 definitions of another: admitted, but capped at
        // EDGE_TARGETS_PER_NAME_CAP targets.
        for index in 0..26 {
            fs::write(
                repo_dir.join(format!("shared_{index:02}.py")),
                "def shared_helper():\n    pass\n",
            )
            .unwrap();
        }
        for index in 0..10 {
            fs::write(
                repo_dir.join(format!("ten_{index:02}.py")),
                "def ten_helper():\n    pass\n",
            )
            .unwrap();
        }
        fs::write(
            repo_dir.join("caller.py"),
            "def caller():\n    shared_helper()\n    ten_helper()\n",
        )
        .unwrap();

        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:fanout-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(ingest.files_indexed, 37);

        let caller = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "caller".to_string(),
                repo_id: ingest.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        let caller_id = caller.hits[0].node_id.clone();
        let callees = store
            .neighbors(NeighborQuery {
                node_id: caller_id,
                direction: Direction::Out,
                edge_type: Some(CALLS_SYMBOL.to_string()),
                include_expired: false,
            })
            .unwrap();
        let callee_names: Vec<String> = callees
            .iter()
            .filter_map(|hit| store.get_node(&hit.node_id).ok().flatten())
            .filter_map(|node| node.properties["name"].as_str().map(str::to_string))
            .collect();
        assert_eq!(
            callees.len(),
            EDGE_TARGETS_PER_NAME_CAP,
            "ten_helper capped at {EDGE_TARGETS_PER_NAME_CAP}: {callee_names:?}"
        );
        assert!(callee_names.iter().all(|name| name == "ten_helper"));

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn file_text_lives_in_side_records_not_on_file_nodes() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();
        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:text-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();

        let files = store
            .query_nodes(NodeQuery::label(CODE_FILE_LABEL).with_limit(100))
            .unwrap();
        assert_eq!(files.len() as u64, ingest.files_indexed);
        assert!(
            files
                .iter()
                .all(|node| node.properties.get("text").is_none()),
            "CodeFile nodes carry no inline text"
        );
        let texts = store
            .query_nodes(NodeQuery::label(CODE_FILE_TEXT_LABEL).with_limit(100))
            .unwrap();
        assert_eq!(texts.len() as u64, ingest.files_indexed);
        assert!(texts
            .iter()
            .all(|node| node.properties["text"].as_str().is_some()));

        // Context reads the side record and returns the correct window.
        let context = code_context_in_store(
            &mut store,
            CodeContextInput {
                tenant_id: "theorem".to_string(),
                repo_id: ingest.repo_id.clone(),
                file_path: "src/lib.rs".to_string(),
                after_lines: 50,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(context.context.contains("helper_len(query)"));

        // Legacy fallback: a pre-side-record CodeFile node with inline text
        // (and no CodeFileText record) still serves context.
        let legacy_repo = "repo:legacy-text";
        store
            .commit_batch(GraphMutationBatch::new([
                GraphMutation::NodeUpsert(NodeRecord::new(
                    legacy_repo,
                    [CODE_REPO_LABEL],
                    json!({
                        "tenant_id": "theorem",
                        "repo_id": legacy_repo,
                        "latest_generation": 7,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "code:file:legacy-1",
                    [CODE_FILE_LABEL],
                    json!({
                        "tenant_id": "theorem",
                        "repo_id": legacy_repo,
                        "file_id": "code:file:legacy-1",
                        "path": "legacy.py",
                        "content_hash": "legacy-hash-without-side-record",
                        "text": "def legacy_symbol():\n    return 1\n",
                        "generation": 7,
                        "source": SOURCE,
                    }),
                )),
            ]))
            .unwrap();
        let legacy = code_context_in_store(
            &mut store,
            CodeContextInput {
                tenant_id: "theorem".to_string(),
                repo_id: legacy_repo.to_string(),
                file_path: "legacy.py".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(legacy.context.contains("legacy_symbol"));

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn submitted_ingest_job_streams_events_and_search_runs_during_parse() {
        use crate::ingest_jobs::TestIngestPause;

        let (repo_dir, store_dir) = write_fixture_repo();
        let runtime = CodeIndexRuntime::try_new_at(&store_dir, test_options()).unwrap();
        let pause = std::sync::Arc::new(TestIngestPause::default());
        let submitted = runtime.submit_ingest_job(IngestJobRequest {
            input: IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:job-fixture".to_string(),
                ..Default::default()
            },
            operation: "ingest".to_string(),
            test_pause: Some(std::sync::Arc::clone(&pause)),
            ..Default::default()
        });
        assert_eq!(submitted.state, IngestJobState::Queued);
        assert!(!submitted.job_id.is_empty());

        // The worker is paused AFTER walk/parse and BEFORE commit: the store
        // lock is free, so a concurrent search returns instead of blocking.
        pause.wait_until_arrived();
        let concurrent = runtime
            .search_code(SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(concurrent.total_returned, 0, "nothing committed yet");
        let mid = runtime.ingest_job_status(&submitted.job_id).unwrap();
        assert!(!mid.state.is_terminal());
        pause.release();

        // Drain the stream: events arrive in order and end with `finished`.
        let mut labels = Vec::new();
        let mut after = 0u64;
        loop {
            let (events, terminal) = runtime
                .wait_ingest_job_events(
                    &submitted.job_id,
                    after,
                    std::time::Duration::from_secs(10),
                )
                .expect("job is known");
            for event in events {
                after = event.sequence;
                labels.push(event.kind.label());
            }
            if terminal {
                break;
            }
        }
        let position = |label: &str| labels.iter().position(|item| *item == label);
        assert!(
            position("walk_done") < position("parse_progress"),
            "{labels:?}"
        );
        assert!(
            position("parse_progress") < position("commit_done"),
            "{labels:?}"
        );
        assert_eq!(labels.last(), Some(&"finished"), "{labels:?}");

        let done = runtime.ingest_job_status(&submitted.job_id).unwrap();
        assert_eq!(done.state, IngestJobState::Finished);
        let output = done.output.expect("finished output");
        assert_eq!(output.files_indexed, 2);

        let search = runtime
            .search_code(SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                repo_id: output.repo_id.clone(),
                limit: 5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(search.hits[0].name, "helper_len");

        drop(runtime);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }

    /// Open a runtime over `store_dir`, retrying the in-process RedCore
    /// directory lock briefly. A real restart is a fresh process so the lock
    /// never conflicts; simulating restart in one process can momentarily race
    /// the prior runtime's worker releasing its transient store handle.
    fn open_runtime_with_retry(store_dir: &Path) -> CodeIndexRuntime {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match CodeIndexRuntime::try_new_at(store_dir, test_options()) {
                Ok(runtime) => return runtime,
                Err(error) if error.code == "redcore_lock_unavailable" => {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "store dir never released: {error}"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => panic!("unexpected open error: {error}"),
            }
        }
    }

    #[test]
    fn finished_job_survives_a_runtime_restart() {
        let (repo_dir, store_dir) = write_fixture_repo();
        // First runtime: submit a job and let it finish.
        let first = CodeIndexRuntime::try_new_at(&store_dir, test_options()).unwrap();
        let submitted = first.submit_ingest_job(IngestJobRequest {
            input: IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:durable-finished".to_string(),
                ..Default::default()
            },
            operation: "ingest".to_string(),
            ..Default::default()
        });
        let job_id = submitted.job_id.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let status = first.ingest_job_status(&job_id).unwrap();
            if status.state.is_terminal() {
                assert_eq!(status.state, IngestJobState::Finished);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "job did not finish");
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        drop(first);

        // Second runtime over the SAME store dir: the finished job is recovered,
        // queryable, and its event log replays through the terminal event.
        let second = open_runtime_with_retry(&store_dir);
        let recovered = second
            .ingest_job_status(&job_id)
            .expect("finished job recovered after restart");
        assert_eq!(recovered.state, IngestJobState::Finished);
        let output = recovered.output.expect("recovered finished output");
        assert_eq!(output.files_indexed, 2);
        assert_eq!(output.repo_id, "repo:durable-finished");
        let (events, terminal) = second.ingest_jobs().events_after(&job_id, 0).unwrap();
        assert!(terminal);
        assert_eq!(
            events.last().map(|event| event.kind.label()),
            Some("finished")
        );

        drop(second);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }

    #[test]
    fn authed_job_request_persists_installation_id_but_no_token() {
        let (repo_dir, store_dir) = write_fixture_repo();
        let runtime = CodeIndexRuntime::try_new_at(&store_dir, test_options()).unwrap();
        let submitted = runtime.submit_ingest_job(IngestJobRequest {
            input: IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_id: "repo:private".to_string(),
                ..Default::default()
            },
            operation: "reindex".to_string(),
            repo_url: "https://github.com/example/private.git".to_string(),
            installation_id: Some(42),
            caps: RepoFetchCaps::default(),
            parse_budget_ms: None,
            ..Default::default()
        });

        let store = runtime.lock_store().unwrap();
        let node = GraphStore::get_node(&*store, &submitted.job_id).expect("job mirror node");
        let request = node.properties.get("request").expect("request json");
        assert_eq!(
            request.get("installation_id").and_then(Value::as_u64),
            Some(42)
        );
        let serialized = request.to_string();
        assert!(!serialized.contains("token"));
        assert!(!serialized.contains("Authorization"));

        drop(store);
        drop(runtime);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }

    #[test]
    fn interrupted_job_reruns_after_a_restart() {
        use crate::ingest_jobs::CODE_INGEST_JOB_LABEL;

        let (repo_dir, store_dir) = write_fixture_repo();

        // Simulate a crash mid-flight: write a durable CodeIngestJob node in the
        // `running` state directly (the request persisted, no terminal event),
        // exactly the shape a process that died during parse leaves behind.
        {
            let mut store = RedCoreGraphStore::open(&store_dir, test_options()).unwrap();
            let request = json!({
                "input": {
                    "tenant_id": "theorem",
                    "repo_path": repo_dir.display().to_string(),
                    "repo_id": "repo:durable-interrupted",
                    "include_extensions": [],
                    "exclude_dirs": [],
                    "max_files": 0,
                    "max_file_bytes": 0,
                    "max_total_bytes": 0,
                    "actor": "",
                },
                "operation": "ingest",
                "repo_url": "",
                "installation_id": null,
                "caps": { "max_total_bytes": 0, "clone_timeout_ms": 20000 },
                "parse_budget_ms": null,
            });
            let node = NodeRecord::new(
                "code:ingest-job:interrupted-fixture",
                [CODE_INGEST_JOB_LABEL],
                json!({
                    "tenant_id": "theorem",
                    "repo_id": "repo:durable-interrupted",
                    "operation": "ingest",
                    "state": "running",
                    "stage": "parse",
                    "files_total": 2,
                    "files_done": 0,
                    "submitted_at_ms": 1,
                    "updated_at_ms": 1,
                    "error_code": "",
                    "error_message": "",
                    "request": request,
                    "output": Value::Null,
                    "milestone_events": [],
                    "source": SOURCE,
                }),
            );
            store
                .commit_batch(GraphMutationBatch::new([GraphMutation::NodeUpsert(node)]))
                .unwrap();
        }

        // Restart: recovery re-enqueues the interrupted job and runs it to
        // completion against the real repo.
        let runtime = open_runtime_with_retry(&store_dir);
        let job_id = "code:ingest-job:interrupted-fixture";
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            let status = runtime
                .ingest_job_status(job_id)
                .expect("interrupted job recovered");
            if status.state.is_terminal() {
                assert_eq!(status.state, IngestJobState::Finished, "{:?}", status.stage);
                let output = status.output.expect("re-run produced output");
                assert_eq!(output.files_indexed, 2);
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "recovered job did not re-run: stage {}",
                status.stage
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        // The re-run actually wrote the code graph: search finds a symbol.
        let search = runtime
            .search_code(SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                repo_id: "repo:durable-interrupted".to_string(),
                limit: 5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(search.hits[0].name, "helper_len");

        drop(runtime);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }

    #[test]
    fn ingest_commits_partial_progress_when_parse_budget_exceeded() {
        let repo_dir = unique_dir("budget-repo");
        fs::create_dir_all(&repo_dir).unwrap();
        // One big file that sorts FIRST guarantees chunk 0 takes measurable
        // time, so the 1ms budget trips deterministically before chunk 1.
        let mut big = String::new();
        for index in 0..5_000 {
            big.push_str(&format!("def big_symbol_{index}():\n    pass\n"));
        }
        fs::write(repo_dir.join("a_big.py"), big).unwrap();
        for index in 0..220 {
            fs::write(
                repo_dir.join(format!("tiny_{index:03}.py")),
                format!("def tiny_{index:03}():\n    pass\n"),
            )
            .unwrap();
        }

        let started = Instant::now();
        let config = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo_dir.display().to_string(),
            repo_id: "repo:budget-fixture".to_string(),
            ..Default::default()
        })
        .unwrap();
        let prepared = prepare_codebase_ingest_resolved(
            config,
            0,
            0,
            started,
            IngestPipelineOptions {
                prior: None,
                sink: None,
                carried_text_loader: None,
                parse_budget_ms: 1,
            },
        )
        .unwrap();
        let mut store = RedCoreGraphStore::memory();
        let output = commit_prepared_ingest(&mut store, prepared, "ingest").unwrap();

        assert_eq!(output.status, "budget_exceeded", "{:?}", output.to_json());
        assert_eq!(output.files_parsed, PARSE_CHUNK_FILES as u64);
        assert!(output.message.contains("221"), "{}", output.message);

        // Partial progress is COMMITTED: a chunk-0 symbol is searchable.
        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "big_symbol_0".to_string(),
                repo_id: output.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert!(search.total_returned > 0);

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn inverted_index_edge_inference_handles_20k_symbols() {
        let repo_dir = unique_dir("scale-repo");
        fs::create_dir_all(&repo_dir).unwrap();
        // 200 files x 100 symbols = 20k symbols. Each symbol references one
        // cross-file name (bucket size 1, inside every cap), so edges are
        // emitted at scale through the inverted index.
        for file_index in 0..200 {
            let mut body = String::new();
            let next_file = (file_index + 1) % 200;
            for symbol_index in 0..100 {
                body.push_str(&format!(
                    "def fn_{file_index}_{symbol_index}():\n    return fn_{next_file}_{symbol_index}\n"
                ));
            }
            fs::write(repo_dir.join(format!("module_{file_index:03}.py")), body).unwrap();
        }

        let mut store = RedCoreGraphStore::memory();
        let output = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                repo_id: "repo:scale-fixture".to_string(),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(output.symbols_indexed, 20_000);
        println!(
            "20k-symbol fixture stage timings: {}",
            output.stage_timings.to_json()
        );
        // The old quadratic branch was symbols x distinct_names x body scans
        // (minutes at this size). The inverted index keeps mutation build in
        // interactive range even in debug builds.
        assert!(
            output.stage_timings.mutation_ms < 30_000,
            "mutation_ms regression: {}",
            output.stage_timings.mutation_ms
        );

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn plugin_manifest_describes_code_graph_capability() {
        let manifest = CodeParsingPlugin.manifest();
        assert_eq!(manifest.name, "rustyred.thg.code");
        assert!(manifest.labels.contains(&CODE_SYMBOL_LABEL));
        assert!(manifest.labels.contains(&CODE_SYMBOL_NAME_LABEL));
        assert!(manifest.edge_types.contains(&CALLS_SYMBOL));
        assert!(manifest.operations.iter().any(|operation| operation.command
            == "RUSTYRED_THG.CODE.INGEST"
            && operation.aliases.contains(&"code.ingest")
            && operation.aliases.contains(&"RUSTYRED.CODE.INGEST")
            && operation.writes_graph));
        assert!(manifest
            .operations
            .iter()
            .any(|operation| operation.operation == "search" && !operation.writes_graph));
        assert!(manifest
            .operations
            .iter()
            .any(|operation| operation.operation == "context_pack" && operation.writes_graph));
        assert!(manifest
            .capabilities
            .iter()
            .any(|capability| capability.name == "code.encoder.tree_sitter_tags"));
        assert!(CodeParsingPlugin.manifest_json()["operations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|operation| operation["command"] == "RUSTYRED_THG.CODE.SEARCH"));
    }

    #[test]
    fn builtin_registry_executes_code_plugin_aliases() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let registry = builtin_code_plugin_registry();
        assert!(registry
            .plugins()
            .iter()
            .any(|plugin| plugin.name() == "rustyred.thg.code"));
        assert!(registry.operation("code.ingest").is_some());
        assert!(registry.operation("RUSTYRED_THG.CODE.SEARCH").is_some());
        assert!(registry.operation("RUSTYRED.CODE.SEARCH").is_some());

        let mut store = RedCoreGraphStore::memory();
        let ingest = execute_code_plugin_operation(
            &mut store,
            "theorem",
            "rustyred.thg.code.ingest",
            json!({
                "repo_path": repo_dir.display().to_string(),
                "actor": "codex-test"
            }),
        )
        .unwrap();
        assert_eq!(ingest.operation, "ingest");
        assert!(ingest.writes_graph);
        assert_eq!(ingest.result["files_indexed"], json!(2));
        let repo_id = ingest.result["repo_id"].as_str().unwrap().to_string();

        let search = execute_code_plugin_operation(
            &mut store,
            "theorem",
            "RUSTYRED_THG.CODE.SEARCH",
            json!({
                "query": "helper_len",
                "repo_id": repo_id,
                "limit": 5
            }),
        )
        .unwrap();
        assert_eq!(search.operation, "search");
        assert_eq!(search.command, "RUSTYRED_THG.CODE.SEARCH");
        assert_eq!(search.result["hits"][0]["name"], "helper_len");

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn in_store_ingest_writes_code_graph_to_caller_store() {
        let (repo_dir, _store_dir) = write_fixture_repo();
        let mut store = RedCoreGraphStore::memory();

        let ingest = ingest_codebase_in_store(
            &mut store,
            IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(ingest.files_indexed, 2);
        let symbols = store
            .query_nodes(
                NodeQuery::label(CODE_SYMBOL_LABEL)
                    .with_property("tenant_id", json!("theorem"))
                    .with_limit(100),
            )
            .unwrap();
        assert!(symbols
            .iter()
            .any(|node| node.properties["name"] == "search_code"));

        let search = search_code_in_store(
            &mut store,
            SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "helper_len".to_string(),
                repo_id: ingest.repo_id.clone(),
                limit: 5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(search.hits[0].name, "helper_len");

        fs::remove_dir_all(repo_dir).ok();
    }

    #[test]
    fn ingest_search_and_context_round_trip() {
        let (repo_dir, store_dir) = write_fixture_repo();
        let runtime = CodeIndexRuntime::try_new_at(&store_dir, test_options()).unwrap();
        let ingest = runtime
            .ingest_codebase(IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(ingest.files_indexed, 2);
        assert!(ingest.symbols_indexed >= 3);
        assert!(!ingest.receipt_hash.is_empty());

        let search = runtime
            .search_code(SearchCodeInput {
                tenant_id: "theorem".to_string(),
                query: "search_code".to_string(),
                repo_id: ingest.repo_id.clone(),
                limit: 5,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(search.total_returned, 1);
        assert_eq!(search.hits[0].name, "search_code");
        assert_eq!(search.hits[0].trust_tier, DEFAULT_TRUST_TIER);
        assert!(!search.hits[0].community_id.is_empty());

        let recognized = runtime
            .recognize_code(RecognizeCodeInput {
                tenant_id: "theorem".to_string(),
                repo_id: ingest.repo_id.clone(),
                text: "pub fn inline_symbol() -> usize {\n    1\n}\n".to_string(),
                limit: 5,
                ..Default::default()
            })
            .unwrap();
        assert!(recognized
            .symbols
            .iter()
            .any(|symbol| symbol.name == "inline_symbol"));

        let explored = runtime
            .explore_code(ExploreCodeInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                max_depth: 1,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(explored.focus.as_ref().unwrap().name, "search_code");
        assert!(explored
            .related_symbols
            .iter()
            .any(|symbol| symbol.name == "helper_len"));
        assert!(explored.edges.iter().any(|edge| {
            edge.edge_type == CALLS_SYMBOL
                && edge.from_name == "search_code"
                && edge.to_name == "helper_len"
        }));

        let dependency_graph = runtime
            .explore_code(ExploreCodeInput {
                tenant_id: "theorem".to_string(),
                query: "QueryPlan".to_string(),
                repo_id: ingest.repo_id.clone(),
                max_depth: 1,
                limit: 10,
                ..Default::default()
            })
            .unwrap();
        assert!(dependency_graph.edges.iter().any(|edge| {
            edge.edge_type == DEPENDS_ON_SYMBOL
                && edge.from_name == "QueryPlan"
                && edge.to_name == "SearchKernel"
                && edge.evidence.contains("rust_ast_dependency")
        }));

        let explained = runtime
            .explain_code(ExplainCodeInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                max_chars: 1_000,
                ..Default::default()
            })
            .unwrap();
        assert!(explained.summary.contains("Trust tier: advisory"));
        assert!(explained.context.contains("helper_len(query)"));
        assert!(explained
            .edges
            .iter()
            .any(|edge| edge.to_name == "helper_len"));

        let use_receipt = runtime
            .record_use_receipt(RecordUseReceiptInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                repo_id: ingest.repo_id.clone(),
                query: "search_code".to_string(),
                action: "explain".to_string(),
                outcome: "useful".to_string(),
                actor: "codex-test".to_string(),
                use_json: r#"{"selected":true}"#.to_string(),
            })
            .unwrap();
        assert_eq!(use_receipt.status, "ok");
        assert!(!use_receipt.receipt_hash.is_empty());

        let context = runtime
            .code_context(CodeContextInput {
                tenant_id: "theorem".to_string(),
                node_id: search.hits[0].node_id.clone(),
                before_lines: 1,
                after_lines: 2,
                ..Default::default()
            })
            .unwrap();
        assert!(context.context.contains("search_code"));
        assert!(context
            .symbols
            .iter()
            .any(|symbol| symbol.name == "search_code"));

        drop(runtime);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }

    #[test]
    fn codebase_map_projection_emits_ranked_entry_kinds() {
        let store_dir = unique_dir("map-projection-store");
        let mut store = RedCoreGraphStore::open(&store_dir, test_options()).unwrap();
        let tenant = "Travis-Gilbert";
        let repo_id = "repo:projection-demo";
        store
            .commit_batch(GraphMutationBatch::new([
                GraphMutation::NodeUpsert(NodeRecord::new(
                    repo_id,
                    [CODE_REPO_LABEL],
                    json!({
                        "tenant_id": tenant,
                        "repo_id": repo_id,
                        "latest_generation": 1,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "code:file:projection",
                    [CODE_FILE_LABEL],
                    json!({
                        "tenant_id": tenant,
                        "repo_id": repo_id,
                        "path": "src/lib.rs",
                        "generation": 1,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "code:symbol:main",
                    [CODE_SYMBOL_LABEL],
                    json!({
                        "tenant_id": tenant,
                        "repo_id": repo_id,
                        "name": "main",
                        "kind": "function",
                        "file_path": "src/lib.rs",
                        "line": 3,
                        "generation": 1,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "code:symbol:adapter",
                    [CODE_SYMBOL_LABEL],
                    json!({
                        "tenant_id": tenant,
                        "repo_id": repo_id,
                        "name": "Adapter",
                        "kind": "struct",
                        "file_path": "src/lib.rs",
                        "line": 9,
                        "generation": 1,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "code:symbol:config",
                    [CODE_SYMBOL_LABEL],
                    json!({
                        "tenant_id": tenant,
                        "repo_id": repo_id,
                        "name": "Config",
                        "kind": "struct",
                        "file_path": "src/lib.rs",
                        "line": 16,
                        "generation": 1,
                        "source": SOURCE,
                    }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:repo:file",
                    repo_id,
                    CONTAINS_FILE,
                    "code:file:projection",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:file:main",
                    "code:file:projection",
                    DECLARES_SYMBOL,
                    "code:symbol:main",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:file:adapter",
                    "code:file:projection",
                    DECLARES_SYMBOL,
                    "code:symbol:adapter",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:file:config",
                    "code:file:projection",
                    DECLARES_SYMBOL,
                    "code:symbol:config",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:main:adapter",
                    "code:symbol:main",
                    CALLS_SYMBOL,
                    "code:symbol:adapter",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
                GraphMutation::EdgeUpsert(EdgeRecord::new(
                    "edge:main:config",
                    "code:symbol:main",
                    DEPENDS_ON_SYMBOL,
                    "code:symbol:config",
                    json!({ "tenant_id": tenant, "repo_id": repo_id }),
                )),
            ]))
            .unwrap();

        let projection = project_codebase_map_in_store(&store, tenant, repo_id, 12);
        let kinds = projection
            .entries
            .iter()
            .map(|entry| entry["kind"].as_str().unwrap().to_string())
            .collect::<BTreeSet<_>>();
        assert!(kinds.contains("module"));
        assert!(kinds.contains("entry_point"));
        assert!(kinds.contains("dependency"));
        assert!(kinds.contains("key_symbol"));
        assert!(projection.markdown_body.contains("## Modules"));
        assert!(projection.markdown_body.contains("## Entry points"));
        assert!(projection
            .entries
            .iter()
            .all(|entry| { entry["metadata"]["pagerank_score"].as_f64().unwrap_or(0.0) > 0.0 }));

        fs::remove_dir_all(store_dir).ok();
    }

    #[test]
    fn local_ingest_with_map_sink_writes_codebase_markdown() {
        let (repo_dir, store_dir) = write_fixture_repo();
        let store = RedCoreGraphStore::open(&store_dir, test_options()).unwrap();
        let runtime = CodeIndexRuntime::try_new_with_store_and_map_projection_sink(
            store,
            Arc::new(CodebaseMarkdownFileSink),
        )
        .unwrap();
        let output = runtime
            .ingest_codebase(IngestCodebaseInput {
                tenant_id: "theorem".to_string(),
                repo_path: repo_dir.display().to_string(),
                actor: "codex-test".to_string(),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(output.status, "ok");
        let codebase_md = fs::read_to_string(repo_dir.join("codebase.md")).unwrap();
        assert!(codebase_md.contains("# CodebaseMap for "));
        assert!(codebase_md.contains("## Modules"));
        assert!(codebase_md.contains("## Key symbols") || codebase_md.contains("## Entry points"));

        drop(runtime);
        fs::remove_dir_all(repo_dir).ok();
        fs::remove_dir_all(store_dir).ok();
    }
}
