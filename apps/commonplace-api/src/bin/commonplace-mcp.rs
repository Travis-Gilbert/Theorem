//! The CommonPlace MCP stdio server (plan unit AG3 + durable backing).
//!
//! Any model reaches the store over MCP: this binary speaks newline-delimited
//! JSON-RPC 2.0 over stdin/stdout (the MCP stdio transport). Spawn it as an MCP
//! server and it exposes read/write item tools over the consumer store.
//!
//! Backing: set `COMMONPLACE_DATA_DIR` to persist durably (a `RedCoreGraphStore`
//! over `<dir>/graph` + a `DiskObjectStore` over `<dir>/blobs`), so items written
//! by a model survive a restart. Unset = an ephemeral in-memory store (one
//! dataset per process). `handle_request` is generic over the store, so the
//! single `serve` loop runs over either backing.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use commonplace::{BlobStore, Commonplace, EmbeddingGraphStore, InMemoryBlobStore};
use commonplace_api::mcp;
use rustyred_thg_core::{
    DiskObjectStore, InMemoryGraphStore, RedCoreDurability, RedCoreGraphStore, RedCoreOptions,
};
use serde_json::{json, Value};

fn main() {
    match std::env::var("COMMONPLACE_DATA_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let root = PathBuf::from(dir);
            let options = RedCoreOptions {
                durability: RedCoreDurability::AofAlways,
                ..RedCoreOptions::default()
            };
            let store = RedCoreGraphStore::open(root.join("graph"), options)
                .expect("open RedCore graph store");
            let blobs = DiskObjectStore::open(root.join("blobs")).expect("open disk blob store");
            serve(Commonplace::new(store, blobs));
        }
        _ => serve(Commonplace::new(
            InMemoryGraphStore::new(),
            InMemoryBlobStore::new(),
        )),
    }
}

fn serve<S, B>(mut cp: Commonplace<S, B>)
where
    S: EmbeddingGraphStore,
    B: BlobStore,
{
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
            let json = match serde_json::to_string(&response) {
                Ok(json) => json,
                Err(error) => {
                    eprintln!("mcp response serialization error: {error}");
                    break;
                }
            };
            if writeln!(out, "{json}").is_err() {
                break;
            }
            let _ = out.flush();
        }
    }
}
