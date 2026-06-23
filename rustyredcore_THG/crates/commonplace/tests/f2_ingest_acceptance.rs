//! F2 acceptance: auto-structuring ingest.
//!
//! Plan acceptance:
//! "a dropped document or image is embedded, classified into a collection, filed
//! into the folder tree, linked to similar items, and made
//! similarity-searchable with no user action; a near-duplicate entity resolves
//! to the existing one."

use commonplace::{
    Commonplace, InMemoryBlobStore, IngestInput, IngestPipeline, ItemKind, Residency, ENTITY_LABEL,
    ITEM_EMBEDDING_PROPERTY, MENTIONS_ENTITY_EDGE, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{InMemoryGraphStore, NeighborQuery, NodeQuery};

fn fresh() -> Commonplace<InMemoryGraphStore, InMemoryBlobStore> {
    Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new())
}

#[test]
fn document_ingest_auto_structures_without_user_action() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    let receipt = pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Acme contract memo",
                "Client: Acme Corp. Contract review for indemnity and venue clauses.",
            )
            .with_source("dropzone")
            .with_residency(Residency::Synced),
        )
        .unwrap();

    assert_eq!(receipt.item.kind, ItemKind::Doc);
    assert_eq!(receipt.item.residency, Residency::Synced);
    assert!(receipt.item.embedding_ref.is_some());
    assert_eq!(receipt.collection.name, "Legal");
    assert!(receipt.item.collections.contains(&receipt.collection.id));
    assert_eq!(
        receipt
            .item
            .extra
            .get("folder_path")
            .and_then(|v| v.as_str()),
        Some(receipt.folder_path.as_str())
    );
    assert!(receipt.folder_path.starts_with("collections/legal/"));
    assert!(!receipt.embedding.is_empty());

    let collection_items = cp.collection_items(&receipt.collection.id).unwrap();
    assert_eq!(collection_items.len(), 1);
    assert_eq!(collection_items[0].id, receipt.item.id);

    let search = pipeline.search(&cp, "indemnity contract venue", 1).unwrap();
    assert_eq!(search[0].0, receipt.item.id);
    assert!(
        cp.store()
            .get_node(&receipt.item.id)
            .unwrap()
            .properties
            .get(ITEM_EMBEDDING_PROPERTY)
            .is_some(),
        "embedding is a top-level graph property for vector search"
    );
}

#[test]
fn related_documents_are_linked_to_similar_items() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    let first = pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Lease review",
                "Client: Acme Corp. Lease contract with indemnity language.",
            ),
        )
        .unwrap();
    let second = pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Lease follow-up",
                "Client: Acme Corp. Follow-up contract memo about indemnity and lease terms.",
            ),
        )
        .unwrap();

    assert!(
        second
            .similar_items
            .iter()
            .any(|link| link.item_id == first.item.id),
        "second ingest links to the earlier similar document"
    );
    let similar_neighbors = cp
        .store()
        .neighbors(NeighborQuery::out(&second.item.id).with_edge_type(SIMILAR_TO_EDGE));
    assert_eq!(similar_neighbors[0].node_id, first.item.id);
}

#[test]
fn image_ingest_embeds_classifies_files_and_resolves_blob() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();
    let bytes = b"\x89PNG\r\ncommonplace-image-payload".to_vec();

    let receipt = pipeline
        .ingest(
            &mut cp,
            IngestInput::image("Lobby photo", bytes.clone(), Some("image/png".to_string()))
                .with_tags(["photos"]),
        )
        .unwrap();

    assert_eq!(receipt.item.kind, ItemKind::Image);
    assert_eq!(receipt.collection.name, "Photos");
    let blob = cp.read_blob(&receipt.item).unwrap().expect("image blob");
    assert_eq!(blob, bytes);

    let search = pipeline
        .search_embedding(&cp, &receipt.embedding, 1)
        .unwrap();
    assert_eq!(search[0].0, receipt.item.id);
}

#[test]
fn near_duplicate_entities_resolve_to_existing_entity_node() {
    let mut cp = fresh();
    let pipeline = IngestPipeline::default();

    let first = pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Initial matter",
                "Client: Acme Corp. Intake notes for contract dispute.",
            ),
        )
        .unwrap();
    let second = pipeline
        .ingest(
            &mut cp,
            IngestInput::document(
                "Follow-up matter",
                "Client: ACME Corporation. Follow-up notes for the same contract dispute.",
            ),
        )
        .unwrap();

    assert_eq!(first.entities.len(), 1);
    assert_eq!(second.entities.len(), 1);
    assert_eq!(first.entities[0].entity_id, second.entities[0].entity_id);

    let entities = cp
        .store()
        .query_nodes(NodeQuery::label(ENTITY_LABEL).with_limit(usize::MAX));
    assert_eq!(entities.len(), 1);
    let mentions = cp
        .store()
        .neighbors(NeighborQuery::out(&second.item.id).with_edge_type(MENTIONS_ENTITY_EDGE));
    assert_eq!(mentions[0].node_id, first.entities[0].entity_id);
}
