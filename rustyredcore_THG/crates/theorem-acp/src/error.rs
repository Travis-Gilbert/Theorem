use std::path::PathBuf;

pub type AcpResult<T> = Result<T, AcpError>;

#[derive(Debug, thiserror::Error)]
pub enum AcpError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("path is outside the ACP workspace: {0}")]
    OutsideWorkspace(PathBuf),
    #[error("workspace path has no existing parent: {0}")]
    MissingParent(PathBuf),
    #[error("pending file write not found: {0}")]
    PendingWriteNotFound(String),
    #[error("pending command not found: {0}")]
    PendingCommandNotFound(String),
    #[error("terminal error: {0}")]
    Terminal(String),
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("receiver sidecar error: {0}")]
    Receiver(String),
    #[error("copresence error: {0}")]
    Copresence(#[from] theorem_copresence::CoError),
}
