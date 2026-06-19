//! The CommonPlace MCP stdio server (plan unit AG3).
//!
//! Any model reaches the store over MCP: this binary speaks newline-delimited
//! JSON-RPC 2.0 over stdin/stdout (the MCP stdio transport). Spawn it as an MCP
//! server and it exposes read/write item tools over the consumer store.
//!
//! Backing is the in-memory store (one dataset per process); a durable
//! `RedCoreGraphStore` + `DiskObjectStore` backing over a data dir is the named
//! follow-up, alongside the HTTP `commonplace-api` binary.

use std::io::{BufRead, Write};

use commonplace::{Commonplace, InMemoryBlobStore};
use commonplace_api::mcp;
use rustyred_thg_core::InMemoryGraphStore;
use serde_json::{json, Value};

fn main() {
    let mut cp = Commonplace::new(InMemoryGraphStore::new(), InMemoryBlobStore::new());
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {error}") }
                });
                if writeln!(out, "{response}").is_err() {
                    break;
                }
                let _ = out.flush();
                continue;
            }
        };
        if let Some(response) = mcp::handle_request(&mut cp, &request) {
            if writeln!(
                out,
                "{}",
                serde_json::to_string(&response).unwrap_or_default()
            )
            .is_err()
            {
                break;
            }
            let _ = out.flush();
        }
    }
}
