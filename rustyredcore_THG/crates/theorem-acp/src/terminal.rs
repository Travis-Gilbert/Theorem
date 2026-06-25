use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{AcpError, AcpResult};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CreateTerminalRequest {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandApproval {
    pub terminal_id: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TerminalExitStatus {
    #[serde(rename = "exitCode")]
    pub exit_code: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CommandRunResult {
    pub terminal_id: String,
    pub stdout: String,
    pub stderr: String,
    pub status: TerminalExitStatus,
}

#[derive(Debug)]
pub struct TerminalHost {
    workspace_root: PathBuf,
    pending_commands: BTreeMap<String, CommandApproval>,
}

impl TerminalHost {
    pub fn new(workspace_root: impl Into<PathBuf>) -> AcpResult<Self> {
        Ok(Self {
            workspace_root: workspace_root.into().canonicalize()?,
            pending_commands: BTreeMap::new(),
        })
    }

    pub fn request_create(&mut self, request: CreateTerminalRequest) -> AcpResult<CommandApproval> {
        let cwd = self.resolve_cwd(request.cwd.as_deref())?;
        let approval = CommandApproval {
            terminal_id: format!("terminal-{}", Uuid::new_v4()),
            command: request.command,
            args: request.args,
            cwd: cwd.to_string_lossy().into_owned(),
        };
        self.pending_commands
            .insert(approval.terminal_id.clone(), approval.clone());
        Ok(approval)
    }

    pub fn approve(&mut self, terminal_id: &str) -> AcpResult<CommandRunResult> {
        let approval = self
            .pending_commands
            .remove(terminal_id)
            .ok_or_else(|| AcpError::PendingCommandNotFound(terminal_id.to_string()))?;
        self.run_pty_command(approval)
    }

    pub fn deny(&mut self, terminal_id: &str) -> AcpResult<CommandApproval> {
        self.pending_commands
            .remove(terminal_id)
            .ok_or_else(|| AcpError::PendingCommandNotFound(terminal_id.to_string()))
    }

    pub fn pending_command(&self, terminal_id: &str) -> Option<&CommandApproval> {
        self.pending_commands.get(terminal_id)
    }

    fn resolve_cwd(&self, requested: Option<&str>) -> AcpResult<PathBuf> {
        let cwd = match requested {
            Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
            Some(path) => self.workspace_root.join(path),
            None => self.workspace_root.clone(),
        }
        .canonicalize()?;

        if cwd.starts_with(&self.workspace_root) {
            Ok(cwd)
        } else {
            Err(AcpError::OutsideWorkspace(cwd))
        }
    }

    fn run_pty_command(&self, approval: CommandApproval) -> AcpResult<CommandRunResult> {
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 120,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|exc| AcpError::Terminal(exc.to_string()))?;
        let mut command = CommandBuilder::new(&approval.command);
        command.args(&approval.args);
        command.cwd(&approval.cwd);

        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|exc| AcpError::Terminal(exc.to_string()))?;
        drop(pair.slave);

        let mut output = String::new();
        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|exc| AcpError::Terminal(exc.to_string()))?;
        reader.read_to_string(&mut output)?;

        let status = child.wait()?;
        Ok(CommandRunResult {
            terminal_id: approval.terminal_id,
            stdout: output,
            stderr: String::new(),
            status: TerminalExitStatus {
                exit_code: status.exit_code(),
                signal: status.signal().map(ToOwned::to_owned),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_is_staged_before_execution() {
        let dir = tempfile::tempdir().unwrap();
        let mut host = TerminalHost::new(dir.path()).unwrap();
        let approval = host
            .request_create(CreateTerminalRequest {
                command: "printf".to_string(),
                args: vec!["hello".to_string()],
                cwd: None,
            })
            .unwrap();

        assert!(host.pending_command(&approval.terminal_id).is_some());
        assert_eq!(approval.command, "printf");
    }

    #[test]
    fn cwd_is_scoped_to_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let mut host = TerminalHost::new(dir.path()).unwrap();
        let err = host
            .request_create(CreateTerminalRequest {
                command: "pwd".to_string(),
                args: vec![],
                cwd: Some(outside.path().to_string_lossy().into_owned()),
            })
            .unwrap_err();

        assert!(matches!(err, AcpError::OutsideWorkspace(_)));
    }
}
