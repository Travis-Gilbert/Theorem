//! AM4/AM5: the ambient code-KG session layer.
//!
//! `session_reingest` (AM4 / IK-3) overlays the local uncommitted edits (inline
//! file text) on the committed code-graph base as a cached `SessionDelta`.
//! `context_pack` (AM5) composes the one ready-to-inject markdown block the
//! UserPromptSubmit hook needs: PPR over the merged base+delta from the
//! dirty/footprint/prompt seeds, hydrated hits with a one-line edge-path "why",
//! and an impact block from the dirty symbols. Both are server-side so the hook
//! scripts stay dumb shell (the mediate-don't-separate rule).

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use rustyred_thg_core::{CodeKgManifest, Direction, HarnessInstantKg, NodeRecord, SessionDelta};
use serde_json::{json, Value};

use crate::code_index::{
    collect_code_files, resolve_ingest_config, CodeIndexError, CodeIndexRuntime, IndexedFile,
    IngestCodebaseInput,
};
use crate::session_delta::build_session_delta_with_base;

/// `CodeIndexError::{invalid,io}` are private to rustyred-thg-code; build the
/// public-field struct directly here with the same error codes the gRPC layer
/// maps (`invalid_code_index_request` -> invalid_argument, `code_index_io_error`
/// -> failed_precondition).
fn invalid_input(message: impl Into<String>) -> CodeIndexError {
    CodeIndexError {
        code: "invalid_code_index_request".to_string(),
        message: message.into(),
    }
}

fn io_error(action: &str, path: &Path, error: impl std::fmt::Display) -> CodeIndexError {
    CodeIndexError {
        code: "code_index_io_error".to_string(),
        message: format!("{action} {}: {error}", path.display()),
    }
}

const SESSION_DELTA_TTL_MS: u64 = 30 * 60 * 1000; // 30 minutes
const DEFAULT_TOP_K: usize = 20;
const MAX_TOP_K: usize = 50;
const DEFAULT_BUDGET_TOKENS: usize = 2000;
const PPR_ALPHA: f64 = 0.15;
const PPR_EPSILON: f64 = 1e-4;
const PPR_MAX_PUSHES: usize = 200_000;
const CODE_SYMBOL_LABEL: &str = "CodeSymbol";

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Clone)]
struct CachedDelta {
    delta: SessionDelta,
    expires_at_ms: u64,
}

/// Server-side cache of session deltas keyed by `(tenant, repo_id, session_id)`,
/// so the hook ships file text once per flush and `context_pack` references it.
/// TTL-evicted on read; cloned per service but the inner map is shared.
#[derive(Clone, Default)]
pub struct SessionKgCache {
    inner: Arc<Mutex<HashMap<(String, String, String), CachedDelta>>>,
}

impl SessionKgCache {
    pub fn new() -> Self {
        Self::default()
    }

    fn put(&self, key: (String, String, String), delta: SessionDelta) {
        if let Ok(mut map) = self.inner.lock() {
            map.insert(
                key,
                CachedDelta {
                    delta,
                    expires_at_ms: now_ms() + SESSION_DELTA_TTL_MS,
                },
            );
        }
    }

    fn get(&self, key: &(String, String, String)) -> Option<SessionDelta> {
        let now = now_ms();
        let mut map = self.inner.lock().ok()?;
        match map.get(key) {
            Some(cached) if cached.expires_at_ms > now => Some(cached.delta.clone()),
            Some(_) => {
                map.remove(key);
                None
            }
            None => None,
        }
    }
}

fn cache_key(tenant: &str, repo_id: &str, session_id: &str) -> (String, String, String) {
    (
        tenant.to_string(),
        repo_id.to_string(),
        session_id.to_string(),
    )
}

/// Repo-relative, traversal-safe path. The hook sends `git status` paths (already
/// repo-relative); this strips any leading `/`/`./` and drops `..` for safety.
fn sanitize_rel_path(path: &str) -> PathBuf {
    let trimmed = path.trim().trim_start_matches("./").trim_start_matches('/');
    let mut out = PathBuf::new();
    for comp in Path::new(trimmed).components() {
        if let Component::Normal(part) = comp {
            out.push(part);
        }
    }
    out
}

fn rel_path_string(path: &str) -> String {
    sanitize_rel_path(path).display().to_string()
}

/// A throwaway checkout of the inline edited files. The disk parser produces
/// IndexedFiles with the same `file_id`/`symbol_id` as the original ingest
/// (ids key on repo_id + relative path, not repo_root), so the session delta
/// diffs correctly against the committed base. Removed on drop.
struct TempCheckout {
    root: PathBuf,
}

impl TempCheckout {
    fn materialize(files: &[(String, String)]) -> Result<Self, CodeIndexError> {
        let root = std::env::temp_dir().join(format!(
            "theorem-session-reingest-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root)
            .map_err(|err| io_error("create session work dir", &root, err))?;
        for (path, text) in files {
            let rel = sanitize_rel_path(path);
            if rel.as_os_str().is_empty() {
                continue;
            }
            let dest = root.join(&rel);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|err| io_error("create session subdir", parent, err))?;
            }
            std::fs::write(&dest, text)
                .map_err(|err| io_error("write session file", &dest, err))?;
        }
        Ok(Self { root })
    }

    fn root_display(&self) -> String {
        self.root.display().to_string()
    }
}

impl Drop for TempCheckout {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// --- AM4: session_reingest ---

pub struct SessionReingestInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub session_id: String,
    pub base_commit_sha: String,
    /// (relative path, file text) pairs. Inline only; the server never reads a
    /// client filesystem.
    pub files: Vec<(String, String)>,
}

/// AM4 (IK-3): parse the inline edited files, diff against the committed
/// code-graph base (`build_session_delta_with_base`: additions + tombstones +
/// removed edges), cache the delta keyed `(tenant, repo_id, session_id)`, and
/// return delta counts.
pub fn session_reingest(
    code_index: &CodeIndexRuntime,
    cache: &SessionKgCache,
    input: SessionReingestInput,
) -> Result<Value, CodeIndexError> {
    let tenant = input.tenant_id.trim().to_string();
    let repo_id = input.repo_id.trim().to_string();
    let session_id = input.session_id.trim().to_string();
    if tenant.is_empty() || repo_id.is_empty() || session_id.is_empty() {
        return Err(invalid_input(
            "session_reingest requires tenant_id, repo_id, and session_id".to_string(),
        ));
    }

    let base = code_index.code_graph_snapshot(&tenant, &repo_id)?;

    let work = TempCheckout::materialize(&input.files)?;
    let config = resolve_ingest_config(IngestCodebaseInput {
        tenant_id: tenant.clone(),
        repo_path: work.root_display(),
        repo_id: repo_id.clone(),
        ..Default::default()
    })?;
    let mut files: Vec<IndexedFile> = Vec::new();
    let mut skipped = 0u64;
    collect_code_files(&config.repo_root, &config, &mut files, &mut skipped)?;

    let commit_sha = if input.base_commit_sha.trim().is_empty() {
        None
    } else {
        Some(input.base_commit_sha.trim().to_string())
    };
    let delta = build_session_delta_with_base(&config, &files, &base, commit_sha);

    let counts = json!({
        "tenant_id": tenant,
        "repo_id": repo_id,
        "session_id": session_id,
        "delta_objects": delta.objects.len(),
        "delta_edges": delta.edges.len(),
        "tombstoned_objects": delta.tombstoned_object_ids.len(),
        "removed_edges": delta.removed_edge_ids.len(),
        "changed_files": delta.changed_files.len(),
        "files_parsed": files.len(),
        "files_skipped": skipped,
        "ttl_ms": SESSION_DELTA_TTL_MS,
    });
    cache.put(cache_key(&tenant, &repo_id, &session_id), delta);
    Ok(counts)
}

// --- AM5: context_pack ---

pub struct ContextPackInput {
    pub tenant_id: String,
    pub repo_id: String,
    pub session_id: String,
    pub dirty_files: Vec<String>,
    pub footprint_files: Vec<String>,
    pub prompt_text: String,
    pub top_k: usize,
    pub budget_tokens: usize,
}

fn is_symbol(node: &NodeRecord) -> bool {
    node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL)
}

fn prop_str<'a>(node: &'a NodeRecord, key: &str) -> &'a str {
    node.properties
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
}

fn node_name(node: &NodeRecord) -> String {
    let name = prop_str(node, "name");
    if !name.is_empty() {
        return name.to_string();
    }
    let path = prop_str(node, "path");
    if !path.is_empty() {
        return path.to_string();
    }
    node.id.clone()
}

fn file_line(node: &NodeRecord) -> String {
    let path = if !prop_str(node, "file_path").is_empty() {
        prop_str(node, "file_path")
    } else {
        prop_str(node, "path")
    };
    let line = node
        .properties
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if line > 0 {
        format!("{path}:{line}")
    } else {
        path.to_string()
    }
}

/// Lowercased, de-noised identifier tokens from the prompt for lexical symbol
/// resolution. Server-side only -- no model call in the hook path, ever.
fn prompt_tokens(text: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut current = String::new();
    let flush = |current: &mut String, seen: &mut HashSet<String>, out: &mut Vec<String>| {
        if current.len() >= 3 {
            let token = current.clone();
            if seen.insert(token.clone()) {
                out.push(token);
            }
        }
        current.clear();
    };
    for ch in text.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            current.push(ch);
        } else {
            flush(&mut current, &mut seen, &mut out);
        }
    }
    flush(&mut current, &mut seen, &mut out);
    out.truncate(64);
    out
}

fn approx_tokens(text: &str) -> usize {
    // ~4 chars per token is the usual rough estimate.
    (text.len() / 4) + 1
}

/// AM5: the composed verb for the UserPromptSubmit hook. Resolves seeds, runs
/// PPR over the merged base+delta, hydrates the top hits with a one-line
/// edge-path "why", computes the impact block, and returns `{ markdown,
/// node_ids, seed_report, latency_ms }` trimmed to the token budget.
pub fn context_pack(
    code_index: &CodeIndexRuntime,
    cache: &SessionKgCache,
    input: ContextPackInput,
) -> Result<Value, CodeIndexError> {
    let started = Instant::now();
    let tenant = input.tenant_id.trim().to_string();
    let repo_id = input.repo_id.trim().to_string();
    if tenant.is_empty() || repo_id.is_empty() {
        return Err(invalid_input(
            "context_pack requires tenant_id and repo_id".to_string(),
        ));
    }
    let top_k = if input.top_k == 0 {
        DEFAULT_TOP_K
    } else {
        input.top_k.min(MAX_TOP_K)
    };
    let budget_tokens = if input.budget_tokens == 0 {
        DEFAULT_BUDGET_TOKENS
    } else {
        input.budget_tokens
    };

    let base = code_index.code_graph_snapshot(&tenant, &repo_id)?;
    let base_objects = base.nodes.len();
    let session_id = input.session_id.trim();
    let delta = cache
        .get(&cache_key(&tenant, &repo_id, session_id))
        .unwrap_or_default();
    let delta_objects = delta.objects.len();
    let manifest = CodeKgManifest::from_base_snapshot(&repo_id, "", &base);
    let kg = HarnessInstantKg::new(base, Some(manifest), delta);
    let merged = kg.merged_snapshot();
    let name_by_id: HashMap<String, String> = merged
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node_name(node)))
        .collect();

    // --- Seeds: dirty-file symbols, footprint symbols, prompt-token matches ---
    let dirty_set: HashSet<String> = input
        .dirty_files
        .iter()
        .map(|p| rel_path_string(p))
        .filter(|p| !p.is_empty())
        .collect();
    let footprint_set: HashSet<String> = input
        .footprint_files
        .iter()
        .map(|p| rel_path_string(p))
        .filter(|p| !p.is_empty())
        .collect();

    let mut seeds: HashMap<String, f64> = HashMap::new();
    let mut dirty_symbol_ids: Vec<String> = Vec::new();
    let mut dirty_count = 0usize;
    let mut footprint_count = 0usize;
    for node in &merged.nodes {
        if !is_symbol(node) {
            continue;
        }
        let file_path = if !prop_str(node, "file_path").is_empty() {
            rel_path_string(prop_str(node, "file_path"))
        } else {
            rel_path_string(prop_str(node, "path"))
        };
        if dirty_set.contains(&file_path) {
            seeds.insert(node.id.clone(), 1.0);
            dirty_symbol_ids.push(node.id.clone());
            dirty_count += 1;
        } else if footprint_set.contains(&file_path) {
            seeds.insert(node.id.clone(), 0.8);
            footprint_count += 1;
        }
    }
    let mut prompt_matches = 0usize;
    for token in prompt_tokens(&input.prompt_text) {
        if let Some(id) = kg.resolve_symbol_name(&token) {
            seeds.entry(id).or_insert(0.6);
            prompt_matches += 1;
        }
    }
    let seed_report = json!({
        "dirty_file_symbols": dirty_count,
        "footprint_symbols": footprint_count,
        "prompt_matches": prompt_matches,
        "total_seeds": seeds.len(),
        "base_objects": base_objects,
        "delta_objects": delta_objects,
    });

    // --- PPR over the merged base+delta ---
    let ranked = if seeds.is_empty() {
        Vec::new()
    } else {
        kg.ppr(&seeds, PPR_ALPHA, PPR_EPSILON, PPR_MAX_PUSHES, top_k)
    };

    // --- Assemble markdown under the token budget ---
    let seed_ids: HashSet<String> = seeds.keys().cloned().collect();
    let mut node_ids: Vec<String> = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut used_tokens = 0usize;
    let base_building = base_objects == 0;

    if base_building {
        lines.push(
            "_Code KG base is still building (0 indexed objects); lexical fallback only._"
                .to_string(),
        );
    }

    // Impact block first: the accuracy mechanism (load-bearing callers before edits).
    let impact_lines = impact_block(&kg, &dirty_symbol_ids, &name_by_id);
    if !impact_lines.is_empty() {
        lines.push("**Impact (editing the dirty symbols reaches):**".to_string());
        for line in impact_lines {
            if used_tokens + approx_tokens(&line) > budget_tokens {
                break;
            }
            used_tokens += approx_tokens(&line);
            lines.push(line);
        }
        lines.push(String::new());
    }

    if !ranked.is_empty() {
        lines.push("**Nearest code (PPR-ranked):**".to_string());
    }
    for hit in &ranked {
        let Some(node) = &hit.object else { continue };
        if !is_symbol(node) {
            continue;
        }
        let name = node_name(node);
        let loc = file_line(node);
        let sig = prop_str(node, "signature");
        let why = edge_why(&kg, &hit.object_id, &seed_ids, &name_by_id);
        let mut entry = format!("- `{name}` ({loc})");
        if !sig.is_empty() {
            entry.push_str(&format!(" -- {}", truncate(sig, 160)));
        }
        if let Some(why) = why {
            entry.push_str(&format!(" [{why}]"));
        }
        if used_tokens + approx_tokens(&entry) > budget_tokens {
            break;
        }
        used_tokens += approx_tokens(&entry);
        node_ids.push(hit.object_id.clone());
        lines.push(entry);
    }

    if lines.is_empty() {
        lines.push("_No code neighborhood resolved for this prompt._".to_string());
    }

    let markdown = lines.join("\n");
    Ok(json!({
        "markdown": markdown,
        "node_ids": node_ids,
        "seed_report": seed_report,
        "base_building": base_building,
        "latency_ms": started.elapsed().as_millis() as u64,
    }))
}

/// One impact line per dirty symbol: who is reached by editing it (incoming
/// callers/dependents up to one hop).
fn impact_block(
    kg: &HarnessInstantKg,
    dirty_symbol_ids: &[String],
    name_by_id: &HashMap<String, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for sym in dirty_symbol_ids.iter().take(8) {
        let reached = kg.impact(sym, Direction::In, 1);
        if reached.is_empty() {
            continue;
        }
        let from_name = name_by_id.get(sym).cloned().unwrap_or_else(|| sym.clone());
        let names: Vec<String> = reached
            .iter()
            .filter_map(|hit| hit.object.as_ref())
            .map(node_name)
            .take(6)
            .collect();
        if names.is_empty() {
            continue;
        }
        out.push(format!(
            "- editing `{from_name}` reaches: {}",
            names.join(", ")
        ));
    }
    out
}

/// A one-line "why" for a hit: the first 1-hop edge connecting it to a seed.
fn edge_why(
    kg: &HarnessInstantKg,
    hit_id: &str,
    seed_ids: &HashSet<String>,
    name_by_id: &HashMap<String, String>,
) -> Option<String> {
    if seed_ids.contains(hit_id) {
        return Some("seed".to_string());
    }
    // Outgoing edges from the hit that land on a seed.
    for edge in kg.get_edges_from(hit_id) {
        if seed_ids.contains(&edge.to_id) {
            let target = name_by_id
                .get(&edge.to_id)
                .cloned()
                .unwrap_or_else(|| edge.to_id.clone());
            return Some(format!("{} -> {target}", edge.edge_type));
        }
    }
    // Incoming: a seed has an edge to the hit.
    for seed in seed_ids {
        for edge in kg.get_edges_from(seed) {
            if edge.to_id == hit_id {
                let src = name_by_id
                    .get(seed)
                    .cloned()
                    .unwrap_or_else(|| seed.clone());
                return Some(format!("{src} {} this", edge.edge_type));
            }
        }
    }
    None
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max).collect();
        format!("{truncated}...")
    }
}
