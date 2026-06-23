//! Shared code embedding seam for file-level and CodeCrawler symbol vectors.
//!
//! The default `hash` backend is deterministic and offline for tests. The
//! `http` backend is the hosted real-encoder path and is feature-gated so the
//! default build does not pull a network client into every consumer. The
//! `local` backend is a feature-gated Candle/BGE model load for deployments
//! that want real embeddings without a hosted endpoint.

use std::fmt;
use std::sync::Arc;
#[cfg(feature = "http")]
use std::time::Duration;

#[cfg(feature = "local")]
use candle_core::{Device, Tensor};
#[cfg(feature = "local")]
use candle_nn::VarBuilder;
#[cfg(feature = "local")]
use candle_transformers::models::bert::{BertModel, Config as BertConfig, DTYPE};
#[cfg(feature = "local")]
use hf_hub::api::sync::Api;
#[cfg(feature = "local")]
use hf_hub::{Repo, RepoType};
use serde_json::Value;
#[cfg(feature = "local")]
use tokenizers::utils::truncation::TruncationParams;
#[cfg(feature = "local")]
use tokenizers::Tokenizer;

pub const DEFAULT_REAL_CODE_EMBEDDING_DIM: usize = 384;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
#[cfg(feature = "local")]
const DEFAULT_LOCAL_MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
#[cfg(feature = "local")]
const DEFAULT_LOCAL_MAX_TOKENS: usize = 512;

const EMBEDDER_ENV: &str = "RUSTYRED_CODE_EMBEDDER";
const EMBED_URL_ENV: &str = "RUSTYRED_CODE_EMBED_URL";
const EMBED_DIM_ENV: &str = "RUSTYRED_CODE_EMBED_DIM";
const EMBED_TIMEOUT_ENV: &str = "RUSTYRED_CODE_EMBED_TIMEOUT_SECONDS";
#[cfg(feature = "local")]
const EMBED_LOCAL_MODEL_ENV: &str = "RUSTYRED_CODE_EMBED_LOCAL_MODEL";

/// Embedding abstraction shared by RustyRed code workspace write paths.
pub trait CodeEmbedder: Send + Sync {
    fn embed_code(&self, text: &str) -> Result<Vec<f32>, CodeEmbeddingError>;
    fn dimension(&self) -> usize;
    fn name(&self) -> &str;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodeEmbeddingConfig {
    pub kind: CodeEmbeddingKind,
    pub dimension: usize,
    pub http_url: Option<String>,
    pub timeout_secs: u64,
}

impl CodeEmbeddingConfig {
    pub fn hash(dimension: usize) -> Self {
        Self {
            kind: CodeEmbeddingKind::Hash,
            dimension: dimension.max(1),
            http_url: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    pub fn http(url: impl Into<String>, dimension: usize) -> Self {
        Self {
            kind: CodeEmbeddingKind::Http,
            dimension: dimension.max(1),
            http_url: Some(url.into()),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    pub fn local(dimension: usize) -> Self {
        Self {
            kind: CodeEmbeddingKind::Local,
            dimension: dimension.max(1),
            http_url: None,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }

    /// Resolve from environment, preserving the caller's legacy hash dimension
    /// when no real encoder is configured.
    pub fn from_env_or_hash(default_hash_dimension: usize) -> Result<Self, CodeEmbeddingError> {
        let Some(kind_raw) = non_empty(std::env::var(EMBEDDER_ENV).ok()) else {
            return Ok(Self::hash(default_hash_dimension));
        };
        let kind = CodeEmbeddingKind::parse(&kind_raw)?;
        let fallback_dim = match kind {
            CodeEmbeddingKind::Hash => default_hash_dimension,
            CodeEmbeddingKind::Http | CodeEmbeddingKind::Local => DEFAULT_REAL_CODE_EMBEDDING_DIM,
        };
        let dimension = parse_env_usize(EMBED_DIM_ENV, fallback_dim)?;
        let timeout_secs = parse_env_u64(EMBED_TIMEOUT_ENV, DEFAULT_TIMEOUT_SECS)?.max(1);
        let http_url = non_empty(std::env::var(EMBED_URL_ENV).ok());
        Ok(Self {
            kind,
            dimension,
            http_url,
            timeout_secs,
        })
    }

    pub fn build(&self) -> Result<Arc<dyn CodeEmbedder>, CodeEmbeddingError> {
        match self.kind {
            CodeEmbeddingKind::Hash => Ok(Arc::new(HashCodeEmbedder::new(self.dimension))),
            CodeEmbeddingKind::Http => build_http_embedder(self),
            CodeEmbeddingKind::Local => build_local_embedder(self),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodeEmbeddingKind {
    Hash,
    Http,
    Local,
}

impl CodeEmbeddingKind {
    pub fn parse(raw: &str) -> Result<Self, CodeEmbeddingError> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "hash" => Ok(Self::Hash),
            "http" => Ok(Self::Http),
            "local" => Ok(Self::Local),
            other => Err(CodeEmbeddingError::Config(format!(
                "unknown code embedder `{other}` (expected hash | http | local)"
            ))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hash => "hash",
            Self::Http => "http",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Debug)]
pub struct HashCodeEmbedder {
    dimension: usize,
}

impl HashCodeEmbedder {
    pub fn new(dimension: usize) -> Self {
        Self {
            dimension: dimension.max(1),
        }
    }
}

impl CodeEmbedder for HashCodeEmbedder {
    fn embed_code(&self, text: &str) -> Result<Vec<f32>, CodeEmbeddingError> {
        Ok(hash_code_embedding(text, self.dimension))
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn name(&self) -> &str {
        "hash"
    }
}

pub fn hash_code_embedding(text: &str, dimension: usize) -> Vec<f32> {
    let mut vector = vec![0f32; dimension.max(1)];
    for token in tokenize_code(text) {
        let hash = fnv1a(token.as_bytes());
        let index = (hash % vector.len() as u64) as usize;
        let sign = if (hash >> 1) & 1 == 0 { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    l2_normalize(&mut vector);
    vector
}

fn tokenize_code(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(|token| token.to_ascii_lowercase())
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

fn l2_normalize(vector: &mut [f32]) {
    let norm: f32 = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in vector {
            *value /= norm;
        }
    } else if let Some(first) = vector.first_mut() {
        *first = 1.0;
    }
}

pub fn cosine_similarity(left: &[f32], right: &[f32]) -> f32 {
    left.iter().zip(right).map(|(x, y)| x * y).sum()
}

#[cfg(feature = "http")]
#[derive(Clone)]
pub struct HttpCodeEmbedder {
    client: reqwest::blocking::Client,
    url: String,
    dimension: usize,
}

#[cfg(feature = "http")]
impl HttpCodeEmbedder {
    pub fn new(
        url: impl Into<String>,
        dimension: usize,
        timeout_secs: u64,
    ) -> Result<Self, CodeEmbeddingError> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(timeout_secs.max(1)))
            .user_agent("rustyred-code-embedding/0.1")
            .build()
            .map_err(|error| CodeEmbeddingError::Http(error.to_string()))?;
        Ok(Self {
            client,
            url: url.into(),
            dimension: dimension.max(1),
        })
    }
}

#[cfg(feature = "http")]
impl CodeEmbedder for HttpCodeEmbedder {
    fn embed_code(&self, text: &str) -> Result<Vec<f32>, CodeEmbeddingError> {
        let response = self
            .client
            .post(&self.url)
            .json(&serde_json::json!({ "input": text }))
            .send()
            .map_err(|error| CodeEmbeddingError::Http(error.to_string()))?;
        if !response.status().is_success() {
            return Err(CodeEmbeddingError::Http(format!(
                "code embed endpoint returned HTTP {}",
                response.status().as_u16()
            )));
        }
        let body: Value = response
            .json()
            .map_err(|error| CodeEmbeddingError::Http(error.to_string()))?;
        let vector = extract_embedding_vector(&body).ok_or_else(|| {
            CodeEmbeddingError::Http(
                "code embed response had no recognizable embedding vector".to_string(),
            )
        })?;
        if vector.len() != self.dimension {
            return Err(CodeEmbeddingError::Dimension {
                expected: self.dimension,
                actual: vector.len(),
            });
        }
        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn name(&self) -> &str {
        "http"
    }
}

#[cfg(feature = "local")]
pub struct LocalCodeEmbedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    dimension: usize,
    name: String,
}

#[cfg(feature = "local")]
impl LocalCodeEmbedder {
    pub fn new(dimension: usize) -> Result<Self, CodeEmbeddingError> {
        let requested_dimension = dimension.max(1);
        let model_id = non_empty(std::env::var(EMBED_LOCAL_MODEL_ENV).ok())
            .unwrap_or_else(|| DEFAULT_LOCAL_MODEL_ID.to_string());
        let device = Device::Cpu;
        let repo = Api::new()
            .map_err(local_model_err)?
            .repo(Repo::new(model_id.clone(), RepoType::Model));

        let config_path = repo.get("config.json").map_err(local_model_err)?;
        let tokenizer_path = repo.get("tokenizer.json").map_err(local_model_err)?;
        let weights_path = repo.get("model.safetensors").map_err(local_model_err)?;

        let bert_config: BertConfig =
            serde_json::from_str(&std::fs::read_to_string(config_path).map_err(local_model_err)?)
                .map_err(local_model_err)?;
        if bert_config.hidden_size != requested_dimension {
            return Err(CodeEmbeddingError::Dimension {
                expected: requested_dimension,
                actual: bert_config.hidden_size,
            });
        }

        let mut tokenizer = Tokenizer::from_file(tokenizer_path).map_err(local_model_err)?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: DEFAULT_LOCAL_MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(local_model_err)?;

        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(&[weights_path], DTYPE, &device)
                .map_err(local_model_err)?
        };
        let model = BertModel::load(vb, &bert_config).map_err(local_model_err)?;
        Ok(Self {
            model,
            tokenizer,
            device,
            dimension: requested_dimension,
            name: format!("local:{model_id}"),
        })
    }
}

#[cfg(feature = "local")]
impl CodeEmbedder for LocalCodeEmbedder {
    fn embed_code(&self, text: &str) -> Result<Vec<f32>, CodeEmbeddingError> {
        let tokens = self.tokenizer.encode(text, true).map_err(local_model_err)?;
        let ids = tokens.get_ids().to_vec();
        let token_ids = Tensor::new(ids.as_slice(), &self.device)
            .map_err(local_model_err)?
            .unsqueeze(0)
            .map_err(local_model_err)?;
        let token_type_ids = token_ids.zeros_like().map_err(local_model_err)?;
        let hidden = self
            .model
            .forward(&token_ids, &token_type_ids, None)
            .map_err(local_model_err)?;
        let cls = hidden
            .narrow(1, 0, 1)
            .map_err(local_model_err)?
            .squeeze(1)
            .map_err(local_model_err)?;
        let normalized = normalize_l2(&cls).map_err(local_model_err)?;
        let vector = normalized
            .squeeze(0)
            .map_err(local_model_err)?
            .to_vec1::<f32>()
            .map_err(local_model_err)?;
        if vector.len() != self.dimension {
            return Err(CodeEmbeddingError::Dimension {
                expected: self.dimension,
                actual: vector.len(),
            });
        }
        Ok(vector)
    }

    fn dimension(&self) -> usize {
        self.dimension
    }

    fn name(&self) -> &str {
        &self.name
    }
}

#[cfg(feature = "local")]
fn normalize_l2(tensor: &Tensor) -> candle_core::Result<Tensor> {
    let norm = tensor.sqr()?.sum_keepdim(1)?.sqrt()?;
    tensor.broadcast_div(&norm)
}

#[cfg(feature = "http")]
fn build_http_embedder(
    config: &CodeEmbeddingConfig,
) -> Result<Arc<dyn CodeEmbedder>, CodeEmbeddingError> {
    let url = config.http_url.clone().ok_or_else(|| {
        CodeEmbeddingError::Config(format!(
            "code embedder `http` requires {EMBED_URL_ENV} or an explicit URL"
        ))
    })?;
    Ok(Arc::new(HttpCodeEmbedder::new(
        url,
        config.dimension,
        config.timeout_secs,
    )?))
}

#[cfg(not(feature = "http"))]
fn build_http_embedder(
    _config: &CodeEmbeddingConfig,
) -> Result<Arc<dyn CodeEmbedder>, CodeEmbeddingError> {
    Err(CodeEmbeddingError::Config(
        "code embedder `http` requires building with feature `http`".to_string(),
    ))
}

#[cfg(feature = "local")]
fn build_local_embedder(
    config: &CodeEmbeddingConfig,
) -> Result<Arc<dyn CodeEmbedder>, CodeEmbeddingError> {
    Ok(Arc::new(LocalCodeEmbedder::new(config.dimension)?))
}

#[cfg(not(feature = "local"))]
fn build_local_embedder(
    _config: &CodeEmbeddingConfig,
) -> Result<Arc<dyn CodeEmbedder>, CodeEmbeddingError> {
    Err(CodeEmbeddingError::Config(
        "code embedder `local` requires building with feature `local`".to_string(),
    ))
}

pub fn extract_embedding_vector(body: &Value) -> Option<Vec<f32>> {
    if let Some(array) = body.get("embedding").and_then(Value::as_array) {
        return as_f32_vec(array);
    }
    if let Some(array) = body
        .get("embeddings")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(Value::as_array)
    {
        return as_f32_vec(array);
    }
    if let Some(array) = body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|values| values.first())
        .and_then(|value| value.get("embedding"))
        .and_then(Value::as_array)
    {
        return as_f32_vec(array);
    }
    None
}

fn as_f32_vec(values: &[Value]) -> Option<Vec<f32>> {
    values
        .iter()
        .map(|value| value.as_f64().map(|number| number as f32))
        .collect()
}

fn parse_env_usize(key: &str, default: usize) -> Result<usize, CodeEmbeddingError> {
    match std::env::var(key) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| CodeEmbeddingError::Config(format!("{key} must be a positive integer"))),
        _ => Ok(default.max(1)),
    }
}

fn parse_env_u64(key: &str, default: u64) -> Result<u64, CodeEmbeddingError> {
    match std::env::var(key) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<u64>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| CodeEmbeddingError::Config(format!("{key} must be a positive integer"))),
        _ => Ok(default.max(1)),
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
}

#[cfg(feature = "local")]
fn local_model_err<E: fmt::Display>(error: E) -> CodeEmbeddingError {
    CodeEmbeddingError::Model(error.to_string())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CodeEmbeddingError {
    Config(String),
    Http(String),
    Model(String),
    Dimension { expected: usize, actual: usize },
}

impl fmt::Display for CodeEmbeddingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "config: {message}"),
            Self::Http(message) => write!(f, "http: {message}"),
            Self::Model(message) => write!(f, "model: {message}"),
            Self::Dimension { expected, actual } => {
                write!(f, "dimension mismatch: expected {expected}, got {actual}")
            }
        }
    }
}

impl std::error::Error for CodeEmbeddingError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_embedder_is_deterministic_and_dimensioned() {
        let embedder = HashCodeEmbedder::new(17);
        let first = embedder
            .embed_code("pub fn alpha() -> usize { 1 }")
            .unwrap();
        let second = embedder
            .embed_code("pub fn alpha() -> usize { 1 }")
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(first.len(), 17);
    }

    #[test]
    fn hash_embedder_keeps_shared_code_tokens_nearer() {
        let embedder = HashCodeEmbedder::new(256);
        let query = embedder
            .embed_code("fn parse_request(input: &str) -> Request")
            .unwrap();
        let near = embedder
            .embed_code("fn parse_response(input: &str) -> Response")
            .unwrap();
        let far = embedder
            .embed_code("class InvoiceTotal { renderCurrency() }")
            .unwrap();
        assert!(cosine_similarity(&query, &near) > cosine_similarity(&query, &far));
    }

    #[test]
    fn extracts_supported_http_response_shapes() {
        assert_eq!(
            extract_embedding_vector(&serde_json::json!({ "embedding": [0.1, 0.2] })),
            Some(vec![0.1, 0.2])
        );
        assert_eq!(
            extract_embedding_vector(&serde_json::json!({ "embeddings": [[0.3, 0.4]] })),
            Some(vec![0.3, 0.4])
        );
        assert_eq!(
            extract_embedding_vector(&serde_json::json!({ "data": [{ "embedding": [0.5, 0.6] }] })),
            Some(vec![0.5, 0.6])
        );
    }

    #[test]
    fn http_feature_is_explicit_when_not_enabled() {
        #[cfg(not(feature = "http"))]
        assert!(matches!(
            CodeEmbeddingConfig::http("http://127.0.0.1/embedding", 3).build(),
            Err(CodeEmbeddingError::Config(message))
                if message.contains("feature `http`")
        ));
    }

    #[test]
    fn local_feature_is_explicit_when_not_enabled() {
        #[cfg(not(feature = "local"))]
        assert!(matches!(
            CodeEmbeddingConfig::local(DEFAULT_REAL_CODE_EMBEDDING_DIM).build(),
            Err(CodeEmbeddingError::Config(message))
                if message.contains("feature `local`")
        ));
    }

    #[cfg(feature = "http")]
    #[test]
    fn http_embedder_reads_mock_endpoint_and_preserves_dimension() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/embed", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 2048];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("POST /embed HTTP/1.1"));
            assert!(request.contains("\"input\""));
            let body = r#"{"embedding":[0.9,0.1,0.0]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
                body.len(),
                body
            )
            .unwrap();
        });

        let embedder = HttpCodeEmbedder::new(url, 3, 5).unwrap();
        assert_eq!(
            embedder.embed_code("fn alpha() -> usize { 1 }").unwrap(),
            vec![0.9, 0.1, 0.0]
        );
        server.join().unwrap();
    }

    #[cfg(feature = "http")]
    #[test]
    #[ignore = "requires a live hosted code embedding endpoint; set RUSTYRED_CODE_EMBED_URL"]
    fn live_http_embedder_endpoint_prefers_related_code() {
        let url = std::env::var(EMBED_URL_ENV)
            .expect("set RUSTYRED_CODE_EMBED_URL to run the live HTTP embedder smoke");
        let dimension = parse_env_usize(EMBED_DIM_ENV, DEFAULT_REAL_CODE_EMBEDDING_DIM).unwrap();
        let timeout_secs = parse_env_u64(EMBED_TIMEOUT_ENV, DEFAULT_TIMEOUT_SECS).unwrap();
        let embedder = HttpCodeEmbedder::new(url, dimension, timeout_secs).unwrap();
        assert_eq!(embedder.dimension(), dimension);

        let query = embedder
            .embed_code("fn parse_invoice_total(input: &str) -> Money")
            .unwrap();
        let near = embedder
            .embed_code("fn extract_invoice_amount(raw: &str) -> Money")
            .unwrap();
        let far = embedder
            .embed_code("pub struct OpenGlTextureAtlas { slots: Vec<Sprite> }")
            .unwrap();

        assert_eq!(query.len(), dimension);
        assert_eq!(near.len(), dimension);
        assert_eq!(far.len(), dimension);
        assert!(query.iter().all(|value| value.is_finite()));
        assert!(near.iter().all(|value| value.is_finite()));
        assert!(far.iter().all(|value| value.is_finite()));
        assert!(cosine_similarity(&query, &near) > cosine_similarity(&query, &far));
    }

    #[cfg(feature = "local")]
    #[test]
    #[ignore = "downloads and loads BAAI/bge-small-en-v1.5 from the Hugging Face cache/hub"]
    fn local_bge_embedder_loads_and_prefers_related_code() {
        let embedder = CodeEmbeddingConfig::local(DEFAULT_REAL_CODE_EMBEDDING_DIM)
            .build()
            .unwrap();
        assert_eq!(embedder.dimension(), DEFAULT_REAL_CODE_EMBEDDING_DIM);
        assert!(embedder.name().contains("bge-small-en-v1.5"));

        let query = embedder
            .embed_code("fn parse_invoice_total(input: &str) -> Money")
            .unwrap();
        let near = embedder
            .embed_code("fn extract_invoice_amount(raw: &str) -> Money")
            .unwrap();
        let far = embedder
            .embed_code("pub struct OpenGlTextureAtlas { slots: Vec<Sprite> }")
            .unwrap();
        assert!(cosine_similarity(&query, &near) > cosine_similarity(&query, &far));
    }
}
