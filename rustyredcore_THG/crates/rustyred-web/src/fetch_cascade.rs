use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::header::{CONTENT_TYPE, ETAG, LAST_MODIFIED};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{global_robots_cache, RustyWebError, RustyWebResult};

pub const PROMOTION_BODY_THRESHOLD: usize = 512;
pub const CLOUDFLARE_INTERSTITIAL_THRESHOLD: usize = 4096;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub enum FetchTier {
    Http2 = 1,
    Impersonate = 2,
    Rendered = 3,
}

impl FetchTier {
    fn next(self) -> Option<Self> {
        match self {
            FetchTier::Http2 => Some(FetchTier::Impersonate),
            FetchTier::Impersonate => Some(FetchTier::Rendered),
            FetchTier::Rendered => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FetchTierResult {
    pub tier_used: FetchTier,
    pub html_bytes: Vec<u8>,
    pub http_status: u16,
    pub content_type: String,
    pub etag: String,
    pub last_modified: String,
    pub final_url: String,
    pub error: String,
    /// True when the body hit `max_bytes` and was capped (more remains at the
    /// origin). Lets the progressive-disclosure path show a "read more"
    /// affordance and lets the reader stop downloading early instead of
    /// erroring on oversize pages.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FetchCascadeOptions {
    pub user_agent: String,
    pub timeout_seconds: u64,
    pub allow_impersonate: bool,
    pub rendered_endpoint: Option<String>,
    pub respect_robots_for_escalation: bool,
}

impl FetchCascadeOptions {
    pub fn http2_only(user_agent: String, timeout_seconds: u64) -> Self {
        Self {
            user_agent,
            timeout_seconds,
            allow_impersonate: false,
            rendered_endpoint: None,
            respect_robots_for_escalation: true,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct DomainTierState {
    tiers: Arc<Mutex<HashMap<String, FetchTier>>>,
}

impl DomainTierState {
    pub fn tier_for(&self, domain: &str) -> FetchTier {
        self.tiers
            .lock()
            .ok()
            .and_then(|tiers| tiers.get(domain).copied())
            .unwrap_or(FetchTier::Http2)
    }

    pub fn promote(&self, domain: &str, tier: FetchTier) {
        if let Ok(mut tiers) = self.tiers.lock() {
            let current = tiers.entry(domain.to_string()).or_insert(FetchTier::Http2);
            if tier > *current {
                *current = tier;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct FetchCascade {
    client: reqwest::Client,
    #[cfg(feature = "impersonate-fetch")]
    impersonate_client: Option<rquest::Client>,
    state: DomainTierState,
    user_agent: String,
    allow_impersonate: bool,
    rendered_endpoint: Option<String>,
    respect_robots_for_escalation: bool,
}

impl FetchCascade {
    pub fn new(options: FetchCascadeOptions) -> RustyWebResult<Self> {
        let client = reqwest::Client::builder()
            .user_agent(options.user_agent.clone())
            .redirect(reqwest::redirect::Policy::none())
            // A dead or unresponsive host should fail fast on connect rather than
            // tie up the full request timeout (which can be the whole crawl
            // budget). Capped at the total timeout so it never exceeds it.
            .connect_timeout(Duration::from_secs(options.timeout_seconds.min(5).max(1)))
            .timeout(Duration::from_secs(options.timeout_seconds))
            .build()
            .map_err(|err| RustyWebError::Fetch {
                url: "<client>".to_string(),
                reason: err.to_string(),
            })?;
        #[cfg(feature = "impersonate-fetch")]
        let impersonate_client = if options.allow_impersonate {
            Some(
                rquest::Client::builder()
                    // wreq renamed rquest's `.impersonate(Impersonate::*)` to
                    // `.emulation(Emulation::*)`, with the presets moved into the
                    // companion wreq-util crate. Same Firefox 133 fingerprint.
                    .emulation(wreq_util::Emulation::Firefox133)
                    .timeout(Duration::from_secs(options.timeout_seconds))
                    .redirect(rquest::redirect::Policy::none())
                    .build()
                    .map_err(|err| RustyWebError::Fetch {
                        url: "<impersonate-client>".to_string(),
                        reason: err.to_string(),
                    })?,
            )
        } else {
            None
        };
        Ok(Self {
            client,
            #[cfg(feature = "impersonate-fetch")]
            impersonate_client,
            state: DomainTierState::default(),
            user_agent: options.user_agent,
            allow_impersonate: options.allow_impersonate,
            rendered_endpoint: options.rendered_endpoint,
            respect_robots_for_escalation: options.respect_robots_for_escalation,
        })
    }

    pub fn client(&self) -> &reqwest::Client {
        &self.client
    }

    pub fn max_supported_tier(&self) -> FetchTier {
        if self.rendered_endpoint.is_some() {
            FetchTier::Rendered
        } else if self.impersonate_supported() {
            FetchTier::Impersonate
        } else {
            FetchTier::Http2
        }
    }

    pub async fn fetch_with_promotion(
        &self,
        canonical_url: &str,
        max_bytes: usize,
    ) -> RustyWebResult<FetchTierResult> {
        let domain = domain_for_url(canonical_url)?;
        let desired_tier = self.state.tier_for(&domain);
        let supported_tier = desired_tier.min(self.max_supported_tier());
        let tier = if supported_tier > FetchTier::Http2
            && !self.escalation_allowed(canonical_url).await?
        {
            FetchTier::Http2
        } else {
            supported_tier
        };
        let result = match tier {
            FetchTier::Http2 => self.fetch_http2(canonical_url, max_bytes, tier).await?,
            FetchTier::Impersonate => self.fetch_impersonate(canonical_url, max_bytes).await?,
            FetchTier::Rendered => self.fetch_rendered(canonical_url, max_bytes).await?,
        };

        if should_promote(&result) {
            if let Some(next_tier) = result.tier_used.next() {
                if next_tier <= self.max_supported_tier()
                    && self.escalation_allowed(canonical_url).await?
                {
                    self.state.promote(&domain, next_tier);
                }
            }
        }

        Ok(result)
    }

    async fn fetch_http2(
        &self,
        canonical_url: &str,
        max_bytes: usize,
        tier_used: FetchTier,
    ) -> RustyWebResult<FetchTierResult> {
        let response =
            self.client
                .get(canonical_url)
                .send()
                .await
                .map_err(|err| RustyWebError::Fetch {
                    url: canonical_url.to_string(),
                    reason: err.to_string(),
                })?;
        let status = response.status().as_u16();
        let final_url = response.url().to_string();
        let headers = response.headers().clone();
        let content_type = header_string(&headers, CONTENT_TYPE.as_str())
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let etag = header_string(&headers, ETAG.as_str()).unwrap_or_default();
        let last_modified = header_string(&headers, LAST_MODIFIED.as_str()).unwrap_or_default();
        let (html_bytes, truncated) =
            read_limited_body(response, canonical_url, max_bytes).await?;

        Ok(FetchTierResult {
            tier_used,
            html_bytes,
            http_status: status,
            content_type,
            etag,
            last_modified,
            final_url,
            error: String::new(),
            truncated,
        })
    }

    async fn fetch_impersonate(
        &self,
        canonical_url: &str,
        max_bytes: usize,
    ) -> RustyWebResult<FetchTierResult> {
        #[cfg(feature = "impersonate-fetch")]
        {
            let Some(client) = &self.impersonate_client else {
                return self
                    .fetch_http2(canonical_url, max_bytes, FetchTier::Http2)
                    .await;
            };
            let response =
                client
                    .get(canonical_url)
                    .send()
                    .await
                    .map_err(|err| RustyWebError::Fetch {
                        url: canonical_url.to_string(),
                        reason: err.to_string(),
                    })?;
            let status = response.status().as_u16();
            let final_url = response.url().to_string();
            let headers = response.headers().clone();
            let content_type =
                rquest_header_string(&headers, rquest::header::CONTENT_TYPE.as_str())
                    .unwrap_or_else(|| "application/octet-stream".to_string());
            let etag =
                rquest_header_string(&headers, rquest::header::ETAG.as_str()).unwrap_or_default();
            let last_modified =
                rquest_header_string(&headers, rquest::header::LAST_MODIFIED.as_str())
                    .unwrap_or_default();
            let raw = response
                .bytes()
                .await
                .map_err(|err| RustyWebError::Fetch {
                    url: canonical_url.to_string(),
                    reason: err.to_string(),
                })?;
            // Cap oversize bodies instead of erroring (see cap_body): the
            // progressive-disclosure path wants the top of the page, not a hard
            // failure. The impersonate client buffers, so this truncates
            // post-read; the Http2 tier early-stops mid-stream.
            let (html_bytes, truncated) = cap_body(&raw[..], max_bytes);
            Ok(FetchTierResult {
                tier_used: FetchTier::Impersonate,
                html_bytes,
                http_status: status,
                content_type,
                etag,
                last_modified,
                final_url,
                error: String::new(),
                truncated,
            })
        }
        #[cfg(not(feature = "impersonate-fetch"))]
        {
            self.fetch_http2(canonical_url, max_bytes, FetchTier::Http2)
                .await
        }
    }

    async fn fetch_rendered(
        &self,
        canonical_url: &str,
        max_bytes: usize,
    ) -> RustyWebResult<FetchTierResult> {
        let Some(endpoint) = &self.rendered_endpoint else {
            return self.fetch_impersonate(canonical_url, max_bytes).await;
        };
        let response = self
            .client
            .post(endpoint)
            .json(&RenderedFetchRequest {
                action: "navigate_render_extract",
                url: canonical_url,
                max_bytes,
            })
            .send()
            .await
            .map_err(|err| RustyWebError::Fetch {
                url: canonical_url.to_string(),
                reason: format!("rendered fetch endpoint failed: {err}"),
            })?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(RustyWebError::Fetch {
                url: canonical_url.to_string(),
                reason: format!("rendered fetch endpoint returned {status}"),
            });
        }
        let rendered: RenderedFetchResponse =
            response.json().await.map_err(|err| RustyWebError::Fetch {
                url: canonical_url.to_string(),
                reason: format!("rendered fetch response decode failed: {err}"),
            })?;
        let raw = rendered.html.into_bytes();
        let (html_bytes, truncated) = cap_body(&raw[..], max_bytes);
        Ok(FetchTierResult {
            tier_used: FetchTier::Rendered,
            html_bytes,
            http_status: rendered.http_status.unwrap_or(200),
            content_type: rendered
                .content_type
                .unwrap_or_else(|| "text/html; charset=utf-8".to_string()),
            etag: String::new(),
            last_modified: String::new(),
            final_url: rendered
                .final_url
                .unwrap_or_else(|| canonical_url.to_string()),
            error: String::new(),
            truncated,
        })
    }

    async fn escalation_allowed(&self, canonical_url: &str) -> RustyWebResult<bool> {
        if !self.respect_robots_for_escalation {
            return Ok(true);
        }
        let decision = global_robots_cache()
            .check(&self.client, canonical_url, &self.user_agent)
            .await?;
        Ok(decision.allowed)
    }

    fn impersonate_supported(&self) -> bool {
        if !self.allow_impersonate {
            return false;
        }
        #[cfg(feature = "impersonate-fetch")]
        {
            self.impersonate_client.is_some()
        }
        #[cfg(not(feature = "impersonate-fetch"))]
        {
            false
        }
    }
}

#[derive(Serialize)]
struct RenderedFetchRequest<'a> {
    action: &'static str,
    url: &'a str,
    max_bytes: usize,
}

#[derive(Deserialize)]
struct RenderedFetchResponse {
    final_url: Option<String>,
    html: String,
    http_status: Option<u16>,
    content_type: Option<String>,
}

pub fn should_promote(result: &FetchTierResult) -> bool {
    match result.http_status {
        0 => false,
        403 | 429 | 503 => true,
        status if (200..300).contains(&status) => {
            let size = result.html_bytes.len();
            size < PROMOTION_BODY_THRESHOLD
                || (size < CLOUDFLARE_INTERSTITIAL_THRESHOLD
                    && (contains_bytes(&result.html_bytes, b"Just a moment")
                        || contains_bytes(&result.html_bytes, b"cf-browser-verification")))
        }
        _ => false,
    }
}

/// Cap an already-buffered body at `limit` bytes, returning `(body, truncated)`.
///
/// Used by the tiers whose client buffers the whole response (impersonate,
/// rendered). The Http2 tier early-stops mid-stream via `read_limited_body`
/// instead. Truncation is at a fixed byte count so the cut point is
/// deterministic (parity-safe).
fn cap_body(raw: &[u8], limit: usize) -> (Vec<u8>, bool) {
    if raw.len() > limit {
        (raw[..limit].to_vec(), true)
    } else {
        (raw.to_vec(), false)
    }
}

/// Read the response body, capping at `limit` bytes. Returns `(body, truncated)`.
///
/// When the cap is reached the remaining stream is dropped (early-stop), so a
/// large page costs only `limit` bytes of transfer instead of the full payload.
/// Oversize pages are truncated rather than rejected: the progressive-disclosure
/// path needs only the top of the page (enough to extract relevant passages),
/// and the full body is retrieved lazily on demand.
async fn read_limited_body(
    mut response: reqwest::Response,
    url: &str,
    limit: usize,
) -> RustyWebResult<(Vec<u8>, bool)> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|err| RustyWebError::Fetch {
        url: url.to_string(),
        reason: err.to_string(),
    })? {
        if body.len().saturating_add(chunk.len()) > limit {
            let take = limit.saturating_sub(body.len());
            body.extend_from_slice(&chunk[..take]);
            return Ok((body, true));
        }
        body.extend_from_slice(&chunk);
    }
    Ok((body, false))
}

fn header_string(headers: &reqwest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

#[cfg(feature = "impersonate-fetch")]
fn rquest_header_string(headers: &rquest::header::HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string)
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn domain_for_url(raw: &str) -> RustyWebResult<String> {
    let url = Url::parse(raw).map_err(|err| RustyWebError::InvalidUrl {
        url: raw.to_string(),
        reason: err.to_string(),
    })?;
    url.host_str()
        .map(|host| host.to_ascii_lowercase())
        .ok_or_else(|| RustyWebError::InvalidUrl {
            url: raw.to_string(),
            reason: "missing host".to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(status: u16, body: &[u8]) -> FetchTierResult {
        FetchTierResult {
            tier_used: FetchTier::Http2,
            html_bytes: body.to_vec(),
            http_status: status,
            content_type: "text/html".to_string(),
            etag: String::new(),
            last_modified: String::new(),
            final_url: "https://example.com/".to_string(),
            error: String::new(),
            truncated: false,
        }
    }

    fn options() -> FetchCascadeOptions {
        FetchCascadeOptions::http2_only("RustyWeb test".to_string(), 5)
    }

    #[test]
    fn promotion_triggers_match_the_theseus_fetch_contract() {
        assert!(should_promote(&result(403, b"forbidden")));
        assert!(should_promote(&result(429, b"slow down")));
        assert!(should_promote(&result(503, b"unavailable")));
        assert!(should_promote(&result(200, b"tiny")));
        assert!(should_promote(&result(200, b"Just a moment")));
        assert!(should_promote(&result(200, b"cf-browser-verification")));
    }

    #[test]
    fn promotion_does_not_fire_for_transport_failure_or_healthy_body() {
        let healthy = vec![b'x'; PROMOTION_BODY_THRESHOLD + 16];
        assert!(!should_promote(&result(0, b"")));
        assert!(!should_promote(&result(200, &healthy)));
        assert!(!should_promote(&result(404, b"not found")));
    }

    #[test]
    fn domain_tier_state_only_promotes() {
        let state = DomainTierState::default();
        assert_eq!(state.tier_for("example.com"), FetchTier::Http2);
        state.promote("example.com", FetchTier::Impersonate);
        state.promote("example.com", FetchTier::Http2);
        assert_eq!(state.tier_for("example.com"), FetchTier::Impersonate);
    }

    #[test]
    fn tier_order_advances_from_http_to_impersonate_to_rendered() {
        assert_eq!(FetchTier::Http2.next(), Some(FetchTier::Impersonate));
        assert_eq!(FetchTier::Impersonate.next(), Some(FetchTier::Rendered));
        assert_eq!(FetchTier::Rendered.next(), None);
    }

    #[test]
    fn cascade_reports_rendered_supported_when_endpoint_is_configured() {
        let mut options = options();
        options.rendered_endpoint = Some("http://127.0.0.1:9/render".to_string());
        let cascade = FetchCascade::new(options).expect("cascade");
        assert_eq!(cascade.max_supported_tier(), FetchTier::Rendered);
    }

    #[test]
    fn cascade_stays_http2_only_without_optional_escalators() {
        let cascade = FetchCascade::new(options()).expect("cascade");
        assert_eq!(cascade.max_supported_tier(), FetchTier::Http2);
    }

    #[test]
    fn cap_body_truncates_at_a_deterministic_boundary() {
        // Under the cap: whole body, not flagged.
        let (body, truncated) = cap_body(b"hello", 16);
        assert_eq!(body, b"hello");
        assert!(!truncated);

        // Exactly at the cap: whole body, not flagged.
        let (body, truncated) = cap_body(b"hello", 5);
        assert_eq!(body, b"hello");
        assert!(!truncated);

        // Over the cap: first `limit` bytes, flagged.
        let (body, truncated) = cap_body(b"hello world", 5);
        assert_eq!(body, b"hello");
        assert!(truncated);

        // Zero cap: empty body, flagged.
        let (body, truncated) = cap_body(b"data", 0);
        assert!(body.is_empty());
        assert!(truncated);
    }
}
