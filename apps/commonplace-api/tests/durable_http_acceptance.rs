//! Durable backing for the HTTP surface: items written over HTTP survive a
//! restart of the `commonplace-api` binary.
//!
//! Spawns the real binary with `COMMONPLACE_DATA_DIR` set, ingests over HTTP,
//! kills it, restarts it over the same dir, and reads the item back over HTTP.
//! A process restart is the faithful test: `RedCoreGraphStore` persists at
//! commit time and its data-dir lock is process-scoped, so durability is proven
//! across processes (an in-process reopen is not a valid restart).

use std::path::Path;
use std::process::{Child, Command, Stdio};

const KEY: &str = "durable-key";

fn port() -> u16 {
    51000 + (std::process::id() % 2000) as u16
}

fn spawn_server(dir: &Path, port: u16) -> Child {
    Command::new(env!("CARGO_BIN_EXE_commonplace-api"))
        .env("COMMONPLACE_DATA_DIR", dir)
        .env("COMMONPLACE_API_KEY", KEY)
        .env("PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn commonplace-api")
}

fn curl(args: &[&str]) -> String {
    let output = Command::new("curl")
        .args(args)
        .stderr(Stdio::null())
        .output()
        .expect("run curl");
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn wait_healthy(port: u16) {
    // curl's own retry loop waits for the bind (no foreground sleep needed).
    curl(&[
        "-s",
        "--retry",
        "40",
        "--retry-connrefused",
        "--retry-delay",
        "1",
        &format!("http://127.0.0.1:{port}/healthz"),
    ]);
}

fn graphql(port: u16, query: &str) -> String {
    let body = serde_json::json!({ "query": query }).to_string();
    curl(&[
        "-s",
        "-X",
        "POST",
        &format!("http://127.0.0.1:{port}/graphql"),
        "-H",
        "content-type: application/json",
        "-H",
        &format!("x-api-key: {KEY}"),
        "--data",
        &body,
    ])
}

fn stop(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn http_binary_persists_items_across_restart() {
    let port = port();
    let dir = std::env::temp_dir().join(format!("commonplace-http-restart-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);

    // Session 1: write over HTTP, then kill the server.
    let server = spawn_server(&dir, port);
    wait_healthy(port);
    let write = graphql(
        port,
        r#"mutation { putNote(title: "Durable http binary", text: "rust ownership notes") { id } }"#,
    );
    assert!(write.contains("\"id\""), "putNote over HTTP: {write}");
    stop(server);

    // Session 2: restart over the same dir, read over HTTP.
    let server = spawn_server(&dir, port);
    wait_healthy(port);
    let list = graphql(port, r#"query { items { id title } }"#);
    stop(server);

    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        list.contains("Durable http binary"),
        "item persisted across an HTTP server restart: {list}"
    );
}

// KNOWN GAP (core lane): the full auto-structuring `ingest` write-sequence does
// NOT survive an abrupt process restart, even with AofAlways -- yet it survives
// an in-process reopen, and `put_note`/`editItem` survive a restart. Suspected
// RedCore AOF-flush nuance for the multi-write ingest pattern under abrupt stop
// (rustyred-thg-core). Un-ignore when core fixes it; this asserts the target.
#[test]
#[ignore = "blocked: RedCore loses an ingest write-sequence across an abrupt process restart (core lane)"]
fn ingest_survives_restart_blocked_on_redcore() {
    let port = port() + 1;
    let dir = std::env::temp_dir().join(format!(
        "commonplace-http-ingest-restart-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);

    let server = spawn_server(&dir, port);
    wait_healthy(port);
    let write = graphql(
        port,
        r#"mutation { ingest(input: { title: "Durable ingest", text: "rust ownership notes", kind: "doc" }) { id } }"#,
    );
    assert!(write.contains("\"id\""), "ingest over HTTP: {write}");
    stop(server);

    // DIAGNOSTIC: what is on disk after the SIGKILL, before respawn?
    let aof =
        std::fs::read_to_string(dir.join("graph").join("graph.aof.current")).unwrap_or_default();
    eprintln!("POST-SIGKILL AOF lines = {}", aof.lines().count());
    eprintln!(
        "POST-SIGKILL manifest = {}",
        std::fs::read_to_string(dir.join("graph").join("manifest.json")).unwrap_or_default()
    );

    let server = spawn_server(&dir, port);
    wait_healthy(port);
    let list = graphql(port, r#"query { items { id title } }"#);
    stop(server);

    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        list.contains("Durable ingest"),
        "ingested item should persist across an HTTP server restart: {list}"
    );
}
