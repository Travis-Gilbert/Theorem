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

use rustyred_code_embedding::{CodeEmbedder, CodeEmbeddingConfig, CodeEmbeddingError};
use rustyred_thg_core::{
    HookContext, HookError, HookHandler, HookOutcome, HookRegistration, MutationEvent,
    MutationKind, MutationMatcher, NodeRecord, RedCoreGraphStore,
};
use serde_json::{json, Value};

use crate::{property_string, CODE_SYMBOL_LABEL};

/// Property holding the symbol embedding (a float array) + its designation.
pub const EMBEDDING_PROPERTY: &str = "embedding";
/// Legacy no-config symbol embedding dimension. Configured W4 embedders may use
/// a different dimension; the designation follows the selected embedder.
pub const EMBEDDING_DIM: usize = 64;

/// Text property keys whose change should refresh the embedding. The spec names
/// `{signature, doc, body_hash}`; the current code schema carries `signature`,
/// `snippet`, and `search_text`, so we trigger on the union (the spec names are
/// kept for forward-compat when symbol docs/body hashes land).
const EMBED_TRIGGER_PROPS: [&str; 5] = ["signature", "snippet", "search_text", "doc", "body_hash"];

/// Idempotency threshold for the per-component embedding diff.
const EMBED_EPSILON: f32 = 1e-6;

pub fn incremental_embed_hook() -> HookRegistration {
    incremental_embed_hook_with_embedder(default_symbol_embedder())
}

pub fn incremental_embed_hook_with_embedder(embedder: Arc<dyn CodeEmbedder>) -> HookRegistration {
    let handler_embedder = Arc::clone(&embedder);
    let handler: HookHandler =
        Arc::new(move |ctx, events| embed_handler(ctx, events, handler_embedder.as_ref()));
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

pub(crate) fn default_symbol_embedder() -> Arc<dyn CodeEmbedder> {
    CodeEmbeddingConfig::from_env_or_hash(EMBEDDING_DIM)
        .and_then(|config| config.build())
        .unwrap_or_else(|error| Arc::new(FailingCodeEmbedder::new(error)))
}

fn coalesce_code_embed(_event: &MutationEvent) -> Option<String> {
    Some("code-kg-embed".to_string())
}

fn embed_handler(
    ctx: &mut HookContext,
    events: &[MutationEvent],
    embedder: &dyn CodeEmbedder,
) -> Result<HookOutcome, HookError> {
    ensure_embedding_designation_with_dim(ctx.store, embedder.dimension())?;

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
        let vector = embedder
            .embed_code(&text)
            .map_err(|error| HookError::new(format!("code embedding failed: {error}")))?;

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
    ensure_embedding_designation_with_dim(store, EMBEDDING_DIM)
}

pub(crate) fn ensure_embedding_designation_with_dim(
    store: &mut RedCoreGraphStore,
    dimension: usize,
) -> Result<(), HookError> {
    let already = store.vector_designations().into_iter().any(|designation| {
        designation.label == CODE_SYMBOL_LABEL && designation.property == EMBEDDING_PROPERTY
    });
    if !already {
        store
            .designate_vector_property(CODE_SYMBOL_LABEL, EMBEDDING_PROPERTY, dimension)
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

#[derive(Clone, Debug)]
struct FailingCodeEmbedder {
    message: String,
}

impl FailingCodeEmbedder {
    fn new(error: CodeEmbeddingError) -> Self {
        Self {
            message: error.to_string(),
        }
    }
}

impl CodeEmbedder for FailingCodeEmbedder {
    fn embed_code(&self, _text: &str) -> Result<Vec<f32>, CodeEmbeddingError> {
        Err(CodeEmbeddingError::Config(self.message.clone()))
    }

    fn dimension(&self) -> usize {
        EMBEDDING_DIM
    }

    fn name(&self) -> &str {
        "unconfigured"
    }
}
