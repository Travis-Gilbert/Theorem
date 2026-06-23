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
//! items (still grounded and fully traceable). A local OpenAI-compatible model
//! (Gemma via RustyRed's resident llama-server path) drops in behind the same
//! seam.
//!
//! Scope notes (surfaced): the lexical arm is an in-crate scorer, not the core
//! `FullTextIndex`/tantivy backend (the native-FTS upgrade path); the graph arm
//! is `SIMILAR_TO` propagation, not full personalized PageRank (the PPR upgrade
//! path). Both are named follow-ups; the seam and fusion are real.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use commonplace::{
    BlobStore, Commonplace, EmbeddingGraphStore, IngestPipeline, Item, ItemBody, SIMILAR_TO_EDGE,
};
use rustyred_thg_core::{GraphStoreResult, NeighborQuery};
use serde_json::{json, Value};

const DEFAULT_LOCAL_OPENAI_CHAT_URL: &str = "http://127.0.0.1:8080/v1/chat/completions";
const DEFAULT_GEMMA_MODEL: &str = "gemma-4-12b-it-q4";
const DEFAULT_MODEL_TIMEOUT_SECS: u64 = 90;
const DEFAULT_MODEL_MAX_TOKENS: u32 = 700;
const DEFAULT_MODEL_TEMPERATURE: f32 = 0.2;

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

/// OpenAI-compatible answer model for the local RustyRed/Gemma runtime.
///
/// The model is intentionally optional: request errors return `None`, preserving
/// the extractive answer path when the resident model is not running.
pub struct LocalOpenAiAnswerModel {
    chat_url: String,
    model: String,
    api_key: Option<String>,
    temperature: f32,
    max_tokens: u32,
    timeout: Duration,
}

impl LocalOpenAiAnswerModel {
    pub fn new(
        endpoint: impl AsRef<str>,
        model: impl Into<String>,
        api_key: Option<String>,
        temperature: f32,
        max_tokens: u32,
        timeout: Duration,
    ) -> Result<Self, String> {
        Ok(Self {
            chat_url: chat_completions_url(endpoint.as_ref()),
            model: model.into(),
            api_key,
            temperature,
            max_tokens,
            timeout,
        })
    }

    pub fn from_env() -> Option<Self> {
        if !env_flag("COMMONPLACE_ASK_MODEL_ENABLED", true) {
            return None;
        }

        let endpoint = first_env(&["COMMONPLACE_ASK_MODEL_URL", "THEOREM_LOCAL_OPENAI_URL"])
            .or_else(|| {
                env_flag("COMMONPLACE_ASK_MODEL_AUTO", true)
                    .then(|| DEFAULT_LOCAL_OPENAI_CHAT_URL.to_string())
            })?;
        let model = first_env(&[
            "COMMONPLACE_ASK_MODEL",
            "COMMONPLACE_GEMMA_MODEL",
            "THEOREM_LOCAL_OPENAI_MODEL",
            "AGENTD_SMOKE_MODEL",
        ])
        .unwrap_or_else(|| DEFAULT_GEMMA_MODEL.to_string());
        let api_key = first_env(&["COMMONPLACE_ASK_MODEL_API_KEY", "THEOREM_MODEL_API_KEY"]);
        let timeout = Duration::from_secs(env_u64(
            "COMMONPLACE_ASK_MODEL_TIMEOUT_SECS",
            DEFAULT_MODEL_TIMEOUT_SECS,
        ));
        let max_tokens = env_u32("COMMONPLACE_ASK_MODEL_MAX_TOKENS", DEFAULT_MODEL_MAX_TOKENS);
        let temperature = env_f32(
            "COMMONPLACE_ASK_MODEL_TEMPERATURE",
            DEFAULT_MODEL_TEMPERATURE,
        );

        Self::new(endpoint, model, api_key, temperature, max_tokens, timeout).ok()
    }
}

impl AnswerModel for LocalOpenAiAnswerModel {
    fn synthesize(&self, question: &str, context: &[RetrievedItem]) -> Option<String> {
        if context.is_empty() {
            return None;
        }

        let body = json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": "You answer questions for CommonPlace using only the provided grounding items. Be concise, faithful to the sources, and say what is missing when the grounding is thin."
                },
                {
                    "role": "user",
                    "content": grounded_prompt(question, context)
                }
            ],
            "temperature": self.temperature,
            "max_tokens": self.max_tokens,
            "stream": false
        });

        let chat_url = self.chat_url.clone();
        let api_key = self.api_key.clone();
        let timeout = self.timeout;
        std::thread::spawn(move || {
            let http = reqwest::blocking::Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(timeout)
                .build()
                .ok()?;
            let mut request = http.post(chat_url).json(&body);
            if let Some(api_key) = api_key {
                request = request.bearer_auth(api_key);
            }
            let value: Value = request.send().ok()?.error_for_status().ok()?.json().ok()?;
            parse_chat_content(&value)
        })
        .join()
        .ok()
        .flatten()
    }
}

/// Build the configured answer model for HTTP serving.
///
/// By default this tries RustyRed's local OpenAI-compatible Gemma endpoint at
/// `127.0.0.1:8080`; set `COMMONPLACE_ASK_MODEL_ENABLED=0` or
/// `COMMONPLACE_ASK_MODEL_AUTO=0` to keep the API extractive-only unless an
/// explicit model URL is provided.
pub fn answer_model_from_env() -> Arc<dyn AnswerModel> {
    LocalOpenAiAnswerModel::from_env()
        .map(|model| Arc::new(model) as Arc<dyn AnswerModel>)
        .unwrap_or_else(|| Arc::new(NoModel))
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
    let provenance = retrieve_grounding(cp, question, config)?;
    Ok(answer_from_provenance(model, question, provenance))
}

/// Retrieve the grounding set for a question without running answer synthesis.
pub fn retrieve_grounding<S, B>(
    cp: &Commonplace<S, B>,
    question: &str,
    config: &AskConfig,
) -> GraphStoreResult<Vec<RetrievedItem>>
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

    Ok(provenance)
}

/// Synthesize an ask result from already-retrieved provenance.
pub fn answer_from_provenance(
    model: &dyn AnswerModel,
    question: &str,
    provenance: Vec<RetrievedItem>,
) -> AskResult {
    let (answer, answer_kind) = match model.synthesize(question, &provenance) {
        Some(answer) => (answer, AnswerKind::Model),
        None if provenance.is_empty() => (
            "No matching items were found in your store.".to_string(),
            AnswerKind::Empty,
        ),
        None => (extractive_answer(&provenance), AnswerKind::Extractive),
    };

    AskResult {
        answer,
        answer_kind,
        provenance,
    }
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

fn grounded_prompt(question: &str, context: &[RetrievedItem]) -> String {
    let mut prompt = format!(
        "Question:\n{question}\n\nGrounding items:\n{}\n\nAnswer in 2-6 sentences. Include item titles when they help the user trace the answer.",
        context
            .iter()
            .enumerate()
            .map(|(idx, hit)| grounding_item(idx + 1, hit))
            .collect::<Vec<_>>()
            .join("\n\n")
    );
    if prompt.chars().count() > 24_000 {
        prompt = prompt.chars().take(24_000).collect();
        prompt.push_str("\n\n[grounding truncated]");
    }
    prompt
}

fn grounding_item(index: usize, hit: &RetrievedItem) -> String {
    let item = &hit.item;
    let mut lines = vec![
        format!("[{index}] title: {}", item.title),
        format!("id: {}", item.id),
        format!("kind: {}", item.kind.as_str()),
    ];
    if let Some(source) = &item.source {
        lines.push(format!("source: {source}"));
    }
    if !item.tags.is_empty() {
        lines.push(format!("tags: {}", item.tags.join(", ")));
    }
    if let Some(classification) = &item.classification {
        lines.push(format!("classification: {classification}"));
    }
    let body = match &item.body {
        ItemBody::Inline { text } => truncate_chars(text.trim(), 1800),
        ItemBody::Blob { mime, .. } => mime
            .as_deref()
            .map(|mime| format!("[blob item; mime: {mime}]"))
            .unwrap_or_else(|| "[blob item]".to_string()),
        ItemBody::Empty => "[empty item]".to_string(),
    };
    lines.push(format!("text: {body}"));
    lines.push(format!(
        "retrieval: score {:.4}, arms {}",
        hit.score,
        hit.arms.join(", ")
    ));
    lines.join("\n")
}

fn truncate_chars(text: &str, limit: usize) -> String {
    let mut output: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        output.push_str("...");
    }
    output
}

fn parse_chat_content(value: &Value) -> Option<String> {
    if value.get("error").is_some() {
        return None;
    }
    let content = value
        .get("choices")
        .and_then(Value::as_array)
        .and_then(|choices| choices.first())
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(Value::as_str)?
        .trim();
    (!content.is_empty()).then(|| content.to_string())
}

fn chat_completions_url(endpoint: &str) -> String {
    let trimmed = endpoint.trim().trim_end_matches('/');
    if trimmed.ends_with("/chat/completions") {
        trimmed.to_string()
    } else if trimmed.ends_with("/v1") {
        format!("{trimmed}/chat/completions")
    } else {
        format!("{trimmed}/v1/chat/completions")
    }
}

fn first_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .filter(|value| !value.trim().is_empty())
    })
}

fn env_flag(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_u32(name: &str, default: u32) -> u32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn chat_url_accepts_base_v1_or_full_endpoint() {
        assert_eq!(
            chat_completions_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://127.0.0.1:8080/v1"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
        assert_eq!(
            chat_completions_url("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080/v1/chat/completions"
        );
    }

    #[test]
    fn local_openai_model_posts_grounded_prompt_and_reads_answer() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut headers = String::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                headers.push_str(&line);
            }
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap();
            let mut body = vec![0; content_length];
            reader.read_exact(&mut body).unwrap();
            let payload =
                r#"{"choices":[{"message":{"content":"Gemma answered from grounded notes."}}]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                payload.len(),
                payload
            );
            stream.write_all(response.as_bytes()).unwrap();
            String::from_utf8(body).unwrap()
        });

        let model = LocalOpenAiAnswerModel::new(
            format!("http://{addr}/v1"),
            "test-gemma",
            None,
            0.1,
            64,
            Duration::from_secs(5),
        )
        .unwrap();
        let context = vec![RetrievedItem {
            item: Item::note(
                "Gemma note",
                "The local Gemma model should synthesize grounded answers.",
            ),
            score: 1.0,
            arms: vec!["lexical".to_string()],
        }];

        let answer = model.synthesize("what should answer?", &context).unwrap();
        let body = handle.join().unwrap();

        assert_eq!(answer, "Gemma answered from grounded notes.");
        assert!(
            body.contains("test-gemma"),
            "body should include model: {body}"
        );
        assert!(
            body.contains("Gemma note"),
            "body should include grounding title: {body}"
        );
        assert!(
            body.contains("what should answer?"),
            "body should include the question: {body}"
        );
    }
}
