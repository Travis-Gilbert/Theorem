// Integration tests for the 0.2 outreach seams. Like tests/client_http.rs, a
// minimal in-process mock server (std only) stands in for the remote service so
// we prove the two seams most likely to drift:
//
//  1. The Gmail transport: create_draft posts the right route + bearer + body
//     shape and parses (draft_id, thread_id); list/thread reads parse too.
//  2. The load-bearing read-modify-write: setting outreach_status on a Role GETs
//     the full node and re-upserts it with the original title/body/embedding
//     PRESERVED (RustyRed's upsert replaces wholesale, so a naive write would
//     wipe them). draft_top proves the same through the full draft flow.
//
// The included modules re-run their own unit tests inside this binary (same as
// client_http.rs); that is harmless.
#![allow(dead_code)]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;

#[path = "../src/error.rs"]
mod error;
#[path = "../src/config.rs"]
mod config;
#[path = "../src/model.rs"]
mod model;
#[path = "../src/client.rs"]
mod client;
#[path = "../src/embed/mod.rs"]
mod embed;
#[path = "../src/graph/mod.rs"]
mod graph;
#[path = "../src/profile.rs"]
mod profile;
#[path = "../src/outreach/mod.rs"]
mod outreach;

use client::RustyRedClient;
use config::Config;
use model::{Role, ScoredLead};
use outreach::draft::{draft_top, DraftSink};
use outreach::gmail::{DraftRef, GmailClient};
use outreach::state::{apply_role_updates, LeadState};
use outreach::sync::{run_sync, ReplySource};
use outreach::testclock::FixedClock;
use outreach::OutreachStatus;
use profile::{ProofPoints, ResolvedProfile};
use serde_json::{json, Value};

// ---- mock HTTP server ------------------------------------------------------

struct Captured {
    method: String,
    path: String,
    auth: Option<String>,
    body: String,
}

/// Start a mock that answers `expected` requests, choosing a JSON body by
/// (method, path), then exits. Returns (base_url, captured-rx).
fn start_mock(
    expected: usize,
    responder: fn(&str, &str) -> String,
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

fn handle_conn(stream: TcpStream, tx: &mpsc::Sender<Captured>, responder: fn(&str, &str) -> String) {
    let mut reader = BufReader::new(stream);
    let mut request_line = String::new();
    reader.read_line(&mut request_line).unwrap();
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

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
    let mut body_buf = vec![0u8; content_length];
    reader.read_exact(&mut body_buf).unwrap();
    let body = String::from_utf8_lossy(&body_buf).to_string();

    let response_body = responder(&method, &path);
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

fn drain(rx: &mpsc::Receiver<Captured>, n: usize) -> Vec<Captured> {
    (0..n).map(|_| rx.recv().unwrap()).collect()
}

// ---- fixtures --------------------------------------------------------------

fn rustyred_config(base: &str) -> Config {
    Config {
        rustyred_url: base.to_string(),
        tenant: "jobintel".to_string(),
        token: "rr-token".to_string(),
        hunter_api_key: None,
        embed_url: None,
        embed_dim: 384,
        gmail_token: Some("gmail-token".to_string()),
        gmail_api_base: base.to_string(),
        daily_draft_cap: 8,
        followup_days: vec![4, 9],
    }
}

fn profile() -> ResolvedProfile {
    ResolvedProfile {
        id: "profile:travis".into(),
        handle: "travis".into(),
        text: "rust graph vector".into(),
        skills: vec!["rust".into(), "graph".into()],
        embedding: vec![0.1, 0.2],
        proof: ProofPoints::default(),
    }
}

fn hn_lead() -> ScoredLead {
    ScoredLead {
        role: Role {
            id: "role:hn:1".into(),
            company: "Qdrant".into(),
            company_id: "company:qdrant".into(),
            title: "Senior Rust Engineer".into(),
            location: "Remote".into(),
            url: "https://news.ycombinator.com/item?id=1".into(),
            body: "We need someone to build a Rust graph engine with vector search.".into(),
            source: "hn".into(),
            remote: true,
            contract: false,
            founder_posted: true,
            email_present: true,
            emails: vec!["hire@qdrant.tech".into()],
            comp: None,
            company_domain: None,
        },
        score: 0.8,
        semantic: 0.7,
        graph: 0.6,
        flags: 1.0,
        matched_skills: vec!["rust".into(), "graph".into()],
        contact: Some("hire@qdrant.tech".into()),
        needs_contact: false,
    }
}

/// The full Role node a GET returns: the ingest props the read-modify-write must
/// preserve when it adds outreach state.
fn role_node_json() -> String {
    json!({
        "ok": true,
        "node": {
            "id": "role:hn:1",
            "labels": ["Role"],
            "properties": {
                "title": "Senior Rust Engineer",
                "company": "Qdrant",
                "company_id": "company:qdrant",
                "source": "hn",
                "body": "We need someone to build a Rust graph engine with vector search.",
                "url": "https://news.ycombinator.com/item?id=1",
                "remote": true,
                "contract": false,
                "founder_posted": true,
                "email_present": true,
                "emails": ["hire@qdrant.tech"],
                "embedding": [0.1, 0.2, 0.3]
            }
        }
    })
    .to_string()
}

// ---- 1. Gmail transport seam ----------------------------------------------

fn gmail_responder(method: &str, path: &str) -> String {
    if method == "POST" && path.ends_with("/drafts") {
        r#"{"id":"draft-99","message":{"id":"msg-1","threadId":"thread-7"}}"#.to_string()
    } else if method == "GET" && path.ends_with("/drafts") {
        r#"{"drafts":[{"id":"draft-99"},{"id":"draft-12"}]}"#.to_string()
    } else if method == "GET" && path.contains("/threads/") {
        json!({
            "messages": [
                { "payload": { "headers": [ { "name": "From", "value": "Lead <hire@qdrant.tech>" } ] } }
            ]
        })
        .to_string()
    } else {
        r#"{}"#.to_string()
    }
}

#[test]
fn gmail_create_draft_posts_route_auth_body_and_parses_ids() {
    let (base, rx) = start_mock(1, gmail_responder);
    let gmail = GmailClient::new(Some("gmail-token"), &base).unwrap();

    let draft = gmail
        .create_draft("hire@qdrant.tech", "Hello", "Body text", None)
        .unwrap();
    assert_eq!(
        draft,
        DraftRef {
            draft_id: "draft-99".into(),
            thread_id: "thread-7".into()
        }
    );

    let req = rx.recv().unwrap();
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/gmail/v1/users/me/drafts");
    assert_eq!(req.auth.as_deref(), Some("Bearer gmail-token"));
    let body: Value = serde_json::from_str(&req.body).unwrap();
    // The raw RFC822 is base64url under message.raw; threadId omitted for a new thread.
    let raw = body["message"]["raw"].as_str().unwrap();
    assert!(!raw.is_empty());
    assert!(body["message"].get("threadId").is_none());
}

#[test]
fn gmail_create_draft_threads_a_followup() {
    let (base, _rx) = start_mock(1, gmail_responder);
    let gmail = GmailClient::new(Some("gmail-token"), &base).unwrap();
    let draft = gmail
        .create_draft("x@y.com", "Re: Hello", "nudge", Some("thread-7"))
        .unwrap();
    assert_eq!(draft.thread_id, "thread-7");
}

#[test]
fn gmail_list_drafts_and_thread_senders_parse() {
    let (base, _rx) = start_mock(2, gmail_responder);
    let gmail = GmailClient::new(Some("gmail-token"), &base).unwrap();
    let ids = gmail.list_draft_ids().unwrap();
    assert_eq!(ids, vec!["draft-99", "draft-12"]);
    let senders = gmail.thread_senders("thread-7").unwrap();
    assert_eq!(senders, vec!["hire@qdrant.tech"]);
}

// ---- 2. Read-modify-write seam --------------------------------------------

fn rustyred_node_responder(method: &str, path: &str) -> String {
    if method == "GET" && path.contains("/graph/nodes/role") {
        role_node_json()
    } else {
        r#"{"ok":true,"node":{"id":"role:hn:1"}}"#.to_string()
    }
}

#[test]
fn apply_role_updates_preserves_existing_properties() {
    // GET (read) + POST (upsert) = 2 requests.
    let (base, rx) = start_mock(2, rustyred_node_responder);
    let client = RustyRedClient::new(&rustyred_config(&base)).unwrap();

    apply_role_updates(
        &client,
        "role:hn:1",
        &[
            (model::props::OUTREACH_STATUS, json!("drafted")),
            (model::props::GMAIL_DRAFT_ID, json!("draft-99")),
        ],
    )
    .unwrap();

    let reqs = drain(&rx, 2);
    let get = reqs.iter().find(|r| r.method == "GET").unwrap();
    assert!(get.path.contains("/graph/nodes/role:hn:1"));
    assert_eq!(get.auth.as_deref(), Some("Bearer rr-token"));

    let upsert = reqs.iter().find(|r| r.method == "POST").unwrap();
    let body: Value = serde_json::from_str(&upsert.body).unwrap();
    let props = &body["properties"];
    // The new outreach keys are set...
    assert_eq!(props["outreach_status"], "drafted");
    assert_eq!(props["gmail_draft_id"], "draft-99");
    // ...AND the original ingest props are PRESERVED (the whole point).
    assert_eq!(props["title"], "Senior Rust Engineer");
    assert_eq!(props["body"], role_node_json_body());
    assert_eq!(props["embedding"], json!([0.1, 0.2, 0.3]));
    // The Role label is preserved so the node stays queryable as a Role.
    assert_eq!(body["labels"][0], "Role");
}

fn role_node_json_body() -> &'static str {
    "We need someone to build a Rust graph engine with vector search."
}

// ---- 3. Draft flow end-to-end ---------------------------------------------

/// A captured draft: (to, subject, body, thread_id).
type CapturedDraft = (String, String, String, Option<String>);

/// In-process sink that captures the email instead of calling Gmail.
struct FakeSink {
    captured: std::cell::RefCell<Vec<CapturedDraft>>,
}
impl FakeSink {
    fn new() -> Self {
        Self {
            captured: std::cell::RefCell::new(Vec::new()),
        }
    }
}
impl DraftSink for FakeSink {
    fn create_draft(
        &self,
        to: &str,
        subject: &str,
        body: &str,
        thread_id: Option<&str>,
    ) -> error::Result<DraftRef> {
        self.captured.borrow_mut().push((
            to.into(),
            subject.into(),
            body.into(),
            thread_id.map(str::to_string),
        ));
        Ok(DraftRef {
            draft_id: "draft-99".into(),
            thread_id: "thread-7".into(),
        })
    }
}

#[test]
fn draft_top_renders_creates_draft_and_flips_status_to_drafted() {
    // Per lead: context/pack + GET + upsert + event-node + event-edge = 5 requests.
    let (base, rx) = start_mock(5, rustyred_node_responder);
    let client = RustyRedClient::new(&rustyred_config(&base)).unwrap();
    let sink = FakeSink::new();
    let clock = FixedClock::on("2026-06-16");

    let stats = draft_top(&client, &sink, &clock, &profile(), &[hn_lead()]).unwrap();
    assert_eq!(stats.drafted, 1);

    // The sink saw a complete, evidence-led email to the lead.
    let captured = sink.captured.borrow();
    assert_eq!(captured.len(), 1);
    let (to, subject, body, thread) = &captured[0];
    assert_eq!(to, "hire@qdrant.tech");
    assert!(subject.contains("Senior Rust Engineer"));
    assert!(body.contains("Rust graph engine")); // role's own language
    assert!(body.contains("github.com/Travis-Gilbert/Theseus")); // proof block
    assert!(thread.is_none()); // initial draft opens a new thread

    // The Role was read-modify-written to `drafted` with the gmail ids, title kept.
    let reqs = drain(&rx, 5);
    let upsert = reqs
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/graph/nodes"))
        .map(|r| serde_json::from_str::<Value>(&r.body).unwrap())
        .find(|b| b["id"] == "role:hn:1")
        .expect("role upsert present");
    assert_eq!(upsert["properties"]["outreach_status"], "drafted");
    assert_eq!(upsert["properties"]["gmail_draft_id"], "draft-99");
    assert_eq!(upsert["properties"]["gmail_thread_id"], "thread-7");
    assert_eq!(upsert["properties"]["outreach_template"], "hn_founder");
    assert_eq!(upsert["properties"]["title"], "Senior Rust Engineer"); // preserved
}

// ---- 4. Sync: reply detection + sent advancement --------------------------

struct FakeSource {
    senders: Vec<String>,
    live_drafts: Vec<String>,
}
impl ReplySource for FakeSource {
    fn thread_senders(&self, _thread_id: &str) -> error::Result<Vec<String>> {
        Ok(self.senders.clone())
    }
    fn live_draft_ids(&self) -> error::Result<Vec<String>> {
        Ok(self.live_drafts.clone())
    }
}

/// nodes/query returns one drafted lead with a thread + draft id on file.
fn drafted_lead_query_responder(method: &str, path: &str) -> String {
    if path.ends_with("/graph/nodes/query") {
        json!({
            "ok": true,
            "nodes": [{
                "id": "role:hn:1",
                "labels": ["Role"],
                "properties": {
                    "title": "Senior Rust Engineer", "company": "Qdrant",
                    "company_id": "company:qdrant", "source": "hn", "body": "rust graph",
                    "emails": ["hire@qdrant.tech"], "outreach_status": "drafted",
                    "gmail_draft_id": "draft-99", "gmail_thread_id": "thread-7",
                    "outreach_to": "hire@qdrant.tech", "outreach_template": "hn_founder"
                }
            }]
        })
        .to_string()
    } else if method == "GET" && path.contains("/graph/nodes/role") {
        role_node_json()
    } else {
        r#"{"ok":true,"node":{"id":"role:hn:1"}}"#.to_string()
    }
}

#[test]
fn sync_advances_drafted_to_sent_when_draft_left_drafts() {
    // query + GET + upsert + event-node + event-edge = 5 requests.
    let (base, rx) = start_mock(5, drafted_lead_query_responder);
    let client = RustyRedClient::new(&rustyred_config(&base)).unwrap();
    // Draft gone from the live list, thread has the operator's message -> sent.
    let source = FakeSource {
        senders: vec!["me@self.com".into()],
        live_drafts: vec!["draft-other".into()],
    };
    let clock = FixedClock::on("2026-06-16");

    let stats = run_sync(&client, &source, &clock, &[4, 9]).unwrap();
    assert_eq!(stats.advanced_sent, 1);
    assert_eq!(stats.replied, 0);

    let reqs = drain(&rx, 5);
    let upsert = reqs
        .iter()
        .filter(|r| r.method == "POST" && r.path.ends_with("/graph/nodes"))
        .map(|r| serde_json::from_str::<Value>(&r.body).unwrap())
        .find(|b| b["id"] == "role:hn:1")
        .expect("role upsert present");
    assert_eq!(upsert["properties"]["outreach_status"], "sent");
    assert_eq!(upsert["properties"]["outreach_sent_at"], "2026-06-16");
    // First follow-up scheduled at sent + 4 days.
    assert_eq!(upsert["properties"]["next_followup_at"], "2026-06-20");
    assert_eq!(upsert["properties"]["touch_count"], 1);
}

#[test]
fn sync_flips_to_replied_when_lead_address_in_thread() {
    // query + GET + upsert + event-node + event-edge + outcome-node + outcome-edge = 7.
    let (base, _rx) = start_mock(7, drafted_lead_query_responder);
    let client = RustyRedClient::new(&rustyred_config(&base)).unwrap();
    let source = FakeSource {
        senders: vec!["hire@qdrant.tech".into()], // the lead replied
        live_drafts: vec![],
    };
    let clock = FixedClock::on("2026-06-16");

    let stats = run_sync(&client, &source, &clock, &[4, 9]).unwrap();
    assert_eq!(stats.replied, 1);
    assert_eq!(stats.advanced_sent, 0);
}

fn _silence_unused(_: &OutreachStatus, _: &LeadState) {}
