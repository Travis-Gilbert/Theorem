use std::collections::HashMap;

use rustyred_thg_core::{
    personalized_pagerank, GraphStore, GraphStoreResult, NeighborQuery, NodeQuery,
    RedCoreGraphStore,
};

use super::model::{UrlFingerprint, UrlNodeView, EDGE_LINKS_TO, LABEL_URL};

pub struct FrontierCtx<'a> {
    pub store: &'a RedCoreGraphStore,
    pub tenant: &'a str,
}

pub trait Prioritizer: Send + Sync {
    fn score(&self, ctx: &FrontierCtx<'_>, node: &UrlNodeView) -> f64;

    fn wants_recompute(&self, enqueued_since_last: u64) -> bool;

    fn recompute(&self, ctx: &FrontierCtx<'_>) -> GraphStoreResult<Vec<(UrlFingerprint, f64)>>;
}

#[derive(Clone, Debug)]
pub struct DepthPrioritizer {
    pub base: f64,
    pub depth_decay: f64,
}

impl Default for DepthPrioritizer {
    fn default() -> Self {
        Self {
            base: 1000.0,
            depth_decay: 10.0,
        }
    }
}

impl Prioritizer for DepthPrioritizer {
    fn score(&self, _ctx: &FrontierCtx<'_>, node: &UrlNodeView) -> f64 {
        self.base - (node.depth as f64 * self.depth_decay)
    }

    fn wants_recompute(&self, _enqueued_since_last: u64) -> bool {
        false
    }

    fn recompute(&self, _ctx: &FrontierCtx<'_>) -> GraphStoreResult<Vec<(UrlFingerprint, f64)>> {
        Ok(Vec::new())
    }
}

#[derive(Clone, Debug)]
pub struct PprPrioritizer {
    pub seeds: Vec<UrlFingerprint>,
    pub recompute_every: u64,
    pub alpha: f64,
    pub epsilon: f64,
    pub max_pushes: usize,
}

impl Default for PprPrioritizer {
    fn default() -> Self {
        Self {
            seeds: Vec::new(),
            recompute_every: 25,
            alpha: 0.15,
            epsilon: 1e-6,
            max_pushes: 100_000,
        }
    }
}

impl Prioritizer for PprPrioritizer {
    fn score(&self, ctx: &FrontierCtx<'_>, node: &UrlNodeView) -> f64 {
        let scores = self.recompute(ctx).unwrap_or_default();
        scores
            .into_iter()
            .find_map(|(fp, score)| (fp == node.fp).then_some(score))
            .unwrap_or_else(|| 1.0 / (node.depth as f64 + 1.0))
    }

    fn wants_recompute(&self, enqueued_since_last: u64) -> bool {
        self.recompute_every > 0 && enqueued_since_last >= self.recompute_every
    }

    fn recompute(&self, ctx: &FrontierCtx<'_>) -> GraphStoreResult<Vec<(UrlFingerprint, f64)>> {
        let nodes = GraphStore::query_nodes(ctx.store, NodeQuery::label(LABEL_URL));
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut adjacency = HashMap::new();
        for node in &nodes {
            let neighbors = GraphStore::neighbors(
                ctx.store,
                NeighborQuery::out(node.id.clone()).with_edge_type(EDGE_LINKS_TO),
            )
            .into_iter()
            .map(|hit| (hit.node_id, 1.0))
            .collect::<Vec<_>>();
            adjacency.insert(node.id.clone(), neighbors);
        }

        let mut seed_scores = HashMap::new();
        if self.seeds.is_empty() {
            let seed_mass = 1.0 / nodes.len().max(1) as f64;
            for node in &nodes {
                seed_scores.insert(node.id.clone(), seed_mass);
            }
        } else {
            let seed_mass = 1.0 / self.seeds.len().max(1) as f64;
            for seed in &self.seeds {
                seed_scores.insert(seed.to_hex(), seed_mass);
            }
        }

        let scores = personalized_pagerank(
            &adjacency,
            &seed_scores,
            self.alpha,
            self.epsilon,
            self.max_pushes,
        );
        Ok(scores
            .into_iter()
            .filter_map(|(id, score)| UrlFingerprint::from_hex(&id).map(|fp| (fp, score)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::frontier::model::{fingerprint, EDGE_LINKS_TO};
    use rustyred_thg_core::{EdgeRecord, NodeRecord, RedCoreGraphStore};

    #[test]
    fn ppr_prioritizer_ranks_central_node() {
        let a = fingerprint("GET", "https://example.com/a", b"");
        let b = fingerprint("GET", "https://example.com/b", b"");
        let c = fingerprint("GET", "https://example.com/c", b"");
        let mut store = RedCoreGraphStore::memory();
        for fp in [a, b, c] {
            store
                .upsert_node(NodeRecord::new(fp.to_hex(), [LABEL_URL], json!({})))
                .unwrap();
        }
        store
            .upsert_edge(EdgeRecord::new(
                "a-b",
                a.to_hex(),
                EDGE_LINKS_TO,
                b.to_hex(),
                json!({}),
            ))
            .unwrap();
        store
            .upsert_edge(EdgeRecord::new(
                "c-b",
                c.to_hex(),
                EDGE_LINKS_TO,
                b.to_hex(),
                json!({}),
            ))
            .unwrap();

        let prioritizer = PprPrioritizer {
            seeds: vec![a, c],
            ..Default::default()
        };
        let scores = prioritizer
            .recompute(&FrontierCtx {
                store: &store,
                tenant: "test",
            })
            .unwrap()
            .into_iter()
            .collect::<HashMap<_, _>>();
        assert!(scores[&b] > 0.0);
    }
}
