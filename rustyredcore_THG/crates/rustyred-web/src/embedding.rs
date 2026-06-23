use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::time::Duration;

use futures_util::future::BoxFuture;
use rustyred_hipporag::{HippoError, HippoResult, HippoTextEmbedder};
use rustyred_thg_core::{GraphMutation, VectorDesignation};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{CrawlGraph, EDGE_HAS_SNAPSHOT, LABEL_CONTENT_SNAPSHOT, LABEL_PAGE};

pub const QWEN3_EMBEDDING_4B_MODEL_ID: &str = "Qwen/Qwen3-Embedding-4B";
pub const QWEN3_EMBEDDING_4B_DIMENSION: usize = 2560;
pub const SEMANTIC_VECTOR_PROPERTY: &str = "semantic_vec";
pub const SEMANTIC_VECTOR_METRIC: &str = "cosine";
pub const DEFAULT_EMBEDDING_TEXT_BYTES: usize = 8192;

const EMBED_ENDPOINT_ENV_KEYS: &[&str] = &[
    "RUSTYWEB_QWEN4B_EMBED_URL",
    "RUSTYWEB_QWEN3_EMBEDDING_4B_URL",
    "RUSTY_RED_QWEN4B_EMBED_URL",
    "RUNPOD_QWEN3_EMBED_URL",
    "QWEN3_EMBEDDING_4B_URL",
];
const EMBED_MODEL_ENV_KEYS: &[&str] = &[
    "RUSTYWEB_QWEN4B_MODEL_ID",
    "RUSTYWEB_QWEN3_EMBEDDING_4B_MODEL_ID",
    "QWEN3_EMBEDDING_4B_MODEL_ID",
];
const EMBED_DIMENSION_ENV_KEYS: &[&str] = &[
    "RUSTYWEB_QWEN4B_DIMENSION",
    "RUSTYWEB_QWEN3_EMBEDDING_4B_DIMENSION",
    "QWEN3_EMBEDDING_4B_DIMENSION",
];
const EMBED_BATCH_ENV_KEYS: &[&str] = &[
    "RUSTYWEB_QWEN4B_BATCH_SIZE",
    "RUSTYWEB_QWEN3_EMBEDDING_4B_BATCH_SIZE",
    "QWEN3_EMBEDDING_4B_BATCH_SIZE",
];
const EMBED_TIMEOUT_ENV_KEYS: &[&str] = &[
    "RUSTYWEB_QWEN4B_TIMEOUT_SECONDS",
    "RUSTYWEB_QWEN3_EMBEDDING_4B_TIMEOUT_SECONDS",
    "QWEN3_EMBEDDING_4B_TIMEOUT_SECONDS",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EmbeddingModelContract {
    pub model_id: String,
    pub label: String,
    pub property: String,
    pub dimension: usize,
    pub metric: String,
    pub normalized: bool,
}

impl EmbeddingModelContract {
    pub fn vector_designation(&self) -> VectorDesignation {
        VectorDesignation {
            label: self.label.clone(),
            property: self.property.clone(),
            dimension: self.dimension,
        }
    }
}

pub fn qwen3_embedding_4b_contract() -> EmbeddingModelContract {
    EmbeddingModelContract {
        model_id: QWEN3_EMBEDDING_4B_MODEL_ID.to_string(),
        label: LABEL_PAGE.to_string(),
        property: SEMANTIC_VECTOR_PROPERTY.to_string(),
        dimension: QWEN3_EMBEDDING_4B_DIMENSION,
        metric: SEMANTIC_VECTOR_METRIC.to_string(),
        normalized: true,
    }
}

pub fn qwen3_embedding_4b_vector_designation() -> VectorDesignation {
    qwen3_embedding_4b_contract().vector_designation()
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CrawlEmbeddingReceipt {
    pub model_id: String,
    pub label: String,
    pub property: String,
    pub dimension: usize,
    pub metric: String,
    pub normalized: bool,
    pub embedded_pages: usize,
    pub skipped_pages: usize,
    pub vector_designation: VectorDesignation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmbeddingRequestFormat {
    Auto,
    OpenAi,
    TextEmbeddingsInference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QwenEmbeddingConfig {
    pub endpoint: String,
    pub model_id: String,
    pub dimension: usize,
    pub batch_size: usize,
    pub timeout_seconds: u64,
    pub request_format: EmbeddingRequestFormat,
}

impl QwenEmbeddingConfig {
    pub fn from_env() -> Result<Option<Self>, EmbeddingError> {
        let Some(endpoint) = first_env(EMBED_ENDPOINT_ENV_KEYS) else {
            return Ok(None);
        };
        let model_id = first_env(EMBED_MODEL_ENV_KEYS)
            .unwrap_or_else(|| QWEN3_EMBEDDING_4B_MODEL_ID.to_string());
        let dimension = parse_env_usize(
            EMBED_DIMENSION_ENV_KEYS,
            QWEN3_EMBEDDING_4B_DIMENSION,
            "embedding dimension",
        )?;
        let batch_size = parse_env_usize(EMBED_BATCH_ENV_KEYS, 16, "embedding batch size")?.max(1);
        let timeout_seconds =
            parse_env_u64(EMBED_TIMEOUT_ENV_KEYS, 60, "embedding timeout seconds")?.max(1);
        let request_format = parse_request_format(
            env::var("RUSTYWEB_QWEN4B_REQUEST_FORMAT")
                .or_else(|_| env::var("QWEN3_EMBEDDING_4B_REQUEST_FORMAT"))
                .ok()
                .as_deref(),
        )?;
        Ok(Some(Self {
            endpoint,
            model_id,
            dimension,
            batch_size,
            timeout_seconds,
            request_format,
        }))
    }
}

pub fn configured_qwen3_embedding_4b_client_from_env(
) -> Result<Option<QwenEmbeddingClient>, EmbeddingError> {
    QwenEmbeddingConfig::from_env()?
        .map(QwenEmbeddingClient::new)
        .transpose()
}

#[derive(Clone)]
pub struct QwenEmbeddingClient {
    config: QwenEmbeddingConfig,
    client: reqwest::Client,
}

impl QwenEmbeddingClient {
    pub fn new(config: QwenEmbeddingConfig) -> Result<Self, EmbeddingError> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(config.timeout_seconds))
            .build()
            .map_err(|err| EmbeddingError::Http {
                reason: format!("failed to build embedding HTTP client: {err}"),
            })?;
        Ok(Self { config, client })
    }

    pub fn config(&self) -> &QwenEmbeddingConfig {
        &self.config
    }

    fn request_format(&self) -> EmbeddingRequestFormat {
        match self.config.request_format {
            EmbeddingRequestFormat::Auto
                if self.config.endpoint.contains("/v1/embeddings")
                    || self.config.endpoint.contains("/embeddings") =>
            {
                EmbeddingRequestFormat::OpenAi
            }
            EmbeddingRequestFormat::Auto => EmbeddingRequestFormat::TextEmbeddingsInference,
            format => format,
        }
    }

    fn request_payload(&self, inputs: &[String]) -> Value {
        match self.request_format() {
            EmbeddingRequestFormat::OpenAi => json!({
                "model": self.config.model_id,
                "input": inputs,
            }),
            EmbeddingRequestFormat::TextEmbeddingsInference | EmbeddingRequestFormat::Auto => {
                json!({
                    "model": self.config.model_id,
                    "inputs": inputs,
                })
            }
        }
    }
}

impl TextEmbedder for QwenEmbeddingClient {
    fn model_id(&self) -> &str {
        &self.config.model_id
    }

    fn dimension(&self) -> usize {
        self.config.dimension
    }

    fn embed<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbeddingError>> {
        Box::pin(async move {
            let mut all_embeddings = Vec::with_capacity(inputs.len());
            for chunk in inputs.chunks(self.config.batch_size) {
                let response = self
                    .client
                    .post(&self.config.endpoint)
                    .json(&self.request_payload(chunk))
                    .send()
                    .await
                    .map_err(|err| EmbeddingError::Http {
                        reason: format!("embedding request failed: {err}"),
                    })?;
                let status = response.status();
                let body = response.text().await.map_err(|err| EmbeddingError::Http {
                    reason: format!("failed to read embedding response: {err}"),
                })?;
                if !status.is_success() {
                    return Err(EmbeddingError::Http {
                        reason: format!("embedding endpoint returned {status}: {body}"),
                    });
                }
                let value = serde_json::from_str::<Value>(&body).map_err(|err| {
                    EmbeddingError::Response {
                        reason: format!("embedding endpoint returned non-JSON response: {err}"),
                    }
                })?;
                let mut embeddings = parse_embedding_response(&value)?;
                if embeddings.len() != chunk.len() {
                    return Err(EmbeddingError::Response {
                        reason: format!(
                            "embedding endpoint returned {} vectors for {} inputs",
                            embeddings.len(),
                            chunk.len()
                        ),
                    });
                }
                for vector in &mut embeddings {
                    validate_embedding_dimension(vector, self.config.dimension)?;
                    normalize_vector_in_place(vector);
                }
                all_embeddings.extend(embeddings);
            }
            Ok(all_embeddings)
        })
    }
}

impl HippoTextEmbedder for QwenEmbeddingClient {
    fn model_id(&self) -> &str {
        TextEmbedder::model_id(self)
    }

    fn dimension(&self) -> usize {
        TextEmbedder::dimension(self)
    }

    fn property(&self) -> &str {
        TextEmbedder::property(self)
    }

    fn metric(&self) -> &str {
        TextEmbedder::metric(self)
    }

    fn embed<'a>(&'a self, inputs: &'a [String]) -> BoxFuture<'a, HippoResult<Vec<Vec<f32>>>> {
        Box::pin(async move {
            TextEmbedder::embed(self, inputs)
                .await
                .map_err(|error| HippoError::new("embedding", error.to_string()))
        })
    }
}

pub trait TextEmbedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn dimension(&self) -> usize;
    fn property(&self) -> &str {
        SEMANTIC_VECTOR_PROPERTY
    }
    fn metric(&self) -> &str {
        SEMANTIC_VECTOR_METRIC
    }
    fn normalized(&self) -> bool {
        true
    }
    fn embed<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbeddingError>>;
}

#[derive(Clone, Debug, PartialEq)]
pub struct StaticTextEmbedder {
    model_id: String,
    dimension: usize,
    vectors: Vec<Vec<f32>>,
}

impl StaticTextEmbedder {
    pub fn new(model_id: impl Into<String>, dimension: usize, vectors: Vec<Vec<f32>>) -> Self {
        Self {
            model_id: model_id.into(),
            dimension,
            vectors,
        }
    }
}

impl TextEmbedder for StaticTextEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn embed<'a>(
        &'a self,
        inputs: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<Vec<f32>>, EmbeddingError>> {
        Box::pin(async move {
            let embeddings = if self.vectors.len() == 1 && inputs.len() > 1 {
                inputs
                    .iter()
                    .map(|_| self.vectors[0].clone())
                    .collect::<Vec<_>>()
            } else {
                self.vectors.clone()
            };
            if embeddings.len() != inputs.len() {
                return Err(EmbeddingError::Response {
                    reason: format!(
                        "static embedder has {} vectors for {} inputs",
                        embeddings.len(),
                        inputs.len()
                    ),
                });
            }
            for vector in &embeddings {
                validate_embedding_dimension(vector, self.dimension)?;
            }
            Ok(embeddings)
        })
    }
}

impl HippoTextEmbedder for StaticTextEmbedder {
    fn model_id(&self) -> &str {
        TextEmbedder::model_id(self)
    }

    fn dimension(&self) -> usize {
        TextEmbedder::dimension(self)
    }

    fn property(&self) -> &str {
        TextEmbedder::property(self)
    }

    fn metric(&self) -> &str {
        TextEmbedder::metric(self)
    }

    fn embed<'a>(&'a self, inputs: &'a [String]) -> BoxFuture<'a, HippoResult<Vec<Vec<f32>>>> {
        Box::pin(async move {
            TextEmbedder::embed(self, inputs)
                .await
                .map_err(|error| HippoError::new("embedding", error.to_string()))
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EmbeddingError {
    InvalidConfig { reason: String },
    Http { reason: String },
    Response { reason: String },
    DimensionMismatch { expected: usize, actual: usize },
}

impl fmt::Display for EmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidConfig { reason } => write!(f, "invalid embedding config: {reason}"),
            Self::Http { reason } => write!(f, "embedding HTTP error: {reason}"),
            Self::Response { reason } => write!(f, "embedding response error: {reason}"),
            Self::DimensionMismatch { expected, actual } => {
                write!(
                    f,
                    "embedding dimension mismatch: expected {expected}, got {actual}"
                )
            }
        }
    }
}

impl std::error::Error for EmbeddingError {}

pub async fn embed_crawl_graph_pages(
    graph: &mut CrawlGraph,
    embedder: &dyn TextEmbedder,
) -> Result<CrawlEmbeddingReceipt, EmbeddingError> {
    let candidates = page_embedding_candidates(graph);
    let inputs = candidates
        .iter()
        .map(|candidate| candidate.text.clone())
        .collect::<Vec<_>>();
    let mut embeddings = embedder.embed(&inputs).await?;
    if embeddings.len() != candidates.len() {
        return Err(EmbeddingError::Response {
            reason: format!(
                "embedder returned {} vectors for {} page candidates",
                embeddings.len(),
                candidates.len()
            ),
        });
    }

    for (candidate, vector) in candidates.iter().zip(embeddings.iter_mut()) {
        validate_embedding_dimension(vector, embedder.dimension())?;
        normalize_vector_in_place(vector);
        if let Some(GraphMutation::NodeUpsert(node)) =
            graph.batch.mutations.get_mut(candidate.mutation_index)
        {
            let props = object_props(&mut node.properties);
            props.insert(embedder.property().to_string(), json!(vector));
            props.insert(
                format!("{}_model", embedder.property()),
                json!(embedder.model_id()),
            );
            props.insert(
                format!("{}_dimension", embedder.property()),
                json!(embedder.dimension()),
            );
            props.insert(
                format!("{}_metric", embedder.property()),
                json!(embedder.metric()),
            );
            props.insert(
                format!("{}_normalized", embedder.property()),
                json!(embedder.normalized()),
            );
        }
    }

    let page_count = graph
        .batch
        .mutations
        .iter()
        .filter(|mutation| match mutation {
            GraphMutation::NodeUpsert(node) => node.labels.iter().any(|label| label == LABEL_PAGE),
            GraphMutation::EdgeUpsert(_) => false,
        })
        .count();
    let vector_designation = VectorDesignation {
        label: LABEL_PAGE.to_string(),
        property: embedder.property().to_string(),
        dimension: embedder.dimension(),
    };
    Ok(CrawlEmbeddingReceipt {
        model_id: embedder.model_id().to_string(),
        label: LABEL_PAGE.to_string(),
        property: embedder.property().to_string(),
        dimension: embedder.dimension(),
        metric: embedder.metric().to_string(),
        normalized: embedder.normalized(),
        embedded_pages: candidates.len(),
        skipped_pages: page_count.saturating_sub(candidates.len()),
        vector_designation,
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PageEmbeddingCandidate {
    mutation_index: usize,
    text: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PageTextParts {
    mutation_index: usize,
    url: String,
    title: Option<String>,
    page_state: Option<String>,
    snapshots: Vec<String>,
}

fn page_embedding_candidates(graph: &CrawlGraph) -> Vec<PageEmbeddingCandidate> {
    let mut pages = BTreeMap::<String, PageTextParts>::new();
    let mut snapshot_text = BTreeMap::<String, String>::new();
    let mut snapshot_page = BTreeMap::<String, String>::new();

    for (index, mutation) in graph.batch.mutations.iter().enumerate() {
        match mutation {
            GraphMutation::NodeUpsert(node) if node.labels.iter().any(|l| l == LABEL_PAGE) => {
                let Some(url) = property_string(&node.properties, "url")
                    .or_else(|| property_string(&node.properties, "canonical_url"))
                else {
                    continue;
                };
                pages.insert(
                    node.id.clone(),
                    PageTextParts {
                        mutation_index: index,
                        url,
                        title: property_string(&node.properties, "title"),
                        page_state: property_string(&node.properties, "page_state"),
                        snapshots: Vec::new(),
                    },
                );
            }
            GraphMutation::NodeUpsert(node)
                if node.labels.iter().any(|l| l == LABEL_CONTENT_SNAPSHOT) =>
            {
                if let Some(text) = property_string(&node.properties, "text") {
                    snapshot_text.insert(node.id.clone(), text);
                }
            }
            GraphMutation::EdgeUpsert(edge) if edge.edge_type == EDGE_HAS_SNAPSHOT => {
                snapshot_page.insert(edge.to_id.clone(), edge.from_id.clone());
            }
            GraphMutation::NodeUpsert(_) | GraphMutation::EdgeUpsert(_) => {}
        }
    }

    for (snapshot_id, page_id) in snapshot_page {
        if let (Some(page), Some(text)) = (pages.get_mut(&page_id), snapshot_text.get(&snapshot_id))
        {
            page.snapshots.push(text.clone());
        }
    }

    pages
        .into_values()
        .filter_map(|page| {
            if page.page_state.as_deref() == Some("alias") {
                return None;
            }
            let text = page_embedding_text(&page)?;
            Some(PageEmbeddingCandidate {
                mutation_index: page.mutation_index,
                text,
            })
        })
        .collect()
}

fn page_embedding_text(page: &PageTextParts) -> Option<String> {
    let mut parts = Vec::new();
    push_nonempty(&mut parts, page.title.as_deref());
    push_nonempty(&mut parts, Some(&page.url));
    for snapshot in &page.snapshots {
        push_nonempty(&mut parts, Some(snapshot));
    }
    let joined = parts.join("\n");
    let trimmed = joined.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(bounded_text(trimmed, DEFAULT_EMBEDDING_TEXT_BYTES))
    }
}

fn push_nonempty(parts: &mut Vec<String>, value: Option<&str>) {
    if let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) {
        parts.push(value.to_string());
    }
}

fn bounded_text(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn property_string(properties: &Value, key: &str) -> Option<String> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn object_props(value: &mut Value) -> &mut serde_json::Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("object value just created")
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter()
        .filter_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .find(|value| !value.is_empty())
}

fn parse_env_usize(keys: &[&str], default: usize, label: &str) -> Result<usize, EmbeddingError> {
    let Some(value) = first_env(keys) else {
        return Ok(default);
    };
    value
        .parse::<usize>()
        .map_err(|err| EmbeddingError::InvalidConfig {
            reason: format!("{label} must be an unsigned integer: {err}"),
        })
}

fn parse_env_u64(keys: &[&str], default: u64, label: &str) -> Result<u64, EmbeddingError> {
    let Some(value) = first_env(keys) else {
        return Ok(default);
    };
    value
        .parse::<u64>()
        .map_err(|err| EmbeddingError::InvalidConfig {
            reason: format!("{label} must be an unsigned integer: {err}"),
        })
}

fn parse_request_format(value: Option<&str>) -> Result<EmbeddingRequestFormat, EmbeddingError> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .as_deref()
    {
        None | Some("auto") => Ok(EmbeddingRequestFormat::Auto),
        Some("openai") | Some("open_ai") | Some("vllm") => Ok(EmbeddingRequestFormat::OpenAi),
        Some("tei") | Some("text_embeddings_inference") | Some("text-embeddings-inference") => {
            Ok(EmbeddingRequestFormat::TextEmbeddingsInference)
        }
        Some(other) => Err(EmbeddingError::InvalidConfig {
            reason: format!("unsupported embedding request format {other:?}"),
        }),
    }
}

fn parse_embedding_response(value: &Value) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    if let Some(data) = value.get("data").and_then(Value::as_array) {
        let mut rows = Vec::with_capacity(data.len());
        for (fallback_index, item) in data.iter().enumerate() {
            let embedding = item
                .get("embedding")
                .ok_or_else(|| EmbeddingError::Response {
                    reason: "OpenAI embedding data row is missing embedding".to_string(),
                })?;
            let index = item
                .get("index")
                .and_then(Value::as_u64)
                .map(|index| index as usize)
                .unwrap_or(fallback_index);
            rows.push((index, parse_vector(embedding)?));
        }
        rows.sort_by_key(|(index, _)| *index);
        return Ok(rows.into_iter().map(|(_, vector)| vector).collect());
    }
    if let Some(embeddings) = value.get("embeddings") {
        return parse_embedding_matrix(embeddings);
    }
    if let Some(embedding) = value.get("embedding") {
        return Ok(vec![parse_vector(embedding)?]);
    }
    parse_embedding_matrix(value)
}

fn parse_embedding_matrix(value: &Value) -> Result<Vec<Vec<f32>>, EmbeddingError> {
    let array = value.as_array().ok_or_else(|| EmbeddingError::Response {
        reason: "embedding response did not include an array".to_string(),
    })?;
    if array.first().and_then(Value::as_f64).is_some() {
        return Ok(vec![parse_vector(value)?]);
    }
    array.iter().map(parse_vector).collect()
}

fn parse_vector(value: &Value) -> Result<Vec<f32>, EmbeddingError> {
    let array = value.as_array().ok_or_else(|| EmbeddingError::Response {
        reason: "embedding vector is not an array".to_string(),
    })?;
    let mut vector = Vec::with_capacity(array.len());
    for item in array {
        let Some(number) = item.as_f64() else {
            return Err(EmbeddingError::Response {
                reason: "embedding vector contains a non-number".to_string(),
            });
        };
        vector.push(number as f32);
    }
    Ok(vector)
}

fn validate_embedding_dimension(vector: &[f32], expected: usize) -> Result<(), EmbeddingError> {
    if vector.len() == expected {
        Ok(())
    } else {
        Err(EmbeddingError::DimensionMismatch {
            expected,
            actual: vector.len(),
        })
    }
}

fn normalize_vector_in_place(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm < 1e-10 {
        return;
    }
    for value in vector {
        *value /= norm;
    }
}

#[cfg(test)]
mod tests {
    use rustyred_thg_core::InMemoryGraphStore;

    use crate::{build_v2_fixture_crawl, CrawlRequest, FetchedPage};

    use super::*;

    #[test]
    fn qwen4b_contract_targets_page_semantic_vectors() {
        let contract = qwen3_embedding_4b_contract();

        assert_eq!(contract.model_id, QWEN3_EMBEDDING_4B_MODEL_ID);
        assert_eq!(contract.label, LABEL_PAGE);
        assert_eq!(contract.property, SEMANTIC_VECTOR_PROPERTY);
        assert_eq!(contract.dimension, QWEN3_EMBEDDING_4B_DIMENSION);
        assert_eq!(
            qwen3_embedding_4b_vector_designation(),
            VectorDesignation {
                label: LABEL_PAGE.to_string(),
                property: SEMANTIC_VECTOR_PROPERTY.to_string(),
                dimension: QWEN3_EMBEDDING_4B_DIMENSION,
            }
        );
    }

    #[test]
    fn parses_openai_and_tei_embedding_shapes() {
        let openai = json!({
            "data": [
                { "index": 1, "embedding": [0.0, 1.0] },
                { "index": 0, "embedding": [1.0, 0.0] }
            ]
        });
        let tei = json!({ "embeddings": [[1.0, 0.0], [0.0, 1.0]] });

        assert_eq!(
            parse_embedding_response(&openai).unwrap(),
            vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        );
        assert_eq!(
            parse_embedding_response(&tei).unwrap(),
            vec![vec![1.0, 0.0], vec![0.0, 1.0]]
        );
    }

    #[tokio::test]
    async fn page_embedding_annotation_feeds_turbovec_vector_search() {
        let mut output = build_v2_fixture_crawl(
            CrawlRequest::new(
                "embedding-run",
                vec!["https://example.com/qwen4b-grounding".to_string()],
            ),
            &[FetchedPage::html(
                "https://example.com/qwen4b-grounding",
                "<html><body>Qwen4B semantic crawl grounding for Turbovec</body></html>",
            )],
        )
        .unwrap();
        let embedder = StaticTextEmbedder::new("static-qwen4b-test", 3, vec![vec![2.0, 0.0, 0.0]]);

        let receipt = embed_crawl_graph_pages(&mut output.graph, &embedder)
            .await
            .unwrap();

        assert_eq!(receipt.embedded_pages, 1);
        assert_eq!(receipt.dimension, 3);
        let mut store = InMemoryGraphStore::new();
        store
            .designate_vector_property(LABEL_PAGE, SEMANTIC_VECTOR_PROPERTY, 3)
            .unwrap();
        output.graph.apply_to_store(&mut store).unwrap();

        let hits = store
            .vector_search(
                Some(LABEL_PAGE),
                SEMANTIC_VECTOR_PROPERTY,
                &[1.0, 0.0, 0.0],
                1,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        let page = store.get_node(&hits[0].0).unwrap();
        assert_eq!(
            page.properties.get("url").and_then(Value::as_str).unwrap(),
            "https://example.com/qwen4b-grounding"
        );
        assert_eq!(
            page.properties
                .get("semantic_vec_model")
                .and_then(Value::as_str)
                .unwrap(),
            "static-qwen4b-test"
        );
    }
}
