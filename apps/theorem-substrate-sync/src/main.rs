use std::sync::Arc;
use std::time::Instant;

use clap::{Parser, Subcommand};
use theorem_substrate_sync::bootstrap::bootstrap_from_remote;
use theorem_substrate_sync::config::SyncConfig;
use theorem_substrate_sync::cursor::{CursorStore, ValkeyCursorStore};
use theorem_substrate_sync::drainer::{retry_after, OutboxDrainer};
use theorem_substrate_sync::outbox::{OutboxStore, ValkeyOutbox};
use theorem_substrate_sync::railway_client::{McpClient, TenantToken};
use theorem_substrate_sync::round::run_round;
use theorem_substrate_sync::scheduler::RoundScheduler;
use theorem_substrate_sync::status::{serve_status, ConnectionState, StatusHandle, SyncStatus};
use theorem_substrate_sync::subscriber::{read_and_apply_once, subscribe};
use theorem_substrate_sync::{Result, SyncError};
use tokio::sync::mpsc;

#[derive(Parser)]
#[command(
    name = "theorem-substrate-sync",
    version,
    about = "Local-to-hosted Theorem substrate sync daemon"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the status endpoint and, when enabled, the sync loops.
    Serve,
    /// Probe remote auth/connectivity and print a compact status.
    Doctor,
    /// Run one Prolly convergence round.
    Once,
    /// Bootstrap local state from the hosted head.
    Bootstrap,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = SyncConfig::from_env();
    let token = TenantToken::load(&config)?;
    let local = McpClient::unauthenticated(config.local_mcp_url.clone(), config.tenant.clone());
    let remote = McpClient::new(
        config.remote_mcp_url.clone(),
        config.tenant.clone(),
        token.clone(),
    );
    let status = StatusHandle::new(SyncStatus::new(
        config.sync_enabled,
        config.tenant.clone(),
        config.idle_interval.as_millis() as u64,
    ));

    match cli.command.unwrap_or(Command::Serve) {
        Command::Doctor => {
            let connection = remote.doctor().await;
            println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "connected": connection.as_connected_bool(),
                    "connection": connection,
                    "sync_enabled": config.sync_enabled,
                    "tenant": config.tenant,
                }))?
            );
            Ok(())
        }
        Command::Once => {
            ensure_enabled(&config)?;
            let receipt = run_round(&local, &remote, &status).await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        Command::Bootstrap => {
            ensure_enabled(&config)?;
            let receipt = bootstrap_from_remote(&local, &remote, &status).await?;
            println!("{}", serde_json::to_string_pretty(&receipt)?);
            Ok(())
        }
        Command::Serve => serve(config, local, remote, status).await,
    }
}

async fn serve(
    config: SyncConfig,
    local: McpClient,
    remote: McpClient,
    status: StatusHandle,
) -> Result<()> {
    let (trigger_tx, mut trigger_rx) = mpsc::unbounded_channel();
    let status_addr = config.status_addr;
    let status_server = tokio::spawn(serve_status(status_addr, status.clone(), trigger_tx));

    if !config.sync_enabled {
        println!("theorem-substrate-sync status at http://{status_addr}/status (sync disabled)");
        tokio::signal::ctrl_c().await?;
        status_server.abort();
        return Ok(());
    }

    let connection = remote.doctor().await;
    status
        .update(|status| status.set_connection(connection.clone()))
        .await;
    if matches!(connection, ConnectionState::TokenInvalid) {
        return Err(SyncError::Auth("remote token invalid".to_string()));
    }

    let outbox: Arc<dyn OutboxStore> = Arc::new(ValkeyOutbox::new(&config.valkey_url)?);
    let cursors: Arc<dyn CursorStore> = Arc::new(ValkeyCursorStore::new(&config.valkey_url)?);
    let drainer = OutboxDrainer::new(
        config.tenant.clone(),
        remote.clone(),
        Arc::clone(&outbox),
        status.clone(),
    );
    let subscriber_task = spawn_subscriber(
        config.tenant.clone(),
        local.clone(),
        remote.clone(),
        Arc::clone(&cursors),
        status.clone(),
    );
    let mut scheduler = RoundScheduler::new(config.idle_interval, config.active_interval);
    let mut failures = 0u32;
    println!("theorem-substrate-sync status at http://{status_addr}/status");

    loop {
        let interval = scheduler.current_interval(Instant::now());
        status
            .update(|status| status.current_interval_ms = interval.as_millis() as u64)
            .await;
        tokio::select! {
            _ = tokio::time::sleep(interval) => {
                if let Err(error) = run_round(&local, &remote, &status).await {
                    eprintln!("sync round failed: {error}");
                }
            }
            Some(()) = trigger_rx.recv() => {
                scheduler.note_activity(Instant::now());
                if let Err(error) = run_round(&local, &remote, &status).await {
                    eprintln!("manual sync round failed: {error}");
                }
            }
            _ = tokio::signal::ctrl_c() => {
                break;
            }
        }

        match drainer.drain_once().await {
            Ok(Some(_)) => failures = 0,
            Ok(None) => {}
            Err(error) => {
                failures = failures.saturating_add(1);
                let retry = retry_after(failures);
                status
                    .update(|status| {
                        status.stream_retry_after_ms = Some(retry.as_millis() as u64);
                    })
                    .await;
                eprintln!("outbox drain failed: {error}");
            }
        }

        let mut saved = cursors.load(&config.tenant)?;
        saved.last_round = Some(format!("{:?}", std::time::SystemTime::now()));
        cursors.save(&config.tenant, &saved)?;
    }

    subscriber_task.abort();
    status_server.abort();
    Ok(())
}

fn spawn_subscriber(
    tenant: String,
    local: McpClient,
    remote: McpClient,
    cursors: Arc<dyn CursorStore>,
    status: StatusHandle,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(error) = subscribe(&remote, &tenant).await {
            eprintln!("stream subscribe failed: {error}");
        }

        let mut failures = 0u32;
        loop {
            match read_and_apply_once(&local, &remote, cursors.as_ref(), &tenant, &status).await {
                Ok(_) => failures = 0,
                Err(error) => {
                    failures = failures.saturating_add(1);
                    eprintln!("stream read/apply failed: {error}");
                }
            }
            tokio::time::sleep(retry_after(failures)).await;
        }
    })
}

fn ensure_enabled(config: &SyncConfig) -> Result<()> {
    if config.sync_enabled {
        Ok(())
    } else {
        Err(SyncError::Config(
            "sync is disabled; set THEOREM_SYNC_ENABLED=1".to_string(),
        ))
    }
}
