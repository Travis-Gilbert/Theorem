// This integration test pulls in client.rs via #[path] but exercises only a
// subset of its methods, so unused-method dead_code warnings are expected here.
#![allow(dead_code)]

//! HTTP-seam integration test. A minimal in-process mock server (std only, no
//! extra deps) stands in for RustyRed and lets us prove the client end of the
//! contract: correct route, `Authorization: Bearer` header, request body shape,
//! and response parsing. This is the seam most likely to drift from the server
//! without a pure-function unit test catching it.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

/// A captured request the mock saw.
struct Captured {
    method: String,
    path: String,
    auth: Option<String>,
    body: String,
}

/// Start a mock HTTP server that answers `expected` requests, choosing a canned
/// JSON body by path substring, then exits. Returns (base_url, captured-rx).
fn start_mock(
    expected: usize,
    responder: fn(&str) -> &'static str,
) -> (String, mpsc::Receiver<Captured>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        for _ in 0..expected {
            let (stream, _) = listener.accept().unwrap();
            handle_conn(stream, &tx, responder);
        }
    });

    (format!("http://127.0.0.1:{port}"), rx)
}

fn handle_conn(
    stream: TcpStream,
    tx: &mpsc::Sender<Captured>,
    responder: fn(&str) -> &'static str,
) {
    let mut reader = BufReader::new(stream);

    // Request line.
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    // Headers.
    let mut content_length = 0usize;
    let mut auth = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        let lower = trimmed.to_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap_or(0);
        } else if lower.starts_with("authorization:") {
            auth = Some(trimmed["authorization:".len()..].trim().to_string());
        }
    }

    // Body.
    let mut body_buf = vec![0u8; content_length];
    reader.read_exact(&mut body_buf).unwrap();
    let body = String::from_utf8_lossy(&body_buf).to_string();

    let response_body = responder(&path);
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        response_body.len(),
        response_body
    );
    reader.get_mut().write_all(response.as_bytes()).unwrap();
    reader.get_mut().flush().unwrap();

    tx.send(Captured {
        method,
        path,
        auth,
        body,
    })
    .unwrap();
}

fn canned(path: &str) -> &'static str {
    if path.ends_with("/graph/nodes") {
        r#"{"ok":true,"node":{"id":"company:qdrant"}}"#
    } else if path.ends_with("/graph/nodes/query") {
        r#"{"ok":true,"nodes":[{"id":"role:hn:1","labels":["Role"],"properties":{"title":"Rust Engineer"}}]}"#
    } else if path.ends_with("/graph/algorithms/ppr") {
        r#"{"ok":true,"tenant":"t","scores":[{"node_id":"role:hn:1","score":0.42}]}"#
    } else if path.ends_with("/graph/vector/search") {
        r#"{"ok":true,"results":[{"node_id":"role:hn:1","distance":0.1,"node":null}]}"#
    } else {
        r#"{"ok":true}"#
    }
}

// The client module lives in the binary crate; re-declare the minimal env the
// test needs by driving the public CLI surface would be heavier, so instead we
// reach the client through a tiny config built from env. We exercise the real
// reqwest calls against the mock.
#[path = "../src/client.rs"]
mod client;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/error.rs"]
mod error;

use client::{NodeSpec, RustyRedClient};
use config::Config;
use serde_json::json;
use std::collections::HashMap;

fn config_for(base: &str) -> Config {
    Config {
        rustyred_url: base.to_string(),
        tenant: "demo".to_string(),
        token: "secret-token".to_string(),
        hunter_api_key: None,
        embed_url: None,
        embed_dim: 384,
    }
}

#[test]
fn client_round_trips_against_mock_server() {
    let (base, rx) = start_mock(4, canned);
    let client = RustyRedClient::new(&config_for(&base)).unwrap();

    // 1. node upsert
    let node = NodeSpec {
        id: "company:qdrant".into(),
        labels: vec!["Company".into()],
        properties: json!({ "name": "Qdrant" }),
    };
    let node_resp = client.upsert_node(&node).unwrap();
    assert_eq!(node_resp["ok"], true);

    // 2. nodes/query readback
    let nodes = client.query_nodes("Role", None).unwrap();
    assert_eq!(nodes.len(), 1);
    assert_eq!(nodes[0]["id"], "role:hn:1");

    // 3. ppr
    let mut seeds = HashMap::new();
    seeds.insert("profile:travis".to_string(), 1.0);
    let scores = client.ppr(&seeds, None).unwrap();
    assert_eq!(scores.len(), 1);
    assert_eq!(scores[0].node_id, "role:hn:1");
    assert!((scores[0].score - 0.42).abs() < 1e-9);

    // 4. vector search
    let hits = client
        .vector_search(&[0.1, 0.2], 5, Some("Role"), "embedding")
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert!((hits[0].distance - 0.1).abs() < 1e-6);

    // Assert what the server actually received.
    let reqs: Vec<Captured> = (0..4).map(|_| rx.recv().unwrap()).collect();
    for r in &reqs {
        assert_eq!(r.method, "POST");
        assert_eq!(
            r.auth.as_deref(),
            Some("Bearer secret-token"),
            "bearer auth must be sent"
        );
        assert!(
            r.path.starts_with("/v1/tenants/demo/"),
            "path scoped to tenant: {}",
            r.path
        );
    }
    // The node-upsert body must carry the NodeWriteBody fields.
    let upsert = reqs
        .iter()
        .find(|r| r.path.ends_with("/graph/nodes"))
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&upsert.body).unwrap();
    assert_eq!(parsed["id"], "company:qdrant");
    assert_eq!(parsed["labels"][0], "Company");
    // The PPR body must send seeds as an object (node_id -> weight).
    let ppr = reqs
        .iter()
        .find(|r| r.path.ends_with("/graph/algorithms/ppr"))
        .unwrap();
    let ppr_body: serde_json::Value = serde_json::from_str(&ppr.body).unwrap();
    assert_eq!(ppr_body["seeds"]["profile:travis"], 1.0);
}
