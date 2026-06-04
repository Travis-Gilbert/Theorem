//! Durable native code index over RedCore.
//!
//! This module owns the local graph shape for code search. It is intentionally
//! independent of tonic so the same runtime can be called by the direct
//! CodeCrawlerService and by AppAffordanceService handlers.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rustyred_thg_core::{
    now_ms, stable_hash, Direction, EdgeRecord, GraphMutation, GraphMutationBatch, GraphStoreError,
    NeighborQuery, NodeQuery, NodeRecord, RedCoreDurability, RedCoreGraphStore, RedCoreOptions,
};
use serde_json::{json, Value};

const CODE_REPO_LABEL: &str = "CodeRepository";
const CODE_FILE_LABEL: &str = "CodeFile";
const CODE_SYMBOL_LABEL: &str = "CodeSymbol";
const CODE_RECEIPT_LABEL: &str = "CodeInvocationReceipt";
const SOURCE: &str = "theorem_grpc_code_index";
const CONTAINS_FILE: &str = "CONTAINS_FILE";
const DECLARES_SYMBOL: &str = "DECLARES_SYMBOL";
const CALLS_SYMBOL: &str = "CALLS_SYMBOL";
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

    pub fn reindex_codebase(
        &self,
        input: IngestCodebaseInput,
    ) -> Result<IngestCodebaseOutput, CodeIndexError> {
        let mut store = self.lock_store()?;
        ingest_codebase_with_store(&mut store, input, "reindex")
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

    fn lock_store(&self) -> Result<std::sync::MutexGuard<'_, RedCoreGraphStore>, CodeIndexError> {
        self.store.lock().map_err(|_| CodeIndexError {
            code: "code_index_lock_poisoned".to_string(),
            message: "code index RedCore store lock poisoned".to_string(),
        })
    }
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

impl std::fmt::Display for CodeIndexError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CodeIndexError {}

struct IngestConfig {
    tenant_id: String,
    repo_root: PathBuf,
    repo_root_display: String,
    repo_id: String,
    include_extensions: BTreeSet<String>,
    exclude_dirs: BTreeSet<String>,
    max_files: usize,
    max_file_bytes: u64,
    actor: String,
    generation: u64,
}

#[derive(Clone)]
struct IndexedFile {
    file_id: String,
    rel_path: String,
    language: String,
    extension: String,
    content_hash: String,
    text: String,
    symbols: Vec<IndexedSymbol>,
}

#[derive(Clone)]
struct IndexedSymbol {
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
    let mut symbols_indexed = 0u64;
    for file in &collected {
        symbols_indexed += file.symbols.len() as u64;
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
    for edge in infer_symbol_call_edges(&collected, &config) {
        mutations.push(GraphMutation::EdgeUpsert(edge));
    }

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
        "{} `{}` is a {} in `{}`. Trust tier: {}. It calls {} symbol(s) and is called by {} symbol(s).",
        symbol.language,
        symbol.name,
        symbol.kind,
        symbol.file_path,
        symbol.trust_tier,
        symbol.callees.len(),
        symbol.callers.len()
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

fn collect_code_files(
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

fn resolve_ingest_config(input: IngestCodebaseInput) -> Result<IngestConfig, CodeIndexError> {
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
        for (name, targets) in &symbols_by_name {
            if *name == symbol.name.as_str() || !body_references_name(&symbol.body, name) {
                continue;
            }
            for target in targets {
                if target.symbol_id == symbol.symbol_id {
                    continue;
                }
                let call_edge_id = edge_id(
                    "code:edge:symbol_call",
                    &symbol.symbol_id,
                    &target.symbol_id,
                );
                if !seen.insert(call_edge_id.clone()) {
                    continue;
                }
                edges.push(EdgeRecord::new(
                    call_edge_id,
                    &symbol.symbol_id,
                    CALLS_SYMBOL,
                    &target.symbol_id,
                    json!({
                        "tenant_id": config.tenant_id,
                        "repo_id": config.repo_id,
                        "generation": config.generation,
                        "evidence": format!("{} body references {}", symbol.name, target.name),
                        "source": SOURCE,
                    }),
                ));
            }
        }
    }
    edges
}

fn symbol_from_line(line: &str, language: &str) -> Option<(String, String)> {
    if line.is_empty() || line.starts_with("//") || line.starts_with('#') {
        return None;
    }
    let normalized = strip_leading_modifiers(line);
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
        for direction in [Direction::Out, Direction::In] {
            let neighbors = store
                .neighbors(NeighborQuery {
                    node_id: node_id.clone(),
                    direction,
                    edge_type: Some(CALLS_SYMBOL.to_string()),
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
    symbol.callees = neighbor_symbol_names(store, &symbol.node_id, Direction::Out)?;
    symbol.callers = neighbor_symbol_names(store, &symbol.node_id, Direction::In)?;
    Ok(())
}

fn neighbor_symbol_names(
    store: &RedCoreGraphStore,
    node_id: &str,
    direction: Direction,
) -> Result<Vec<String>, CodeIndexError> {
    let mut names = store
        .neighbors(NeighborQuery {
            node_id: node_id.to_string(),
            direction,
            edge_type: Some(CALLS_SYMBOL.to_string()),
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
        "rs", "swift", "py", "ts", "tsx", "js", "jsx", "mjs", "cjs", "proto", "toml", "md", "json",
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
            "pub struct SearchKernel {}\n\npub fn helper_len(query: &str) -> usize {\n    query.len()\n}\n\npub fn search_code(query: &str) -> usize {\n    helper_len(query)\n}\n",
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
