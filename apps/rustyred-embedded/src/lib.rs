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

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{
    ColdTierKind, DiskObjectStore, DocEntry, DocTree, PathKey, RedCoreDurability,
    RedCoreGraphStore, RedCoreOptions, DOC_TREE_CONTENT_HASH_PROPERTY, DOC_TREE_PATH_PROPERTY,
};
use rustyred_thg_mcp::{handle_mcp_request, McpServerConfig, SharedStore};
use serde::Deserialize;
use serde_json::{json, Value};

/// Local, single-tenant embedded configuration (E0.4 will also load this from a
/// `theorem.toml`). Defaults to durable everysec, GraphQL as the default surface.
#[derive(Clone, Debug)]
pub struct EmbeddedConfig {
    /// The single local tenant. The GraphQL surface rejects an empty tenant, so
    /// this must be non-empty; defaults to `local`.
    pub tenant: String,
    /// AOF durability mode for the local store.
    pub durability: RedCoreDurability,
    /// Advertise GraphQL as the default agent path (hide the covered flat tools
    /// from `tools/list`); the embedded surface is GraphQL-first by default.
    pub graphql_default_surface: bool,
}

impl Default for EmbeddedConfig {
    fn default() -> Self {
        Self {
            tenant: "local".to_string(),
            durability: RedCoreDurability::AofEverysec,
            graphql_default_surface: true,
        }
    }
}

/// The on-disk `theorem.toml` shape: every field optional, so a partial (or
/// empty) file just overlays the defaults.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlConfig {
    tenant: Option<String>,
    durability: Option<String>,
    graphql_default_surface: Option<bool>,
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
    /// or malformed TOML is a clean `Err` (never a panic). A non-empty tenant is
    /// enforced (the GraphQL surface rejects an empty tenant).
    pub fn from_toml_str(body: &str) -> Result<Self, EngineError> {
        let raw: TomlConfig =
            toml::from_str(body).map_err(|error| EngineError::Config(error.to_string()))?;
        let mut config = EmbeddedConfig::default();
        if let Some(tenant) = raw.tenant {
            if tenant.trim().is_empty() {
                return Err(EngineError::Config("tenant must be non-empty".to_string()));
            }
            config.tenant = tenant;
        }
        if let Some(durability) = raw.durability {
            config.durability = parse_durability(&durability)?;
        }
        if let Some(flag) = raw.graphql_default_surface {
            config.graphql_default_surface = flag;
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

/// Errors surfaced by the embedded engine.
#[derive(Debug)]
pub enum EngineError {
    /// Opening / recovering the local store failed.
    Open(String),
    /// The GraphQL document returned an execution error, or the transport failed.
    Graphql(String),
    /// Loading / parsing the local `theorem.toml` config failed.
    Config(String),
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EngineError::Open(message) => write!(f, "open: {message}"),
            EngineError::Graphql(message) => write!(f, "graphql: {message}"),
            EngineError::Config(message) => write!(f, "config: {message}"),
        }
    }
}

impl std::error::Error for EngineError {}

/// The embedded engine: a durable, in-process RustyRed over a local directory.
pub struct Engine {
    store: SharedStore<RedCoreGraphStore>,
    config: McpServerConfig,
    tenant: String,
    // E0.5 folder tier: the CoW path-keyed document tree (B3's DocTree) is the
    // agent's working filesystem. Persisted as a JSON snapshot beside the store;
    // overflow bodies live in the disk object store. Behind a RefCell so the
    // folder API matches query/mutate's `&self` shape.
    doc_tree: RefCell<DocTree>,
    object_store: DiskObjectStore,
    doc_tree_path: PathBuf,
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
        let mcp_config = McpServerConfig {
            default_tenant: config.tenant.clone(),
            read_only: false,
            graphql_default_surface: config.graphql_default_surface,
            // Embedded callers want the full payload inline, never a fetch handle.
            tool_result_budget_bytes: 0,
            ..McpServerConfig::default()
        };
        let engine = Self {
            store: SharedStore::new(store),
            config: mcp_config,
            tenant: config.tenant,
            doc_tree: RefCell::new(doc_tree),
            object_store,
            doc_tree_path,
        };
        // E0.5 embedding: designate the file-embedding vector property so written
        // files are semantically searchable. Best-effort + idempotent across
        // restarts (re-designating an existing property is tolerated).
        let _ = engine.mutate(
            "mutation{ designateVector(label: \"File\", property: \"embedding\", dimension: 16) }",
            json!({}),
        );
        Ok(engine)
    }

    /// The single local tenant this engine is scoped to.
    pub fn tenant(&self) -> &str {
        &self.tenant
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
        let response = self.call("graphql_introspect", json!({ "tenant": self.tenant }));
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
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        let entry = self
            .doc_tree
            .borrow_mut()
            .put_body(
                key,
                content,
                ColdTierKind::Cold,
                now_ms(),
                None,
                &self.object_store,
            )
            .map_err(|error| EngineError::Open(error.message))?;
        let hash = entry.content_hash.clone().unwrap_or_default();

        let mut properties = serde_json::Map::new();
        properties.insert(DOC_TREE_PATH_PROPERTY.to_string(), json!(path));
        properties.insert(DOC_TREE_CONTENT_HASH_PROPERTY.to_string(), json!(hash));
        // E0.5: the file also gains a (deterministic, offline) embedding, so it is
        // semantically searchable via vectorSearch over the designated property.
        properties.insert(
            "embedding".to_string(),
            json!(hash_embedding(content, FILE_EMBEDDING_DIM)),
        );
        let node = json!({
            "id": format!("file:{path}"),
            "labels": ["File"],
            "properties": properties
        });
        self.mutate(
            "mutation($n:JSON!){ bulkNodes(nodes:$n){ inserted } }",
            json!({ "n": [node] }),
        )?;
        self.persist_doc_tree()?;
        Ok(hash)
    }

    /// Read a file's body from the folder tree (point lookup).
    pub fn fs_read(&self, path: &str) -> Result<Option<Vec<u8>>, EngineError> {
        let key =
            PathKey::from_slash_path(path).map_err(|error| EngineError::Open(error.message))?;
        self.doc_tree
            .borrow()
            .resolve_body(&key, &self.object_store)
            .map_err(|error| EngineError::Open(error.message))
    }

    /// List the paths under a prefix -- `ls` / `tree`, a prefix range scan over
    /// the document namespace.
    pub fn fs_ls(&self, prefix: &str) -> Result<Vec<String>, EngineError> {
        let prefix_key = PathKey::prefix_from_segments(prefix.split('/').filter(|s| !s.is_empty()))
            .map_err(|error| EngineError::Open(error.message))?;
        let tree = self.doc_tree.borrow();
        Ok(tree
            .range_prefix(prefix_key.as_bytes())
            .map(|(key, _)| key.to_slash_path())
            .collect())
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
            .map_err(|error| EngineError::Open(format!("writing doc tree: {error}")))
    }

    fn run(&self, tool: &str, gql: &str, variables: Option<Value>) -> Result<Value, EngineError> {
        let mut arguments = json!({ "tenant": self.tenant, "query": gql });
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

/// Dimension of the deterministic offline file embedding (E0.5). A real encoder
/// is the quality swap; this hash embedding makes a written file semantically
/// searchable with no model dependency, and is deterministic so identical
/// content maps to an identical unit vector (cosine 1.0).
const FILE_EMBEDDING_DIM: usize = 16;

fn hash_embedding(content: &[u8], dim: usize) -> Vec<f32> {
    let mut vector = vec![0.0f32; dim];
    for (index, &byte) in content.iter().enumerate() {
        let bucket = (index.wrapping_add(byte as usize)) % dim;
        vector[bucket] += byte as f32 + 1.0;
    }
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in vector.iter_mut() {
            *x /= norm;
        }
    } else {
        vector[0] = 1.0;
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::*;
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

    // E0.4: theorem.toml overlays the defaults; empty file -> default; an invalid
    // durability or an empty tenant is a clean Err, never a panic.
    #[test]
    fn embedded_config_parses_toml() {
        let cfg = EmbeddedConfig::from_toml_str(
            "tenant = \"acme\"\ndurability = \"always\"\ngraphql_default_surface = false\n",
        )
        .expect("parse toml");
        assert_eq!(cfg.tenant, "acme");
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
        assert_eq!(cfg.tenant, default.tenant);
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
    fn embedded_config_empty_tenant_rejected() {
        let err = EmbeddedConfig::from_toml_str("tenant = \"\"\n");
        assert!(
            matches!(err, Err(EngineError::Config(_))),
            "empty tenant must be rejected: {err:?}"
        );
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

        let query_vec = hash_embedding(b"hello embedded world", FILE_EMBEDDING_DIM);
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
}
