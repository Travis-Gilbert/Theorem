//! theorem-receiver standalone binary (Option B).
//!
//! Usage:
//!   THEOREM_HARNESS_TOKEN=<bearer> theorem-receiver [config.toml]
//!
//! Config path defaults to `theorem-receiver.toml` in the working directory.
//! The bearer token is read from the environment when present, never from disk.
//! Authless local/dev harnesses may run without it.

use std::process::ExitCode;

use theorem_receiver::{config::ReceiverConfig, HarnessClient};

const TOKEN_ENV: &str = "THEOREM_HARNESS_TOKEN";
const DEFAULT_CONFIG: &str = "theorem-receiver.toml";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("[theorem-receiver] fatal: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> theorem_receiver::ReceiverResult<()> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_CONFIG.to_string());
    let config = ReceiverConfig::load(&config_path)?;

    let token = std::env::var(TOKEN_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());

    let client = HarnessClient::new(
        config.harness_url.clone(),
        token,
        config.tenant_slug.clone(),
    )?;

    theorem_receiver::run_loop(&config, &client)
}
