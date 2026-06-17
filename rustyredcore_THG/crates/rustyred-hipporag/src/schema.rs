use serde::{Deserialize, Serialize};

pub const LABEL_PAGE: &str = "Page";
pub const LABEL_PHRASE: &str = "Phrase";
pub const LABEL_HUB: &str = "Hub";

pub const EDGE_CONTAINS: &str = "CONTAINS";
pub const EDGE_RELATES: &str = "RELATES";
pub const EDGE_SYNONYM: &str = "SYNONYM";
pub const EDGE_SUMMARIZES: &str = "SUMMARIZES";
pub const EDGE_HUB_PARENT: &str = "HUB_PARENT";

pub const SEMANTIC_VECTOR_PROPERTY: &str = "semantic_vec";
pub const NODE_SPECIFICITY_PROPERTY: &str = "node_specificity";
pub const CENTRALITY_PROPERTY: &str = "centrality";
pub const HUB_SCORE_PROPERTY: &str = "hipporag_score";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HippoLabel {
    Passage,
    Phrase,
    Hub,
}

impl HippoLabel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passage => LABEL_PAGE,
            Self::Phrase => LABEL_PHRASE,
            Self::Hub => LABEL_HUB,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhraseNode {
    pub id: String,
    pub text: String,
    pub semantic_vec: Vec<f32>,
    pub node_specificity: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HubNode {
    pub id: String,
    pub summary: String,
    pub level: u32,
    pub semantic_vec: Vec<f32>,
    pub centrality: f32,
    pub member_count: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum HippoEdge {
    Contains,
    Relates,
    Synonym,
    Summarizes,
    HubParent,
}

impl HippoEdge {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Contains => EDGE_CONTAINS,
            Self::Relates => EDGE_RELATES,
            Self::Synonym => EDGE_SYNONYM,
            Self::Summarizes => EDGE_SUMMARIZES,
            Self::HubParent => EDGE_HUB_PARENT,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HippoError {
    pub code: &'static str,
    pub message: String,
}

impl HippoError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HippoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for HippoError {}

impl From<rustyred_thg_core::GraphStoreError> for HippoError {
    fn from(error: rustyred_thg_core::GraphStoreError) -> Self {
        Self::new("graph_store", format!("{}: {}", error.code, error.message))
    }
}

pub type HippoResult<T> = Result<T, HippoError>;

pub(crate) fn stable_digest(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

pub(crate) fn hash_vector(text: &str, dim: usize) -> Vec<f32> {
    let mut vector = vec![0.0; dim.max(1)];
    for token in text
        .to_ascii_lowercase()
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|token| token.len() > 1)
    {
        let digest = blake3::hash(token.as_bytes());
        let bytes = digest.as_bytes();
        let bucket =
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize % vector.len();
        let sign = if bytes[4] & 1 == 0 { 1.0 } else { -1.0 };
        vector[bucket] += sign;
    }
    normalize(&mut vector);
    vector
}

pub(crate) fn cosine(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    left.iter()
        .zip(right)
        .map(|(a, b)| a * b)
        .sum::<f32>()
        .clamp(-1.0, 1.0)
}

fn normalize(vector: &mut [f32]) {
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 1e-6 {
        for value in vector {
            *value /= norm;
        }
    }
}
