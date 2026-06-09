//! Real-server smoke: connect to an actual stdio MCP server, register its tools
//! as affordance nodes, and print them. This is the manual end-to-end proof that
//! complements the in-process `FakeTransport` bridge test.
//!
//! Default target is the canonical MCP test server (fetched via npx on first run):
//!
//!   cargo run -p rustyred-thg-connectors --example connect_real_server
//!
//! Override with any stdio MCP server command + args:
//!
//!   cargo run -p rustyred-thg-connectors --example connect_real_server -- npx -y @modelcontextprotocol/server-filesystem /tmp

use std::collections::BTreeMap;

use rustyred_thg_affordances::affordance_nodes;
use rustyred_thg_connectors::{connect_and_register, spawn_stdio, ConnectionTarget};
use rustyred_thg_core::InMemoryGraphStore;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut argv = std::env::args().skip(1);
    let command = argv.next().unwrap_or_else(|| "npx".to_string());
    let mut args: Vec<String> = argv.collect();
    if command == "npx" && args.is_empty() {
        args = vec![
            "-y".to_string(),
            "@modelcontextprotocol/server-everything".to_string(),
        ];
    }

    eprintln!("spawning: {command} {}", args.join(" "));
    let target = ConnectionTarget::Stdio {
        command,
        args,
        env: BTreeMap::new(),
    };

    let mut transport = spawn_stdio(&target)?;
    let mut store = InMemoryGraphStore::default();

    let result = connect_and_register(
        &mut transport,
        &mut store,
        "demo",
        "everything",
        "Everything Server",
        Some("operator"),
    )?;

    println!(
        "\nconnected to: {} v{} (protocol {})",
        result.server_info.server_name,
        result.server_info.server_version,
        result.server_info.protocol_version
    );
    println!(
        "registered {} tools as Affordance nodes:\n",
        result.registration.affordance_node_ids.len()
    );
    for node in affordance_nodes(&store).map_err(|e| format!("{e:?}"))? {
        let name = node
            .properties
            .get("tool_name")
            .and_then(|v| v.as_str())
            .unwrap_or("?");
        let description = node
            .properties
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        println!("  - {name}: {description}");
    }
    Ok(())
}
