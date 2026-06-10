use prose_check::{check, writing_engineering_pack_payload, Register};
use std::io::{self, Read};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn run() -> Result<(), String> {
    let mut register = Register::Plain;
    let mut identifiers = Vec::new();
    let mut pack_payload = false;
    let mut parent_hash: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--pack-payload" => {
                pack_payload = true;
            }
            "--parent-hash" => {
                parent_hash = Some(
                    args.next()
                        .ok_or_else(|| "--parent-hash requires a content hash".to_string())?,
                );
            }
            "--register" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--register requires plain, spare, or wire".to_string())?;
                register = parse_register(&value)?;
            }
            "--identifiers" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--identifiers requires a comma-separated list".to_string())?;
                identifiers = value
                    .split(',')
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(str::to_string)
                    .collect();
            }
            "--status" => {
                let _ = args
                    .next()
                    .ok_or_else(|| "--status requires a pack status".to_string())?;
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }

    if pack_payload {
        let payload = writing_engineering_pack_payload(parent_hash.as_deref());
        let output = serde_json::to_string(&payload)
            .map_err(|error| format!("failed to serialize pack payload: {error}"))?;
        println!("{output}");
        return Ok(());
    }

    let mut text = String::new();
    io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| format!("failed to read stdin: {error}"))?;
    let receipt = check(&text, register, &identifiers);
    let output = serde_json::to_string(&receipt)
        .map_err(|error| format!("failed to serialize receipt: {error}"))?;
    println!("{output}");
    Ok(())
}

fn parse_register(value: &str) -> Result<Register, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "plain" => Ok(Register::Plain),
        "spare" => Ok(Register::Spare),
        "wire" => Ok(Register::Wire),
        other => Err(format!("unknown register: {other}")),
    }
}

fn print_usage() {
    println!(
        "Usage: prose-check [--pack-payload [--parent-hash HASH] | --register plain|spare|wire --identifiers a,b --status shadow|advisory|validated|canonical < text]"
    );
}
