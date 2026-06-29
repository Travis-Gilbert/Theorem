use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SyncConfig {
    pub sync_enabled: bool,
    pub tenant: String,
    pub remote_mcp_url: String,
    pub local_mcp_url: String,
    pub token_path: PathBuf,
    pub token_env: Option<String>,
    pub status_addr: SocketAddr,
    pub valkey_url: String,
    pub idle_interval: Duration,
    pub active_interval: Duration,
}

impl SyncConfig {
    pub fn from_env() -> Self {
        let status_port = env_u16("THEOREM_SYNC_STATUS_PORT", 8790);
        Self {
            sync_enabled: truthy_env("THEOREM_SYNC_ENABLED"),
            tenant: env::var("THEOREM_SYNC_TENANT")
                .or_else(|_| env::var("THEOREM_PROXY_TENANT"))
                .unwrap_or_else(|_| "Travis-Gilbert".to_string()),
            remote_mcp_url: env::var("THEOREM_SYNC_REMOTE_URL").unwrap_or_else(|_| {
                "https://rustyredcore-theorem-production.up.railway.app/mcp".to_string()
            }),
            local_mcp_url: env::var("THEOREM_SYNC_LOCAL_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:8380/mcp".to_string()),
            token_path: env::var("THEOREM_SYNC_TENANT_TOKEN_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_token_path()),
            token_env: env::var("THEOREM_SYNC_TENANT_TOKEN")
                .ok()
                .filter(|token| !token.trim().is_empty()),
            status_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), status_port),
            valkey_url: env::var("THEOREM_SYNC_VALKEY_URL")
                .unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string()),
            idle_interval: Duration::from_secs(env_u64("THEOREM_SYNC_IDLE_SECS", 30)),
            active_interval: Duration::from_secs(env_u64("THEOREM_SYNC_ACTIVE_SECS", 5)),
        }
    }
}

pub fn truthy_env(name: &str) -> bool {
    std::env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_u16(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(default)
}

fn default_token_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".theorem-substrate-sync").join("tenant-token")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_env_accepts_expected_values() {
        std::env::set_var("THEOREM_SYNC_TEST_TRUTHY", "yes");
        assert!(truthy_env("THEOREM_SYNC_TEST_TRUTHY"));
        std::env::set_var("THEOREM_SYNC_TEST_TRUTHY", "0");
        assert!(!truthy_env("THEOREM_SYNC_TEST_TRUTHY"));
        std::env::remove_var("THEOREM_SYNC_TEST_TRUTHY");
    }
}
