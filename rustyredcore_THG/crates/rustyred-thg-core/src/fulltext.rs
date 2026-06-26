//! Phase 5: Inverted-index BM25 full-text search.
//!
//! A purpose-built lexical index keyed by (label, property). We tokenize on
//! non-alphanumeric boundaries, lowercase, and skip stop words. Each
//! `FullTextIndex` holds:
//!   - postings: term -> Vec<(doc_id, term_freq)>
//!   - doc_lengths: doc_id -> u32
//!   - avg_doc_length
//!
//! Scoring is BM25 with k1 = 1.2, b = 0.75 (standard defaults).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::index_manifest::{
    IndexBackend, IndexBuildStatus, IndexCreatedBy, IndexKind, IndexManifest, IndexScope,
};

const BM25_K1: f64 = 1.2;
const BM25_B: f64 = 0.75;

/// §P5-A pa5.3: env var that selects the fulltext backend at construction time.
pub const RUSTY_RED_FULLTEXT_BACKEND_ENV: &str = "RUSTY_RED_FULLTEXT_BACKEND";

// Canonical backend-name strings; the dispatcher accepts a few aliases per kind.
pub(crate) const FULLTEXT_BACKEND_HAND_ROLLED: &str = "hand_rolled";
pub(crate) const FULLTEXT_BACKEND_TANTIVY: &str = "tantivy";

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FullTextDesignation {
    pub label: String,
    pub property: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FullTextSearchBackend {
    Bm25,
    Tantivy,
}

impl FullTextSearchBackend {
    fn index_backend(self) -> IndexBackend {
        match self {
            Self::Bm25 => IndexBackend::Bm25,
            Self::Tantivy => IndexBackend::Tantivy,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FieldedFullTextDefinition {
    pub manifest_id: String,
    pub target_label: String,
    pub fields: Vec<String>,
    pub backend: FullTextSearchBackend,
}

impl FieldedFullTextDefinition {
    pub fn bm25(
        manifest_id: impl Into<String>,
        target_label: impl Into<String>,
        fields: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            manifest_id: manifest_id.into(),
            target_label: target_label.into(),
            fields: fields.into_iter().map(Into::into).collect(),
            backend: FullTextSearchBackend::Bm25,
        }
    }

    pub fn to_manifest(&self, scope: IndexScope, created_by: IndexCreatedBy) -> IndexManifest {
        let mut manifest = IndexManifest::new(
            self.manifest_id.clone(),
            format!("{} fielded full-text", self.target_label),
            IndexKind::FullText,
            self.backend.index_backend(),
            scope,
            self.target_label.clone(),
            created_by,
        )
        .with_target_properties(self.fields.clone());
        manifest.build_status = IndexBuildStatus::Active;
        manifest.refresh_hashes();
        manifest
    }
}

/// §P5-A pa5.1 + cross-cutting cc.3: the full-text designation is now a
/// required, owned field. The per-tenant map already keys by (label, property),
/// so wrapping the designation in `Option` was redundant and prone to silent
/// fall-throughs ("missing designation" only surfaced via `.unwrap_or_default`).
#[derive(Debug)]
pub struct FullTextIndex {
    pub designation: FullTextDesignation,
    /// term -> list of (doc_id, term frequency in this doc)
    postings: HashMap<String, Vec<(String, u32)>>,
    /// per-doc total term count
    doc_lengths: HashMap<String, u32>,
    /// per-doc unique terms (for O(doc_terms) removes instead of full vocab scan)
    doc_terms: HashMap<String, Vec<String>>,
    /// docs known to this index (for re-indexing on update)
    indexed: BTreeSet<String>,
    total_length: u64,
}

/// §P5-A pa5.1: trait abstraction over the full-text storage layer. The
/// hand-rolled `FullTextIndex` is the only impl today; a tantivy-backed
/// implementation will sit behind a `tantivy` feature flag once the SPEC's
/// switch is implemented.
pub trait FullTextBackend: Send + Sync + std::fmt::Debug {
    /// Index (or re-index) a document under `doc_id` with the given text.
    fn upsert(&mut self, doc_id: &str, text: &str);
    /// Remove a document from the index.
    fn remove(&mut self, doc_id: &str);
    /// Return the top-k scored hits for the query.
    fn search(&self, query: &str, k: usize) -> Vec<(String, f32)>;
    /// Return the designation this backend was created for.
    fn designation(&self) -> &FullTextDesignation;
    /// Number of documents currently indexed.
    fn doc_count(&self) -> usize;
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct FieldedFullTextDocument {
    pub doc_id: String,
    pub fields: BTreeMap<String, String>,
}

impl FieldedFullTextDocument {
    pub fn new(doc_id: impl Into<String>) -> Self {
        Self {
            doc_id: doc_id.into(),
            fields: BTreeMap::new(),
        }
    }

    pub fn with_field(mut self, field: impl Into<String>, text: impl Into<String>) -> Self {
        self.fields.insert(field.into(), text.into());
        self
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FullTextSnippet {
    pub field: String,
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FieldedFullTextHit {
    pub doc_id: String,
    pub score: f32,
    pub field_scores: BTreeMap<String, f32>,
    pub snippets: Vec<FullTextSnippet>,
}

#[derive(Debug)]
pub struct FieldedFullTextIndex {
    designation_label: String,
    field_indexes: BTreeMap<String, FullTextIndex>,
    field_boosts: BTreeMap<String, f32>,
    raw_fields: BTreeMap<String, BTreeMap<String, String>>,
}

impl FieldedFullTextIndex {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            designation_label: label.into(),
            field_indexes: BTreeMap::new(),
            field_boosts: BTreeMap::new(),
            raw_fields: BTreeMap::new(),
        }
    }

    pub fn set_field_boost(&mut self, field: impl Into<String>, boost: f32) {
        self.field_boosts.insert(field.into(), boost.max(0.0));
    }

    pub fn upsert(&mut self, document: FieldedFullTextDocument) {
        let doc_id = document.doc_id.clone();
        let designation_label = self.designation_label.clone();
        self.remove(&doc_id);
        for (field, text) in &document.fields {
            let index_text = if is_code_field(field) {
                code_aware_terms(text).join(" ")
            } else {
                text.clone()
            };
            self.field_indexes
                .entry(field.clone())
                .or_insert_with(|| {
                    FullTextIndex::for_designation(FullTextDesignation {
                        label: designation_label.clone(),
                        property: field.clone(),
                    })
                })
                .upsert(&doc_id, &index_text);
        }
        self.raw_fields.insert(doc_id, document.fields);
    }

    pub fn remove(&mut self, doc_id: &str) {
        for index in self.field_indexes.values_mut() {
            index.remove(doc_id);
        }
        self.raw_fields.remove(doc_id);
    }

    pub fn search(&self, query: &str, k: usize) -> Vec<FieldedFullTextHit> {
        let mut scores: BTreeMap<String, FieldedFullTextHit> = BTreeMap::new();
        for (field, index) in &self.field_indexes {
            let boost = self.field_boosts.get(field).copied().unwrap_or(1.0);
            for (doc_id, score) in index.search(query, k.saturating_mul(4).max(k)) {
                let weighted = score * boost;
                let hit = scores
                    .entry(doc_id.clone())
                    .or_insert_with(|| FieldedFullTextHit {
                        doc_id: doc_id.clone(),
                        score: 0.0,
                        field_scores: BTreeMap::new(),
                        snippets: Vec::new(),
                    });
                hit.score += weighted;
                hit.field_scores.insert(field.clone(), weighted);
            }
        }
        let mut hits = scores.into_values().collect::<Vec<_>>();
        for hit in &mut hits {
            hit.snippets = self.snippets(&hit.doc_id, query, 2);
        }
        sort_fulltext_hits(&mut hits, k);
        hits
    }

    pub fn phrase_search(&self, phrase: &str, k: usize) -> Vec<FieldedFullTextHit> {
        self.literal_scan(phrase, k, LiteralMode::Phrase)
    }

    pub fn prefix_search(&self, prefix: &str, k: usize) -> Vec<FieldedFullTextHit> {
        self.literal_scan(prefix, k, LiteralMode::Prefix)
    }

    pub fn fuzzy_search(
        &self,
        term: &str,
        max_distance: usize,
        k: usize,
    ) -> Vec<FieldedFullTextHit> {
        let query = term.to_ascii_lowercase();
        let mut hits = Vec::new();
        for (doc_id, fields) in &self.raw_fields {
            let mut score = 0.0;
            let mut field_scores = BTreeMap::new();
            for (field, text) in fields {
                let matched = tokenize(text)
                    .into_iter()
                    .chain(code_aware_terms(text))
                    .any(|token| edit_distance(&token, &query) <= max_distance);
                if matched {
                    let boosted = self.field_boosts.get(field).copied().unwrap_or(1.0);
                    score += boosted;
                    field_scores.insert(field.clone(), boosted);
                }
            }
            if score > 0.0 {
                hits.push(FieldedFullTextHit {
                    doc_id: doc_id.clone(),
                    score,
                    field_scores,
                    snippets: self.snippets(doc_id, term, 2),
                });
            }
        }
        sort_fulltext_hits(&mut hits, k);
        hits
    }

    fn literal_scan(&self, needle: &str, k: usize, mode: LiteralMode) -> Vec<FieldedFullTextHit> {
        let needle = needle.to_ascii_lowercase();
        let mut hits = Vec::new();
        for (doc_id, fields) in &self.raw_fields {
            let mut score = 0.0;
            let mut field_scores = BTreeMap::new();
            for (field, text) in fields {
                let matched = match mode {
                    LiteralMode::Phrase => text.to_ascii_lowercase().contains(&needle),
                    LiteralMode::Prefix => tokenize(text)
                        .into_iter()
                        .chain(code_aware_terms(text))
                        .any(|token| token.starts_with(&needle)),
                };
                if matched {
                    let boosted = self.field_boosts.get(field).copied().unwrap_or(1.0);
                    score += boosted;
                    field_scores.insert(field.clone(), boosted);
                }
            }
            if score > 0.0 {
                hits.push(FieldedFullTextHit {
                    doc_id: doc_id.clone(),
                    score,
                    field_scores,
                    snippets: self.snippets(doc_id, &needle, 2),
                });
            }
        }
        sort_fulltext_hits(&mut hits, k);
        hits
    }

    fn snippets(&self, doc_id: &str, query: &str, limit: usize) -> Vec<FullTextSnippet> {
        let mut snippets = Vec::new();
        let Some(fields) = self.raw_fields.get(doc_id) else {
            return snippets;
        };
        let terms = tokenize(query);
        for (field, text) in fields {
            let lower = text.to_ascii_lowercase();
            let hit = terms
                .iter()
                .find_map(|term| lower.find(term).map(|start| (start, term.len())))
                .or_else(|| {
                    let needle = query.to_ascii_lowercase();
                    lower.find(&needle).map(|start| (start, needle.len()))
                });
            if let Some((start, len)) = hit {
                let snippet_start = clamp_to_char_boundary(text, start.saturating_sub(40));
                let snippet_end = clamp_to_char_boundary(text, (start + len + 40).min(text.len()));
                snippets.push(FullTextSnippet {
                    field: field.clone(),
                    start,
                    end: start + len,
                    text: text[snippet_start..snippet_end].to_string(),
                });
                if snippets.len() >= limit {
                    break;
                }
            }
        }
        snippets
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LiteralMode {
    Phrase,
    Prefix,
}

impl FullTextIndex {
    pub fn for_designation(d: FullTextDesignation) -> Self {
        Self {
            designation: d,
            postings: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_terms: HashMap::new(),
            indexed: BTreeSet::new(),
            total_length: 0,
        }
    }

    pub fn doc_count(&self) -> usize {
        self.indexed.len()
    }

    pub fn upsert(&mut self, doc_id: &str, text: &str) {
        if self.indexed.contains(doc_id) {
            self.remove(doc_id);
        }
        let tokens = tokenize(text);
        if tokens.is_empty() {
            // Mark as indexed-with-empty so later removes know about it.
            self.indexed.insert(doc_id.to_string());
            self.doc_lengths.insert(doc_id.to_string(), 0);
            return;
        }

        let mut term_freq: HashMap<String, u32> = HashMap::new();
        for tok in tokens.iter() {
            *term_freq.entry(tok.clone()).or_insert(0) += 1;
        }
        let length = tokens.len() as u32;
        let unique_terms: Vec<String> = term_freq.keys().cloned().collect();
        for (term, freq) in term_freq {
            self.postings
                .entry(term)
                .or_default()
                .push((doc_id.to_string(), freq));
        }
        self.doc_lengths.insert(doc_id.to_string(), length);
        self.doc_terms.insert(doc_id.to_string(), unique_terms);
        self.indexed.insert(doc_id.to_string());
        self.total_length += length as u64;
    }

    pub fn remove(&mut self, doc_id: &str) {
        if !self.indexed.remove(doc_id) {
            return;
        }
        if let Some(len) = self.doc_lengths.remove(doc_id) {
            self.total_length = self.total_length.saturating_sub(len as u64);
        }
        // Only scan the doc's own terms, not the entire vocabulary.
        if let Some(terms) = self.doc_terms.remove(doc_id) {
            for term in terms {
                if let Some(plist) = self.postings.get_mut(&term) {
                    plist.retain(|(id, _)| id != doc_id);
                    if plist.is_empty() {
                        self.postings.remove(&term);
                    }
                }
            }
        }
    }

    /// Return the top-k doc_ids ranked by BM25 against the query string.
    pub fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        let n = self.indexed.len() as f64;
        if n == 0.0 {
            return Vec::new();
        }
        let avg_len = (self.total_length as f64) / n;
        let tokens = tokenize(query);
        if tokens.is_empty() {
            return Vec::new();
        }
        let mut scores: HashMap<String, f64> = HashMap::new();
        for tok in tokens.iter() {
            let Some(postings) = self.postings.get(tok) else {
                continue;
            };
            let df = postings.len() as f64;
            // BM25 idf with the +0.5 add-half smoothing.
            let idf = ((n - df + 0.5) / (df + 0.5) + 1.0).ln();
            for (doc_id, tf) in postings {
                let dl = *self.doc_lengths.get(doc_id).unwrap_or(&0) as f64;
                let tf = *tf as f64;
                let norm = 1.0 - BM25_B + BM25_B * dl / avg_len.max(1.0);
                let s = idf * tf * (BM25_K1 + 1.0) / (tf + BM25_K1 * norm);
                *scores.entry(doc_id.clone()).or_insert(0.0) += s;
            }
        }
        let mut entries: Vec<(String, f32)> =
            scores.into_iter().map(|(id, s)| (id, s as f32)).collect();
        entries.sort_by(|a, b| {
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        entries.truncate(k);
        entries
    }
}

const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "for", "from", "has", "he", "in", "is", "it",
    "its", "of", "on", "or", "that", "the", "to", "was", "were", "will", "with", "this", "these",
    "those",
];

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .filter(|s| !s.is_empty())
        .map(|s| s.to_lowercase())
        .filter(|s| !STOP_WORDS.contains(&s.as_str()))
        .collect()
}

fn is_code_field(field: &str) -> bool {
    matches!(
        field,
        "name" | "symbol" | "symbol_path" | "path" | "file_path" | "body" | "docstring"
    )
}

fn code_aware_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in text.split_whitespace() {
        let trimmed = raw.trim_matches(|c: char| {
            !(c.is_alphanumeric() || matches!(c, '_' | ':' | '.' | '/' | '-'))
        });
        if trimmed.is_empty() {
            continue;
        }
        terms.push(trimmed.to_ascii_lowercase());
        terms.extend(
            trimmed
                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .filter(|part| !part.is_empty())
                .map(|part| part.to_ascii_lowercase()),
        );
    }
    terms.sort();
    terms.dedup();
    terms
}

fn sort_fulltext_hits(hits: &mut Vec<FieldedFullTextHit>, k: usize) {
    hits.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.doc_id.cmp(&b.doc_id))
    });
    hits.truncate(k);
}

fn clamp_to_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut cur = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left.chars().enumerate() {
        cur[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let cost = usize::from(left_ch != *right_ch);
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[right_chars.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ranks_relevant_doc_higher() {
        let mut idx = FullTextIndex::for_designation(FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        });
        idx.upsert(
            "d1",
            "Rust is a systems programming language focused on safety.",
        );
        idx.upsert(
            "d2",
            "Python is a popular dynamic programming language for data science.",
        );
        idx.upsert("d3", "The graph database holds tenant snapshots.");

        let results = idx.search("rust programming", 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "d1");
        // d2 should appear since it shares "programming"
        let ids: Vec<&str> = results.iter().map(|(i, _)| i.as_str()).collect();
        assert!(ids.contains(&"d2"));
    }

    #[test]
    fn remove_excludes_doc_from_future_searches() {
        let mut idx = FullTextIndex::for_designation(FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        });
        idx.upsert("d1", "alpha beta");
        idx.upsert("d2", "alpha");
        idx.remove("d1");
        let results = idx.search("alpha", 5);
        let ids: Vec<&str> = results.iter().map(|(i, _)| i.as_str()).collect();
        assert!(!ids.contains(&"d1"));
        assert!(ids.contains(&"d2"));
    }

    #[test]
    fn upsert_replaces_existing_text() {
        let mut idx = FullTextIndex::for_designation(FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        });
        idx.upsert("d1", "knowledge graph database");
        // overwrite
        idx.upsert("d1", "weather forecast for tomorrow");
        let r = idx.search("knowledge", 5);
        assert!(r.is_empty());
        let r = idx.search("weather", 5);
        assert_eq!(r[0].0, "d1");
    }

    // §P5-A pa5.1 + cc.3: backend trait + designation-required guarantees.

    #[test]
    fn fulltext_index_implements_fulltext_backend_trait() {
        let designation = FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        };
        let mut backend: Box<dyn FullTextBackend> =
            Box::new(FullTextIndex::for_designation(designation));
        backend.upsert("d1", "the quick brown fox");
        backend.upsert("d2", "the lazy dog");
        let hits = backend.search("fox", 5);
        assert!(!hits.is_empty());
        assert_eq!(hits[0].0, "d1");
        assert_eq!(backend.doc_count(), 2);
    }

    #[test]
    fn designation_is_owned_value_not_optional() {
        let idx = FullTextIndex::for_designation(FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        });
        assert_eq!(idx.designation.label, "Doc");
        assert_eq!(idx.designation.property, "text");
    }

    // §P5-A pa5.3: env-switch factory. Tests go through the pure dispatcher so
    // they can run in parallel without mutating the global env.

    fn fixture_designation() -> FullTextDesignation {
        FullTextDesignation {
            label: "Doc".into(),
            property: "text".into(),
        }
    }

    #[test]
    fn make_fulltext_backend_defaults_to_hand_rolled() {
        let backend = make_fulltext_backend_from_value(fixture_designation(), "").unwrap();
        assert_eq!(backend.designation().label, "Doc");
        assert_eq!(backend.doc_count(), 0);
    }

    #[test]
    fn make_fulltext_backend_rejects_unknown_backend() {
        let err = make_fulltext_backend_from_value(fixture_designation(), "elastic")
            .expect_err("unknown backend should error");
        assert_eq!(err.code(), "unknown_fulltext_backend");
    }

    #[cfg(not(feature = "tantivy"))]
    #[test]
    fn make_fulltext_backend_errors_when_tantivy_requested_without_feature() {
        let err = make_fulltext_backend_from_value(fixture_designation(), "tantivy")
            .expect_err("tantivy without feature should error");
        assert_eq!(err.code(), "unknown_fulltext_backend");
        assert!(err.message().to_ascii_lowercase().contains("tantivy"));
    }
}

// §P5-A pa5.1: the hand-rolled backend wired through the trait. The tantivy
// alternative lives in `crate::fulltext_tantivy` behind the `tantivy` feature.
impl FullTextBackend for FullTextIndex {
    fn upsert(&mut self, doc_id: &str, text: &str) {
        FullTextIndex::upsert(self, doc_id, text);
    }

    fn remove(&mut self, doc_id: &str) {
        FullTextIndex::remove(self, doc_id);
    }

    fn search(&self, query: &str, k: usize) -> Vec<(String, f32)> {
        FullTextIndex::search(self, query, k)
    }

    fn designation(&self) -> &FullTextDesignation {
        &self.designation
    }

    fn doc_count(&self) -> usize {
        FullTextIndex::doc_count(self)
    }
}

/// §P5-A pa5.3: env-switch factory. Reads `RUSTY_RED_FULLTEXT_BACKEND` and
/// forwards to the pure dispatcher below. The pure dispatcher exists so unit
/// tests can exercise every code path without racing on a process-global env
/// var; this thin wrapper is the production entry point.
pub fn make_fulltext_backend(
    designation: FullTextDesignation,
) -> Result<Box<dyn FullTextBackend>, FullTextBackendError> {
    let raw = std::env::var(RUSTY_RED_FULLTEXT_BACKEND_ENV).unwrap_or_default();
    make_fulltext_backend_from_value(designation, &raw)
}

/// Pure dispatcher; takes the env-var value as an explicit parameter so unit
/// tests can run in parallel without mutating global state. Default ("" /
/// "hand_rolled" / "hand-rolled" / "bm25") returns the BM25 impl. `"tantivy"`
/// returns the tantivy-backed impl when compiled with `--features tantivy`;
/// otherwise an explicit error so the caller knows to rebuild.
pub fn make_fulltext_backend_from_value(
    designation: FullTextDesignation,
    raw: &str,
) -> Result<Box<dyn FullTextBackend>, FullTextBackendError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | FULLTEXT_BACKEND_HAND_ROLLED | "hand-rolled" | "bm25" => {
            Ok(Box::new(FullTextIndex::for_designation(designation)))
        }
        FULLTEXT_BACKEND_TANTIVY => {
            #[cfg(feature = "tantivy")]
            {
                Ok(Box::new(
                    crate::fulltext_tantivy::TantivyFullTextBackend::new(designation)
                        .map_err(FullTextBackendError::TantivyInit)?,
                ))
            }
            #[cfg(not(feature = "tantivy"))]
            {
                let _ = designation;
                Err(FullTextBackendError::UnknownBackend(
                    "tantivy backend requires building with --features tantivy".to_string(),
                ))
            }
        }
        other => Err(FullTextBackendError::UnknownBackend(format!(
            "unknown {RUSTY_RED_FULLTEXT_BACKEND_ENV} value: {other}"
        ))),
    }
}

#[derive(Debug, Clone)]
pub enum FullTextBackendError {
    UnknownBackend(String),
    TantivyInit(String),
}

impl FullTextBackendError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::UnknownBackend(_) => "unknown_fulltext_backend",
            Self::TantivyInit(_) => "tantivy_init_failed",
        }
    }
    pub fn message(&self) -> String {
        match self {
            Self::UnknownBackend(s) => s.clone(),
            Self::TantivyInit(s) => format!("tantivy initialization failed: {s}"),
        }
    }
}

impl std::fmt::Display for FullTextBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for FullTextBackendError {}
