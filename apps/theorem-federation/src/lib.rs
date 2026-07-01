pub mod blob;
pub mod delta;
pub mod driver;
pub mod gossip;
pub mod identity;
pub mod transport;
pub mod trust;

pub type Result<T> = std::result::Result<T, FederationError>;

#[derive(Debug, thiserror::Error)]
pub enum FederationError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("copresence error: {0}")]
    Copresence(#[from] theorem_copresence::CoError),
    #[error("hex error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("iroh error: {0}")]
    Iroh(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("randomness error: {0}")]
    Random(#[from] getrandom::Error),
    #[error("frame too large: {actual} bytes exceeds {limit} bytes")]
    FrameTooLarge { actual: usize, limit: usize },
    #[error("trust policy rejected peer {endpoint_id}: score {score} is below floor {floor}")]
    TrustRejected {
        endpoint_id: String,
        score: f64,
        floor: f64,
    },
}

impl FederationError {
    pub fn iroh(error: impl std::fmt::Display) -> Self {
        Self::Iroh(error.to_string())
    }
}
