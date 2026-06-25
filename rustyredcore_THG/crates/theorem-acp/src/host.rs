use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::fs::{ReadTextFileRequest, WorkspaceFs, WriteTextFileRequest};
use crate::harness::default_harness_mcp_server;
use crate::protocol::{
    initialize_request, new_session_request, prompt_request, JsonRpcMessage, JsonRpcResponse,
};
use crate::terminal::{CreateTerminalRequest, TerminalHost};
use crate::transport::StdioJsonRpcTransport;
use crate::{AcpError, AcpResult};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AcpAgentCommand {
    pub agent_id: String,
    pub program: String,
    pub args: Vec<String>,
}

impl AcpAgentCommand {
    pub fn configured(agent_id: &str) -> Self {
        let env_prefix = format!("COMMONPLACE_ACP_{}", agent_id.to_ascii_uppercase());
        let program = std::env::var(format!("{env_prefix}_COMMAND"))
            .unwrap_or_else(|_| default_program(agent_id).to_string());
        let args = std::env::var(format!("{env_prefix}_ARGS"))
            .ok()
            .map(|raw| raw.split_whitespace().map(ToOwned::to_owned).collect())
            .unwrap_or_else(|| default_args(agent_id));
        Self {
            agent_id: agent_id.to_string(),
            program,
            args,
        }
    }
}

#[derive(Debug)]
pub struct AcpSessionHandle {
    pub local_session_id: String,
    pub agent_id: String,
    pub workspace_root: PathBuf,
    pub transport: StdioJsonRpcTransport,
    pub fs: WorkspaceFs,
    pub terminal: TerminalHost,
}

#[derive(Debug, Default)]
pub struct AcpHost {
    next_id: u64,
}

impl AcpHost {
    pub fn new() -> Self {
        Self { next_id: 1 }
    }

    pub fn spawn_session(
        &mut self,
        agent: AcpAgentCommand,
        workspace_root: impl AsRef<Path>,
    ) -> AcpResult<AcpSessionHandle> {
        let workspace_root = workspace_root.as_ref().canonicalize()?;
        let mut transport = StdioJsonRpcTransport::spawn(&agent.program, &agent.args)?;
        let init_id = self.next_rpc_id();
        transport.send_request(&initialize_request(init_id, env!("CARGO_PKG_VERSION")))?;
        let new_session_id = self.next_rpc_id();
        transport.send_request(&new_session_request(
            new_session_id,
            workspace_root.to_string_lossy().into_owned(),
            vec![default_harness_mcp_server()],
            vec![],
        ))?;

        Ok(AcpSessionHandle {
            local_session_id: format!("pending-{}", transport.child_id()),
            agent_id: agent.agent_id,
            workspace_root: workspace_root.clone(),
            transport,
            fs: WorkspaceFs::new(&workspace_root)?,
            terminal: TerminalHost::new(&workspace_root)?,
        })
    }

    pub fn next_rpc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl AcpSessionHandle {
    pub fn send_prompt(&mut self, request_id: Value, session_id: &str, text: &str) -> AcpResult<()> {
        self.transport
            .send_request(&prompt_request(request_id, session_id, text))
    }

    pub fn read_agent_message(&mut self) -> AcpResult<JsonRpcMessage> {
        self.transport.read_message()
    }

    pub fn handle_client_request(&mut self, method: &str, params: Value) -> AcpResult<Value> {
        match method {
            "fs/read_text_file" => {
                let request: ReadTextFileRequest = serde_json::from_value(params)?;
                Ok(Value::String(self.fs.read_text_file(request)?))
            }
            "fs/write_text_file" => {
                let request: WriteTextFileRequest = serde_json::from_value(params)?;
                let review = self.fs.stage_write_text_file(request)?;
                Ok(serde_json::to_value(review)?)
            }
            "terminal/create" => {
                let request: CreateTerminalRequest = serde_json::from_value(params)?;
                let approval = self.terminal.request_create(request)?;
                Ok(serde_json::to_value(approval)?)
            }
            other => Err(AcpError::Protocol(format!(
                "unsupported ACP client request method: {other}"
            ))),
        }
    }

    pub fn success_response(id: Value, result: Value) -> JsonRpcResponse {
        JsonRpcResponse {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }
}

fn default_program(agent_id: &str) -> &str {
    match agent_id {
        "claude" => "claude-code",
        "codex" => "codex",
        "deepseek" => "deepseek",
        "gemini" => "gemini",
        "opencode" => "opencode",
        _ => "agent",
    }
}

fn default_args(agent_id: &str) -> Vec<String> {
    match agent_id {
        "gemini" => vec!["--acp".to_string()],
        _ => vec!["acp".to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn built_in_agent_defaults_use_acp_subcommands() {
        assert_eq!(
            AcpAgentCommand::configured("claude"),
            AcpAgentCommand {
                agent_id: "claude".to_string(),
                program: "claude-code".to_string(),
                args: vec!["acp".to_string()],
            }
        );
        assert_eq!(AcpAgentCommand::configured("gemini").args, vec!["--acp"]);
    }

    #[test]
    fn client_requests_dispatch_to_reviewable_handlers() {
        let dir = tempfile::tempdir().unwrap();
        let transport = StdioJsonRpcTransport::spawn("sh", &["-c".to_string(), "cat".to_string()]);
        if transport.is_err() {
            return;
        }
        let mut session = AcpSessionHandle {
            local_session_id: "test".to_string(),
            agent_id: "claude".to_string(),
            workspace_root: dir.path().to_path_buf(),
            transport: transport.unwrap(),
            fs: WorkspaceFs::new(dir.path()).unwrap(),
            terminal: TerminalHost::new(dir.path()).unwrap(),
        };

        let value = session
            .handle_client_request(
                "fs/write_text_file",
                json!({ "path": "x.txt", "content": "hello" }),
            )
            .unwrap();
        assert!(value["request_id"]
            .as_str()
            .unwrap()
            .starts_with("file-write-"));
        assert!(!dir.path().join("x.txt").exists());
    }
}
