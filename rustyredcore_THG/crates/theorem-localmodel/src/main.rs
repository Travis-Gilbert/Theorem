use std::path::PathBuf;
use std::time::Duration;

use theorem_localmodel::config::LocalModelConfig;
use theorem_localmodel::mcp::McpRouter;
use theorem_localmodel::model::ModelClient;
use theorem_localmodel::receiver_sidecar::spawn_receiver_sidecar;
use theorem_localmodel::tools::ToolCatalog;
use theorem_localmodel::turn_loop::{run_once, run_tick};
use theorem_localmodel::{LocalModelError, LocalModelResult};

fn main() {
    if let Err(error) = run() {
        eprintln!("[theorem-localmodel] fatal: {error}");
        std::process::exit(1);
    }
}

fn run() -> LocalModelResult<()> {
    let args = Args::parse(std::env::args().skip(1).collect())?;
    let config = if args.config_path_explicit {
        LocalModelConfig::load(&args.config_path)?
    } else {
        LocalModelConfig::load_or_default(&args.config_path)?
    };
    match args.command {
        Command::ServeLocalModel => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(theorem_localmodel::local_host::serve(config.local_model));
        }
        Command::DoctorLocalModel => {
            let report = theorem_localmodel::local_host::doctor_report(&config.local_model);
            println!("{}", serde_json::to_string_pretty(&report)?);
            return Ok(());
        }
        Command::Loop => {}
    }
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
        let report =
            theorem_localmodel::capture::run_capture(&router, &config.capture, &config.actor)?;
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
    command: Command,
    config_path: PathBuf,
    config_path_explicit: bool,
    once: Option<String>,
    no_receiver: bool,
    print_tool_grammar: bool,
    capture_once: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Loop,
    ServeLocalModel,
    DoctorLocalModel,
}

impl Args {
    fn parse(args: Vec<String>) -> LocalModelResult<Self> {
        let mut command = Command::Loop;
        let mut config_path = PathBuf::from("theorem-localmodel.toml");
        let mut config_path_explicit = false;
        let mut once = None;
        let mut no_receiver = false;
        let mut print_tool_grammar = false;
        let mut capture_once = false;
        let mut i = 0;
        while i < args.len() {
            match args[i].as_str() {
                "serve" => {
                    command = Command::ServeLocalModel;
                }
                "doctor" => {
                    command = Command::DoctorLocalModel;
                }
                "--once" => {
                    i += 1;
                    let Some(prompt) = args.get(i) else {
                        return Err(LocalModelError::Config(
                            "--once requires a prompt".to_string(),
                        ));
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
                    return Err(LocalModelError::Config(format!("unknown flag {value}")));
                }
                value => {
                    config_path = PathBuf::from(value);
                    config_path_explicit = true;
                }
            }
            i += 1;
        }
        Ok(Self {
            command,
            config_path,
            config_path_explicit,
            once,
            no_receiver,
            print_tool_grammar,
            capture_once,
        })
    }
}

fn print_help() {
    println!(
        "usage: theorem-localmodel [serve|doctor] [--once <prompt>] [--capture-once] [--no-receiver] [--print-tool-grammar] [config.toml]\n\nCommands:\n  serve   Start the local mistral.rs model host at [local_model].host/port.\n  doctor  Print local mistral.rs/Theorem host diagnostics without loading a model.\n\nIf the implicit theorem-localmodel.toml is absent, theorem-localmodel starts with no-config local defaults. Explicit config paths must exist."
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
            "localmodel.toml".to_string(),
        ])
        .unwrap();
        assert_eq!(args.command, Command::Loop);
        assert_eq!(args.once, Some("hello".to_string()));
        assert_eq!(args.config_path, PathBuf::from("localmodel.toml"));
        assert!(args.config_path_explicit);
    }

    #[test]
    fn parses_no_receiver() {
        let args = Args::parse(vec!["--no-receiver".to_string()]).unwrap();
        assert!(args.no_receiver);
        assert!(!args.config_path_explicit);
    }

    #[test]
    fn parses_local_model_commands() {
        let serve = Args::parse(vec!["serve".to_string(), "localmodel.toml".to_string()]).unwrap();
        assert_eq!(serve.command, Command::ServeLocalModel);
        assert_eq!(serve.config_path, PathBuf::from("localmodel.toml"));

        let doctor = Args::parse(vec!["doctor".to_string()]).unwrap();
        assert_eq!(doctor.command, Command::DoctorLocalModel);
        assert!(!doctor.config_path_explicit);
    }
}
