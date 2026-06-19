//! Semantic-similarity edges over memory documents.
//!
//! Memory docs are otherwise edgeless for layout: only authored `MEMORY_RELATES`
//! wikilinks and lifecycle edges (`MEMORY_SUPERSEDES`, `DERIVED_FROM`, ...) connect
//! them, and most docs carry none. This builder gives the memory graph a navigable,
//! clusterable topology: it embeds filtered memory docs for a tenant and writes a
//! `MEMORY_SIMILAR` edge to each doc's top-k nearest neighbors by cosine similarity.
//! Those edges drive the native graph view in the Obsidian mirror and the dense
//! semantic galaxy in Theseus / Scene OS.
//!
//! The default embedder is deterministic, offline, and dependency-free (token-hash
//! bag-of-words). It carries no learned semantics; it proves the edge mechanism and
//! keeps the builder testable without a model server. A real SBERT / GL-Fusion
//! encoder swaps in behind [`MemoryEmbedder`] for meaningful clusters.
//!
//! The builder ranks and writes within a bounded, enumerated space (the tenant's
//! own docs); it never authors nodes, only `MEMORY_SIMILAR` edges keyed by a
//! deterministic id, so a re-run over the same doc set is idempotent.

use rustyred_thg_core::{now_ms, EdgeRecord, GraphStore, GraphStoreResult, NodeRecord};
use serde_json::json;

use crate::{
    memory_edge_id, memory_nodes, normalized_tenant_pair, prop_str, MEMORY_DOCUMENT_LABEL,
};

/// Edge type written between two semantically-close memory docs.
pub const MEMORY_SIMILAR: &str = "MEMORY_SIMILAR";
pub const DEFAULT_EXCLUDED_SIMILARITY_KINDS: &[&str] = &["community_summary", "orchestrate"];

/// Maps a doc's text to a fixed-dimension embedding. The default is [`HashEmbedder`];
/// a real model implements this trait to produce semantically-meaningful vectors.
pub trait MemoryEmbedder {
    fn embed(&self, text: &str) -> Vec<f32>;
}

/// Deterministic, offline, dependency-free embedder: signed token-hash bag-of-words
/// into a fixed-dim, L2-normalized vector. Shared tokens land on shared dimensions,
/// so word-overlap drives similarity. No learned semantics; the default and the
/// testable floor, not the production encoder.
#[derive(Clone, Copy, Debug)]
pub struct HashEmbedder {
    pub dim: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }
}

impl Default for HashEmbedder {
    fn default() -> Self {
        Self { dim: 64 }
    }
}

impl MemoryEmbedder for HashEmbedder {
    fn embed(&self, text: &str) -> Vec<f32> {
        let mut vec = vec![0.0f32; self.dim];
        for token in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let hash = fnv1a(token.to_lowercase().as_bytes());
            let idx = (hash as usize) % self.dim;
            // Signed hashing: a second hash bit picks the sign so distinct tokens
            // that collide on a dimension tend to cancel rather than reinforce.
            let sign = if (hash >> 17) & 1 == 0 { 1.0 } else { -1.0 };
            vec[idx] += sign;
        }
        l2_normalize(&mut vec);
        vec
    }
}

/// Knobs for the edge builder.
#[derive(Clone, Debug, PartialEq)]
pub struct SimilarityOptions {
    /// Max neighbors to link per doc (the k in kNN).
    pub k: usize,
    /// Minimum cosine similarity for an edge; prunes weak links.
    pub min_score: f64,
    /// Optional kind allowlist. Empty means every kind not denied is eligible.
    pub include_kinds: Vec<String>,
    /// Kind denylist. Exclude wins over include.
    pub exclude_kinds: Vec<String>,
}

impl Default for SimilarityOptions {
    fn default() -> Self {
        Self {
            k: 8,
            min_score: 0.15,
            include_kinds: Vec::new(),
            exclude_kinds: DEFAULT_EXCLUDED_SIMILARITY_KINDS
                .iter()
                .map(|kind| kind.to_string())
                .collect(),
        }
    }
}

/// What a run produced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimilarityStats {
    pub docs: usize,
    pub edges_written: usize,
}

/// Embed filtered memory docs for `tenant_slug` and write `MEMORY_SIMILAR` edges to
/// each doc's top-k nearest neighbors above `opts.min_score`. O(n^2) cosine, which
/// is fine at memory scale (hundreds of docs per tenant). Deterministic edge ids
/// make a re-run over the same doc set idempotent.
pub fn compute_memory_similarity_edges<S: GraphStore>(
    store: &mut S,
    tenant_slug: &str,
    embedder: &dyn MemoryEmbedder,
    opts: &SimilarityOptions,
) -> GraphStoreResult<SimilarityStats> {
    let tenant = normalized_tenant_pair("", tenant_slug);
    let nodes: Vec<NodeRecord> = memory_nodes(&*store, &tenant, false)?
        .into_iter()
        .filter(|node| {
            node.labels
                .iter()
                .any(|label| label == MEMORY_DOCUMENT_LABEL)
        })
        .filter(|node| kind_allowed(node, opts))
        .collect();
    let embedded: Vec<(String, Vec<f32>)> = nodes
        .iter()
        .map(|node| (node.id.clone(), embedder.embed(&memory_text(node))))
        .collect();

    let now = now_ms();
    let k = opts.k.max(1);
    let mut stats = SimilarityStats {
        docs: embedded.len(),
        edges_written: 0,
    };

    for (i, (from_id, from_vec)) in embedded.iter().enumerate() {
        let mut sims: Vec<(usize, f64)> = embedded
            .iter()
            .enumerate()
            .filter(|(j, _)| *j != i)
            .map(|(j, (_, other))| (j, cosine(from_vec, other)))
            .filter(|(_, score)| *score >= opts.min_score)
            .collect();
        // Strongest first; break ties by neighbor id for a stable, reproducible set.
        sims.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| embedded[a.0].0.cmp(&embedded[b.0].0))
        });
        sims.truncate(k);

        for (j, score) in sims {
            let to_id = &embedded[j].0;
            store.upsert_edge(EdgeRecord::new(
                memory_edge_id(&tenant, MEMORY_SIMILAR, from_id, to_id),
                from_id.clone(),
                MEMORY_SIMILAR,
                to_id.clone(),
                json!({
                    "tenant_id": tenant,
                    "tenant_slug": tenant,
                    "score": score,
                    "source": "memory_similarity",
                    "computed_at_ms": now,
                }),
            ))?;
            stats.edges_written += 1;
        }
    }

    Ok(stats)
}

/// The text the embedder sees for a doc: title, summary, and content joined.
fn memory_text(node: &NodeRecord) -> String {
    [
        prop_str(&node.properties, "title"),
        prop_str(&node.properties, "summary"),
        prop_str(&node.properties, "content"),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("\n")
}

fn kind_allowed(node: &NodeRecord, opts: &SimilarityOptions) -> bool {
    let kind = normalize_kind(prop_str(&node.properties, "kind").unwrap_or_default());
    if !opts.include_kinds.is_empty()
        && !opts
            .include_kinds
            .iter()
            .any(|candidate| normalize_kind(candidate) == kind)
    {
        return false;
    }
    !opts
        .exclude_kinds
        .iter()
        .any(|candidate| normalize_kind(candidate) == kind)
}

fn normalize_kind(kind: impl AsRef<str>) -> String {
    kind.as_ref().trim().to_lowercase()
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0f64;
    let mut na = 0.0f64;
    let mut nb = 0.0f64;
    let len = a.len().min(b.len());
    for i in 0..len {
        let (x, y) = (a[i] as f64, b[i] as f64);
        dot += x * y;
        na += x * x;
        nb += y * y;
    }
    if na == 0.0 || nb == 0.0 {
        0.0
    } else {
        dot / (na.sqrt() * nb.sqrt())
    }
}

fn l2_normalize(vec: &mut [f32]) {
    let norm = vec.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if norm > 0.0 {
        for x in vec.iter_mut() {
            *x = (*x as f64 / norm) as f32;
        }
    }
}

fn fnv1a(bytes: &[u8]) -> u32 {
    let mut hash = 2_166_136_261u32;
    for byte in bytes {
        hash ^= *byte as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}
