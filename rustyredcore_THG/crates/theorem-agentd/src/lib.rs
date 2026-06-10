//! theorem-agentd: local assistant daemon plus MCP tool host.
//!
//! The daemon is intentionally small at the process boundary. A resident local
//! model chooses one schema-guarded tool call at a time, MCP servers execute the
//! tool, and the existing `theorem-receiver` remains the only component that
//! launches Claude or Codex sessions.

pub mod capture;
pub mod config;
pub mod ledger;
pub mod mcp;
pub mod model;
pub mod receiver_sidecar;
pub mod relay;
pub mod tools;
pub mod turn_loop;

use std::fmt;

pub type AgentdResult<T> = Result<T, AgentdError>;

#[derive(Debug)]
pub enum AgentdError {
    Config(String),
    Io(std::io::Error),
    Http(String),
    Json(serde_json::Error),
    Mcp(String),
    Model(String),
    Tool(String),
}

impl fmt::Display for AgentdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Io(error) => write!(f, "io error: {error}"),
            Self::Http(message) => write!(f, "http error: {message}"),
            Self::Json(error) => write!(f, "json error: {error}"),
            Self::Mcp(message) => write!(f, "mcp error: {message}"),
            Self::Model(message) => write!(f, "model error: {message}"),
            Self::Tool(message) => write!(f, "tool error: {message}"),
        }
    }
}

impl std::error::Error for AgentdError {}

impl From<std::io::Error> for AgentdError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for AgentdError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<reqwest::Error> for AgentdError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error.to_string())
    }
}
