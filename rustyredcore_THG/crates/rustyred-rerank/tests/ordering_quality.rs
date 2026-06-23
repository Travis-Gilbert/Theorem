//! Acceptance #3: with `RerankScorer` enabled, ordering quality on a fixed
//! query set improves measurably against RRF-only ordering.
//!
//! This is the benchmark Travis asked to run on our own candidates rather than
//! vendor tables. The fixed set encodes a realistic failure mode of pure RRF:
//! a popular-but-irrelevant page wins the fan-out merge, while the lexically
//! on-topic page the user wanted is buried. The relevance-dominant web scorer
//! (`ArmWeights::web`) reorders by query relevance and recovers the gold order.
//! We measure nDCG over the fixed set and assert the reranker beats RRF.

use rustyred_membrane::{Candidate, ScoreContext, Scorer};
use rustyred_rerank::cross_encoder::LexicalCrossEncoder;
use rustyred_rerank::RerankScorer;

struct GoldCandidate {
    id: &'static str,
    text: &'static str,
    gold: f32,
    rrf_score: f32,
}

struct GoldQuery {
    query: &'static str,
    candidates: Vec<GoldCandidate>,
}

fn fixed_query_set() -> Vec<GoldQuery> {
    vec![
        GoldQuery {
            query: "rust async runtime tokio",
            candidates: vec![
                GoldCandidate {
                    id: "tokio",
                    text: "tokio asynchronous runtime for rust futures",
                    gold: 3.0,
                    rrf_score: 0.31,
                },
                GoldCandidate {
                    id: "js-loop",
                    text: "javascript promises and the browser event loop",
                    gold: 0.0,
                    rrf_score: 0.92,
                },
                GoldCandidate {
                    id: "executor",
                    text: "rust async executor task scheduling internals",
                    gold: 2.0,
                    rrf_score: 0.50,
                },
                GoldCandidate {
                    id: "py-thread",
                    text: "python threading and the global interpreter lock",
                    gold: 0.0,
                    rrf_score: 0.74,
                },
            ],
        },
        GoldQuery {
            query: "graph personalized pagerank seeds",
            candidates: vec![
                GoldCandidate {
                    id: "ppr",
                    text: "personalized pagerank push algorithm with seed mass",
                    gold: 3.0,
                    rrf_score: 0.28,
                },
                GoldCandidate {
                    id: "celebrity",
                    text: "celebrity gossip and entertainment news today",
                    gold: 0.0,
                    rrf_score: 0.95,
                },
                GoldCandidate {
                    id: "centrality",
                    text: "graph centrality and pagerank for ranking nodes",
                    gold: 2.0,
                    rrf_score: 0.55,
                },
                GoldCandidate {
                    id: "weather",
                    text: "weekly weather forecast and rainfall totals",
                    gold: 0.0,
                    rrf_score: 0.70,
                },
            ],
        },
    ]
}

/// Discounted cumulative gain at full depth, linear gain, log2 discount.
fn dcg(rels_in_order: &[f32]) -> f32 {
    rels_in_order
        .iter()
        .enumerate()
        .map(|(rank, rel)| rel / ((rank as f32 + 2.0).log2()))
        .sum()
}

fn ndcg(order: &[f32]) -> f32 {
    let ideal_dcg = {
        let mut ideal = order.to_vec();
        ideal.sort_by(|a, b| b.partial_cmp(a).unwrap());
        dcg(&ideal)
    };
    if ideal_dcg == 0.0 {
        0.0
    } else {
        dcg(order) / ideal_dcg
    }
}

#[test]
fn reranker_beats_rrf_only_ordering_on_fixed_set() {
    let scorer = RerankScorer::web(Box::new(LexicalCrossEncoder::new("bench-web")));
    let mut rrf_ndcg_sum = 0.0;
    let mut rerank_ndcg_sum = 0.0;
    let queries = fixed_query_set();
    let n = queries.len() as f32;

    for q in &queries {
        let active: Vec<String> = Vec::new();
        let ctx = ScoreContext::new(q.query, &active).without_redundancy();

        // RRF-only ordering: descending fan-out fusion score.
        let mut rrf_order: Vec<&GoldCandidate> = q.candidates.iter().collect();
        rrf_order.sort_by(|a, b| b.rrf_score.partial_cmp(&a.rrf_score).unwrap());
        let rrf_rels: Vec<f32> = rrf_order.iter().map(|c| c.gold).collect();

        // RerankScorer ordering: relevance-dominant graph-aware score.
        let mut rerank_order: Vec<&GoldCandidate> = q.candidates.iter().collect();
        rerank_order.sort_by(|a, b| {
            let sa = scorer.score(&Candidate::new(a.id, a.text, 8), &ctx);
            let sb = scorer.score(&Candidate::new(b.id, b.text, 8), &ctx);
            sb.partial_cmp(&sa).unwrap()
        });
        let rerank_rels: Vec<f32> = rerank_order.iter().map(|c| c.gold).collect();

        rrf_ndcg_sum += ndcg(&rrf_rels);
        rerank_ndcg_sum += ndcg(&rerank_rels);
    }

    let rrf_mean = rrf_ndcg_sum / n;
    let rerank_mean = rerank_ndcg_sum / n;

    eprintln!("mean nDCG  rrf-only={rrf_mean:.4}  rerank={rerank_mean:.4}");
    assert!(
        rerank_mean > rrf_mean + 0.10,
        "reranker must measurably beat RRF-only: rerank={rerank_mean:.4} rrf={rrf_mean:.4}"
    );
    // The reranker should surface the gold-best document first for each query.
    assert!(
        rerank_mean > 0.95,
        "reranker should approach ideal ordering on this set"
    );
}
