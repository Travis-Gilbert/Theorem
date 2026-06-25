//! ACP host primitives for docking external coding agents into CommonPlace.
//!
//! This crate is the sibling of `theorem-agentd`: it hosts an external agent
//! process over ACP stdio instead of running an in-house model loop.

pub mod copresence;
pub mod error;
pub mod fs;
pub mod harness;
pub mod host;
pub mod protocol;
pub mod receiver;
pub mod terminal;
pub mod transport;
pub mod websocket;

pub use copresence::{
    announce_agent_session, agent_presence, CopresenceSession, DEFAULT_COPRESENCE_SCOPE_PREFIX,
};
pub use error::{AcpError, AcpResult};
pub use fs::{FileWriteReview, ReadTextFileRequest, WorkspaceFs, WriteTextFileRequest};
pub use harness::{default_harness_mcp_server, HarnessMcpConfig, DEFAULT_HARNESS_MCP_URL};
pub use host::{AcpAgentCommand, AcpHost, AcpSessionHandle};
pub use protocol::{
    initialize_request, new_session_request, prompt_request, ClientCapabilities, ContentBlock,
    JsonRpcMessage, JsonRpcRequest, McpServer, PROTOCOL_VERSION,
};
pub use receiver::{spawn_receiver_sidecar, ReceiverSidecarConfig};
pub use terminal::{
    CommandApproval, CommandRunResult, CreateTerminalRequest, TerminalExitStatus, TerminalHost,
};
pub use websocket::{FrontendEnvelope, FrontendInbound, FrontendOutbound};
