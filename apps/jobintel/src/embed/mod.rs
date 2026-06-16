//! The `Embedder` seam.
//!
//! Spec ("Module 2 - graph write", Embeddings): embed `title + body` with a
//! local sentence-transformers model (bge-small-en-v1.5, D=384), swappable to
//! the existing Theseus SBERT through a trait. The spec's illustrative
//! signature is `fn embed(&self, text: &str) -> Vec<f32>`; we return
//! `Result<Vec<f32>>` so network/model embedders surface failures instead of
//! poisoning the HNSW index with zero vectors. The seam itself - one `embed`
//! method with interchangeable backends - is exactly as specified.
//!
//! Three backends, all selectable from the CLI:
//! - `hash` HashEmbedder: deterministic feature-hashing, offline. Default, so
//!   the one-command demo needs no model download.
//! - `http` HttpEmbedder: POSTs to a remote encoder (the Theseus SBERT swap).
//! - `bge` BgeEmbedder: real bge-small-en-v1.5 via candle (feature `bge`).

mod hash;
mod http;

#[cfg(feature = "bge")]
mod bge;

pub use hash::HashEmbedder;
pub use http::HttpEmbedder;

use crate::config::Config;
use crate::error::{JobIntelError, Result};

/// The embedding abstraction every ranking + graph-write path depends on.
pub trait Embedder {
    /// Embed `text` (the spec's `title + body`) into a `dim()`-length vector.
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Embedding dimension D. The vector index is designated at this size.
    fn dim(&self) -> usize;
    /// Short identifier for logs / receipts (e.g. "hash", "bge-small-en-v1.5").
    fn name(&self) -> &str;
}

/// Which backend to construct.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedKind {
    Hash,
    Http,
    Bge,
}

impl EmbedKind {
    pub fn parse(raw: &str) -> Result<Self> {
        match raw.trim().to_lowercase().as_str() {
            "hash" => Ok(EmbedKind::Hash),
            "http" => Ok(EmbedKind::Http),
            "bge" => Ok(EmbedKind::Bge),
            other => Err(JobIntelError::Embed(format!(
                "unknown embedder '{other}' (expected hash|http|bge)"
            ))),
        }
    }
}

/// Construct the selected embedder. Failure modes (missing URL, missing
/// feature, model load) are front-loaded here so `embed()` is robust afterward.
pub fn build_embedder(kind: EmbedKind, config: &Config) -> Result<Box<dyn Embedder>> {
    match kind {
        EmbedKind::Hash => Ok(Box::new(HashEmbedder::new(config.embed_dim))),
        EmbedKind::Http => Ok(Box::new(HttpEmbedder::new(config)?)),
        EmbedKind::Bge => build_bge(config),
    }
}

#[cfg(feature = "bge")]
fn build_bge(config: &Config) -> Result<Box<dyn Embedder>> {
    Ok(Box::new(bge::BgeEmbedder::new(config)?))
}

#[cfg(not(feature = "bge"))]
fn build_bge(_config: &Config) -> Result<Box<dyn Embedder>> {
    Err(JobIntelError::Embed(
        "embedder 'bge' requires building with `--features bge`".into(),
    ))
}
