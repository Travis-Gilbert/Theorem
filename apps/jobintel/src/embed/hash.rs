//! Deterministic offline embedder via the feature-hashing trick.
//!
//! Each token is hashed to a bucket in `[0, dim)` and accumulated with a signed
//! weight; the vector is then L2-normalized. Texts sharing vocabulary land
//! closer in cosine space, so designate -> HNSW -> vector-search is a *real*
//! pipeline with no model download. It is not a semantic model - it captures
//! lexical overlap, not meaning - but it makes the demo runnable from one
//! command and the tests hermetic. Swap to `http`/`bge` for true semantics.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::Embedder;
use crate::error::Result;

pub struct HashEmbedder {
    dim: usize,
}

impl HashEmbedder {
    pub fn new(dim: usize) -> Self {
        Self { dim: dim.max(1) }
    }

    fn embed_inner(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dim];
        for token in tokenize(text) {
            let h = hash64(&token);
            let bucket = (h % self.dim as u64) as usize;
            // A second hash bit decides the sign, so distinct tokens can cancel
            // rather than all pushing the same direction (standard signed
            // feature hashing, reduces collision bias).
            let sign = if (h >> 33) & 1 == 0 { 1.0 } else { -1.0 };
            v[bucket] += sign;
        }
        l2_normalize(&mut v);
        v
    }
}

impl Embedder for HashEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        Ok(self.embed_inner(text))
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "hash"
    }
}

/// Lowercase alphanumeric word tokenizer. Words shorter than 2 chars are
/// dropped (mostly noise for lexical similarity).
fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 2)
        .map(|t| t.to_lowercase())
        .collect()
}

fn hash64(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dimension_is_respected() {
        let e = HashEmbedder::new(384);
        assert_eq!(e.dim(), 384);
        assert_eq!(e.embed("rust graph engineer").unwrap().len(), 384);
    }

    #[test]
    fn embedding_is_deterministic() {
        let e = HashEmbedder::new(64);
        let a = e
            .embed("Senior Rust Engineer, RAG and vector search")
            .unwrap();
        let b = e
            .embed("Senior Rust Engineer, RAG and vector search")
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn shared_vocabulary_scores_higher_than_disjoint() {
        let e = HashEmbedder::new(384);
        let profile = e.embed("rust graph vector database engineer").unwrap();
        let near = e.embed("rust engineer for graph vector database").unwrap();
        let far = e.embed("marketing manager fashion retail brand").unwrap();
        assert!(
            cosine(&profile, &near) > cosine(&profile, &far),
            "overlapping text must be nearer than disjoint text"
        );
    }

    #[test]
    fn normalized_to_unit_length() {
        let e = HashEmbedder::new(128);
        let v = e.embed("infrastructure llm agent retrieval").unwrap();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
}
