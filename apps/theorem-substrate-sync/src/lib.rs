pub mod bootstrap;
pub mod config;
pub mod cursor;
pub mod drainer;
pub mod outbox;
pub mod railway_client;
pub mod round;
pub mod scheduler;
pub mod status;
pub mod subscriber;

use std::fmt;

pub type Result<T> = std::result::Result<T, SyncError>;

#[derive(Debug)]
pub enum SyncError {
    Auth(String),
    Config(String),
    Http(reqwest::Error),
    Io(std::io::Error),
    Json(serde_json::Error),
    Mcp(String),
    Redis(redis::RedisError),
}

impl fmt::Display for SyncError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Auth(message) => write!(f, "auth error: {message}"),
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Http(error) => write!(f, "http error: {error}"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::Mcp(message) => write!(f, "mcp error: {message}"),
            Self::Redis(error) => write!(f, "valkey error: {error}"),
        }
    }
}

impl std::error::Error for SyncError {}

impl From<reqwest::Error> for SyncError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

impl From<std::io::Error> for SyncError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for SyncError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<redis::RedisError> for SyncError {
    fn from(error: redis::RedisError) -> Self {
        Self::Redis(error)
    }
}

pub fn stable_hash(value: &serde_json::Value) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value).unwrap_or_default();
    let digest = Sha256::digest(bytes);
    format!(
        "sha256:{}",
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
