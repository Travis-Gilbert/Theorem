//! theorem-proxy CLI entrypoint.
//!
//! `theorem-proxy proxy` starts the local Anthropic Messages proxy on localhost and
//! prints the one line that connects Claude Code to it (SPEC-ONECLICK deliverable 4:
//! the first-run message is the next action, not a log dump).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use theorem_proxy::memory::{DirectoryMemorySource, MemorySource};
use theorem_proxy::{serve, ProxyConfig};

#[derive(Parser)]
#[command(
    name = "theorem-proxy",
    about = "Local Theorem node: a faithful Anthropic Messages proxy on the model path"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Start the local proxy on localhost and forward Anthropic Messages to upstream.
    Proxy {
        #[arg(long, default_value_t = 8788)]
        port: u16,
        #[arg(long, default_value = "https://api.anthropic.com")]
        upstream: String,
        /// Directory of `*.md` memories to inject ambiently (D3). Omit for faithful
        /// passthrough.
        #[arg(long)]
        memory_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    match Cli::parse().command {
        Command::Proxy {
            port,
            upstream,
            memory_dir,
        } => {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let memory = memory_dir
                .as_ref()
                .map(|dir| Arc::new(DirectoryMemorySource::new(dir)) as Arc<dyn MemorySource>);
            println!("theorem proxy live at http://{addr}");
            println!();
            println!("point Claude Code at it:");
            println!("    export ANTHROPIC_BASE_URL=http://{addr}");
            println!();
            match &memory_dir {
                Some(dir) => println!("ambient memory: injecting relevant memory from {}", dir.display()),
                None => println!("ambient memory: off (faithful passthrough)"),
            }
            println!("forwarding to {upstream} (CPU-only, no model download)");
            serve(
                addr,
                ProxyConfig {
                    upstream,
                    memory,
                    max_memories: 8,
                },
            )
            .await
        }
    }
}
