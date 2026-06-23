//! theorem-receiver standalone binary (Option B).
//!
//! Usage:
//!   THEOREM_HARNESS_TOKEN=<bearer> theorem-receiver [config.toml]
//!
//! Config path defaults to `theorem-receiver.toml` in the working directory.
//! The bearer token is read from the environment when present, never from disk.
//! Authless local/dev harnesses may run without it.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use theorem_receiver::{
    config::ReceiverConfig, wake_dry_run_report_json, wake_run_report_json, HarnessClient,
    WakeLedger, DEFAULT_WAKE_MAX_PLANS, DEFAULT_WAKE_MESSAGE_LIMIT,
};

const TOKEN_ENV: &str = "THEOREM_HARNESS_TOKEN";
const WAKE_LEDGER_ENV: &str = "THEOREM_WAKE_LEDGER";
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
    let command = ReceiverCommand::parse(std::env::args().skip(1).collect());
    let config_path = command.config_path();
    let config = ReceiverConfig::load(&config_path)?;

    let token = std::env::var(TOKEN_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());

    let client = HarnessClient::new(
        config.harness_url.clone(),
        token,
        config.tenant_slug.clone(),
    )?;

    match command {
        ReceiverCommand::RunLoop { .. } => theorem_receiver::run_loop(&config, &client),
        ReceiverCommand::WakeDryRun { room_id, actor, .. } => {
            let messages = client.read_messages_for_room(&room_id, DEFAULT_WAKE_MESSAGE_LIMIT)?;
            let report = theorem_receiver::build_wake_dry_run_report(
                &config,
                &actor,
                &messages,
                &WakeLedger::default(),
                DEFAULT_WAKE_MAX_PLANS,
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&wake_dry_run_report_json(&report))?
            );
            Ok(())
        }
        ReceiverCommand::WakeRun {
            room_id,
            actor,
            ledger_path,
            ..
        } => {
            let ledger_path = resolve_wake_ledger_path(&config_path, ledger_path);
            let mut ledger = WakeLedger::load_or_default(&ledger_path)?;
            let messages = client.read_messages_for_room(&room_id, DEFAULT_WAKE_MESSAGE_LIMIT)?;
            let report = theorem_receiver::run_wake_report_with_spawner(
                &config,
                &actor,
                &messages,
                &mut ledger,
                DEFAULT_WAKE_MAX_PLANS,
                |ledger| ledger.save(&ledger_path).map_err(|error| error.to_string()),
                theorem_receiver::spawn_wake_command,
            );
            println!(
                "{}",
                serde_json::to_string_pretty(&wake_run_report_json(&report))?
            );
            Ok(())
        }
    }
}

enum ReceiverCommand {
    RunLoop {
        config_path: String,
    },
    WakeDryRun {
        room_id: String,
        actor: String,
        config_path: String,
    },
    WakeRun {
        room_id: String,
        actor: String,
        config_path: String,
        ledger_path: Option<String>,
    },
}

impl ReceiverCommand {
    fn parse(args: Vec<String>) -> Self {
        if args.first().map(String::as_str) == Some("--wake-dry-run") {
            return Self::WakeDryRun {
                room_id: args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| "room:ungrouped".to_string()),
                actor: args.get(2).cloned().unwrap_or_else(|| "codex".to_string()),
                config_path: args
                    .get(3)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_CONFIG.to_string()),
            };
        }
        if args.first().map(String::as_str) == Some("--wake-run") {
            return Self::WakeRun {
                room_id: args
                    .get(1)
                    .cloned()
                    .unwrap_or_else(|| "room:ungrouped".to_string()),
                actor: args.get(2).cloned().unwrap_or_else(|| "codex".to_string()),
                config_path: args
                    .get(3)
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_CONFIG.to_string()),
                ledger_path: args.get(4).cloned(),
            };
        }
        Self::RunLoop {
            config_path: args
                .first()
                .cloned()
                .unwrap_or_else(|| DEFAULT_CONFIG.to_string()),
        }
    }

    fn config_path(&self) -> String {
        match self {
            Self::RunLoop { config_path }
            | Self::WakeDryRun { config_path, .. }
            | Self::WakeRun { config_path, .. } => config_path.clone(),
        }
    }
}

fn resolve_wake_ledger_path(config_path: &str, explicit_path: Option<String>) -> PathBuf {
    if let Some(path) = explicit_path {
        return PathBuf::from(path);
    }
    if let Ok(path) = std::env::var(WAKE_LEDGER_ENV) {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    let config_dir = Path::new(config_path)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    config_dir.join(".theorem").join("wake-ledger.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_default_run_loop_command() {
        let command = ReceiverCommand::parse(Vec::new());
        match command {
            ReceiverCommand::RunLoop { config_path } => {
                assert_eq!(config_path, DEFAULT_CONFIG);
            }
            ReceiverCommand::WakeDryRun { .. } | ReceiverCommand::WakeRun { .. } => {
                panic!("expected run loop")
            }
        }
    }

    #[test]
    fn parses_wake_dry_run_command() {
        let command = ReceiverCommand::parse(vec![
            "--wake-dry-run".to_string(),
            "repo:theorem:branch:main".to_string(),
            "codex".to_string(),
            "receiver.toml".to_string(),
        ]);
        match command {
            ReceiverCommand::WakeDryRun {
                room_id,
                actor,
                config_path,
            } => {
                assert_eq!(room_id, "repo:theorem:branch:main");
                assert_eq!(actor, "codex");
                assert_eq!(config_path, "receiver.toml");
            }
            ReceiverCommand::RunLoop { .. } | ReceiverCommand::WakeRun { .. } => {
                panic!("expected wake dry-run")
            }
        }
    }

    #[test]
    fn parses_wake_run_command() {
        let command = ReceiverCommand::parse(vec![
            "--wake-run".to_string(),
            "repo:theorem:branch:main".to_string(),
            "codex".to_string(),
            "receiver.toml".to_string(),
            "/tmp/wake-ledger.json".to_string(),
        ]);
        match command {
            ReceiverCommand::WakeRun {
                room_id,
                actor,
                config_path,
                ledger_path,
            } => {
                assert_eq!(room_id, "repo:theorem:branch:main");
                assert_eq!(actor, "codex");
                assert_eq!(config_path, "receiver.toml");
                assert_eq!(ledger_path, Some("/tmp/wake-ledger.json".to_string()));
            }
            ReceiverCommand::RunLoop { .. } | ReceiverCommand::WakeDryRun { .. } => {
                panic!("expected wake run")
            }
        }
    }

    #[test]
    fn default_wake_ledger_lives_next_to_config() {
        assert_eq!(
            resolve_wake_ledger_path("/repos/theorem/theorem-receiver.toml", None),
            PathBuf::from("/repos/theorem/.theorem/wake-ledger.json")
        );
        assert_eq!(
            resolve_wake_ledger_path("theorem-receiver.toml", None),
            PathBuf::from("./.theorem/wake-ledger.json")
        );
    }
}
