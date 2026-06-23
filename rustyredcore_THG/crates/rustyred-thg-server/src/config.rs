use std::{collections::BTreeMap, env, fs};

use crate::auth::ApiToken;
use rustyred_thg_core::{default_hybrid_edge_type_weights, HybridScoringConfig, RedCoreDurability};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StorageMode {
    Embedded,
    Memory,
    Redis,
}

impl StorageMode {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "redis" | "legacy_redis" => Self::Redis,
            "memory" | "ram" => Self::Memory,
            _ => Self::Embedded,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Memory => "memory",
            Self::Redis => "redis",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub storage_mode: StorageMode,
    pub data_dir: String,
    pub require_volume: bool,
    pub volume_available: bool,
    pub durability: RedCoreDurability,
    pub snapshot_interval_writes: u64,
    pub strict_acid: bool,
    pub concurrency: String,
    pub txn_isolation: String,
    pub tenant_memory_quota_bytes: usize,
    pub tenant_memory_quota_config_error: Option<String>,
    pub tenant_config_overrides: BTreeMap<String, TenantConfigOverride>,
    pub tenant_config_error: Option<String>,
    pub slow_query_threshold_nanos: u64,
    pub slow_query_capacity: usize,
    pub slow_query_log: Option<String>,
    pub hybrid_scoring: HybridScoringConfig,
    pub redis_url: String,
    pub redis_key_prefix: String,
    pub require_auth: bool,
    pub allowed_origins: Vec<String>,
    pub api_tokens: Vec<ApiToken>,
    pub service_name: String,
    pub api_title: String,
    pub public_url: Option<String>,
    pub mcp_enabled: bool,
    pub mcp_read_only: bool,
    pub mcp_allow_admin: bool,
    pub mcp_default_tenant: String,
    /// TTL background sweep interval in milliseconds. Default 1000ms.
    /// Env: `RUSTYRED_THG_TTL_SWEEP_MS`. Lower = tighter expiration
    /// precision but more CPU; higher = cheaper but expired nodes
    /// linger longer in storage (reads filter them either way).
    pub ttl_sweep_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct TenantConfigOverride {
    #[serde(default)]
    pub durability: Option<RedCoreDurability>,
    #[serde(default)]
    pub snapshot_interval_writes: Option<u64>,
    #[serde(default)]
    pub strict_acid: Option<bool>,
    #[serde(default)]
    pub tenant_memory_quota_bytes: Option<usize>,
    #[serde(default)]
    pub hybrid_scoring: Option<HybridScoringConfig>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveTenantConfig {
    pub durability: RedCoreDurability,
    pub snapshot_interval_writes: u64,
    pub strict_acid: bool,
    pub tenant_memory_quota_bytes: usize,
    pub hybrid_scoring: HybridScoringConfig,
}

impl Config {
    pub fn from_env() -> Self {
        let railway_port = env::var("PORT").ok();
        let host =
            env_first(&["RUSTY_RED_HOST", "RUSTYRED_THG_PRODUCT_HOST"]).unwrap_or_else(|_| {
                if railway_port.is_some() {
                    "0.0.0.0".to_string()
                } else {
                    "127.0.0.1".to_string()
                }
            });
        let port = railway_port
            .clone()
            .or_else(|| env_first(&["RUSTY_RED_PORT", "RUSTYRED_THG_PRODUCT_PORT"]).ok())
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(8380);
        let storage_mode = env_first(&["RUSTY_RED_MODE", "RUSTYRED_THG_PRODUCT_STORE"])
            .map(|value| StorageMode::parse(&value))
            .unwrap_or(StorageMode::Embedded);
        let railway_volume_mount_path = env::var("RAILWAY_VOLUME_MOUNT_PATH")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let data_dir = env_first(&["RUSTY_RED_DATA_DIR", "RUSTYRED_THG_PRODUCT_DATA_DIR"])
            .or_else(|_| {
                railway_volume_mount_path
                    .clone()
                    .ok_or(env::VarError::NotPresent)
            })
            .unwrap_or_else(|_| {
                if railway_port.is_some() {
                    "/app/data/rusty-red".to_string()
                } else {
                    "data/rusty-red".to_string()
                }
            });
        let require_volume = env_bool(
            &[
                "RUSTY_RED_REQUIRE_VOLUME",
                "RUSTYRED_THG_PRODUCT_REQUIRE_VOLUME",
            ],
            railway_port.is_some(),
        );
        let volume_available = railway_volume_mount_path.is_some()
            || env_bool(
                &[
                    "RUSTY_RED_VOLUME_MOUNTED",
                    "RUSTYRED_THG_PRODUCT_VOLUME_MOUNTED",
                ],
                false,
            );
        let durability = env_first(&["RUSTY_RED_DURABILITY", "RUSTYRED_THG_PRODUCT_DURABILITY"])
            .map(|value| RedCoreDurability::parse(&value))
            .unwrap_or(RedCoreDurability::AofEverysec);
        let snapshot_interval_writes = env_first(&[
            "RUSTY_RED_SNAPSHOT_INTERVAL_WRITES",
            "RUSTYRED_THG_PRODUCT_SNAPSHOT_INTERVAL_WRITES",
        ])
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1_000);
        let strict_acid = env_bool(
            &["RUSTY_RED_STRICT_ACID", "RUSTYRED_THG_PRODUCT_STRICT_ACID"],
            false,
        );
        let concurrency = env_first(&["RUSTY_RED_CONCURRENCY", "RUSTYRED_THG_PRODUCT_CONCURRENCY"])
            .unwrap_or_else(|_| "single_writer".to_string());
        let txn_isolation = env_first(&[
            "RUSTY_RED_TXN_ISOLATION",
            "RUSTYRED_THG_PRODUCT_TXN_ISOLATION",
        ])
        .unwrap_or_else(|_| {
            if strict_acid {
                "serializable".to_string()
            } else {
                "snapshot".to_string()
            }
        });
        let (tenant_memory_quota_bytes, tenant_memory_quota_config_error) = env_usize(
            &[
                "RUSTY_RED_TENANT_MEMORY_QUOTA_BYTES",
                "RUSTYRED_THG_PRODUCT_TENANT_MEMORY_QUOTA_BYTES",
            ],
            0,
        );
        let (slow_query_threshold_nanos, slow_query_threshold_error) = env_u64(
            &[
                "RUSTY_RED_SLOW_QUERY_NANOS",
                "RUSTYRED_THG_PRODUCT_SLOW_QUERY_NANOS",
            ],
            100_000_000,
        );
        let (slow_query_capacity, slow_query_capacity_error) = env_usize(
            &[
                "RUSTY_RED_SLOW_QUERY_CAPACITY",
                "RUSTYRED_THG_PRODUCT_SLOW_QUERY_CAPACITY",
            ],
            128,
        );
        let slow_query_log = env_first(&[
            "RUSTY_RED_SLOW_QUERY_LOG",
            "RUSTYRED_THG_PRODUCT_SLOW_QUERY_LOG",
        ])
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
        let mut tenant_config_error = slow_query_threshold_error.or(slow_query_capacity_error);
        let (tenant_config_overrides, parsed_tenant_config_error) = tenant_config_from_env();
        if tenant_config_error.is_none() {
            tenant_config_error = parsed_tenant_config_error;
        }
        let hybrid_scoring = HybridScoringConfig::default();
        let redis_url = env_first(&["RUSTY_RED_REDIS_URL", "RUSTYRED_THG_REDIS_URL"])
            .or_else(|_| env::var("REDIS_URL"))
            .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let redis_key_prefix =
            env_first(&["RUSTY_RED_KEY_PREFIX", "RUSTYRED_THG_REDIS_KEY_PREFIX"])
                .unwrap_or_else(|_| "theseus:thg:tenant".to_string());
        let require_auth = env_first(&["RUSTY_RED_REQUIRE_AUTH", "RUSTYRED_THG_REQUIRE_AUTH"])
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(true);
        let allowed_origins =
            env_first(&["RUSTY_RED_ALLOWED_ORIGINS", "RUSTYRED_THG_ALLOWED_ORIGINS"])
                .unwrap_or_else(|_| "http://localhost:3000".to_string())
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect();
        let api_tokens = env_first(&["RUSTY_RED_API_TOKENS", "RUSTYRED_THG_API_TOKENS"])
            .unwrap_or_default()
            .split(',')
            .filter_map(ApiToken::parse)
            .collect();
        let service_name = env_first(&["RUSTY_RED_SERVICE_NAME", "RUSTYRED_THG_SERVICE_NAME"])
            .unwrap_or_else(|_| "thg-product".to_string());
        let api_title = env_first(&["RUSTY_RED_API_TITLE", "RUSTYRED_THG_API_TITLE"])
            .unwrap_or_else(|_| "Theorem Context THG API".to_string());
        let public_url = env_first(&["RUSTY_RED_PUBLIC_URL", "RUSTYRED_THG_PUBLIC_URL"]).ok();
        let mcp_enabled = env_bool(&["RUSTY_RED_MCP_ENABLED", "RUSTYRED_THG_MCP_ENABLED"], true);
        let mcp_read_only = env_bool(
            &["RUSTY_RED_MCP_READ_ONLY", "RUSTYRED_THG_MCP_READ_ONLY"],
            false,
        );
        let mcp_allow_admin = env_bool(
            &["RUSTY_RED_MCP_ALLOW_ADMIN", "RUSTYRED_THG_MCP_ALLOW_ADMIN"],
            false,
        );
        let mcp_default_tenant = env_first(&[
            "RUSTY_RED_MCP_DEFAULT_TENANT",
            "RUSTYRED_THG_MCP_DEFAULT_TENANT",
        ])
        .unwrap_or_else(|_| "default".to_string());

        // TTL-04: background sweep interval. Default 1000ms balances
        // expiration precision against per-tick CPU.
        let ttl_sweep_ms = env_first(&["RUSTYRED_THG_TTL_SWEEP_MS", "RUSTY_RED_TTL_SWEEP_MS"])
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(1000);

        Self {
            host,
            port,
            storage_mode,
            data_dir,
            require_volume,
            volume_available,
            durability,
            snapshot_interval_writes,
            strict_acid,
            concurrency,
            txn_isolation,
            tenant_memory_quota_bytes,
            tenant_memory_quota_config_error,
            tenant_config_overrides,
            tenant_config_error,
            slow_query_threshold_nanos,
            slow_query_capacity,
            slow_query_log,
            hybrid_scoring,
            redis_url,
            redis_key_prefix,
            require_auth,
            allowed_origins,
            api_tokens,
            service_name,
            api_title,
            public_url,
            mcp_enabled,
            mcp_read_only,
            mcp_allow_admin,
            mcp_default_tenant,
            ttl_sweep_ms,
        }
    }

    /// Minimal Config for cross-module test usage. Mirrors
    /// `config::tests::base_config` but is `pub(crate)` so other
    /// modules' tests can use it. Uses Memory storage mode so tests
    /// don't need on-disk fixtures.
    #[cfg(test)]
    pub(crate) fn default_for_tests() -> Self {
        use std::collections::BTreeMap;
        Self {
            host: "127.0.0.1".to_string(),
            port: 0,
            storage_mode: StorageMode::Memory,
            data_dir: "data/test-rusty-red".to_string(),
            require_volume: false,
            volume_available: true,
            durability: RedCoreDurability::None,
            snapshot_interval_writes: 0,
            strict_acid: false,
            concurrency: "single_writer".to_string(),
            txn_isolation: "serializable".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: BTreeMap::new(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 16,
            slow_query_log: None,
            hybrid_scoring: HybridScoringConfig::default(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            redis_key_prefix: "test".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "test".to_string(),
            api_title: "Test".to_string(),
            public_url: None,
            mcp_enabled: false,
            mcp_read_only: false,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            ttl_sweep_ms: 50,
        }
    }

    pub fn bind_addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    pub fn validate(&self) -> Result<(), String> {
        if let Some(error) = &self.tenant_memory_quota_config_error {
            return Err(error.clone());
        }
        if let Some(error) = &self.tenant_config_error {
            return Err(error.clone());
        }
        if self.slow_query_capacity == 0 {
            return Err("RUSTY_RED_SLOW_QUERY_CAPACITY must be greater than 0".to_string());
        }
        if self.ttl_sweep_ms == 0 {
            return Err("RUSTYRED_THG_TTL_SWEEP_MS must be greater than 0".to_string());
        }
        if self.storage_mode == StorageMode::Embedded
            && self.require_volume
            && !self.volume_available
        {
            return Err(
                "RUSTY_RED_REQUIRE_VOLUME=true requires RAILWAY_VOLUME_MOUNT_PATH or RUSTY_RED_VOLUME_MOUNTED=true"
                    .to_string(),
                );
        }
        if self.storage_mode == StorageMode::Redis && self.tenant_memory_quota_bytes > 0 {
            return Err(
                "RUSTY_RED_TENANT_MEMORY_QUOTA_BYTES is currently enforced only for RUSTY_RED_MODE=embedded or RUSTY_RED_MODE=memory; redis mode quota enforcement is a separate follow-up gate"
                    .to_string(),
            );
        }
        if !self.strict_acid {
            return Ok(());
        }
        if self.storage_mode != StorageMode::Embedded {
            return Err(format!(
                "RUSTY_RED_STRICT_ACID=true requires RUSTY_RED_MODE=embedded, got {}",
                self.storage_mode.as_str()
            ));
        }
        if self.durability != RedCoreDurability::AofAlways {
            return Err(format!(
                "RUSTY_RED_STRICT_ACID=true requires RUSTY_RED_DURABILITY=aof_always, got {}",
                self.durability.as_str()
            ));
        }
        if self.concurrency.trim() != "single_writer" {
            return Err(format!(
                "RUSTY_RED_STRICT_ACID=true requires RUSTY_RED_CONCURRENCY=single_writer, got {}",
                self.concurrency
            ));
        }
        if self.txn_isolation.trim() != "serializable" {
            return Err(format!(
                "RUSTY_RED_STRICT_ACID=true requires RUSTY_RED_TXN_ISOLATION=serializable, got {}",
                self.txn_isolation
            ));
        }
        Ok(())
    }

    pub fn tenant_config(&self, tenant_id: &str) -> EffectiveTenantConfig {
        let key = rustyred_thg_core::sanitize_tenant_segment(tenant_id);
        let override_config = self
            .tenant_config_overrides
            .get(tenant_id)
            .or_else(|| self.tenant_config_overrides.get(&key));
        let mut hybrid_scoring = self.hybrid_scoring.clone();
        if hybrid_scoring.edge_type_weights.is_empty() {
            hybrid_scoring.edge_type_weights = default_hybrid_edge_type_weights();
        }
        EffectiveTenantConfig {
            durability: override_config
                .and_then(|config| config.durability)
                .unwrap_or(self.durability),
            snapshot_interval_writes: override_config
                .and_then(|config| config.snapshot_interval_writes)
                .unwrap_or(self.snapshot_interval_writes),
            strict_acid: override_config
                .and_then(|config| config.strict_acid)
                .unwrap_or(self.strict_acid),
            tenant_memory_quota_bytes: override_config
                .and_then(|config| config.tenant_memory_quota_bytes)
                .unwrap_or(self.tenant_memory_quota_bytes),
            hybrid_scoring: override_config
                .and_then(|config| config.hybrid_scoring.clone())
                .unwrap_or(hybrid_scoring),
        }
    }
}

fn env_bool(keys: &[&str], default: bool) -> bool {
    env_first(keys)
        .map(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(default)
}

fn env_usize(keys: &[&str], default: usize) -> (usize, Option<String>) {
    match env_first(keys) {
        Ok(value) => match value.parse::<usize>() {
            Ok(parsed) => (parsed, None),
            Err(_) => (
                default,
                Some(format!(
                    "{} must be an unsigned byte count, got {value}",
                    keys.join(" or "),
                )),
            ),
        },
        Err(_) => (default, None),
    }
}

fn env_u64(keys: &[&str], default: u64) -> (u64, Option<String>) {
    match env_first(keys) {
        Ok(value) => match value.parse::<u64>() {
            Ok(parsed) => (parsed, None),
            Err(_) => (
                default,
                Some(format!(
                    "{} must be an unsigned integer, got {value}",
                    keys.join(" or "),
                )),
            ),
        },
        Err(_) => (default, None),
    }
}

fn tenant_config_from_env() -> (BTreeMap<String, TenantConfigOverride>, Option<String>) {
    let mut merged = BTreeMap::new();
    if let Ok(path) = env_first(&[
        "RUSTY_RED_TENANT_CONFIG_PATH",
        "RUSTYRED_THG_PRODUCT_TENANT_CONFIG_PATH",
    ]) {
        match fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|raw| parse_tenant_config_json(&raw))
        {
            Ok(values) => merged.extend(values),
            Err(error) => {
                return (
                    merged,
                    Some(format!("RUSTY_RED_TENANT_CONFIG_PATH {path}: {error}")),
                )
            }
        }
    }
    if let Ok(raw) = env_first(&[
        "RUSTY_RED_TENANT_CONFIG_JSON",
        "RUSTYRED_THG_PRODUCT_TENANT_CONFIG_JSON",
    ]) {
        match parse_tenant_config_json(&raw) {
            Ok(values) => merged.extend(values),
            Err(error) => {
                return (
                    merged,
                    Some(format!("RUSTY_RED_TENANT_CONFIG_JSON: {error}")),
                )
            }
        }
    }
    (merged, None)
}

fn parse_tenant_config_json(raw: &str) -> Result<BTreeMap<String, TenantConfigOverride>, String> {
    serde_json::from_str::<BTreeMap<String, TenantConfigOverride>>(raw)
        .map_err(|error| error.to_string())
}

fn env_first(keys: &[&str]) -> Result<String, env::VarError> {
    for key in keys {
        match env::var(key) {
            Ok(value) if !value.trim().is_empty() => return Ok(value),
            Ok(_) => continue,
            Err(env::VarError::NotPresent) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(env::VarError::NotPresent)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{
        parse_tenant_config_json, Config, HybridScoringConfig, RedCoreDurability, StorageMode,
    };

    fn base_config() -> Config {
        Config {
            host: "127.0.0.1".to_string(),
            port: 8380,
            storage_mode: StorageMode::Embedded,
            data_dir: "data/rusty-red".to_string(),
            require_volume: false,
            volume_available: false,
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 1_000,
            strict_acid: true,
            concurrency: "single_writer".to_string(),
            txn_isolation: "serializable".to_string(),
            tenant_memory_quota_bytes: 0,
            tenant_memory_quota_config_error: None,
            tenant_config_overrides: BTreeMap::new(),
            tenant_config_error: None,
            slow_query_threshold_nanos: 100_000_000,
            slow_query_capacity: 128,
            slow_query_log: None,
            hybrid_scoring: HybridScoringConfig::default(),
            redis_url: "redis://127.0.0.1:6379".to_string(),
            redis_key_prefix: "rusty-red".to_string(),
            require_auth: false,
            allowed_origins: Vec::new(),
            api_tokens: Vec::new(),
            service_name: "rusty-red".to_string(),
            api_title: "Rusty Red".to_string(),
            public_url: None,
            mcp_enabled: true,
            mcp_read_only: false,
            mcp_allow_admin: false,
            mcp_default_tenant: "default".to_string(),
            ttl_sweep_ms: 1000,
        }
    }

    #[test]
    fn strict_acid_config_requires_aof_always() {
        let mut config = base_config();
        config.durability = RedCoreDurability::AofEverysec;

        assert!(config.validate().unwrap_err().contains("aof_always"));
    }

    #[test]
    fn strict_acid_config_accepts_single_writer_serializable_embedded() {
        assert_eq!(base_config().validate(), Ok(()));
    }

    #[test]
    fn embedded_config_rejects_missing_required_volume() {
        let mut config = base_config();
        config.require_volume = true;
        config.volume_available = false;

        assert!(config.validate().unwrap_err().contains("REQUIRE_VOLUME"));
    }

    #[test]
    fn embedded_config_accepts_required_available_volume() {
        let mut config = base_config();
        config.require_volume = true;
        config.volume_available = true;

        assert_eq!(config.validate(), Ok(()));
    }

    #[test]
    fn invalid_tenant_memory_quota_config_fails_validation() {
        let mut config = base_config();
        config.tenant_memory_quota_config_error =
            Some("RUSTY_RED_TENANT_MEMORY_QUOTA_BYTES must be an unsigned byte count".to_string());

        assert!(config
            .validate()
            .unwrap_err()
            .contains("unsigned byte count"));
    }

    #[test]
    fn ttl_sweep_interval_must_be_positive() {
        let mut config = base_config();
        config.ttl_sweep_ms = 0;

        assert!(config
            .validate()
            .unwrap_err()
            .contains("TTL_SWEEP_MS must be greater than 0"));
    }

    #[test]
    fn redis_mode_rejects_tenant_memory_quota_until_supported() {
        let mut config = base_config();
        config.storage_mode = StorageMode::Redis;
        config.tenant_memory_quota_bytes = 1024;

        assert!(config.validate().unwrap_err().contains("redis mode quota"));
    }

    #[test]
    fn tenant_config_json_overlays_per_tenant_runtime_values() {
        let mut config = base_config();
        config.tenant_config_overrides = parse_tenant_config_json(
            r#"{
                "tenant-a": {
                    "durability": "snapshot_only",
                    "snapshot_interval_writes": 12,
                    "strict_acid": false,
                    "tenant_memory_quota_bytes": 4096,
                    "hybrid_scoring": {
                        "alpha": 0.25,
                        "confidence_weighted_graph_distance": false,
                        "edge_type_weights": { "CONTRADICTS": -2.0 }
                    }
                }
            }"#,
        )
        .unwrap();

        let tenant = config.tenant_config("tenant-a");

        assert_eq!(tenant.durability, RedCoreDurability::SnapshotOnly);
        assert_eq!(tenant.snapshot_interval_writes, 12);
        assert!(!tenant.strict_acid);
        assert_eq!(tenant.tenant_memory_quota_bytes, 4096);
        assert_eq!(tenant.hybrid_scoring.alpha, 0.25);
        assert!(!tenant.hybrid_scoring.confidence_weighted_graph_distance);
        assert_eq!(tenant.hybrid_scoring.edge_type_weights["CONTRADICTS"], -2.0);
    }
}
