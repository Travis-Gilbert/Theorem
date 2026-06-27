//! Slice 2 acceptance: the [`CommonplaceIngestSink`] lands settled change sets in
//! a durable commonplace graph inside the sidecar, idempotently per path, and
//! writes a change-set lineage chain (Part A deliverables 1 + 4).

use std::path::PathBuf;

use commonplace::{ItemKind, SourceRef};
use commonplace_desktop_runtime::{
    ChangeKind, ChangeSet, CommonplaceIngestSink, FileChange, WatchConfig, CHANGE_SET_LABEL,
    FILE_SOURCE,
};
use rustyred_thg_core::{GraphStore, NodeQuery};

/// A change set of one created file at `<root>/<relative>`.
fn created(root: &std::path::Path, relative: &str) -> ChangeSet {
    ChangeSet {
        changes: vec![FileChange {
            path: root.join(relative),
            kind: ChangeKind::Created,
        }],
    }
}

#[test]
fn ingests_text_file_into_queryable_item() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("notes.md"), b"# Plan\nfirst body").unwrap();

    let config = WatchConfig::new(&root);
    let mut sink = CommonplaceIngestSink::open(&config).unwrap();

    let outcome = sink.ingest_change_set(&created(&root, "notes.md")).unwrap();

    assert_eq!(outcome.ingested.len(), 1, "the text file is ingested");
    assert_eq!(outcome.ingested[0].relative_path, "notes.md");
    assert!(outcome.change_set_node_id.is_some(), "a lineage node is written");

    // The ingested item is queryable in the durable graph, keyed by its source ref.
    let item = sink
        .commonplace()
        .item_by_source_ref(FILE_SOURCE, "notes.md")
        .unwrap()
        .expect("item is queryable by source ref");
    assert_eq!(item.title, "notes.md");
    assert_eq!(item.kind, ItemKind::Doc);
    assert_eq!(item.source_ref, Some(SourceRef::new(FILE_SOURCE, "notes.md")));
    assert_eq!(item.id, outcome.ingested[0].item_id);
}

#[test]
fn reingesting_same_path_updates_same_item_not_a_duplicate() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let file = root.join("doc.md");
    std::fs::write(&file, b"version one").unwrap();

    let config = WatchConfig::new(&root);
    let mut sink = CommonplaceIngestSink::open(&config).unwrap();

    let first = sink.ingest_change_set(&created(&root, "doc.md")).unwrap();
    let first_id = first.ingested[0].item_id.clone();
    let first_hash = first.ingested[0].content_hash.clone();

    // Edit the file in place and re-ingest the same path (a Modified change).
    std::fs::write(&file, b"version two, edited").unwrap();
    let second_set = ChangeSet {
        changes: vec![FileChange {
            path: file.clone(),
            kind: ChangeKind::Modified,
        }],
    };
    let second = sink.ingest_change_set(&second_set).unwrap();
    let second_id = second.ingested[0].item_id.clone();
    let second_hash = second.ingested[0].content_hash.clone();

    // Idempotent / provenance across edits: SAME item id, different content hash.
    assert_eq!(first_id, second_id, "re-ingest reuses the same item id");
    assert_ne!(first_hash, second_hash, "the content hash tracks the edit");

    // Exactly one item exists for this path (no duplicate minted on re-ingest).
    let file_items: Vec<_> = sink
        .commonplace()
        .all_items()
        .unwrap()
        .into_iter()
        .filter(|item| item.source_ref == Some(SourceRef::new(FILE_SOURCE, "doc.md")))
        .collect();
    assert_eq!(file_items.len(), 1, "re-ingest must not duplicate the item");

    // Two change-set lineage nodes were written, forming a chain.
    let change_sets = GraphStore::query_nodes(
        sink.commonplace().store(),
        NodeQuery::label(CHANGE_SET_LABEL).with_limit(usize::MAX),
    );
    assert_eq!(change_sets.len(), 2, "one lineage node per change set");
    // The newer node records its predecessor (the chain is walkable).
    let has_predecessor_link = change_sets.iter().any(|node| {
        node.properties
            .get("previous")
            .and_then(|value| value.as_str())
            .is_some()
    });
    assert!(has_predecessor_link, "the second change set follows the first");
}

#[test]
fn skips_binary_and_defers_removed() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // A non-UTF-8 file: must be skipped this slice.
    std::fs::write(root.join("blob.bin"), [0xff, 0xfe, 0x00, 0x01]).unwrap();
    std::fs::write(root.join("keep.txt"), b"real text").unwrap();

    let config = WatchConfig::new(&root);
    let mut sink = CommonplaceIngestSink::open(&config).unwrap();

    let change_set = ChangeSet {
        changes: vec![
            FileChange {
                path: root.join("blob.bin"),
                kind: ChangeKind::Created,
            },
            FileChange {
                path: root.join("keep.txt"),
                kind: ChangeKind::Created,
            },
            FileChange {
                path: root.join("gone.txt"),
                kind: ChangeKind::Removed,
            },
        ],
    };
    let outcome = sink.ingest_change_set(&change_set).unwrap();

    assert_eq!(outcome.ingested.len(), 1, "only the text file is ingested");
    assert_eq!(outcome.ingested[0].relative_path, "keep.txt");
    assert_eq!(
        outcome.skipped,
        vec![root.join("blob.bin")],
        "the binary file is skipped (deferred)"
    );
    assert_eq!(
        outcome.removed_deferred,
        vec![root.join("gone.txt")],
        "removed paths are noted as deferred"
    );
}

#[test]
fn durable_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("persist.md"), b"durable body").unwrap();
    let config = WatchConfig::new(&root);

    {
        let mut sink = CommonplaceIngestSink::open(&config).unwrap();
        sink.ingest_change_set(&created(&root, "persist.md")).unwrap();
    }

    // Reopen the sink over the same sidecar: the item rehydrates from the AOF.
    let reopened = CommonplaceIngestSink::open(&config).unwrap();
    let item = reopened
        .commonplace()
        .item_by_source_ref(FILE_SOURCE, "persist.md")
        .unwrap();
    assert!(
        item.is_some(),
        "the ingested item survives a sink reopen (durable sidecar)"
    );
}

/// The canonical-git boundary: the sink writes only into the sidecar, never the
/// tracked tree. After ingest, the working tree holds exactly the files the test
/// wrote, and the sidecar holds the graph + blobs.
#[test]
fn writes_only_into_the_sidecar() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("src.md"), b"tree content").unwrap();

    let config = WatchConfig::new(&root);
    let mut sink = CommonplaceIngestSink::open(&config).unwrap();
    sink.ingest_change_set(&created(&root, "src.md")).unwrap();

    // Top-level entries of the tree: the one tracked file plus the sidecar dir.
    let mut top_level: Vec<PathBuf> = std::fs::read_dir(&root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    top_level.sort();

    let expected_file = root.join("src.md");
    let sidecar = config.sidecar_dir.clone();
    assert!(top_level.contains(&expected_file), "the tracked file is untouched");
    assert!(top_level.contains(&sidecar), "the sidecar dir exists");
    assert_eq!(
        top_level.len(),
        2,
        "no other files were written into the tree: {top_level:?}"
    );
    // The original file's bytes are unchanged (read-only on the tree).
    assert_eq!(std::fs::read(&expected_file).unwrap(), b"tree content");
}
