//! rustyred-embedded: North Star E0 (embedded mode), the linkable-core slice.
//!
//! `Engine` runs RustyRed in-process over a local directory: it owns a durable
//! `RedCoreGraphStore` behind the in-process `SharedStore` seam (from
//! `rustyred-thg-mcp`) and exposes the full typed GraphQL surface as plain local
//! calls (`query` / `mutate` / `introspect`). No socket, no async runtime, no
//! server -- the GraphQL document executes synchronously on the calling thread.
//!
//! Restart rehydration is the store's own contract: `RedCoreGraphStore::open`
//! replays the AOF via `recover()`, so re-opening an `Engine` over the same
//! directory sees prior writes (proven by `engine_rehydrates_after_restart`).
//!
//! This is E0.1 (linkable `Engine`) + E0.2 (restart rehydration) from
//! `docs/plans/rustyred-multimodel/E0-embedded-mode.md`. The single stdio binary
//! (E0.3), TOML config (E0.4), and folder-tree wiring (E0.5) build on top.

#[cfg(test)]
use std::cell::Cell;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_code_embedding::{CodeEmbedder, CodeEmbeddingConfig, CodeEmbeddingKind};
use rustyred_thg_core::{
    content_hash_bytes, ColdTierKind, DiskObjectStore, DocEntry, DocTree, PathKey,
    RedCoreDurability, RedCoreGraphStore, RedCoreOptions, DOC_TREE_CONTENT_HASH_PROPERTY,
    DOC_TREE_PATH_PROPERTY,
};
use rustyred_thg_mcp::{handle_mcp_request, McpServerConfig, SharedStore};
use serde::Deserialize;
use serde_json::{json, Value};

const LOCAL_ENGINE_TENANT: &str = "local";

/// Local embedded configuration (E0.4 will also load this from a
/// `theorem.toml`). Defaults to durable everysec, GraphQL as the default surface.
#[derive(Clone, Debug)]
pub struct EmbeddedConfig {
    /// AOF durability mode for the local store.
    pub durability: RedCoreDurability,
    /// Advertise GraphQL as the default agent path (hide the covered flat tools
    /// from `tools/list`); the embedded surface is GraphQL-first by default.
    pub graphql_default_surface: bool,
    /// Code/file embedding backend. Defaults to deterministic hash with the
    /// legacy E0.5 dimension; W4 can swap this to a hosted encoder without
    /// changing the file-write path.
    pub code_embedding: CodeEmbeddingConfig,
}

impl Default for EmbeddedConfig {
    fn default() -> Self {
        Self {
            durability: RedCoreDurability::AofEverysec,
            graphql_default_surface: true,
            code_embedding: CodeEmbeddingConfig::hash(FILE_EMBEDDING_DIM),
        }
    }
}

/// The on-disk `theorem.toml` shape: every field optional, so a partial (or
/// empty) file just overlays the defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlConfig {
    durability: Option<String>,
    graphql_default_surface: Option<bool>,
    code_embedding: Option<TomlCodeEmbeddingConfig>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlCodeEmbeddingConfig {
    kind: Option<String>,
    dimension: Option<usize>,
    url: Option<String>,
    timeout_secs: Option<u64>,
}

fn parse_durability(value: &str) -> Result<RedCoreDurability, EngineError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "none" => Ok(RedCoreDurability::None),
        "everysec" | "aof_everysec" | "aofeverysec" => Ok(RedCoreDurability::AofEverysec),
        "always" | "aof_always" | "aofalways" => Ok(RedCoreDurability::AofAlways),
        "snapshot" | "snapshot_only" | "snapshotonly" => Ok(RedCoreDurability::SnapshotOnly),
        other => Err(EngineError::Config(format!(
            "unknown durability `{other}` (expected none | everysec | always | snapshot)"
        ))),
    }
}

impl EmbeddedConfig {
    /// Parse a `theorem.toml` body, overlaying any present fields onto the
    /// defaults. Missing fields keep their default; an unknown durability string
    /// or malformed TOML is a clean `Err` (never a panic).
    pub fn from_toml_str(body: &str) -> Result<Self, EngineError> {
        let raw: TomlConfig =
            toml::from_str(body).map_err(|error| EngineError::Config(error.to_string()))?;
        let mut config = EmbeddedConfig::default();
        if let Some(durability) = raw.durability {
            config.durability = parse_durability(&durability)?;
        }
        if let Some(flag) = raw.graphql_default_surface {
            config.graphql_default_surface = flag;
        }
        if let Some(code_embedding) = raw.code_embedding {
            config.code_embedding = parse_code_embedding_config(code_embedding)?;
        }
        Ok(config)
    }

    /// Load `theorem.toml` from `path`. Errors if the file is unreadable or
    /// malformed. Callers that want "absent file -> default" should check
    /// existence first (see `EmbeddedConfig::load_for_dir`).
    pub fn from_toml_file(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let body = std::fs::read_to_string(path.as_ref()).map_err(|error| {
            EngineError::Config(format!("reading {:?}: {error}", path.as_ref()))
        })?;
        Self::from_toml_str(&body)
    }

    /// Resolve the config for a data directory: load `{dir}/theorem.toml` if it
    /// exists, else the single-tenant local default. A malformed file is an
    /// `Err` (so a typo is surfaced, not silently ignored).
    pub fn load_for_dir(dir: impl AsRef<Path>) -> Result<Self, EngineError> {
        let path = dir.as_ref().join("theorem.toml");
        if path.exists() {
            Self::from_toml_file(path)
        } else {
            Ok(EmbeddedConfig::default())
        }
    }
}

fn parse_code_embedding_config(
    raw: TomlCodeEmbeddingConfig,
) -> Result<CodeEmbeddingConfig, EngineError> {
    let kind = match raw.kind.as_deref() {
        Some(kind) => CodeEmbeddingKind::parse(kind)
            .map_err(|error| EngineError::Config(error.to_string()))?,
        None if raw.url.as_ref().is_some_and(|url| !url.trim().is_empty()) => {
            CodeEmbeddingKind::Http
        }
        None => CodeEmbeddingKind::Hash,
    };
    let dimension = raw.dimension.unwrap_or(match kind {
        CodeEmbeddingKind::Hash => FILE_EMBEDDING_DIM,
        CodeEmbeddingKind::Http | CodeEmbeddingKind::Local => {
            rustyred_code_embedding::DEFAULT_REAL_CODE_EMBEDDING_DIM
        }
    });
    if dimension == 0 {
        return Err(EngineError::Config(
            "code_embedding.dimension must be positive".to_string(),
        ));
    }
    let mut config = match kind {
        CodeEmbeddingKind::Hash => CodeEmbeddingConfig::hash(dimension),
        CodeEmbeddingKind::Http => {
            let Some(url) = raw.url.filter(|url| !url.trim().is_empty()) else {
                return Err(EngineError::Config(
                    "code_embedding.url is required for kind = \"http\"".to_string(),
                ));
            };
            CodeEmbeddingConfig::http(url, dimension)
        }
        CodeEmbeddingKind::Local => CodeEmbeddingConfig::local(dimension),
    };
    if let Some(timeout_secs) = raw.timeout_secs {
        if timeout_secs == 0 {
            return Err(EngineError::Config(
                "code_embedding.timeout_secs must be positive".to_string(),
            ));
        }
        config.timeout_secs = timeout_secs;
    }
    Ok(config)
}

/// Errors surfaced by the embedded engine.
#[derive(Debug)]
pub enum EngineError {
    /// Opening / recovering the local store failed.
    Open(String),
    /// The GraphQL document returned an execution error, or the transport failed.
    Graphql(String),
    /// Loading / parsing the local `theorem.toml` config failed.
    Config(String),
    /// A post-file-write maintenance hook failed after the File nodes were durable.
    Hook(String),
    /// A configured code/file embedder failed while preparing a `File` node.
    Embedding(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Open(message) => write!(f, "open: {message}"),
            EngineError::Graphql(message) => write!(f, "graphql: {message}"),
            EngineError::Config(message) => write!(f, "config: {message}"),
            EngineError::Hook(message) => write!(f, "hook: {message}"),
            EngineError::Embedding(message) => write!(f, "embedding: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// One file write inside the embedded workspace filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileWrite {
    pub path: String,
    pub content: Vec<u8>,
}

impl FileWrite {
    pub fn new(path: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

/// Receipt for one file written into the embedded workspace filesystem.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileWriteReceipt {
    pub path: String,
    pub content_hash: String,
}

/// Outcome of removing a directory from the embedded workspace filesystem.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectoryRemoveDisposition {
    Removed,
    Missing,
    NotEmpty,
    NotDirectory,
}

/// Receipt for a batch re-embed of existing `File` nodes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileReembedReceipt {
    pub prefix: String,
    pub embedder: String,
    pub dimension: usize,
    pub files_scanned: usize,
    pub files_reembedded: usize,
    pub bytes_reembedded: u64,
    pub paths: Vec<String>,
}

/// Optional post-write maintenance for embedded workspace files.
///
/// Hooks run after `fs_write_batch` has committed the `File` graph nodes and
/// persisted the `DocTree` snapshot. They are intentionally generic so crates
/// above `rustyred-embedded` can attach domain-specific maintenance, such as
/// CodeCrawler-on-write indexing, without making the embedded core depend on
/// those domains.
pub trait FileWriteHook {
    fn after_file_write(
        &self,
        engine: &Engine,
        writes: &[FileWrite],
        receipts: &[FileWriteReceipt],
    ) -> Result<(), EngineError>;
}

impl<F> FileWriteHook for F
where
    F: Fn(&Engine, &[FileWrite], &[FileWriteReceipt]) -> Result<(), EngineError>,
{
    fn after_file_write(
        &self,
        engine: &Engine,
        writes: &[FileWrite],
        receipts: &[FileWriteReceipt],
    ) -> Result<(), EngineError> {
        self(engine, writes, receipts)
    }
}

/// The embedded engine: a durable, in-process RustyRed over a local directory.
pub struct Engine {
    store: SharedStore<RedCoreGraphStore>,
    config: McpServerConfig,
    // E0.5 folder tier: the CoW path-keyed document tree (B3's DocTree) is the
    // agent's working filesystem. Persisted as a JSON snapshot beside the store;
    // overflow bodies live in the disk object store. Behind a RefCell so the
    // folder API matches query/mutate's `&self` shape.
    doc_tree: RefCell<DocTree>,
    object_store: DiskObjectStore,
    doc_tree_path: PathBuf,
    file_embedder: Arc<dyn CodeEmbedder>,
    file_write_hooks: RefCell<Vec<Box<dyn FileWriteHook>>>,
    #[cfg(test)]
    doc_tree_persist_count: Cell<u64>,
}

impl Engine {
    /// Open (or create) an embedded engine rooted at `data_dir`. The store
    /// rehydrates from its AOF on open (`RedCoreGraphStore::open` -> `recover`);
    /// the folder tree rehydrates from `{data_dir}/doc-tree.json`.
    pub fn open(data_dir: impl Into<PathBuf>, config: EmbeddedConfig) -> Result<Self, EngineError> {
        let data_dir = data_dir.into();
        let options = RedCoreOptions {
            durability: config.durability,
            ..RedCoreOptions::default()
        };
        let store = RedCoreGraphStore::open(data_dir.as_path(), options)
            .map_err(|error| EngineError::Open(error.message))?;
        let object_store = DiskObjectStore::open(data_dir.join("objects"))
            .map_err(|error| EngineError::Open(error.message))?;
        let doc_tree_path = data_dir.join("doc-tree.json");
        let doc_tree = if doc_tree_path.exists() {
            // Persisted as path-keyed pairs (OrdMap<PathKey, _> has non-string keys
            // JSON cannot use as object keys; the slash path round-trips losslessly).
            let bytes = std::fs::read(&doc_tree_path)
                .map_err(|error| EngineError::Open(format!("reading doc tree: {error}")))?;
            let entries: Vec<(String, DocEntry)> = serde_json::from_slice(&bytes)
                .map_err(|error| EngineError::Open(format!("parsing doc tree: {error}")))?;
            let mut tree = DocTree::default();
            for (path, entry) in entries {
                let key = PathKey::from_slash_path(&path)
                    .map_err(|error| EngineError::Open(error.message))?;
                tree.put(key, entry);
            }
            tree
        } else {
            DocTree::default()
        };
        let file_embedder = config
            .code_embedding
            .build()
            .map_err(|error| EngineError::Config(format!("code embedding: {error}")))?;
        let file_embedding_dimension = file_embedder.dimension();
        let mcp_config = McpServerConfig {
            default_tenant: LOCAL_ENGINE_TENANT.to_string(),
            read_only: false,
            graphql_default_surface: config.graphql_default_surface,
            // Embedded callers want the full payload inline, never a fetch handle.
            tool_result_budget_bytes: 0,
            ..McpServerConfig::default()
        };
        let engine = Self {
            store: SharedStore::new(store),
            config: mcp_config,
            doc_tree: RefCell::new(doc_tree),
            object_store,
            doc_tree_path,
            file_embedder,
            file_write_hooks: RefCell::new(Vec::new()),
            #[cfg(test)]
            doc_tree_persist_count: Cell::new(0),
        };
        // E0.5 embedding: designate the file-embedding vector property so written
        // files are semantically searchable. Best-effort + idempotent across
        // restarts (re-designating an existing property is tolerated).
        let mutation = format!(
            "mutation{{ designateVector(label: \"File\", property: \"embedding\", dimension: {file_embedding_dimension}) }}"
        );
        let _ = engine.mutate(&mutation, json!({}));
        Ok(engine)
    }

    /// Run a closure with mutable access to the durable graph store backing this
    /// engine. This is the workspace seam used by import/materialize layers; it
    /// keeps the single embedded store authoritative instead of opening a second
    /// engine.
    pub fn with_store<R>(&self, f: impl FnOnce(&mut RedCoreGraphStore) -> R) -> R {
        self.store.with_store(f)
    }

    /// Register an opt-in post-write hook. Hooks run in registration order
    /// after `fs_write_batch` has made the `File` graph nodes and DocTree
    /// snapshot durable.
    pub fn register_file_write_hook<H>(&self, hook: H)
    where
        H: FileWriteHook + 'static,
    {
        self.file_write_hooks.borrow_mut().push(Box::new(hook));
    }

    /// Run a closure with read-only access to the folder tree.
    pub fn with_doc_tree<R>(&self, f: impl FnOnce(&DocTree) -> R) -> R {
        let tree = self.doc_tree.borrow();
        f(&tree)
    }

    /// Run a closure with mutable access to the folder tree. Callers must not
    /// re-enter folder APIs from inside the closure; the RefCell borrow enforces
    /// the same single-borrow discipline as the rest of the embedded surface.
    pub fn with_doc_tree_mut<R>(&self, f: impl FnOnce(&mut DocTree) -> R) -> R {
        let mut tree = self.doc_tree.borrow_mut();
        f(&mut tree)
    }

    /// The content-addressed object store used by the folder tree overflow path.
    pub fn object_store(&self) -> &DiskObjectStore {
        &self.object_store
    }

    /// Run a GraphQL query (read). Returns the `data` block on success.
    pub fn query(&self, gql: &str) -> Result<Value, EngineError> {
        self.run("graphql_query", gql, None)
    }

    /// Run a GraphQL query (read) with variables.
    pub fn query_with(&self, gql: &str, variables: Value) -> Result<Value, EngineError> {
        self.run("graphql_query", gql, Some(variables))
    }

    /// Run a GraphQL mutation (write) with variables. Returns the `data` block.
    pub fn mutate(&self, gql: &str, variables: Value) -> Result<Value, EngineError> {
        self.run("graphql_mutate", gql, Some(variables))
    }

    /// The schema SDL (`graphql_introspect`).
    pub fn introspect(&self) -> Result<Value, EngineError> {
        let response = self.call(
            "graphql_introspect",
            json!({ "tenant": LOCAL_ENGINE_TENANT }),
        );
        structured(&response)
    }

    /// Drive a raw MCP JSON-RPC request against this engine's store + config,
    /// in-process. The stdio binary (E0.3) uses this to serve the full MCP tool
    /// surface (initialize / tools/list / tools/call) with no transport layer.
    pub fn handle(&self, request: Value) -> Value {
        handle_mcp_request(&self.store, &self.config, request)
    }

    /// Write a file into the folder tree (E0.5). The path-keyed document tree is
    /// the agent's working filesystem: the write gains a content hash (via the
    /// CoW tree -- inline, or overflow into the disk object store), a durable
    /// graph node keyed by path (so the file is findable through the GraphQL
    /// surface too), and the tree is persisted so it rehydrates on restart.
    /// Returns the content hash.
    pub fn fs_write(&self, path: &str, content: &[u8]) -> Result<String, EngineError> {
        let mut receipts = self.fs_write_batch([FileWrite::new(path, content.to_vec())])?;
        Ok(receipts
            .pop()
            .map(|receipt| receipt.content_hash)
            .unwrap_or_default())
    }

    /// Link a DocTree path to a real filesystem file and upsert the downstream
    /// `File` node without copying the bytes into the DocTree object store.
    pub fn fs_link_real_file(
        &self,
        path: &str,
        real_path: &Path,
        content: &[u8],
    ) -> Result<FileWriteReceipt, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        let content_hash = content_hash_bytes(content);
        let created_ms = now_ms();
        let real_path = real_path
            .canonicalize()
            .unwrap_or_else(|_| real_path.to_path_buf());
        {
            let mut tree = self.doc_tree.borrow_mut();
            tree.put_real_file(
                key,
                real_path.to_string_lossy().into_owned(),
                content_hash.clone(),
                content.len() as u64,
                ColdTierKind::Cold,
                created_ms,
                None,
            );
        }
        self.mutate(
            "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
            json!({ "n": [file_node(
                path,
                &content_hash,
                content,
                self.file_embedder.as_ref(),
            )?] }),
        )?;
        self.persist_doc_tree()?;
        Ok(FileWriteReceipt {
            path: path.to_string(),
            content_hash,
        })
    }

    pub fn file_content_hash(content: &[u8]) -> String {
        content_hash_bytes(content)
    }

    /// Write multiple files into the folder tree while serializing
    /// `doc-tree.json` exactly once. This is the W0 import seam: the tree borrow
    /// is held for the batch, all `File` nodes are upserted together, and the
    /// snapshot is persisted only after the graph write succeeds.
    pub fn fs_write_batch<I>(&self, writes: I) -> Result<Vec<FileWriteReceipt>, EngineError>
    where
        I: IntoIterator<Item = FileWrite>,
    {
        let writes: Vec<FileWrite> = writes.into_iter().collect();
        if writes.is_empty() {
            return Ok(Vec::new());
        }

        let mut nodes = Vec::with_capacity(writes.len());
        let mut receipts = Vec::with_capacity(writes.len());
        let created_ms = now_ms();
        {
            let mut tree = self.doc_tree.borrow_mut();
            for write in &writes {
                let key = PathKey::from_slash_path(&write.path)
                    .map_err(|error| EngineError::Open(error.message))?;
                let entry = tree
                    .put_body(
                        key,
                        &write.content,
                        ColdTierKind::Cold,
                        created_ms,
                        None,
                        &self.object_store,
                    )
                    .map_err(|error| EngineError::Open(error.message))?;
                let content_hash = entry.content_hash.clone().unwrap_or_default();
                nodes.push(file_node(
                    &write.path,
                    &content_hash,
                    &write.content,
                    self.file_embedder.as_ref(),
                )?);
                receipts.push(FileWriteReceipt {
                    path: write.path.clone(),
                    content_hash,
                });
            }
        }

        self.mutate(
            "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
            json!({ "n": nodes }),
        )?;
        self.persist_doc_tree()?;
        self.run_file_write_hooks(&writes, &receipts)?;
        Ok(receipts)
    }

    /// Read a file's body from the folder tree (point lookup).
    pub fn fs_read(&self, path: &str) -> Result<Option<Vec<u8>>, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        if self
            .doc_tree
            .borrow()
            .get(&key)
            .map(is_directory_entry)
            .unwrap_or(false)
        {
            return Ok(None);
        }
        self.doc_tree
            .borrow()
            .resolve_body(&key, &self.object_store)
            .map_err(|error| EngineError::Open(error.message))
    }

    /// List the paths under a prefix -- `ls` / `tree`, a prefix range scan over
    /// the document namespace.
    pub fn fs_ls(&self, prefix: &str) -> Result<Vec<String>, EngineError> {
        self.list_paths(prefix)
    }

    /// Create an explicit directory marker in the embedded workspace.
    /// Descendant files or directories already imply a synthetic directory, so
    /// those cases return `false` without adding a redundant marker.
    pub fn fs_mkdir(&self, path: &str) -> Result<bool, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        {
            let tree = self.doc_tree.borrow();
            if tree.get(&key).is_some() {
                return Ok(false);
            }
        }
        if self.fs_is_dir(path)? {
            return Ok(false);
        }
        self.doc_tree
            .borrow_mut()
            .put(key, directory_entry(now_ms()));
        self.persist_doc_tree()?;
        Ok(true)
    }

    /// Returns true for explicit directory markers and synthetic directories
    /// implied by descendant files or descendant explicit directories.
    pub fn fs_is_dir(&self, path: &str) -> Result<bool, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        {
            let tree = self.doc_tree.borrow();
            if let Some(entry) = tree.get(&key) {
                return Ok(is_directory_entry(entry));
            }
        }
        Ok(!self.file_entries(path)?.is_empty() || !self.directory_entries(path)?.is_empty())
    }

    /// Remove an empty explicit directory marker. Synthetic directories with
    /// descendant files or explicit child directories report `NotEmpty`.
    pub fn fs_rmdir(&self, path: &str) -> Result<DirectoryRemoveDisposition, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        {
            let tree = self.doc_tree.borrow();
            if let Some(entry) = tree.get(&key) {
                if !is_directory_entry(entry) {
                    return Ok(DirectoryRemoveDisposition::NotDirectory);
                }
            } else if self.file_entries(path)?.is_empty()
                && self.directory_entries(path)?.is_empty()
            {
                return Ok(DirectoryRemoveDisposition::Missing);
            } else {
                return Ok(DirectoryRemoveDisposition::NotEmpty);
            }
        }
        if !self.file_entries(path)?.is_empty() || !self.directory_entries(path)?.is_empty() {
            return Ok(DirectoryRemoveDisposition::NotEmpty);
        }
        self.doc_tree.borrow_mut().remove(&key);
        self.persist_doc_tree()?;
        Ok(DirectoryRemoveDisposition::Removed)
    }

    /// Remove one file from the embedded workspace filesystem and delete the
    /// matching `File` graph node through RedCore's durable `NodeDelete` path.
    /// Returns `false` when the DocTree path is already absent.
    pub fn fs_unlink(&self, path: &str) -> Result<bool, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        {
            let tree = self.doc_tree.borrow();
            let Some(entry) = tree.get(&key) else {
                return Ok(false);
            };
            if is_directory_entry(entry) {
                return Ok(false);
            }
        }

        let node_id = file_node_id(path);
        self.with_store(|store| {
            store
                .delete_node(&node_id)
                .map_err(|error| EngineError::Graphql(error.message))
        })?;

        let removed = self.doc_tree.borrow_mut().remove(&key);
        debug_assert!(removed, "DocTree path checked before unlink");
        self.persist_doc_tree()?;
        Ok(true)
    }

    /// Remove a downstream `File` node and any DocTree entry for `path`.
    /// Unlike `fs_unlink`, this also repairs stale graph-only index rows.
    pub fn fs_remove_file_index(&self, path: &str) -> Result<bool, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        let node_id = file_node_id(path);
        let node_removed = self.with_store(|store| {
            store
                .delete_node(&node_id)
                .map_err(|error| EngineError::Graphql(error.message))
        })?;
        let entry_removed = self.doc_tree.borrow_mut().remove(&key);
        if entry_removed {
            self.persist_doc_tree()?;
        }
        Ok(node_removed || entry_removed)
    }

    /// Rename one embedded workspace file. The implementation deliberately
    /// routes the destination through `fs_write`, so the new `File` node and
    /// post-write hooks match ordinary writes; the source is then durably
    /// unlinked through `fs_unlink`.
    pub fn fs_rename(&self, from: &str, to: &str) -> Result<bool, EngineError> {
        if from == to {
            return self.fs_read(from).map(|body| body.is_some());
        }
        let Some(body) = self.fs_read(from)? else {
            return Ok(false);
        };
        self.fs_write(to, &body)?;
        self.fs_unlink(from)?;
        Ok(true)
    }

    /// Explicitly refresh `File.embedding` vectors for an existing workspace
    /// subtree using the currently configured code embedder. This is W4's
    /// re-embed operation for encoder/model/dimension changes: it updates graph
    /// metadata from the durable DocTree bytes without rewriting file contents
    /// and without firing post-file-write hooks.
    pub fn reembed_files(&self, prefix: &str) -> Result<FileReembedReceipt, EngineError> {
        let entries = self.file_entries(prefix)?;
        let files_scanned = entries.len();
        let mut nodes = Vec::with_capacity(entries.len());
        let mut paths = Vec::with_capacity(entries.len());
        let mut bytes_reembedded = 0u64;
        for (path, entry) in entries {
            let key = PathKey::from_slash_path(&path)
                .map_err(|error| EngineError::Open(error.message))?;
            let Some(content) = self
                .doc_tree
                .borrow()
                .resolve_body(&key, &self.object_store)
                .map_err(|error| EngineError::Open(error.message))?
            else {
                continue;
            };
            bytes_reembedded += content.len() as u64;
            nodes.push(file_node(
                &path,
                entry.content_hash.as_deref().unwrap_or_default(),
                &content,
                self.file_embedder.as_ref(),
            )?);
            paths.push(path);
        }

        self.with_store(|store| {
            store
                .designate_vector_property("File", "embedding", self.file_embedder.dimension())
                .map_err(|error| EngineError::Graphql(error.message))
        })?;
        if !nodes.is_empty() {
            self.mutate(
                "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                json!({ "n": nodes }),
            )?;
        }
        let files_reembedded = paths.len();
        Ok(FileReembedReceipt {
            prefix: prefix.trim_matches('/').to_string(),
            embedder: self.file_embedder.name().to_string(),
            dimension: self.file_embedder.dimension(),
            files_scanned,
            files_reembedded,
            bytes_reembedded,
            paths,
        })
    }

    /// Enumerate every path under `prefix`; an empty prefix lists the whole
    /// workspace tree.
    pub fn list_paths(&self, prefix: &str) -> Result<Vec<String>, EngineError> {
        Ok(self
            .file_entries(prefix)?
            .into_iter()
            .map(|(path, _)| path)
            .collect())
    }

    /// Enumerate explicit directory markers under `prefix`; an empty prefix
    /// lists every persisted empty directory marker.
    pub fn list_directories(&self, prefix: &str) -> Result<Vec<String>, EngineError> {
        Ok(self
            .directory_entries(prefix)?
            .into_iter()
            .map(|(path, _)| path)
            .collect())
    }

    fn file_entries(&self, prefix: &str) -> Result<Vec<(String, DocEntry)>, EngineError> {
        Ok(self
            .doc_tree_entries(prefix)?
            .into_iter()
            .filter(|(_, entry)| !is_directory_entry(entry))
            .collect())
    }

    fn directory_entries(&self, prefix: &str) -> Result<Vec<(String, DocEntry)>, EngineError> {
        Ok(self
            .doc_tree_entries(prefix)?
            .into_iter()
            .filter(|(_, entry)| is_directory_entry(entry))
            .collect())
    }

    fn doc_tree_entries(&self, prefix: &str) -> Result<Vec<(String, DocEntry)>, EngineError> {
        let segments: Vec<&str> = prefix.split('/').filter(|s| !s.is_empty()).collect();
        let tree = self.doc_tree.borrow();
        if segments.is_empty() {
            Ok(tree
                .range_prefix(b"")
                .map(|(key, entry)| (key.to_slash_path(), entry.clone()))
                .collect())
        } else {
            let prefix_key = PathKey::prefix_from_segments(segments)
                .map_err(|error| EngineError::Open(error.message))?;
            Ok(tree
                .range_prefix(prefix_key.as_bytes())
                .map(|(key, entry)| (key.to_slash_path(), entry.clone()))
                .collect())
        }
    }

    fn persist_doc_tree(&self) -> Result<(), EngineError> {
        // Snapshot as path-keyed pairs (see `open`: JSON object keys must be
        // strings; PathKey is bytes, so the slash path is the portable key).
        let entries: Vec<(String, DocEntry)> = self
            .doc_tree
            .borrow()
            .range_prefix(b"")
            .map(|(key, entry)| (key.to_slash_path(), entry.clone()))
            .collect();
        let bytes = serde_json::to_vec(&entries)
            .map_err(|error| EngineError::Open(format!("serializing doc tree: {error}")))?;
        std::fs::write(&self.doc_tree_path, bytes)
            .map_err(|error| EngineError::Open(format!("writing doc tree: {error}")))?;
        #[cfg(test)]
        self.doc_tree_persist_count
            .set(self.doc_tree_persist_count.get() + 1);
        Ok(())
    }

    fn run_file_write_hooks(
        &self,
        writes: &[FileWrite],
        receipts: &[FileWriteReceipt],
    ) -> Result<(), EngineError> {
        let hooks = self.file_write_hooks.borrow();
        for hook in hooks.iter() {
            hook.after_file_write(self, writes, receipts)?;
        }
        Ok(())
    }

    #[cfg(test)]
    fn doc_tree_persist_count(&self) -> u64 {
        self.doc_tree_persist_count.get()
    }

    fn run(&self, tool: &str, gql: &str, variables: Option<Value>) -> Result<Value, EngineError> {
        let mut arguments = json!({ "tenant": LOCAL_ENGINE_TENANT, "query": gql });
        if let Some(variables) = variables {
            arguments["variables"] = variables;
        }
        let response = self.call(tool, arguments);
        let body = structured(&response)?;
        if let Some(errors) = body.get("errors").and_then(Value::as_array) {
            if !errors.is_empty() {
                let message = errors[0]["message"]
                    .as_str()
                    .unwrap_or("graphql error")
                    .to_string();
                return Err(EngineError::Graphql(message));
            }
        }
        Ok(body.get("data").cloned().unwrap_or(Value::Null))
    }

    fn call(&self, tool: &str, arguments: Value) -> Value {
        handle_mcp_request(
            &self.store,
            &self.config,
            json!({
                "jsonrpc": "2.0",
                "id": "embedded",
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments }
            }),
        )
    }
}

fn file_node(
    path: &str,
    content_hash: &str,
    content: &[u8],
    embedder: &dyn CodeEmbedder,
) -> Result<Value, EngineError> {
    let text = String::from_utf8_lossy(content);
    let embedding = embedder
        .embed_code(&text)
        .map_err(|error| EngineError::Embedding(error.to_string()))?;
    let mut properties = serde_json::Map::new();
    properties.insert(DOC_TREE_PATH_PROPERTY.to_string(), json!(path));
    properties.insert(
        DOC_TREE_CONTENT_HASH_PROPERTY.to_string(),
        json!(content_hash),
    );
    properties.insert("embedding".to_string(), json!(embedding_json(&embedding)));
    Ok(json!({
        "id": file_node_id(path),
        "labels": ["File"],
        "properties": properties
    }))
}

fn file_node_id(path: &str) -> String {
    format!("file:{path}")
}

const DIRECTORY_GIST: &str = "rustyred:directory";

fn directory_entry(created_ms: i64) -> DocEntry {
    DocEntry {
        content_hash: None,
        inline: None,
        real_path: None,
        inline_compressed: false,
        tier: ColdTierKind::Cold,
        size: 0,
        created_ms,
        gist: Some(DIRECTORY_GIST.to_string()),
        previous_hashes: Vec::new(),
    }
}

fn is_directory_entry(entry: &DocEntry) -> bool {
    entry.content_hash.is_none()
        && entry.inline.is_none()
        && entry.size == 0
        && entry.gist.as_deref() == Some(DIRECTORY_GIST)
}

/// Unwrap `result.structuredContent` from a JSON-RPC response, surfacing a
/// transport-level `error` as `EngineError::Graphql`.
fn structured(response: &Value) -> Result<Value, EngineError> {
    if let Some(error) = response.get("error") {
        return Err(EngineError::Graphql(error.to_string()));
    }
    Ok(response["result"]["structuredContent"].clone())
}

/// Milliseconds since the Unix epoch, for `DocEntry.created_ms`.
fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Legacy default dimension for the deterministic offline file embedding. W4
/// keeps this as the no-config path, while configured encoders supply their
/// own dimension.
const FILE_EMBEDDING_DIM: usize = 16;

fn embedding_json(vector: &[f32]) -> Vec<f64> {
    vector
        .iter()
        .copied()
        .map(|value| ((value as f64) * 1_000_000.0).round() / 1_000_000.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyred_thg_core::PathKey;
    use rustyred_thg_mcp::McpGraphBackend;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A unique temp directory per test (no Date/random needed: pid + counter).
    fn unique_temp_dir(tag: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let mut dir = std::env::temp_dir();
        dir.push(format!(
            "rustyred-embedded-{tag}-{}-{n}",
            std::process::id()
        ));
        dir
    }

    // E0.1: link the crate, open an Engine over a local dir, and run a full
    // write-then-read task through the typed GraphQL surface entirely in-process.
    #[test]
    fn engine_runs_graphql_in_process_over_local_dir() {
        let dir = unique_temp_dir("e01");
        let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");

        let inserted = engine
            .mutate(
                "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                json!({ "n": [
                    { "id": "a", "labels": ["Doc"], "properties": {} },
                    { "id": "b", "labels": ["Doc"], "properties": {} }
                ] }),
            )
            .expect("bulkNodes");
        assert_eq!(
            inserted["bulkNodes"]["inserted"],
            json!(2),
            "bulkNodes: {inserted}"
        );

        engine
            .mutate(
                "mutation($e:JSON!){ bulkEdges(edges:$e){ inserted } }",
                json!({ "e": [ { "id": "a->b", "from_id": "a", "to_id": "b", "type": "LINKS" } ] }),
            )
            .expect("bulkEdges");

        let result = engine
            .query("query{ neighbors(nodeId:\"a\", direction:\"out\") }")
            .expect("neighbors");
        let neighbors = &result["neighbors"]["neighbors"];
        assert!(
            neighbors.as_array().map(|a| !a.is_empty()).unwrap_or(false),
            "neighbors must find b in-process: {result}"
        );

        // The typed surface is introspectable in-process too.
        let sdl = engine.introspect().expect("introspect");
        assert!(
            sdl.as_str()
                .map(|s| s.contains("graphAlgorithm"))
                .unwrap_or(false),
            "introspect should return the SDL: {sdl}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // E0.2 (the North Star E0 headline acceptance): an agent runs the engine over
    // a local directory, restarts, and rehydrates. Write through the engine, drop
    // it, re-open over the SAME directory, and the data is still there.
    #[test]
    fn engine_rehydrates_after_restart() {
        let dir = unique_temp_dir("e02");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };

        {
            let engine = Engine::open(&dir, config.clone()).expect("open engine");
            let inserted = engine
                .mutate(
                    "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
                    json!({ "n": [ {
                        "id": "persisted",
                        "labels": ["Doc"],
                        "properties": { "title": "survives restart" }
                    } ] }),
                )
                .expect("bulkNodes");
            assert_eq!(inserted["bulkNodes"]["inserted"], json!(1), "{inserted}");
        } // engine dropped: the store closes, the AOF is fsynced (AofAlways).

        // Re-open over the SAME directory; recover() replays the AOF.
        let reopened = Engine::open(&dir, config).expect("re-open engine");
        let node = reopened
            .query("query{ graphNode(id:\"persisted\") }")
            .expect("graphNode");
        assert!(
            !node["graphNode"].is_null(),
            "the node must rehydrate through the agent surface after restart: {node}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // E0.4: theorem.toml overlays the defaults; empty file -> default; invalid
    // fields are clean Errs, never panics.
    #[test]
    fn embedded_config_parses_toml() {
        let cfg = EmbeddedConfig::from_toml_str(
            "durability = \"always\"\ngraphql_default_surface = false\n",
        )
        .expect("parse toml");
        assert!(
            matches!(cfg.durability, RedCoreDurability::AofAlways),
            "durability: {:?}",
            cfg.durability
        );
        assert!(!cfg.graphql_default_surface);
    }

    #[test]
    fn embedded_config_empty_toml_is_default() {
        let cfg = EmbeddedConfig::from_toml_str("").expect("empty toml");
        let default = EmbeddedConfig::default();
        assert!(matches!(cfg.durability, RedCoreDurability::AofEverysec));
        assert_eq!(cfg.graphql_default_surface, default.graphql_default_surface);
    }

    #[test]
    fn embedded_config_invalid_durability_is_clean_err() {
        let err = EmbeddedConfig::from_toml_str("durability = \"turbo\"\n");
        assert!(
            matches!(err, Err(EngineError::Config(_))),
            "invalid durability must be a clean Config err, got {err:?}"
        );
    }

    #[test]
    fn embedded_config_rejects_tenant_field() {
        let err = EmbeddedConfig::from_toml_str("tenant = \"acme\"\n");
        assert!(
            matches!(err, Err(EngineError::Config(_))),
            "embedded config must not expose tenant: {err:?}"
        );
    }

    #[test]
    fn embedded_public_surface_does_not_expose_tenant() {
        let source = include_str!("lib.rs");
        let forbidden = [
            concat!("pub ", "tenant"),
            concat!("pub fn ", "tenant"),
            concat!("pub(crate) ", "tenant"),
            concat!("pub struct ", "Tenant"),
            concat!("pub enum ", "Tenant"),
        ];
        for needle in forbidden {
            assert!(
                !source.contains(needle),
                "embedded public surface must stay tenancy-blind; found {needle:?}"
            );
        }
    }

    // E1: the stream-coordination transport answers through the embedded engine.
    // Publish an event then read it back via `engine.handle` (the flat stream_*
    // MCP tools, fully in-process over the durable store -- a local live-tail,
    // no server). The events persist as CoordinationStreamEvent graph nodes.
    #[test]
    fn engine_stream_publish_then_read_round_trips() {
        let dir = unique_temp_dir("e1");
        let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");

        let published = engine.handle(json!({
            "jsonrpc": "2.0", "id": "p", "method": "tools/call",
            "params": { "name": "stream_publish", "arguments": {
                "tenant": "local", "stream": "room:embedded", "actor": "codex",
                "kind": "hello", "payload": { "note": "hello" }
            } }
        }));
        assert!(
            published.get("error").is_none()
                && published["result"]["isError"].as_bool() != Some(true),
            "stream_publish must succeed in-process: {published}"
        );

        let read = engine.handle(json!({
            "jsonrpc": "2.0", "id": "r", "method": "tools/call",
            "params": { "name": "stream_read", "arguments": {
                "tenant": "local", "actor": "claude-code", "stream": "room:embedded"
            } }
        }));
        let events = &read["result"]["structuredContent"]["events"];
        assert!(
            events.as_array().map(|a| !a.is_empty()).unwrap_or(false),
            "stream_read must return the published event in-process: {read}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // E0.5: the folder tree is the agent's working filesystem. A written file
    // gains a content hash, is re-findable by point lookup AND prefix scan, AND
    // gains a durable graph node (findable through the GraphQL surface).
    #[test]
    fn engine_folder_tree_write_read_ls_and_graph_node() {
        let dir = unique_temp_dir("e05");
        let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");

        let hash = engine
            .fs_write("docs/readme", b"hello world")
            .expect("fs_write");
        assert!(!hash.is_empty(), "a written file gains a content hash");

        // Point lookup.
        let body = engine.fs_read("docs/readme").expect("fs_read");
        assert_eq!(body.as_deref(), Some(&b"hello world"[..]), "fs_read body");

        // Prefix range scan (ls / tree).
        engine
            .fs_write("docs/guide", b"the guide")
            .expect("fs_write guide");
        let listing = engine.fs_ls("docs").expect("fs_ls");
        assert!(
            listing.contains(&"docs/readme".to_string()),
            "ls under docs: {listing:?}"
        );
        assert!(
            listing.contains(&"docs/guide".to_string()),
            "ls under docs: {listing:?}"
        );

        // Re-findable as a graph node through the GraphQL surface.
        let node = engine
            .query("query{ graphNode(id:\"file:docs/readme\") }")
            .expect("graphNode");
        assert!(
            !node["graphNode"].is_null(),
            "the written file must also be a graph node: {node}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // W0: batch writes are the importer primitive. They hold one DocTree borrow,
    // write all File nodes together, and persist the doc-tree snapshot exactly
    // once for the whole batch.
    #[test]
    fn engine_batch_write_persists_once_and_exposes_workspace_seam() {
        let dir = unique_temp_dir("w0batch");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };
        let engine = Engine::open(&dir, config.clone()).expect("open engine");
        let before = engine.doc_tree_persist_count();

        let receipts = engine
            .fs_write_batch([
                FileWrite::new("README.md", b"# fixture\n".to_vec()),
                FileWrite::new("src/large.txt", vec![b'x'; 5000]),
                FileWrite::new("src/lib.rs", b"pub fn answer() -> u8 { 42 }\n".to_vec()),
            ])
            .expect("batch write");

        assert_eq!(receipts.len(), 3, "one receipt per written file");
        assert_eq!(
            engine.doc_tree_persist_count() - before,
            1,
            "batch import must persist doc-tree.json once"
        );

        let all_paths = engine.list_paths("").expect("list all paths");
        assert_eq!(
            all_paths,
            vec![
                "README.md".to_string(),
                "src/large.txt".to_string(),
                "src/lib.rs".to_string(),
            ],
            "list_paths(\"\") enumerates the workspace"
        );

        let tree_len = engine.with_doc_tree(|tree| tree.len());
        assert_eq!(tree_len, 3, "read-only DocTree seam");
        let mut_tree_len = engine.with_doc_tree_mut(|tree| tree.len());
        assert_eq!(mut_tree_len, 3, "mutable DocTree seam");
        let object_hash = engine
            .object_store()
            .put_document_bytes(b"object-store seam")
            .expect("object-store put");
        let object_body = engine
            .object_store()
            .get_document_bytes(&object_hash)
            .expect("object-store get");
        assert_eq!(
            object_body.as_deref(),
            Some(&b"object-store seam"[..]),
            "object-store seam reaches the overflow store"
        );

        let key = PathKey::from_slash_path("src/lib.rs").expect("path key");
        let doc_hash = engine
            .with_doc_tree(|tree| tree.get(&key).and_then(|entry| entry.content_hash.clone()))
            .expect("doc entry hash");
        assert_eq!(doc_hash, receipts[2].content_hash);

        let node = engine
            .with_store(|store| McpGraphBackend::get_node(store, "file:src/lib.rs"))
            .expect("store get_node")
            .expect("file graph node");
        assert_eq!(node.labels, vec!["File".to_string()]);

        drop(engine);
        let reopened = Engine::open(&dir, config).expect("re-open engine");
        let node = reopened
            .with_store(|store| McpGraphBackend::get_node(store, "file:src/lib.rs"))
            .expect("store get_node after restart")
            .expect("file graph node after restart");
        assert_eq!(node.labels, vec!["File".to_string()]);

        let _ = std::fs::remove_dir_all(&dir);
    }

    // E0.5 durability: the folder tree rehydrates after restart (the DocTree JSON
    // snapshot + the disk object store), so the agent's filesystem survives.
    #[test]
    fn engine_folder_tree_rehydrates_after_restart() {
        let dir = unique_temp_dir("e05r");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };
        {
            let engine = Engine::open(&dir, config.clone()).expect("open engine");
            engine
                .fs_write("notes/n1", b"persisted file")
                .expect("fs_write");
        }
        let reopened = Engine::open(&dir, config).expect("re-open engine");
        let body = reopened.fs_read("notes/n1").expect("fs_read");
        assert_eq!(
            body.as_deref(),
            Some(&b"persisted file"[..]),
            "the folder tree must rehydrate after restart"
        );
        assert!(
            reopened
                .fs_ls("notes")
                .expect("fs_ls")
                .contains(&"notes/n1".to_string()),
            "ls must list the rehydrated file"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // W6: explicit empty directories are durable DocTree metadata, not
    // zero-byte File nodes.
    #[test]
    fn engine_fs_mkdir_rehydrates_empty_directory_without_file_node() {
        let dir = unique_temp_dir("w6mkdir");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };
        {
            let engine = Engine::open(&dir, config.clone()).expect("open engine");
            assert!(engine.fs_mkdir("src/generated").expect("mkdir"));
            assert!(
                !engine.fs_mkdir("src/generated").expect("mkdir existing"),
                "second mkdir reports an existing directory"
            );
            assert!(
                engine
                    .fs_read("src/generated")
                    .expect("read directory")
                    .is_none(),
                "directory marker must not read as a zero-byte file"
            );
            assert!(
                engine.list_paths("").expect("list files").is_empty(),
                "directory marker is not a File path"
            );
            assert_eq!(
                engine.list_directories("").expect("list dirs"),
                vec!["src/generated".to_string()]
            );
            assert!(engine.fs_is_dir("src").expect("synthetic parent dir"));
            assert!(engine
                .with_store(|store| McpGraphBackend::get_node(store, "file:src/generated"))
                .expect("directory file node lookup")
                .is_none());
        }

        let reopened = Engine::open(&dir, config).expect("re-open engine");
        assert_eq!(
            reopened
                .list_directories("")
                .expect("list dirs after restart"),
            vec!["src/generated".to_string()],
            "explicit empty directories must survive DocTree snapshot rehydrate"
        );
        assert!(reopened.fs_is_dir("src/generated").expect("dir exists"));
        assert!(reopened
            .fs_read("src/generated")
            .expect("read dir after restart")
            .is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn engine_fs_rmdir_removes_only_empty_explicit_directories() {
        let dir = unique_temp_dir("w6rmdir");
        let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");
        engine
            .fs_write("src/lib.rs", b"pub fn lib() {}\n")
            .expect("write lib");
        engine.fs_mkdir("empty").expect("mkdir empty");
        engine.fs_mkdir("parent/child").expect("mkdir child");

        assert_eq!(
            engine.fs_rmdir("src").expect("rmdir synthetic src"),
            DirectoryRemoveDisposition::NotEmpty
        );
        assert_eq!(
            engine.fs_rmdir("src/lib.rs").expect("rmdir file"),
            DirectoryRemoveDisposition::NotDirectory
        );
        assert_eq!(
            engine.fs_rmdir("missing").expect("rmdir missing"),
            DirectoryRemoveDisposition::Missing
        );
        assert_eq!(
            engine.fs_rmdir("parent").expect("rmdir parent"),
            DirectoryRemoveDisposition::NotEmpty
        );
        assert_eq!(
            engine.fs_rmdir("empty").expect("rmdir empty"),
            DirectoryRemoveDisposition::Removed
        );
        assert!(!engine.fs_is_dir("empty").expect("empty gone"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    // W6: unlink is a durable filesystem mutation, not a DocTree-only edit. It
    // removes the bytes from the folder tree and the matching File graph node.
    #[test]
    fn engine_fs_unlink_removes_doctree_entry_and_file_node_across_restart() {
        let dir = unique_temp_dir("w6unlink");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };
        {
            let engine = Engine::open(&dir, config.clone()).expect("open engine");
            engine
                .fs_write("src/lib.rs", b"pub fn deleted() {}\n")
                .expect("write deleted file");
            engine
                .fs_write("src/main.rs", b"fn main() {}\n")
                .expect("write kept file");

            assert!(engine.fs_unlink("src/lib.rs").expect("unlink"));
            assert!(
                !engine.fs_unlink("src/lib.rs").expect("unlink missing"),
                "second unlink reports an already-missing file"
            );
            assert!(engine
                .fs_read("src/lib.rs")
                .expect("read deleted")
                .is_none());
            assert_eq!(
                engine.fs_read("src/main.rs").expect("read kept").as_deref(),
                Some(&b"fn main() {}\n"[..])
            );
            assert!(!engine
                .list_paths("src")
                .expect("list src")
                .contains(&"src/lib.rs".to_string()));
            assert!(
                engine
                    .with_store(|store| McpGraphBackend::get_node(store, "file:src/lib.rs"))
                    .expect("store get deleted")
                    .is_none(),
                "File graph metadata is deleted with the DocTree entry"
            );
        }

        let reopened = Engine::open(&dir, config).expect("re-open engine");
        assert!(
            reopened
                .fs_read("src/lib.rs")
                .expect("read deleted after restart")
                .is_none(),
            "unlink must survive DocTree snapshot rehydrate"
        );
        assert!(
            reopened
                .with_store(|store| McpGraphBackend::get_node(store, "file:src/lib.rs"))
                .expect("store get deleted after restart")
                .is_none(),
            "NodeDelete AOF entry must survive graph replay"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    // W6: rename rewrites the File path/node identity and keeps the body
    // readable at the destination after restart.
    #[test]
    fn engine_fs_rename_moves_body_and_file_node_across_restart() {
        let dir = unique_temp_dir("w6rename");
        let config = EmbeddedConfig {
            durability: RedCoreDurability::AofAlways,
            ..EmbeddedConfig::default()
        };
        {
            let engine = Engine::open(&dir, config.clone()).expect("open engine");
            engine
                .fs_write("src/old.rs", b"pub fn renamed() {}\n")
                .expect("write old");

            assert!(engine
                .fs_rename("src/old.rs", "src/new.rs")
                .expect("rename"));
            assert!(engine.fs_read("src/old.rs").expect("read old").is_none());
            assert_eq!(
                engine.fs_read("src/new.rs").expect("read new").as_deref(),
                Some(&b"pub fn renamed() {}\n"[..])
            );
            assert!(
                engine
                    .with_store(|store| McpGraphBackend::get_node(store, "file:src/old.rs"))
                    .expect("old graph node")
                    .is_none(),
                "old File node id is removed"
            );
            let new_node = engine
                .with_store(|store| McpGraphBackend::get_node(store, "file:src/new.rs"))
                .expect("new graph node")
                .expect("new File node");
            assert_eq!(
                new_node.properties[DOC_TREE_PATH_PROPERTY],
                json!("src/new.rs")
            );
        }

        let reopened = Engine::open(&dir, config).expect("re-open engine");
        assert!(reopened
            .fs_read("src/old.rs")
            .expect("read old after restart")
            .is_none());
        assert_eq!(
            reopened
                .fs_read("src/new.rs")
                .expect("read new after restart")
                .as_deref(),
            Some(&b"pub fn renamed() {}\n"[..])
        );
        assert!(reopened
            .with_store(|store| McpGraphBackend::get_node(store, "file:src/old.rs"))
            .expect("old graph node after restart")
            .is_none());
        assert!(reopened
            .with_store(|store| McpGraphBackend::get_node(store, "file:src/new.rs"))
            .expect("new graph node after restart")
            .is_some());

        let _ = std::fs::remove_dir_all(&dir);
    }

    // E0.5 embedding: a written file gains a deterministic offline embedding and
    // is semantically searchable -- vectorSearch over the designated File/embedding
    // property returns the file node. Completes E0.5's "content hash, an
    // embedding, and a graph node".
    #[test]
    fn engine_folder_file_is_vector_searchable() {
        let dir = unique_temp_dir("e05e");
        let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");
        engine
            .fs_write("docs/readme", b"hello embedded world")
            .expect("fs_write");

        let query_vec = engine
            .file_embedder
            .embed_code("hello embedded world")
            .expect("embed query");
        let hits = engine
            .query_with(
                "query($q:[Float!]!){ vectorSearch(property:\"embedding\", query:$q, label:\"File\", k:5){ nodeId score } }",
                json!({ "q": query_vec }),
            )
            .expect("vectorSearch");
        let ids: Vec<&str> = hits["vectorSearch"]
            .as_array()
            .expect("vectorSearch list")
            .iter()
            .filter_map(|hit| hit["nodeId"].as_str())
            .collect();
        assert!(
            ids.contains(&"file:docs/readme"),
            "the written file must be vector-searchable via its embedding: {hits}"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn engine_file_embedding_dimension_follows_configured_embedder() {
        let dir = unique_temp_dir("w4dim");
        let config = EmbeddedConfig {
            code_embedding: CodeEmbeddingConfig::hash(7),
            ..EmbeddedConfig::default()
        };
        let engine = Engine::open(&dir, config).expect("open engine");
        engine
            .fs_write("src/lib.rs", b"pub fn alpha() -> usize { 1 }\n")
            .expect("fs_write");

        let designations = engine.with_store(|store| store.vector_designations());
        assert!(
            designations.iter().any(|designation| {
                designation.label == "File"
                    && designation.property == "embedding"
                    && designation.dimension == 7
            }),
            "File/embedding designation must use the configured dimension: {designations:?}"
        );
        let query_vec = engine
            .file_embedder
            .embed_code("pub fn alpha() -> usize { 1 }\n")
            .expect("embed query");
        assert_eq!(query_vec.len(), 7);
        let hits = engine
            .query_with(
                "query($q:[Float!]!){ vectorSearch(property:\"embedding\", query:$q, label:\"File\", k:1){ nodeId score } }",
                json!({ "q": query_vec }),
            )
            .expect("vectorSearch");
        assert_eq!(hits["vectorSearch"][0]["nodeId"], "file:src/lib.rs");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn reembed_files_refreshes_existing_file_vectors_after_dimension_change() {
        let dir = unique_temp_dir("w4reembed");
        {
            let engine = Engine::open(&dir, EmbeddedConfig::default()).expect("open engine");
            engine
                .fs_write("src/lib.rs", b"pub fn alpha() -> usize { 1 }\n")
                .expect("fs_write");
        }

        let config = EmbeddedConfig {
            code_embedding: CodeEmbeddingConfig::hash(7),
            ..EmbeddedConfig::default()
        };
        let engine = Engine::open(&dir, config).expect("re-open engine with new embedding dim");
        let query_vec = engine
            .file_embedder
            .embed_code("pub fn alpha() -> usize { 1 }\n")
            .expect("embed query");
        assert_eq!(query_vec.len(), 7);
        let before = engine
            .query_with(
                "query($q:[Float!]!){ vectorSearch(property:\"embedding\", query:$q, label:\"File\", k:5){ nodeId score } }",
                json!({ "q": query_vec.clone() }),
            )
            .expect("vectorSearch before reembed");
        assert!(
            before["vectorSearch"]
                .as_array()
                .expect("vectorSearch list")
                .is_empty(),
            "old 16-d File embedding should not satisfy the new 7-d designation before re-embed: {before}"
        );

        let receipt = engine.reembed_files("src").expect("reembed files");
        assert_eq!(receipt.files_scanned, 1);
        assert_eq!(receipt.files_reembedded, 1);
        assert_eq!(receipt.dimension, 7);
        assert_eq!(receipt.embedder, "hash");
        assert_eq!(receipt.paths, vec!["src/lib.rs".to_string()]);

        let after = engine
            .query_with(
                "query($q:[Float!]!){ vectorSearch(property:\"embedding\", query:$q, label:\"File\", k:5){ nodeId score } }",
                json!({ "q": query_vec }),
            )
            .expect("vectorSearch after reembed");
        assert_eq!(after["vectorSearch"][0]["nodeId"], "file:src/lib.rs");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
