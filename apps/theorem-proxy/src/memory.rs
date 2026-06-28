//! Memory source seam for ambient injection (SPEC-LOCAL-PROXY-MVP D3 +
//! SPEC-PROXY-PROVE-AND-PRUNE D1: relevance-ranked, not wholesale).
//!
//! The proxy retrieves over a `MemorySource` and injects the top hits at the
//! cache-stable suffix. The default ranks by query token overlap -- a real, if
//! simple, relevance signal. The substrate retrieval (`hippo_retrieve` /
//! index-context, with embeddings + PPR) is the production impl that plugs in behind
//! this trait without the proxy changing.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// A ranked memory retrieval surface.
pub trait MemorySource: Send + Sync {
    /// Up to `limit` memories relevant to `query`, most relevant first.
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemoryHit {
    pub title: String,
    pub body: String,
    pub score: f64,
}

/// Lowercased alphanumeric tokens of length >= 3 (drops trivial connective words
/// that would inflate overlap scores).
fn tokens(text: &str) -> BTreeSet<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 3)
        .map(str::to_string)
        .collect()
}

/// Relevance: how many distinct query tokens appear in the memory text.
fn relevance(query_tokens: &BTreeSet<String>, memory_text: &str) -> f64 {
    let memory_tokens = tokens(memory_text);
    query_tokens
        .iter()
        .filter(|token| memory_tokens.contains(*token))
        .count() as f64
}

fn rank(items: impl Iterator<Item = MemoryHit>, query: &str, limit: usize) -> Vec<MemoryHit> {
    let query_tokens = tokens(query);
    if query_tokens.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<MemoryHit> = items
        .map(|mut hit| {
            hit.score = relevance(&query_tokens, &format!("{} {}", hit.title, hit.body));
            hit
        })
        .filter(|hit| hit.score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.title.cmp(&b.title))
    });
    scored.truncate(limit.max(1));
    scored
}

/// In-memory source (tests, small static sets).
pub struct VecMemorySource {
    items: Vec<MemoryHit>,
}

impl VecMemorySource {
    pub fn new(items: Vec<(&str, &str)>) -> Self {
        Self {
            items: items
                .into_iter()
                .map(|(title, body)| MemoryHit {
                    title: title.to_string(),
                    body: body.to_string(),
                    score: 0.0,
                })
                .collect(),
        }
    }
}

impl MemorySource for VecMemorySource {
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        rank(self.items.iter().cloned(), query, limit)
    }
}

/// Directory source: one `*.md` file per memory (title = file stem, body = file
/// contents). The simplest durable real source; the substrate retrieval replaces it.
pub struct DirectoryMemorySource {
    dir: PathBuf,
}

impl DirectoryMemorySource {
    pub fn new(dir: impl AsRef<Path>) -> Self {
        Self {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    fn load(&self) -> Vec<MemoryHit> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
                continue;
            }
            let title = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default()
                .to_string();
            if let Ok(body) = std::fs::read_to_string(&path) {
                out.push(MemoryHit {
                    title,
                    body: body.trim().to_string(),
                    score: 0.0,
                });
            }
        }
        out
    }
}

impl MemorySource for DirectoryMemorySource {
    fn retrieve(&self, query: &str, limit: usize) -> Vec<MemoryHit> {
        rank(self.load().into_iter(), query, limit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_relevant_above_irrelevant_and_drops_zero() {
        let source = VecMemorySource::new(vec![
            ("planner", "the planner lives in planner.rs and does boolean pushdown"),
            ("cats", "cats are nice"),
        ]);
        let hits = source.retrieve("tell me about the planner pushdown", 5);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].title, "planner");
    }

    #[test]
    fn empty_query_returns_nothing() {
        let source = VecMemorySource::new(vec![("a", "b")]);
        assert!(source.retrieve("", 5).is_empty());
    }
}
