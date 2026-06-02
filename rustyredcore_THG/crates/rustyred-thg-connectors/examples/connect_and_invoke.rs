//! Slice 2 driving surface (operator-runnable): connect to a real stdio MCP
//! server, register its tools WITH a persisted connection target, then resolve
//! and DRY-RUN invoke one of them. Complements `connect_real_server` (slice 1,
//! register only) by exercising the invoke bridge end-to-end against a live
//! server. Nothing is sent to the server: the default `InvokePolicy::DryRun`
//! plans the call and fires nothing.
//!
//!   cargo run -p rustyred-thg-connectors --example connect_and_invoke
//!
//! Override the server with any stdio MCP command + args:
//!
//!   cargo run -p rustyred-thg-connectors --example connect_and_invoke -- npx -y @modelcontextprotocol/server-filesystem /tmp

use std::collections::BTreeMap;

use rustyred_thg_affordances::affordance_nodes;
use rustyred_thg_connectors::{
    connect_target, invoke_affordance, ConnectionTarget, InvokePolicy, InvokeRequest,
};
use rustyred_thg_core::InMemoryGraphStore;
use serde_json::json;

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

    let mut store = InMemoryGraphStore::default();

    // Slice 2: connect_target registers the tools AND persists the reach
    // (the ConnectionTarget) on the Connector node, so a selected tool can be
    // invoked later without re-supplying the connection details. connect_and_register
    // (slice 1) registers but does NOT persist the reach.
    let result = connect_target(
        target,
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
        "registered {} tools, with a persisted connection target.\n",
        result.registration.affordance_node_ids.len()
    );

    // Pick the first registered tool to demonstrate the invoke bridge against.
    let nodes = affordance_nodes(&store).map_err(|e| format!("{e:?}"))?;
    let Some(affordance_id) = nodes.iter().find_map(|node| {
        node.properties
            .get("affordance_id")
            .and_then(|v| v.as_str())
            .map(str::to_string)
    }) else {
        println!("no tools registered; nothing to invoke.");
        return Ok(());
    };

    // Dry-run invoke (the DEFAULT): resolve the planned call from the persisted
    // reach, then fire NOTHING. This is the safe posture; live firing is opt-in.
    let report = invoke_affordance(
        &mut store,
        InvokeRequest {
            tenant_id: "demo".to_string(),
            task_type: "demo-task".to_string(),
            affordance_id: affordance_id.clone(),
            arguments: json!({}),
            candidate_affordance_ids: vec![affordance_id.clone()],
        },
        &InvokePolicy::DryRun,
        Some("operator"),
    )
    .map_err(|e| format!("{e:?}"))?;

    println!("dry-run invoke of `{affordance_id}`:");
    println!("  tool       : {}", report.planned.tool_name);
    println!("  server     : {}", report.planned.server_id);
    println!("  reachable  : {:?}", report.planned.connection_target);
    println!("  fired      : {}", report.fired);
    if let Some(reason) = &report.dry_run_reason {
        println!("  why        : {reason}");
    }
    println!(
        "\nNothing was sent to the server. To actually fire this tool, invoke with an\n\
         explicit allowlist: InvokePolicy::FireAllowlist(vec![\"{affordance_id}\".into()]).\n\
         The outcome would then feed record_invocation and update the tool's fitness."
    );
    Ok(())
}
