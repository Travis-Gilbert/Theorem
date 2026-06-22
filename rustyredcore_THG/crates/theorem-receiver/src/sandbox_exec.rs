//! OpenSandbox execution backend for receiver proofs.
//!
//! This mirrors [`crate::local_exec`] at the contract boundary: callers provide
//! a [`ProofPlan`] and receive the same [`ProofReceipt`] shape, but the command
//! runs through an OpenSandbox execd endpoint and is tagged
//! `substrate_rerun_sandbox`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Duration;
use std::time::Instant;

use reqwest::blocking::{multipart, Client, RequestBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::SandboxConfig;
use crate::local_exec::{ProofPlan, ProofReceipt, TRUST_TIER_LOCAL};
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

/// Streaming output event from a sandbox-backed command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
    Exit {
        exit_code: Option<i32>,
        timed_out: bool,
        cancelled: bool,
    },
}

/// Cooperative cancellation token for streaming sandbox runs.
#[derive(Clone, Debug, Default)]
pub struct SandboxCancelToken {
    cancelled: Arc<AtomicBool>,
}

impl SandboxCancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::SeqCst)
    }
}

pub trait SandboxRuntime {
    fn provision(&self, request: SandboxProvisionRequest) -> ReceiverResult<SandboxHandle>;
    fn run(&self, handle: &SandboxHandle, plan: &ProofPlan) -> ReceiverResult<ProofReceipt>;
    fn run_streaming(
        &self,
        handle: &SandboxHandle,
        plan: &ProofPlan,
        cancel: &SandboxCancelToken,
        on_event: &mut dyn FnMut(&SandboxStreamEvent),
    ) -> ReceiverResult<ProofReceipt> {
        if cancel.is_cancelled() {
            let event = SandboxStreamEvent::Exit {
                exit_code: None,
                timed_out: false,
                cancelled: true,
            };
            on_event(&event);
            return Ok(cancelled_receipt(plan));
        }
        let receipt = self.run(handle, plan)?;
        if !receipt.stdout.is_empty() {
            on_event(&SandboxStreamEvent::Stdout(
                receipt.stdout.as_bytes().to_vec(),
            ));
        }
        if !receipt.stderr.is_empty() {
            on_event(&SandboxStreamEvent::Stderr(
                receipt.stderr.as_bytes().to_vec(),
            ));
        }
        on_event(&SandboxStreamEvent::Exit {
            exit_code: receipt.exit_code,
            timed_out: receipt.timed_out,
            cancelled: false,
        });
        Ok(receipt)
    }
    fn put_files(&self, handle: &SandboxHandle, files: &[SandboxFile]) -> ReceiverResult<()>;
    fn get_files(
        &self,
        handle: &SandboxHandle,
        paths: &[String],
    ) -> ReceiverResult<Vec<SandboxFile>>;
    fn destroy(&self, handle: &SandboxHandle) -> ReceiverResult<()>;
}

/// Development/no-sidecar implementation of [`SandboxRuntime`].
///
/// It uses a real temporary directory and local child process, while preserving
/// the sandbox contract shape: files enter through `put_files`, commands run
/// against the handle's target worktree, and changed files are read through
/// `get_files`. Environment inheritance is deny-by-default so this backend does
/// not regress to the receiver's older unstripped recipe execution path.
#[derive(Clone, Debug)]
pub struct LocalProcessSandbox {
    root_parent: PathBuf,
}

impl LocalProcessSandbox {
    pub fn new() -> Self {
        Self {
            root_parent: std::env::temp_dir(),
        }
    }

    pub fn with_root_parent(root_parent: impl Into<PathBuf>) -> Self {
        Self {
            root_parent: root_parent.into(),
        }
    }
}

impl Default for LocalProcessSandbox {
    fn default() -> Self {
        Self::new()
    }
}

impl SandboxRuntime for LocalProcessSandbox {
    fn provision(&self, request: SandboxProvisionRequest) -> ReceiverResult<SandboxHandle> {
        fs::create_dir_all(&self.root_parent)?;
        let sandbox_id = format!("local-sandbox-{}", local_sandbox_stamp());
        let target_worktree = self.root_parent.join(&sandbox_id);
        fs::create_dir_all(&target_worktree)?;
        Ok(SandboxHandle {
            sandbox_id: sandbox_id.clone(),
            repo: request.repo,
            source_session_id: request.session_id,
            target_session_id: sandbox_id,
            target_worktree,
            exec_endpoint: "local-process".to_string(),
            exec_headers: BTreeMap::new(),
        })
    }

    fn run(&self, handle: &SandboxHandle, plan: &ProofPlan) -> ReceiverResult<ProofReceipt> {
        if !plan.cwd.starts_with(&handle.target_worktree) {
            return Err(ReceiverError::Config(format!(
                "proof cwd {:?} is outside sandbox worktree {:?}",
                plan.cwd, handle.target_worktree
            )));
        }
        run_local_sandbox_process(plan)
    }

    fn run_streaming(
        &self,
        handle: &SandboxHandle,
        plan: &ProofPlan,
        cancel: &SandboxCancelToken,
        on_event: &mut dyn FnMut(&SandboxStreamEvent),
    ) -> ReceiverResult<ProofReceipt> {
        if !plan.cwd.starts_with(&handle.target_worktree) {
            return Err(ReceiverError::Config(format!(
                "proof cwd {:?} is outside sandbox worktree {:?}",
                plan.cwd, handle.target_worktree
            )));
        }
        run_local_sandbox_streaming(plan, cancel, on_event)
    }

    fn put_files(&self, handle: &SandboxHandle, files: &[SandboxFile]) -> ReceiverResult<()> {
        for file in files {
            let relative = safe_sandbox_relative_path(&file.path)?;
            let path = handle.target_worktree.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, &file.content)?;
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
            let relative = safe_sandbox_relative_path(path)?;
            files.push(SandboxFile {
                path: path.clone(),
                content: fs::read(handle.target_worktree.join(relative))?,
            });
        }
        Ok(files)
    }

    fn destroy(&self, handle: &SandboxHandle) -> ReceiverResult<()> {
        if handle.sandbox_id.starts_with("local-sandbox-") && handle.target_worktree.exists() {
            fs::remove_dir_all(&handle.target_worktree)?;
        }
        Ok(())
    }
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

    fn run_streaming(
        &self,
        handle: &SandboxHandle,
        plan: &ProofPlan,
        cancel: &SandboxCancelToken,
        on_event: &mut dyn FnMut(&SandboxStreamEvent),
    ) -> ReceiverResult<ProofReceipt> {
        if cancel.is_cancelled() {
            on_event(&SandboxStreamEvent::Exit {
                exit_code: None,
                timed_out: false,
                cancelled: true,
            });
            return Ok(cancelled_receipt(plan));
        }
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
        if !status.is_success() {
            let body = response.text()?;
            return Err(ReceiverError::Protocol(format!(
                "OpenSandbox command failed in {}: {} {}",
                handle.sandbox_id,
                status.as_u16(),
                truncate(&body)
            )));
        }
        proof_receipt_from_execd_stream(plan, response, cancel, on_event)
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

fn proof_receipt_from_execd_stream<R: Read>(
    plan: &ProofPlan,
    mut reader: R,
    cancel: &SandboxCancelToken,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) -> ReceiverResult<ProofReceipt> {
    let mut buffer = [0u8; 4096];
    let mut body = String::new();
    let mut pending = String::new();
    let mut stream = ExecdStreamState::default();

    loop {
        if cancel.is_cancelled() {
            on_event(&SandboxStreamEvent::Exit {
                exit_code: None,
                timed_out: false,
                cancelled: true,
            });
            return Ok(cancelled_stream_receipt(plan, stream.stdout, stream.stderr));
        }
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer[..read]);
        body.push_str(&chunk);
        pending.push_str(&chunk);
        drain_execd_stream_lines(&mut pending, &mut stream, cancel, on_event);
    }
    if !pending.trim().is_empty() {
        process_execd_stream_line(pending.trim_end_matches('\r'), &mut stream, on_event);
    }

    if stream.saw_event {
        let event = SandboxStreamEvent::Exit {
            exit_code: stream.exit_code,
            timed_out: stream.timed_out,
            cancelled: false,
        };
        on_event(&event);
        let passed = !stream.timed_out && stream.exit_code == Some(0);
        Ok(ProofReceipt {
            command: plan.command.clone(),
            args: plan.args.clone(),
            cwd: plan.cwd.display().to_string(),
            exit_code: stream.exit_code,
            stdout: stream.stdout,
            stderr: stream.stderr,
            timed_out: stream.timed_out,
            status: if passed { "passed" } else { "failed" }.to_string(),
            trust_tier: TRUST_TIER_SANDBOX.to_string(),
        })
    } else {
        let receipt = proof_receipt_from_execd_body(plan, &body);
        emit_receipt_events(&receipt, on_event);
        Ok(receipt)
    }
}

#[derive(Default)]
struct ExecdStreamState {
    stdout: String,
    stderr: String,
    exit_code: Option<i32>,
    timed_out: bool,
    saw_event: bool,
}

fn drain_execd_stream_lines(
    pending: &mut String,
    stream: &mut ExecdStreamState,
    cancel: &SandboxCancelToken,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) {
    while let Some(index) = pending.find('\n') {
        let mut line = pending[..index].to_string();
        if line.ends_with('\r') {
            line.pop();
        }
        pending.drain(..=index);
        process_execd_stream_line(&line, stream, on_event);
        if cancel.is_cancelled() {
            break;
        }
    }
}

fn process_execd_stream_line(
    line: &str,
    stream: &mut ExecdStreamState,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) {
    let Some(raw) = line.trim_start().strip_prefix("data:") else {
        return;
    };
    let raw = raw.trim();
    if raw.is_empty() || raw == "[DONE]" {
        stream.saw_event = true;
        return;
    }
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    stream.saw_event = true;
    let (stdout, stderr, exit_code, timed_out) = command_output_from_value(&value);
    if !stdout.is_empty() {
        on_event(&SandboxStreamEvent::Stdout(stdout.as_bytes().to_vec()));
        stream.stdout.push_str(&stdout);
    }
    if !stderr.is_empty() {
        on_event(&SandboxStreamEvent::Stderr(stderr.as_bytes().to_vec()));
        stream.stderr.push_str(&stderr);
    }
    if exit_code.is_some() {
        stream.exit_code = exit_code;
    }
    stream.timed_out |= timed_out;
}

fn emit_receipt_events(receipt: &ProofReceipt, on_event: &mut dyn FnMut(&SandboxStreamEvent)) {
    if !receipt.stdout.is_empty() {
        on_event(&SandboxStreamEvent::Stdout(
            receipt.stdout.as_bytes().to_vec(),
        ));
    }
    if !receipt.stderr.is_empty() {
        on_event(&SandboxStreamEvent::Stderr(
            receipt.stderr.as_bytes().to_vec(),
        ));
    }
    on_event(&SandboxStreamEvent::Exit {
        exit_code: receipt.exit_code,
        timed_out: receipt.timed_out,
        cancelled: false,
    });
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

fn run_local_sandbox_process(plan: &ProofPlan) -> ReceiverResult<ProofReceipt> {
    let stamp = local_sandbox_stamp();
    let out_path = std::env::temp_dir().join(format!("theorem-sandbox-{stamp}.out"));
    let err_path = std::env::temp_dir().join(format!("theorem-sandbox-{stamp}.err"));
    let outcome = run_local_sandbox_capture(plan, &out_path, &err_path);

    let stdout = fs::read_to_string(&out_path).unwrap_or_default();
    let stderr = fs::read_to_string(&err_path).unwrap_or_default();
    let _ = fs::remove_file(&out_path);
    let _ = fs::remove_file(&err_path);

    let (exit_code, timed_out) = outcome?;
    let passed = !timed_out && exit_code == Some(0);
    Ok(ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code,
        stdout,
        stderr,
        timed_out,
        status: if passed { "passed" } else { "failed" }.to_string(),
        trust_tier: TRUST_TIER_LOCAL.to_string(),
    })
}

fn run_local_sandbox_streaming(
    plan: &ProofPlan,
    cancel: &SandboxCancelToken,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) -> ReceiverResult<ProofReceipt> {
    let mut command = Command::new(&plan.command);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in local_sandbox_inherited_env() {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }

    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| ReceiverError::Protocol("child stdout pipe missing".to_string()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| ReceiverError::Protocol("child stderr pipe missing".to_string()))?;
    let (tx, rx) = mpsc::channel();
    let stdout_thread = spawn_stream_reader(stdout, tx.clone(), StreamKind::Stdout);
    let stderr_thread = spawn_stream_reader(stderr, tx, StreamKind::Stderr);

    let deadline = Instant::now() + plan.timeout;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut exit_code = None;
    let mut timed_out = false;
    let mut cancelled = false;

    loop {
        drain_stream_events(&rx, &mut stdout_bytes, &mut stderr_bytes, on_event);
        if let Some(status) = child.try_wait()? {
            exit_code = status.code();
            break;
        }
        if cancel.is_cancelled() {
            cancelled = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();
    drain_stream_events(&rx, &mut stdout_bytes, &mut stderr_bytes, on_event);

    let exit_event = SandboxStreamEvent::Exit {
        exit_code,
        timed_out,
        cancelled,
    };
    on_event(&exit_event);

    let status = if !timed_out && !cancelled && exit_code == Some(0) {
        "passed"
    } else if cancelled {
        "cancelled"
    } else {
        "failed"
    };
    Ok(ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
        timed_out,
        status: status.to_string(),
        trust_tier: TRUST_TIER_LOCAL.to_string(),
    })
}

enum StreamKind {
    Stdout,
    Stderr,
}

fn spawn_stream_reader<R: Read + Send + 'static>(
    mut reader: R,
    tx: mpsc::Sender<SandboxStreamEvent>,
    kind: StreamKind,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let bytes = buf[..n].to_vec();
                    let event = match kind {
                        StreamKind::Stdout => SandboxStreamEvent::Stdout(bytes),
                        StreamKind::Stderr => SandboxStreamEvent::Stderr(bytes),
                    };
                    if tx.send(event).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

fn drain_stream_events(
    rx: &mpsc::Receiver<SandboxStreamEvent>,
    stdout: &mut Vec<u8>,
    stderr: &mut Vec<u8>,
    on_event: &mut dyn FnMut(&SandboxStreamEvent),
) {
    while let Ok(event) = rx.try_recv() {
        match &event {
            SandboxStreamEvent::Stdout(bytes) => stdout.extend_from_slice(bytes),
            SandboxStreamEvent::Stderr(bytes) => stderr.extend_from_slice(bytes),
            SandboxStreamEvent::Exit { .. } => {}
        }
        on_event(&event);
    }
}

fn cancelled_receipt(plan: &ProofPlan) -> ProofReceipt {
    ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code: None,
        stdout: String::new(),
        stderr: String::new(),
        timed_out: false,
        status: "cancelled".to_string(),
        trust_tier: TRUST_TIER_SANDBOX.to_string(),
    }
}

fn cancelled_stream_receipt(plan: &ProofPlan, stdout: String, stderr: String) -> ProofReceipt {
    ProofReceipt {
        command: plan.command.clone(),
        args: plan.args.clone(),
        cwd: plan.cwd.display().to_string(),
        exit_code: None,
        stdout,
        stderr,
        timed_out: false,
        status: "cancelled".to_string(),
        trust_tier: TRUST_TIER_SANDBOX.to_string(),
    }
}

fn run_local_sandbox_capture(
    plan: &ProofPlan,
    out_path: &Path,
    err_path: &Path,
) -> ReceiverResult<(Option<i32>, bool)> {
    let out_file = fs::File::create(out_path)?;
    let err_file = fs::File::create(err_path)?;
    let mut command = Command::new(&plan.command);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(out_file)
        .stderr(err_file);
    for key in local_sandbox_inherited_env() {
        if let Ok(value) = std::env::var(key) {
            command.env(key, value);
        }
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + plan.timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok((status.code(), false));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok((None, true));
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

fn local_sandbox_inherited_env() -> &'static [&'static str] {
    &[
        "PATH",
        "HOME",
        "TMPDIR",
        "TEMP",
        "TMP",
        "CARGO_HOME",
        "RUSTUP_HOME",
    ]
}

fn safe_sandbox_relative_path(path: &str) -> ReceiverResult<PathBuf> {
    let raw = Path::new(path);
    let mut out = PathBuf::new();
    for component in raw.components() {
        match component {
            Component::Normal(value) => out.push(value),
            Component::CurDir => {}
            _ => {
                return Err(ReceiverError::Config(format!(
                    "unsafe sandbox path {path:?}"
                )));
            }
        }
    }
    if out.as_os_str().is_empty() {
        return Err(ReceiverError::Config("empty sandbox path".to_string()));
    }
    Ok(out)
}

fn local_sandbox_stamp() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{n}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::sync::{Mutex, OnceLock};
    use std::thread;

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
    fn execd_stream_emits_events_and_receipt_shape() {
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
        let cancel = SandboxCancelToken::new();
        let mut events = Vec::new();

        let receipt =
            proof_receipt_from_execd_stream(&plan, body.as_bytes(), &cancel, &mut |event| {
                events.push(event.clone())
            })
            .unwrap();

        assert_eq!(receipt.status, "passed");
        assert_eq!(receipt.stdout, "ok\n");
        assert_eq!(receipt.stderr, "warn\n");
        assert_eq!(receipt.exit_code, Some(0));
        assert_eq!(receipt.trust_tier, TRUST_TIER_SANDBOX);
        assert!(events
            .iter()
            .any(|event| matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ok\n")));
        assert!(
            events.iter().any(|event| matches!(
                event,
                SandboxStreamEvent::Exit {
                    exit_code: Some(0),
                    cancelled: false,
                    ..
                }
            )),
            "stream callback saw exit event: {events:?}"
        );
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

    #[test]
    fn open_sandbox_runtime_streams_execd_response_and_cancels() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let read = stream.read(&mut request).unwrap();
            let request = String::from_utf8_lossy(&request[..read]);
            assert!(request.contains("POST /command HTTP/1.1"));
            assert!(request.contains("\"command\""));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n")
                .unwrap();
            stream
                .write_all(br#"data: {"type":"stdout","text":"ready"}"#)
                .unwrap();
            stream.write_all(b"\n").unwrap();
            stream.flush().unwrap();
            thread::sleep(Duration::from_millis(200));
            let _ = stream.write_all(br#"data: {"type":"stdout","text":"late"}"#);
            let _ = stream.write_all(b"\n");
        });
        let runtime = OpenSandboxRuntime::new(SandboxConfig {
            enabled: true,
            base_url: None,
            api_key_env: None,
            image: "ubuntu:22.04".to_string(),
            timeout_secs: 300,
            execd_port: 44_772,
            worktree_root: "/workspace/theorem".to_string(),
            secure_runtime: None,
            egress_allowlist: Vec::new(),
            env: BTreeMap::new(),
        })
        .unwrap();
        let handle = SandboxHandle {
            sandbox_id: "sbx_stream".to_string(),
            repo: "Travis-Gilbert/Theorem".to_string(),
            source_session_id: "room-session".to_string(),
            target_session_id: "sbx_stream".to_string(),
            target_worktree: PathBuf::from("/workspace/theorem"),
            exec_endpoint: endpoint,
            exec_headers: BTreeMap::new(),
        };
        let cancel = SandboxCancelToken::new();
        let cancel_from_event = cancel.clone();
        let mut events = Vec::new();

        let receipt = runtime
            .run_streaming(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec!["-c".to_string(), "printf ready; sleep 5".to_string()],
                    "/workspace/theorem",
                    Duration::from_secs(30),
                ),
                &cancel,
                &mut |event| {
                    if matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ready") {
                        cancel_from_event.cancel();
                    }
                    events.push(event.clone());
                },
            )
            .unwrap();

        assert_eq!(receipt.status, "cancelled");
        assert_eq!(receipt.stdout, "ready");
        assert_eq!(receipt.trust_tier, TRUST_TIER_SANDBOX);
        assert!(
            events.iter().any(|event| matches!(
                event,
                SandboxStreamEvent::Exit {
                    cancelled: true,
                    timed_out: false,
                    ..
                }
            )),
            "stream callback saw cancellation exit event: {events:?}"
        );
        server.join().unwrap();
    }

    #[test]
    fn local_process_sandbox_uses_put_run_get_and_strips_sensitive_env() {
        let _env = ScopedEnv::new("ANTHROPIC_API_KEY", Some("must-not-leak"));
        let runtime = LocalProcessSandbox::new();
        let handle = runtime
            .provision(SandboxProvisionRequest::new("Travis-Gilbert/Theorem", "w3"))
            .unwrap();
        runtime
            .put_files(
                &handle,
                &[SandboxFile {
                    path: "src/main.rs".to_string(),
                    content: b"fn main() {}\n".to_vec(),
                }],
            )
            .unwrap();

        let receipt = runtime
            .run(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec![
                        "-c".to_string(),
                        "test -z \"$ANTHROPIC_API_KEY\" && printf 'changed\\n' > src/main.rs && printf ok"
                            .to_string(),
                    ],
                    handle.target_worktree.clone(),
                    Duration::from_secs(5),
                ),
            )
            .unwrap();

        assert_eq!(receipt.exit_code, Some(0), "stderr={}", receipt.stderr);
        assert_eq!(receipt.stdout, "ok");
        assert_eq!(receipt.trust_tier, TRUST_TIER_LOCAL);
        let files = runtime
            .get_files(&handle, &["src/main.rs".to_string()])
            .unwrap();
        assert_eq!(files[0].content, b"changed\n");
        runtime.destroy(&handle).unwrap();
        assert!(!handle.target_worktree.exists());
    }

    #[test]
    fn local_process_sandbox_streams_output_and_cancels_running_command() {
        let runtime = LocalProcessSandbox::new();
        let handle = runtime
            .provision(SandboxProvisionRequest::new(
                "Travis-Gilbert/Theorem",
                "w3-stream",
            ))
            .unwrap();
        let cancel = SandboxCancelToken::new();
        let cancel_from_event = cancel.clone();
        let mut events = Vec::new();

        let receipt = runtime
            .run_streaming(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec![
                        "-c".to_string(),
                        "printf ready; sleep 5; printf late".to_string(),
                    ],
                    handle.target_worktree.clone(),
                    Duration::from_secs(30),
                ),
                &cancel,
                &mut |event| {
                    if matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ready") {
                        cancel_from_event.cancel();
                    }
                    events.push(event.clone());
                },
            )
            .unwrap();

        assert_eq!(receipt.status, "cancelled");
        assert_eq!(receipt.exit_code, None);
        assert_eq!(receipt.stdout, "ready");
        assert!(
            events.iter().any(
                |event| matches!(event, SandboxStreamEvent::Stdout(bytes) if bytes == b"ready")
            ),
            "stream callback saw stdout before cancellation: {events:?}"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                SandboxStreamEvent::Exit {
                    cancelled: true,
                    timed_out: false,
                    ..
                }
            )),
            "stream callback saw cancellation exit event: {events:?}"
        );
        runtime.destroy(&handle).unwrap();
    }

    #[test]
    fn local_process_sandbox_streaming_reports_timeout_exit_event() {
        let runtime = LocalProcessSandbox::new();
        let handle = runtime
            .provision(SandboxProvisionRequest::new(
                "Travis-Gilbert/Theorem",
                "w3-stream-timeout",
            ))
            .unwrap();
        let cancel = SandboxCancelToken::new();
        let mut events = Vec::new();

        let receipt = runtime
            .run_streaming(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec!["-c".to_string(), "printf start; sleep 5".to_string()],
                    handle.target_worktree.clone(),
                    Duration::from_millis(100),
                ),
                &cancel,
                &mut |event| events.push(event.clone()),
            )
            .unwrap();

        assert!(receipt.timed_out);
        assert_eq!(receipt.status, "failed");
        assert_eq!(receipt.stdout, "start");
        assert!(
            events.iter().any(|event| matches!(
                event,
                SandboxStreamEvent::Exit {
                    timed_out: true,
                    cancelled: false,
                    ..
                }
            )),
            "stream callback saw timeout exit event: {events:?}"
        );
        runtime.destroy(&handle).unwrap();
    }

    #[test]
    #[ignore = "requires a live OpenSandbox sidecar; set OPEN_SANDBOX_BASE_URL and optionally OPEN_SANDBOX_API_KEY"]
    fn live_open_sandbox_round_trips_files_and_receipt_shape() {
        let runtime = live_open_sandbox_runtime();
        let handle = runtime
            .provision(SandboxProvisionRequest::new(
                "Travis-Gilbert/Theorem",
                "w3-live-smoke",
            ))
            .expect("provision live sandbox");

        let result = (|| -> ReceiverResult<()> {
            runtime.put_files(
                &handle,
                &[SandboxFile {
                    path: "src/main.rs".to_string(),
                    content: b"fn main() {}\n".to_vec(),
                }],
            )?;
            let receipt = runtime.run(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec![
                        "-lc".to_string(),
                        "test -f src/main.rs && printf 'fn main() { println!(\"sandbox\"); }\\n' > src/main.rs && printf sandbox-ok"
                            .to_string(),
                    ],
                    handle.target_worktree.clone(),
                    Duration::from_secs(30),
                ),
            )?;
            assert_eq!(receipt.exit_code, Some(0), "stderr={}", receipt.stderr);
            assert!(
                receipt.stdout.contains("sandbox-ok"),
                "stdout={}",
                receipt.stdout
            );
            assert_eq!(receipt.trust_tier, TRUST_TIER_SANDBOX);
            let files = runtime.get_files(&handle, &["src/main.rs".to_string()])?;
            assert_eq!(files[0].content, b"fn main() { println!(\"sandbox\"); }\n");
            Ok(())
        })();

        let destroy_result = runtime.destroy(&handle);
        result.expect("live OpenSandbox put/run/get");
        destroy_result.expect("destroy live sandbox");
    }

    #[test]
    #[ignore = "requires a live OpenSandbox sidecar; set OPEN_SANDBOX_BASE_URL and optionally OPEN_SANDBOX_API_KEY"]
    fn live_open_sandbox_streaming_can_cancel_running_command() {
        let runtime = live_open_sandbox_runtime();
        let handle = runtime
            .provision(SandboxProvisionRequest::new(
                "Travis-Gilbert/Theorem",
                "w3-live-streaming",
            ))
            .expect("provision live sandbox");

        let result = (|| -> ReceiverResult<()> {
            let cancel = SandboxCancelToken::new();
            let cancel_from_event = cancel.clone();
            let mut events = Vec::new();
            let receipt = runtime.run_streaming(
                &handle,
                &ProofPlan::new(
                    "sh",
                    vec![
                        "-lc".to_string(),
                        "printf 'ready\\n'; sleep 30; printf late".to_string(),
                    ],
                    handle.target_worktree.clone(),
                    Duration::from_secs(60),
                ),
                &cancel,
                &mut |event| {
                    if matches!(event, SandboxStreamEvent::Stdout(bytes) if stdout_contains(bytes, "ready")) {
                        cancel_from_event.cancel();
                    }
                    events.push(event.clone());
                },
            )?;

            assert_eq!(receipt.trust_tier, TRUST_TIER_SANDBOX);
            assert_eq!(receipt.status, "cancelled");
            assert_eq!(receipt.exit_code, None);
            assert!(!receipt.timed_out);
            assert!(
                receipt.stdout.contains("ready"),
                "stdout={}",
                receipt.stdout
            );
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    SandboxStreamEvent::Stdout(bytes) if stdout_contains(bytes, "ready")
                )),
                "stream callback saw stdout before cancellation: {events:?}"
            );
            assert!(
                events.iter().any(|event| matches!(
                    event,
                    SandboxStreamEvent::Exit {
                        cancelled: true,
                        timed_out: false,
                        ..
                    }
                )),
                "stream callback saw cancellation exit event: {events:?}"
            );
            Ok(())
        })();

        let destroy_result = runtime.destroy(&handle);
        result.expect("live OpenSandbox streaming/cancellation");
        destroy_result.expect("destroy live sandbox");
    }

    fn live_open_sandbox_runtime() -> OpenSandboxRuntime {
        let base_url = std::env::var("OPEN_SANDBOX_BASE_URL")
            .expect("set OPEN_SANDBOX_BASE_URL to run the live OpenSandbox smoke");
        let execd_port = std::env::var("OPEN_SANDBOX_EXECD_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(crate::config::DEFAULT_OPENSANDBOX_EXECD_PORT);
        let worktree_root = std::env::var("OPEN_SANDBOX_WORKTREE_ROOT")
            .unwrap_or_else(|_| crate::config::DEFAULT_OPENSANDBOX_WORKTREE_ROOT.to_string());
        let image = std::env::var("OPEN_SANDBOX_IMAGE")
            .unwrap_or_else(|_| crate::config::DEFAULT_OPENSANDBOX_IMAGE.to_string());
        let api_key_env = std::env::var("OPEN_SANDBOX_API_KEY")
            .ok()
            .map(|_| "OPEN_SANDBOX_API_KEY".to_string());
        OpenSandboxRuntime::new(SandboxConfig {
            enabled: true,
            base_url: Some(base_url),
            api_key_env,
            image,
            timeout_secs: 300,
            execd_port,
            worktree_root,
            secure_runtime: None,
            egress_allowlist: Vec::new(),
            env: BTreeMap::new(),
        })
        .unwrap()
    }

    fn stdout_contains(bytes: &[u8], needle: &str) -> bool {
        String::from_utf8_lossy(bytes).contains(needle)
    }

    struct ScopedEnv {
        key: &'static str,
        previous: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl ScopedEnv {
        fn new(key: &'static str, value: Option<&str>) -> Self {
            static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
            let guard = ENV_LOCK
                .get_or_init(|| Mutex::new(()))
                .lock()
                .expect("env lock poisoned");
            let previous = std::env::var(key).ok();
            match value {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
            Self {
                key,
                previous,
                _guard: guard,
            }
        }
    }

    impl Drop for ScopedEnv {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
