//! SHA-keyed idempotent repo knowledge-graph entry (SPEC-CONTEXT-MEMBRANE-1.0,
//! code arm, acceptance #6).
//!
//! Entering a repo is not the same as ingesting it. If the code graph already
//! holds this repo at the requested commit, re-entering must LOAD THE SNAPSHOT,
//! not re-walk and re-parse the tree. [`ensure_repo_kg`] makes that path the
//! default and observable: it reads the `CodeRepository` node's stamped
//! `head_sha`, compares it to the target sha, and:
//!
//! - sha already present in the graph  -> [`RepoKgStatus::LoadedFromSnapshot`]
//!   (NO clone, NO ingest; the warm centrality / embeddings stay as the hooks
//!   left them);
//! - repo known at a different sha     -> reindex only the diff via
//!   [`CodeIndexRuntime::reindex_codebase_from_url`] ->
//!   [`RepoKgStatus::IncrementallyIngested`] (`files_changed` = files re-parsed
//!   this pass; the `IncrementalCentralityHook` re-warms the touched
//!   neighborhood on the resulting mutations, so we do not recompute it here);
//! - repo unknown                      -> full ingest via
//!   [`CodeIndexRuntime::ingest_codebase_from_url`] ->
//!   [`RepoKgStatus::FullyIngested`].
//!
//! After any real ingest/reindex the resolved `head_sha`/`repo_url` are stamped
//! onto the `CodeRepository` node, so the *next* entry at the same sha takes the
//! snapshot path. The default `git clone --depth 1` keeps a `.git`, so the
//! checkout's HEAD is resolvable when the caller does not pass a sha.

use std::path::Path;

use rustyred_thg_core::{NodeQuery, RedCoreGraphStore};
use serde_json::{json, Value};

use crate::repo_fetch::{fetch_repo, RepoFetchCaps};
use crate::{
    code_graph_snapshot_in_store, normalize_tenant, property_string, CodeIndexError,
    CodeIndexRuntime, IngestCodebaseInput, CODE_REPO_LABEL, CODE_SYMBOL_LABEL,
};

/// Graph-node property holding the commit the repo's current generation was
/// ingested from. Read to decide the snapshot-vs-ingest entry path.
pub const HEAD_SHA_PROPERTY: &str = "head_sha";
/// Graph-node property holding the URL the repo was ingested from.
pub const REPO_URL_PROPERTY: &str = "repo_url";

/// How [`ensure_repo_kg`] resolved an entry into a repo's code knowledge graph.
/// The discriminant makes the snapshot-hit path observable (acceptance #6): a
/// `LoadedFromSnapshot` means no clone and no parse happened.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RepoKgStatus {
    /// The graph already held this repo at `sha`; the snapshot was loaded and no
    /// ingest ran.
    LoadedFromSnapshot { sha: String },
    /// The repo was known at a prior sha; only the diff was reindexed.
    /// `files_changed` is the number of files re-parsed this pass.
    IncrementallyIngested { sha: String, files_changed: usize },
    /// The repo was unknown; a full ingest ran.
    FullyIngested { sha: String },
}

impl RepoKgStatus {
    /// The commit the repo's current generation is at, regardless of how it was
    /// reached.
    pub fn sha(&self) -> &str {
        match self {
            RepoKgStatus::LoadedFromSnapshot { sha }
            | RepoKgStatus::IncrementallyIngested { sha, .. }
            | RepoKgStatus::FullyIngested { sha } => sha,
        }
    }

    /// True only for the snapshot-load path: no clone, no parse, no commit. This
    /// is the observable that proves re-entry at the current sha is cheap.
    pub fn loaded_from_snapshot(&self) -> bool {
        matches!(self, RepoKgStatus::LoadedFromSnapshot { .. })
    }

    /// Number of files re-parsed this entry. Zero for a snapshot load.
    pub fn files_changed(&self) -> usize {
        match self {
            RepoKgStatus::IncrementallyIngested { files_changed, .. } => *files_changed,
            _ => 0,
        }
    }

    /// A short, stable discriminant for receipts/metadata.
    pub fn mode(&self) -> &'static str {
        match self {
            RepoKgStatus::LoadedFromSnapshot { .. } => "loaded_from_snapshot",
            RepoKgStatus::IncrementallyIngested { .. } => "incrementally_ingested",
            RepoKgStatus::FullyIngested { .. } => "fully_ingested",
        }
    }

    pub fn to_json(&self) -> Value {
        json!({
            "mode": self.mode(),
            "sha": self.sha(),
            "files_changed": self.files_changed(),
            "loaded_from_snapshot": self.loaded_from_snapshot(),
        })
    }
}

/// Best-effort stable repo id from a clone URL: `repo:<last-path-segment>`.
/// Mirrors the slug the ingest path derives so an `ensure` lookup matches the
/// repo node a prior ingest wrote.
pub(crate) fn repo_id_from_url(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let trimmed = trimmed.strip_suffix(".git").unwrap_or(trimmed);
    let slug = trimmed.rsplit(['/', ':']).next().filter(|s| !s.is_empty());
    format!("repo:{}", slug.unwrap_or("repo"))
}

/// Read the stamped `head_sha` for a repo's current generation, plus whether the
/// repo carries any current-generation `CodeSymbol` nodes (a stamped-but-empty
/// repo node is not a usable snapshot).
fn snapshot_state(
    store: &RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
) -> (Option<String>, bool) {
    let base = code_graph_snapshot_in_store(store, tenant_id, repo_id);
    let head_sha = base
        .nodes
        .iter()
        .find(|node| node.labels.iter().any(|label| label == CODE_REPO_LABEL))
        .and_then(|node| property_string(&node.properties, HEAD_SHA_PROPERTY))
        .filter(|sha| !sha.trim().is_empty());
    let has_symbols = base
        .nodes
        .iter()
        .any(|node| node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL));
    (head_sha, has_symbols)
}

/// True when a `CodeRepository` node exists for this repo at all (any
/// generation), so re-entry should reindex the diff rather than full-ingest.
fn repo_known(store: &RedCoreGraphStore, tenant_id: &str, repo_id: &str) -> bool {
    let tenant = normalize_tenant(tenant_id);
    let repo_id = repo_id.trim();
    store
        .query_nodes(NodeQuery::label(CODE_REPO_LABEL).with_limit(10_000))
        .map(|nodes| {
            nodes.iter().any(|node| {
                node.properties.get("tenant_id").and_then(Value::as_str) == Some(tenant.as_str())
                    && node.properties.get("repo_id").and_then(Value::as_str) == Some(repo_id)
            })
        })
        .unwrap_or(false)
}

/// Resolve the target commit. If the caller supplied one, trust it (and avoid a
/// clone). Otherwise shallow-clone and `git rev-parse HEAD` so the entry path is
/// keyed on the real commit rather than an empty string.
fn resolve_target_sha(
    url: &str,
    sha: Option<&str>,
    caps: &RepoFetchCaps,
) -> Result<String, CodeIndexError> {
    if let Some(sha) = sha {
        let sha = sha.trim();
        if !sha.is_empty() {
            return Ok(sha.to_string());
        }
    }
    let fetched = fetch_repo(url, caps).map_err(|err| CodeIndexError::invalid(err.to_string()))?;
    Ok(git_head_sha(fetched.path()))
}

/// Best-effort `git rev-parse HEAD` in `path`. Empty string when the directory
/// is not a git checkout or git is unavailable.
fn git_head_sha(path: &Path) -> String {
    std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Stamp the resolved commit + url onto the repo's `CodeRepository` node so the
/// next entry at this sha takes the snapshot path. Idempotent: re-stamping the
/// same values is a no-op upsert.
fn stamp_head_sha(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    repo_id: &str,
    repo_url: &str,
    sha: &str,
) -> Result<(), CodeIndexError> {
    if sha.trim().is_empty() {
        return Ok(());
    }
    let tenant = normalize_tenant(tenant_id);
    let repo_id = repo_id.trim();
    let Some(mut node) = store
        .get_node(repo_id)
        .map_err(CodeIndexError::from_store)?
    else {
        return Ok(());
    };
    if !node.labels.iter().any(|label| label == CODE_REPO_LABEL) {
        return Ok(());
    }
    if node.properties.get("tenant_id").and_then(Value::as_str) != Some(tenant.as_str()) {
        return Ok(());
    }
    let already = property_string(&node.properties, HEAD_SHA_PROPERTY).as_deref() == Some(sha)
        && property_string(&node.properties, REPO_URL_PROPERTY).as_deref() == Some(repo_url);
    if already {
        return Ok(());
    }
    match node.properties.as_object_mut() {
        Some(map) => {
            map.insert(HEAD_SHA_PROPERTY.to_string(), json!(sha));
            if !repo_url.trim().is_empty() {
                map.insert(REPO_URL_PROPERTY.to_string(), json!(repo_url));
            }
        }
        None => {
            node.properties = json!({ HEAD_SHA_PROPERTY: sha, REPO_URL_PROPERTY: repo_url });
        }
    }
    store
        .upsert_node(node)
        .map_err(CodeIndexError::from_store)?;
    Ok(())
}

/// SHA-keyed idempotent entry into a repo's code knowledge graph, store-level.
///
/// `caps` bound any clone this path needs (sha resolution and/or ingest). When
/// `sha` is supplied and already present in the graph, no clone or ingest runs.
pub fn ensure_repo_kg_in_store(
    store: &mut RedCoreGraphStore,
    tenant_id: &str,
    repo_url: &str,
    sha: Option<&str>,
    repo_id: Option<&str>,
    caps: &RepoFetchCaps,
) -> Result<RepoKgStatus, CodeIndexError> {
    let tenant = normalize_tenant(tenant_id);
    let repo_url = repo_url.trim();
    if repo_url.is_empty() {
        return Err(CodeIndexError::invalid(
            "ensure_repo_kg requires a repo_url",
        ));
    }
    let repo_id = repo_id
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| repo_id_from_url(repo_url));

    // 1. Cheap snapshot-hit check FIRST when the caller gave us a sha: if the
    //    graph already holds this repo at this commit we must not clone or parse.
    if let Some(sha) = sha.map(str::trim).filter(|s| !s.is_empty()) {
        let (stored_sha, has_symbols) = snapshot_state(store, &tenant, &repo_id);
        if has_symbols && stored_sha.as_deref() == Some(sha) {
            return Ok(RepoKgStatus::LoadedFromSnapshot {
                sha: sha.to_string(),
            });
        }
    }

    // 2. Resolve the real target commit (clones only if no sha was supplied).
    let target_sha = resolve_target_sha(repo_url, sha, caps)?;

    // 3. Re-check the snapshot now that the sha is resolved (covers the
    //    sha-omitted caller whose checkout HEAD already matches the graph).
    let (stored_sha, has_symbols) = snapshot_state(store, &tenant, &repo_id);
    if has_symbols && !target_sha.is_empty() && stored_sha.as_deref() == Some(target_sha.as_str()) {
        return Ok(RepoKgStatus::LoadedFromSnapshot { sha: target_sha });
    }

    // 4. Known repo at a different sha -> reindex the diff. Unknown -> full ingest.
    let input = IngestCodebaseInput {
        tenant_id: tenant.clone(),
        repo_id: repo_id.clone(),
        ..IngestCodebaseInput::default()
    };
    let known = repo_known(store, &tenant, &repo_id);
    let status = if known {
        let out = crate::reindex_codebase_from_url_in_store(store, repo_url, input, caps)?;
        RepoKgStatus::IncrementallyIngested {
            sha: target_sha.clone(),
            files_changed: out.files_parsed as usize,
        }
    } else {
        crate::ingest_codebase_from_url_in_store(store, repo_url, input, caps)?;
        RepoKgStatus::FullyIngested {
            sha: target_sha.clone(),
        }
    };

    // 5. Stamp the resolved commit so the next entry at this sha loads the
    //    snapshot instead of re-ingesting.
    stamp_head_sha(store, &tenant, &repo_id, repo_url, &target_sha)?;
    Ok(status)
}

/// SHA-keyed idempotent entry into a repo's code knowledge graph over a running
/// [`CodeIndexRuntime`]. The hooked runtime's `IncrementalCentralityHook`
/// observes the resulting mutations, so warm centrality is refreshed without a
/// separate recompute here.
pub fn ensure_repo_kg(
    runtime: &CodeIndexRuntime,
    tenant_id: &str,
    repo_url: &str,
    sha: Option<&str>,
    repo_id: Option<&str>,
    caps: &RepoFetchCaps,
) -> Result<RepoKgStatus, CodeIndexError> {
    let mut store = runtime.lock_store()?;
    ensure_repo_kg_in_store(&mut store, tenant_id, repo_url, sha, repo_id, caps)
}

/// Spec-named convenience wrapper for the MCP surface:
/// `code_ingest_ensure(repo_url, sha?)`.
///
/// The caller supplies tenant scope; the repo id is derived from the URL and the
/// clone budget uses the default repo-fetch caps.
pub fn code_ingest_ensure(
    runtime: &CodeIndexRuntime,
    tenant_id: &str,
    repo_url: &str,
    sha: Option<&str>,
) -> Result<RepoKgStatus, CodeIndexError> {
    ensure_repo_kg(
        runtime,
        tenant_id,
        repo_url,
        sha,
        None,
        &RepoFetchCaps::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingest_codebase_from_url_in_store;
    use rustyred_thg_core::RedCoreGraphStore;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "rustyred-ensure-test-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    fn write_fixture_repo(dir: &Path) {
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(
            dir.join("src/lib.rs"),
            "pub fn helper_len(query: &str) -> usize {\n    query.len()\n}\n\npub fn search_code(query: &str) -> usize {\n    helper_len(query)\n}\n",
        )
        .unwrap();
    }

    fn init_git(dir: &Path) -> bool {
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

    fn head_sha(dir: &Path) -> String {
        git_head_sha(dir)
    }

    #[test]
    fn repo_id_from_url_slugs_last_segment() {
        assert_eq!(repo_id_from_url("https://x/y/widget.git"), "repo:widget");
        assert_eq!(repo_id_from_url("git@github.com:org/Repo"), "repo:Repo");
    }

    #[test]
    fn entering_at_current_sha_loads_snapshot_without_reingest() {
        let repo_dir = unique_dir("repo");
        write_fixture_repo(&repo_dir);
        if !init_git(&repo_dir) {
            fs::remove_dir_all(&repo_dir).ok();
            return; // git unavailable; skip rather than fail.
        }
        let url = format!("file://{}", repo_dir.display());
        let sha = head_sha(&repo_dir);
        assert!(!sha.is_empty());

        let mut store = RedCoreGraphStore::memory();
        // Full ingest the first time, then stamp the sha as ensure would.
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
        let repo_id = out.repo_id.clone();
        stamp_head_sha(&mut store, "theorem", &repo_id, &url, &sha).unwrap();

        let symbols_before = store
            .query_nodes(NodeQuery::label(CODE_SYMBOL_LABEL))
            .unwrap()
            .len();
        assert!(symbols_before > 0);

        // Re-entering at the SAME sha must load the snapshot: no clone, no parse.
        let status = ensure_repo_kg_in_store(
            &mut store,
            "theorem",
            &url,
            Some(&sha),
            Some(&repo_id),
            &RepoFetchCaps::default(),
        )
        .unwrap();
        assert!(
            status.loaded_from_snapshot(),
            "re-entry at current sha must load snapshot, got {status:?}"
        );
        assert_eq!(status.sha(), sha);

        let symbols_after = store
            .query_nodes(NodeQuery::label(CODE_SYMBOL_LABEL))
            .unwrap()
            .len();
        assert_eq!(
            symbols_before, symbols_after,
            "snapshot load must not change the graph"
        );

        fs::remove_dir_all(&repo_dir).ok();
    }

    #[test]
    fn entering_unknown_then_new_sha_full_then_incremental() {
        let repo_dir = unique_dir("repo");
        write_fixture_repo(&repo_dir);
        if !init_git(&repo_dir) {
            fs::remove_dir_all(&repo_dir).ok();
            return;
        }
        let url = format!("file://{}", repo_dir.display());
        let sha1 = head_sha(&repo_dir);

        let mut store = RedCoreGraphStore::memory();
        // First entry: unknown repo -> full ingest.
        let first = ensure_repo_kg_in_store(
            &mut store,
            "theorem",
            &url,
            Some(&sha1),
            None,
            &RepoFetchCaps::default(),
        )
        .unwrap();
        assert!(
            matches!(first, RepoKgStatus::FullyIngested { .. }),
            "first entry must be a full ingest, got {first:?}"
        );
        assert_eq!(first.sha(), sha1);

        // Re-entry at the same sha now loads the snapshot (stamp landed).
        let again = ensure_repo_kg_in_store(
            &mut store,
            "theorem",
            &url,
            Some(&sha1),
            None,
            &RepoFetchCaps::default(),
        )
        .unwrap();
        assert!(again.loaded_from_snapshot(), "got {again:?}");

        // Advance the repo to a new commit and re-enter: known repo, new sha ->
        // incremental reindex.
        fs::write(
            repo_dir.join("src/extra.rs"),
            "pub fn added_symbol() -> usize {\n    7\n}\n",
        )
        .unwrap();
        let run = |args: &[&str]| {
            std::process::Command::new("git")
                .args(args)
                .current_dir(&repo_dir)
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false)
        };
        assert!(run(&["add", "."]));
        assert!(run(&["commit", "--quiet", "-m", "add extra"]));
        let sha2 = head_sha(&repo_dir);
        assert_ne!(sha1, sha2);

        let incremental = ensure_repo_kg_in_store(
            &mut store,
            "theorem",
            &url,
            Some(&sha2),
            None,
            &RepoFetchCaps::default(),
        )
        .unwrap();
        assert!(
            matches!(incremental, RepoKgStatus::IncrementallyIngested { .. }),
            "entry at a new sha on a known repo must reindex, got {incremental:?}"
        );
        assert_eq!(incremental.sha(), sha2);

        fs::remove_dir_all(&repo_dir).ok();
    }
}
