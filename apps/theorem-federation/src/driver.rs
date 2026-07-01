use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::identity::{bind_endpoint, identity_status, IdentityConfig, IdentityStatus};
use crate::trust::DEFAULT_TRUST_FLOOR;
use crate::{FederationError, Result};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ConfiguredPeer {
    pub endpoint_id: String,
    pub relay_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct FederationConfig {
    pub federation_enabled: bool,
    pub tenant: String,
    pub data_dir: PathBuf,
    pub status_addr: SocketAddr,
    pub trust_floor: f64,
    pub peers: Vec<ConfiguredPeer>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerConnectionState {
    Configured,
    Connected,
    RelayRouted,
    Disconnected,
    RejectedByTrust,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PeerStatus {
    pub endpoint_id: String,
    pub relay_url: Option<String>,
    pub state: PeerConnectionState,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RelayStatus {
    pub mode: String,
    pub reachable: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FederationDoctor {
    pub enabled: bool,
    pub identity: IdentityStatus,
    pub endpoint_bound: bool,
    pub relay: RelayStatus,
    pub peers: Vec<PeerStatus>,
    pub warnings: Vec<String>,
}

impl FederationConfig {
    pub fn from_env() -> Self {
        let tenant = env::var("THEOREM_FEDERATION_TENANT")
            .or_else(|_| env::var("THEOREM_SYNC_TENANT"))
            .or_else(|_| env::var("THEOREM_PROXY_TENANT"))
            .unwrap_or_else(|_| "Travis-Gilbert".to_string());
        let data_dir = env::var("THEOREM_FEDERATION_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| default_data_dir());
        let status_port = env_u16("THEOREM_FEDERATION_STATUS_PORT", 8791);
        Self {
            federation_enabled: truthy_env("THEOREM_FEDERATION_ENABLED"),
            tenant,
            data_dir,
            status_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), status_port),
            trust_floor: env_f64("THEOREM_FEDERATION_TRUST_FLOOR", DEFAULT_TRUST_FLOOR),
            peers: parse_peers(env::var("THEOREM_FEDERATION_PEERS").ok().as_deref()),
        }
    }

    pub fn identity_config(&self) -> IdentityConfig {
        IdentityConfig::new(&self.data_dir)
    }
}

pub async fn doctor(config: &FederationConfig, bind: bool) -> Result<FederationDoctor> {
    let identity = identity_status(&config.identity_config())?;
    let mut warnings = Vec::new();
    let endpoint_bound = if bind {
        if !config.federation_enabled {
            return Err(FederationError::Config(
                "set THEOREM_FEDERATION_ENABLED=1 before binding the federation endpoint"
                    .to_string(),
            ));
        }
        let endpoint = bind_endpoint(&config.identity_config()).await?;
        endpoint.close().await;
        true
    } else {
        warnings.push(
            "endpoint bind skipped; pass --bind-endpoint for a live Iroh socket probe".to_string(),
        );
        false
    };

    if !config.federation_enabled {
        warnings.push(
            "federation is disabled; set THEOREM_FEDERATION_ENABLED=1 to run the driver"
                .to_string(),
        );
    }
    if config.peers.is_empty() {
        warnings.push("no peers configured in THEOREM_FEDERATION_PEERS".to_string());
    }

    Ok(FederationDoctor {
        enabled: config.federation_enabled,
        identity,
        endpoint_bound,
        relay: RelayStatus {
            mode: "n0_public_relays".to_string(),
            reachable: if endpoint_bound { Some(true) } else { None },
        },
        peers: config
            .peers
            .iter()
            .map(|peer| PeerStatus {
                endpoint_id: peer.endpoint_id.clone(),
                relay_url: peer.relay_url.clone(),
                state: PeerConnectionState::Configured,
            })
            .collect(),
        warnings,
    })
}

pub fn parse_peers(value: Option<&str>) -> Vec<ConfiguredPeer> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|raw| {
            let raw = raw.trim();
            if raw.is_empty() {
                return None;
            }
            let (endpoint_id, relay_url) = raw
                .split_once('@')
                .map(|(id, relay)| (id.trim(), Some(relay.trim().to_string())))
                .unwrap_or((raw, None));
            Some(ConfiguredPeer {
                endpoint_id: endpoint_id.to_string(),
                relay_url,
            })
        })
        .collect()
}

pub fn truthy_env(name: &str) -> bool {
    env::var(name)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn env_u16(name: &str, default: u16) -> u16 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(default)
}

fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .unwrap_or(default)
}

fn default_data_dir() -> PathBuf {
    env::var_os("THEOREM_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".theorem")))
        .unwrap_or_else(|| PathBuf::from(".theorem"))
}
