//! Multi-vector retrieval primitives for late-interaction rerank.
//!
//! This module is intentionally dependency-light. ColPali/Candle inference can
//! produce the vectors later; these functions provide the CPU oracle and the
//! binary projection shape that graph search and cold rerank can agree on now.

use std::cmp::Ordering;
use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use rustyred_thg_core::{ThgError, ThgResult};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorStorageCost {
    pub vector_count: usize,
    pub dim: usize,
    pub exact_f32_bytes: usize,
    pub exact_f16_bytes: usize,
    pub binary_projection_bytes: usize,
}

impl MultiVectorStorageCost {
    pub fn exact_f32_to_binary_ratio(&self) -> f32 {
        byte_ratio(self.exact_f32_bytes, self.binary_projection_bytes)
    }

    pub fn exact_f16_to_binary_ratio(&self) -> f32 {
        byte_ratio(self.exact_f16_bytes, self.binary_projection_bytes)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxSimAggregation {
    Sum,
    Mean,
}

impl Default for MaxSimAggregation {
    fn default() -> Self {
        Self::Mean
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MaxSimScorer {
    ExactFloat,
    BinaryHamming,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MultiVectorEmbeddingSet {
    pub embedding_set_id: String,
    pub content_id: String,
    pub model_id: String,
    pub model_version: String,
    pub vectors: Vec<Vec<f32>>,
}

impl MultiVectorEmbeddingSet {
    pub fn dim(&self) -> ThgResult<usize> {
        validate_vector_matrix("vectors", &self.vectors)
    }

    pub fn vector_count(&self) -> usize {
        self.vectors.len()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BinaryMultiVectorSet {
    pub embedding_set_id: String,
    pub content_id: String,
    pub model_id: String,
    pub model_version: String,
    pub dim: usize,
    pub vector_count: usize,
    pub words_per_vector: usize,
    pub words: Vec<u64>,
}

impl BinaryMultiVectorSet {
    fn vector_words(&self, idx: usize) -> &[u64] {
        let start = idx * self.words_per_vector;
        &self.words[start..start + self.words_per_vector]
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MultiVectorManifest {
    pub embedding_set_id: String,
    pub content_id: String,
    pub model_id: String,
    pub model_version: String,
    pub dim: usize,
    pub vector_count: usize,
    pub exact_object_ref: Option<String>,
    pub binary_projection_ref: Option<String>,
    pub exact_bytes: usize,
    pub binary_projection_bytes: usize,
}

impl MultiVectorManifest {
    pub fn from_exact_set(
        set: &MultiVectorEmbeddingSet,
        exact_object_ref: Option<String>,
        binary_projection_ref: Option<String>,
    ) -> ThgResult<Self> {
        let dim = set.dim()?;
        let vector_count = set.vector_count();
        Ok(Self {
            embedding_set_id: set.embedding_set_id.clone(),
            content_id: set.content_id.clone(),
            model_id: set.model_id.clone(),
            model_version: set.model_version.clone(),
            dim,
            vector_count,
            exact_object_ref,
            binary_projection_ref,
            exact_bytes: exact_f32_bytes(vector_count, dim),
            binary_projection_bytes: binary_projection_bytes(vector_count, dim),
        })
    }

    pub fn exact_to_binary_byte_ratio(&self) -> f32 {
        byte_ratio(self.exact_bytes, self.binary_projection_bytes)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MultiVectorScore {
    pub content_id: String,
    pub embedding_set_id: String,
    pub score: f32,
    pub scorer: MaxSimScorer,
    pub vector_count: usize,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct MultiVectorRecallReport {
    pub exact_top_k: usize,
    pub candidate_top_k: usize,
    pub exact_count: usize,
    pub candidate_count: usize,
    pub overlap_count: usize,
    pub recall: f32,
    pub missing_content_ids: Vec<String>,
}

pub fn exact_f32_bytes(vector_count: usize, dim: usize) -> usize {
    vector_count.saturating_mul(dim).saturating_mul(4)
}

pub fn exact_f16_bytes(vector_count: usize, dim: usize) -> usize {
    vector_count.saturating_mul(dim).saturating_mul(2)
}

pub fn binary_projection_bytes(vector_count: usize, dim: usize) -> usize {
    vector_count.saturating_mul(dim.saturating_add(7) / 8)
}

pub fn storage_costs(vector_count: usize, dim: usize) -> MultiVectorStorageCost {
    MultiVectorStorageCost {
        vector_count,
        dim,
        exact_f32_bytes: exact_f32_bytes(vector_count, dim),
        exact_f16_bytes: exact_f16_bytes(vector_count, dim),
        binary_projection_bytes: binary_projection_bytes(vector_count, dim),
    }
}

pub fn exact_maxsim_score(
    query_vectors: &[Vec<f32>],
    document_vectors: &[Vec<f32>],
    aggregation: MaxSimAggregation,
) -> ThgResult<f32> {
    let query_dim = validate_vector_matrix("query_vectors", query_vectors)?;
    let document_dim = validate_vector_matrix("document_vectors", document_vectors)?;
    if query_dim != document_dim {
        return Err(ThgError::new(
            "multivector_dim_mismatch",
            format!("query dim {query_dim} does not match document dim {document_dim}"),
        ));
    }

    let mut total = 0.0_f32;
    for query in query_vectors {
        let mut best = f32::NEG_INFINITY;
        for document in document_vectors {
            best = best.max(dot(query, document));
        }
        total += best;
    }
    Ok(apply_aggregation(total, query_vectors.len(), aggregation))
}

pub fn quantize_sign_bits(set: &MultiVectorEmbeddingSet) -> ThgResult<BinaryMultiVectorSet> {
    let dim = set.dim()?;
    let words_per_vector = dim.saturating_add(63) / 64;
    let mut words = vec![0_u64; set.vector_count() * words_per_vector];
    for (vector_idx, vector) in set.vectors.iter().enumerate() {
        let base = vector_idx * words_per_vector;
        for (dim_idx, value) in vector.iter().enumerate() {
            if *value >= 0.0 {
                words[base + dim_idx / 64] |= 1_u64 << (dim_idx % 64);
            }
        }
    }

    Ok(BinaryMultiVectorSet {
        embedding_set_id: set.embedding_set_id.clone(),
        content_id: set.content_id.clone(),
        model_id: set.model_id.clone(),
        model_version: set.model_version.clone(),
        dim,
        vector_count: set.vector_count(),
        words_per_vector,
        words,
    })
}

pub fn binary_hamming_maxsim_score(
    query: &BinaryMultiVectorSet,
    document: &BinaryMultiVectorSet,
    aggregation: MaxSimAggregation,
) -> ThgResult<f32> {
    validate_binary_pair(query, document)?;
    let mut total = 0.0_f32;
    for query_idx in 0..query.vector_count {
        let query_words = query.vector_words(query_idx);
        let mut best = f32::NEG_INFINITY;
        for document_idx in 0..document.vector_count {
            let document_words = document.vector_words(document_idx);
            best = best.max(binary_signed_similarity(
                query_words,
                document_words,
                query.dim,
            ));
        }
        total += best;
    }
    Ok(apply_aggregation(total, query.vector_count, aggregation))
}

pub fn rank_exact_maxsim(
    query_vectors: &[Vec<f32>],
    documents: &[MultiVectorEmbeddingSet],
    aggregation: MaxSimAggregation,
    limit: usize,
) -> ThgResult<Vec<MultiVectorScore>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut scores = Vec::with_capacity(documents.len());
    for document in documents {
        let score = exact_maxsim_score(query_vectors, &document.vectors, aggregation)?;
        scores.push(MultiVectorScore {
            content_id: document.content_id.clone(),
            embedding_set_id: document.embedding_set_id.clone(),
            score,
            scorer: MaxSimScorer::ExactFloat,
            vector_count: document.vector_count(),
        });
    }
    sort_and_truncate(&mut scores, limit);
    Ok(scores)
}

pub fn rank_binary_hamming_maxsim(
    query: &BinaryMultiVectorSet,
    documents: &[BinaryMultiVectorSet],
    aggregation: MaxSimAggregation,
    limit: usize,
) -> ThgResult<Vec<MultiVectorScore>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut scores = Vec::with_capacity(documents.len());
    for document in documents {
        let score = binary_hamming_maxsim_score(query, document, aggregation)?;
        scores.push(MultiVectorScore {
            content_id: document.content_id.clone(),
            embedding_set_id: document.embedding_set_id.clone(),
            score,
            scorer: MaxSimScorer::BinaryHamming,
            vector_count: document.vector_count,
        });
    }
    sort_and_truncate(&mut scores, limit);
    Ok(scores)
}

fn validate_vector_matrix(name: &str, vectors: &[Vec<f32>]) -> ThgResult<usize> {
    if vectors.is_empty() {
        return Err(ThgError::new(
            "multivector_empty",
            format!("{name} must contain at least one vector"),
        ));
    }
    let dim = vectors[0].len();
    if dim == 0 {
        return Err(ThgError::new(
            "multivector_empty_dim",
            format!("{name} vectors must have non-zero dimension"),
        ));
    }
    for (idx, vector) in vectors.iter().enumerate() {
        if vector.len() != dim {
            return Err(ThgError::new(
                "multivector_shape_mismatch",
                format!("{name}[{idx}] has dim {}, expected {dim}", vector.len()),
            ));
        }
        if vector.iter().any(|value| !value.is_finite()) {
            return Err(ThgError::new(
                "multivector_non_finite",
                format!("{name}[{idx}] contains a non-finite value"),
            ));
        }
    }
    Ok(dim)
}

fn validate_binary_set(name: &str, set: &BinaryMultiVectorSet) -> ThgResult<()> {
    if set.dim == 0 || set.vector_count == 0 || set.words_per_vector == 0 {
        return Err(ThgError::new(
            "binary_multivector_empty",
            format!("{name} must contain vectors with non-zero dimension"),
        ));
    }
    let expected_words_per_vector = set.dim.saturating_add(63) / 64;
    if set.words_per_vector != expected_words_per_vector {
        return Err(ThgError::new(
            "binary_multivector_shape_mismatch",
            format!(
                "{name} words_per_vector {} does not match dim {}",
                set.words_per_vector, set.dim
            ),
        ));
    }
    let expected_words = set.vector_count * set.words_per_vector;
    if set.words.len() != expected_words {
        return Err(ThgError::new(
            "binary_multivector_shape_mismatch",
            format!(
                "{name} has {} words, expected {expected_words}",
                set.words.len()
            ),
        ));
    }
    Ok(())
}

fn validate_binary_pair(
    query: &BinaryMultiVectorSet,
    document: &BinaryMultiVectorSet,
) -> ThgResult<()> {
    validate_binary_set("query", query)?;
    validate_binary_set("document", document)?;
    if query.dim != document.dim {
        return Err(ThgError::new(
            "binary_multivector_dim_mismatch",
            format!(
                "query dim {} does not match document dim {}",
                query.dim, document.dim
            ),
        ));
    }
    if query.words_per_vector != document.words_per_vector {
        return Err(ThgError::new(
            "binary_multivector_shape_mismatch",
            "query and document words_per_vector differ",
        ));
    }
    Ok(())
}

fn dot(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(a, b)| a * b).sum()
}

fn binary_signed_similarity(query: &[u64], document: &[u64], dim: usize) -> f32 {
    let mut distance = 0_u32;
    for (idx, (query_word, document_word)) in query.iter().zip(document).enumerate() {
        let mut diff = query_word ^ document_word;
        let is_last = idx + 1 == query.len();
        let remainder = dim % 64;
        if is_last && remainder != 0 {
            diff &= (1_u64 << remainder) - 1;
        }
        distance += diff.count_ones();
    }
    (dim as i32 - 2 * distance as i32) as f32 / dim as f32
}

fn apply_aggregation(total: f32, query_count: usize, aggregation: MaxSimAggregation) -> f32 {
    match aggregation {
        MaxSimAggregation::Sum => total,
        MaxSimAggregation::Mean => total / query_count.max(1) as f32,
    }
}

fn sort_and_truncate(scores: &mut Vec<MultiVectorScore>, limit: usize) {
    scores.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.content_id.cmp(&right.content_id))
            .then_with(|| left.embedding_set_id.cmp(&right.embedding_set_id))
    });
    scores.truncate(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: &str, content_id: &str, vectors: Vec<Vec<f32>>) -> MultiVectorEmbeddingSet {
        MultiVectorEmbeddingSet {
            embedding_set_id: id.to_string(),
            content_id: content_id.to_string(),
            model_id: "colpali-fixture".to_string(),
            model_version: "test-v1".to_string(),
            vectors,
        }
    }

    #[test]
    fn exact_maxsim_ranks_document_with_matching_regions() {
        let query = vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]];
        let relevant = set(
            "mv:relevant",
            "doc:relevant",
            vec![vec![1.0, 0.0, 0.0], vec![0.0, 1.0, 0.0]],
        );
        let weak = set(
            "mv:weak",
            "doc:weak",
            vec![vec![0.0, 0.0, 1.0], vec![-1.0, 0.0, 0.0]],
        );

        let ranked = rank_exact_maxsim(&query, &[weak, relevant], MaxSimAggregation::Mean, 2)
            .expect("exact MaxSim ranking");

        assert_eq!(ranked[0].content_id, "doc:relevant");
        assert_eq!(ranked[0].scorer, MaxSimScorer::ExactFloat);
        assert!(ranked[0].score > ranked[1].score);
    }

    #[test]
    fn binary_hamming_maxsim_matches_exact_top_one_on_sign_fixture() {
        let query = set("mv:query", "query", vec![vec![1.0, -1.0, 1.0, -1.0]]);
        let relevant = set(
            "mv:relevant",
            "doc:relevant",
            vec![vec![1.0, -1.0, 1.0, -1.0]],
        );
        let distractor = set(
            "mv:distractor",
            "doc:distractor",
            vec![vec![-1.0, 1.0, -1.0, 1.0]],
        );

        let exact = rank_exact_maxsim(
            &query.vectors,
            &[distractor.clone(), relevant.clone()],
            MaxSimAggregation::Mean,
            1,
        )
        .expect("exact ranking");
        let binary_query = quantize_sign_bits(&query).expect("binary query");
        let binary_docs = vec![
            quantize_sign_bits(&distractor).expect("binary distractor"),
            quantize_sign_bits(&relevant).expect("binary relevant"),
        ];
        let binary =
            rank_binary_hamming_maxsim(&binary_query, &binary_docs, MaxSimAggregation::Mean, 1)
                .expect("binary ranking");

        assert_eq!(exact[0].content_id, "doc:relevant");
        assert_eq!(binary[0].content_id, exact[0].content_id);
        assert_eq!(binary[0].scorer, MaxSimScorer::BinaryHamming);
    }

    #[test]
    fn manifest_records_exact_and_binary_storage_costs() {
        let vectors = vec![vec![1.0; 128]; 32];
        let embedding_set = set("mv:page:1", "page:1", vectors);

        let manifest = MultiVectorManifest::from_exact_set(
            &embedding_set,
            Some("cold://exact/page-1.f32".to_string()),
            Some("cold://binary/page-1.bits".to_string()),
        )
        .expect("manifest");

        assert_eq!(manifest.dim, 128);
        assert_eq!(manifest.vector_count, 32);
        assert_eq!(manifest.exact_bytes, 32 * 128 * 4);
        assert_eq!(manifest.binary_projection_bytes, 32 * 16);
        assert!(manifest.exact_to_binary_byte_ratio() >= 32.0);
    }

    #[test]
    fn exact_maxsim_rejects_dimension_mismatch() {
        let err = exact_maxsim_score(
            &[vec![1.0, 0.0]],
            &[vec![1.0, 0.0, 0.0]],
            MaxSimAggregation::Mean,
        )
        .expect_err("dimension mismatch");

        assert_eq!(err.code, "multivector_dim_mismatch");
    }

    #[test]
    fn binary_hamming_rejects_mismatched_dimensions() {
        let query =
            quantize_sign_bits(&set("q", "q", vec![vec![1.0, -1.0]])).expect("binary query");
        let document = quantize_sign_bits(&set("d", "d", vec![vec![1.0, -1.0, 1.0]]))
            .expect("binary document");

        let err = binary_hamming_maxsim_score(&query, &document, MaxSimAggregation::Mean)
            .expect_err("dimension mismatch");

        assert_eq!(err.code, "binary_multivector_dim_mismatch");
    }
}
