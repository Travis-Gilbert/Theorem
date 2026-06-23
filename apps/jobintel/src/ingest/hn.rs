//! Hacker News "Who is Hiring" ingest via the HN Algolia API (no auth).
//!
//!   thread discovery: GET /search?tags=story,author_whoishiring  -> newest id
//!   posts:            GET /search?tags=comment,story_{id}&hitsPerPage=1000
//!                     keep hits where parent_id == thread_id (top-level posts)
//!
//! The pure parser `parse_hn_hits` is unit-tested against a fixture; the HTTP
//! wrappers only fetch and delegate.

use reqwest::blocking::Client;
use serde_json::Value;

use super::text;
use crate::error::{JobIntelError, Result};
use crate::model::{JobRecord, Source};

const ALGOLIA: &str = "https://hn.algolia.com/api/v1/search";
/// Date-sorted endpoint. `/search` ranks by relevance/points (which surfaces an
/// old, highly-upvoted thread); `/search_by_date` returns newest-first, which is
/// what "the newest 'Who is hiring' story" actually requires.
const ALGOLIA_BY_DATE: &str = "https://hn.algolia.com/api/v1/search_by_date";
const MAX_BODY_CHARS: usize = 8000;

/// Find the newest "Who is hiring" thread id authored by `whoishiring`.
pub fn discover_thread(client: &Client) -> Result<u64> {
    let url = format!("{ALGOLIA_BY_DATE}?tags=story,author_whoishiring&hitsPerPage=20");
    let body: Value = client.get(url).send()?.error_for_status()?.json()?;
    let hits = body
        .get("hits")
        .and_then(Value::as_array)
        .ok_or_else(|| JobIntelError::Ingest("HN story search returned no hits".into()))?;
    // Hits are newest-first; take the first whose title says "who is hiring".
    for hit in hits {
        let title = hit.get("title").and_then(Value::as_str).unwrap_or_default();
        if title.to_lowercase().contains("who is hiring") {
            if let Some(id) = hit
                .get("objectID")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u64>().ok())
            {
                return Ok(id);
            }
        }
    }
    Err(JobIntelError::Ingest(
        "no 'Who is hiring' story found in author_whoishiring feed".into(),
    ))
}

/// Fetch and parse the top-level job posts of a thread.
pub fn fetch_hn(client: &Client, thread_id: u64) -> Result<Vec<JobRecord>> {
    let url = format!("{ALGOLIA}?tags=comment,story_{thread_id}&hitsPerPage=1000");
    let body: Value = client.get(url).send()?.error_for_status()?.json()?;
    let hits = body
        .get("hits")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(parse_hn_hits(&hits, thread_id))
}

/// Pure: turn raw Algolia hits into JobRecords, keeping only top-level posts.
pub fn parse_hn_hits(hits: &[Value], thread_id: u64) -> Vec<JobRecord> {
    hits.iter()
        .filter(|hit| is_top_level(hit, thread_id))
        .filter_map(hit_to_record)
        .collect()
}

fn is_top_level(hit: &Value, thread_id: u64) -> bool {
    hit.get("parent_id")
        .and_then(Value::as_u64)
        .map(|p| p == thread_id)
        .unwrap_or(false)
}

fn hit_to_record(hit: &Value) -> Option<JobRecord> {
    let object_id = hit.get("objectID").and_then(Value::as_str)?;
    let raw = hit
        .get("comment_text")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if raw.trim().is_empty() {
        return None;
    }
    let body_full = text::strip_html(raw);
    // Emails can hide in the visible text OR in stripped `mailto:` hrefs, so
    // scan both the rendered body and the raw HTML and union the results.
    let mut emails = text::extract_emails(&body_full);
    for e in text::extract_emails(raw) {
        if !emails.contains(&e) {
            emails.push(e);
        }
    }

    let (company, title, location) = parse_header(&body_full);
    let body = truncate_chars(&body_full, MAX_BODY_CHARS);

    Some(JobRecord {
        id: format!("role:hn:{object_id}"),
        source: Source::Hn,
        company,
        company_domain: None,
        title,
        location,
        remote: text::detect_remote(&body_full),
        comp: text::find_comp(&body_full),
        url: format!("https://news.ycombinator.com/item?id={object_id}"),
        body,
        posted_at: hit
            .get("created_at")
            .and_then(Value::as_str)
            .map(String::from),
        emails,
        contract: text::detect_contract(&body_full),
        founder_posted: text::detect_founder(&body_full),
    })
}

/// Parse the conventional HN header line: `Company | Role | Location | ...`.
/// Falls back to en/em-dash separators, then to a best-effort single field.
fn parse_header(body: &str) -> (String, String, String) {
    let first = body.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let parts: Vec<String> = split_header(first)
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    let company = parts
        .first()
        .map(|c| clean_company(c))
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| "Unknown".to_string());
    let title = parts
        .get(1)
        .map(|t| truncate_chars(t, 120))
        .unwrap_or_else(|| "(see post)".to_string());
    let location = parts
        .iter()
        .skip(2)
        .find(|p| looks_like_location(p))
        .map(|l| truncate_chars(l, 80))
        .unwrap_or_default();

    (company, title, location)
}

fn split_header(line: &str) -> Vec<String> {
    if line.contains('|') {
        line.split('|').map(String::from).collect()
    } else if line.contains('\u{2014}') {
        line.split('\u{2014}').map(String::from).collect()
    } else if line.contains(" - ") {
        line.split(" - ").map(String::from).collect()
    } else {
        vec![line.to_string()]
    }
}

fn clean_company(raw: &str) -> String {
    // Drop a trailing "(YC S21)" style tag and any parenthetical, cap length.
    let without_paren = raw.split('(').next().unwrap_or(raw).trim();
    truncate_chars(without_paren, 80)
}

fn looks_like_location(part: &str) -> bool {
    let lower = part.to_lowercase();
    part.contains(',')
        || [
            "remote",
            "sf",
            "san francisco",
            "nyc",
            "new york",
            "london",
            "berlin",
            "us",
            "usa",
            "eu",
            "europe",
            "onsite",
            "hybrid",
            "anywhere",
        ]
        .iter()
        .any(|k| lower.contains(k))
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn fixture() -> Value {
        json!({
            "hits": [
                {
                    "objectID": "40001",
                    "parent_id": 40000,
                    "author": "founder1",
                    "created_at": "2024-06-03T12:00:00Z",
                    "comment_text": "<p>Qdrant (YC S21) | Senior Rust Engineer | Remote (EU) | $140k-$180k</p><p>We build a vector database in Rust. I'm the founder. Email jobs&#x2F;qdrant: hiring@qdrant.tech</p>"
                },
                {
                    "objectID": "40002",
                    "parent_id": 40000,
                    "author": "recruiter2",
                    "created_at": "2024-06-03T13:00:00Z",
                    "comment_text": "<p>BigCorp | Backend Engineer | New York, NY | Onsite only</p><p>No remote. Full-time only.</p>"
                },
                {
                    "objectID": "40003",
                    "parent_id": 99999,
                    "comment_text": "<p>This is a reply, not a top-level post.</p>"
                }
            ]
        })
    }

    #[test]
    fn keeps_only_top_level_posts() {
        let hits = fixture()["hits"].as_array().unwrap().clone();
        let records = parse_hn_hits(&hits, 40000);
        assert_eq!(records.len(), 2);
        assert!(records.iter().all(|r| r.source == Source::Hn));
    }

    #[test]
    fn parses_header_flags_and_email() {
        let hits = fixture()["hits"].as_array().unwrap().clone();
        let records = parse_hn_hits(&hits, 40000);
        let qdrant = &records[0];
        assert_eq!(qdrant.company, "Qdrant");
        assert_eq!(qdrant.title, "Senior Rust Engineer");
        assert!(qdrant.remote);
        assert!(qdrant.founder_posted);
        assert_eq!(qdrant.emails, vec!["hiring@qdrant.tech"]);
        assert_eq!(qdrant.comp.as_deref(), Some("$140k-$180k"));

        let bigcorp = &records[1];
        assert!(!bigcorp.remote, "onsite-only post must not be remote");
        assert!(!bigcorp.founder_posted);
        assert!(bigcorp.emails.is_empty());
    }
}
