//! E0.3 acceptance: the embedded binary serves the MCP surface over stdio with
//! no server. Spawn the bin, pipe one JSON-RPC request line to stdin, read the
//! response line from stdout.

use std::io::{Read, Write};
use std::process::{Command, Stdio};

#[test]
fn stdio_binary_serves_mcp_over_stdin_stdout() {
    let dir = std::env::temp_dir().join(format!("rustyred-embedded-stdio-{}", std::process::id()));

    let mut child = Command::new(env!("CARGO_BIN_EXE_rustyred-embedded"))
        .arg(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn embedded binary");

    // One tools/call graphql_introspect request, newline-delimited.
    let request = r#"{"jsonrpc":"2.0","id":"1","method":"tools/call","params":{"name":"graphql_introspect","arguments":{"tenant":"local"}}}"#;
    {
        let mut stdin = child.stdin.take().expect("stdin");
        writeln!(stdin, "{request}").expect("write request");
    } // drop stdin -> EOF -> the read loop ends after this line.

    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read stdout");
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);

    // The response carries the GraphQL SDL (graphAlgorithm appears only there),
    // proving the MCP surface answered in-process over stdio with no server.
    assert!(
        out.contains("graphAlgorithm"),
        "stdio MCP must return the introspected SDL: {out}"
    );
    assert!(
        out.contains("\"jsonrpc\""),
        "response must be a JSON-RPC envelope: {out}"
    );
}
