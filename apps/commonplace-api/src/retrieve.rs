//! Ask-over-your-store: unified retrieval (plan unit I1).
//!
//! Points a unified retrieve at the CommonPlace consumer store so a question
//! answers from everything saved, with provenance. Three arms fused with
//! reciprocal-rank fusion (RRF):
//! - vector: the substrate's embedding index (via the F2 ingest pipeline's search);
//! - lexical: an in-crate idf-weighted token-overlap scorer over item text;
//! - graph: relevance propagation over the F2 `SIMILAR_TO` edges from the
//!   strongest vector/lexical seeds.
//!
//! The answer itself comes from an [`AnswerModel`] seam; with no model configured
//! ([`NoModel`]) the result is an honest extractive answer drawn from the top
//! items (still grounded and fully traceable). A generative model (GL-Fusion on
//! RunPod) drops in behind the same seam.
//!
//! Scope notes (surfaced): the lexical arm is an in-crate scorer, not the core
//! `FullTextIndex`/tantivy backend (the native-FTS upgrade path); the graph arm
//! is `SIMILAR_TO` propagation, not full personalized PageRank (the PPR upgrade
//! path). Both are named follow-ups; the seam and fusion are real.

use std::collections::{HashMap, HashSet};

use commonplace::{
    BlobStore, Commonplace, EmbeddingGraphStore, IngestPipeline, Item, ItemBody, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{GraphStoreResult, NeighborQuery};

/// One retrieved item with its fused score and the arms that surfaced it.
#[derive(Clone, Debug)]
pub struct RetrievedItem {
    pub item: Item,
    pub score: f64,
    pub arms: Vec<String>,
}

/// How the answer was produced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnswerKind {
    Model,
    Extractive,
    Empty,
}

/// The result of an ask: an answer plus the items it is grounded in.
#[derive(Clone, Debug)]
pub struct AskResult {
    pub answer: String,
    pub answer_kind: AnswerKind,
    pub provenance: Vec<RetrievedItem>,
}

/// The answer-synthesis seam. A generative model implements this; the default
/// [`NoModel`] returns `None` so the caller falls back to an extractive answer.
pub trait AnswerModel: Send + Sync {
    fn synthesize(&self, question: &str, context: &[RetrievedItem]) -> Option<String>;
}

/// The honest default: no generative model configured.
pub struct NoModel;

impl AnswerModel for NoModel {
    fn synthesize(&self, _question: &str, _context: &[RetrievedItem]) -> Option<String> {
        None
    }
}

/// Tuning for unified retrieval.
#[derive(Clone, Debug)]
pub struct AskConfig {
    /// Number of provenance items to return.
    pub k: usize,
    /// Per-arm candidate pool depth before fusion.
    pub pool: usize,
    /// RRF damping constant (standard is 60).
    pub rrf_k: f64,
    /// How many top seeds (per arm) feed the graph-propagation arm.
    pub graph_seeds: usize,
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            k: 5,
            pool: 20,
            rrf_k: 60.0,
            graph_seeds: 5,
        }
    }
}

/// Run unified retrieval over the consumer store and synthesize an answer.
pub fn ask<S, B>(
    cp: &Commonplace<S, B>,
    model: &dyn AnswerModel,
    question: &str,
    config: &AskConfig,
) -> GraphStoreResult<AskResult>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    // Arm 1: vector (semantic) over the engine embedding index.
    let vector: Vec<String> = IngestPipeline::default()
        .search(cp, question, config.pool)?
        .into_iter()
        .map(|(id, _distance)| id)
        .collect();

    // Arm 2: lexical (exact-term) over all item text.
    let items = cp.all_items()?;
    let lexical = lexical_rank(question, &items, config.pool);

    // Arm 3: graph propagation over SIMILAR_TO from the strongest seeds.
    let mut seeds: Vec<String> = Vec::new();
    seeds.extend(vector.iter().take(config.graph_seeds).cloned());
    seeds.extend(lexical.iter().take(config.graph_seeds).cloned());
    let graph = graph_rank(cp, &seeds, config.pool);

    // Reciprocal-rank fusion of the three ranked lists.
    let mut fused: HashMap<String, (f64, Vec<String>)> = HashMap::new();
    for (arm, list) in [
        ("vector", &vector),
        ("lexical", &lexical),
        ("graph", &graph),
    ] {
        for (rank, id) in list.iter().enumerate() {
            let entry = fused.entry(id.clone()).or_insert_with(|| (0.0, Vec::new()));
            entry.0 += 1.0 / (config.rrf_k + (rank as f64) + 1.0);
            if !entry.1.iter().any(|a| a == arm) {
                entry.1.push(arm.to_string());
            }
        }
    }
    let mut ranked: Vec<(String, f64, Vec<String>)> = fused
        .into_iter()
        .map(|(id, (score, arms))| (id, score, arms))
        .collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(config.k);

    let mut provenance = Vec::with_capacity(ranked.len());
    for (id, score, arms) in ranked {
        if let Some(item) = cp.get_item(&id)? {
            provenance.push(RetrievedItem { item, score, arms });
        }
    }

    let (answer, answer_kind) = match model.synthesize(question, &provenance) {
        Some(answer) => (answer, AnswerKind::Model),
        None if provenance.is_empty() => (
            "No matching items were found in your store.".to_string(),
            AnswerKind::Empty,
        ),
        None => (extractive_answer(&provenance), AnswerKind::Extractive),
    };

    Ok(AskResult {
        answer,
        answer_kind,
        provenance,
    })
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| token.len() >= 2)
        .map(|token| token.to_lowercase())
        .collect()
}

fn item_text(item: &Item) -> String {
    let mut text = item.title.clone();
    if let ItemBody::Inline { text: body } = &item.body {
        text.push(' ');
        text.push_str(body);
    }
    if let Some(classification) = &item.classification {
        text.push(' ');
        text.push_str(classification);
    }
    for tag in &item.tags {
        text.push(' ');
        text.push_str(tag);
    }
    text
}

fn lexical_rank(question: &str, items: &[Item], pool: usize) -> Vec<String> {
    let query: HashSet<String> = tokenize(question).into_iter().collect();
    if query.is_empty() || items.is_empty() {
        return Vec::new();
    }
    let docs: Vec<(String, HashSet<String>)> = items
        .iter()
        .map(|item| {
            (
                item.id.clone(),
                tokenize(&item_text(item)).into_iter().collect(),
            )
        })
        .collect();
    let total = docs.len() as f64;
    let mut document_frequency: HashMap<String, f64> = HashMap::new();
    for (_, tokens) in &docs {
        for token in tokens {
            *document_frequency.entry(token.clone()).or_insert(0.0) += 1.0;
        }
    }
    let mut scored: Vec<(String, f64)> = docs
        .iter()
        .map(|(id, tokens)| {
            let score = query
                .iter()
                .filter(|token| tokens.contains(*token))
                .map(|token| {
                    let df = document_frequency.get(token).copied().unwrap_or(1.0);
                    (total / (1.0 + df)).ln().max(0.0001)
                })
                .sum::<f64>();
            (id.clone(), score)
        })
        .filter(|(_, score)| *score > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    scored.truncate(pool);
    scored.into_iter().map(|(id, _)| id).collect()
}

fn graph_rank<S, B>(cp: &Commonplace<S, B>, seeds: &[String], pool: usize) -> Vec<String>
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
    let mut accumulated: HashMap<String, f64> = HashMap::new();
    for (rank, seed) in seeds.iter().enumerate() {
        let weight = 1.0 / (rank as f64 + 1.0);
        for direction in [
            NeighborQuery::out(seed).with_edge_type(SIMILAR_TO_EDGE),
            NeighborQuery::in_(seed).with_edge_type(SIMILAR_TO_EDGE),
        ] {
            for hit in cp.store().neighbors(direction) {
                *accumulated.entry(hit.node_id).or_insert(0.0) += weight;
            }
        }
    }
    // The graph arm contributes structural signal: drop the seeds themselves so
    // it surfaces connected-but-not-already-seeded items.
    for seed in seeds {
        accumulated.remove(seed);
    }
    let mut ranked: Vec<(String, f64)> = accumulated.into_iter().collect();
    ranked.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    ranked.truncate(pool);
    ranked.into_iter().map(|(id, _)| id).collect()
}

fn extractive_answer(provenance: &[RetrievedItem]) -> String {
    let titles: Vec<&str> = provenance
        .iter()
        .take(3)
        .map(|hit| hit.item.title.as_str())
        .collect();
    let lead = match &provenance[0].item.body {
        ItemBody::Inline { text } => first_sentence(text),
        _ => provenance[0].item.title.clone(),
    };
    format!(
        "{lead} [grounded in {} item(s): {}; no generative model configured]",
        provenance.len(),
        titles.join(", ")
    )
}

fn first_sentence(text: &str) -> String {
    let trimmed = text.trim();
    match trimmed.find(['.', '!', '?']) {
        Some(idx) => trimmed[..=idx].trim().to_string(),
        None => trimmed.chars().take(200).collect(),
    }
}
