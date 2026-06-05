use std::env;
use std::fs;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::search::{SearchCandidate, SearchOpts, SearchProvider, SearchProviderError};

const BRAVE_WEB_SEARCH_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
const MOJEEK_SEARCH_ENDPOINT: &str = "https://api.mojeek.com/search";
const EXA_SEARCH_ENDPOINT: &str = "https://api.exa.ai/search";
const SERPAPI_SEARCH_ENDPOINT: &str = "https://serpapi.com/search.json";

pub fn configured_search_providers_from_env() -> Vec<Arc<dyn SearchProvider>> {
    let enabled =
        env_first(&["RUSTYWEB_SEARCH_PROVIDERS", "RUSTY_RED_SEARCH_PROVIDERS"]).unwrap_or_default();
    enabled
        .split(',')
        .map(|provider| provider.trim().to_ascii_lowercase())
        .filter(|provider| !provider.is_empty())
        .filter_map(|provider| provider_from_env(&provider))
        .collect()
}

fn provider_from_env(provider: &str) -> Option<Arc<dyn SearchProvider>> {
    match provider {
        "brave" | "brave_search" => env_first(&[
            "RUSTYWEB_BRAVE_SEARCH_API_KEY",
            "RUSTY_RED_BRAVE_SEARCH_API_KEY",
            "BRAVE_SEARCH_API_KEY",
        ])
        .map(BraveSearchProvider::new)
        .map(|provider| Arc::new(provider) as Arc<dyn SearchProvider>),
        "mojeek" | "mojeek_search" => env_first(&[
            "RUSTYWEB_MOJEEK_SEARCH_API_KEY",
            "RUSTY_RED_MOJEEK_SEARCH_API_KEY",
            "MOJEEK_SEARCH_API_KEY",
        ])
        .map(MojeekSearchProvider::new)
        .map(|provider| Arc::new(provider) as Arc<dyn SearchProvider>),
        "exa" | "exa_search" => env_first(&[
            "RUSTYWEB_EXA_API_KEY",
            "RUSTY_RED_EXA_API_KEY",
            "EXA_API_KEY",
        ])
        .map(ExaSearchProvider::new)
        .map(|provider| Arc::new(provider) as Arc<dyn SearchProvider>),
        "serpapi" | "serp_api" | "google_serpapi" => env_first(&[
            "RUSTYWEB_SERPAPI_API_KEY",
            "RUSTY_RED_SERPAPI_API_KEY",
            "SERPAPI_API_KEY",
        ])
        .map(SerpApiSearchProvider::new)
        .map(|provider| Arc::new(provider) as Arc<dyn SearchProvider>),
        "offline" | "offline_jsonl" | "seed_manifest" => env_first(&[
            "RUSTYWEB_OFFLINE_SEARCH_MANIFEST",
            "RUSTY_RED_OFFLINE_SEARCH_MANIFEST",
        ])
        .and_then(|path| OfflineSearchProvider::from_path("offline", path).ok())
        .map(|provider| Arc::new(provider) as Arc<dyn SearchProvider>),
        _ => None,
    }
}

fn env_first(keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| env::var(key).ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[derive(Clone, Debug)]
pub struct BraveSearchProvider {
    api_key: String,
    client: Client,
    endpoint: String,
}

impl BraveSearchProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: Client::new(),
            endpoint: BRAVE_WEB_SEARCH_ENDPOINT.to_string(),
        }
    }
}

impl SearchProvider for BraveSearchProvider {
    fn name(&self) -> &str {
        "brave"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        opts: &'a SearchOpts,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let count = opts.provider_limit.min(20).to_string();
            let response = self
                .client
                .get(&self.endpoint)
                .header("X-Subscription-Token", &self.api_key)
                .query(&[("q", query), ("count", count.as_str())])
                .send()
                .await
                .map_err(|error| provider_error(self.name(), error.to_string()))?;
            let payload = response_json(self.name(), response).await?;
            Ok(brave_candidates(&payload, self.name()))
        })
    }
}

#[derive(Clone, Debug)]
pub struct MojeekSearchProvider {
    api_key: String,
    client: Client,
    endpoint: String,
}

impl MojeekSearchProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: Client::new(),
            endpoint: MOJEEK_SEARCH_ENDPOINT.to_string(),
        }
    }
}

impl SearchProvider for MojeekSearchProvider {
    fn name(&self) -> &str {
        "mojeek"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        opts: &'a SearchOpts,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let limit = opts.provider_limit.min(100).to_string();
            let response = self
                .client
                .get(&self.endpoint)
                .query(&[
                    ("q", query),
                    ("api_key", self.api_key.as_str()),
                    ("fmt", "json"),
                    ("t", limit.as_str()),
                ])
                .send()
                .await
                .map_err(|error| provider_error(self.name(), error.to_string()))?;
            let payload = response_json(self.name(), response).await?;
            Ok(mojeek_candidates(&payload, self.name()))
        })
    }
}

#[derive(Clone, Debug)]
pub struct ExaSearchProvider {
    api_key: String,
    client: Client,
    endpoint: String,
}

impl ExaSearchProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: Client::new(),
            endpoint: EXA_SEARCH_ENDPOINT.to_string(),
        }
    }
}

impl SearchProvider for ExaSearchProvider {
    fn name(&self) -> &str {
        "exa"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        opts: &'a SearchOpts,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.endpoint)
                .header("x-api-key", &self.api_key)
                .json(&json!({
                    "query": query,
                    "numResults": opts.provider_limit.min(100),
                    "contents": { "highlights": true }
                }))
                .send()
                .await
                .map_err(|error| provider_error(self.name(), error.to_string()))?;
            let payload = response_json(self.name(), response).await?;
            Ok(exa_candidates(&payload, self.name()))
        })
    }
}

#[derive(Clone, Debug)]
pub struct SerpApiSearchProvider {
    api_key: String,
    client: Client,
    endpoint: String,
}

impl SerpApiSearchProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            client: Client::new(),
            endpoint: SERPAPI_SEARCH_ENDPOINT.to_string(),
        }
    }
}

impl SearchProvider for SerpApiSearchProvider {
    fn name(&self) -> &str {
        "serpapi"
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        opts: &'a SearchOpts,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let num = opts.provider_limit.min(100).to_string();
            let response = self
                .client
                .get(&self.endpoint)
                .query(&[
                    ("engine", "google"),
                    ("q", query),
                    ("api_key", self.api_key.as_str()),
                    ("num", num.as_str()),
                ])
                .send()
                .await
                .map_err(|error| provider_error(self.name(), error.to_string()))?;
            let payload = response_json(self.name(), response).await?;
            Ok(serpapi_candidates(&payload, self.name()))
        })
    }
}

/// File-backed provider for offline OWI/Common Crawl seed manifests. This is
/// the light adapter path: upstream corpus jobs can emit JSON/JSONL candidates,
/// and RustyWeb feeds them through the same provider fan-out and RRF layer.
#[derive(Clone, Debug)]
pub struct OfflineSearchProvider {
    name: String,
    records: Arc<Vec<OfflineCandidateRecord>>,
}

impl OfflineSearchProvider {
    pub fn from_path(
        name: impl Into<String>,
        path: impl AsRef<std::path::Path>,
    ) -> Result<Self, SearchProviderError> {
        let provider_name = name.into();
        let content = fs::read_to_string(path.as_ref())
            .map_err(|error| provider_error(&provider_name, error.to_string()))?;
        Self::from_str(provider_name, &content)
    }

    pub fn from_str(name: impl Into<String>, content: &str) -> Result<Self, SearchProviderError> {
        let provider_name = name.into();
        let records = parse_offline_manifest(&provider_name, content)?;
        Ok(Self {
            name: provider_name,
            records: Arc::new(records),
        })
    }
}

impl SearchProvider for OfflineSearchProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn search<'a>(
        &'a self,
        query: &'a str,
        opts: &'a SearchOpts,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<SearchCandidate>, SearchProviderError>> + Send + 'a>>
    {
        Box::pin(async move {
            let query_terms = query_terms(query);
            let mut matches: Vec<(f64, usize, SearchCandidate)> = self
                .records
                .iter()
                .enumerate()
                .filter_map(|(index, record)| {
                    let score = offline_record_score(record, &query_terms);
                    if score <= 0.0 {
                        return None;
                    }
                    Some((
                        score,
                        record.rank.unwrap_or(index + 1).max(1),
                        SearchCandidate {
                            url: record.url.clone(),
                            title: record.title.clone(),
                            snippet: record.snippet.clone(),
                            source: record
                                .source
                                .clone()
                                .filter(|source| !source.trim().is_empty())
                                .unwrap_or_else(|| self.name.clone()),
                            rank: 0,
                        },
                    ))
                })
                .collect();
            matches.sort_by(|a, b| {
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(&b.1))
                    .then_with(|| a.2.url.cmp(&b.2.url))
            });
            Ok(matches
                .into_iter()
                .take(opts.provider_limit)
                .enumerate()
                .map(|(index, (_, _, mut candidate))| {
                    candidate.rank = index + 1;
                    candidate
                })
                .collect())
        })
    }
}

async fn response_json(
    provider: &str,
    response: reqwest::Response,
) -> Result<Value, SearchProviderError> {
    let status = response.status();
    if !status.is_success() {
        return Err(provider_error(provider, format!("HTTP {status}")));
    }
    response
        .json::<Value>()
        .await
        .map_err(|error| provider_error(provider, error.to_string()))
}

fn provider_error(provider: &str, message: impl Into<String>) -> SearchProviderError {
    SearchProviderError::new(provider, message)
}

fn brave_candidates(payload: &Value, source: &str) -> Vec<SearchCandidate> {
    payload
        .pointer("/web/results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, result)| {
            let url = string_field(result, "url")?;
            Some(SearchCandidate {
                url,
                title: string_field(result, "title"),
                snippet: string_field(result, "description"),
                source: source.to_string(),
                rank: index + 1,
            })
        })
        .collect()
}

fn mojeek_candidates(payload: &Value, source: &str) -> Vec<SearchCandidate> {
    payload
        .pointer("/response/results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, result)| {
            let url = string_field(result, "url")?;
            Some(SearchCandidate {
                url,
                title: string_field(result, "title"),
                snippet: string_field(result, "desc"),
                source: source.to_string(),
                rank: index + 1,
            })
        })
        .collect()
}

fn exa_candidates(payload: &Value, source: &str) -> Vec<SearchCandidate> {
    payload
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, result)| {
            let url = string_field(result, "url")?;
            Some(SearchCandidate {
                url,
                title: string_field(result, "title"),
                snippet: first_string(result.get("highlights"))
                    .or_else(|| string_field(result, "summary"))
                    .or_else(|| string_field(result, "text")),
                source: source.to_string(),
                rank: index + 1,
            })
        })
        .collect()
}

fn serpapi_candidates(payload: &Value, source: &str) -> Vec<SearchCandidate> {
    payload
        .get("organic_results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, result)| {
            let url = string_field(result, "link")?;
            Some(SearchCandidate {
                url,
                title: string_field(result, "title"),
                snippet: string_field(result, "snippet"),
                source: source.to_string(),
                rank: result
                    .get("position")
                    .and_then(Value::as_u64)
                    .map(|rank| rank as usize)
                    .unwrap_or(index + 1),
            })
        })
        .collect()
}

fn string_field(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn first_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_array)
        .and_then(|values| values.iter().find_map(Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[derive(Clone, Debug, Deserialize)]
struct OfflineCandidateRecord {
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    snippet: Option<String>,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    rank: Option<usize>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    terms: Vec<String>,
}

fn parse_offline_manifest(
    provider: &str,
    content: &str,
) -> Result<Vec<OfflineCandidateRecord>, SearchProviderError> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<OfflineCandidateRecord>>(trimmed)
            .map_err(|error| provider_error(provider, error.to_string()));
    }

    let mut records = Vec::new();
    for (line_index, line) in trimmed.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        records.push(
            serde_json::from_str::<OfflineCandidateRecord>(line).map_err(|error| {
                provider_error(provider, format!("line {}: {}", line_index + 1, error))
            })?,
        );
    }
    Ok(records)
}

fn query_terms(query: &str) -> Vec<String> {
    query
        .to_ascii_lowercase()
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::trim)
        .filter(|term| term.len() > 1)
        .map(str::to_string)
        .collect()
}

fn offline_record_score(record: &OfflineCandidateRecord, query_terms: &[String]) -> f64 {
    if query_terms.is_empty() {
        return 1.0;
    }
    let haystack = [
        record.url.as_str(),
        record.title.as_deref().unwrap_or_default(),
        record.snippet.as_deref().unwrap_or_default(),
        record.query.as_deref().unwrap_or_default(),
    ]
    .into_iter()
    .chain(record.terms.iter().map(String::as_str))
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    let matches = query_terms
        .iter()
        .filter(|term| haystack.contains(term.as_str()))
        .count();
    if matches == 0 {
        0.0
    } else {
        matches as f64 / query_terms.len() as f64
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_provider_result_shapes_into_search_candidates() {
        let brave = brave_candidates(
            &json!({
                "web": {
                    "results": [{
                        "url": "https://example.com/brave",
                        "title": "Brave title",
                        "description": "Brave snippet"
                    }]
                }
            }),
            "brave",
        );
        assert_eq!(brave[0].url, "https://example.com/brave");
        assert_eq!(brave[0].snippet.as_deref(), Some("Brave snippet"));

        let mojeek = mojeek_candidates(
            &json!({
                "response": {
                    "results": [{
                        "url": "https://example.com/mojeek",
                        "title": "Mojeek title",
                        "desc": "Mojeek snippet"
                    }]
                }
            }),
            "mojeek",
        );
        assert_eq!(mojeek[0].url, "https://example.com/mojeek");
        assert_eq!(mojeek[0].snippet.as_deref(), Some("Mojeek snippet"));

        let exa = exa_candidates(
            &json!({
                "results": [{
                    "url": "https://example.com/exa",
                    "title": "Exa title",
                    "highlights": ["Exa highlight"]
                }]
            }),
            "exa",
        );
        assert_eq!(exa[0].url, "https://example.com/exa");
        assert_eq!(exa[0].snippet.as_deref(), Some("Exa highlight"));

        let serpapi = serpapi_candidates(
            &json!({
                "organic_results": [{
                    "position": 3,
                    "link": "https://example.com/serpapi",
                    "title": "SerpAPI title",
                    "snippet": "SerpAPI snippet"
                }]
            }),
            "serpapi",
        );
        assert_eq!(serpapi[0].url, "https://example.com/serpapi");
        assert_eq!(serpapi[0].rank, 3);
    }

    #[tokio::test]
    async fn offline_manifest_provider_filters_and_ranks_seed_candidates() {
        let provider = OfflineSearchProvider::from_str(
            "offline",
            r#"
{"url":"https://example.com/owi","title":"OWI search corpus","snippet":"open web index seed","source":"owi","rank":7,"terms":["openwebsearch"]}
{"url":"https://example.com/other","title":"Unrelated","snippet":"nothing to see"}
{"url":"https://example.com/common-crawl","title":"Common Crawl records","snippet":"crawl seed corpus","rank":2}
"#,
        )
        .unwrap();

        let candidates = provider
            .search("common crawl search", &SearchOpts::default())
            .await
            .unwrap();

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].url, "https://example.com/common-crawl");
        assert_eq!(candidates[0].source, "offline");
        assert_eq!(candidates[0].rank, 1);
        assert_eq!(candidates[1].url, "https://example.com/owi");
        assert_eq!(candidates[1].source, "owi");
    }
}
