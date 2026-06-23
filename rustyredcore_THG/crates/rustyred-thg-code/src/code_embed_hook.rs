//! Hook 2 from the spec: keep `CodeSymbol` embeddings fresh as their text
//! changes, with no batch backfill job.
//!
//! When a symbol's signature/snippet/search text changes, re-embed it and write
//! the vector onto the node's designated vector property, so semantic neighbor
//! search stays current. The embedder is a deterministic, offline hash embedder
//! (the repo's `hash` embedder seam): real and dependency-free, swappable for an
//! SBERT/bge embedder behind the same write path.

use std::collections::BTreeSet;
use std::sync::Arc;

use rustyred_thg_core::{
    HookContext, HookError, HookHandler, HookOutcome, HookRegistration, MutationEvent,
    MutationKind, MutationMatcher, NodeRecord, RedCoreGraphStore,
};
use serde_json::{json, Value};

use crate::{property_string, CODE_SYMBOL_LABEL};

/// Property holding the symbol embedding (a float array) + its designation.
pub const EMBEDDING_PROPERTY: &str = "embedding";
pub const EMBEDDING_DIM: usize = 64;

/// Text property keys whose change should refresh the embedding. The spec names
/// `{signature, doc, body_hash}`; the current code schema carries `signature`,
/// `snippet`, and `search_text`, so we trigger on the union (the spec names are
/// kept for forward-compat when symbol docs/body hashes land).
const EMBED_TRIGGER_PROPS: [&str; 5] = ["signature", "snippet", "search_text", "doc", "body_hash"];

/// Idempotency threshold for the per-component embedding diff.
const EMBED_EPSILON: f32 = 1e-6;

pub fn incremental_embed_hook() -> HookRegistration {
    let handler: HookHandler = Arc::new(embed_handler);
    HookRegistration::new(
        "code.incremental_embed",
        MutationMatcher::any()
            .with_kinds([MutationKind::NodeUpserted])
            .with_labels([CODE_SYMBOL_LABEL])
            .with_changed_props_any(EMBED_TRIGGER_PROPS),
        coalesce_code_embed,
        handler,
    )
}

fn coalesce_code_embed(_event: &MutationEvent) -> Option<String> {
    Some("code-kg-embed".to_string())
}

fn embed_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
) -> Result<HookOutcome, HookError> {
    ensure_embedding_designation(ctx.store)?;

    let mut writes = 0usize;
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for event in events {
        if event.kind != MutationKind::NodeUpserted {
            continue;
        }
        if !seen.insert(event.id.clone()) {
            continue;
        }
        let Some(mut node) = ctx.store.get_node(&event.id).map_err(HookError::from)? else {
            continue;
        };
        if !node.labels.iter().any(|label| label == CODE_SYMBOL_LABEL) {
            continue;
        }
        let text = symbol_embedding_text(&node.properties);
        if text.trim().is_empty() {
            continue;
        }
        let vector = embed_text(&text, EMBEDDING_DIM);

        // Idempotent: skip the write if the same embedding is already present.
        if let Some(existing) = extract_float_vec(&node.properties, EMBEDDING_PROPERTY) {
            if vectors_close(&existing, &vector) {
                continue;
            }
        }
        set_embedding(&mut node, &vector);
        ctx.store.upsert_node(node).map_err(HookError::from)?;
        writes += 1;
    }
    Ok(HookOutcome::Wrote { mutations: writes })
}

/// Designate `(CodeSymbol, embedding)` as a vector field once. Re-designating is
/// avoided because it re-indexes every matching node.
pub(crate) fn ensure_embedding_designation(store: &mut RedCoreGraphStore) -> Result<(), HookError> {
    let already = store.vector_designations().into_iter().any(|designation| {
        designation.label == CODE_SYMBOL_LABEL && designation.property == EMBEDDING_PROPERTY
    });
    if !already {
        store
            .designate_vector_property(CODE_SYMBOL_LABEL, EMBEDDING_PROPERTY, EMBEDDING_DIM)
            .map_err(HookError::from)?;
    }
    Ok(())
}

/// Assemble the embeddable text from the symbol's available fields.
pub(crate) fn symbol_embedding_text(properties: &Value) -> String {
    let mut parts = Vec::new();
    for key in ["name", "signature", "snippet", "doc"] {
        if let Some(value) = property_string(properties, key) {
            if !value.trim().is_empty() {
                parts.push(value);
            }
        }
    }
    parts.join(" ")
}

/// Deterministic offline embedding: hashed bag-of-tokens with signed buckets,
/// L2-normalized. Stable across runs (idempotent) and dependency-free.
pub(crate) fn embed_text(text: &str, dim: usize) -> Vec<f32> {
    let mut vector = vec![0f32; dim];
    for token in text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
    {
        let hash = fnv1a(token.to_ascii_lowercase().as_bytes());
        let index = (hash % dim as u64) as usize;
        let sign = if (hash >> 1) & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    let norm: f32 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

pub(crate) fn extract_float_vec(properties: &Value, key: &str) -> Option<Vec<f32>> {
    properties
        .get(key)?
        .as_array()?
        .iter()
        .map(|value| value.as_f64().map(|f| f as f32))
        .collect()
}

fn vectors_close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= EMBED_EPSILON)
}

pub(crate) fn set_embedding(node: &mut NodeRecord, vector: &[f32]) {
    let array: Vec<Value> = vector.iter().map(|v| json!(*v)).collect();
    match node.properties.as_object_mut() {
        Some(map) => {
            map.insert(EMBEDDING_PROPERTY.to_string(), Value::Array(array));
        }
        None => {
            node.properties = json!({ EMBEDDING_PROPERTY: array });
        }
    }
}
