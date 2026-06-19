//! F1 acceptance: the consumer object model.
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, F1):
//! "an `Item` of any kind writes and reads back with its metadata, residency,
//! tags, and collections; a `File` resolves its blob by content hash; a
//! `Collection` returns its items."
//!
//! Verified against the running engine two ways: the in-memory engine (fast)
//! and the durable `RedCoreGraphStore` + `DiskObjectStore` across a restart.

use commonplace::{
    content_hash, Collection, CollectionKind, Commonplace, InMemoryBlobStore, Item, ItemBody,
    ItemKind, Residency,
};
use rustyred_thg_core::InMemoryGraphStore;

fn fresh() -> Commonplace<InMemoryGraphStore, InMemoryBlobStore> {
    Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
}

#[test]
fn item_of_any_kind_writes_and_reads_back_with_metadata() {
    let mut cp = fresh();
    let collection = cp
        .create_collection("Research", CollectionKind::Manual)
        .unwrap();

    let draft = Item::new(ItemKind::Note, "On embeddings")
        .with_text("Vectors that sit near each other tend to be similar.")
        .with_source("web-clipper")
        .with_residency(Residency::Synced)
        .with_tags(["ml", "vectors", "ml"]) // duplicate is collapsed
        .with_collections([collection.id.clone()])
        .with_classification("knowledge")
        .with_extra("starred", serde_json::json!(true));

    let stored = cp.put_item(draft).unwrap();
    assert!(!stored.id.is_empty(), "id assigned on write");
    assert!(stored.created_at_ms > 0 && stored.updated_at_ms > 0);

    let got = cp.get_item(&stored.id).unwrap().expect("item reads back");
    assert_eq!(got.kind, ItemKind::Note);
    assert_eq!(got.title, "On embeddings");
    assert_eq!(got.residency, Residency::Synced);
    assert_eq!(got.source.as_deref(), Some("web-clipper"));
    assert_eq!(got.classification.as_deref(), Some("knowledge"));
    assert_eq!(got.tags, vec!["ml".to_string(), "vectors".to_string()]);
    assert_eq!(got.collections, vec![collection.id.clone()]);
    assert_eq!(got.extra.get("starred"), Some(&serde_json::json!(true)));
    match got.body {
        ItemBody::Inline { text } => assert!(text.contains("similar")),
        other => panic!("expected inline body, got {other:?}"),
    }

    // A different kind round-trips too (the model stores anything).
    let link = cp
        .put_item(Item::new(ItemKind::Other("bookmark".into()), "A bookmark"))
        .unwrap();
    let link = cp.get_item(&link.id).unwrap().unwrap();
    assert_eq!(link.kind, ItemKind::Other("bookmark".into()));
}

#[test]
fn file_resolves_its_blob_by_content_hash() {
    let mut cp = fresh();
    let bytes: &[u8] = b"binary-payload-\x00\x01\x02-the-rest";

    let stored = cp
        .put_file("avatar.png", bytes, Some("image/png".to_string()))
        .unwrap();
    assert_eq!(stored.kind, ItemKind::File);

    let (hash, len, mime) = match &stored.body {
        ItemBody::Blob {
            content_hash,
            byte_len,
            mime,
        } => (content_hash.clone(), *byte_len, mime.clone()),
        other => panic!("expected blob body, got {other:?}"),
    };
    assert!(hash.starts_with("sha256:"));
    assert_eq!(len, bytes.len() as u64);
    assert_eq!(mime.as_deref(), Some("image/png"));
    assert_eq!(
        content_hash(bytes),
        hash,
        "content addressing is deterministic"
    );

    let got = cp.get_item(&stored.id).unwrap().unwrap();
    let resolved = cp.read_blob(&got).unwrap().expect("blob resolves by hash");
    assert_eq!(resolved, bytes);
}

#[test]
fn collection_returns_its_items() {
    let mut cp = fresh();
    let collection = cp
        .create_collection("Reading list", CollectionKind::Manual)
        .unwrap();

    let a = cp
        .put_item(Item::new(ItemKind::Link, "A").with_collections([collection.id.clone()]))
        .unwrap();
    let b = cp
        .put_item(Item::new(ItemKind::Link, "B").with_collections([collection.id.clone()]))
        .unwrap();
    // An item NOT in the collection must not appear.
    let _outside = cp.put_item(Item::new(ItemKind::Link, "C")).unwrap();

    let items = cp.collection_items(&collection.id).unwrap();
    let mut titles: Vec<String> = items.iter().map(|item| item.title.clone()).collect();
    titles.sort();
    assert_eq!(titles, vec!["A".to_string(), "B".to_string()]);
    assert!(items
        .iter()
        .all(|item| item.collections.contains(&collection.id)));

    // Round-trip the collection itself.
    let loaded: Collection = cp.get_collection(&collection.id).unwrap().unwrap();
    assert_eq!(loaded.name, "Reading list");
    assert_eq!(loaded.kind, CollectionKind::Manual);

    assert!(a.id != b.id);
}

#[test]
fn tags_are_graph_nodes_deduped_by_slug() {
    let mut cp = fresh();
    let item = cp
        .put_item(Item::new(ItemKind::Note, "Tagged").with_tags(["Machine Learning"]))
        .unwrap();
    // Same label under different casing/separators resolves to one tag node.
    cp.tag_item(&item.id, "machine-learning").unwrap();

    let tags = cp.item_tags(&item.id).unwrap();
    assert_eq!(tags.len(), 1, "exact-name dedup via stable slug");
    assert_eq!(tags[0].id, "tag:machine-learning");
}

#[test]
fn items_and_blobs_survive_engine_restart() {
    use rustyred_thg_core::{DiskObjectStore, RedCoreGraphStore, RedCoreOptions};

    let root = unique_tmp("commonplace-f1");
    let graph_dir = root.join("graph");
    let blob_dir = root.join("blobs");

    let item_id;
    let collection_id;
    {
        let store = RedCoreGraphStore::open(&graph_dir, RedCoreOptions::default()).unwrap();
        let blobs = DiskObjectStore::open(&blob_dir).unwrap();
        let mut cp = Commonplace::new(store, blobs);

        let file = cp.put_file("doc.txt", b"durable bytes", None).unwrap();
        item_id = file.id.clone();
        let collection = cp
            .create_collection("Persisted", CollectionKind::Auto)
            .unwrap();
        collection_id = collection.id.clone();
        cp.add_to_collection(&item_id, &collection_id).unwrap();
    }
    {
        let store = RedCoreGraphStore::open(&graph_dir, RedCoreOptions::default()).unwrap();
        let blobs = DiskObjectStore::open(&blob_dir).unwrap();
        let cp = Commonplace::new(store, blobs);

        let got = cp.get_item(&item_id).unwrap().expect("item rehydrated");
        assert_eq!(got.kind, ItemKind::File);
        let resolved = cp.read_blob(&got).unwrap().expect("blob rehydrated");
        assert_eq!(resolved, b"durable bytes");
        assert_eq!(got.collections, vec![collection_id.clone()]);

        let members = cp.collection_items(&collection_id).unwrap();
        assert_eq!(members.len(), 1);
        assert_eq!(members[0].id, item_id);
    }

    std::fs::remove_dir_all(&root).ok();
}

fn unique_tmp(prefix: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("{prefix}-{nanos:x}"));
    std::fs::create_dir_all(&path).unwrap();
    path
}
