//! Acceptance #2 (mechanism benchmark): the warm-centrality prior the
//! IncrementalCentralityHook materializes is what `context_pack` reads at prompt
//! time instead of running cold global PPR. The hook does the PPR work *off the
//! prompt path* (background, coalesced); at prompt time the cost collapses to
//! property reads. This times both prompt-time paths over a synthetic
//! monorepo-scale code graph.
//!
//! The warm prior is written directly here (untimed) to model "the background
//! hook already ran" - the hook's own correctness is covered by
//! `code_kg_hooks.rs`. This test isolates the prompt-time latency premise.
//!
//! `#[ignore]`d so timing never gates CI; run on demand:
//!   cargo test -p rustyred-thg-code --test centrality_latency -- --ignored --nocapture

use std::collections::HashMap;
use std::time::Instant;

use rustyred_thg_code::CENTRALITY_PROPERTY;
use rustyred_thg_core::{
    personalized_pagerank, Direction, EdgeRecord, GraphMutation, GraphMutationBatch, NeighborQuery,
    NodeRecord, RedCoreGraphStore,
};
use serde_json::json;

const N_SYMBOLS: usize = 3_000;
const FANOUT: usize = 4;

// One batched commit: RedCore clones the store once per `commit_batch`, so bulk
// inserts must batch (15k individual upserts would be O(N^2) cloning).
fn build_call_graph(store: &mut RedCoreGraphStore) {
    let mut mutations: Vec<GraphMutation> = Vec::new();
    for i in 0..N_SYMBOLS {
        mutations.push(GraphMutation::NodeUpsert(NodeRecord::new(
            format!("code:symbol:{i}"),
            ["CodeSymbol"],
            json!({ "repo_id": "repo:bench", "name": format!("sym_{i}") }),
        )));
    }
    for i in 0..N_SYMBOLS {
        for k in 1..=FANOUT {
            let target = (i + k * 7) % N_SYMBOLS;
            if target == i {
                continue;
            }
            mutations.push(GraphMutation::EdgeUpsert(EdgeRecord::new(
                format!("code:edge:{i}-{target}"),
                format!("code:symbol:{i}"),
                "CALLS_SYMBOL",
                format!("code:symbol:{target}"),
                json!({}),
            )));
        }
    }
    store
        .commit_batch(GraphMutationBatch::new(mutations))
        .unwrap();
}

fn repo_adjacency(store: &RedCoreGraphStore) -> HashMap<String, Vec<(String, f64)>> {
    let mut adjacency = HashMap::new();
    for i in 0..N_SYMBOLS {
        let id = format!("code:symbol:{i}");
        let outs = store
            .neighbors(NeighborQuery {
                node_id: id.clone(),
                direction: Direction::Out,
                edge_type: Some("CALLS_SYMBOL".to_string()),
                include_expired: false,
            })
            .unwrap()
            .into_iter()
            .map(|hit| (hit.node_id, 1.0))
            .collect();
        adjacency.insert(id, outs);
    }
    adjacency
}

#[test]
#[ignore = "timing benchmark; run with --ignored --nocapture"]
fn warm_centrality_prior_beats_cold_ppr() {
    let seeds: Vec<String> = (0..5).map(|i| format!("code:symbol:{i}")).collect();

    let mut store = RedCoreGraphStore::memory();
    build_call_graph(&mut store);

    // --- Cold prompt-time path: build adjacency + global PPR every prompt. ---
    let t0 = Instant::now();
    let adjacency = repo_adjacency(&store);
    let seed_map: HashMap<String, f64> = seeds.iter().map(|s| (s.clone(), 1.0)).collect();
    let cold_scores = personalized_pagerank(&adjacency, &seed_map, 0.15, 1e-4, 1_000_000);
    let cold_elapsed = t0.elapsed();

    // --- Background (untimed): the hook warmed `centrality` once, off-path. ---
    let warm: HashMap<String, f64> = {
        let adjacency = repo_adjacency(&store);
        let all_seeds: HashMap<String, f64> = (0..N_SYMBOLS)
            .map(|i| (format!("code:symbol:{i}"), 1.0))
            .collect();
        personalized_pagerank(&adjacency, &all_seeds, 0.15, 1e-4, 5_000_000)
    };
    let mut warm_writes: Vec<GraphMutation> = Vec::new();
    for (id, score) in &warm {
        if let Some(mut node) = store.get_node(id).unwrap() {
            node.properties
                .as_object_mut()
                .unwrap()
                .insert(CENTRALITY_PROPERTY.to_string(), json!(*score));
            warm_writes.push(GraphMutation::NodeUpsert(node));
        }
    }
    store
        .commit_batch(GraphMutationBatch::new(warm_writes))
        .unwrap();

    // --- Warm prompt-time path: read the prior off the seeds. ---
    let t1 = Instant::now();
    let mut warm_scores: HashMap<String, f64> = HashMap::new();
    for seed in &seeds {
        if let Some(node) = store.get_node(seed).unwrap() {
            if let Some(c) = node
                .properties
                .get(CENTRALITY_PROPERTY)
                .and_then(|v| v.as_f64())
            {
                warm_scores.insert(seed.clone(), c);
            }
        }
    }
    let warm_elapsed = t1.elapsed();

    eprintln!(
        "[centrality bench] {N_SYMBOLS} symbols, fanout {FANOUT}\n  \
         cold prompt-time global PPR: {cold_elapsed:?}\n  \
         warm prompt-time prior read: {warm_elapsed:?}\n  \
         speedup: {:.0}x",
        cold_elapsed.as_secs_f64() / warm_elapsed.as_secs_f64().max(1e-9)
    );

    assert!(!cold_scores.is_empty(), "cold PPR produced a ranking");
    assert_eq!(
        warm_scores.len(),
        seeds.len(),
        "warm prior present for all seeds"
    );
    assert!(
        warm_elapsed < cold_elapsed,
        "warm prior read ({warm_elapsed:?}) should beat cold global PPR ({cold_elapsed:?})"
    );
}
