use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::Value;

use crate::protocol::{JsonRpcMessage, JsonRpcRequest};
use crate::{AcpError, AcpResult};

#[derive(Debug)]
pub struct StdioJsonRpcTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl StdioJsonRpcTransport {
    pub fn spawn(program: &str, args: &[String]) -> AcpResult<Self> {
        let mut child = Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| AcpError::Protocol("agent stdin was not piped".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AcpError::Protocol("agent stdout was not piped".to_string()))?;
        let stderr = child.stderr.take();
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            stderr,
        })
    }

    pub fn send_request(&mut self, request: &JsonRpcRequest) -> AcpResult<()> {
        self.send_value(&serde_json::to_value(request)?)
    }

    pub fn send_value(&mut self, value: &Value) -> AcpResult<()> {
        let line = serde_json::to_string(value)?;
        if line.contains('\n') {
            return Err(AcpError::Protocol(
                "ACP stdio messages must be newline-delimited single lines".to_string(),
            ));
        }
        self.stdin.write_all(line.as_bytes())?;
        self.stdin.write_all(b"\n")?;
        self.stdin.flush()?;
        Ok(())
    }

    pub fn read_message(&mut self) -> AcpResult<JsonRpcMessage> {
        let mut line = String::new();
        let n = self.stdout.read_line(&mut line)?;
        if n == 0 {
            return Err(AcpError::Protocol("agent stdout closed".to_string()));
        }
        Ok(serde_json::from_str(line.trim_end())?)
    }

    pub fn child_id(&self) -> u32 {
        self.child.id()
    }

    pub fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.stderr.take()
    }
}

#[cfg(test)]
mod tests {
    use crate::protocol::initialize_request;

    #[test]
    fn request_lines_are_single_json_objects() {
        let request = initialize_request(1, "0.1.0");
        let line = serde_json::to_string(&request).unwrap();
        assert!(!line.contains('\n'));
    }
}
