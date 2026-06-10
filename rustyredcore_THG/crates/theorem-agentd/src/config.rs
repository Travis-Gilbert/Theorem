use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{AgentdError, AgentdResult};

fn default_actor() -> String {
    "theorem-agentd".to_string()
}

fn default_room_id() -> String {
    "repo:theorem:branch:main".to_string()
}

fn default_model_provider() -> ModelProvider {
    ModelProvider::OpenAiCompatible
}

fn default_temperature() -> f32 {
    0.2
}

fn default_max_tokens() -> u32 {
    1200
}

fn default_timeout() -> u64 {
    120
}

fn default_true() -> bool {
    true
}

fn default_max_iterations() -> usize {
    8
}

fn default_tick_interval() -> u64 {
    60
}

fn default_storm_guard() -> u64 {
    600
}

fn default_ledger_path() -> PathBuf {
    PathBuf::from(".theorem/agentd-token-ledger.jsonl")
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub enum ModelProvider {
    #[serde(rename = "openai-compatible", alias = "open-ai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "rule")]
    Rule,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AgentdConfig {
    #[serde(default = "default_actor")]
    pub actor: String,
    #[serde(default = "default_room_id")]
    pub default_room_id: String,
    pub model: ModelConfig,
    pub harness: McpServerConfig,
    #[serde(default)]
    pub mcp_servers: Vec<McpServerConfig>,
    #[serde(default)]
    pub receiver: ReceiverSidecarConfig,
    #[serde(default)]
    pub loop_config: LoopConfig,
    #[serde(default)]
    pub ledger: LedgerConfig,
}

impl AgentdConfig {
    pub fn load(path: impl AsRef<Path>) -> AgentdResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|error| {
            AgentdError::Config(format!("cannot read {}: {error}", path.display()))
        })?;
        Self::from_toml(&raw)
    }

    pub fn from_toml(raw: &str) -> AgentdResult<Self> {
        let config: Self =
            toml::from_str(raw).map_err(|error| AgentdError::Config(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    pub fn all_mcp_servers(&self) -> Vec<McpServerConfig> {
        let mut servers = Vec::with_capacity(self.mcp_servers.len() + 1);
        servers.push(self.harness.clone());
        servers.extend(self.mcp_servers.clone());
        servers
    }

    fn validate(&self) -> AgentdResult<()> {
        if self.actor.trim().is_empty() {
            return Err(AgentdError::Config("actor is required".to_string()));
        }
        if self.default_room_id.trim().is_empty() {
            return Err(AgentdError::Config(
                "default_room_id is required".to_string(),
            ));
        }
        if self.model.provider == ModelProvider::OpenAiCompatible
            && self.model.base_url.trim().is_empty()
        {
            return Err(AgentdError::Config(
                "model.base_url is required for openai-compatible provider".to_string(),
            ));
        }
        self.harness.validate()?;
        for server in &self.mcp_servers {
            server.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelConfig {
    #[serde(default = "default_model_provider")]
    pub provider: ModelProvider,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_timeout")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_true")]
    pub grammar_constrained: bool,
}

#[derive(Clone, Debug, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default = "default_tenant")]
    pub tenant_slug: String,
    /// When true, this server speaks the MCP streamable-HTTP session protocol
    /// (initialize -> mcp-session-id header -> notifications/initialized ->
    /// tools/call carrying Mcp-Session-Id, responses as SSE). Spec-compliant
    /// servers like the TickTick MCP require it; the lenient/stateless harness
    /// does not, so it defaults off.
    #[serde(default)]
    pub session: bool,
}

impl McpServerConfig {
    fn validate(&self) -> AgentdResult<()> {
        if self.name.trim().is_empty() {
            return Err(AgentdError::Config(
                "mcp server name is required".to_string(),
            ));
        }
        if self.url.trim().is_empty() {
            return Err(AgentdError::Config(format!(
                "mcp server {} url is required",
                self.name
            )));
        }
        Ok(())
    }
}

fn default_tenant() -> String {
    "default".to_string()
}

#[derive(Clone, Debug, Deserialize)]
pub struct ReceiverSidecarConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_receiver_config_path")]
    pub config_path: PathBuf,
}

impl Default for ReceiverSidecarConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            config_path: default_receiver_config_path(),
        }
    }
}

fn default_receiver_config_path() -> PathBuf {
    PathBuf::from("theorem-receiver.toml")
}

#[derive(Clone, Debug, Deserialize)]
pub struct LoopConfig {
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
    #[serde(default = "default_tick_interval")]
    pub tick_interval_secs: u64,
    #[serde(default = "default_storm_guard")]
    pub storm_guard_window_secs: u64,
}

impl Default for LoopConfig {
    fn default() -> Self {
        Self {
            max_iterations: default_max_iterations(),
            tick_interval_secs: default_tick_interval(),
            storm_guard_window_secs: default_storm_guard(),
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LedgerConfig {
    #[serde(default = "default_ledger_path")]
    pub path: PathBuf,
}

impl Default for LedgerConfig {
    fn default() -> Self {
        Self {
            path: default_ledger_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rule_config_with_defaults() {
        let raw = r#"
actor = "agentd"

[model]
provider = "rule"

[harness]
name = "harness"
url = "https://example.test/mcp"
"#;
        let config = AgentdConfig::from_toml(raw).unwrap();
        assert_eq!(config.actor, "agentd");
        assert_eq!(config.default_room_id, "repo:theorem:branch:main");
        assert_eq!(config.model.provider, ModelProvider::Rule);
        assert_eq!(config.loop_config.max_iterations, 8);
        assert!(!config.receiver.enabled);
    }
}
