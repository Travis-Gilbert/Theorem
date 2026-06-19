//! E0.3: the single embedded RustyRed binary. Opens an `Engine` over a local
//! directory and serves the MCP tool surface over STDIO (newline-delimited
//! JSON-RPC): one request line in, one response line out. No TCP, no HTTP, no
//! socket listener -- "drop the binary, run locally, no server required".
//!
//! Usage: `rustyred-embedded [DATA_DIR]` (or `THEOREM_DATA_DIR=...`; default
//! `./theorem-data`). Each stdin line is a JSON-RPC request (`initialize`,
//! `tools/list`, `tools/call`, ...); each stdout line is its response.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use rustyred_embedded::{EmbeddedConfig, Engine};
use serde_json::{json, Value};

fn data_dir() -> PathBuf {
    if let Some(arg) = std::env::args().nth(1) {
        return PathBuf::from(arg);
    }
    match std::env::var("THEOREM_DATA_DIR") {
        Ok(env) if !env.trim().is_empty() => PathBuf::from(env),
        _ => PathBuf::from("./theorem-data"),
    }
}

fn main() -> std::io::Result<()> {
    let dir = data_dir();
    // Load {dir}/theorem.toml if present, else the single-tenant local default. A
    // malformed config is surfaced (not silently ignored) but does not abort: fall
    // back to the default so the engine still starts.
    let config = EmbeddedConfig::load_for_dir(&dir).unwrap_or_else(|error| {
        eprintln!("rustyred-embedded: {error}; using defaults");
        EmbeddedConfig::default()
    });
    let engine = Engine::open(&dir, config).unwrap_or_else(|error| {
        eprintln!(
            "rustyred-embedded: failed to open store at {}: {error}",
            dir.display()
        );
        std::process::exit(1);
    });
    eprintln!(
        "rustyred-embedded: serving MCP over stdio (tenant={}, data_dir={}); newline-delimited JSON-RPC",
        engine.tenant(),
        dir.display()
    );

    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(request) => engine.handle(request),
            Err(error) => json!({
                "jsonrpc": "2.0",
                "id": Value::Null,
                "error": { "code": -32700, "message": format!("parse error: {error}") }
            }),
        };
        writeln!(stdout, "{}", serde_json::to_string(&response)?)?;
        stdout.flush()?;
    }
    Ok(())
}
