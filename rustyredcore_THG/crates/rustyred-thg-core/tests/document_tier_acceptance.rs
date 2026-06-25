//! Spec-anchored acceptance suite for SPEC-RUSTYRED-DOCUMENT-TIER.
//!
//! The DOCUMENT-TIER implementation (`doc_tree.rs` + the document-bytes path in
//! `object_store.rs`) was built by Codex. This suite is the CC verification
//! layer that traces each numbered acceptance criterion from the spec to an
//! observable behavior over the PUBLIC crate API, mirroring the
//! `relational_core_acceptance.rs` pattern (Codex builds, CC proves the bar).
//!
//! Every test exercises only `rustyred_thg_core` public exports, so it doubles
//! as a contract test: if the doc-tree surface drifts, this breaks.

use std::path::PathBuf;

use rustyred_thg_core::{
    ColdTierKind, DiskObjectStore, DocTree, InMemoryGraphStore, NodeQuery, NodeRecord, PathKey,
    DOC_TREE_CONTENT_HASH_PROPERTY, DOC_TREE_PATH_PROPERTY,
};
use serde_json::json;

/// Zstd frame magic number (little-endian 0xFD2FB528). Cold bodies on disk must
/// start with this, which is how we prove "compressed at rest" without a zstd
/// dev-dependency.
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];

fn temp_store(name: &str) -> (PathBuf, DiskObjectStore) {
    let dir = std::env::temp_dir().join(format!("doctier-accept-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let store = DiskObjectStore::open(&dir).expect("open disk object store");
    (dir, store)
}

/// AC #1: A document put under a path is retrievable by that path, and a prefix
/// scan returns every document under a namespace in path order.
#[test]
fn ac1_path_lookup_and_ordered_prefix_scan() {
    let (_dir, store) = temp_store("ac1");
    let mut tree = DocTree::new(64);

    // Insert out of order across two sibling namespaces.
    for segments in [
        ["tenant", "project", "episode", "2"],
        ["tenant", "project", "episode", "10"],
        ["tenant", "project", "episode", "1"],
        ["tenant", "project2", "episode", "9"],
    ] {
        let key = PathKey::from_segments(segments).unwrap();
        tree.put_body(key, b"body", ColdTierKind::Cold, 1, None, &store)
            .unwrap();
    }

    // Point lookup by exact path resolves.
    let one = PathKey::from_slash_path("tenant/project/episode/1").unwrap();
    assert!(tree.get(&one).is_some(), "exact path must be retrievable");

    // Prefix scan returns ONLY the namespace, in path (byte) order, and the
    // NUL separator prevents `project2` from matching the `project` prefix.
    let prefix = PathKey::prefix_from_segments(["tenant", "project"]).unwrap();
    let paths: Vec<String> = tree
        .range_prefix(prefix.as_bytes())
        .map(|(path, _)| path.to_slash_path())
        .collect();
    assert_eq!(
        paths,
        vec![
            // "1" < "10" < "2" lexically: prefix scan is byte-ordered, which is
            // the documented path order.
            "tenant/project/episode/1".to_string(),
            "tenant/project/episode/10".to_string(),
            "tenant/project/episode/2".to_string(),
        ],
        "prefix scan must return exactly the namespace in path order, excluding the project2 sibling"
    );
}

/// AC #2: Eviction and rehydration make no sqlx call and no network hop; the
/// native path-keyed structure serves residency.
///
/// Observable proof: a full store -> evict-from-hot -> resolve cycle runs using
/// ONLY a `DocTree` (in-process imbl::OrdMap) and a `DiskObjectStore` (local
/// files). There is no database handle, connection pool, or socket anywhere on
/// the path; the residency lookup is a keyed `get`/`resolve_body`, not a scan.
#[test]
fn ac2_residency_is_in_process_no_db_no_network() {
    let (_dir, store) = temp_store("ac2");
    let mut tree = DocTree::new(8);
    let path = PathKey::from_slash_path("tenant/memory/mem:42").unwrap();
    let body = b"a long-term memory body that has been spilled to the cold tier";

    // "Eviction": materialize the body cold (overflow to the object store).
    tree.put_body(path.clone(), body, ColdTierKind::Cold, 7, None, &store)
        .unwrap();

    // "Rehydration": resolve it back. The only collaborators are the path-keyed
    // tree and the on-disk object store -- no sqlx, no network.
    let rehydrated = tree.resolve_body(&path, &store).unwrap().unwrap();
    assert_eq!(
        rehydrated, body,
        "cold body must rehydrate to identical bytes"
    );

    // A miss is a keyed lookup returning None, not a scan/error.
    let absent = PathKey::from_slash_path("tenant/memory/missing").unwrap();
    assert_eq!(tree.resolve_body(&absent, &store).unwrap(), None);
}

/// AC #3: A document under the inline threshold is stored inline in the leaf and
/// resolves in one tree lookup; a document over the threshold is stored in the
/// object store with the leaf holding the pointer; both round-trip to identical
/// bytes.
#[test]
fn ac3_inline_and_overflow_round_trip_identical() {
    let (_dir, store) = temp_store("ac3");
    let mut tree = DocTree::new(16);

    let small = b"tiny body"; // 9 bytes <= 16 threshold -> inline
    let large = b"this document body is comfortably larger than the sixteen byte inline threshold"; // overflow

    let small_path = PathKey::from_slash_path("t/p/doc/small").unwrap();
    let large_path = PathKey::from_slash_path("t/p/doc/large").unwrap();

    let small_entry = tree
        .put_body(
            small_path.clone(),
            small,
            ColdTierKind::Cold,
            1,
            None,
            &store,
        )
        .unwrap();
    let large_entry = tree
        .put_body(
            large_path.clone(),
            large,
            ColdTierKind::Cold,
            1,
            None,
            &store,
        )
        .unwrap();

    // Inline: body lives in the leaf, no object-store file written for it.
    assert!(small_entry.is_inline(), "sub-threshold body must be inline");
    // Overflow: leaf holds the pointer (content_hash), body in the object store.
    assert!(
        !large_entry.is_inline(),
        "over-threshold body must overflow"
    );
    let large_hash = large_entry.content_hash.clone().expect("overflow hash");
    assert!(
        store.document_path(&large_hash).exists(),
        "overflow body must be written to the object store by content hash"
    );

    // Both round-trip to identical bytes.
    assert_eq!(
        tree.resolve_body(&small_path, &store).unwrap().unwrap(),
        small
    );
    assert_eq!(
        tree.resolve_body(&large_path, &store).unwrap().unwrap(),
        large
    );
}

/// AC #4: Cold bodies are ZStandard-compressed at rest and round-trip to
/// identical bytes, and the content hash is computed over the uncompressed bytes.
///
/// - compressed-at-rest: the on-disk overflow file begins with the zstd magic.
/// - round-trip identical: `resolve_body` returns the original bytes.
/// - hash over uncompressed bytes: the address is content-deterministic over the
///   BODY (same body at two different paths yields the same hash and dedups to a
///   single on-disk object), which is only possible if the hash is taken over
///   the raw body, not the per-write compressed frame. (`put_body` computes
///   `content_hash_bytes(body)` before `compress_cold_bytes`.)
#[test]
fn ac4_compressed_at_rest_hash_over_uncompressed() {
    let (_dir, store) = temp_store("ac4");
    let mut tree = DocTree::new(2); // threshold 2 -> "abc" (3 bytes) overflows to disk

    // "abc" is the canonical SHA-256 test vector. The DISCRIMINATING check:
    //   sha256("abc")       = ba7816bf...20015ad     (hash of the RAW body)
    //   sha256(zstd("abc")) = a DIFFERENT value      (hash of the stored frame)
    // Asserting the address carries the raw-body digest proves the hash is taken
    // over the UNCOMPRESSED bytes, not the compressed frame. (The earlier
    // dedup-only check could not tell these apart because zstd is deterministic.)
    const SHA256_ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    let entry = tree
        .put_body(
            PathKey::from_slash_path("t/p/abc").unwrap(),
            b"abc",
            ColdTierKind::Cold,
            1,
            None,
            &store,
        )
        .unwrap();
    let hash = entry.content_hash.clone().expect("overflow hash");
    assert!(hash.starts_with("sha256:"), "address is a sha256: {hash}");
    assert!(
        hash.ends_with(SHA256_ABC),
        "content address must be sha256 of the RAW body (not the zstd frame): {hash}"
    );

    // Compressed at rest: the on-disk file is a zstd frame, not the raw body.
    let on_disk = std::fs::read(store.document_path(&hash)).expect("read cold file");
    assert_eq!(&on_disk[..4], &ZSTD_MAGIC, "cold body must be a zstd frame");
    assert_ne!(
        on_disk.as_slice(),
        b"abc",
        "stored bytes differ from the raw body"
    );

    // Round-trips to identical bytes.
    assert_eq!(store.get_document_bytes(&hash).unwrap().unwrap(), b"abc");

    // Same body at a different path dedups to one object; a different body does not.
    let again = tree
        .put_body(
            PathKey::from_slash_path("t/p/abc2").unwrap(),
            b"abc",
            ColdTierKind::Cold,
            2,
            None,
            &store,
        )
        .unwrap();
    assert_eq!(again.content_hash, entry.content_hash);
    let other = tree
        .put_body(
            PathKey::from_slash_path("t/p/xyz").unwrap(),
            b"a different cold body",
            ColdTierKind::Cold,
            3,
            None,
            &store,
        )
        .unwrap();
    assert_ne!(other.content_hash, entry.content_hash);
}

/// AC #5: Updating a document at a path retains the prior content as
/// addressable, and a snapshot taken before the update still resolves the prior
/// body (copy-on-write versioning via imbl::OrdMap structural sharing).
#[test]
fn ac5_update_retains_prior_and_snapshot_resolves_it() {
    let (_dir, store) = temp_store("ac5");
    let mut tree = DocTree::new(64);
    let path = PathKey::from_slash_path("tenant/project/doc/evolving").unwrap();

    let before = tree
        .put_body(
            path.clone(),
            b"the original body",
            ColdTierKind::Cold,
            1,
            None,
            &store,
        )
        .unwrap();
    let before_hash = before.content_hash.clone().unwrap();

    // O(1) copy-on-write snapshot before the update.
    let snapshot = tree.snapshot();

    let after = tree
        .put_body(
            path.clone(),
            b"the revised body",
            ColdTierKind::Cold,
            2,
            None,
            &store,
        )
        .unwrap();

    // Live tree resolves the new body; the snapshot still resolves the old one.
    assert_eq!(
        tree.resolve_body(&path, &store).unwrap().unwrap(),
        b"the revised body"
    );
    assert_eq!(
        snapshot.resolve_body(&path, &store).unwrap().unwrap(),
        b"the original body",
        "a pre-update snapshot must still resolve the pre-update body"
    );

    // The prior content hash is retained as addressable history on the new entry.
    assert!(
        after.previous_hashes.contains(&before_hash),
        "the update must retain the prior content hash as addressable history"
    );
}

/// AC #6: A hot memory node carrying a doc-tree path resolves its body through
/// one tree lookup plus at most one body fetch.
#[test]
fn ac6_memory_node_dock_resolves_body() {
    let (_dir, store) = temp_store("ac6");
    let mut tree = DocTree::new(8);

    let mut node = NodeRecord::new(
        "mem:dock-1",
        ["Memory", "Episode"],
        json!({ "gist": "short gist that stays hot", "topic": "planning" }),
    );
    let path = PathKey::from_slash_path("tenant/memory/mem:dock-1").unwrap();
    let body = b"the full episode body that lives cold while the dock stays hot";

    tree.materialize_node_body(&mut node, path, body, 99, &store)
        .unwrap();

    // The node now carries the dock path; resolution is one tree lookup + one
    // body fetch via the path property.
    assert!(node.properties.get(DOC_TREE_PATH_PROPERTY).is_some());
    let resolved = tree
        .resolve_memory_node_body(&node, &store)
        .unwrap()
        .unwrap();
    assert_eq!(resolved, body);
}

/// AC #7: Promotion of a recalled working node materializes its body as a cold
/// document and leaves the node findable by every index that covered it (the
/// dock-and-document split: gist + labels + indexed properties stay hot).
#[test]
fn ac7_promotion_keeps_node_findable() {
    let (_dir, store) = temp_store("ac7");
    let mut tree = DocTree::new(8);

    let mut node = NodeRecord::new(
        "mem:promote-1",
        ["Memory", "Episode"],
        json!({
            "gist": "the searchable summary",
            "body": "a heavy body that should be evicted to the cold document tier",
            "topic": "planning",
            "project": "theorem"
        }),
    );
    let path = PathKey::from_slash_path("tenant/memory/mem:promote-1").unwrap();
    let body = b"a heavy body that should be evicted to the cold document tier";

    tree.materialize_node_body(&mut node, path, body, 123, &store)
        .unwrap();

    // The body left the hot node...
    assert!(
        node.properties.get("body").is_none(),
        "heavy body must leave the hot node"
    );

    // ...and the promoted dock node is still FOUND by a real index after promotion:
    // insert it into a graph store and retrieve it through the label query (not
    // merely assert fields on a detached struct).
    let mut graph = InMemoryGraphStore::new();
    graph.upsert_node(node.clone()).unwrap();
    let found = graph.query_nodes(NodeQuery::label("Memory"));
    let dock = found
        .iter()
        .find(|n| n.id == "mem:promote-1")
        .expect("promoted dock must be findable by the label index");

    // The dock retrieved from the store still carries the searchable surface
    // (gist/topic/project) and the cold content-hash dock; only the body is gone.
    assert!(dock.labels.contains(&"Episode".to_string()));
    assert_eq!(
        dock.properties.get("gist"),
        Some(&json!("the searchable summary"))
    );
    assert_eq!(dock.properties.get("topic"), Some(&json!("planning")));
    assert_eq!(dock.properties.get("project"), Some(&json!("theorem")));
    assert!(dock.properties.get("body").is_none());
    assert!(
        dock.properties
            .get(DOC_TREE_CONTENT_HASH_PROPERTY)
            .is_some(),
        "promoted node must keep the cold content-hash dock"
    );

    // And the promoted body is recoverable from the cold tier via the dock.
    assert_eq!(
        tree.resolve_memory_node_body(dock, &store)
            .unwrap()
            .unwrap(),
        body
    );
}
