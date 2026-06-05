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

use rustyred_thg_core::{
    EdgeRecord, GraphMutation, GraphSnapshot, HarnessInstantKg, NodeRecord, SessionDelta,
};

use crate::code_index::{build_code_mutations, IndexedFile, IngestConfig};

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
}
