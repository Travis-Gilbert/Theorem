//! Receiver configuration (TOML).
//!
//! The bearer token is NOT part of this file; it is read from the environment
//! (`THEOREM_HARNESS_TOKEN`) at startup so no credential is ever stored on disk.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::{ReceiverError, ReceiverResult};

/// Default claim poll interval. SSE wake on the jobs channel is a named
/// follow-up (gated on the tenant-scoped push fix); until it lands, polling is
/// the mechanism.
pub const DEFAULT_CLAIM_INTERVAL_SECS: u64 = 5;
/// Default per-repo capacity (concurrent jobs).
pub const DEFAULT_CAPACITY: u32 = 1;
/// Default environment variable containing the Postgres dispatch database URL.
pub const DEFAULT_DISPATCH_DATABASE_URL_ENV: &str = "THEOREM_DISPATCH_DATABASE_URL";
/// Default Postgres claim lease.
pub const DEFAULT_DISPATCH_LEASE_SECS: u64 = 600;
/// Default lease heartbeat cadence.
pub const DEFAULT_DISPATCH_HEARTBEAT_SECS: u64 = 60;
/// Default expired-lease reaper cadence.
pub const DEFAULT_DISPATCH_REAP_INTERVAL_SECS: u64 = 30;
pub const DEFAULT_OPENSANDBOX_EXECD_PORT: u16 = 44_772;
pub const DEFAULT_OPENSANDBOX_TIMEOUT_SECS: u64 = 3_600;
pub const DEFAULT_OPENSANDBOX_IMAGE: &str = "ubuntu:22.04";
pub const DEFAULT_OPENSANDBOX_WORKTREE_ROOT: &str = "/workspace";

fn default_tenant() -> String {
    "default".to_string()
}

fn default_interval() -> u64 {
    DEFAULT_CLAIM_INTERVAL_SECS
}

fn default_capacity() -> u32 {
    DEFAULT_CAPACITY
}

fn default_dispatch_database_url_env() -> String {
    DEFAULT_DISPATCH_DATABASE_URL_ENV.to_string()
}

fn default_dispatch_lease_secs() -> u64 {
    DEFAULT_DISPATCH_LEASE_SECS
}

fn default_dispatch_heartbeat_secs() -> u64 {
    DEFAULT_DISPATCH_HEARTBEAT_SECS
}

fn default_dispatch_reap_interval_secs() -> u64 {
    DEFAULT_DISPATCH_REAP_INTERVAL_SECS
}

fn default_opensandbox_execd_port() -> u16 {
    DEFAULT_OPENSANDBOX_EXECD_PORT
}

fn default_opensandbox_timeout_secs() -> u64 {
    DEFAULT_OPENSANDBOX_TIMEOUT_SECS
}

fn default_opensandbox_image() -> String {
    DEFAULT_OPENSANDBOX_IMAGE.to_string()
}

fn default_opensandbox_worktree_root() -> String {
    DEFAULT_OPENSANDBOX_WORKTREE_ROOT.to_string()
}

fn default_openai_chat_path() -> String {
    "/v1/chat/completions".to_string()
}

fn default_openai_responses_path() -> String {
    "/v1/responses".to_string()
}

fn default_anthropic_messages_path() -> String {
    "/v1/messages".to_string()
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ProviderSeamConfig {
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_openai_chat_path")]
    pub chat_path: String,
    #[serde(default = "default_openai_responses_path")]
    pub responses_path: String,
    #[serde(default = "default_anthropic_messages_path")]
    pub messages_path: String,
    #[serde(default)]
    pub models: BTreeMap<String, String>,
}

impl Default for ProviderSeamConfig {
    fn default() -> Self {
        Self {
            base_url: None,
            api_key_env: None,
            chat_path: default_openai_chat_path(),
            responses_path: default_openai_responses_path(),
            messages_path: default_anthropic_messages_path(),
            models: BTreeMap::new(),
        }
    }
}

impl ProviderSeamConfig {
    pub fn endpoint_for_wire_mode(&self, wire_mode: ProviderWireMode) -> Option<String> {
        let base_url = self.base_url.as_deref()?.trim().trim_end_matches('/');
        if base_url.is_empty() {
            return None;
        }
        let path = match wire_mode {
            ProviderWireMode::ChatCompletions => &self.chat_path,
            ProviderWireMode::Responses => &self.responses_path,
            ProviderWireMode::Messages => &self.messages_path,
        };
        Some(format!(
            "{base_url}/{}",
            path.trim().trim_start_matches('/')
        ))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelBackendKind {
    #[default]
    Single,
    Composed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderWireMode {
    #[default]
    ChatCompletions,
    Responses,
    Messages,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ModelBackendConfig {
    #[serde(default)]
    pub kind: ModelBackendKind,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub credential_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub wire_mode: ProviderWireMode,
    #[serde(default)]
    pub composed_binding_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct HeadRuntimeRecipe {
    pub runtime_binary: String,
    pub model_backend: String,
    #[serde(default)]
    pub wire_mode: ProviderWireMode,
    #[serde(default)]
    pub sandbox: bool,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SandboxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_opensandbox_image")]
    pub image: String,
    #[serde(default = "default_opensandbox_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_opensandbox_execd_port")]
    pub execd_port: u16,
    #[serde(default = "default_opensandbox_worktree_root")]
    pub worktree_root: String,
    #[serde(default)]
    pub secure_runtime: Option<String>,
    #[serde(default)]
    pub egress_allowlist: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// The receiver's static configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct ReceiverConfig {
    /// The cloud harness MCP endpoint, e.g.
    /// `https://rustyredcore-theorem-production.up.railway.app/mcp`.
    pub harness_url: String,
    #[serde(default = "default_tenant")]
    pub tenant_slug: String,
    /// Stable receiver id; defaults to a hostname-derived value.
    #[serde(default)]
    pub receiver_id: Option<String>,
    #[serde(default = "default_interval")]
    pub claim_interval_secs: u64,
    /// Per-repo capacity. The default-1 loop runs one job to completion before
    /// claiming the next; values > 1 are accepted but currently processed
    /// sequentially (parallel dispatch is a named follow-up).
    #[serde(default = "default_capacity")]
    pub capacity: u32,
    /// Environment variable holding the Postgres queue URL. Leave empty to keep
    /// the legacy THG-board polling loop.
    #[serde(default = "default_dispatch_database_url_env")]
    pub dispatch_database_url_env: String,
    #[serde(default = "default_dispatch_lease_secs")]
    pub dispatch_lease_secs: u64,
    #[serde(default = "default_dispatch_heartbeat_secs")]
    pub dispatch_heartbeat_secs: u64,
    #[serde(default = "default_dispatch_reap_interval_secs")]
    pub dispatch_reap_interval_secs: u64,
    /// Map of repo (`Travis-Gilbert/theorem`) to local worktree path. A job for
    /// an unmapped repo is never claimed (security fence).
    pub worktrees: BTreeMap<String, PathBuf>,
    /// Optional LiteLLM/OpenAI-compatible provider seam used by sandboxed
    /// coding runtimes and hosted composed-agent heads.
    #[serde(default)]
    pub provider_seam: ProviderSeamConfig,
    /// Named model backends available to runtime recipes.
    #[serde(default)]
    pub model_backends: BTreeMap<String, ModelBackendConfig>,
    /// Per-head coding runtime recipes. These are data-only until a job opts in
    /// to sandbox execution.
    #[serde(default)]
    pub head_runtime_recipes: BTreeMap<String, HeadRuntimeRecipe>,
    /// Optional OpenSandbox execution substrate. Local execution remains the
    /// default unless this is present and a recipe/job opts in.
    #[serde(default)]
    pub sandbox: Option<SandboxConfig>,
}

impl ReceiverConfig {
    /// Load and parse a receiver config from a TOML file.
    pub fn load(path: impl AsRef<Path>) -> ReceiverResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|error| {
            ReceiverError::Config(format!("cannot read {}: {error}", path.display()))
        })?;
        Self::from_toml(&raw)
    }

    /// Parse a receiver config from a TOML string.
    pub fn from_toml(raw: &str) -> ReceiverResult<Self> {
        let config: ReceiverConfig =
            toml::from_str(raw).map_err(|error| ReceiverError::Config(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> ReceiverResult<()> {
        if self.harness_url.trim().is_empty() {
            return Err(ReceiverError::Config("harness_url is required".to_string()));
        }
        if self.worktrees.is_empty() {
            return Err(ReceiverError::Config(
                "at least one repo -> worktree mapping is required".to_string(),
            ));
        }
        if self.dispatch_lease_secs == 0 {
            return Err(ReceiverError::Config(
                "dispatch_lease_secs must be positive".to_string(),
            ));
        }
        if self.dispatch_heartbeat_secs == 0 {
            return Err(ReceiverError::Config(
                "dispatch_heartbeat_secs must be positive".to_string(),
            ));
        }
        if self.dispatch_reap_interval_secs == 0 {
            return Err(ReceiverError::Config(
                "dispatch_reap_interval_secs must be positive".to_string(),
            ));
        }
        if self.dispatch_heartbeat_secs >= self.dispatch_lease_secs {
            return Err(ReceiverError::Config(
                "dispatch_heartbeat_secs must be shorter than dispatch_lease_secs".to_string(),
            ));
        }
        for (head, recipe) in &self.head_runtime_recipes {
            if recipe.runtime_binary.trim().is_empty() {
                return Err(ReceiverError::Config(format!(
                    "head_runtime_recipes.{head}.runtime_binary is required"
                )));
            }
            if !self.model_backends.contains_key(&recipe.model_backend) {
                return Err(ReceiverError::Config(format!(
                    "head_runtime_recipes.{head}.model_backend references unknown backend {}",
                    recipe.model_backend
                )));
            }
        }
        if let Some(sandbox) = &self.sandbox {
            if sandbox.enabled
                && sandbox
                    .base_url
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or_default()
                    .is_empty()
            {
                return Err(ReceiverError::Config(
                    "sandbox.base_url is required when sandbox.enabled = true".to_string(),
                ));
            }
            if sandbox.timeout_secs == 0 {
                return Err(ReceiverError::Config(
                    "sandbox.timeout_secs must be positive".to_string(),
                ));
            }
            if sandbox.execd_port == 0 {
                return Err(ReceiverError::Config(
                    "sandbox.execd_port must be positive".to_string(),
                ));
            }
            if sandbox.worktree_root.trim().is_empty() {
                return Err(ReceiverError::Config(
                    "sandbox.worktree_root is required".to_string(),
                ));
            }
        }
        Ok(())
    }

    /// The repos this receiver is configured to execute.
    pub fn repos(&self) -> Vec<String> {
        self.worktrees.keys().cloned().collect()
    }

    /// The local worktree mapped to a repo, if any.
    pub fn worktree_for(&self, repo: &str) -> Option<&Path> {
        self.worktrees.get(repo).map(PathBuf::as_path)
    }

    /// Resolve the receiver id (config value, else hostname-derived, else a
    /// process-derived fallback).
    pub fn resolved_receiver_id(&self) -> String {
        if let Some(id) = &self.receiver_id {
            if !id.trim().is_empty() {
                return id.clone();
            }
        }
        match std::env::var("HOSTNAME").or_else(|_| std::env::var("HOST")) {
            Ok(host) if !host.trim().is_empty() => format!("receiver-{host}"),
            _ => format!("receiver-{}", std::process::id()),
        }
    }

    /// The claim interval as a `Duration`.
    pub fn claim_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.claim_interval_secs)
    }

    /// Configured dispatch database URL, resolved from the named environment variable.
    pub fn dispatch_database_url(&self) -> Option<String> {
        let env_name = self.dispatch_database_url_env.trim();
        if env_name.is_empty() {
            return None;
        }
        std::env::var(env_name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    pub fn dispatch_lease(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.dispatch_lease_secs)
    }

    pub fn dispatch_heartbeat_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.dispatch_heartbeat_secs)
    }

    pub fn dispatch_reap_interval(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.dispatch_reap_interval_secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_minimal_config_with_defaults() {
        let raw = r#"
harness_url = "https://rustyredcore-theorem-production.up.railway.app/mcp"

[worktrees]
"Travis-Gilbert/theorem" = "/Users/travis/Theorem"
"#;
        let config = ReceiverConfig::from_toml(raw).unwrap();
        assert_eq!(config.tenant_slug, "default");
        assert_eq!(config.claim_interval_secs, DEFAULT_CLAIM_INTERVAL_SECS);
        assert_eq!(config.capacity, DEFAULT_CAPACITY);
        assert_eq!(
            config.dispatch_database_url_env,
            DEFAULT_DISPATCH_DATABASE_URL_ENV
        );
        assert_eq!(config.dispatch_lease_secs, DEFAULT_DISPATCH_LEASE_SECS);
        assert_eq!(
            config.dispatch_heartbeat_secs,
            DEFAULT_DISPATCH_HEARTBEAT_SECS
        );
        assert_eq!(
            config.dispatch_reap_interval_secs,
            DEFAULT_DISPATCH_REAP_INTERVAL_SECS
        );
        assert_eq!(config.provider_seam.chat_path, "/v1/chat/completions");
        assert!(config.model_backends.is_empty());
        assert!(config.head_runtime_recipes.is_empty());
        assert!(config.sandbox.is_none());
        assert_eq!(config.repos(), vec!["Travis-Gilbert/theorem".to_string()]);
        assert_eq!(
            config.worktree_for("Travis-Gilbert/theorem"),
            Some(Path::new("/Users/travis/Theorem"))
        );
        assert!(config.worktree_for("other/repo").is_none());
    }

    #[test]
    fn rejects_config_without_worktrees() {
        let raw = r#"harness_url = "https://example/mcp""#;
        assert!(ReceiverConfig::from_toml(raw).is_err());
    }

    #[test]
    fn honors_explicit_overrides() {
        let raw = r#"
harness_url = "https://example/mcp"
tenant_slug = "acme"
receiver_id = "laptop-a"
claim_interval_secs = 5
capacity = 2
dispatch_database_url_env = "CUSTOM_DISPATCH_DATABASE_URL"
dispatch_lease_secs = 120
dispatch_heartbeat_secs = 20
dispatch_reap_interval_secs = 10

[worktrees]
"acme/app" = "/repos/app"
"#;
        let config = ReceiverConfig::from_toml(raw).unwrap();
        assert_eq!(config.tenant_slug, "acme");
        assert_eq!(config.resolved_receiver_id(), "laptop-a");
        assert_eq!(config.claim_interval_secs, 5);
        assert_eq!(config.capacity, 2);
        assert_eq!(
            config.dispatch_database_url_env,
            "CUSTOM_DISPATCH_DATABASE_URL"
        );
        assert_eq!(config.dispatch_lease_secs, 120);
        assert_eq!(config.dispatch_heartbeat_secs, 20);
        assert_eq!(config.dispatch_reap_interval_secs, 10);
    }

    #[test]
    fn rejects_heartbeat_that_cannot_renew_before_expiry() {
        let raw = r#"
harness_url = "https://example/mcp"
dispatch_lease_secs = 10
dispatch_heartbeat_secs = 10

[worktrees]
"acme/app" = "/repos/app"
"#;
        let error = ReceiverConfig::from_toml(raw).unwrap_err().to_string();
        assert!(error.contains("heartbeat"));
    }

    #[test]
    fn parses_sandbox_runtime_recipe_and_provider_seam() {
        let raw = r#"
harness_url = "https://example/mcp"

[provider_seam]
base_url = "http://litellm.internal:4000"
api_key_env = "LITELLM_API_KEY"

[provider_seam.models]
deepseek = "deepseek-v4-pro"

[model_backends.codex_single]
kind = "single"
provider = "openai"
model = "gpt-4.1-mini"
credential_env = "OPENAI_API_KEY"
wire_mode = "responses"

[head_runtime_recipes.codex]
runtime_binary = "codex"
model_backend = "codex_single"
wire_mode = "responses"
sandbox = true

[head_runtime_recipes.codex.env]
OPENAI_API_BASE = "http://litellm.internal:4000/v1"

[sandbox]
enabled = true
base_url = "http://localhost:8080/v1"
api_key_env = "OPEN_SANDBOX_API_KEY"
image = "ghcr.io/example/theorem-codex:latest"
worktree_root = "/workspace/theorem"
egress_allowlist = ["litellm.internal", "github.com"]

[worktrees]
"acme/app" = "/repos/app"
"#;
        let config = ReceiverConfig::from_toml(raw).unwrap();

        assert_eq!(
            config
                .provider_seam
                .endpoint_for_wire_mode(ProviderWireMode::Responses)
                .as_deref(),
            Some("http://litellm.internal:4000/v1/responses")
        );
        assert_eq!(
            config.model_backends["codex_single"].wire_mode,
            ProviderWireMode::Responses
        );
        assert!(config.head_runtime_recipes["codex"].sandbox);
        let sandbox = config.sandbox.unwrap();
        assert!(sandbox.enabled);
        assert_eq!(sandbox.execd_port, DEFAULT_OPENSANDBOX_EXECD_PORT);
        assert_eq!(
            sandbox.egress_allowlist,
            vec!["litellm.internal".to_string(), "github.com".to_string()]
        );
    }

    #[test]
    fn rejects_recipe_with_unknown_model_backend() {
        let raw = r#"
harness_url = "https://example/mcp"

[head_runtime_recipes.codex]
runtime_binary = "codex"
model_backend = "missing"

[worktrees]
"acme/app" = "/repos/app"
"#;
        let error = ReceiverConfig::from_toml(raw).unwrap_err().to_string();
        assert!(error.contains("unknown backend"));
    }
}
