//! theorem-proxy CLI entrypoint.
//!
//! `theorem-proxy proxy` starts the local Anthropic Messages proxy on localhost and
//! prints the one line that connects Claude Code to it (SPEC-ONECLICK deliverable 4:
//! the first-run message is the next action, not a log dump).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use theorem_proxy::memory::{DirectoryMemorySource, HttpMemorySource, MemorySource};
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
        /// Live local Theorem node MCP endpoint (e.g. http://127.0.0.1:8790/mcp). When
        /// set, ambient memory is the node's relevance-ranked graph memory
        /// (`hippo_retrieve`). Takes precedence over --memory-dir.
        #[arg(long, env = "THEOREM_PROXY_MEMORY_URL")]
        memory_url: Option<String>,
        /// Tenant slug for the node memory query (optional; node default if omitted).
        #[arg(long, env = "THEOREM_PROXY_TENANT")]
        tenant: Option<String>,
        /// Directory of `*.md` memories to inject ambiently. Fallback when no node URL
        /// is set; omit both for faithful passthrough.
        #[arg(long, env = "THEOREM_PROXY_MEMORY_DIR")]
        memory_dir: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    match Cli::parse().command {
        Command::Proxy {
            port,
            upstream,
            memory_url,
            tenant,
            memory_dir,
        } => {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            // The live node memory (--memory-url) wins; the directory is the no-node
            // fallback; neither set is faithful passthrough.
            let (memory, memory_desc): (Option<Arc<dyn MemorySource>>, String) =
                if let Some(url) = &memory_url {
                    (
                        Some(Arc::new(HttpMemorySource::new(url.clone(), tenant.clone()))),
                        format!("live local node memory at {url}"),
                    )
                } else if let Some(dir) = &memory_dir {
                    (
                        Some(Arc::new(DirectoryMemorySource::new(dir))),
                        format!("relevant memory from {}", dir.display()),
                    )
                } else {
                    (None, "off (faithful passthrough)".to_string())
                };
            println!("theorem proxy live at http://{addr}");
            println!();
            println!("point Claude Code at it:");
            println!("    export ANTHROPIC_BASE_URL=http://{addr}");
            println!();
            println!("ambient memory: {memory_desc}");
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
