use std::net::IpAddr;
use std::path::PathBuf;

use theorem_agentd::config::AgentdConfig;
use theorem_agentd::proxy::{ProxyCli, ProxyConfig};
use theorem_agentd::{AgentdError, AgentdResult};

fn main() {
    if let Err(error) = run() {
        eprintln!("[rustyred-proxy] fatal: {error}");
        std::process::exit(1);
    }
}

fn run() -> AgentdResult<()> {
    let args = Args::parse(std::env::args().skip(1).collect())?;
    let config = if args.config_path_explicit {
        AgentdConfig::load(&args.config_path)?
    } else {
        AgentdConfig::load_or_default(&args.config_path)?
    };
    let proxy_config = ProxyConfig::from_agentd(&config, &args.proxy);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(theorem_agentd::proxy::serve_proxy(proxy_config))
}

#[derive(Debug)]
struct Args {
    config_path: PathBuf,
    config_path_explicit: bool,
    proxy: ProxyCli,
}

impl Args {
    fn parse(args: Vec<String>) -> AgentdResult<Self> {
        let mut config_path = PathBuf::from("theorem-agentd.toml");
        let mut config_path_explicit = false;
        let mut proxy = ProxyCli::default();
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--proxy-port" | "--port" => {
                    i += 1;
                    let Some(port) = args.get(i).and_then(|value| value.parse::<u16>().ok()) else {
                        return Err(AgentdError::Config(
                            "--proxy-port requires a TCP port".to_string(),
                        ));
                    };
                    proxy.port = Some(port);
                }
                "--proxy-bind" | "--bind" => {
                    i += 1;
                    let Some(bind) = args.get(i).and_then(|value| value.parse::<IpAddr>().ok())
                    else {
                        return Err(AgentdError::Config(
                            "--proxy-bind requires an IP address".to_string(),
                        ));
                    };
                    proxy.bind = Some(bind);
                }
                "--proxy-data-dir" | "--data-dir" => {
                    i += 1;
                    let Some(path) = args.get(i) else {
                        return Err(AgentdError::Config(
                            "--proxy-data-dir requires a path".to_string(),
                        ));
                    };
                    proxy.data_dir = Some(PathBuf::from(path));
                }
                "--proxy-upstream" | "--upstream" => {
                    i += 1;
                    let Some(url) = args.get(i) else {
                        return Err(AgentdError::Config(
                            "--proxy-upstream requires a base URL".to_string(),
                        ));
                    };
                    proxy.upstream_base_url = Some(url.clone());
                }
                "--proxy-harness-url" | "--harness-url" => {
                    i += 1;
                    let Some(url) = args.get(i) else {
                        return Err(AgentdError::Config(
                            "--proxy-harness-url requires a URL".to_string(),
                        ));
                    };
                    proxy.harness_mcp_url = Some(url.clone());
                }
                "--proxy-room-id" | "--room-id" | "--room" => {
                    i += 1;
                    let Some(room_id) = args.get(i) else {
                        return Err(AgentdError::Config(
                            "--proxy-room-id requires a room id".to_string(),
                        ));
                    };
                    proxy.room_id = Some(room_id.clone());
                }
                "--proxy-no-ambient" | "--no-ambient" => {
                    proxy.enable_ambient = Some(false);
                }
                "-h" | "--help" => {
                    print_help();
                    std::process::exit(0);
                }
                value if value.starts_with('-') => {
                    return Err(AgentdError::Config(format!("unknown flag {value}")));
                }
                value => {
                    config_path = PathBuf::from(value);
                    config_path_explicit = true;
                }
            }
            i += 1;
        }
        Ok(Self {
            config_path,
            config_path_explicit,
            proxy,
        })
    }
}

fn print_help() {
    println!(
        "usage: rustyred-proxy [--proxy-port <port>] [--proxy-room-id <room>] [--proxy-data-dir <path>] [--proxy-harness-url <url>] [--proxy-upstream <url>] [--proxy-no-ambient] [config.toml]\n\nIf the implicit theorem-agentd.toml is absent, rustyred-proxy starts with no-config local defaults. Explicit config paths must exist."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_defaults_without_explicit_config() {
        let args = Args::parse(Vec::new()).unwrap();
        assert_eq!(args.config_path, PathBuf::from("theorem-agentd.toml"));
        assert!(!args.config_path_explicit);
        assert_eq!(args.proxy.port, None);
    }

    #[test]
    fn parses_proxy_flags_and_config_path() {
        let args = Args::parse(vec![
            "--proxy-port".to_string(),
            "18484".to_string(),
            "--proxy-data-dir".to_string(),
            "/tmp/theorem-proxy".to_string(),
            "--proxy-room-id".to_string(),
            "repo:theorem:branch:main".to_string(),
            "--proxy-no-ambient".to_string(),
            "agentd.toml".to_string(),
        ])
        .unwrap();
        assert_eq!(args.proxy.port, Some(18484));
        assert_eq!(
            args.proxy.data_dir,
            Some(PathBuf::from("/tmp/theorem-proxy"))
        );
        assert_eq!(
            args.proxy.room_id,
            Some("repo:theorem:branch:main".to_string())
        );
        assert_eq!(args.proxy.enable_ambient, Some(false));
        assert_eq!(args.config_path, PathBuf::from("agentd.toml"));
        assert!(args.config_path_explicit);
    }
}
