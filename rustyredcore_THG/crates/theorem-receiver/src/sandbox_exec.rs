//! OpenSandbox execution backend for receiver proofs.
//!
//! This mirrors [`crate::local_exec`] at the contract boundary: callers provide
//! a [`ProofPlan`] and receive the same [`ProofReceipt`] shape, but the command
//! runs through an OpenSandbox execd endpoint and is tagged
//! `substrate_rerun_sandbox`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use reqwest::blocking::{multipart, Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SandboxConfig;
use crate::local_exec::{ProofPlan, ProofReceipt};
use crate::{ReceiverError, ReceiverResult};

pub const TRUST_TIER_SANDBOX: &str = "substrate_rerun_sandbox";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxProvisionRequest {
    pub repo: String,
    pub session_id: String,
    pub env: BTreeMap<String, String>,
    pub egress_allowlist: Vec<String>,
}

impl SandboxProvisionRequest {
    pub fn new(repo: impl Into<String>, session_id: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            session_id: session_id.into(),
            env: BTreeMap::new(),
            egress_allowlist: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SandboxHandle {
    pub sandbox_id: String,
    pub repo: String,
    pub source_session_id: String,
    pub target_session_id: String,
    pub target_worktree: PathBuf,
    pub exec_endpoint: String,
    #[serde(default)]
    pub exec_headers: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxFile {
    pub path: String,
    pub content: Vec<u8>,
}

pub trait SandboxRuntime {
    fn provision(&self, request: SandboxProvisionRequest) -> ReceiverResult<SandboxHandle>;
    fn run(&self, handle: &SandboxHandle, plan: &ProofPlan) -> ReceiverResult<ProofReceipt>;
    fn put_files(&self, handle: &SandboxHandle, files: &[SandboxFile]) -> ReceiverResult<()>;
    fn get_files(
        &self,
        handle: &SandboxHandle,
        paths: &[String],
    ) -> ReceiverResult<Vec<SandboxFile>>;
    fn destroy(&self, handle: &SandboxHandle) -> ReceiverResult<()>;
}

#[derive(Clone)]
pub struct OpenSandboxRuntime {
    config: SandboxConfig,
    http: Client,
}

impl OpenSandboxRuntime {
    pub fn new(config: SandboxConfig) -> ReceiverResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(120))
            .build()?;
        Ok(Self { config, http })
    }

    fn lifecycle_base(&self) -> ReceiverResult<String> {
        self.config
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|base_url| !base_url.is_empty())
            .map(|base_url| base_url.trim_end_matches('/').to_string())
            .ok_or_else(|| ReceiverError::Config("sandbox.base_url is required".to_string()))
    }

    fn with_auth(&self, request: RequestBuilder) -> ReceiverResult<RequestBuilder> {
        let Some(env_name) = self.config.api_key_env.as_deref().map(str::trim) else {
            return Ok(request);
        };
        if env_name.is_empty() {
            return Ok(request);
        }
        let api_key = std::env::var(env_name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ReceiverError::Config(format!("missing OpenSandbox api key env {env_name}"))
            })?;
        Ok(request.header("OPEN-SANDBOX-API-KEY", api_key))
    }

    fn create_body(&self, request: &SandboxProvisionRequest) -> Value {
        let volume_name = persistent_volume_name(&request.repo, &request.session_id);
        let mut env = self.config.env.clone();
        env.extend(request.env.clone());
        let allowlist = if request.egress_allowlist.is_empty() {
            self.config.egress_allowlist.clone()
        } else {
            request.egress_allowlist.clone()
        };
        let network_policy = if allowlist.is_empty() {
            Value::Null
        } else {
            json!({
                "defaultAction": "deny",
                "egress": allowlist
                    .iter()
                    .map(|target| json!({ "action": "allow", "target": target }))
                    .collect::<Vec<_>>()
            })
        };
        let mut body = json!({
            "image": { "uri": self.config.image.clone() },
            "timeout": self.config.timeout_secs,
            "entrypoint": ["tail", "-f", "/dev/null"],
            "env": env,
            "metadata": {
                "repo": request.repo.clone(),
                "source_session_id": request.session_id.clone(),
                "target_worktree": self.config.worktree_root.clone(),
            },
            "volumes": [{
                "type": "pvc",
                "name": volume_name,
                "mountPath": self.config.worktree_root.clone(),
            }],
        });
        if !network_policy.is_null() {
            body["networkPolicy"] = network_policy;
        }
        if let Some(secure_runtime) = self
            .config
            .secure_runtime
            .as_deref()
            .map(str::trim)
            .filter(|secure_runtime| !secure_runtime.is_empty())
        {
            body["extensions"] = json!({ "secureRuntime": secure_runtime });
        }
        body
    }

    fn resolve_exec_endpoint(&self, sandbox_id: &str) -> ReceiverResult<ExecEndpoint> {
        let base = self.lifecycle_base()?;
        let response = self
            .with_auth(self.http.get(format!(
                "{base}/sandboxes/{sandbox_id}/endpoints/{}",
                self.config.execd_port
            )))?
            .query(&[("use_server_proxy", "true")])
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(ReceiverError::Protocol(format!(
                "OpenSandbox endpoint resolve failed for {sandbox_id}: {} {}",
                status.as_u16(),
                truncate(&body)
            )));
        }
        exec_endpoint_from_value(serde_json::from_str(&body)?)
    }

    fn exec_request(&self, handle: &SandboxHandle, path: &str) -> RequestBuilder {
        let mut request = self.http.post(format!(
            "{}/{}",
            handle.exec_endpoint.trim_end_matches('/'),
            path.trim_start_matches('/')
        ));
        for (name, value) in &handle.exec_headers {
            request = request.header(name, value);
        }
        request
    }
}

impl SandboxRuntime for OpenSandboxRuntime {
    fn provision(&self, request: SandboxProvisionRequest) -> ReceiverResult<SandboxHandle> {
        let base = self.lifecycle_base()?;
        let body = self.create_body(&request);
        let response = self
            .with_auth(self.http.post(format!("{base}/sandboxes")))?
            .json(&body)
            .send()?;
        let status = response.status();
        let response_body = response.text()?;
        if !status.is_success() {
            return Err(ReceiverError::Protocol(format!(
                "OpenSandbox create failed: {} {}",
                status.as_u16(),
                truncate(&response_body)
            )));
        }
        let value: Value = serde_json::from_str(&response_body)?;
        let sandbox_id = value
            .get("id")
            .or_else(|| value.get("sandboxId"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                ReceiverError::Protocol(
                    "OpenSandbox create response did not include id".to_string(),
                )
            })?;
        let endpoint = self.resolve_exec_endpoint(&sandbox_id)?;
        Ok(SandboxHandle {
            sandbox_id: sandbox_id.clone(),
            repo: request.repo,
            source_session_id: request.session_id,
            target_session_id: sandbox_id,
            target_worktree: PathBuf::from(&self.config.worktree_root),
            exec_endpoint: endpoint.url,
            exec_headers: endpoint.headers,
        })
    }

    fn run(&self, handle: &SandboxHandle, plan: &ProofPlan) -> ReceiverResult<ProofReceipt> {
        let command = command_line(&plan.command, &plan.args);
        let response = self
            .exec_request(handle, "/command")
            .json(&json!({
                "command": command,
                "cwd": plan.cwd.display().to_string(),
                "background": false,
                "timeout": plan.timeout.as_millis() as u64,
                "envs": self.config.env.clone(),
            }))
            .send()?;
        let status = response.status();
        let body = response.text()?;
        if !status.is_success() {
            return Err(ReceiverError::Protocol(format!(
                "OpenSandbox command failed in {}: {} {}",
                handle.sandbox_id,
                status.as_u16(),
                truncate(&body)
            )));
        }
        Ok(proof_receipt_from_execd_body(plan, &body))
    }

    fn put_files(&self, handle: &SandboxHandle, files: &[SandboxFile]) -> ReceiverResult<()> {
        for file in files {
            let metadata = json!({
                "path": file.path.clone(),
                "mode": 0o644,
            })
            .to_string();
            let form = multipart::Form::new()
                .part("metadata", multipart::Part::text(metadata))
                .part(
                    "file",
                    multipart::Part::bytes(file.content.clone())
                        .file_name(file.path.clone())
                        .mime_str("application/octet-stream")
                        .map_err(|error| ReceiverError::Config(error.to_string()))?,
                );
            let response = self
                .exec_request(handle, "/files/upload")
                .multipart(form)
                .send()?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text()?;
                return Err(ReceiverError::Protocol(format!(
                    "OpenSandbox file upload failed for {}: {} {}",
                    file.path,
                    status.as_u16(),
                    truncate(&body)
                )));
            }
        }
        Ok(())
    }

    fn get_files(
        &self,
        handle: &SandboxHandle,
        paths: &[String],
    ) -> ReceiverResult<Vec<SandboxFile>> {
        let mut files = Vec::new();
        for path in paths {
            let mut request = self.http.get(format!(
                "{}/files/download",
                handle.exec_endpoint.trim_end_matches('/')
            ));
            for (name, value) in &handle.exec_headers {
                request = request.header(name, value);
            }
            let response = request.query(&[("path", path)]).send()?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text()?;
                return Err(ReceiverError::Protocol(format!(
                    "OpenSandbox file download failed for {path}: {} {}",
                    status.as_u16(),
                    truncate(&body)
                )));
            }
            files.push(SandboxFile {
                path: path.clone(),
                content: response.bytes()?.to_vec(),
            });
        }
        Ok(files)
    }

    fn destroy(&self, handle: &SandboxHandle) -> ReceiverResult<()> {
        let base = self.lifecycle_base()?;
        let response = self
            .with_auth(
                self.http
                    .delete(format!("{base}/sandboxes/{}", handle.sandbox_id)),
            )?
            .send()?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text()?;
            Err(ReceiverError::Protocol(format!(
                "OpenSandbox destroy failed for {}: {} {}",
                handle.sandbox_id,
                status.as_u16(),
                truncate(&body)
            )))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ExecEndpoint {
    url: String,
    headers: BTreeMap<String, String>,
}

fn exec_endpoint_from_value(value: Value) -> ReceiverResult<ExecEndpoint> {
    let url = value
        .get("url")
        .or_else(|| value.get("endpoint"))
        .or_else(|| value.get("publicUrl"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| {
            ReceiverError::Protocol("OpenSandbox endpoint response did not include url".to_string())
        })?;
    let headers = value
        .get("headers")
        .and_then(Value::as_object)
        .map(|headers| {
            headers
                .iter()
                .filter_map(|(name, value)| {
                    value
                        .as_str()
                        .map(|value| (name.clone(), value.to_string()))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    Ok(ExecEndpoint { url, headers })
}

fn proof_receipt_from_execd_body(plan: &ProofPlan, body: &str) -> ProofReceipt {
    let (stdout, stderr, exit_code, timed_out) = parse_execd_command_output(body);
    let passed = !timed_out && exit_code == Some(0);
    ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code,
        stdout,
        stderr,
        timed_out,
        status: if passed { "passed" } else { "failed" }.to_string(),
        trust_tier: TRUST_TIER_SANDBOX.to_string(),
    }
}

fn parse_execd_command_output(body: &str) -> (String, String, Option<i32>, bool) {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return command_output_from_value(&value);
    }
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut exit_code = None;
    let mut timed_out = false;
    for line in body.lines() {
        let Some(raw) = line.strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(raw.trim()) else {
            continue;
        };
        let (event_stdout, event_stderr, event_code, event_timeout) =
            command_output_from_value(&value);
        stdout.push_str(&event_stdout);
        stderr.push_str(&event_stderr);
        if event_code.is_some() {
            exit_code = event_code;
        }
        timed_out |= event_timeout;
    }
    (stdout, stderr, exit_code, timed_out)
}

fn command_output_from_value(value: &Value) -> (String, String, Option<i32>, bool) {
    let event_type = value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let text = value
        .get("text")
        .or_else(|| value.get("content"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let stdout = if event_type == "stdout" || value.get("stdout").is_some() {
        value
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or(&text)
            .to_string()
    } else {
        String::new()
    };
    let stderr = if event_type == "stderr" || value.get("stderr").is_some() {
        value
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or(&text)
            .to_string()
    } else {
        String::new()
    };
    let exit_code = value
        .get("exit_code")
        .or_else(|| value.get("exitCode"))
        .or_else(|| value.pointer("/result/exit_code"))
        .and_then(Value::as_i64)
        .map(|code| code as i32);
    let timed_out = value
        .get("timed_out")
        .or_else(|| value.get("timedOut"))
        .and_then(Value::as_bool)
        .unwrap_or_else(|| {
            value
                .get("error")
                .and_then(Value::as_str)
                .map(|error| error.to_ascii_lowercase().contains("timeout"))
                .unwrap_or(false)
        });
    (stdout, stderr, exit_code, timed_out)
}

fn command_line(command: &str, args: &[String]) -> String {
    std::iter::once(command.to_string())
        .chain(args.iter().map(|arg| shell_quote(arg)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || "-_./:=+".contains(character))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn persistent_volume_name(repo: &str, session_id: &str) -> String {
    let slug = format!("{repo}-{session_id}")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    format!("theorem-{}", slug.trim_matches('-'))
}

fn truncate(body: &str) -> String {
    body.chars().take(512).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sandbox_receipt_matches_local_shape_with_sandbox_tier() {
        let plan = ProofPlan::new(
            "cargo",
            vec!["test".to_string()],
            "/workspace/theorem",
            Duration::from_secs(30),
        );
        let body = r#"data: {"type":"stdout","text":"ok\n"}
data: {"type":"stderr","text":"warn\n"}
data: {"type":"result","exit_code":0}
"#;

        let receipt = proof_receipt_from_execd_body(&plan, body);

        assert_eq!(receipt.command, "cargo");
        assert_eq!(receipt.args, vec!["test"]);
        assert_eq!(receipt.cwd, "/workspace/theorem");
        assert_eq!(receipt.stdout, "ok\n");
        assert_eq!(receipt.stderr, "warn\n");
        assert_eq!(receipt.exit_code, Some(0));
        assert_eq!(receipt.status, "passed");
        assert_eq!(receipt.trust_tier, TRUST_TIER_SANDBOX);
    }

    #[test]
    fn provision_body_carries_persistent_volume_and_egress_policy() {
        let runtime = OpenSandboxRuntime::new(SandboxConfig {
            enabled: true,
            base_url: Some("http://localhost:8080/v1".to_string()),
            api_key_env: None,
            image: "ubuntu:22.04".to_string(),
            timeout_secs: 300,
            execd_port: 44_772,
            worktree_root: "/workspace/theorem".to_string(),
            secure_runtime: Some("gvisor".to_string()),
            egress_allowlist: vec!["litellm.internal".to_string(), "github.com".to_string()],
            env: BTreeMap::new(),
        })
        .unwrap();
        let body = runtime.create_body(&SandboxProvisionRequest::new(
            "Travis-Gilbert/Theorem",
            "session-123",
        ));

        assert_eq!(body["volumes"][0]["type"], json!("pvc"));
        assert_eq!(body["volumes"][0]["mountPath"], json!("/workspace/theorem"));
        assert_eq!(
            body["metadata"]["target_worktree"],
            json!("/workspace/theorem")
        );
        assert_eq!(body["networkPolicy"]["defaultAction"], json!("deny"));
        assert_eq!(body["networkPolicy"]["egress"].as_array().unwrap().len(), 2);
        assert_eq!(body["extensions"]["secureRuntime"], json!("gvisor"));
    }

    #[test]
    fn handle_target_fields_point_at_live_sandbox_identity() {
        let handle = SandboxHandle {
            sandbox_id: "sbx_123".to_string(),
            repo: "Travis-Gilbert/Theorem".to_string(),
            source_session_id: "room-session".to_string(),
            target_session_id: "sbx_123".to_string(),
            target_worktree: PathBuf::from("/workspace/theorem"),
            exec_endpoint: "http://localhost:44772".to_string(),
            exec_headers: BTreeMap::new(),
        };

        assert_eq!(handle.target_session_id, handle.sandbox_id);
        assert_eq!(handle.target_worktree, PathBuf::from("/workspace/theorem"));
    }

    #[test]
    fn command_line_quotes_arguments_for_shell_execution() {
        assert_eq!(
            command_line("sh", &["-c".to_string(), "echo hi".to_string()]),
            "sh -c 'echo hi'"
        );
    }
}
