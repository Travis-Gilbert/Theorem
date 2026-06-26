//! content-core subprocess front-end for CommonPlace ingest.
//!
//! This module deliberately treats content-core as an extraction step only. It
//! shells out to the installed CLI, parses the JSON output, and returns text plus
//! extraction metadata for the existing ingest organizer to consume.

use std::collections::BTreeMap;
use std::io;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const CONTENT_CORE_ENABLED_ENV: &str = "THEOREM_CONTENT_CORE_ENABLED";
pub const CONTENT_CORE_COMMAND_ENV: &str = "THEOREM_CONTENT_CORE_COMMAND";
pub const CONTENT_CORE_ARGS_ENV: &str = "THEOREM_CONTENT_CORE_ARGS";
pub const CONTENT_CORE_TIMEOUT_MS_ENV: &str = "THEOREM_CONTENT_CORE_TIMEOUT_MS";
pub const DEFAULT_CONTENT_CORE_TIMEOUT_MS: u64 = 120_000;

pub const CCORE_ENV_KEYS: &[&str] = &[
    "CCORE_URL_ENGINE",
    "CCORE_DOCUMENT_ENGINE",
    "CCORE_STT_PROVIDER",
    "CCORE_STT_MODEL",
    "CCORE_AUDIO_CONCURRENCY",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentCoreCommand {
    pub program: String,
    pub prefix_args: Vec<String>,
}

impl ContentCoreCommand {
    pub fn new(program: impl Into<String>, prefix_args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            prefix_args,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContentCoreExtractionConfig {
    pub enabled: bool,
    pub timeout: Duration,
    pub commands: Vec<ContentCoreCommand>,
    pub env: BTreeMap<String, String>,
}

impl Default for ContentCoreExtractionConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl ContentCoreExtractionConfig {
    pub fn from_env() -> Self {
        let enabled = std::env::var(CONTENT_CORE_ENABLED_ENV)
            .ok()
            .map(|value| !is_falsey(&value))
            .unwrap_or(true);
        let timeout = std::env::var(CONTENT_CORE_TIMEOUT_MS_ENV)
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_CONTENT_CORE_TIMEOUT_MS);
        let mut env = BTreeMap::new();
        for key in CCORE_ENV_KEYS {
            if let Ok(value) = std::env::var(key) {
                if !value.trim().is_empty() {
                    env.insert((*key).to_string(), value);
                }
            }
        }
        Self {
            enabled,
            timeout: Duration::from_millis(timeout),
            commands: content_core_commands_from_env(),
            env,
        }
    }

    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Self::from_env()
        }
    }

    pub fn with_commands(mut self, commands: Vec<ContentCoreCommand>) -> Self {
        self.commands = commands;
        self
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn engine_for_source(&self, source: &str, source_type: Option<&str>) -> String {
        let is_url = source_type == Some("url") || is_url(source);
        let key = if is_url {
            "CCORE_URL_ENGINE"
        } else {
            "CCORE_DOCUMENT_ENGINE"
        };
        self.env
            .get(key)
            .cloned()
            .or_else(|| std::env::var(key).ok())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "auto".to_string())
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ExtractedDoc {
    pub text: String,
    pub title: Option<String>,
    pub source_type: Option<String>,
    pub detected_type: Option<String>,
    pub engine: Option<String>,
    pub metadata: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContentCoreExtractionError {
    Disabled,
    Unavailable(String),
    Timeout { timeout_ms: u64 },
    Failed { status: Option<i32>, stderr: String },
    InvalidJson(String),
}

impl ContentCoreExtractionError {
    pub fn reason(&self) -> String {
        match self {
            Self::Disabled => "content-core extraction disabled".to_string(),
            Self::Unavailable(message) => format!("content-core unavailable: {message}"),
            Self::Timeout { timeout_ms } => {
                format!("content-core extraction timed out after {timeout_ms}ms")
            }
            Self::Failed { status, stderr } => {
                let code = status
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "signal".to_string());
                let detail = stderr.trim();
                if detail.is_empty() {
                    format!("content-core extract exited with status {code}")
                } else {
                    format!("content-core extract exited with status {code}: {detail}")
                }
            }
            Self::InvalidJson(message) => {
                format!("content-core returned invalid JSON: {message}")
            }
        }
    }
}

pub fn content_core_extract(
    source: impl AsRef<str>,
) -> Result<ExtractedDoc, ContentCoreExtractionError> {
    let config = ContentCoreExtractionConfig::from_env();
    content_core_extract_with_config(source.as_ref(), &config)
}

pub fn content_core_extract_with_config(
    source: &str,
    config: &ContentCoreExtractionConfig,
) -> Result<ExtractedDoc, ContentCoreExtractionError> {
    if !config.enabled {
        return Err(ContentCoreExtractionError::Disabled);
    }
    let mut unavailable = Vec::new();
    for command in &config.commands {
        match run_content_core_command(command, source, config) {
            Ok(value) => return parse_extracted_doc(source, value, config),
            Err(ContentCoreExtractionError::Unavailable(message)) => unavailable.push(message),
            Err(error) => return Err(error),
        }
    }
    Err(ContentCoreExtractionError::Unavailable(
        unavailable.join("; "),
    ))
}

pub fn parse_extracted_doc(
    source: &str,
    value: Value,
    config: &ContentCoreExtractionConfig,
) -> Result<ExtractedDoc, ContentCoreExtractionError> {
    let text = value
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let title = optional_string(&value, "title");
    let source_type = optional_string(&value, "source_type");
    let detected_type = optional_string(&value, "identified_type")
        .or_else(|| optional_string(&value, "detected_type"))
        .or_else(|| source_type.clone());
    let metadata = value.get("metadata").cloned().unwrap_or_else(|| json!({}));
    let engine = optional_string(&value, "engine")
        .or_else(|| optional_string(&metadata, "engine"))
        .or_else(|| optional_string(&metadata, "used_engine"))
        .or_else(|| optional_string(&metadata, "url_engine"))
        .or_else(|| optional_string(&metadata, "document_engine"))
        .or_else(|| Some(config.engine_for_source(source, source_type.as_deref())));
    Ok(ExtractedDoc {
        text,
        title,
        source_type,
        detected_type,
        engine,
        metadata,
    })
}

fn run_content_core_command(
    command: &ContentCoreCommand,
    source: &str,
    config: &ContentCoreExtractionConfig,
) -> Result<Value, ContentCoreExtractionError> {
    let mut cmd = Command::new(&command.program);
    cmd.args(&command.prefix_args)
        .arg("extract")
        .arg(source)
        .arg("--format")
        .arg("json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in &config.env {
        cmd.env(key, value);
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(ContentCoreExtractionError::Unavailable(format!(
                "spawn {}: {error}",
                command.program
            )));
        }
        Err(error) => {
            return Err(ContentCoreExtractionError::Unavailable(format!(
                "spawn {}: {error}",
                command.program
            )));
        }
    };

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child
                    .wait_with_output()
                    .map_err(|error| ContentCoreExtractionError::Unavailable(error.to_string()))?;
                if !output.status.success() {
                    return Err(ContentCoreExtractionError::Failed {
                        status: output.status.code(),
                        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
                    });
                }
                return serde_json::from_slice(&output.stdout)
                    .map_err(|error| ContentCoreExtractionError::InvalidJson(error.to_string()));
            }
            Ok(None) => {
                if started.elapsed() >= config.timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(ContentCoreExtractionError::Timeout {
                        timeout_ms: config.timeout.as_millis() as u64,
                    });
                }
                thread::sleep(Duration::from_millis(25));
            }
            Err(error) => {
                return Err(ContentCoreExtractionError::Unavailable(error.to_string()));
            }
        }
    }
}

fn content_core_commands_from_env() -> Vec<ContentCoreCommand> {
    if let Ok(raw) = std::env::var(CONTENT_CORE_COMMAND_ENV) {
        let mut parts = split_words(&raw);
        if !parts.is_empty() {
            let program = parts.remove(0);
            let mut prefix_args = parts;
            prefix_args.extend(
                std::env::var(CONTENT_CORE_ARGS_ENV)
                    .ok()
                    .map(|value| split_words(&value))
                    .unwrap_or_default(),
            );
            return vec![ContentCoreCommand::new(program, prefix_args)];
        }
    }
    vec![
        ContentCoreCommand::new("content-core", Vec::new()),
        ContentCoreCommand::new("uvx", vec!["content-core".to_string()]),
    ]
}

fn split_words(raw: &str) -> Vec<String> {
    raw.split_whitespace()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect()
}

fn optional_string(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn is_url(source: &str) -> bool {
    let lower = source.trim().to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn is_falsey(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "0" | "false" | "no" | "off" | "disabled"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracted_doc_reads_content_core_json_shape() {
        let config = ContentCoreExtractionConfig {
            enabled: true,
            timeout: Duration::from_secs(1),
            commands: vec![],
            env: BTreeMap::from([("CCORE_DOCUMENT_ENGINE".to_string(), "docling".to_string())]),
        };
        let doc = parse_extracted_doc(
            "/tmp/report.pdf",
            json!({
                "content": "Quarterly results",
                "title": "report.pdf",
                "source_type": "file",
                "identified_type": "application/pdf",
                "metadata": { "pages": 3 }
            }),
            &config,
        )
        .unwrap();
        assert_eq!(doc.text, "Quarterly results");
        assert_eq!(doc.detected_type.as_deref(), Some("application/pdf"));
        assert_eq!(doc.engine.as_deref(), Some("docling"));
        assert_eq!(doc.metadata["pages"], 3);
    }

    #[test]
    fn parse_extracted_doc_prefers_reported_engine() {
        let config = ContentCoreExtractionConfig {
            enabled: true,
            timeout: Duration::from_secs(1),
            commands: vec![],
            env: BTreeMap::from([("CCORE_URL_ENGINE".to_string(), "auto".to_string())]),
        };
        let doc = parse_extracted_doc(
            "https://example.com",
            json!({
                "content": "Article",
                "source_type": "url",
                "identified_type": "article",
                "metadata": { "engine": "firecrawl" }
            }),
            &config,
        )
        .unwrap();
        assert_eq!(doc.engine.as_deref(), Some("firecrawl"));
    }
}
