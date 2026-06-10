use std::path::Path;
use std::thread::{self, JoinHandle};

use theorem_receiver::{HarnessClient, ReceiverConfig};

use crate::{AgentdError, AgentdResult};

pub fn spawn_receiver_sidecar(config_path: &Path) -> AgentdResult<JoinHandle<()>> {
    let config = ReceiverConfig::load(config_path)
        .map_err(|error| AgentdError::Config(format!("receiver config: {error}")))?;
    let token = std::env::var("THEOREM_HARNESS_TOKEN").ok();
    let client = HarnessClient::new(
        config.harness_url.clone(),
        token,
        config.tenant_slug.clone(),
    )
    .map_err(|error| AgentdError::Config(format!("receiver client: {error}")))?;
    let handle = thread::Builder::new()
        .name("theorem-receiver-sidecar".to_string())
        .spawn(move || {
            if let Err(error) = theorem_receiver::run_loop(&config, &client) {
                eprintln!("[theorem-agentd] receiver sidecar stopped: {error}");
            }
        })?;
    Ok(handle)
}
