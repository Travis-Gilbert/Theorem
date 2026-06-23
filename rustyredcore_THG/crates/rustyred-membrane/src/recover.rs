//! Graph-backed lossless overflow recovery for the membrane gate.
//!
//! [`fill_to_budget`](crate::fill_to_budget) is pure: it returns [`Handle`]s for
//! the deferred candidates but never touches a store. This module adds the
//! store-coupled half SPEC-CONTEXT-MEMBRANE-1.0 requires: deferred overflow is
//! persisted byte-exact as graph-resident `DeferredContext` nodes,
//! content-addressed by the same blake3 digest the `Handle` already carries, so
//! a handle resolves to its bytes with no side table. [`context_fetch`] reads
//! them back and verifies integrity by re-hashing. Nothing is summarized or
//! dropped at the gate; the overflow is recoverable, which is what keeps the
//! Ariadne recovery property intact across the membrane and compaction specs.
//!
//! This is the `context_fetch` seam the spec describes as belonging to the
//! compaction spec. That crate is not yet in-tree, so the membrane provides the
//! single shared recovery implementation here; a future compaction `page_back`
//! recovers through the same function rather than carrying its own.
//!
//! Behind the optional `graph-store` feature so the default membrane stays a
//! pure cache-mechanics crate with no graph dependency.

use std::collections::HashMap;

use rustyred_thg_core::{GraphStore, GraphStoreResult, NodeRecord};
use serde_json::json;

use crate::gate::{fill_to_budget, Admission, Handle};
use crate::receipt::MembraneReceipt;
use crate::scorer::{Candidate, ScoreContext, Scorer};

/// Label for graph-resident deferred-context nodes.
pub const DEFERRED_CONTEXT_LABEL: &str = "DeferredContext";
/// Label for content-addressed membrane receipt nodes.
pub const MEMBRANE_RECEIPT_LABEL: &str = "MembraneReceipt";
/// Property holding the byte-exact realized text of a deferred candidate.
pub const DEFERRED_TEXT_PROPERTY: &str = "membrane_text";

/// Content-addressed id for a deferred-context node. The digest is the same
/// blake3 hex the gate stamps on every [`Handle`], so a handle resolves to its
/// bytes directly.
pub fn deferred_node_id(digest: &str) -> String {
    format!("membrane:deferred:{digest}")
}

fn text_digest(text: &str) -> String {
    blake3::hash(text.as_bytes()).to_hex().to_string()
}

/// Persist one candidate's realized text as a graph-resident node, addressed by
/// the digest of its bytes. Idempotent: re-persisting identical bytes upserts
/// the same content-addressed id.
pub fn persist_deferred<S: GraphStore>(
    store: &mut S,
    candidate: &Candidate,
) -> GraphStoreResult<Handle> {
    let handle = Handle::from_candidate(candidate);
    let node = NodeRecord::new(
        deferred_node_id(&handle.digest),
        [DEFERRED_CONTEXT_LABEL],
        json!({
            "digest": handle.digest,
            "source_node_id": candidate.node_id,
            "token_count": candidate.token_count,
            DEFERRED_TEXT_PROPERTY: candidate.text,
        }),
    );
    store.upsert_node(node)?;
    Ok(handle)
}

/// Store-backed admission. Runs the pure [`fill_to_budget`], then persists the
/// bytes of every deferred candidate so each returned `Handle` is recoverable
/// byte-exact through [`context_fetch`]. Overflow is never dropped or
/// summarized. The admitted set and ordering are identical to the pure fill.
pub fn admit_to_budget<S: GraphStore>(
    store: &mut S,
    candidates: Vec<Candidate>,
    scorer: &dyn Scorer,
    ctx: &ScoreContext<'_>,
    budget_tokens: usize,
) -> GraphStoreResult<Admission> {
    let lookup: HashMap<String, Candidate> = candidates
        .iter()
        .cloned()
        .map(|candidate| (text_digest(&candidate.text), candidate))
        .collect();
    let admission = fill_to_budget(candidates, scorer, ctx, budget_tokens);
    for handle in &admission.deferred {
        if let Some(candidate) = lookup.get(&handle.digest) {
            persist_deferred(store, candidate)?;
        }
    }
    Ok(admission)
}

/// Recover the exact bytes behind a deferred [`Handle`]. Returns `Some(text)`
/// only when the stored bytes re-hash to the handle digest (byte-exact
/// integrity check); `None` if the node is missing or the digest disagrees.
pub fn context_fetch<S: GraphStore>(store: &S, handle: &Handle) -> Option<String> {
    let node = store.get_node(&deferred_node_id(&handle.digest))?;
    let text = node
        .properties
        .get(DEFERRED_TEXT_PROPERTY)
        .and_then(|value| value.as_str())?;
    if text_digest(text) == handle.digest {
        Some(text.to_string())
    } else {
        None
    }
}

/// Emit a content-addressed [`MembraneReceipt`] node per gate invocation. The
/// node id is derived from the receipt content address, so identical receipts
/// collapse and instrumentation stays auditable.
pub fn emit_receipt<S: GraphStore>(
    store: &mut S,
    receipt: &MembraneReceipt,
) -> GraphStoreResult<String> {
    let address = receipt.content_address();
    let node = NodeRecord::new(
        format!("membrane:receipt:{address}"),
        [MEMBRANE_RECEIPT_LABEL],
        serde_json::to_value(receipt).unwrap_or_else(|_| json!({})),
    );
    store.upsert_node(node)?;
    Ok(address)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receipt::Source;
    use crate::scorer::Candidate;
    use rustyred_thg_core::InMemoryGraphStore;

    #[derive(Clone, Copy)]
    struct ProximityScorer;
    impl Scorer for ProximityScorer {
        fn score(&self, c: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
            c.ppr_proximity
        }
    }

    fn candidate(id: &str, body: &str, score: f32, tokens: usize) -> Candidate {
        let mut candidate = Candidate::new(id, body, tokens);
        candidate.ppr_proximity = score;
        candidate
    }

    #[test]
    fn deferred_overflow_is_recoverable_byte_exact() {
        let mut store = InMemoryGraphStore::new();
        let admitted_text = "the admitted passage stays in the window";
        let deferred_text = "the deferred passage -- bytes: \"quotes\", \n newlines";
        let candidates = vec![
            candidate("keep", admitted_text, 0.9, 5),
            candidate("overflow", deferred_text, 0.1, 9),
        ];
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).without_redundancy();

        let admission = admit_to_budget(&mut store, candidates, &ProximityScorer, &ctx, 6).unwrap();

        assert_eq!(admission.admitted.len(), 1);
        assert_eq!(admission.admitted[0].node_id, "keep");
        assert_eq!(admission.deferred.len(), 1);

        let recovered =
            context_fetch(&store, &admission.deferred[0]).expect("deferred handle must recover");
        assert_eq!(recovered, deferred_text, "recovery must be byte-exact");
    }

    #[test]
    fn context_fetch_rejects_a_tampered_digest() {
        let mut store = InMemoryGraphStore::new();
        let candidates = vec![candidate("only", "some deferred body", 0.1, 50)];
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active).without_redundancy();
        let admission = admit_to_budget(&mut store, candidates, &ProximityScorer, &ctx, 4).unwrap();

        let mut tampered = admission.deferred[0].clone();
        tampered.digest = "0".repeat(64);
        assert!(context_fetch(&store, &tampered).is_none());
    }

    #[test]
    fn receipt_emits_a_content_addressed_node() {
        let mut store = InMemoryGraphStore::new();
        let receipt = MembraneReceipt {
            source: Source::Web,
            candidates_scored: 12,
            tokens_admitted: 800,
            tokens_deferred: 4200,
            reranker_version: "lexical-cross-encoder:membrane-v1".to_string(),
            task_token_delta_vs_baseline: Some(4200),
        };
        let address = emit_receipt(&mut store, &receipt).unwrap();
        let node = store.get_node(&format!("membrane:receipt:{address}"));
        assert!(node.is_some());
    }
}
