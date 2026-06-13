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

fn default_operator_memory_tenant() -> String {
    "default".to_string()
}

fn default_capture_repo() -> String {
    "Travis-Gilbert/theorem".to_string()
}

fn default_dispatched_subtask() -> String {
    "dispatched".to_string()
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
    /// Tenant the operator's personal memory (recall/remember/encode) is routed
    /// to. Coordination rooms, jobs, and shared substrate stay on the harness
    /// server's own tenant; only personal-memory calls carry this override.
    /// Defaults to "default" so behavior is unchanged until an operator names a
    /// tenant. See `docs/plans/local-loop/` and CHK023-026.
    #[serde(default = "default_operator_memory_tenant")]
    pub operator_memory_tenant: String,
    /// Agent Queue capture: turning mobile-created TickTick tasks into jobs.
    #[serde(default)]
    pub capture: CaptureConfig,
    /// Milestone relay + completion back to the originating TickTick task.
    #[serde(default)]
    pub relay: RelayConfig,
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
    /// Mirror each ledger line into the graph as a receipt so the agentd corpus
    /// accumulates beside the Claude Code and Codex traces (the label factory,
    /// CHK027-029). The JSONL ledger is the training-data source of truth and is
    /// never rotated away; this is an additive best-effort mirror.
    #[serde(default = "default_true")]
    pub mirror_to_graph: bool,
}

impl Default for LedgerConfig {
    fn default() -> Self {
        Self {
            path: default_ledger_path(),
            mirror_to_graph: true,
        }
    }
}

/// Agent Queue capture (CHK004-008). A dedicated TickTick list is the only
/// capture trigger: each task there is converted to a job once per tick.
#[derive(Clone, Debug, Deserialize)]
pub struct CaptureConfig {
    /// Master switch. Off by default so a misconfigured daemon never sweeps an
    /// unintended list.
    #[serde(default)]
    pub enabled: bool,
    /// TickTick project (list) id of the Agent Queue. This is the ONLY list the
    /// capture step ever reads; tasks anywhere else are never converted (CHK007).
    #[serde(default)]
    pub agent_queue_project_id: Option<String>,
    /// Human name to resolve the Agent Queue id from when the id is not set
    /// directly. Resolved against ticktick_list_projects at capture time.
    #[serde(default)]
    pub agent_queue_project_name: Option<String>,
    /// Product list a task is moved into once it has been dispatched (CHK006).
    /// When unset, the task is stamped and its dispatched subtask is checked, but
    /// it is not moved (so capture still works without a configured destination).
    #[serde(default)]
    pub dispatched_project_id: Option<String>,
    /// Repo a captured job targets.
    #[serde(default = "default_capture_repo")]
    pub repo: String,
    /// Optional target-head hint for captured jobs ("claude" | "codex" | "either").
    #[serde(default)]
    pub target_head: Option<String>,
    /// Title of the checklist item checked off when a task is dispatched (CHK006).
    #[serde(default = "default_dispatched_subtask")]
    pub dispatched_subtask_title: String,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            agent_queue_project_id: None,
            agent_queue_project_name: None,
            dispatched_project_id: None,
            repo: default_capture_repo(),
            target_head: None,
            dispatched_subtask_title: default_dispatched_subtask(),
        }
    }
}

/// Milestone relay + completion (CHK009-015). Gemma composes the milestone prose;
/// the loop gates transitions, dedupes, posts, and completes-on-merge.
#[derive(Clone, Debug, Deserialize)]
pub struct RelayConfig {
    /// Master switch. Off by default.
    #[serde(default)]
    pub enabled: bool,
}

impl Default for RelayConfig {
    fn default() -> Self {
        Self { enabled: false }
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
        // Local-loop additions default to safe/off and unchanged behavior.
        assert_eq!(config.operator_memory_tenant, "default");
        assert!(!config.capture.enabled);
        assert!(!config.relay.enabled);
        assert_eq!(config.capture.repo, "Travis-Gilbert/theorem");
        assert_eq!(config.capture.dispatched_subtask_title, "dispatched");
        assert!(config.ledger.mirror_to_graph);
    }

    #[test]
    fn parses_capture_and_relay_config() {
        let raw = r#"
actor = "agentd"
operator_memory_tenant = "operator:travis"

[model]
provider = "rule"

[harness]
name = "harness"
url = "https://example.test/mcp"

[capture]
enabled = true
agent_queue_project_id = "6a2911688f08a8907c774531"
dispatched_project_id = "689cdbfd8f083a8c93d0134e"
repo = "Travis-Gilbert/theorem"

[relay]
enabled = true
"#;
        let config = AgentdConfig::from_toml(raw).unwrap();
        assert_eq!(config.operator_memory_tenant, "operator:travis");
        assert!(config.capture.enabled);
        assert_eq!(
            config.capture.agent_queue_project_id.as_deref(),
            Some("6a2911688f08a8907c774531")
        );
        assert_eq!(
            config.capture.dispatched_project_id.as_deref(),
            Some("689cdbfd8f083a8c93d0134e")
        );
        assert!(config.relay.enabled);
    }
}
