//! Remote-encoder embedder: the spec's "swappable to the existing Theseus
//! SBERT" path. POSTs text to `JOBINTEL_EMBED_URL` and reads back a vector.
//!
//! Accepts the three response shapes a sentence-transformers / OpenAI-compatible
//! endpoint commonly returns:
//!   { "embedding":  [..] }
//!   { "embeddings": [[..]] }
//!   { "data": [ { "embedding": [..] } ] }   (OpenAI style)
//! Request body is `{ "input": <text> }`, which both styles accept.

use reqwest::blocking::Client;
use serde_json::{json, Value};

use super::Embedder;
use crate::config::Config;
use crate::error::{JobIntelError, Result};

pub struct HttpEmbedder {
    http: Client,
    url: String,
    dim: usize,
}

impl HttpEmbedder {
    pub fn new(config: &Config) -> Result<Self> {
        let url = config.embed_url.clone().ok_or_else(|| {
            JobIntelError::Embed("embedder 'http' requires JOBINTEL_EMBED_URL to be set".into())
        })?;
        let http = Client::builder().user_agent("jobintel/0.1").build()?;
        Ok(Self {
            http,
            url,
            dim: config.embed_dim,
        })
    }

    fn request(&self, text: &str) -> Result<Vec<f32>> {
        let resp = self
            .http
            .post(&self.url)
            .json(&json!({ "input": text }))
            .send()?;
        if !resp.status().is_success() {
            let status = resp.status().as_u16();
            return Err(JobIntelError::Embed(format!(
                "embed endpoint returned HTTP {status}"
            )));
        }
        let body: Value = resp.json()?;
        extract_vector(&body).ok_or_else(|| {
            JobIntelError::Embed("embed response had no recognizable vector field".into())
        })
    }
}

impl Embedder for HttpEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let v = self.request(text)?;
        if v.len() != self.dim {
            return Err(JobIntelError::Embed(format!(
                "embed endpoint returned dim {} but config expects {}",
                v.len(),
                self.dim
            )));
        }
        Ok(v)
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn name(&self) -> &str {
        "http"
    }
}

/// Pull a float vector out of any of the supported response shapes.
fn extract_vector(body: &Value) -> Option<Vec<f32>> {
    if let Some(arr) = body.get("embedding").and_then(Value::as_array) {
        return as_f32_vec(arr);
    }
    if let Some(first) = body
        .get("embeddings")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_array)
    {
        return as_f32_vec(first);
    }
    if let Some(arr) = body
        .get("data")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(|d| d.get("embedding"))
        .and_then(Value::as_array)
    {
        return as_f32_vec(arr);
    }
    None
}

fn as_f32_vec(arr: &[Value]) -> Option<Vec<f32>> {
    arr.iter().map(|v| v.as_f64().map(|f| f as f32)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_plain_embedding_shape() {
        let body = json!({ "embedding": [0.1, 0.2, 0.3] });
        assert_eq!(extract_vector(&body), Some(vec![0.1f32, 0.2, 0.3]));
    }

    #[test]
    fn extracts_openai_data_shape() {
        let body = json!({ "data": [ { "embedding": [1.0, 2.0] } ] });
        assert_eq!(extract_vector(&body), Some(vec![1.0f32, 2.0]));
    }

    #[test]
    fn extracts_batched_embeddings_shape() {
        let body = json!({ "embeddings": [[4.0, 5.0, 6.0]] });
        assert_eq!(extract_vector(&body), Some(vec![4.0f32, 5.0, 6.0]));
    }

    #[test]
    fn returns_none_on_unrecognized_shape() {
        assert_eq!(extract_vector(&json!({ "nope": true })), None);
    }
}
