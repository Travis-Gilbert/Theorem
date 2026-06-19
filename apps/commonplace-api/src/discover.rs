//! Discovery: propose connections the user did not make (plan unit I4).
//!
//! Finds item pairs that are semantically similar but NOT yet joined by a
//! `SIMILAR_TO` edge, ranked by similarity. These are the latent connections the
//! F2 auto-linker missed (e.g. pairs that fell below its link threshold, or items
//! that became related only after later items were added). It is the inverse of
//! the I2 briefing's connectedness axis: surface the edges that should exist but
//! don't.
//!
//! Deterministic: for each embedded item it pulls nearest neighbors from the
//! engine index (via the F2 ingest pipeline's search over the item's own text),
//! drops self and already-linked pairs, keeps the strongest similarity per
//! unordered pair, and ranks.

use std::collections::{HashMap, HashSet};

use commonplace::{
    BlobStore, Commonplace, EmbeddingGraphStore, IngestPipeline, Item, ItemBody, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{GraphStoreResult, NeighborQuery};

/// A proposed connection between two not-yet-linked items.
#[derive(Clone, Debug)]
pub struct CandidateLink {
    pub a: Item,
    pub b: Item,
    pub similarity: f64,
    pub reason: String,
}

/// Tuning for discovery.
#[derive(Clone, Debug)]
pub struct DiscoverConfig {
    /// Minimum cosine similarity for a pair to be proposed.
    pub min_similarity: f64,
    /// Max candidate links returned.
    pub max_results: usize,
    /// Nearest neighbors examined per item.
    pub per_item_neighbors: usize,
}

impl Default for DiscoverConfig {
    fn default() -> Self {
        Self {
            min_similarity: 0.5,
            max_results: 20,
            per_item_neighbors: 10,
        }
    }
}

/// Propose ranked candidate links over the consumer store.
pub fn discover<S, B>(
    cp: &Commonplace<S, B>,
    config: &DiscoverConfig,
) -> GraphStoreResult<Vec<CandidateLink>>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let items = cp.all_items()?;

    // Already-connected unordered pairs, to exclude.
    let mut linked: HashSet<(String, String)> = HashSet::new();
    for item in &items {
        for direction in [
            NeighborQuery::out(&item.id).with_edge_type(SIMILAR_TO_EDGE),
            NeighborQuery::in_(&item.id).with_edge_type(SIMILAR_TO_EDGE),
        ] {
            for hit in cp.store().neighbors(direction) {
                linked.insert(ordered_pair(&item.id, &hit.node_id));
            }
        }
    }

    // Strongest similarity per candidate (unordered) pair.
    let pipeline = IngestPipeline::default();
    let mut candidates: HashMap<(String, String), f64> = HashMap::new();
    for item in &items {
        let query = item_query_text(item);
        if query.trim().is_empty() {
            continue;
        }
        let hits = pipeline.search(cp, &query, config.per_item_neighbors + 1)?;
        for (other_id, distance) in hits {
            if other_id == item.id {
                continue;
            }
            let similarity = 1.0 - distance as f64;
            if similarity < config.min_similarity {
                continue;
            }
            let pair = ordered_pair(&item.id, &other_id);
            if linked.contains(&pair) {
                continue;
            }
            let best = candidates.entry(pair).or_insert(0.0);
            if similarity > *best {
                *best = similarity;
            }
        }
    }

    let mut ranked: Vec<((String, String), f64)> = candidates.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(config.max_results);

    let mut links = Vec::with_capacity(ranked.len());
    for ((a_id, b_id), similarity) in ranked {
        if let (Some(a), Some(b)) = (cp.get_item(&a_id)?, cp.get_item(&b_id)?) {
            links.push(CandidateLink {
                a,
                b,
                similarity,
                reason: format!("semantically similar ({similarity:.2}) but not yet linked"),
            });
        }
    }
    Ok(links)
}

fn ordered_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_string(), b.to_string())
    } else {
        (b.to_string(), a.to_string())
    }
}

fn item_query_text(item: &Item) -> String {
    let mut text = item.title.clone();
    if let ItemBody::Inline { text: body } = &item.body {
        text.push(' ');
        text.push_str(body);
    }
    for tag in &item.tags {
        text.push(' ');
        text.push_str(tag);
    }
    text
}
