use std::path::PathBuf;
use std::time::Duration;

use theorem_agentd::config::AgentdConfig;
use theorem_agentd::mcp::McpRouter;
use theorem_agentd::model::ModelClient;
use theorem_agentd::receiver_sidecar::spawn_receiver_sidecar;
use theorem_agentd::tools::ToolCatalog;
use theorem_agentd::turn_loop::{run_once, run_tick};
use theorem_agentd::{AgentdError, AgentdResult};

fn main() {
    if let Err(error) = run() {
        eprintln!("[theorem-agentd] fatal: {error}");
        std::process::exit(1);
    }
}

fn run() -> AgentdResult<()> {
    let args = Args::parse(std::env::args().skip(1).collect())?;
    let config = AgentdConfig::load(&args.config_path)?;
    let catalog = ToolCatalog::default_catalog();
    if args.print_tool_grammar {
        println!("{}", catalog.gbnf_grammar());
        return Ok(());
    }
    let _receiver = if config.receiver.enabled
        && !args.no_receiver
        && args.once.is_none()
        && !args.capture_once
    {
        Some(spawn_receiver_sidecar(&config.receiver.config_path)?)
    } else {
        None
    };
    let router = McpRouter::from_configs(config.all_mcp_servers())?;

    // One mechanical Agent Queue capture sweep, then exit. No model required.
    if args.capture_once {
        let report = theorem_agentd::capture::run_capture(&router, &config.capture, &config.actor)?;
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let model = ModelClient::from_config(
        config.model.clone(),
        config.default_room_id.clone(),
        config.actor.clone(),
    )?;

    if let Some(prompt) = args.once {
        let transcript = run_once(&config, &model, &router, &catalog, &prompt)?;
        println!("{}", serde_json::to_string_pretty(&transcript)?);
        return Ok(());
    }

    loop {
        // Each tick: capture Agent Queue tasks into jobs, relay run milestones
        // back to TickTick, then take a proactive coordination turn.
        let prompt = "timer tick: poll coordination room and inbox for proactive work";
        let report = run_tick(&config, &model, &router, &catalog, prompt);
        if let Some(transcript) = &report.transcript {
            println!("{}", serde_json::to_string(transcript)?);
        }
        std::thread::sleep(Duration::from_secs(config.loop_config.tick_interval_secs));
    }
}

struct Args {
    config_path: PathBuf,
    once: Option<String>,
    no_receiver: bool,
    print_tool_grammar: bool,
    capture_once: bool,
}

impl Args {
    fn parse(args: Vec<String>) -> AgentdResult<Self> {
        let mut config_path = PathBuf::from("theorem-agentd.toml");
        let mut once = None;
        let mut no_receiver = false;
        let mut print_tool_grammar = false;
        let mut capture_once = false;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "--once" => {
                    i += 1;
                    let Some(prompt) = args.get(i) else {
                        return Err(AgentdError::Config("--once requires a prompt".to_string()));
                    };
                    once = Some(prompt.clone());
                }
                "--no-receiver" => {
                    no_receiver = true;
                }
                "--capture-once" => {
                    capture_once = true;
                }
                "--print-tool-grammar" => {
                    print_tool_grammar = true;
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
                }
            }
            i += 1;
        }
        Ok(Self {
            config_path,
            once,
            no_receiver,
            print_tool_grammar,
            capture_once,
        })
    }
}

fn print_help() {
    println!(
        "usage: theorem-agentd [--once <prompt>] [--capture-once] [--no-receiver] [--print-tool-grammar] [config.toml]"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_once_and_config_path() {
        let args = Args::parse(vec![
            "--once".to_string(),
            "hello".to_string(),
            "agentd.toml".to_string(),
        ])
        .unwrap();
        assert_eq!(args.once, Some("hello".to_string()));
        assert_eq!(args.config_path, PathBuf::from("agentd.toml"));
    }

    #[test]
    fn parses_no_receiver() {
        let args = Args::parse(vec!["--no-receiver".to_string()]).unwrap();
        assert!(args.no_receiver);
    }
}
