//! Live REST source transports (Layer A6, production pullers).
//!
//! One injected HTTP seam ([`HttpRequest`] + [`HttpFetch`]) carries all five
//! curated sources: GSuite (Drive), Gmail, Outlook (Microsoft Graph), Notion,
//! and Linear (GraphQL). Each transport builds its service's request from the
//! [`SourceScope`] + cursor, then normalizes the native response into the flat
//! record shape its spoke ([`crate::sources`]) expects. The HTTP call is injected
//! so query-build + response-parse is fixture-tested and only a real token swap
//! reaches the network (each transport has a `with_token` real path and an
//! `#[ignore]` live smoke).
//!
//! Scope: content bodies are not exported (Gmail uses `snippet`, Drive/Notion are
//! metadata), so an item files by title + kind + embedding; full-content fetch
//! (gdoc/file export, Notion blocks, message bodies) is a documented follow-up.

use serde_json::{json, Map, Value};

use crate::spoke::{SourceCursor, SourceError, SourcePage, SourceRecord, SourceResult, SourceScope};
use crate::transport::RecordTransport;

// ---- the shared HTTP seam ---------------------------------------------------

/// One HTTP request. The transports build these; [`ureq_fetch`] (or a test
/// closure) executes them.
pub struct HttpRequest {
    pub method: &'static str,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Option<String>,
}

impl HttpRequest {
    pub fn get(url: String, headers: Vec<(String, String)>) -> Self {
        Self {
            method: "GET",
            url,
            headers,
            body: None,
        }
    }

    pub fn post(url: String, headers: Vec<(String, String)>, body: String) -> Self {
        Self {
            method: "POST",
            url,
            headers,
            body: Some(body),
        }
    }
}

/// The HTTP boundary: execute a request, return the response body. Injected so
/// transports test over recorded responses without a network call.
pub type HttpFetch = dyn Fn(&HttpRequest) -> SourceResult<String>;

/// The real network client (the only OS/network-touching code). Maps Google/
/// Microsoft/Notion/Linear auth + rate-limit statuses onto [`SourceError`].
pub fn ureq_fetch(req: &HttpRequest) -> SourceResult<String> {
    // A global deadline so a stalled upstream cannot block sync_source forever.
    let agent = ureq::Agent::new_with_config(
        ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .build(),
    );
    let mut response = if req.method == "POST" {
        let mut builder = agent.post(&req.url);
        for (key, value) in &req.headers {
            builder = builder.header(key, value);
        }
        builder.send(req.body.as_deref().unwrap_or("").as_bytes())
    } else {
        let mut builder = agent.get(&req.url);
        for (key, value) in &req.headers {
            builder = builder.header(key, value);
        }
        builder.call()
    }
    .map_err(map_ureq_err)?;
    response
        .body_mut()
        .read_to_string()
        .map_err(|e| SourceError::Transport(format!("http body: {e}")))
}

fn map_ureq_err(error: ureq::Error) -> SourceError {
    match error {
        ureq::Error::StatusCode(401) | ureq::Error::StatusCode(403) => {
            SourceError::Auth(format!("http {error}"))
        }
        ureq::Error::StatusCode(429) => SourceError::RateLimit(format!("http {error}")),
        other => SourceError::Transport(format!("http: {other}")),
    }
}

fn bearer(token: &str) -> (String, String) {
    ("Authorization".to_string(), format!("Bearer {token}"))
}

/// Percent-encode a query value per RFC 3986 (unreserved chars pass through).
fn pct(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn encode_url(base: &str, params: &[(String, String)]) -> String {
    let query = params
        .iter()
        .map(|(key, value)| format!("{}={}", pct(key), pct(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

fn json_array(value: &Value, path: &[&str]) -> Vec<Value> {
    let mut node = value;
    for segment in path {
        match node.get(segment) {
            Some(next) => node = next,
            None => return Vec::new(),
        }
    }
    node.as_array().cloned().unwrap_or_default()
}

fn str_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut node = value;
    for segment in path {
        node = node.get(segment)?;
    }
    node.as_str().map(str::to_string)
}

// ---- date helpers (no date dependency) --------------------------------------

/// Epoch ms -> RFC 3339 UTC, for `modifiedTime`/`receivedDateTime`/`updatedAt`
/// filters. Civil date from days via Howard Hinnant's algorithm.
fn epoch_ms_to_rfc3339(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (hour, minute, second) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let yoe = year - era * 400;
    let mp = if month > 2 { month - 3 } else { month + 9 } as i64;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parse a leading `YYYY-MM-DD` (e.g. Linear's `dueDate`) to epoch ms.
fn parse_iso_date_to_ms(value: &str) -> Option<i64> {
    let date = value.get(0..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    Some(days_from_civil(year, month, day) * 86_400 * 1000)
}

/// Parse a leading `YYYY-MM-DD[THH:MM:SS...]` (RFC 3339, fractional seconds and
/// offset tolerated by truncation; treated as UTC) to epoch ms. Used to stamp a
/// record's source instant so the driver can advance the high-water cursor.
fn parse_rfc3339_to_ms(value: &str) -> Option<i64> {
    let date_ms = parse_iso_date_to_ms(value)?;
    let bytes = value.as_bytes();
    if value.len() >= 19 && (bytes[10] == b'T' || bytes[10] == b' ') {
        let hour: i64 = value.get(11..13)?.parse().ok()?;
        let minute: i64 = value.get(14..16)?.parse().ok()?;
        let second: i64 = value.get(17..19)?.parse().ok()?;
        return Some(date_ms + (hour * 3600 + minute * 60 + second) * 1000);
    }
    Some(date_ms)
}

/// The effective `since` bound for a fetch: the caller's `scope.since_ms`, or the
/// cursor's high-water mark when starting a fresh sync (empty page token). Within
/// pagination the token carries position, so the cursor high-water is not
/// re-applied as a filter.
fn effective_since(scope: &SourceScope, cursor: &SourceCursor) -> Option<i64> {
    scope.since_ms.or({
        if cursor.token.is_empty() && cursor.updated_at_ms > 0 {
            Some(cursor.updated_at_ms)
        } else {
            None
        }
    })
}

fn page_size_or(scope: &SourceScope, default: u32, cap: u32) -> u32 {
    scope.max_records.map(|m| m.clamp(1, cap)).unwrap_or(default)
}

// ---- GSuite (Google Drive) --------------------------------------------------

const DRIVE_V3: &str = "https://www.googleapis.com/drive/v3";

/// Drive `files.list`. Auth: OAuth Bearer. Scope: folder ids -> `parents`,
/// `since_ms` -> `modifiedTime`. Cursor: `nextPageToken`.
pub struct GSuiteDriveTransport {
    token: String,
    base_url: String,
    http: Box<HttpFetch>,
}

impl GSuiteDriveTransport {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_http(token, DRIVE_V3, ureq_fetch)
    }

    pub fn with_http<F>(token: impl Into<String>, base_url: impl Into<String>, http: F) -> Self
    where
        F: Fn(&HttpRequest) -> SourceResult<String> + 'static,
    {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            http: Box::new(http),
        }
    }
}

fn gsuite_params(scope: &SourceScope, cursor: &SourceCursor) -> Vec<(String, String)> {
    let mut clauses = vec!["trashed = false".to_string()];
    if !scope.containers.is_empty() {
        let parents = scope
            .containers
            .iter()
            .map(|folder| format!("'{}' in parents", folder.replace('\'', "\\'")))
            .collect::<Vec<_>>()
            .join(" or ");
        clauses.push(format!("({parents})"));
    }
    if let Some(since) = effective_since(scope, cursor) {
        clauses.push(format!("modifiedTime > '{}'", epoch_ms_to_rfc3339(since)));
    }
    for filter in &scope.filters {
        clauses.push(filter.clone());
    }
    let mut params = vec![
        ("q".to_string(), clauses.join(" and ")),
        (
            "fields".to_string(),
            "nextPageToken, files(id, name, mimeType, modifiedTime, parents)".to_string(),
        ),
        ("pageSize".to_string(), page_size_or(scope, 100, 1000).to_string()),
    ];
    if !cursor.token.is_empty() {
        params.push(("pageToken".to_string(), cursor.token.clone()));
    }
    params
}

impl RecordTransport for GSuiteDriveTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let url = encode_url(&format!("{}/files", self.base_url), &gsuite_params(scope, cursor));
        let body = (self.http)(&HttpRequest::get(url, vec![bearer(&self.token)]))?;
        let value: Value = parse_json(&body)?;
        let next = str_at(&value, &["nextPageToken"]).unwrap_or_default();
        let mut records = Vec::new();
        for file in json_array(&value, &["files"]) {
            let id = require_id(&file, "drive file")?;
            let fetched = str_at(&file, &["modifiedTime"])
                .and_then(|t| parse_rfc3339_to_ms(&t))
                .unwrap_or(0);
            records.push(SourceRecord::new(id, file, fetched));
        }
        Ok(page(records, next, cursor))
    }
}

// ---- Gmail ------------------------------------------------------------------

const GMAIL_V1: &str = "https://gmail.googleapis.com/gmail/v1";

/// Gmail. Auth: OAuth Bearer. Scope: labels -> `labelIds`, `since_ms` ->
/// `after:`, `filters` -> search operators. Cursor: list `nextPageToken`. A
/// two-call pull (list ids, then a metadata get per id); the body is the
/// message `snippet`.
pub struct GmailHttpTransport {
    token: String,
    base_url: String,
    http: Box<HttpFetch>,
}

impl GmailHttpTransport {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_http(token, GMAIL_V1, ureq_fetch)
    }

    pub fn with_http<F>(token: impl Into<String>, base_url: impl Into<String>, http: F) -> Self
    where
        F: Fn(&HttpRequest) -> SourceResult<String> + 'static,
    {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            http: Box::new(http),
        }
    }
}

impl RecordTransport for GmailHttpTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let mut params = vec![("maxResults".to_string(), page_size_or(scope, 100, 500).to_string())];
        let mut q = scope.filters.join(" ");
        if let Some(since) = effective_since(scope, cursor) {
            q = format!("{q} after:{}", since.div_euclid(1000)).trim().to_string();
        }
        if !q.trim().is_empty() {
            params.push(("q".to_string(), q));
        }
        for label in &scope.containers {
            params.push(("labelIds".to_string(), label.clone()));
        }
        if !cursor.token.is_empty() {
            params.push(("pageToken".to_string(), cursor.token.clone()));
        }
        let list_url = encode_url(&format!("{}/users/me/messages", self.base_url), &params);
        let list: Value = parse_json(&(self.http)(&HttpRequest::get(list_url, vec![bearer(&self.token)]))?)?;
        let next = str_at(&list, &["nextPageToken"]).unwrap_or_default();

        let mut records = Vec::new();
        for entry in json_array(&list, &["messages"]) {
            let id = require_id(&entry, "gmail message")?;
            let get_url = encode_url(
                &format!("{}/users/me/messages/{id}", self.base_url),
                &[
                    ("format".to_string(), "metadata".to_string()),
                    ("metadataHeaders".to_string(), "Subject".to_string()),
                ],
            );
            let msg: Value = parse_json(&(self.http)(&HttpRequest::get(get_url, vec![bearer(&self.token)]))?)?;
            let subject = json_array(&msg, &["payload", "headers"])
                .into_iter()
                .find(|h| str_at(h, &["name"]).as_deref() == Some("Subject"))
                .and_then(|h| str_at(&h, &["value"]))
                .unwrap_or_else(|| "(no subject)".into());
            let snippet = str_at(&msg, &["snippet"]).unwrap_or_default();
            let label_ids = msg.get("labelIds").cloned().unwrap_or_else(|| json!([]));
            // Gmail `internalDate` is epoch ms as a string.
            let fetched = str_at(&msg, &["internalDate"])
                .and_then(|d| d.parse::<i64>().ok())
                .unwrap_or(0);
            let raw = json!({
                "id": id,
                "subject": subject,
                "snippet": snippet,
                "body": snippet,
                "labelIds": label_ids,
            });
            records.push(SourceRecord::new(id, raw, fetched));
        }
        Ok(page(records, next, cursor))
    }
}

// ---- Outlook (Microsoft Graph) ----------------------------------------------

const GRAPH_V1: &str = "https://graph.microsoft.com/v1.0";

/// Outlook. Auth: Microsoft Graph Bearer. Scope: first container -> mail folder,
/// `since_ms` -> `receivedDateTime ge`. Cursor: the `@odata.nextLink` full URL.
pub struct OutlookHttpTransport {
    token: String,
    base_url: String,
    http: Box<HttpFetch>,
}

impl OutlookHttpTransport {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_http(token, GRAPH_V1, ureq_fetch)
    }

    pub fn with_http<F>(token: impl Into<String>, base_url: impl Into<String>, http: F) -> Self
    where
        F: Fn(&HttpRequest) -> SourceResult<String> + 'static,
    {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            http: Box::new(http),
        }
    }
}

impl RecordTransport for OutlookHttpTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        // `@odata.nextLink` is a full URL: follow it verbatim.
        let url = if cursor.token.starts_with("http") {
            cursor.token.clone()
        } else {
            let path = match scope.containers.first() {
                Some(folder) => format!("{}/me/mailFolders/{folder}/messages", self.base_url),
                None => format!("{}/me/messages", self.base_url),
            };
            let mut filters = Vec::new();
            if let Some(since) = effective_since(scope, cursor) {
                filters.push(format!("receivedDateTime ge {}", epoch_ms_to_rfc3339(since)));
            }
            filters.extend(scope.filters.iter().cloned());
            let mut params = vec![
                (
                    "$select".to_string(),
                    "id,subject,bodyPreview,parentFolderId,receivedDateTime".to_string(),
                ),
                ("$top".to_string(), page_size_or(scope, 50, 1000).to_string()),
            ];
            if !filters.is_empty() {
                params.push(("$filter".to_string(), filters.join(" and ")));
            }
            encode_url(&path, &params)
        };
        let value: Value = parse_json(&(self.http)(&HttpRequest::get(url, vec![bearer(&self.token)]))?)?;
        let next = str_at(&value, &["@odata.nextLink"]).unwrap_or_default();
        let mut records = Vec::new();
        for message in json_array(&value, &["value"]) {
            let id = require_id(&message, "outlook message")?;
            let fetched = str_at(&message, &["receivedDateTime"])
                .and_then(|t| parse_rfc3339_to_ms(&t))
                .unwrap_or(0);
            records.push(SourceRecord::new(id, message, fetched));
        }
        Ok(page(records, next, cursor))
    }
}

// ---- Notion -----------------------------------------------------------------

const NOTION_API: &str = "https://api.notion.com";
const NOTION_VERSION: &str = "2022-06-28";

/// Notion. Auth: integration Bearer + `Notion-Version`. Scope: the first
/// container is the database id, `since_ms` -> a `last_edited_time` filter.
/// Cursor: `next_cursor`. (Query is per-database; a multi-database scope is a
/// documented follow-up.)
pub struct NotionHttpTransport {
    token: String,
    base_url: String,
    http: Box<HttpFetch>,
}

impl NotionHttpTransport {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_http(token, NOTION_API, ureq_fetch)
    }

    pub fn with_http<F>(token: impl Into<String>, base_url: impl Into<String>, http: F) -> Self
    where
        F: Fn(&HttpRequest) -> SourceResult<String> + 'static,
    {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            http: Box::new(http),
        }
    }
}

impl RecordTransport for NotionHttpTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let database = scope.containers.first().ok_or_else(|| {
            SourceError::Mapping("notion requires a database id in scope.containers".into())
        })?;
        let url = format!("{}/v1/databases/{database}/query", self.base_url);

        let mut body = Map::new();
        body.insert(
            "page_size".to_string(),
            json!(page_size_or(scope, 100, 100)),
        );
        if !cursor.token.is_empty() {
            body.insert("start_cursor".to_string(), json!(cursor.token));
        }
        if let Some(since) = effective_since(scope, cursor) {
            body.insert(
                "filter".to_string(),
                json!({
                    "timestamp": "last_edited_time",
                    "last_edited_time": { "on_or_after": epoch_ms_to_rfc3339(since) }
                }),
            );
        }
        let req = HttpRequest::post(
            url,
            vec![
                bearer(&self.token),
                ("Notion-Version".to_string(), NOTION_VERSION.to_string()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            Value::Object(body).to_string(),
        );
        let value: Value = parse_json(&(self.http)(&req)?)?;
        let next = str_at(&value, &["next_cursor"]).unwrap_or_default();
        let has_more = value.get("has_more").and_then(Value::as_bool).unwrap_or(false);

        let mut records = Vec::new();
        for page_obj in json_array(&value, &["results"]) {
            let id = require_id(&page_obj, "notion page")?;
            let title = notion_title(&page_obj).unwrap_or_else(|| "(untitled)".into());
            let fetched = str_at(&page_obj, &["last_edited_time"])
                .and_then(|t| parse_rfc3339_to_ms(&t))
                .unwrap_or(0);
            let raw = json!({ "id": id, "title": title, "text": "", "database_id": database });
            records.push(SourceRecord::new(id, raw, fetched));
        }
        // Notion paginates by `has_more`, not an empty cursor.
        Ok(SourcePage {
            records,
            next: SourceCursor {
                token: next,
                updated_at_ms: cursor.updated_at_ms,
            },
            exhausted: !has_more,
        })
    }
}

/// The plain text of a Notion page's `title`-typed property.
fn notion_title(page: &Value) -> Option<String> {
    let properties = page.get("properties")?.as_object()?;
    for value in properties.values() {
        if let Some(parts) = value.get("title").and_then(Value::as_array) {
            let text = parts
                .iter()
                .filter_map(|p| str_at(p, &["plain_text"]))
                .collect::<String>();
            if !text.is_empty() {
                return Some(text);
            }
        }
    }
    None
}

// ---- Linear (GraphQL) -------------------------------------------------------

const LINEAR_API: &str = "https://api.linear.app";
const LINEAR_QUERY: &str = "query($first:Int!,$after:String,$filter:IssueFilter){issues(first:$first,after:$after,filter:$filter){nodes{id title description priority dueDate updatedAt state{name} team{id}} pageInfo{hasNextPage endCursor}}}";

/// Linear. Auth: API key in `Authorization` (raw, not Bearer). Scope: teams ->
/// an `IssueFilter`, `since_ms` -> `updatedAt.gte`. Cursor: `pageInfo.endCursor`.
/// An issue maps to a `Task` with state/priority/due on the scalars.
pub struct LinearHttpTransport {
    token: String,
    base_url: String,
    http: Box<HttpFetch>,
}

impl LinearHttpTransport {
    pub fn with_token(token: impl Into<String>) -> Self {
        Self::with_http(token, LINEAR_API, ureq_fetch)
    }

    pub fn with_http<F>(token: impl Into<String>, base_url: impl Into<String>, http: F) -> Self
    where
        F: Fn(&HttpRequest) -> SourceResult<String> + 'static,
    {
        Self {
            token: token.into(),
            base_url: base_url.into(),
            http: Box::new(http),
        }
    }
}

impl RecordTransport for LinearHttpTransport {
    fn fetch(&self, scope: &SourceScope, cursor: &SourceCursor) -> SourceResult<SourcePage> {
        let mut filter = Map::new();
        if !scope.containers.is_empty() {
            filter.insert("team".to_string(), json!({ "id": { "in": scope.containers } }));
        }
        if let Some(since) = effective_since(scope, cursor) {
            filter.insert("updatedAt".to_string(), json!({ "gte": epoch_ms_to_rfc3339(since) }));
        }
        let variables = json!({
            "first": page_size_or(scope, 50, 100),
            "after": if cursor.token.is_empty() { Value::Null } else { json!(cursor.token) },
            "filter": Value::Object(filter),
        });
        let req = HttpRequest::post(
            format!("{}/graphql", self.base_url),
            vec![
                ("Authorization".to_string(), self.token.clone()),
                ("Content-Type".to_string(), "application/json".to_string()),
            ],
            json!({ "query": LINEAR_QUERY, "variables": variables }).to_string(),
        );
        let value: Value = parse_json(&(self.http)(&req)?)?;
        if let Some(errors) = value.get("errors") {
            return Err(SourceError::Transport(format!("linear graphql: {errors}")));
        }
        let issues = value.get("data").and_then(|d| d.get("issues")).cloned().unwrap_or(Value::Null);
        let next = str_at(&issues, &["pageInfo", "endCursor"]).unwrap_or_default();
        let has_next = issues
            .get("pageInfo")
            .and_then(|p| p.get("hasNextPage"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let mut records = Vec::new();
        for node in json_array(&issues, &["nodes"]) {
            let id = require_id(&node, "linear issue")?;
            let due_at_ms = str_at(&node, &["dueDate"]).and_then(|d| parse_iso_date_to_ms(&d));
            let fetched = str_at(&node, &["updatedAt"])
                .and_then(|t| parse_rfc3339_to_ms(&t))
                .unwrap_or(0);
            let raw = json!({
                "id": id,
                "title": node.get("title").cloned().unwrap_or(Value::Null),
                "description": node.get("description").cloned().unwrap_or(Value::Null),
                "state": str_at(&node, &["state", "name"]),
                "priority": node.get("priority").cloned().unwrap_or(Value::Null),
                "dueAtMs": due_at_ms,
                "teamId": str_at(&node, &["team", "id"]),
            });
            records.push(SourceRecord::new(id, raw, fetched));
        }
        Ok(SourcePage {
            records,
            next: SourceCursor {
                token: next,
                updated_at_ms: cursor.updated_at_ms,
            },
            exhausted: !has_next,
        })
    }
}

// ---- shared parsing ---------------------------------------------------------

fn parse_json(body: &str) -> SourceResult<Value> {
    serde_json::from_str(body).map_err(|e| SourceError::Transport(format!("response not JSON: {e}")))
}

fn require_id(value: &Value, what: &str) -> SourceResult<String> {
    str_at(value, &["id"]).ok_or_else(|| SourceError::Mapping(format!("{what} missing `id`")))
}

/// A single-call source page: exhausted when there is no next token.
fn page(records: Vec<SourceRecord>, next: String, cursor: &SourceCursor) -> SourcePage {
    let exhausted = next.is_empty();
    SourcePage {
        records,
        next: SourceCursor {
            token: next,
            updated_at_ms: cursor.updated_at_ms,
        },
        exhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gsuite_query_encodes_folders_since_and_paging() {
        let scope = SourceScope::containers(["folder-A", "folder-B"])
            .since(1_700_000_000_000)
            .max_records(25);
        let cursor = SourceCursor {
            token: "next".into(),
            updated_at_ms: 0,
        };
        let params = gsuite_params(&scope, &cursor);
        let q = &params.iter().find(|(k, _)| k == "q").unwrap().1;
        assert!(q.contains("trashed = false"));
        assert!(q.contains("'folder-A' in parents") && q.contains("'folder-B' in parents"));
        assert!(q.contains("modifiedTime > '2023-11-14T22:13:20Z'"));
        assert_eq!(params.iter().find(|(k, _)| k == "pageSize").unwrap().1, "25");
    }

    #[test]
    fn dates_round_trip() {
        assert_eq!(epoch_ms_to_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(epoch_ms_to_rfc3339(1_700_000_000_000), "2023-11-14T22:13:20Z");
        // Parse a date back and confirm it formats to the same day at midnight.
        let ms = parse_iso_date_to_ms("2023-11-14").unwrap();
        assert_eq!(epoch_ms_to_rfc3339(ms), "2023-11-14T00:00:00Z");
        assert!(parse_iso_date_to_ms("not-a-date").is_none());
        // RFC3339 datetime round-trips through the time component too.
        let dt = parse_rfc3339_to_ms("2024-06-01T12:00:00Z").unwrap();
        assert_eq!(epoch_ms_to_rfc3339(dt), "2024-06-01T12:00:00Z");
    }

    #[test]
    fn effective_since_uses_cursor_high_water_only_on_a_fresh_start() {
        let scope = SourceScope::default();
        // Fresh start (empty token) consumes the cursor high-water.
        let fresh = SourceCursor { token: String::new(), updated_at_ms: 123 };
        assert_eq!(effective_since(&scope, &fresh), Some(123));
        // Mid-pagination (non-empty token) does not re-apply it.
        let paging = SourceCursor { token: "p2".into(), updated_at_ms: 123 };
        assert_eq!(effective_since(&scope, &paging), None);
        // An explicit scope.since_ms always wins.
        let scoped = SourceScope::default().since(999);
        assert_eq!(effective_since(&scoped, &paging), Some(999));
    }

    #[test]
    fn notion_title_reads_the_title_property() {
        let page = json!({
            "properties": {
                "Name": { "title": [{"plain_text":"Hello "},{"plain_text":"World"}] },
                "Tags": { "multi_select": [] }
            }
        });
        assert_eq!(notion_title(&page).as_deref(), Some("Hello World"));
    }
}
