//! AG3 acceptance: MCP access over the store.
//!
//! Plan acceptance (COMMONPLACE-CONSUMER-LOOP.md, AG3):
//! "an external model connects over MCP and reads and writes items."
//!
//! Spawns the `commonplace-mcp` binary and drives it over stdio as an MCP client
//! would: initialize, list tools, write an item (put_note), then read it back
//! (list_items). The binary speaks newline-delimited JSON-RPC 2.0 (the MCP stdio
//! transport).

use std::io::{Read, Write};
use std::process::{Command, Stdio};

#[test]
fn external_model_reads_and_writes_items_over_mcp() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_commonplace-mcp"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn commonplace-mcp");

    let requests = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"put_note","arguments":{"title":"From a model","text":"written over MCP","tags":["mcp"]}}}"#,
        r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_items","arguments":{}}}"#,
    ];
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write request");
        }
    } // drop stdin -> EOF -> the server's read loop ends.

    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read stdout");
    let _ = child.wait();

    // initialize answered with protocol version + server identity.
    assert!(
        out.contains("\"protocolVersion\""),
        "initialize result: {out}"
    );
    assert!(out.contains("commonplace-mcp"), "serverInfo present: {out}");
    // tools/list advertises read + write item tools.
    assert!(
        out.contains("put_note") && out.contains("get_item") && out.contains("search"),
        "tools/list advertises the item tools: {out}"
    );
    // The write happened and a subsequent read shows the item back (round-trip).
    assert!(
        out.contains("From a model"),
        "an item written over MCP is read back over MCP: {out}"
    );
    // The notification produced no response; every emitted line is JSON-RPC.
    let response_lines: Vec<&str> = out.lines().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(
        response_lines.len(),
        4,
        "4 requests with ids -> 4 responses, the notification none: {out}"
    );
    for line in response_lines {
        assert!(
            line.contains("\"jsonrpc\""),
            "each response is a JSON-RPC envelope: {line}"
        );
    }
}
