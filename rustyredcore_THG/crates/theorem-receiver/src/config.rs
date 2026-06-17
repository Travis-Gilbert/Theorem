//! Receiver configuration (TOML).
//!
//! The bearer token is NOT part of this file; it is read from the environment
//! (`THEOREM_HARNESS_TOKEN`) at startup so no credential is ever stored on disk.

use serde::Deserialize;
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
}
