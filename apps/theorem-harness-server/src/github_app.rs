//! GitHub App authentication edge.
//!
//! The server owns App JWT signing, installation-token minting, and the
//! in-memory token cache. The code-index worker only sees a short-lived
//! `GitCredential` resolved at clone time.

use std::collections::HashMap;
use std::fs;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use reqwest::header::{ACCEPT, AUTHORIZATION, USER_AGENT};
use rustyred_thg_code::{GitCredential, GitCredentialResolver};
use serde::{Deserialize, Serialize};
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

const DEFAULT_API_BASE: &str = "https://api.github.com";
const GITHUB_API_VERSION: &str = "2022-11-28";
const TOKEN_REUSE_MARGIN: Duration = Duration::from_secs(5 * 60);

#[derive(Clone, Debug)]
pub struct InstallationToken {
    pub token: String,
    pub expires_at: SystemTime,
}

#[derive(Debug)]
pub enum GithubAppError {
    MissingConfig(String),
    InvalidConfig(String),
    Jwt(String),
    Http(String),
    Runtime(String),
}

impl std::fmt::Display for GithubAppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GithubAppError::MissingConfig(message)
            | GithubAppError::InvalidConfig(message)
            | GithubAppError::Jwt(message)
            | GithubAppError::Http(message)
            | GithubAppError::Runtime(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for GithubAppError {}

#[derive(Clone)]
pub struct GithubApp {
    app_id: u64,
    private_key_pem: Arc<Vec<u8>>,
    webhook_secret: Arc<String>,
    http: reqwest::Client,
    api_base: Arc<String>,
    token_cache: Arc<Mutex<HashMap<u64, InstallationToken>>>,
    runtime_handle: Option<tokio::runtime::Handle>,
    #[cfg(test)]
    jwt_override: Option<Arc<String>>,
}

impl GithubApp {
    pub fn from_env() -> Result<Self, GithubAppError> {
        let app_id = first_env(&["THEOREM_GITHUB_APP_ID", "GITHUB_APP_ID"])
            .ok_or_else(|| {
                GithubAppError::MissingConfig(
                    "THEOREM_GITHUB_APP_ID or GITHUB_APP_ID is required".to_string(),
                )
            })?
            .parse::<u64>()
            .map_err(|err| {
                GithubAppError::InvalidConfig(format!("invalid GitHub App id: {err}"))
            })?;
        let private_key_pem = read_private_key_from_env()?;
        let webhook_secret = first_env(&["THEOREM_GITHUB_WEBHOOK_SECRET", "GITHUB_WEBHOOK_SECRET"])
            .ok_or_else(|| {
                GithubAppError::MissingConfig(
                    "THEOREM_GITHUB_WEBHOOK_SECRET or GITHUB_WEBHOOK_SECRET is required"
                        .to_string(),
                )
            })?;
        let api_base = first_env(&["THEOREM_GITHUB_API_BASE", "GITHUB_API_BASE"])
            .unwrap_or_else(|| DEFAULT_API_BASE.to_string());
        Ok(Self::new(
            app_id,
            private_key_pem,
            webhook_secret,
            api_base,
            reqwest::Client::new(),
        ))
    }

    pub fn new(
        app_id: u64,
        private_key_pem: Vec<u8>,
        webhook_secret: String,
        api_base: String,
        http: reqwest::Client,
    ) -> Self {
        Self {
            app_id,
            private_key_pem: Arc::new(private_key_pem),
            webhook_secret: Arc::new(webhook_secret),
            http,
            api_base: Arc::new(api_base.trim_end_matches('/').to_string()),
            token_cache: Arc::new(Mutex::new(HashMap::new())),
            runtime_handle: tokio::runtime::Handle::try_current().ok(),
            #[cfg(test)]
            jwt_override: None,
        }
    }

    #[cfg(test)]
    fn with_test_jwt(app_id: u64, webhook_secret: String, api_base: String, jwt: String) -> Self {
        let mut app = Self::new(
            app_id,
            Vec::new(),
            webhook_secret,
            api_base,
            reqwest::Client::new(),
        );
        app.jwt_override = Some(Arc::new(jwt));
        app
    }

    pub fn webhook_secret(&self) -> &str {
        self.webhook_secret.as_str()
    }

    pub async fn installation_token(
        &self,
        installation_id: u64,
    ) -> Result<InstallationToken, GithubAppError> {
        let now = SystemTime::now();
        if let Some(cached) = self.cached_token(installation_id, now) {
            return Ok(cached);
        }

        let jwt = self.app_jwt()?;
        let url = format!(
            "{}/app/installations/{installation_id}/access_tokens",
            self.api_base
        );
        let response = self
            .http
            .post(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(USER_AGENT, "theorem-harness-server")
            .header(AUTHORIZATION, format!("Bearer {jwt}"))
            .send()
            .await
            .map_err(|err| GithubAppError::Http(format!("GitHub token request failed: {err}")))?;
        if !response.status().is_success() {
            return Err(GithubAppError::Http(format!(
                "GitHub token request returned {}",
                response.status()
            )));
        }
        let token: InstallationTokenResponse = response.json().await.map_err(|err| {
            GithubAppError::Http(format!("GitHub token response was not valid JSON: {err}"))
        })?;
        let minted = InstallationToken {
            token: token.token,
            expires_at: parse_github_time(&token.expires_at)?,
        };
        let mut cache = self.token_cache.lock().expect("github token cache");
        cache.insert(installation_id, minted.clone());
        Ok(minted)
    }

    pub async fn installation_for_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<u64, GithubAppError> {
        let jwt = self.app_jwt()?;
        let url = format!(
            "{}/repos/{}/{}/installation",
            self.api_base,
            owner.trim(),
            repo.trim()
        );
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(USER_AGENT, "theorem-harness-server")
            .header(AUTHORIZATION, format!("Bearer {jwt}"))
            .send()
            .await
            .map_err(|err| {
                GithubAppError::Http(format!("GitHub installation lookup failed: {err}"))
            })?;
        if !response.status().is_success() {
            return Err(GithubAppError::Http(format!(
                "GitHub installation lookup returned {}",
                response.status()
            )));
        }
        let installation: InstallationLookupResponse = response.json().await.map_err(|err| {
            GithubAppError::Http(format!(
                "GitHub installation lookup response was not valid JSON: {err}"
            ))
        })?;
        Ok(installation.id)
    }

    pub async fn pull_request_files(
        &self,
        owner: &str,
        repo: &str,
        pull_number: u64,
        installation_id: u64,
    ) -> Result<Vec<GithubPullRequestFile>, GithubAppError> {
        let token = self.installation_token(installation_id).await?;
        let url = format!(
            "{}/repos/{}/{}/pulls/{pull_number}/files?per_page=100",
            self.api_base,
            owner.trim(),
            repo.trim()
        );
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", GITHUB_API_VERSION)
            .header(USER_AGENT, "theorem-harness-server")
            .header(AUTHORIZATION, format!("Bearer {}", token.token))
            .send()
            .await
            .map_err(|err| {
                GithubAppError::Http(format!("GitHub pull request files request failed: {err}"))
            })?;
        if !response.status().is_success() {
            return Err(GithubAppError::Http(format!(
                "GitHub pull request files request returned {}",
                response.status()
            )));
        }
        response.json().await.map_err(|err| {
            GithubAppError::Http(format!(
                "GitHub pull request files response was not valid JSON: {err}"
            ))
        })
    }

    fn cached_token(&self, installation_id: u64, now: SystemTime) -> Option<InstallationToken> {
        self.token_cache
            .lock()
            .expect("github token cache")
            .get(&installation_id)
            .filter(|token| {
                token
                    .expires_at
                    .duration_since(now)
                    .map(|remaining| remaining > TOKEN_REUSE_MARGIN)
                    .unwrap_or(false)
            })
            .cloned()
    }

    fn app_jwt(&self) -> Result<String, GithubAppError> {
        #[cfg(test)]
        if let Some(jwt) = &self.jwt_override {
            return Ok(jwt.as_ref().clone());
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|err| GithubAppError::Jwt(format!("system clock before epoch: {err}")))?
            .as_secs();
        let claims = AppClaims {
            iat: now.saturating_sub(60),
            exp: now + 10 * 60,
            iss: self.app_id.to_string(),
        };
        encode(
            &Header::new(Algorithm::RS256),
            &claims,
            &EncodingKey::from_rsa_pem(&self.private_key_pem)
                .map_err(|err| GithubAppError::Jwt(format!("invalid GitHub App PEM: {err}")))?,
        )
        .map_err(|err| GithubAppError::Jwt(format!("could not sign GitHub App JWT: {err}")))
    }

    fn block_on<F, T>(&self, future: F) -> Result<T, GithubAppError>
    where
        F: Future<Output = Result<T, GithubAppError>>,
    {
        if let Some(handle) = &self.runtime_handle {
            return Ok(handle.block_on(future)?);
        }
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| GithubAppError::Runtime(format!("could not start runtime: {err}")))?;
        runtime.block_on(future)
    }
}

impl GitCredentialResolver for GithubApp {
    fn resolve(&self, repo_url: &str) -> Option<GitCredential> {
        let (owner, repo) = parse_github_repo(repo_url)?;
        let installation_id = self
            .block_on(self.installation_for_repo(&owner, &repo))
            .ok()?;
        self.resolve_installation(repo_url, installation_id)
    }

    fn resolve_installation(&self, _repo_url: &str, installation_id: u64) -> Option<GitCredential> {
        self.block_on(self.installation_token(installation_id))
            .ok()
            .map(|token| GitCredential::BearerToken(token.token))
    }
}

#[derive(Serialize)]
struct AppClaims {
    iat: u64,
    exp: u64,
    iss: String,
}

#[derive(Deserialize)]
struct InstallationTokenResponse {
    token: String,
    expires_at: String,
}

#[derive(Deserialize)]
struct InstallationLookupResponse {
    id: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct GithubPullRequestFile {
    pub filename: String,
    #[serde(default)]
    pub patch: Option<String>,
}

fn read_private_key_from_env() -> Result<Vec<u8>, GithubAppError> {
    if let Some(raw) = first_env(&["THEOREM_GITHUB_APP_PRIVATE_KEY", "GITHUB_APP_PRIVATE_KEY"]) {
        return Ok(raw.replace("\\n", "\n").into_bytes());
    }
    let path = first_env(&[
        "THEOREM_GITHUB_APP_PRIVATE_KEY_PATH",
        "GITHUB_APP_PRIVATE_KEY_PATH",
    ])
    .ok_or_else(|| {
        GithubAppError::MissingConfig(
            "THEOREM_GITHUB_APP_PRIVATE_KEY or THEOREM_GITHUB_APP_PRIVATE_KEY_PATH is required"
                .to_string(),
        )
    })?;
    fs::read(path.trim())
        .map_err(|err| GithubAppError::InvalidConfig(format!("could not read private key: {err}")))
}

fn first_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn parse_github_time(raw: &str) -> Result<SystemTime, GithubAppError> {
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .map_err(|err| GithubAppError::Http(format!("invalid GitHub expires_at: {err}")))?;
    let timestamp = parsed.unix_timestamp();
    if timestamp < 0 {
        return Err(GithubAppError::Http(
            "GitHub expires_at was before Unix epoch".to_string(),
        ));
    }
    Ok(UNIX_EPOCH + Duration::from_secs(timestamp as u64))
}

fn parse_github_repo(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim().trim_end_matches(".git").trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .or_else(|| trimmed.strip_prefix("git@github.com:"))?;
    let mut parts = without_scheme.split('/');
    let owner = parts.next()?.trim();
    let repo = parts.next()?.trim();
    if owner.is_empty() || repo.is_empty() {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use serde_json::{json, Value};
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Clone)]
    struct TokenStubState {
        calls: Arc<AtomicUsize>,
    }

    async fn token_stub(
        axum::extract::State(state): axum::extract::State<TokenStubState>,
    ) -> Json<Value> {
        let call = state.calls.fetch_add(1, Ordering::SeqCst) + 1;
        Json(json!({
            "token": format!("token-{call}"),
            "expires_at": "2099-01-01T00:00:00Z"
        }))
    }

    async fn stub_server(calls: Arc<AtomicUsize>) -> SocketAddr {
        let app = Router::new()
            .route(
                "/app/installations/:installation_id/access_tokens",
                post(token_stub),
            )
            .with_state(TokenStubState { calls });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        addr
    }

    #[tokio::test]
    async fn installation_token_reuses_cache_and_remints_after_expiry() {
        let calls = Arc::new(AtomicUsize::new(0));
        let addr = stub_server(calls.clone()).await;
        let app = GithubApp::with_test_jwt(
            123,
            "secret".to_string(),
            format!("http://{addr}"),
            "test-jwt".to_string(),
        );

        let first = app.installation_token(99).await.unwrap();
        let second = app.installation_token(99).await.unwrap();
        assert_eq!(first.token, "token-1");
        assert_eq!(second.token, "token-1");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        app.token_cache.lock().unwrap().insert(
            99,
            InstallationToken {
                token: "expired".to_string(),
                expires_at: SystemTime::now() + Duration::from_secs(30),
            },
        );
        let third = app.installation_token(99).await.unwrap();
        assert_eq!(third.token, "token-2");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
