//! IK-0b: the code-edit -> `SessionDelta` converter (the instant-KG overlay).
//!
//! Plan: `docs/plans/instant-kg-code-encoder/`. This is the seam that turns a set
//! of (re)indexed code files into a `rustyred_thg_core::SessionDelta` WITHOUT
//! writing to a store, so a code edit becomes an overlay on a base graph snapshot
//! (served via `HarnessInstantKg`) instead of an unconditional commit.
//!
//! It runs the shared `code_index::build_code_mutations` extraction (IK-0a) and
//! partitions the mutations into the delta's `objects` (node upserts) and `edges`
//! (edge upserts). The mutation set is exactly what the store-writing ingest path
//! produces, so the overlay graph matches a full ingest of the same files.
//!
//! Scope: additions/modifications only. Tombstones (symbols a changed file
//! deleted) and removed edges are computed against a base snapshot in IK-2; until
//! then they stay empty. `HarnessInstantKg::new` overlays these records on the
//! base and drops dangling edges, so an additions-only delta is safe -- it just
//! does not yet retract a symbol that disappeared from a changed file.

use std::collections::{HashMap, HashSet};

use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphSnapshot, HarnessInstantKg, NodeRecord, SessionDelta,
};

use crate::code_index::{build_code_mutations, IndexedFile, IngestConfig};

const CODE_FILE_LABEL: &str = "CodeFile";
const CODE_SYMBOL_LABEL: &str = "CodeSymbol";

/// Build an instant-KG `SessionDelta` from a set of indexed code files. The
/// `commit_sha` (when known, e.g. from `git rev-parse HEAD`) is stamped on the
/// delta for provenance. Wired into a session-reingest path in IK-3.
#[allow(dead_code)]
pub(crate) fn build_session_delta(
    config: &IngestConfig,
    files: &[IndexedFile],
    commit_sha: Option<String>,
) -> SessionDelta {
    let mutations = build_code_mutations(config, files);

    let mut objects: Vec<NodeRecord> = Vec::new();
    let mut edges: Vec<EdgeRecord> = Vec::new();
    for mutation in mutations {
        match mutation {
            GraphMutation::NodeUpsert(node) => objects.push(node),
            GraphMutation::EdgeUpsert(edge) => edges.push(edge),
        }
    }

    let changed_files = files.iter().map(|file| file.rel_path.clone()).collect();

    SessionDelta {
        commit_sha,
        changed_files,
        objects,
        edges,
        // Additions-only: IK-2 computes tombstones/removed-edges against a base.
        tombstoned_object_ids: Vec::new(),
        removed_edge_ids: Vec::new(),
    }
}

/// IK-1: serve an instant code KG for a set of indexed files overlaid on a base
/// snapshot. Builds the `SessionDelta` (IK-0b) and merges it via
/// `HarnessInstantKg`, which the `harness_kg_*` MCP/HTTP surface already serves
/// (search / ppr / impact / explain_edge). The `base` is the committed graph the
/// session edits sit on; an empty base yields a view of just the edit. Manifest
/// provenance (a `CodeKgManifest`) is IK-1b; the merge accepts `None` today. IK-3
/// wires this to a live session-reingest verb.
#[allow(dead_code)]
pub(crate) fn serve_session_kg(
    base: GraphSnapshot,
    config: &IngestConfig,
    files: &[IndexedFile],
    commit_sha: Option<String>,
) -> HarnessInstantKg {
    let delta = build_session_delta(config, files, commit_sha);
    HarnessInstantKg::new(base, None, delta)
}

/// IK-2: build a `SessionDelta` that also RETRACTS what an edited file deleted.
/// Beyond the additions `build_session_delta` produces, this scans the base
/// snapshot for code records under the changed files and, for any the fresh parse
/// no longer emits, records a tombstone (a deleted symbol) or a removed edge (a
/// dropped call/dependency from a changed-file symbol). Ids are deterministic
/// (`stable_hash(repo_id, path, kind, name, line)`, no generation), so a surviving
/// symbol keeps its id across re-index and is never falsely tombstoned.
/// `HarnessInstantKg` also drops edges left dangling by a tombstone, so the edge
/// retraction here is belt-and-suspenders for the both-endpoints-survive case.
#[allow(dead_code)]
pub(crate) fn build_session_delta_with_base(
    config: &IngestConfig,
    files: &[IndexedFile],
    base: &GraphSnapshot,
    commit_sha: Option<String>,
) -> SessionDelta {
    let mut delta = build_session_delta(config, files, commit_sha);

    // The file ids the fresh parse covers (read off the delta's CodeFile nodes).
    let changed_file_ids: HashSet<&str> = delta
        .objects
        .iter()
        .filter(|node| node.labels.iter().any(|l| l.as_str() == CODE_FILE_LABEL))
        .filter_map(|node| node.properties.get("file_id").and_then(|v| v.as_str()))
        .collect();
    let new_object_ids: HashSet<&str> = delta.objects.iter().map(|n| n.id.as_str()).collect();
    let new_edge_ids: HashSet<&str> = delta.edges.iter().map(|e| e.id.as_str()).collect();

    // Base symbols belonging to a changed file: tombstone the ones the fresh parse
    // dropped. Map each base symbol id to its file id for the edge pass below.
    let mut base_symbol_file: HashMap<&str, &str> = HashMap::new();
    let mut tombstoned: Vec<String> = Vec::new();
    for node in &base.nodes {
        if !node.labels.iter().any(|l| l.as_str() == CODE_SYMBOL_LABEL) {
            continue;
        }
        let Some(file_id) = node.properties.get("file_id").and_then(|v| v.as_str()) else {
            continue;
        };
        base_symbol_file.insert(node.id.as_str(), file_id);
        if changed_file_ids.contains(file_id) && !new_object_ids.contains(node.id.as_str()) {
            tombstoned.push(node.id.clone());
        }
    }
    tombstoned.sort();

    // Base edges originating from a changed-file symbol that the fresh parse no
    // longer emits are removed.
    let mut removed_edges: Vec<String> = Vec::new();
    for edge in &base.edges {
        let from_changed = base_symbol_file
            .get(edge.from_id.as_str())
            .is_some_and(|file_id| changed_file_ids.contains(file_id));
        if from_changed && !new_edge_ids.contains(edge.id.as_str()) {
            removed_edges.push(edge.id.clone());
        }
    }

    delta.tombstoned_object_ids = tombstoned;
    delta.removed_edge_ids = removed_edges;
    delta
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::code_index::{collect_code_files, resolve_ingest_config, IngestCodebaseInput};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "theorem-session-delta-{name}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn build_session_delta_packs_code_records_over_changed_files() {
        let repo = unique_dir("repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(
            repo.join("src/lib.rs"),
            "pub fn alpha() -> usize {\n    1\n}\n\npub fn beta() -> usize {\n    alpha()\n}\n",
        )
        .unwrap();

        let config = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            ..Default::default()
        })
        .unwrap();

        let mut files = Vec::new();
        let mut skipped = 0u64;
        collect_code_files(&config.repo_root, &config, &mut files, &mut skipped).unwrap();
        assert!(!files.is_empty(), "fixture file should be indexed");

        let delta = build_session_delta(&config, &files, Some("deadbeef".to_string()));

        assert_eq!(delta.commit_sha, Some("deadbeef".to_string()));
        assert!(
            delta.changed_files.iter().any(|path| path.ends_with("lib.rs")),
            "the changed file is recorded: {:?}",
            delta.changed_files
        );
        // repo node + file node + at least the alpha and beta symbol nodes.
        assert!(
            delta.objects.len() >= 4,
            "expected repo + file + >=2 symbol nodes, got {}",
            delta.objects.len()
        );
        assert!(
            delta
                .objects
                .iter()
                .any(|node| node.labels.iter().any(|label| label.as_str() == "CodeSymbol")),
            "symbol nodes are present in the delta"
        );
        // contains-file + declares-symbol edges (and the beta->alpha call edge).
        assert!(!delta.edges.is_empty(), "edges are present in the delta");
        // Additions-only for now: no tombstones / removed edges yet (IK-2).
        assert!(delta.tombstoned_object_ids.is_empty());
        assert!(delta.removed_edge_ids.is_empty());

        let _ = fs::remove_dir_all(&repo);
    }

    #[test]
    fn served_instant_kg_overlays_code_edit_on_base() {
        let repo = unique_dir("serve-repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        fs::write(repo.join("src/lib.rs"), "pub fn alpha() -> usize {\n    1\n}\n").unwrap();

        let config = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            ..Default::default()
        })
        .unwrap();
        let mut files = Vec::new();
        let mut skipped = 0u64;
        collect_code_files(&config.repo_root, &config, &mut files, &mut skipped).unwrap();

        let base = GraphSnapshot {
            version: 0,
            nodes: vec![],
            edges: vec![],
        };
        let view = serve_session_kg(base, &config, &files, Some("cafef00d".to_string()));

        // Empty base + the delta => the served view is exactly the code records.
        let delta = build_session_delta(&config, &files, None);
        assert_eq!(view.status().total_objects, delta.objects.len());
        assert_eq!(view.status().total_edges, delta.edges.len());
        // The alpha symbol is served through the merged instant KG.
        let merged = view.merged_snapshot();
        assert!(
            merged.nodes.iter().any(|node| {
                node.labels.iter().any(|label| label.as_str() == "CodeSymbol")
                    && node.properties.get("name").and_then(|v| v.as_str()) == Some("alpha")
            }),
            "the alpha symbol is served through the instant KG"
        );

        let _ = fs::remove_dir_all(&repo);
    }

    fn index_repo(repo: &std::path::Path) -> (IngestConfig, Vec<IndexedFile>) {
        let config = resolve_ingest_config(IngestCodebaseInput {
            tenant_id: "theorem".to_string(),
            repo_path: repo.display().to_string(),
            ..Default::default()
        })
        .unwrap();
        let mut files = Vec::new();
        let mut skipped = 0u64;
        collect_code_files(&config.repo_root, &config, &mut files, &mut skipped).unwrap();
        (config, files)
    }

    fn symbol_id(delta: &SessionDelta, name: &str) -> Option<String> {
        delta
            .objects
            .iter()
            .find(|node| {
                node.labels.iter().any(|l| l.as_str() == "CodeSymbol")
                    && node.properties.get("name").and_then(|v| v.as_str()) == Some(name)
            })
            .map(|node| node.id.clone())
    }

    #[test]
    fn session_delta_against_base_tombstones_deleted_symbols() {
        let repo = unique_dir("tomb-repo");
        fs::create_dir_all(repo.join("src")).unwrap();
        let src = repo.join("src/lib.rs");

        // v1: alpha + beta (beta calls alpha).
        fs::write(
            &src,
            "pub fn alpha() -> usize {\n    1\n}\n\npub fn beta() -> usize {\n    alpha()\n}\n",
        )
        .unwrap();
        let (config1, files1) = index_repo(&repo);
        let delta1 = build_session_delta(&config1, &files1, None);
        let beta_id = symbol_id(&delta1, "beta").expect("beta symbol in v1");
        let base = GraphSnapshot {
            version: 1,
            nodes: delta1.objects.clone(),
            edges: delta1.edges.clone(),
        };

        // v2: beta deleted, alpha kept.
        fs::write(&src, "pub fn alpha() -> usize {\n    1\n}\n").unwrap();
        let (config2, files2) = index_repo(&repo);
        let delta2 =
            build_session_delta_with_base(&config2, &files2, &base, Some("v2".to_string()));

        // beta is retracted; alpha survives with its id intact (never tombstoned).
        assert!(
            delta2.tombstoned_object_ids.contains(&beta_id),
            "the deleted beta symbol is tombstoned"
        );
        let alpha_id = symbol_id(&delta2, "alpha").expect("alpha symbol in v2");
        assert!(
            !delta2.tombstoned_object_ids.contains(&alpha_id),
            "the surviving alpha symbol is not tombstoned"
        );
        // beta's call edge (beta -> alpha) originated from a changed-file symbol and
        // is not re-emitted, so it is removed.
        assert!(
            !delta2.removed_edge_ids.is_empty(),
            "beta's dropped call edge is removed"
        );

        let _ = fs::remove_dir_all(&repo);
    }
}
