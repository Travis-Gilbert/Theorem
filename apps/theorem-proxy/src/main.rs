//! theorem-proxy CLI entrypoint.
//!
//! `theorem-proxy proxy` starts the local Anthropic Messages proxy on localhost and
//! prints the one line that connects Claude Code to it (SPEC-ONECLICK deliverable 4:
//! the first-run message is the next action, not a log dump).

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use theorem_proxy::{resolve_memory, run_wrapped, serve, ProxyConfig, UpstreamAuth};

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
        /// OpenAI/Codex upstream base URL for /v1/responses.
        #[arg(
            long,
            env = "THEOREM_PROXY_OPENAI_UPSTREAM",
            default_value = "https://api.openai.com"
        )]
        openai_upstream: String,
        /// Live local Theorem node MCP endpoint (e.g. http://127.0.0.1:8380/mcp). When
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
        /// D2 membrane: max inline tool_result bytes before the latest turn's oversized
        /// results are sampled (full output served at /tool_result/{id}). 0 = off.
        #[arg(long, default_value_t = 0)]
        membrane_threshold: usize,
        /// Upstream Anthropic API key. When set, client auth is stripped and this key
        /// is sent upstream; useful for Claude Desktop gateway mode.
        #[arg(long, env = "THEOREM_PROXY_UPSTREAM_API_KEY", hide_env_values = true)]
        upstream_api_key: Option<String>,
        /// Upstream bearer/OAuth token. Mutually exclusive with --upstream-api-key.
        #[arg(
            long,
            env = "THEOREM_PROXY_UPSTREAM_AUTH_TOKEN",
            hide_env_values = true
        )]
        upstream_auth_token: Option<String>,
        /// Force an Anthropic beta header upstream (optional; comma-separated value).
        #[arg(long, env = "THEOREM_PROXY_UPSTREAM_BETA")]
        upstream_beta: Option<String>,
        /// Upstream OpenAI API key. When set, client auth is stripped and this key
        /// is sent upstream as a bearer token; useful for sidecar/local-key modes.
        #[arg(
            long,
            env = "THEOREM_PROXY_OPENAI_UPSTREAM_API_KEY",
            hide_env_values = true
        )]
        openai_upstream_api_key: Option<String>,
    },
    /// Start the proxy and run a command (e.g. `claude`) pointed at it -- one command,
    /// no manual ANTHROPIC_BASE_URL export. Put the command after `--`.
    Wrap {
        #[arg(long, default_value_t = 8788)]
        port: u16,
        #[arg(long, default_value = "https://api.anthropic.com")]
        upstream: String,
        #[arg(
            long,
            env = "THEOREM_PROXY_OPENAI_UPSTREAM",
            default_value = "https://api.openai.com"
        )]
        openai_upstream: String,
        #[arg(long, env = "THEOREM_PROXY_MEMORY_URL")]
        memory_url: Option<String>,
        #[arg(long, env = "THEOREM_PROXY_TENANT")]
        tenant: Option<String>,
        #[arg(long, env = "THEOREM_PROXY_MEMORY_DIR")]
        memory_dir: Option<PathBuf>,
        /// D2 membrane threshold in bytes (0 = off). See `proxy --help`.
        #[arg(long, default_value_t = 0)]
        membrane_threshold: usize,
        #[arg(long, env = "THEOREM_PROXY_UPSTREAM_API_KEY", hide_env_values = true)]
        upstream_api_key: Option<String>,
        #[arg(
            long,
            env = "THEOREM_PROXY_UPSTREAM_AUTH_TOKEN",
            hide_env_values = true
        )]
        upstream_auth_token: Option<String>,
        #[arg(long, env = "THEOREM_PROXY_UPSTREAM_BETA")]
        upstream_beta: Option<String>,
        #[arg(
            long,
            env = "THEOREM_PROXY_OPENAI_UPSTREAM_API_KEY",
            hide_env_values = true
        )]
        openai_upstream_api_key: Option<String>,
        /// The command to run with ANTHROPIC_BASE_URL set (everything after `--`).
        #[arg(trailing_var_arg = true, required = true)]
        command: Vec<String>,
    },
    /// Check the local stack chain (Valkey, node, proxy) and print a readout.
    Doctor {
        #[arg(long, env = "THEOREM_PROXY_MEMORY_URL")]
        memory_url: Option<String>,
        /// Proxy base URL to check (defaults to $ANTHROPIC_BASE_URL if set).
        #[arg(long, env = "ANTHROPIC_BASE_URL")]
        proxy_url: Option<String>,
        /// Valkey warm-tier `host:port` to check.
        #[arg(long)]
        valkey_addr: Option<String>,
    },
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    match Cli::parse().command {
        Command::Proxy {
            port,
            upstream,
            openai_upstream,
            memory_url,
            tenant,
            memory_dir,
            membrane_threshold,
            upstream_api_key,
            upstream_auth_token,
            upstream_beta,
            openai_upstream_api_key,
        } => {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let (memory, memory_desc) =
                resolve_memory(memory_url.as_deref(), tenant, memory_dir.as_deref());
            let upstream_auth = upstream_auth(upstream_api_key, upstream_auth_token)?;
            println!("theorem-proxy live at http://{addr}");
            println!();
            println!("point Claude Code at it:");
            println!("    export ANTHROPIC_BASE_URL=http://{addr}");
            println!("point Codex at it:");
            println!("    codex -c 'openai_base_url=\"http://{addr}/v1\"'");
            println!();
            println!("ambient memory: {memory_desc}");
            println!("Anthropic upstream: {upstream}");
            println!("OpenAI upstream: {openai_upstream}");
            println!("CPU-only, no model download");
            serve(
                addr,
                ProxyConfig {
                    upstream,
                    openai_upstream,
                    memory,
                    max_memories: 8,
                    membrane_threshold,
                    upstream_auth,
                    upstream_beta,
                    openai_upstream_api_key,
                },
            )
            .await
        }
        Command::Wrap {
            port,
            upstream,
            openai_upstream,
            memory_url,
            tenant,
            memory_dir,
            membrane_threshold,
            upstream_api_key,
            upstream_auth_token,
            upstream_beta,
            openai_upstream_api_key,
            command,
        } => {
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            let (memory, memory_desc) =
                resolve_memory(memory_url.as_deref(), tenant, memory_dir.as_deref());
            let upstream_auth = upstream_auth(upstream_api_key, upstream_auth_token)?;
            eprintln!("theorem-proxy live at http://{addr} (ambient memory: {memory_desc})");
            eprintln!("running: {}", command.join(" "));
            let code = run_wrapped(
                addr,
                ProxyConfig {
                    upstream,
                    openai_upstream,
                    memory,
                    max_memories: 8,
                    membrane_threshold,
                    upstream_auth,
                    upstream_beta,
                    openai_upstream_api_key,
                },
                command,
            )
            .await?;
            std::process::exit(code);
        }
        Command::Doctor {
            memory_url,
            proxy_url,
            valkey_addr,
        } => {
            let checks = theorem_proxy::doctor(
                memory_url.as_deref(),
                proxy_url.as_deref(),
                valkey_addr.as_deref(),
            )
            .await;
            let mut all_ok = true;
            for check in &checks {
                if !check.ok {
                    all_ok = false;
                }
                let mark = if check.ok { "ok  " } else { "FAIL" };
                println!("[{mark}] {:<8} {}", check.name, check.detail);
            }
            if memory_url.is_none() {
                println!("[note] memory   no --memory-url / THEOREM_PROXY_MEMORY_URL set (proxy would run passthrough)");
            }
            println!();
            println!(
                "{}",
                if all_ok {
                    "stack healthy"
                } else {
                    "stack has issues (see FAIL above)"
                }
            );
            Ok(())
        }
    }
}

fn upstream_auth(
    upstream_api_key: Option<String>,
    upstream_auth_token: Option<String>,
) -> std::io::Result<Option<UpstreamAuth>> {
    match (upstream_api_key, upstream_auth_token) {
        (Some(_), Some(_)) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "set only one of --upstream-api-key or --upstream-auth-token",
        )),
        (Some(key), None) => Ok(Some(UpstreamAuth::ApiKey(key))),
        (None, Some(token)) => Ok(Some(UpstreamAuth::Bearer(token))),
        (None, None) => Ok(None),
    }
}
