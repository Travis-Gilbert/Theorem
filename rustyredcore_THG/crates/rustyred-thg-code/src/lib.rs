//! Code parsing plugin/runtime over RedCore.
//!
//! This crate owns the graph shape for code search. It is intentionally
//! independent of tonic/MCP/HTTP so every transport can call the same parser
//! and write into the caller's `RedCoreGraphStore`.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustyred_thg_core::{
    now_ms, stable_hash, Direction, EdgeRecord, GraphMutation, GraphMutationBatch, GraphStoreError,
    GraphStoreResult, NeighborQuery, NodeQuery, NodeRecord, PluginCapability, PluginCapabilityKind,
    PluginExecutionOutput, PluginOperationContext, PluginOperationRegistration, PluginRegistry,
    RedCoreDurability, RedCoreGraphStore, RedCoreOptions, RustyRedPlugin,
};
use serde_json::{json, Value};
use syn::visit::Visit;

mod repo_fetch;
pub use repo_fetch::{
    fetch_repo, is_fetchable_repo_url, FetchedRepo, RepoFetchCaps, RepoFetchError,
};

pub const CODE_REPO_LABEL: &str = "CodeRepository";
pub const CODE_FILE_LABEL: &str = "CodeFile";
pub const CODE_SYMBOL_LABEL: &str = "CodeSymbol";
pub const CODE_RECEIPT_LABEL: &str = "CodeInvocationReceipt";
pub const SOURCE: &str = "rustyred_thg_code";
pub const CONTAINS_FILE: &str = "CONTAINS_FILE";
pub const DECLARES_SYMBOL: &str = "DECLARES_SYMBOL";
pub const CALLS_SYMBOL: &str = "CALLS_SYMBOL";
pub const DEPENDS_ON_SYMBOL: &str = "DEPENDS_ON_SYMBOL";
const DEFAULT_TRUST_TIER: &str = "advisory";
const DEFAULT_MAX_FILES: usize = 2_500;
const DEFAULT_MAX_FILE_BYTES: u64 = 1_000_000;
const DEFAULT_LIMIT: usize = 20;
const DEFAULT_CONTEXT_LINES: u64 = 20;
const DEFAULT_MAX_CONTEXT_CHARS: usize = 20_000;

#[derive(Clone)]
pub struct CodeIndexRuntime {
    store: Arc<Mutex<RedCoreGraphStore>>,
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
                CODE_RECEIPT_LABEL,
            ],
            edge_types: &[
                CONTAINS_FILE,
                DECLARES_SYMBOL,
                CALLS_SYMBOL,
                DEPENDS_ON_SYMBOL,
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
        ]
    }
}

impl CodeIndexRuntime {
    pub fn try_new() -> Result<Self, CodeIndexError> {
        Self::try_new_at(code_index_data_dir(), code_index_options())
    }

    pub fn try_new_at(
        data_dir: impl AsRef<Path>,
        options: RedCoreOptions,
    ) -> Result<Self, CodeIndexError> {
        let store = RedCoreGraphStore::open(data_dir.as_ref(), options)
            .map_err(CodeIndexError::from_store)?;
        Self::try_new_with_store(store)
    }

    pub fn try_new_with_store(store: RedCoreGraphStore) -> Result<Self, CodeIndexError> {
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
        })
    }

    pub fn ingest_codebase(
        &self,
        input: IngestCodebaseInput,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        ingest_codebase_with_store(&mut store, input, "ingest")
    }

    pub fn ingest_codebase_from_url(
        &self,
        url: &str,
        input: IngestCodebaseInput,
        caps: &RepoFetchCaps,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        ingest_codebase_from_url_in_store(&mut store, url, input, caps)
    }

    pub fn reindex_codebase(
        &self,
        input: IngestCodebaseInput,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        ingest_codebase_with_store(&mut store, input, "reindex")
    }

    pub fn reindex_codebase_from_url(
        &self,
        url: &str,
        input: IngestCodebaseInput,
        caps: &RepoFetchCaps,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        reindex_codebase_from_url_in_store(&mut store, url, input, caps)
    }

    pub fn search_code(&self, input: SearchCodeInput) -> Result<SearchCodeOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        search_code_with_store(&mut store, input)
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
    ingest_codebase_with_store(store, input, "ingest")
}

pub fn reindex_codebase_in_store(
    store: &mut RedCoreGraphStore,
    input: IngestCodebaseInput,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    ingest_codebase_with_store(store, input, "reindex")
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
    ingest_codebase_from_url_with_operation(store, url, input, caps, "ingest")
}

pub fn reindex_codebase_from_url_in_store(
    store: &mut RedCoreGraphStore,
    url: &str,
    input: IngestCodebaseInput,
    caps: &RepoFetchCaps,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    ingest_codebase_from_url_with_operation(store, url, input, caps, "reindex")
}

fn ingest_codebase_from_url_with_operation(
    store: &mut RedCoreGraphStore,
    url: &str,
    mut input: IngestCodebaseInput,
    caps: &RepoFetchCaps,
    operation: &str,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    let fetched = fetch_repo(url, caps).map_err(|err| CodeIndexError::invalid(err.to_string()))?;
    input.repo_path = fetched.path().display().to_string();
    if !input.exclude_dirs.iter().any(|dir| dir == ".git") {
        input.exclude_dirs.push(".git".to_string());
    }
    if input.repo_id.trim().is_empty() {
        input.repo_id = repo_id_from_url(url);
    }
    // `fetched` stays alive across the ingest and removes the clone on drop.
    ingest_codebase_with_store(store, input, operation)
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
        actor: code_plugin_arg_string(&arguments, &["actor", "actor_id", "actorId"]),
    };
    let output = if !repo_url.trim().is_empty() {
        // CA-1: ingest by URL -> shallow clone into a quarantined tempdir ->
        // ingest the local checkout (the clone is removed afterward).
        if context.operation == "reindex" {
            reindex_codebase_from_url_in_store(
                context.store,
                &repo_url,
                input,
                &RepoFetchCaps::default(),
            )?
        } else {
            ingest_codebase_from_url_in_store(
                context.store,
                &repo_url,
                input,
                &RepoFetchCaps::default(),
            )?
        }
    } else if context.operation == "reindex" {
        reindex_codebase_in_store(context.store, input)?
    } else {
        ingest_codebase_in_store(context.store, input)?
    };
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
    pub graph_version: u64,
    pub receipt_hash: String,
    pub receipt_json: String,
    pub status: String,
    pub message: String,
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
            "graph_version": self.graph_version,
            "receipt_hash": self.receipt_hash,
            "status": self.status,
            "message": self.message,
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
    pub actor: String,
    pub generation: u64,
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
            "source": SOURCE,
        }),
    );

    let mut mutations = vec![GraphMutation::NodeUpsert(repo_node)];
    for file in files {
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
                "text": file.text,
                "generation": config.generation,
                "indexed_at_ms": config.generation,
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
    for edge in infer_symbol_call_edges(files, config) {
        mutations.push(GraphMutation::EdgeUpsert(edge));
    }

    mutations
}

fn ingest_codebase_with_store(
    store: &mut RedCoreGraphStore,
    input: IngestCodebaseInput,
    operation: &str,
) -> Result<IngestCodebaseOutput, CodeIndexError> {
    let config = resolve_ingest_config(input)?;
    let mut collected = Vec::new();
    let mut skipped = 0u64;
    collect_code_files(&config.repo_root, &config, &mut collected, &mut skipped)?;

    let mutations = build_code_mutations(&config, &collected);
    let symbols_indexed: u64 = collected.iter().map(|file| file.symbols.len() as u64).sum();

    let transaction = store
        .commit_batch(GraphMutationBatch::new(mutations))
        .map_err(CodeIndexError::from_store)?;

    let summary = json!({
        "tenant_id": config.tenant_id,
        "repo_id": config.repo_id,
        "repo_root": config.repo_root_display,
        "generation": config.generation,
        "operation": operation,
        "files_indexed": collected.len(),
        "symbols_indexed": symbols_indexed,
        "files_skipped": skipped,
        "graph_version": transaction.graph_version,
        "actor": config.actor.clone(),
    });
    let receipt = record_receipt(
        store,
        &config.tenant_id,
        &format!("code_{operation}"),
        &summary,
    )?;

    Ok(IngestCodebaseOutput {
        tenant_id: config.tenant_id,
        repo_id: config.repo_id,
        repo_root: config.repo_root_display,
        generation: config.generation,
        files_indexed: collected.len() as u64,
        symbols_indexed,
        files_skipped: skipped,
        graph_version: receipt.graph_version,
        receipt_hash: receipt.receipt_hash,
        receipt_json: receipt.receipt_json,
        status: "ok".to_string(),
        message: "codebase indexed into RedCore".to_string(),
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

    let text = property_string(&file.properties, "text").unwrap_or_default();
    let file_path = property_string(&file.properties, "path").unwrap_or_default();
    let before = nonzero_or(input.before_lines, DEFAULT_CONTEXT_LINES);
    let after = nonzero_or(input.after_lines, DEFAULT_CONTEXT_LINES);
    let max_chars = nonzero_or(input.max_chars, DEFAULT_MAX_CONTEXT_CHARS as u64) as usize;
    let (start_line, end_line, context) =
        line_context(&text, target_line, before, after, max_chars);
    let symbols = symbols_for_file(store, &tenant_id, &file_id, &repo_id, latest.get(&repo_id))?;

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
    let mut edges = Vec::new();
    let mut related_ids = BTreeSet::new();
    expand_symbol_edges(
        store,
        &focus_node.id,
        max_depth,
        limit,
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
                enrich_symbol_graph(store, &mut symbol)?;
                related_symbols.push(symbol);
            }
        }
    }
    let mut focus = symbol_record_from_node(&focus_node);
    if let Some(symbol) = focus.as_mut() {
        enrich_symbol_graph(store, symbol)?;
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
    enrich_symbol_graph(store, &mut symbol)?;
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
    let entries =
        fs::read_dir(dir).map_err(|err| CodeIndexError::io("read directory", dir, err))?;
    for entry in entries {
        let entry = entry.map_err(|err| CodeIndexError::io("read directory entry", dir, err))?;
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().to_string();
        let metadata = entry
            .metadata()
            .map_err(|err| CodeIndexError::io("read metadata", &path, err))?;
        if metadata.is_dir() {
            if should_skip_dir(&file_name, &config.exclude_dirs) {
                continue;
            }
            collect_code_files(&path, config, out, skipped)?;
            if out.len() >= config.max_files {
                return Ok(());
            }
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        let extension = extension_for(&path);
        if !config.include_extensions.contains(&extension) {
            *skipped += 1;
            continue;
        }
        if metadata.len() > config.max_file_bytes {
            *skipped += 1;
            continue;
        }
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            Err(_) => {
                *skipped += 1;
                continue;
            }
        };
        let rel_path = relative_path(&config.repo_root, &path)?;
        let language = language_for_extension(&extension).to_string();
        let file_id = file_node_id(&config.repo_id, &rel_path);
        let content_hash = stable_hash(json!({
            "repo_id": config.repo_id,
            "path": rel_path,
            "content": text,
        }));
        let symbols = extract_symbols(&config.repo_id, &file_id, &rel_path, &language, &text);
        out.push(IndexedFile {
            file_id,
            rel_path,
            language,
            extension,
            content_hash,
            text,
            symbols,
        });
        if out.len() >= config.max_files {
            return Ok(());
        }
    }
    Ok(())
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
        input.max_files.min(DEFAULT_MAX_FILES as u64) as usize
    };
    let max_file_bytes = if input.max_file_bytes == 0 {
        DEFAULT_MAX_FILE_BYTES
    } else {
        input.max_file_bytes.min(DEFAULT_MAX_FILE_BYTES)
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
        actor: input.actor.trim().to_string(),
        generation: now_ms().max(1) as u64,
    })
}

fn extract_symbols(
    repo_id: &str,
    file_id: &str,
    file_path: &str,
    language: &str,
    text: &str,
) -> Vec<IndexedSymbol> {
    let mut symbols = extract_line_symbols(repo_id, file_id, file_path, language, text);
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

fn infer_symbol_call_edges(files: &[IndexedFile], config: &IngestConfig) -> Vec<EdgeRecord> {
    let mut symbols_by_name: HashMap<&str, Vec<&IndexedSymbol>> = HashMap::new();
    for symbol in files.iter().flat_map(|file| file.symbols.iter()) {
        symbols_by_name
            .entry(symbol.name.as_str())
            .or_default()
            .push(symbol);
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
                    "rust_ast_call",
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
                    "rust_ast_dependency",
                    config,
                );
            }
        } else {
            for (name, _) in &symbols_by_name {
                if *name == symbol.name.as_str() || !body_references_name(&symbol.body, name) {
                    continue;
                }
                push_symbol_edges(
                    &mut edges,
                    &mut seen,
                    &symbols_by_name,
                    symbol,
                    name,
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
fn push_symbol_edges(
    edges: &mut Vec<EdgeRecord>,
    seen: &mut BTreeSet<String>,
    symbols_by_name: &HashMap<&str, Vec<&IndexedSymbol>>,
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
    for target in targets {
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
                "evidence": format!("{evidence_kind}: {} references {}", symbol.name, target.name),
                "source": SOURCE,
            }),
        ));
    }
}

fn symbol_from_line(line: &str, language: &str) -> Option<(String, String)> {
    if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
        return None;
    }
    let normalized = strip_leading_modifiers(line);
    if language == "go" {
        return go_symbol_from_line(normalized);
    }
    let patterns: &[(&str, &str)] = match language {
        "python" => &[("def ", "function"), ("class ", "class")],
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
    latest_generation: Option<&u64>,
) -> Result<Vec<CodeSymbolRecord>, CodeIndexError> {
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
        enrich_symbol_graph(store, symbol)?;
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
        enrich_symbol_graph(store, symbol)?;
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

fn expand_symbol_edges(
    store: &RedCoreGraphStore,
    focus_id: &str,
    max_depth: usize,
    limit: usize,
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
                    if !seen_edges.insert(neighbor.edge_id.clone()) {
                        continue;
                    }
                    if let Some(edge) = store
                        .get_edge(&neighbor.edge_id)
                        .map_err(CodeIndexError::from_store)?
                    {
                        if let Some(record) = graph_edge_record(store, &edge)? {
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
) -> Result<(), CodeIndexError> {
    symbol.callees = neighbor_symbol_names(store, &symbol.node_id, Direction::Out, CALLS_SYMBOL)?;
    symbol.callers = neighbor_symbol_names(store, &symbol.node_id, Direction::In, CALLS_SYMBOL)?;
    symbol.dependencies =
        neighbor_symbol_names(store, &symbol.node_id, Direction::Out, DEPENDS_ON_SYMBOL)?;
    symbol.dependents =
        neighbor_symbol_names(store, &symbol.node_id, Direction::In, DEPENDS_ON_SYMBOL)?;
    Ok(())
}

fn neighbor_symbol_names(
    store: &RedCoreGraphStore,
    node_id: &str,
    direction: Direction,
    edge_type: &str,
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
        .filter_map(|node| property_string(&node.properties, "name"))
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    Ok(names)
}

fn graph_edge_record(
    store: &RedCoreGraphStore,
    edge: &EdgeRecord,
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

fn body_references_name(body: &str, name: &str) -> bool {
    body.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '!')
        .any(|token| token == name)
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

fn normalize_tenant(raw: &str) -> String {
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
        "rs", "go", "swift", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "proto", "toml",
        "md", "json",
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
        "dist",
        "node_modules",
        "target",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

fn should_skip_dir(name: &str, excluded: &BTreeSet<String>) -> bool {
    let lower = name.to_ascii_lowercase();
    excluded.contains(&lower) || lower.starts_with('.')
}

fn extension_for(path: &Path) -> String {
    path.extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn language_for_extension(extension: &str) -> &str {
    match extension {
        "go" => "go",
        "py" => "python",
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
    use std::time::{SystemTime, UNIX_EPOCH};

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

    fn write_go_fixture_repo() -> (PathBuf, PathBuf) {
        let repo_dir = unique_dir("go-repo");
        fs::create_dir_all(repo_dir.join("internal")).unwrap();
        fs::write(
            repo_dir.join("go.mod"),
            "module example.com/boltbrowser\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(
            repo_dir.join("README.md"),
            "# boltbrowser fixture\n",
        )
        .unwrap();
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
        assert!(symbols.iter().any(|node| node
            .properties
            .get("name")
            .and_then(Value::as_str)
            == Some("helper_len")));

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
        assert!(symbols.iter().any(|node| node
            .properties
            .get("language")
            .and_then(Value::as_str)
            == Some("go")));
        assert!(symbols.iter().any(|node| node
            .properties
            .get("name")
            .and_then(Value::as_str)
            == Some("main")));
        assert!(symbols.iter().any(|node| node
            .properties
            .get("kind")
            .and_then(Value::as_str)
            == Some("method")
            && node
                .properties
                .get("name")
                .and_then(Value::as_str)
                == Some("Draw")));

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
    fn plugin_manifest_describes_code_graph_capability() {
        let manifest = CodeParsingPlugin.manifest();
        assert_eq!(manifest.name, "rustyred.thg.code");
        assert!(manifest.labels.contains(&CODE_SYMBOL_LABEL));
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
}
