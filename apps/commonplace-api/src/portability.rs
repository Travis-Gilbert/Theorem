//! Import and export: no lock-in (plan unit X2).
//!
//! Plain JSON and markdown in and out, so data enters and leaves cleanly. JSON
//! is the lossless interchange (the full item + collection records round-trip);
//! markdown is a human-readable one-way rendering.
//!
//! Identity is preserved on import: collections are recreated with their
//! original ids first (so item->collection references resolve), then items are
//! written with their original ids (so `IN_COLLECTION` edges re-link). `created_at`
//! survives; `updated_at` refreshes because an import is itself a write.

use std::collections::BTreeMap;

use commonplace::{
    content_hash, BlobStore, Collection, Commonplace, EmbeddingGraphStore, Item, ItemBody,
    COLLECTION_LABEL,
};
use rustyred_thg_core::{GraphStore, GraphStoreError, GraphStoreResult, NodeQuery, NodeRecord};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Current export schema version.
pub const EXPORT_VERSION: u32 = 1;

/// A portable snapshot of a CommonPlace instance.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportDocument {
    pub version: u32,
    pub items: Vec<Item>,
    pub collections: Vec<Collection>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub blobs: BTreeMap<String, Vec<u8>>,
}

/// What an import wrote.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImportSummary {
    pub items: usize,
    pub collections: usize,
}

/// Build the full export document for an instance.
pub fn export<S, B>(cp: &Commonplace<S, B>) -> GraphStoreResult<ExportDocument>
where
    S: GraphStore,
    B: BlobStore,
{
    let items = cp.all_items()?;
    let mut blobs = BTreeMap::new();
    for item in &items {
        if let ItemBody::Blob { content_hash, .. } = &item.body {
            let bytes = cp.read_blob(item)?.ok_or_else(|| {
                GraphStoreError::new(
                    "commonplace_export_missing_blob",
                    format!("item {} references missing blob {content_hash}", item.id),
                )
            })?;
            blobs.insert(content_hash.clone(), bytes);
        }
    }
    let collection_ids: Vec<String> = cp
        .store()
        .query_nodes(NodeQuery::label(COLLECTION_LABEL).with_limit(usize::MAX))
        .into_iter()
        .map(|node| node.id)
        .collect();
    let mut collections = Vec::with_capacity(collection_ids.len());
    for id in collection_ids {
        if let Some(collection) = cp.get_collection(&id)? {
            collections.push(collection);
        }
    }
    Ok(ExportDocument {
        version: EXPORT_VERSION,
        items,
        collections,
        blobs,
    })
}

/// Lossless JSON export.
pub fn export_json<S, B>(cp: &Commonplace<S, B>) -> GraphStoreResult<String>
where
    S: GraphStore,
    B: BlobStore,
{
    let document = export(cp)?;
    serde_json::to_string_pretty(&document).map_err(serde_err)
}

/// Human-readable markdown export (one-way rendering).
pub fn export_markdown<S, B>(cp: &Commonplace<S, B>) -> GraphStoreResult<String>
where
    S: GraphStore,
    B: BlobStore,
{
    let document = export(cp)?;
    let mut out = String::from("# CommonPlace export\n");
    for collection in &document.collections {
        out.push_str(&format!(
            "\n## {} ({})\n",
            collection.name,
            collection.kind.as_str()
        ));
        for item in document
            .items
            .iter()
            .filter(|item| item.collections.contains(&collection.id))
        {
            push_item_markdown(&mut out, item);
        }
    }
    let unfiled: Vec<&Item> = document
        .items
        .iter()
        .filter(|item| item.collections.is_empty())
        .collect();
    if !unfiled.is_empty() {
        out.push_str("\n## Unfiled\n");
        for item in unfiled {
            push_item_markdown(&mut out, item);
        }
    }
    Ok(out)
}

fn push_item_markdown(out: &mut String, item: &Item) {
    out.push_str(&format!("\n### {} [{}]\n", item.title, item.kind.as_str()));
    if !item.tags.is_empty() {
        out.push_str(&format!("tags: {}\n", item.tags.join(", ")));
    }
    match &item.body {
        ItemBody::Inline { text } => out.push_str(&format!("\n{text}\n")),
        ItemBody::Blob { content_hash, .. } => out.push_str(&format!("\n[blob {content_hash}]\n")),
        ItemBody::Empty => {}
    }
}

/// Import an export document, preserving ids so memberships survive.
pub fn import<S, B>(
    cp: &mut Commonplace<S, B>,
    document: &ExportDocument,
) -> GraphStoreResult<ImportSummary>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let mut embedding_dimension = None;
    for item in &document.items {
        if let Some(embedding) = item.embedding.as_ref() {
            if embedding.is_empty() {
                return Err(GraphStoreError::new(
                    "commonplace_import_empty_embedding",
                    format!("item {} has an empty embedding vector", item.id),
                ));
            }
            match embedding_dimension {
                Some(dimension) if dimension != embedding.len() => {
                    return Err(GraphStoreError::new(
                        "commonplace_import_embedding_dimension_mismatch",
                        format!(
                            "item {} has embedding dimension {}, expected {dimension}",
                            item.id,
                            embedding.len()
                        ),
                    ));
                }
                None => embedding_dimension = Some(embedding.len()),
                _ => {}
            }
        }
        if let ItemBody::Blob { content_hash, .. } = &item.body {
            if !document.blobs.contains_key(content_hash) {
                return Err(GraphStoreError::new(
                    "commonplace_import_missing_blob",
                    format!("item {} references missing blob {content_hash}", item.id),
                ));
            }
        }
    }

    for (expected_hash, bytes) in &document.blobs {
        let actual_hash = cp.blobs().put(bytes)?;
        if actual_hash != *expected_hash || content_hash(bytes) != *expected_hash {
            return Err(GraphStoreError::new(
                "commonplace_import_blob_hash_mismatch",
                format!("blob bytes do not match exported hash {expected_hash}"),
            ));
        }
    }

    // Re-index item embeddings if present, so imported items stay searchable.
    if let Some(dimension) = embedding_dimension {
        cp.store_mut()
            .designate_commonplace_item_embedding(dimension)?;
    }

    // Recreate collections with their original ids first.
    for collection in &document.collections {
        let record = NodeRecord::new(
            collection.id.clone(),
            [COLLECTION_LABEL],
            json!({
                "name": collection.name,
                "kind": collection.kind.as_str(),
                "created_at_ms": collection.created_at_ms,
            }),
        );
        cp.store_mut().upsert_node(record)?;
    }

    // Recreate items with their original ids; put_item re-links memberships.
    for item in &document.items {
        cp.put_item(item.clone())?;
    }

    Ok(ImportSummary {
        items: document.items.len(),
        collections: document.collections.len(),
    })
}

fn serde_err(error: serde_json::Error) -> rustyred_thg_core::GraphStoreError {
    rustyred_thg_core::GraphStoreError::new("commonplace_export_serde", error.to_string())
}
