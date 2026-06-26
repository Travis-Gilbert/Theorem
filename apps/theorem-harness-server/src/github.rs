//! GitHub App webhook ingestion.
//!
//! The webhook edge verifies GitHub's raw-body signature, deduplicates delivery
//! ids, submits push events to the existing code-index reindex worker, and writes
//! collaboration objects into the same code graph.

use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use hmac::{Hmac, Mac};
use rustyred_thg_code::{
    CodeIndexRuntime, IngestCodebaseInput, IngestJobRequest, RepoFetchCaps, CODE_SYMBOL_LABEL,
};
use rustyred_thg_core::{stable_hash, EdgeRecord, GraphMutation, NodeQuery, NodeRecord};
use serde_json::{json, Value};
use sha2::Sha256;

use crate::github_app::{GithubApp, GithubPullRequestFile};

type HmacSha256 = Hmac<Sha256>;

const DELIVERY_CACHE_LIMIT: usize = 512;
const SOURCE: &str = "github_app_webhook";

#[derive(Clone)]
pub struct GithubWebhookState {
    pub app: Arc<GithubApp>,
    pub code_index: CodeIndexRuntime,
    pub tenant_slug: String,
    deliveries: Arc<Mutex<DeliveryDeduper>>,
}

impl GithubWebhookState {
    pub fn new(
        app: Arc<GithubApp>,
        code_index: CodeIndexRuntime,
        tenant_slug: impl Into<String>,
    ) -> Self {
        Self {
            app,
            code_index,
            tenant_slug: tenant_slug.into(),
            deliveries: Arc::new(Mutex::new(DeliveryDeduper::new(DELIVERY_CACHE_LIMIT))),
        }
    }
}

#[derive(Debug)]
struct DeliveryDeduper {
    limit: usize,
    seen: HashSet<String>,
    order: VecDeque<String>,
}

impl DeliveryDeduper {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            seen: HashSet::new(),
            order: VecDeque::new(),
        }
    }

    fn accept(&mut self, delivery_id: &str) -> bool {
        if delivery_id.trim().is_empty() {
            return false;
        }
        if self.seen.contains(delivery_id) {
            return false;
        }
        self.seen.insert(delivery_id.to_string());
        self.order.push_back(delivery_id.to_string());
        while self.order.len() > self.limit {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        true
    }
}

pub fn github_router(state: GithubWebhookState) -> Router {
    Router::new()
        .route("/github/webhook", post(post_github_webhook))
        .route("/github/webhooks", post(post_github_webhook))
        .with_state(state)
}

async fn post_github_webhook(
    State(state): State<GithubWebhookState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, Json<Value>), (StatusCode, String)> {
    let signature = header_str(&headers, "X-Hub-Signature-256")
        .ok_or_else(|| (StatusCode::UNAUTHORIZED, "missing signature".to_string()))?;
    if !verify_webhook_signature(state.app.webhook_secret(), &body, signature) {
        return Err((StatusCode::UNAUTHORIZED, "invalid signature".to_string()));
    }
    let event = header_str(&headers, "X-GitHub-Event")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing event".to_string()))?;
    let delivery = header_str(&headers, "X-GitHub-Delivery")
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing delivery".to_string()))?;
    {
        let mut deduper = state.deliveries.lock().expect("github delivery dedupe");
        if !deduper.accept(delivery) {
            return Ok((
                StatusCode::ACCEPTED,
                Json(json!({ "accepted": true, "duplicate": true })),
            ));
        }
    }
    let payload: Value = serde_json::from_slice(&body).map_err(|err| {
        (
            StatusCode::BAD_REQUEST,
            format!("invalid JSON payload: {err}"),
        )
    })?;
    process_event(&state, event, &payload)
        .await
        .map(|value| (StatusCode::ACCEPTED, Json(value)))
        .map_err(|err| (StatusCode::INTERNAL_SERVER_ERROR, err))
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

pub fn verify_webhook_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Some(hex_digest) = signature.trim().strip_prefix("sha256=") else {
        return false;
    };
    let Ok(expected) = hex::decode(hex_digest) else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    mac.verify_slice(&expected).is_ok()
}

pub async fn process_event(
    state: &GithubWebhookState,
    event: &str,
    payload: &Value,
) -> Result<Value, String> {
    let repo = repository(payload).ok_or_else(|| "payload missing repository".to_string())?;
    let installation_id = installation_id(payload);
    let mut job = None;
    if event == "push" {
        let request = IngestJobRequest {
            input: IngestCodebaseInput {
                tenant_id: state.tenant_slug.clone(),
                repo_id: repo.repo_id.clone(),
                actor: "github-app".to_string(),
                ..Default::default()
            },
            operation: "reindex".to_string(),
            repo_url: repo.clone_url.clone(),
            installation_id,
            caps: RepoFetchCaps::default(),
            parse_budget_ms: None,
            ..Default::default()
        };
        let submitted = state
            .code_index
            .submit_ingest_job(request)
            .map_err(|err| err.to_string())?;
        job = Some(submitted.to_json());
    }

    let mutations = collaboration_mutations(state, event, payload, &repo).await?;
    let graph_version = if mutations.is_empty() {
        None
    } else {
        Some(
            state
                .code_index
                .commit_graph_mutations(mutations)
                .map_err(|err| err.to_string())?,
        )
    };

    Ok(json!({
        "accepted": true,
        "event": event,
        "repo_id": repo.repo_id,
        "job": job,
        "graph_version": graph_version,
    }))
}

async fn collaboration_mutations(
    state: &GithubWebhookState,
    event: &str,
    payload: &Value,
    repo: &RepoInfo,
) -> Result<Vec<GraphMutation>, String> {
    let mut mutations = Vec::new();
    ensure_repo_anchor(state, &mut mutations, repo)?;
    match event {
        "pull_request" => {
            let pr = payload.get("pull_request").unwrap_or(&Value::Null);
            let number = pr
                .get("number")
                .and_then(Value::as_u64)
                .or_else(|| payload.get("number").and_then(Value::as_u64))
                .unwrap_or(0);
            if number == 0 {
                return Ok(mutations);
            }
            let pr_id = graph_id(
                "github:pr",
                &[&state.tenant_slug, &repo.repo_id, &number.to_string()],
            );
            mutations.push(node(
                &pr_id,
                "PullRequest",
                json!({
                    "tenant_id": state.tenant_slug,
                    "repo_id": repo.repo_id,
                    "number": number,
                    "title": str_value(pr, "title"),
                    "state": str_value(pr, "state"),
                    "action": str_value(payload, "action"),
                    "url": str_value(pr, "html_url"),
                    "source": SOURCE,
                }),
            ));
            mutations.push(edge(
                &pr_id,
                "PART_OF",
                &repo.repo_id,
                &state.tenant_slug,
                &repo.repo_id,
            ));
            if let Some(login) = user_login(pr.get("user")) {
                let person_id = person_node(&mut mutations, &state.tenant_slug, &login);
                mutations.push(edge(
                    &pr_id,
                    "AUTHORED_BY",
                    &person_id,
                    &state.tenant_slug,
                    &repo.repo_id,
                ));
            }
            if let Some(sha) = pr.pointer("/head/sha").and_then(Value::as_str) {
                let commit_id = commit_node(&mut mutations, &state.tenant_slug, repo, sha, "");
                mutations.push(edge(
                    &commit_id,
                    "PART_OF",
                    &pr_id,
                    &state.tenant_slug,
                    &repo.repo_id,
                ));
            }
            let files =
                pull_request_files(state, payload, repo, number, installation_id(payload)).await;
            add_touch_edges(state, &mut mutations, &pr_id, repo, &files)?;
        }
        "issues" => {
            let issue = payload.get("issue").unwrap_or(&Value::Null);
            let number = issue.get("number").and_then(Value::as_u64).unwrap_or(0);
            if number == 0 {
                return Ok(mutations);
            }
            let issue_id = graph_id(
                "github:issue",
                &[&state.tenant_slug, &repo.repo_id, &number.to_string()],
            );
            mutations.push(node(
                &issue_id,
                "Issue",
                json!({
                    "tenant_id": state.tenant_slug,
                    "repo_id": repo.repo_id,
                    "number": number,
                    "title": str_value(issue, "title"),
                    "state": str_value(issue, "state"),
                    "action": str_value(payload, "action"),
                    "url": str_value(issue, "html_url"),
                    "source": SOURCE,
                }),
            ));
            mutations.push(edge(
                &issue_id,
                "PART_OF",
                &repo.repo_id,
                &state.tenant_slug,
                &repo.repo_id,
            ));
            if let Some(login) = user_login(issue.get("user")) {
                let person_id = person_node(&mut mutations, &state.tenant_slug, &login);
                mutations.push(edge(
                    &issue_id,
                    "AUTHORED_BY",
                    &person_id,
                    &state.tenant_slug,
                    &repo.repo_id,
                ));
            }
        }
        "pull_request_review" => {
            let review = payload.get("review").unwrap_or(&Value::Null);
            let pr = payload.get("pull_request").unwrap_or(&Value::Null);
            let review_id_raw = review.get("id").and_then(Value::as_u64).unwrap_or(0);
            let pr_number = pr.get("number").and_then(Value::as_u64).unwrap_or(0);
            if review_id_raw == 0 || pr_number == 0 {
                return Ok(mutations);
            }
            let review_id = graph_id(
                "github:review",
                &[
                    &state.tenant_slug,
                    &repo.repo_id,
                    &review_id_raw.to_string(),
                ],
            );
            let pr_id = graph_id(
                "github:pr",
                &[&state.tenant_slug, &repo.repo_id, &pr_number.to_string()],
            );
            mutations.push(node(
                &review_id,
                "Review",
                json!({
                    "tenant_id": state.tenant_slug,
                    "repo_id": repo.repo_id,
                    "review_id": review_id_raw,
                    "state": str_value(review, "state"),
                    "url": str_value(review, "html_url"),
                    "source": SOURCE,
                }),
            ));
            mutations.push(edge(
                &review_id,
                "REVIEWS",
                &pr_id,
                &state.tenant_slug,
                &repo.repo_id,
            ));
            if let Some(login) = user_login(review.get("user")) {
                let person_id = person_node(&mut mutations, &state.tenant_slug, &login);
                mutations.push(edge(
                    &review_id,
                    "AUTHORED_BY",
                    &person_id,
                    &state.tenant_slug,
                    &repo.repo_id,
                ));
            }
        }
        "push" => {
            for commit in payload
                .get("commits")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(sha) = commit.get("id").and_then(Value::as_str) else {
                    continue;
                };
                let commit_id = commit_node(
                    &mut mutations,
                    &state.tenant_slug,
                    repo,
                    sha,
                    commit
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                );
                if let Some(login) = commit
                    .pointer("/author/username")
                    .and_then(Value::as_str)
                    .or_else(|| {
                        commit
                            .pointer("/committer/username")
                            .and_then(Value::as_str)
                    })
                {
                    let person_id = person_node(&mut mutations, &state.tenant_slug, login);
                    mutations.push(edge(
                        &commit_id,
                        "AUTHORED_BY",
                        &person_id,
                        &state.tenant_slug,
                        &repo.repo_id,
                    ));
                }
                let files = push_commit_files(commit);
                add_touch_edges(state, &mut mutations, &commit_id, repo, &files)?;
            }
        }
        _ => {}
    }
    Ok(mutations)
}

fn ensure_repo_anchor(
    state: &GithubWebhookState,
    mutations: &mut Vec<GraphMutation>,
    repo: &RepoInfo,
) -> Result<(), String> {
    if state
        .code_index
        .graph_node_exists(&repo.repo_id)
        .map_err(|err| err.to_string())?
    {
        return Ok(());
    }
    mutations.push(node(
        &repo.repo_id,
        "CodeRepository",
        json!({
            "tenant_id": state.tenant_slug,
            "repo_id": repo.repo_id,
            "owner": repo.owner,
            "name": repo.name,
            "clone_url": repo.clone_url,
            "source": SOURCE,
        }),
    ));
    Ok(())
}

async fn pull_request_files(
    state: &GithubWebhookState,
    payload: &Value,
    repo: &RepoInfo,
    number: u64,
    installation_id: Option<u64>,
) -> Vec<FileChange> {
    let inline = file_changes_from_array(payload.pointer("/pull_request/files"))
        .or_else(|| file_changes_from_array(payload.get("files")));
    if let Some(files) = inline {
        return files;
    }
    let Some(installation_id) = installation_id else {
        return Vec::new();
    };
    state
        .app
        .pull_request_files(&repo.owner, &repo.name, number, installation_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(FileChange::from)
        .collect()
}

fn add_touch_edges(
    state: &GithubWebhookState,
    mutations: &mut Vec<GraphMutation>,
    from_id: &str,
    repo: &RepoInfo,
    files: &[FileChange],
) -> Result<(), String> {
    for file in files {
        let file_id = code_file_id(&repo.repo_id, &file.path);
        if !state
            .code_index
            .graph_node_exists(&file_id)
            .map_err(|err| err.to_string())?
        {
            continue;
        }
        mutations.push(edge(
            from_id,
            "TOUCHES_FILE",
            &file_id,
            &state.tenant_slug,
            &repo.repo_id,
        ));
        for symbol_id in symbols_touched_by_patch(state, repo, &file_id, file)? {
            mutations.push(edge(
                from_id,
                "MENTIONS_SYMBOL",
                &symbol_id,
                &state.tenant_slug,
                &repo.repo_id,
            ));
        }
    }
    Ok(())
}

fn symbols_touched_by_patch(
    state: &GithubWebhookState,
    repo: &RepoInfo,
    file_id: &str,
    file: &FileChange,
) -> Result<Vec<String>, String> {
    let Some(patch) = &file.patch else {
        return Ok(Vec::new());
    };
    let ranges = added_line_ranges(patch);
    if ranges.is_empty() {
        return Ok(Vec::new());
    }
    let nodes = state
        .code_index
        .query_graph_nodes(
            NodeQuery::label(CODE_SYMBOL_LABEL)
                .with_property("file_id", json!(file_id))
                .with_property("repo_id", json!(repo.repo_id))
                .with_limit(10_000),
        )
        .map_err(|err| err.to_string())?;
    Ok(nodes
        .into_iter()
        .filter_map(|node| {
            let line = node.properties.get("line").and_then(Value::as_u64)?;
            ranges
                .iter()
                .any(|(start, end)| line >= *start && line <= *end)
                .then_some(node.id)
        })
        .collect())
}

fn added_line_ranges(patch: &str) -> Vec<(u64, u64)> {
    patch
        .lines()
        .filter_map(|line| line.strip_prefix("@@ "))
        .filter_map(|hunk| hunk.split_whitespace().find(|part| part.starts_with('+')))
        .filter_map(|part| {
            let raw = part.trim_start_matches('+');
            let (start, len) = raw
                .split_once(',')
                .map(|(start, len)| (start, len))
                .unwrap_or((raw, "1"));
            let start = start.parse::<u64>().ok()?;
            let len = len.parse::<u64>().ok().unwrap_or(1).max(1);
            Some((start, start + len - 1))
        })
        .collect()
}

fn push_commit_files(commit: &Value) -> Vec<FileChange> {
    ["added", "modified", "removed"]
        .into_iter()
        .flat_map(|key| {
            commit
                .get(key)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(|path| FileChange {
                    path: path.to_string(),
                    patch: None,
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

fn file_changes_from_array(value: Option<&Value>) -> Option<Vec<FileChange>> {
    let files = value?.as_array()?;
    Some(
        files
            .iter()
            .filter_map(|item| {
                let path = item
                    .get("filename")
                    .or_else(|| item.get("path"))
                    .and_then(Value::as_str)?;
                Some(FileChange {
                    path: path.to_string(),
                    patch: item
                        .get("patch")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect(),
    )
}

#[derive(Clone, Debug)]
struct FileChange {
    path: String,
    patch: Option<String>,
}

impl From<GithubPullRequestFile> for FileChange {
    fn from(file: GithubPullRequestFile) -> Self {
        Self {
            path: file.filename,
            patch: file.patch,
        }
    }
}

#[derive(Clone, Debug)]
struct RepoInfo {
    owner: String,
    name: String,
    repo_id: String,
    clone_url: String,
}

fn repository(payload: &Value) -> Option<RepoInfo> {
    let repo = payload.get("repository")?;
    let name = repo.get("name").and_then(Value::as_str)?.to_string();
    let owner = repo
        .pointer("/owner/login")
        .and_then(Value::as_str)
        .or_else(|| repo.pointer("/owner/name").and_then(Value::as_str))?
        .to_string();
    let clone_url = repo
        .get("clone_url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://github.com/{owner}/{name}.git"));
    let repo_id = format!("repo:{owner}/{name}");
    Some(RepoInfo {
        owner,
        repo_id,
        name,
        clone_url,
    })
}

fn installation_id(payload: &Value) -> Option<u64> {
    payload.pointer("/installation/id").and_then(Value::as_u64)
}

fn node(id: &str, label: &str, properties: Value) -> GraphMutation {
    GraphMutation::NodeUpsert(NodeRecord::new(id, [label], properties))
}

fn edge(from: &str, edge_type: &str, to: &str, tenant: &str, repo_id: &str) -> GraphMutation {
    GraphMutation::EdgeUpsert(EdgeRecord::new(
        graph_id("github:edge", &[from, edge_type, to]),
        from,
        edge_type,
        to,
        json!({
            "tenant_id": tenant,
            "repo_id": repo_id,
            "source": SOURCE,
        }),
    ))
}

fn person_node(mutations: &mut Vec<GraphMutation>, tenant: &str, login: &str) -> String {
    let person_id = graph_id("github:person", &[tenant, login]);
    mutations.push(node(
        &person_id,
        "Person",
        json!({
            "tenant_id": tenant,
            "login": login,
            "source": SOURCE,
        }),
    ));
    person_id
}

fn commit_node(
    mutations: &mut Vec<GraphMutation>,
    tenant: &str,
    repo: &RepoInfo,
    sha: &str,
    message: &str,
) -> String {
    let commit_id = graph_id("github:commit", &[tenant, &repo.repo_id, sha]);
    mutations.push(node(
        &commit_id,
        "Commit",
        json!({
            "tenant_id": tenant,
            "repo_id": repo.repo_id,
            "sha": sha,
            "message": message,
            "source": SOURCE,
        }),
    ));
    commit_id
}

fn graph_id(prefix: &str, parts: &[&str]) -> String {
    format!("{prefix}:{}", stable_hash(json!(parts)))
}

fn code_file_id(repo_id: &str, path: &str) -> String {
    format!(
        "code:file:{}",
        stable_hash(json!({ "repo_id": repo_id, "path": path }))
    )
}

fn str_value(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn user_login(value: Option<&Value>) -> Option<String> {
    value?
        .get("login")
        .and_then(Value::as_str)
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::Request;
    use rustyred_thg_code::{CODE_FILE_LABEL, CODE_INGEST_JOB_LABEL};
    use rustyred_thg_core::{RedCoreDurability, RedCoreGraphStore, RedCoreOptions};
    use serde_json::json;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("github-webhook-test-{name}-{nanos}"))
    }

    fn test_options() -> RedCoreOptions {
        RedCoreOptions {
            durability: RedCoreDurability::AofAlways,
            snapshot_interval_writes: 0,
            ..RedCoreOptions::default()
        }
    }

    fn test_state(dir: &std::path::Path) -> GithubWebhookState {
        let store = RedCoreGraphStore::open(dir, test_options()).unwrap();
        let runtime = CodeIndexRuntime::try_new_with_store(store).unwrap();
        let app = Arc::new(GithubApp::new(
            1,
            Vec::new(),
            "secret".to_string(),
            "http://127.0.0.1:9".to_string(),
            reqwest::Client::new(),
        ));
        GithubWebhookState::new(app, runtime, "Travis-Gilbert")
    }

    fn sign_body(secret: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
    }

    fn webhook_request(
        event: &str,
        delivery: &str,
        signature: &str,
        body: impl Into<Body>,
    ) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/github/webhook")
            .header("X-Hub-Signature-256", signature)
            .header("X-GitHub-Event", event)
            .header("X-GitHub-Delivery", delivery)
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }

    fn push_payload(repo_url: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "repository": {
                "name": "demo",
                "clone_url": repo_url,
                "owner": { "login": "acme" }
            },
            "installation": { "id": 42 },
            "commits": [
                {
                    "id": "abc123",
                    "message": "change parser",
                    "author": { "username": "octo" },
                    "modified": ["src/lib.rs"]
                }
            ]
        }))
        .unwrap()
    }

    fn ingest_job_count(state: &GithubWebhookState) -> usize {
        state
            .code_index
            .query_graph_nodes(NodeQuery::label(CODE_INGEST_JOB_LABEL).with_limit(100))
            .unwrap()
            .len()
    }

    #[test]
    fn verifies_github_signature() {
        let body = br#"{"zen":"keep it logically awesome"}"#;
        let mut mac = HmacSha256::new_from_slice(b"secret").unwrap();
        mac.update(body);
        let signature = format!("sha256={}", hex::encode(mac.finalize().into_bytes()));
        assert!(verify_webhook_signature("secret", body, &signature));
        assert!(!verify_webhook_signature("wrong", body, &signature));
        assert!(!verify_webhook_signature("secret", body, "sha1=abc"));
    }

    #[tokio::test]
    async fn forged_signature_returns_401_without_enqueueing() {
        let dir = temp_dir("forged");
        let state = test_state(&dir);
        let body = push_payload("https://github.com/acme/demo.git");
        let request = webhook_request("push", "delivery-forged", "sha256=deadbeef", body);

        let response = github_router(state.clone()).oneshot(request).await.unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(ingest_job_count(&state), 0);
        fs::remove_dir_all(dir).ok();
    }

    #[tokio::test]
    async fn redelivered_delivery_id_is_accepted_without_duplicate_enqueue() {
        let dir = temp_dir("redelivery");
        let state = test_state(&dir);
        let body = push_payload("https://github.com/acme/demo.git");
        let signature = sign_body("secret", &body);

        let first = github_router(state.clone())
            .oneshot(webhook_request(
                "push",
                "delivery-duplicate",
                &signature,
                Body::from(body.clone()),
            ))
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::ACCEPTED);
        assert_eq!(ingest_job_count(&state), 1);

        let second = github_router(state.clone())
            .oneshot(webhook_request(
                "push",
                "delivery-duplicate",
                &signature,
                Body::from(body),
            ))
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::ACCEPTED);
        let body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
        let payload: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["duplicate"], true);
        assert_eq!(ingest_job_count(&state), 1);

        fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn deduplicates_delivery_ids() {
        let mut deduper = DeliveryDeduper::new(2);
        assert!(deduper.accept("a"));
        assert!(!deduper.accept("a"));
        assert!(deduper.accept("b"));
        assert!(deduper.accept("c"));
        assert!(deduper.accept("a"));
    }

    #[tokio::test]
    async fn pull_request_event_writes_pr_author_and_file_edges() {
        let dir = temp_dir("pr");
        let state = test_state(&dir);
        let repo_id = "repo:acme/demo";
        let file_id = code_file_id(repo_id, "src/lib.rs");
        state
            .code_index
            .commit_graph_mutations(vec![
                GraphMutation::NodeUpsert(NodeRecord::new(
                    &file_id,
                    [CODE_FILE_LABEL],
                    json!({
                        "tenant_id": "Travis-Gilbert",
                        "repo_id": repo_id,
                        "file_id": file_id,
                        "path": "src/lib.rs",
                    }),
                )),
                GraphMutation::NodeUpsert(NodeRecord::new(
                    "symbol:demo:main",
                    [CODE_SYMBOL_LABEL],
                    json!({
                        "tenant_id": "Travis-Gilbert",
                        "repo_id": repo_id,
                        "file_id": code_file_id(repo_id, "src/lib.rs"),
                        "line": 3,
                    }),
                )),
            ])
            .unwrap();
        let payload = json!({
            "action": "opened",
            "repository": {
                "name": "demo",
                "clone_url": "https://github.com/acme/demo.git",
                "owner": { "login": "acme" }
            },
            "pull_request": {
                "number": 7,
                "title": "Improve parser",
                "state": "open",
                "html_url": "https://github.com/acme/demo/pull/7",
                "user": { "login": "octo" },
                "head": { "sha": "abc123" },
                "files": [
                    {
                        "filename": "src/lib.rs",
                        "patch": "@@ -1,1 +1,5 @@\n+fn main() {}\n"
                    }
                ]
            }
        });
        let output = process_event(&state, "pull_request", &payload)
            .await
            .unwrap();
        assert!(output["graph_version"].as_u64().is_some());

        let pr_nodes = state
            .code_index
            .query_graph_nodes(NodeQuery::label("PullRequest").with_limit(10))
            .unwrap();
        assert_eq!(pr_nodes.len(), 1);
        let snapshot = state.code_index.graph_snapshot().unwrap();
        assert!(snapshot
            .edges
            .iter()
            .any(|edge| edge.edge_type == "TOUCHES_FILE"));
        assert!(snapshot
            .edges
            .iter()
            .any(|edge| edge.edge_type == "MENTIONS_SYMBOL"));

        fs::remove_dir_all(dir).ok();
    }
}
