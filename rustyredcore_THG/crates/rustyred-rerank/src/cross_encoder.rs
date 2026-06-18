use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use reqwest::blocking::Client;
use rustyred_membrane::{Candidate, ScoreContext, Scorer};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const GTE_RERANKER_MODERNBERT_BASE: &str = "Alibaba-NLP/gte-reranker-modernbert-base";
pub const BGE_RERANKER_V2_M3: &str = "BAAI/bge-reranker-v2-m3";
pub const JINA_RERANKER_V3: &str = "jinaai/jina-reranker-v3";
const DEFAULT_HTTP_TIMEOUT_SECONDS: u64 = 8;

pub trait CrossEncoder: Send + Sync {
    fn model_id(&self) -> &str;
    fn score(&self, query: &str, text: &str) -> f32;
}

/// Configuration record for the intended v1 model family. Runtime bindings can
/// map this to ONNX, Candle, a local HTTP model server, or another executor.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceClassificationModel {
    pub model_id: String,
    pub max_length: usize,
}

impl SequenceClassificationModel {
    pub fn gte_modernbert_base() -> Self {
        Self {
            model_id: GTE_RERANKER_MODERNBERT_BASE.to_string(),
            max_length: 8192,
        }
    }

    pub fn bge_v2_m3() -> Self {
        Self {
            model_id: BGE_RERANKER_V2_M3.to_string(),
            max_length: 8192,
        }
    }
}

/// Deterministic fallback used in tests and offline builds. It is deliberately
/// lexical, not a pretend model.
#[derive(Clone, Debug)]
pub struct LexicalCrossEncoder {
    model_id: String,
}

impl LexicalCrossEncoder {
    pub fn new(model_id: impl Into<String>) -> Self {
        Self {
            model_id: model_id.into(),
        }
    }
}

impl Default for LexicalCrossEncoder {
    fn default() -> Self {
        Self::new("lexical-cross-encoder")
    }
}

impl CrossEncoder for LexicalCrossEncoder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn score(&self, query: &str, text: &str) -> f32 {
        lexical_overlap(query, text)
    }
}

/// HTTP-backed single-forward-pass SequenceClassification reranker.
///
/// The endpoint shape intentionally accepts the small local service in
/// `infra/railway/reranker-service` and common rerank APIs:
/// `{query,text,texts,model}` in, and either `{score}`, `{scores:[...]}`, or
/// `[{index,score}]` out. Failures return `0.0` so the hot path degrades to the
/// graph/PPR terms instead of failing the whole gate.
#[derive(Clone, Debug)]
pub struct HttpCrossEncoder {
    endpoint: String,
    model_id: String,
    client: Client,
}

impl HttpCrossEncoder {
    pub fn new(endpoint: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model_id: model_id.into(),
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS))
                .build()
                .expect("reqwest blocking client"),
        }
    }

    fn score_one(&self, query: &str, text: &str) -> Option<f32> {
        let payload = json!({
            "query": query,
            "text": text,
            "texts": [text],
            "model": self.model_id,
        });
        let response = self
            .client
            .post(&self.endpoint)
            .json(&payload)
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .json::<Value>()
            .ok()?;
        parse_score_response(&response)
    }
}

impl CrossEncoder for HttpCrossEncoder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn score(&self, query: &str, text: &str) -> f32 {
        self.score_one(query, text).unwrap_or(0.0)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelBenchmark {
    pub model_id: String,
    /// Lower is better. This is measured on this repo's candidate set, not a
    /// vendor leaderboard.
    pub mean_latency_ms: f32,
    /// Higher is better. Any fixed query-set quality metric can ride here.
    pub quality: f32,
    pub parameters_millions: Option<f32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct BenchmarkLedger {
    pub candidates: Vec<ModelBenchmark>,
}

impl BenchmarkLedger {
    pub fn winner(&self) -> Option<&ModelBenchmark> {
        self.candidates.iter().max_by(|left, right| {
            let left_score = benchmark_score(left);
            let right_score = benchmark_score(right);
            left_score
                .partial_cmp(&right_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
    }
}

pub fn select_small_cpu_sequence_classifier(
    ledger: &BenchmarkLedger,
) -> Option<SequenceClassificationModel> {
    let winner = ledger.winner()?;
    match winner.model_id.as_str() {
        GTE_RERANKER_MODERNBERT_BASE => Some(SequenceClassificationModel::gte_modernbert_base()),
        BGE_RERANKER_V2_M3 => Some(SequenceClassificationModel::bge_v2_m3()),
        _ => Some(SequenceClassificationModel {
            model_id: winner.model_id.clone(),
            max_length: 8192,
        }),
    }
}

fn benchmark_score(row: &ModelBenchmark) -> f32 {
    let latency_penalty = (row.mean_latency_ms.max(1.0)).ln() * 0.03;
    let size_penalty = row
        .parameters_millions
        .map(|params| (params.max(1.0)).ln() * 0.01)
        .unwrap_or(0.0);
    row.quality - latency_penalty - size_penalty
}

/// Candidate-set reranker seam for the final web arm. jina-v3 listwise belongs
/// here, after candidate generation, not in the per-candidate gate scorer.
pub trait ListwiseReranker: Send + Sync {
    fn model_id(&self) -> &str;
    fn rerank(
        &self,
        candidates: Vec<Candidate>,
        scorer: &dyn Scorer,
        ctx: &ScoreContext<'_>,
    ) -> Vec<Candidate>;
}

/// HTTP candidate-set reranker. The deployed jina-v3 service uses the same
/// request shape as [`HttpCrossEncoder`] but returns an ordering over the whole
/// candidate set so the web arm can stamp a diversity-aware rank before the
/// budget gate.
#[derive(Clone, Debug)]
pub struct HttpListwiseReranker {
    endpoint: String,
    model_id: String,
    client: Client,
}

impl HttpListwiseReranker {
    pub fn new(endpoint: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            model_id: model_id.into(),
            client: Client::builder()
                .timeout(Duration::from_secs(DEFAULT_HTTP_TIMEOUT_SECONDS))
                .build()
                .expect("reqwest blocking client"),
        }
    }

    fn scores_for(&self, query: &str, candidates: &[Candidate]) -> Option<BTreeMap<usize, f32>> {
        let texts: Vec<&str> = candidates
            .iter()
            .map(|candidate| candidate.text.as_str())
            .collect();
        let payload = json!({
            "query": query,
            "texts": texts,
            "model": self.model_id,
        });
        let response = self
            .client
            .post(&self.endpoint)
            .json(&payload)
            .send()
            .ok()?
            .error_for_status()
            .ok()?
            .json::<Value>()
            .ok()?;
        let scores = parse_rerank_scores(&response);
        if scores.is_empty() {
            None
        } else {
            Some(scores)
        }
    }
}

impl ListwiseReranker for HttpListwiseReranker {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn rerank(
        &self,
        candidates: Vec<Candidate>,
        scorer: &dyn Scorer,
        ctx: &ScoreContext<'_>,
    ) -> Vec<Candidate> {
        let Some(scores) = self.scores_for(ctx.query, &candidates) else {
            return NoopListwiseReranker.rerank(candidates, scorer, ctx);
        };
        let mut indexed: Vec<(usize, Candidate)> = candidates.into_iter().enumerate().collect();
        indexed.sort_by(|(a_index, a), (b_index, b)| {
            let a_score = scores
                .get(a_index)
                .copied()
                .unwrap_or_else(|| scorer.score(a, ctx));
            let b_score = scores
                .get(b_index)
                .copied()
                .unwrap_or_else(|| scorer.score(b, ctx));
            b_score
                .partial_cmp(&a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        indexed
            .into_iter()
            .map(|(_index, candidate)| candidate)
            .collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct NoopListwiseReranker;

impl ListwiseReranker for NoopListwiseReranker {
    fn model_id(&self) -> &str {
        "noop-listwise"
    }

    fn rerank(
        &self,
        mut candidates: Vec<Candidate>,
        scorer: &dyn Scorer,
        ctx: &ScoreContext<'_>,
    ) -> Vec<Candidate> {
        candidates.sort_by(|a, b| {
            scorer
                .score(b, ctx)
                .partial_cmp(&scorer.score(a, ctx))
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.node_id.cmp(&b.node_id))
        });
        candidates
    }
}

fn lexical_overlap(query: &str, text: &str) -> f32 {
    let query_terms = terms(query);
    if query_terms.is_empty() {
        return 0.0;
    }
    let text_terms = terms(text);
    if text_terms.is_empty() {
        return 0.0;
    }
    let matches = query_terms.intersection(&text_terms).count() as f32;
    (matches / query_terms.len() as f32).clamp(0.0, 1.0)
}

fn parse_score_response(value: &Value) -> Option<f32> {
    value
        .get("score")
        .and_then(Value::as_f64)
        .or_else(|| {
            value
                .get("scores")
                .and_then(Value::as_array)
                .and_then(|scores| scores.first())
                .and_then(score_value)
        })
        .or_else(|| {
            value
                .get("data")
                .and_then(Value::as_array)
                .and_then(|items| items.first())
                .and_then(|item| item.get("score"))
                .and_then(Value::as_f64)
        })
        .or_else(|| {
            value
                .as_array()
                .and_then(|items| items.first())
                .and_then(|item| item.get("score"))
                .and_then(Value::as_f64)
        })
        .map(normalize_model_score)
}

fn parse_rerank_scores(value: &Value) -> BTreeMap<usize, f32> {
    if let Some(scores) = value.get("scores").and_then(Value::as_array) {
        return scores
            .iter()
            .enumerate()
            .filter_map(|(index, score)| score_value(score).map(|score| (index, score as f32)))
            .collect();
    }

    let rows = value
        .get("results")
        .or_else(|| value.get("data"))
        .unwrap_or(value);
    rows.as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let index = row
                .get("index")
                .or_else(|| row.get("document_index"))
                .and_then(Value::as_u64)? as usize;
            let score = row.get("score").and_then(score_value)? as f32;
            Some((index, score))
        })
        .collect()
}

fn score_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.get("score").and_then(Value::as_f64))
}

fn normalize_model_score(score: f64) -> f32 {
    if (0.0..=1.0).contains(&score) {
        score as f32
    } else {
        (1.0 / (1.0 + (-score).exp())) as f32
    }
}

fn terms(text: &str) -> BTreeSet<String> {
    text.to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() > 1)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn benchmark_ledger_prefers_fast_high_quality_small_cpu_model() {
        let ledger = BenchmarkLedger {
            candidates: vec![
                ModelBenchmark {
                    model_id: "Qwen/Qwen3-Reranker-1.2B".to_string(),
                    mean_latency_ms: 240.0,
                    quality: 0.90,
                    parameters_millions: Some(1200.0),
                },
                ModelBenchmark {
                    model_id: GTE_RERANKER_MODERNBERT_BASE.to_string(),
                    mean_latency_ms: 38.0,
                    quality: 0.89,
                    parameters_millions: Some(149.0),
                },
            ],
        };

        let selected = select_small_cpu_sequence_classifier(&ledger).unwrap();
        assert_eq!(selected.model_id, GTE_RERANKER_MODERNBERT_BASE);
    }

    #[test]
    fn noop_listwise_reranker_orders_by_supplied_scorer() {
        #[derive(Clone, Copy)]
        struct ScoreByPpr;

        impl Scorer for ScoreByPpr {
            fn score(&self, c: &Candidate, _ctx: &ScoreContext<'_>) -> f32 {
                c.ppr_proximity
            }
        }

        let mut low = Candidate::new("low", "low", 1);
        low.ppr_proximity = 0.1;
        let mut high = Candidate::new("high", "high", 1);
        high.ppr_proximity = 0.8;
        let active = Vec::new();
        let ctx = ScoreContext::new("query", &active);

        let reranked = NoopListwiseReranker.rerank(vec![low, high], &ScoreByPpr, &ctx);

        assert_eq!(reranked[0].node_id, "high");
    }
}
