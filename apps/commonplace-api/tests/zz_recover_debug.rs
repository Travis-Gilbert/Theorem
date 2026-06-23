use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

fn session(dir: &Path, requests: &[&str], capture_stderr: bool) -> (String, String) {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_commonplace-mcp"));
    cmd.env("COMMONPLACE_DATA_DIR", dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(if capture_stderr {
            Stdio::piped()
        } else {
            Stdio::null()
        });
    let mut child = cmd.spawn().expect("spawn");
    {
        let mut stdin = child.stdin.take().unwrap();
        for r in requests {
            writeln!(stdin, "{r}").unwrap();
        }
    }
    let mut out = String::new();
    child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
    let mut err = String::new();
    if capture_stderr {
        if let Some(mut e) = child.stderr.take() {
            let _ = e.read_to_string(&mut err);
        }
    }
    let _ = child.wait();
    (out, err)
}

#[test]
fn capture_ingest_recover_error() {
    let dir = std::env::temp_dir().join(format!("cp-recover-dbg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    let (s1, _) = session(
        &dir,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ingest","arguments":{"title":"Durable ingest","text":"rust ownership and borrowing"}}}"#,
        ],
        false,
    );
    eprintln!("SESSION1 stdout head: {}", s1.chars().take(120).collect::<String>());

    let graph = dir.join("graph");
    eprintln!("GRAPH DIR LISTING:");
    if let Ok(rd) = std::fs::read_dir(&graph) {
        for e in rd.flatten() {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            eprintln!("  {} ({len} bytes)", e.file_name().to_string_lossy());
        }
    }
    let aof = std::fs::read_to_string(graph.join("graph.aof.current")).unwrap_or_default();
    eprintln!("AOF lines = {}", aof.lines().count());
    for (i, l) in aof.lines().enumerate() {
        eprintln!("  AOF[{i}] head: {}", l.chars().take(180).collect::<String>());
    }
    let manifest = std::fs::read_to_string(graph.join("manifest.json")).unwrap_or_default();
    eprintln!("MANIFEST: {manifest}");

    let (s2out, s2err) = session(
        &dir,
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"list_items","arguments":{}}}"#,
        ],
        true,
    );
    eprintln!("SESSION2 stdout: {s2out}");
    eprintln!("SESSION2 stderr: {s2err}");

    let _ = std::fs::remove_dir_all(&dir);
}
