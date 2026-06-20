//! Durable backing: items written over MCP survive a process restart.
//!
//! Proves the `COMMONPLACE_DATA_DIR` durable path of `commonplace-mcp`: a note
//! written in one process is read back by a fresh process over the same data
//! dir (RedCoreGraphStore + DiskObjectStore), so a self-hosted instance is a
//! real durable store, not an ephemeral demo.

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

fn run_session(dir: &Path, requests: &[&str]) -> String {
    let mut child = Command::new(env!("CARGO_BIN_EXE_commonplace-mcp"))
        .env("COMMONPLACE_DATA_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn commonplace-mcp");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        for request in requests {
            writeln!(stdin, "{request}").expect("write request");
        }
    } // drop stdin -> EOF -> process exits, store flushes + drops.
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("read stdout");
    let _ = child.wait();
    out
}

#[test]
fn mcp_persists_items_across_restart() {
    let dir = std::env::temp_dir().join(format!(
        "commonplace-mcp-durable-{}-{}",
        std::process::id(),
        env!("CARGO_PKG_NAME")
    ));
    let _ = std::fs::remove_dir_all(&dir);

    // Session 1: a model writes a note durably.
    let first = run_session(
        &dir,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"put_note","arguments":{"title":"Durable note","text":"survives a restart"}}}"#,
        ],
    );
    assert!(first.contains("Durable note"), "write session: {first}");

    // Session 2: a fresh process over the same data dir reads it back.
    let second = run_session(
        &dir,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_items","arguments":{}}}"#,
        ],
    );
    assert!(
        second.contains("Durable note"),
        "the note persisted across a process restart: {second}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}
