use std::path::PathBuf;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use clap::{Parser, Subcommand};
use theorem_federation::driver::{doctor, FederationConfig};
use theorem_federation::identity::{identity_status, IdentityConfig};
use theorem_federation::Result;

#[derive(Parser)]
#[command(
    name = "theorem-federation",
    version,
    about = "Opt-in direct peer federation lane for Theorem nodes"
)]
struct Cli {
    #[arg(long, env = "THEOREM_FEDERATION_DATA_DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, env = "THEOREM_FEDERATION_TENANT")]
    tenant: Option<String>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Print stable Iroh endpoint identity without binding a socket.
    Identity,
    /// Print federation status without a live socket probe.
    Status,
    /// Probe federation configuration. Use --bind-endpoint for a live Iroh bind.
    Doctor {
        #[arg(long)]
        bind_endpoint: bool,
    },
    /// Serve a local status endpoint. The federation lane remains default-off.
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = config_from_cli(&cli);
    match cli.command.unwrap_or(Command::Status) {
        Command::Identity => {
            let status = identity_status(&IdentityConfig::new(config.data_dir))?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::Status => {
            let status = doctor(&config, false).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::Doctor { bind_endpoint } => {
            let status = doctor(&config, bind_endpoint).await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
            Ok(())
        }
        Command::Serve => serve(config).await,
    }
}

fn config_from_cli(cli: &Cli) -> FederationConfig {
    let mut config = FederationConfig::from_env();
    if let Some(data_dir) = cli.data_dir.clone() {
        config.data_dir = data_dir;
    }
    if let Some(tenant) = cli.tenant.clone() {
        config.tenant = tenant;
    }
    config
}

async fn serve(config: FederationConfig) -> Result<()> {
    let addr = config.status_addr;
    let app = Router::new()
        .route("/status", get(status_route))
        .with_state(config);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn status_route(State(config): State<FederationConfig>) -> Json<serde_json::Value> {
    let value = match doctor(&config, false).await {
        Ok(status) => serde_json::to_value(status)
            .unwrap_or_else(|_| serde_json::json!({"ok": false, "error": "status_encode_failed"})),
        Err(error) => serde_json::json!({"ok": false, "error": error.to_string()}),
    };
    Json(value)
}
