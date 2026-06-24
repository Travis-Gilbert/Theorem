//! Deterministic fixture benchmarks for the ColPali-style multi-vector tier.
//!
//! These are not a replacement for model-weight benchmarks. They give the repo a
//! repeatable oracle for the Vespa-inspired binary projection: exact MaxSim
//! remains the truth set, binary Hamming is measured as the hot candidate tier,
//! and storage rows make the f32/f16/binary blowup visible.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use rustyred_thg_core::ThgResult;

use crate::multivector::{
    quantize_sign_bits, rank_binary_hamming_maxsim, rank_exact_maxsim, recall_against_exact_top_k,
    storage_costs, BinaryMultiVectorSet, MaxSimAggregation, MultiVectorEmbeddingSet,
    MultiVectorStorageCost,
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorBenchmarkConfig {
    pub corpus_size: usize,
    pub query_count: usize,
    pub vectors_per_document: usize,
    pub dim: usize,
    pub exact_top_k: usize,
    pub candidate_top_ks: Vec<usize>,
}

impl Default for MultiVectorBenchmarkConfig {
    fn default() -> Self {
        Self {
            corpus_size: 96,
            query_count: 12,
            vectors_per_document: 32,
            dim: 128,
            exact_top_k: 10,
            candidate_top_ks: vec![10, 20, 40, 80],
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MultiVectorBenchmarkRow {
    pub candidate_top_k: usize,
    pub exact_top_k: usize,
    pub recall: f32,
    pub overlap_count: usize,
    pub missing_count: usize,
    pub exact_elapsed_ms: f64,
    pub binary_elapsed_ms: f64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorBackendRow {
    pub backend: String,
    pub status: String,
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MultiVectorBenchmarkReport {
    pub config: MultiVectorBenchmarkConfig,
    pub recall_rows: Vec<MultiVectorBenchmarkRow>,
    pub storage_cost: MultiVectorStorageCost,
    pub backend_rows: Vec<MultiVectorBackendRow>,
}

pub fn run_fixture_benchmark(
    config: MultiVectorBenchmarkConfig,
) -> ThgResult<MultiVectorBenchmarkReport> {
    let config = normalize_config(config);
    let documents = fixture_documents(&config)?;
    let binary_documents = documents
        .iter()
        .map(quantize_sign_bits)
        .collect::<ThgResult<Vec<_>>>()?;
    let queries = fixture_queries(&config, &documents)?;
    let binary_queries = queries
        .iter()
        .map(quantize_sign_bits)
        .collect::<ThgResult<Vec<_>>>()?;

    let mut recall_rows = Vec::new();
    for candidate_top_k in &config.candidate_top_ks {
        let mut overlap_count = 0usize;
        let mut exact_count = 0usize;
        let mut missing_count = 0usize;
        let mut exact_elapsed_ms = 0.0;
        let mut binary_elapsed_ms = 0.0;

        for (query, binary_query) in queries.iter().zip(binary_queries.iter()) {
            let exact_start = Instant::now();
            let exact = rank_exact_maxsim(
                &query.vectors,
                &documents,
                MaxSimAggregation::Mean,
                config.exact_top_k,
            )?;
            exact_elapsed_ms += elapsed_ms(exact_start);

            let binary_start = Instant::now();
            let binary = rank_binary_hamming_maxsim(
                binary_query,
                &binary_documents,
                MaxSimAggregation::Mean,
                *candidate_top_k,
            )?;
            binary_elapsed_ms += elapsed_ms(binary_start);

            let report =
                recall_against_exact_top_k(&exact, &binary, config.exact_top_k, *candidate_top_k);
            overlap_count += report.overlap_count;
            exact_count += report.exact_count;
            missing_count += report.missing_content_ids.len();
        }

        recall_rows.push(MultiVectorBenchmarkRow {
            candidate_top_k: *candidate_top_k,
            exact_top_k: config.exact_top_k,
            recall: if exact_count == 0 {
                0.0
            } else {
                overlap_count as f32 / exact_count as f32
            },
            overlap_count,
            missing_count,
            exact_elapsed_ms,
            binary_elapsed_ms,
        });
    }

    Ok(MultiVectorBenchmarkReport {
        storage_cost: storage_costs(config.vectors_per_document, config.dim),
        backend_rows: backend_rows(),
        config,
        recall_rows,
    })
}

fn normalize_config(mut config: MultiVectorBenchmarkConfig) -> MultiVectorBenchmarkConfig {
    config.corpus_size = config.corpus_size.max(1);
    config.query_count = config.query_count.max(1).min(config.corpus_size);
    config.vectors_per_document = config.vectors_per_document.max(1);
    config.dim = config.dim.max(1);
    config.exact_top_k = config.exact_top_k.max(1).min(config.corpus_size);
    if config.candidate_top_ks.is_empty() {
        config.candidate_top_ks = vec![config.exact_top_k];
    }
    config.candidate_top_ks.sort_unstable();
    config.candidate_top_ks.dedup();
    for candidate_top_k in &mut config.candidate_top_ks {
        *candidate_top_k = (*candidate_top_k).max(1).min(config.corpus_size);
    }
    config
}

fn fixture_documents(
    config: &MultiVectorBenchmarkConfig,
) -> ThgResult<Vec<MultiVectorEmbeddingSet>> {
    let mut documents = Vec::with_capacity(config.corpus_size);
    for doc_idx in 0..config.corpus_size {
        let vectors = (0..config.vectors_per_document)
            .map(|vector_idx| fixture_vector(doc_idx, vector_idx, config.dim, 0))
            .collect::<Vec<_>>();
        let set = MultiVectorEmbeddingSet {
            embedding_set_id: format!("mv:fixture:{doc_idx:04}"),
            content_id: format!("doc:fixture:{doc_idx:04}"),
            model_id: "colpali-fixture-benchmark".to_string(),
            model_version: "deterministic-v1".to_string(),
            vectors,
        };
        set.dim()?;
        documents.push(set);
    }
    Ok(documents)
}

fn fixture_queries(
    config: &MultiVectorBenchmarkConfig,
    documents: &[MultiVectorEmbeddingSet],
) -> ThgResult<Vec<MultiVectorEmbeddingSet>> {
    let mut queries = Vec::with_capacity(config.query_count);
    for query_idx in 0..config.query_count {
        let doc_idx = query_idx * config.corpus_size / config.query_count;
        let source = &documents[doc_idx];
        let mut vectors = source.vectors.iter().take(8).cloned().collect::<Vec<_>>();
        for (vector_idx, vector) in vectors.iter_mut().enumerate() {
            let jitter = fixture_vector(doc_idx, vector_idx, config.dim, 17);
            for (value, noise) in vector.iter_mut().zip(jitter) {
                *value = (*value * 0.94) + (noise * 0.06);
            }
            l2_normalize(vector);
        }
        let set = MultiVectorEmbeddingSet {
            embedding_set_id: format!("mv:query:{query_idx:04}"),
            content_id: format!("query:{query_idx:04}"),
            model_id: source.model_id.clone(),
            model_version: source.model_version.clone(),
            vectors,
        };
        set.dim()?;
        queries.push(set);
    }
    Ok(queries)
}

fn fixture_vector(doc_idx: usize, vector_idx: usize, dim: usize, salt: usize) -> Vec<f32> {
    let mut vector = Vec::with_capacity(dim);
    for dim_idx in 0..dim {
        let raw = splitmix64(
            ((doc_idx as u64) << 32)
                ^ ((vector_idx as u64) << 16)
                ^ (dim_idx as u64)
                ^ ((salt as u64) << 48),
        );
        let magnitude = ((raw & 0xffff) as f32 / 65_535.0) * 2.0 - 1.0;
        let harmonic = (((doc_idx + 1) * (dim_idx + 3) + vector_idx + salt) % 29) as f32 / 29.0;
        vector.push((magnitude * 0.7) + ((harmonic * 2.0 - 1.0) * 0.3));
    }
    l2_normalize(&mut vector);
    vector
}

fn splitmix64(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = x;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn l2_normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    }
}

fn backend_rows() -> Vec<MultiVectorBackendRow> {
    vec![
        MultiVectorBackendRow {
            backend: "hashing_fixture_cpu".to_string(),
            status: "measured".to_string(),
            notes: "Default deterministic producer/search oracle; no model weights required."
                .to_string(),
        },
        MultiVectorBackendRow {
            backend: "candle_colpali_cpu".to_string(),
            status: candle_status().to_string(),
            notes: "Feature-gated producer; real run needs ColPali weights and tokenizer."
                .to_string(),
        },
        MultiVectorBackendRow {
            backend: "candle_colpali_metal".to_string(),
            status: "not_wired".to_string(),
            notes: "The current producer selects CPU only; Metal needs explicit feature/device wiring before measurement.".to_string(),
        },
        MultiVectorBackendRow {
            backend: "candle_colpali_cuda".to_string(),
            status: "not_wired".to_string(),
            notes: "The current producer selects CPU only; CUDA needs explicit feature/device wiring before measurement.".to_string(),
        },
    ]
}

fn candle_status() -> &'static str {
    if cfg!(feature = "colpali-candle") {
        "compiled"
    } else {
        "feature_disabled"
    }
}

fn elapsed_ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

#[allow(dead_code)]
fn _assert_binary_sets_are_send_sync(_: &[BinaryMultiVectorSet]) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_benchmark_reports_recall_and_storage_rows() {
        let report = run_fixture_benchmark(MultiVectorBenchmarkConfig {
            corpus_size: 16,
            query_count: 4,
            vectors_per_document: 8,
            dim: 32,
            exact_top_k: 4,
            candidate_top_ks: vec![4, 8],
        })
        .expect("benchmark report");

        assert_eq!(report.recall_rows.len(), 2);
        assert_eq!(report.storage_cost.vector_count, 8);
        assert_eq!(report.storage_cost.dim, 32);
        assert!(report.storage_cost.exact_f32_to_binary_ratio() >= 32.0);
        assert!(report
            .backend_rows
            .iter()
            .any(|row| row.backend == "hashing_fixture_cpu" && row.status == "measured"));
    }
}
