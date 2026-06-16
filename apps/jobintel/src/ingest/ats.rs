//! Public-ATS ingest with auto-detection. For a company slug we try the three
//! boards in order and keep the first that returns a non-empty list:
//!
//!   Greenhouse: GET boards-api.greenhouse.io/v1/boards/{slug}/jobs?content=true
//!   Lever:      GET api.lever.co/v0/postings/{slug}?mode=json
//!   Ashby:      GET api.ashbyhq.com/posting-api/job-board/{slug}?includeCompensation=true
//!
//! Each board's parser is pure and fixture-tested; the probe only sequences the
//! fetches and swallows per-board failures (a 404 just means "not on this ATS").

use reqwest::blocking::Client;
use serde_json::Value;

use super::text;
use crate::error::Result;
use crate::model::{JobRecord, Source};

/// A board fetcher: (client, slug, domain) -> records.
type BoardFetch = fn(&Client, &str, Option<&str>) -> Result<Vec<JobRecord>>;

/// Try every ATS for `slug`; return the first non-empty result. `domain` (from
/// slugs.toml) is stamped onto each record for later Hunter.io contact lookup.
pub fn probe_ats(client: &Client, slug: &str, domain: Option<&str>) -> Vec<JobRecord> {
    let attempts: [(&str, BoardFetch); 3] = [
        ("greenhouse", fetch_greenhouse),
        ("lever", fetch_lever),
        ("ashby", fetch_ashby),
    ];
    for (board, fetch) in attempts {
        match fetch(client, slug, domain) {
            Ok(records) if !records.is_empty() => return records,
            Ok(_) => {}
            Err(err) => eprintln!("  {board}:{slug} probe failed: {err}"),
        }
    }
    Vec::new()
}

fn get_json(client: &Client, url: &str) -> Result<Option<Value>> {
    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        return Ok(None); // 404/410 => company is not on this board.
    }
    Ok(Some(resp.json()?))
}

fn fetch_greenhouse(client: &Client, slug: &str, domain: Option<&str>) -> Result<Vec<JobRecord>> {
    let url = format!("https://boards-api.greenhouse.io/v1/boards/{slug}/jobs?content=true");
    Ok(match get_json(client, &url)? {
        Some(body) => parse_greenhouse(slug, domain, &body),
        None => Vec::new(),
    })
}

fn fetch_lever(client: &Client, slug: &str, domain: Option<&str>) -> Result<Vec<JobRecord>> {
    let url = format!("https://api.lever.co/v0/postings/{slug}?mode=json");
    Ok(match get_json(client, &url)? {
        Some(body) => parse_lever(slug, domain, &body),
        None => Vec::new(),
    })
}

fn fetch_ashby(client: &Client, slug: &str, domain: Option<&str>) -> Result<Vec<JobRecord>> {
    let url =
        format!("https://api.ashbyhq.com/posting-api/job-board/{slug}?includeCompensation=true");
    Ok(match get_json(client, &url)? {
        Some(body) => parse_ashby(slug, domain, &body),
        None => Vec::new(),
    })
}

// ---- pure parsers ----------------------------------------------------------

pub fn parse_greenhouse(slug: &str, domain: Option<&str>, body: &Value) -> Vec<JobRecord> {
    let company = prettify(slug);
    body.get("jobs")
        .and_then(Value::as_array)
        .map(|jobs| {
            jobs.iter()
                .filter_map(|job| {
                    let id = job.get("id")?.to_string();
                    let title = str_field(job, "title")?;
                    let location = job
                        .get("location")
                        .and_then(|l| l.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // content is entity-encoded HTML; decode before stripping.
                    let body_text = job
                        .get("content")
                        .and_then(Value::as_str)
                        .map(|c| text::strip_html(&text::decode_entities(c)))
                        .unwrap_or_default();
                    Some(make_record(
                        Source::Greenhouse,
                        &format!("role:greenhouse:{id}"),
                        &company,
                        domain,
                        title,
                        location,
                        str_field(job, "absolute_url").unwrap_or_default(),
                        body_text,
                        str_field(job, "updated_at"),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_lever(slug: &str, domain: Option<&str>, body: &Value) -> Vec<JobRecord> {
    let company = prettify(slug);
    body.as_array()
        .map(|postings| {
            postings
                .iter()
                .filter_map(|p| {
                    let id = str_field(p, "id")?;
                    let title = str_field(p, "text")?;
                    let cats = p.get("categories");
                    let location = cats
                        .and_then(|c| c.get("location"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let commitment = cats
                        .and_then(|c| c.get("commitment"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let body_text = str_field(p, "descriptionPlain")
                        .or_else(|| str_field(p, "description").map(|h| text::strip_html(&h)))
                        .unwrap_or_default();
                    // Lever exposes commitment (e.g. "Contract") as structured data.
                    let combined = format!("{location} {commitment} {body_text}");
                    Some(make_record_with_flags(
                        Source::Lever,
                        &format!("role:lever:{id}"),
                        &company,
                        domain,
                        title,
                        location,
                        str_field(p, "hostedUrl").unwrap_or_default(),
                        body_text.clone(),
                        None,
                        text::detect_remote(&combined),
                        text::detect_contract(&combined),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub fn parse_ashby(slug: &str, domain: Option<&str>, body: &Value) -> Vec<JobRecord> {
    let company = body
        .get("organizationName")
        .and_then(Value::as_str)
        .map(String::from)
        .unwrap_or_else(|| prettify(slug));
    body.get("jobs")
        .and_then(Value::as_array)
        .map(|jobs| {
            jobs.iter()
                .filter_map(|job| {
                    let id = str_field(job, "id")?;
                    let title = str_field(job, "title")?;
                    let location = str_field(job, "location").unwrap_or_default();
                    let body_text = str_field(job, "descriptionPlain")
                        .or_else(|| str_field(job, "descriptionHtml").map(|h| text::strip_html(&h)))
                        .unwrap_or_default();
                    let employment = str_field(job, "employmentType").unwrap_or_default();
                    let is_remote = job
                        .get("isRemote")
                        .and_then(Value::as_bool)
                        .unwrap_or_else(|| text::detect_remote(&location));
                    let url = str_field(job, "jobUrl")
                        .or_else(|| str_field(job, "applyUrl"))
                        .unwrap_or_default();
                    let combined = format!("{employment} {body_text}");
                    Some(make_record_with_flags(
                        Source::Ashby,
                        &format!("role:ashby:{id}"),
                        &company,
                        domain,
                        title,
                        location,
                        url,
                        body_text.clone(),
                        None,
                        is_remote,
                        text::detect_contract(&combined),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---- record construction ---------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn make_record(
    source: Source,
    id: &str,
    company: &str,
    domain: Option<&str>,
    title: String,
    location: String,
    url: String,
    body: String,
    posted_at: Option<String>,
) -> JobRecord {
    let remote = text::detect_remote(&format!("{location} {body}"));
    let contract = text::detect_contract(&body);
    make_record_with_flags(
        source, id, company, domain, title, location, url, body, posted_at, remote, contract,
    )
}

#[allow(clippy::too_many_arguments)]
fn make_record_with_flags(
    source: Source,
    id: &str,
    company: &str,
    domain: Option<&str>,
    title: String,
    location: String,
    url: String,
    body: String,
    posted_at: Option<String>,
    remote: bool,
    contract: bool,
) -> JobRecord {
    let comp = text::find_comp(&body);
    JobRecord {
        id: id.to_string(),
        source,
        company: company.to_string(),
        company_domain: domain.map(String::from),
        title,
        location,
        remote,
        comp,
        url,
        body,
        posted_at,
        emails: Vec::new(), // ATS payloads carry no contact email; Hunter fills later.
        contract,
        founder_posted: false,
    }
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(String::from)
}

/// "fireworks-ai" -> "Fireworks Ai". Display only; ids use the raw slug.
fn prettify(slug: &str) -> String {
    slug.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn greenhouse_decodes_encoded_content() {
        let body = json!({
            "jobs": [{
                "id": 555,
                "title": "Rust Engineer",
                "absolute_url": "https://boards.greenhouse.io/qdrant/jobs/555",
                "location": { "name": "Remote - EU" },
                "updated_at": "2024-06-01T00:00:00Z",
                "content": "&lt;p&gt;Build a vector DB in Rust. $150k.&lt;/p&gt;"
            }]
        });
        let records = parse_greenhouse("qdrant", Some("qdrant.tech"), &body);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.source, Source::Greenhouse);
        assert_eq!(r.id, "role:greenhouse:555");
        assert_eq!(r.company_domain.as_deref(), Some("qdrant.tech"));
        assert!(r.body.contains("vector DB in Rust"));
        assert!(!r.body.contains("&lt;"), "entities must be decoded");
        assert!(r.remote);
        assert_eq!(r.comp.as_deref(), Some("$150k"));
    }

    #[test]
    fn lever_reads_array_and_commitment() {
        let body = json!([{
            "id": "abc-123",
            "text": "Contract Backend Engineer",
            "categories": { "location": "Remote", "commitment": "Contract" },
            "hostedUrl": "https://jobs.lever.co/acme/abc-123",
            "descriptionPlain": "Join us building infra."
        }]);
        let records = parse_lever("acme", None, &body);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.source, Source::Lever);
        assert!(r.remote);
        assert!(
            r.contract,
            "Lever commitment=Contract must set contract flag"
        );
    }

    #[test]
    fn ashby_reads_jobs_and_remote_flag() {
        let body = json!({
            "organizationName": "Letta",
            "jobs": [{
                "id": "job_9",
                "title": "Founding Engineer",
                "location": "San Francisco",
                "employmentType": "FullTime",
                "isRemote": false,
                "jobUrl": "https://jobs.ashbyhq.com/letta/job_9",
                "descriptionPlain": "Agents and memory."
            }]
        });
        let records = parse_ashby("letta", Some("letta.com"), &body);
        assert_eq!(records.len(), 1);
        let r = &records[0];
        assert_eq!(r.company, "Letta");
        assert!(!r.remote);
        assert_eq!(r.company_domain.as_deref(), Some("letta.com"));
    }

    #[test]
    fn prettify_titlecases_slug() {
        assert_eq!(prettify("fireworks-ai"), "Fireworks Ai");
        assert_eq!(prettify("qdrant"), "Qdrant");
    }
}
