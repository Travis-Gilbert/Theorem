//! Phase 9: content hashing for dedup and ssdeep fuzzy hashing for similarity.
//!
//! DATAWAVE references:
//! - `warehouse/ingest-core/.../data/hash/`: content hashing for dedup.
//! - `warehouse/ingest-ssdeep`: ssdeep fuzzy hashing for similarity over content.
//!
//! Content dedup uses the same `sha256:<hex>` convention as
//! `rustyred-thg-binformat`, so an ingested document and a reconstructed binary
//! address content identically. Fuzzy similarity uses the `fuzzyhash` crate, a
//! pure-Rust ssdeep / spamsum implementation that produces ssdeep-compatible
//! `blocksize:h1:h2` digests (no C dependency), so content-similarity facts are
//! interoperable with the ssdeep reference and reusable for binary similarity in
//! the reconstruction corpus.

use fuzzyhash::FuzzyHash;
use sha2::{Digest, Sha256};

/// Content-addressed hash for dedup. `sha256:<hex>`, matching binformat.
pub fn content_hash(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// ssdeep-compatible fuzzy hash digest (`blocksize:h1:h2`) for the bytes.
pub fn fuzzy_hash(bytes: &[u8]) -> String {
    FuzzyHash::new(bytes).to_string()
}

/// Similarity of two ssdeep digests, 0..=100 (0 when incomparable). Delegates to
/// the ssdeep comparison in the `fuzzyhash` crate.
pub fn fuzzy_compare(a: &str, b: &str) -> u32 {
    FuzzyHash::compare(a, b).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_hash_is_stable_sha256() {
        assert_eq!(content_hash(b"abc"), content_hash(b"abc"));
        assert_ne!(content_hash(b"abc"), content_hash(b"abd"));
        assert!(content_hash(b"abc").starts_with("sha256:"));
    }

    #[test]
    fn fuzzy_digest_has_ssdeep_shape_and_is_deterministic() {
        let body = "lorem ipsum dolor sit amet ".repeat(40);
        let a = fuzzy_hash(body.as_bytes());
        let b = fuzzy_hash(body.as_bytes());
        // ssdeep digest is blocksize:hash1:hash2 (two colons), deterministic.
        assert_eq!(a, b);
        assert_eq!(a.matches(':').count(), 2, "ssdeep digest shape: {a}");
    }

    #[test]
    fn fuzzy_self_is_perfect_and_similar_beats_different() {
        let base = "the quick brown fox jumps over the lazy dog. ".repeat(40);
        let near = format!("{base}one extra trailing sentence here.");
        let far = "completely unrelated content about ocean tides and lunar phases. ".repeat(40);

        let hb = fuzzy_hash(base.as_bytes());
        let hn = fuzzy_hash(near.as_bytes());
        let hf = fuzzy_hash(far.as_bytes());

        assert_eq!(fuzzy_compare(&hb, &hb), 100);
        assert!(
            fuzzy_compare(&hb, &hn) > fuzzy_compare(&hb, &hf),
            "near {} should beat far {}",
            fuzzy_compare(&hb, &hn),
            fuzzy_compare(&hb, &hf)
        );
    }
}
