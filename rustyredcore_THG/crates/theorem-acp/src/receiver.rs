use std::path::{Path, PathBuf};
use std::thread::{self, JoinHandle};

use theorem_receiver::{HarnessClient, ReceiverConfig};

use crate::{AcpError, AcpResult};

#[derive(Clone, Debug)]
pub struct ReceiverSidecarConfig {
    pub config_path: PathBuf,
}

impl ReceiverSidecarConfig {
    pub fn new(config_path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: config_path.into(),
        }
    }
}

pub fn spawn_receiver_sidecar(config_path: &Path) -> AcpResult<JoinHandle<()>> {
    let config = ReceiverConfig::load(config_path)
        .map_err(|error| AcpError::Receiver(format!("receiver config: {error}")))?;
    let token = std::env::var("THEOREM_HARNESS_TOKEN").ok();
    let client = HarnessClient::new(
        config.harness_url.clone(),
        token,
        config.tenant_slug.clone(),
    )
    .map_err(|error| AcpError::Receiver(format!("receiver client: {error}")))?;
    let handle = thread::Builder::new()
        .name("theorem-acp-receiver-sidecar".to_string())
        .spawn(move || {
            if let Err(error) = theorem_receiver::run_loop(&config, &client) {
                eprintln!("[theorem-acp] receiver sidecar stopped: {error}");
            }
        })?;
    Ok(handle)
}
