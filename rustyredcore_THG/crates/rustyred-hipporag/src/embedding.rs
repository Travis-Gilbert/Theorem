use futures_util::future::BoxFuture;
use serde_json::{json, Map, Value};

use crate::schema::{HippoError, HippoResult, SEMANTIC_VECTOR_PROPERTY};

pub const SEMANTIC_VECTOR_METRIC: &str = "cosine";

pub trait HippoTextEmbedder: Send + Sync {
    fn model_id(&self) -> &str;

    fn dimension(&self) -> usize;

    fn property(&self) -> &str {
        SEMANTIC_VECTOR_PROPERTY
    }

    fn metric(&self) -> &str {
        SEMANTIC_VECTOR_METRIC
    }

    fn embed<'a>(&'a self, inputs: &'a [String]) -> BoxFuture<'a, HippoResult<Vec<Vec<f32>>>>;
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct VectorPayload {
    pub vector: Vec<f32>,
    pub model_id: Option<String>,
    pub dimension: Option<usize>,
    pub metric: Option<String>,
    pub normalized: Option<bool>,
}

impl VectorPayload {
    pub(crate) fn hash(vector: Vec<f32>) -> Self {
        Self {
            vector,
            model_id: None,
            dimension: None,
            metric: None,
            normalized: None,
        }
    }

    pub(crate) fn embedded(
        embedder: &dyn HippoTextEmbedder,
        mut vector: Vec<f32>,
    ) -> HippoResult<Self> {
        if vector.len() != embedder.dimension() {
            return Err(HippoError::new(
                "embedding_dimension_mismatch",
                format!(
                    "embedder {} returned dimension {}, expected {}",
                    embedder.model_id(),
                    vector.len(),
                    embedder.dimension()
                ),
            ));
        }
        let normalized = normalize_vector(&mut vector);
        Ok(Self {
            vector,
            model_id: Some(embedder.model_id().to_string()),
            dimension: Some(embedder.dimension()),
            metric: Some(embedder.metric().to_string()),
            normalized: Some(normalized),
        })
    }
}

pub(crate) fn write_vector_payload(properties: &mut Value, property: &str, payload: VectorPayload) {
    let props = object_props(properties);
    props.insert(property.to_string(), json!(payload.vector));
    if let Some(model_id) = payload.model_id {
        props.insert(format!("{property}_model"), json!(model_id));
    }
    if let Some(dimension) = payload.dimension {
        props.insert(format!("{property}_dimension"), json!(dimension));
    }
    if let Some(metric) = payload.metric {
        props.insert(format!("{property}_metric"), json!(metric));
    }
    if let Some(normalized) = payload.normalized {
        props.insert(format!("{property}_normalized"), json!(normalized));
    }
}

fn object_props(value: &mut Value) -> &mut Map<String, Value> {
    if !value.is_object() {
        *value = json!({});
    }
    value.as_object_mut().expect("value was forced to object")
}

fn normalize_vector(vector: &mut [f32]) -> bool {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm <= 1e-6 {
        return false;
    }
    for value in vector {
        *value /= norm;
    }
    true
}
