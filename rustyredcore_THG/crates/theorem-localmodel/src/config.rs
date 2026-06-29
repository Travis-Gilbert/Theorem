use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{LocalModelError, LocalModelResult};

pub const DEFAULT_HARNESS_MCP_URL: &str = "http://127.0.0.1:8380/mcp";
pub const DEFAULT_TENANT_SLUG: &str = "Travis-Gilbert";

fn default_actor() -> String {
    "theorem-localmodel".to_string()
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
    PathBuf::from(".theorem/localmodel-token-ledger.jsonl")
}

fn default_operator_memory_tenant() -> String {
    "default".to_string()
}

fn default_local_operator_memory_tenant() -> String {
    DEFAULT_TENANT_SLUG.to_string()
}

fn default_capture_repo() -> String {
    "Travis-Gilbert/theorem".to_string()
}

fn default_dispatched_subtask() -> String {
    "dispatched".to_string()
}

fn default_local_model_host() -> String {
    "127.0.0.1".to_string()
}

fn default_local_model_port() -> u16 {
    8080
}

fn default_local_model_api_model_id() -> String {
    "gemma-4-12b-it-qat".to_string()
}

fn default_gemma_12b_tok_model_id() -> Option<String> {
    Some("google/gemma-4-12B-it".to_string())
}

fn default_local_gemma_12b_qat_dir() -> String {
    "apps/theorem-localmodel".to_string()
}

fn default_local_gemma_12b_qat_filename() -> String {
    "gemma-4-12B-it-qat-UD-Q4_K_XL.gguf".to_string()
}

fn default_q4_k_xl_quantization() -> String {
    "Q4_K_XL".to_string()
}

fn default_cache_token_source() -> String {
    "cache".to_string()
}

fn default_max_seqs() -> usize {
    16
}

fn default_max_seq_len() -> usize {
    4096
}

fn default_max_batch_size() -> usize {
    1
}

fn default_q4_quantization() -> String {
    "q4_0".to_string()
}

fn default_gemma_12b_resident_memory_gb() -> f32 {
    9.0
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
pub enum ModelProvider {
    #[serde(rename = "openai-compatible", alias = "open-ai-compatible")]
    OpenAiCompatible,
    #[serde(rename = "rule")]
    Rule,
}

#[derive(Clone, Debug, Deserialize)]
pub struct LocalModelConfig {
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
    pub local_model: LocalModelHostConfig,
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

impl LocalModelConfig {
    pub fn load(path: impl AsRef<Path>) -> LocalModelResult<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|error| {
            LocalModelError::Config(format!("cannot read {}: {error}", path.display()))
        })?;
        Self::from_toml(&raw)
    }

    pub fn from_toml(raw: &str) -> LocalModelResult<Self> {
        let config: Self =
            toml::from_str(raw).map_err(|error| LocalModelError::Config(error.to_string()))?;
        config.validate()?;
        Ok(config)
    }

    /// Load the named config when it exists; otherwise use the no-config local
    /// default. This is the happy path for `theorem start` and first-run
    /// `theorem-localmodel --once ...`: no token, tenant, data-dir, or model env var
    /// is required just to prove the loop is alive.
    pub fn load_or_default(path: impl AsRef<Path>) -> LocalModelResult<Self> {
        let path = path.as_ref();
        if path.exists() {
            return Self::load(path);
        }
        let config = Self::default_local();
        config.validate()?;
        Ok(config)
    }

    pub fn default_local() -> Self {
        Self {
            actor: default_actor(),
            default_room_id: default_room_id(),
            model: ModelConfig::rule_default(),
            harness: McpServerConfig::default_harness(),
            mcp_servers: Vec::new(),
            receiver: ReceiverSidecarConfig::default(),
            local_model: LocalModelHostConfig::default(),
            loop_config: LoopConfig::default(),
            ledger: LedgerConfig::default(),
            operator_memory_tenant: default_local_operator_memory_tenant(),
            capture: CaptureConfig::default(),
            relay: RelayConfig::default(),
        }
    }

    pub fn all_mcp_servers(&self) -> Vec<McpServerConfig> {
        let mut servers = Vec::with_capacity(self.mcp_servers.len() + 1);
        servers.push(self.harness.clone());
        servers.extend(self.mcp_servers.clone());
        servers
    }

    fn validate(&self) -> LocalModelResult<()> {
        if self.actor.trim().is_empty() {
            return Err(LocalModelError::Config("actor is required".to_string()));
        }
        if self.default_room_id.trim().is_empty() {
            return Err(LocalModelError::Config(
                "default_room_id is required".to_string(),
            ));
        }
        if self.model.provider == ModelProvider::OpenAiCompatible
            && self.model.base_url.trim().is_empty()
        {
            return Err(LocalModelError::Config(
                "model.base_url is required for openai-compatible provider".to_string(),
            ));
        }
        self.harness.validate()?;
        for server in &self.mcp_servers {
            server.validate()?;
        }
        self.local_model.validate()?;
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LocalModelHostConfig {
    #[serde(default = "default_local_model_host")]
    pub host: String,
    #[serde(default = "default_local_model_port")]
    pub port: u16,
    #[serde(default = "default_local_model_api_model_id")]
    pub api_model_id: String,
    #[serde(default = "default_gemma_12b_tok_model_id")]
    pub tok_model_id: Option<String>,
    #[serde(default = "default_local_gemma_12b_qat_dir")]
    pub quantized_model_id: String,
    #[serde(default = "default_local_gemma_12b_qat_filename")]
    pub quantized_filename: String,
    #[serde(default = "default_q4_k_xl_quantization")]
    pub quantization: String,
    #[serde(default)]
    pub chat_template: Option<String>,
    #[serde(default)]
    pub jinja_explicit: Option<String>,
    #[serde(default = "default_cache_token_source")]
    pub token_source: String,
    #[serde(default)]
    pub cpu: bool,
    #[serde(default)]
    pub paged_attention: Option<bool>,
    #[serde(default)]
    pub paged_context_len: Option<usize>,
    #[serde(default)]
    pub paged_attention_gpu_mem_mb: Option<usize>,
    #[serde(default)]
    pub paged_attention_gpu_mem_usage: Option<f32>,
    #[serde(default)]
    pub paged_attention_block_size: Option<usize>,
    #[serde(default = "default_max_seqs")]
    pub max_seqs: usize,
    #[serde(default = "default_max_seq_len")]
    pub max_seq_len: usize,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
    #[serde(default)]
    pub in_situ_quant: Option<String>,
    #[serde(default)]
    pub num_device_layers: Option<Vec<String>>,
    #[serde(default)]
    pub tiers: Vec<LocalModelTierConfig>,
    #[serde(default)]
    pub extra_models: Vec<LocalModelTierConfig>,
    #[serde(default)]
    pub drafter: Option<LocalModelDrafterConfig>,
    #[serde(default = "default_gemma_12b_resident_memory_gb")]
    pub resident_memory_estimate_gb: f32,
}

impl Default for LocalModelHostConfig {
    fn default() -> Self {
        Self {
            host: default_local_model_host(),
            port: default_local_model_port(),
            api_model_id: default_local_model_api_model_id(),
            tok_model_id: default_gemma_12b_tok_model_id(),
            quantized_model_id: default_local_gemma_12b_qat_dir(),
            quantized_filename: default_local_gemma_12b_qat_filename(),
            quantization: default_q4_k_xl_quantization(),
            chat_template: None,
            jinja_explicit: None,
            token_source: default_cache_token_source(),
            cpu: false,
            paged_attention: Some(true),
            paged_context_len: None,
            paged_attention_gpu_mem_mb: None,
            paged_attention_gpu_mem_usage: None,
            paged_attention_block_size: None,
            max_seqs: default_max_seqs(),
            max_seq_len: default_max_seq_len(),
            max_batch_size: default_max_batch_size(),
            in_situ_quant: None,
            num_device_layers: None,
            tiers: LocalModelTierConfig::default_tiers(),
            extra_models: Vec::new(),
            drafter: None,
            resident_memory_estimate_gb: default_gemma_12b_resident_memory_gb(),
        }
    }
}

impl LocalModelHostConfig {
    fn validate(&self) -> LocalModelResult<()> {
        if self.host.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model.host is required".to_string(),
            ));
        }
        if self.api_model_id.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model.api_model_id is required".to_string(),
            ));
        }
        if self.quantized_model_id.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model.quantized_model_id is required".to_string(),
            ));
        }
        if self.quantized_filename.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model.quantized_filename is required".to_string(),
            ));
        }
        self.token_source
            .parse::<mistralrs_core::TokenSource>()
            .map_err(|error| {
                LocalModelError::Config(format!("local_model.token_source is invalid: {error}"))
            })?;
        if self.max_seqs == 0 {
            return Err(LocalModelError::Config(
                "local_model.max_seqs must be greater than zero".to_string(),
            ));
        }
        if self.max_seq_len == 0 {
            return Err(LocalModelError::Config(
                "local_model.max_seq_len must be greater than zero".to_string(),
            ));
        }
        if self.max_batch_size == 0 {
            return Err(LocalModelError::Config(
                "local_model.max_batch_size must be greater than zero".to_string(),
            ));
        }
        for tier in self.tiers.iter().chain(self.extra_models.iter()) {
            tier.validate()?;
        }
        if let Some(drafter) = &self.drafter {
            drafter.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LocalModelTierConfig {
    pub model_id: String,
    #[serde(default)]
    pub alias: Option<String>,
    #[serde(default)]
    pub tok_model_id: Option<String>,
    pub quantized_model_id: String,
    pub quantized_filename: String,
    #[serde(default)]
    pub chat_template: Option<String>,
    #[serde(default)]
    pub jinja_explicit: Option<String>,
    #[serde(default)]
    pub in_situ_quant: Option<String>,
    #[serde(default = "default_q4_quantization")]
    pub quantization: String,
    #[serde(default = "default_max_seq_len")]
    pub max_seq_len: usize,
    #[serde(default = "default_max_batch_size")]
    pub max_batch_size: usize,
    #[serde(default)]
    pub resident_memory_estimate_gb: Option<f32>,
}

impl LocalModelTierConfig {
    fn default_tiers() -> Vec<Self> {
        vec![
            Self {
                model_id: "gemma-4-26b-a4b-qat-q4_0".to_string(),
                alias: Some("gemma-4-26b-a4b".to_string()),
                tok_model_id: Some("google/gemma-4-26B-A4B-it".to_string()),
                quantized_model_id: "google/gemma-4-26B-A4B-it-qat-q4_0-gguf".to_string(),
                quantized_filename: "gemma-4-26B_q4_0-it.gguf".to_string(),
                chat_template: None,
                jinja_explicit: None,
                in_situ_quant: None,
                quantization: default_q4_quantization(),
                max_seq_len: default_max_seq_len(),
                max_batch_size: default_max_batch_size(),
                resident_memory_estimate_gb: Some(18.0),
            },
            Self {
                model_id: "gemma-4-31b-dense-qat-q4_0".to_string(),
                alias: Some("gemma-4-31b-dense".to_string()),
                tok_model_id: Some("google/gemma-4-31B-it".to_string()),
                quantized_model_id: "google/gemma-4-31B-it-qat-q4_0-gguf".to_string(),
                quantized_filename: "gemma-4-31B_q4_0-it.gguf".to_string(),
                chat_template: None,
                jinja_explicit: None,
                in_situ_quant: None,
                quantization: default_q4_quantization(),
                max_seq_len: default_max_seq_len(),
                max_batch_size: default_max_batch_size(),
                resident_memory_estimate_gb: Some(24.0),
            },
        ]
    }

    fn validate(&self) -> LocalModelResult<()> {
        if self.model_id.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model tier model_id is required".to_string(),
            ));
        }
        if self.quantized_model_id.trim().is_empty() {
            return Err(LocalModelError::Config(format!(
                "local_model tier {} quantized_model_id is required",
                self.model_id
            )));
        }
        if self.quantized_filename.trim().is_empty() {
            return Err(LocalModelError::Config(format!(
                "local_model tier {} quantized_filename is required",
                self.model_id
            )));
        }
        if self.max_seq_len == 0 {
            return Err(LocalModelError::Config(format!(
                "local_model tier {} max_seq_len must be greater than zero",
                self.model_id
            )));
        }
        if self.max_batch_size == 0 {
            return Err(LocalModelError::Config(format!(
                "local_model tier {} max_batch_size must be greater than zero",
                self.model_id
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct LocalModelDrafterConfig {
    pub model: String,
    #[serde(default)]
    pub n_predict: Option<usize>,
}

impl LocalModelDrafterConfig {
    fn validate(&self) -> LocalModelResult<()> {
        if self.model.trim().is_empty() {
            return Err(LocalModelError::Config(
                "local_model.drafter.model is required".to_string(),
            ));
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

impl ModelConfig {
    pub fn rule_default() -> Self {
        Self {
            provider: ModelProvider::Rule,
            base_url: String::new(),
            model: String::new(),
            api_key_env: None,
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
            request_timeout_secs: default_timeout(),
            grammar_constrained: true,
        }
    }
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
    pub fn default_harness() -> Self {
        Self {
            name: "harness".to_string(),
            url: DEFAULT_HARNESS_MCP_URL.to_string(),
            token_env: None,
            tenant_slug: DEFAULT_TENANT_SLUG.to_string(),
            session: false,
        }
    }

    fn validate(&self) -> LocalModelResult<()> {
        if self.name.trim().is_empty() {
            return Err(LocalModelError::Config(
                "mcp server name is required".to_string(),
            ));
        }
        if self.url.trim().is_empty() {
            return Err(LocalModelError::Config(format!(
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
    /// Mirror each ledger line into the graph as a receipt so the localmodel corpus
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
#[derive(Clone, Debug, Default, Deserialize)]
pub struct RelayConfig {
    /// Master switch. Off by default.
    #[serde(default)]
    pub enabled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rule_config_with_defaults() {
        let raw = r#"
actor = "localmodel"

[model]
provider = "rule"

[harness]
name = "harness"
url = "https://example.test/mcp"
"#;
        let config = LocalModelConfig::from_toml(raw).unwrap();
        assert_eq!(config.actor, "localmodel");
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
        assert_eq!(
            config.local_model.quantized_model_id,
            "apps/theorem-localmodel"
        );
        assert_eq!(
            config.local_model.quantized_filename,
            "gemma-4-12B-it-qat-UD-Q4_K_XL.gguf"
        );
        assert_eq!(config.local_model.quantization, "Q4_K_XL");
        assert_eq!(config.local_model.port, 8080);
    }

    #[test]
    fn missing_config_uses_no_config_local_defaults() {
        let path = std::env::temp_dir().join(format!(
            "theorem-localmodel-missing-{}.toml",
            std::process::id()
        ));
        let config = LocalModelConfig::load_or_default(&path).unwrap();
        assert_eq!(config.actor, "theorem-localmodel");
        assert_eq!(config.model.provider, ModelProvider::Rule);
        assert_eq!(config.harness.name, "harness");
        assert_eq!(config.harness.url, DEFAULT_HARNESS_MCP_URL);
        assert_eq!(config.harness.tenant_slug, DEFAULT_TENANT_SLUG);
        assert_eq!(config.operator_memory_tenant, DEFAULT_TENANT_SLUG);
        assert!(!config.receiver.enabled);
        assert!(!config.capture.enabled);
        assert!(!config.relay.enabled);
    }

    #[test]
    fn rejects_zero_tier_dimensions() {
        let raw = r#"
actor = "localmodel"

[model]
provider = "rule"

[harness]
name = "harness"
url = "https://example.test/mcp"

[[local_model.tiers]]
model_id = "tier-a"
quantized_model_id = "repo/model"
quantized_filename = "model.gguf"
max_seq_len = 0
"#;
        let error = LocalModelConfig::from_toml(raw).unwrap_err().to_string();
        assert!(error.contains("max_seq_len must be greater than zero"));
    }

    #[test]
    fn parses_capture_and_relay_config() {
        let raw = r#"
actor = "localmodel"
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
        let config = LocalModelConfig::from_toml(raw).unwrap();
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
